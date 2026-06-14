//! A hand-rolled parser for the subset of TOML needed by `Cargo.toml` and
//! `Cargo.lock`.
//!
//! Supported: standard tables `[a.b.c]`, array-of-tables `[[package]]`,
//! `key = value`, dotted keys (`a.b = 1`), basic and literal strings (including
//! their multiline forms), integers, floats, booleans, arrays, inline tables
//! `{ a = 1 }`, and `#` comments. Datetimes are accepted as opaque strings.
//!
//! The parser is bounded by a maximum input size and a value/structure depth
//! cap, returns [`PrismError::Corrupt`] on malformed input, and never panics.

use prism_model::{PrismError, Result};

/// Largest input accepted by [`parse_toml`], in bytes (32 MiB).
pub const MAX_TOML_BYTES: usize = 32 * 1024 * 1024;

/// Maximum value/table nesting depth before [`parse_toml`] aborts.
pub const MAX_TOML_DEPTH: usize = 128;

/// A parsed TOML value.
///
/// Datetimes are not interpreted; they are surfaced as [`Toml::Datetime`]
/// holding the raw lexeme, which is all manifest ingestion needs.
#[derive(Clone, Debug, PartialEq)]
pub enum Toml {
    /// A string (escapes already decoded for basic strings).
    Str(String),
    /// A 64-bit integer.
    Int(i64),
    /// A floating-point number.
    Float(f64),
    /// A boolean.
    Bool(bool),
    /// An RFC-3339-ish datetime, stored verbatim.
    Datetime(String),
    /// An array, in document order.
    Array(Vec<Toml>),
    /// A table, in insertion order.
    Table(Vec<(String, Toml)>),
}

impl Toml {
    /// The string payload, if this is a [`Toml::Str`].
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Toml::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The value as `i64`, if it is an integer.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Toml::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// The value as `f64`, if it is numeric.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Toml::Float(f) => Some(*f),
            Toml::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// The boolean payload, if this is a [`Toml::Bool`].
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Toml::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The array elements, if this is a [`Toml::Array`].
    pub fn as_array(&self) -> Option<&[Toml]> {
        match self {
            Toml::Array(a) => Some(a),
            _ => None,
        }
    }

    /// The table entries, if this is a [`Toml::Table`].
    pub fn as_table(&self) -> Option<&[(String, Toml)]> {
        match self {
            Toml::Table(t) => Some(t),
            _ => None,
        }
    }

    /// Look up `key` in a table.
    pub fn get(&self, key: &str) -> Option<&Toml> {
        match self {
            Toml::Table(t) => t.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Index into an array.
    pub fn index(&self, i: usize) -> Option<&Toml> {
        match self {
            Toml::Array(a) => a.get(i),
            _ => None,
        }
    }

    /// Resolve a path of nested table keys.
    pub fn path<I, S>(&self, keys: I) -> Option<&Toml>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut cur = self;
        for k in keys {
            cur = cur.get(k.as_ref())?;
        }
        Some(cur)
    }

    // --- mutation helpers used only during construction ---

    fn as_table_mut(&mut self) -> Option<&mut Vec<(String, Toml)>> {
        match self {
            Toml::Table(t) => Some(t),
            _ => None,
        }
    }
}

/// Parse a complete TOML document into its root table.
///
/// Returns [`PrismError::Corrupt`] on malformed input, oversize input, or
/// nesting deeper than [`MAX_TOML_DEPTH`].
pub fn parse_toml(input: &str) -> Result<Toml> {
    if input.len() > MAX_TOML_BYTES {
        return Err(corrupt(format!(
            "toml input too large: {} bytes (max {MAX_TOML_BYTES})",
            input.len()
        )));
    }
    let mut p = Parser::new(input);
    p.parse_document()?;
    Ok(p.root)
}

fn corrupt(detail: String) -> PrismError {
    PrismError::Corrupt { detail }
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    root: Toml,
    /// Key path of the current `[table]` / `[[array]]` context, into which bare
    /// `key = value` lines are inserted.
    current: Vec<String>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            pos: 0,
            line: 1,
            root: Toml::Table(Vec::new()),
            current: Vec::new(),
        }
    }

    fn err(&self, msg: &str) -> PrismError {
        corrupt(format!("toml: {msg} at line {}", self.line))
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<u8> {
        self.bytes.get(self.pos + off).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek();
        if let Some(c) = b {
            self.pos += 1;
            if c == b'\n' {
                self.line += 1;
            }
        }
        b
    }

    /// Skip spaces and tabs only (not newlines).
    fn skip_inline_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.pos += 1;
        }
    }

    /// Skip a `# comment` to end of line (does not consume the newline).
    fn skip_comment(&mut self) {
        if self.peek() == Some(b'#') {
            while !matches!(self.peek(), None | Some(b'\n')) {
                self.pos += 1;
            }
        }
    }

    fn parse_document(&mut self) -> Result<()> {
        loop {
            self.skip_inline_ws();
            self.skip_comment();
            match self.peek() {
                None => return Ok(()),
                Some(b'\n') => {
                    self.bump();
                }
                Some(b'\r') => {
                    self.bump();
                }
                Some(b'[') => self.parse_table_header()?,
                _ => self.parse_keyval_line()?,
            }
        }
    }

    // ---- table headers ----

    fn parse_table_header(&mut self) -> Result<()> {
        self.pos += 1; // consume first '['
        let is_array = self.peek() == Some(b'[');
        if is_array {
            self.pos += 1;
        }
        self.skip_inline_ws();
        let path = self.parse_key_path()?;
        self.skip_inline_ws();
        // closing bracket(s)
        if self.peek() != Some(b']') {
            return Err(self.err("expected ']' in table header"));
        }
        self.pos += 1;
        if is_array {
            if self.peek() != Some(b']') {
                return Err(self.err("expected ']]' in array-of-tables header"));
            }
            self.pos += 1;
        }
        self.skip_inline_ws();
        self.skip_comment();
        self.expect_eol()?;

        if path.is_empty() {
            return Err(self.err("empty table header"));
        }

        if is_array {
            self.open_array_table(&path)?;
        } else {
            self.open_table(&path)?;
        }
        self.current = path;
        Ok(())
    }

    /// Ensure `[a.b.c]` exists as a table and set it as the insertion context.
    fn open_table(&mut self, path: &[String]) -> Result<()> {
        let line = self.line;
        let mut node = &mut self.root;
        for (i, key) in path.iter().enumerate() {
            let last = i + 1 == path.len();
            node = descend_or_create_table(node, key)
                .map_err(|m| corrupt(format!("toml: {m} at line {line}")))?;
            // Sub-tables may live under the last element of an array-of-tables.
            node = follow_last_array_element(node);
            if last {
                // The header [a] may legally re-open a table created implicitly
                // by [a.b], but a value (or redefinition) there is an error.
                if !matches!(node, Toml::Table(_)) {
                    return Err(corrupt(format!(
                        "toml: table header conflicts with existing value at line {line}"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Append a new element to the array-of-tables at `path`.
    fn open_array_table(&mut self, path: &[String]) -> Result<()> {
        let line = self.line;
        let (parent_path, last) = path.split_at(path.len() - 1);
        let key = &last[0];
        let mut node = &mut self.root;
        for p in parent_path {
            node = descend_or_create_table(node, p)
                .map_err(|m| corrupt(format!("toml: {m} at line {line}")))?;
        }
        let table = node
            .as_table_mut()
            .ok_or_else(|| corrupt(format!("toml: array-of-tables parent is not a table at line {line}")))?;
        // Find or create the array slot.
        match table.iter().position(|(k, _)| k == key) {
            Some(idx) => match &mut table[idx].1 {
                Toml::Array(arr) => arr.push(Toml::Table(Vec::new())),
                _ => {
                    return Err(corrupt(format!(
                        "toml: array-of-tables name conflicts with a value at line {line}"
                    )))
                }
            },
            None => {
                table.push((key.clone(), Toml::Array(vec![Toml::Table(Vec::new())])));
            }
        }
        Ok(())
    }

    // ---- key = value lines ----

    fn parse_keyval_line(&mut self) -> Result<()> {
        let key_path = self.parse_key_path()?;
        if key_path.is_empty() {
            return Err(self.err("expected key"));
        }
        self.skip_inline_ws();
        if self.peek() != Some(b'=') {
            return Err(self.err("expected '=' after key"));
        }
        self.pos += 1;
        self.skip_inline_ws();
        let value = self.parse_value(0)?;
        self.skip_inline_ws();
        self.skip_comment();
        self.expect_eol()?;

        // Resolve into the current [table] context, honoring dotted keys.
        let ctx = self.current.clone();
        self.insert_keyval(&ctx, &key_path, value)
    }

    /// Insert `value` at `current_ctx` + `dotted_key` (dotted keys create
    /// intermediate tables), failing on redefinition.
    fn insert_keyval(
        &mut self,
        ctx: &[String],
        dotted: &[String],
        value: Toml,
    ) -> Result<()> {
        let line = self.line;
        let mut node = &mut self.root;
        for k in ctx {
            node = descend_or_create_table(node, k)
                .map_err(|m| corrupt(format!("toml: {m} at line {line}")))?;
            node = follow_last_array_element(node);
        }
        // Walk dotted-key prefix.
        for k in &dotted[..dotted.len() - 1] {
            node = descend_or_create_table(node, k)
                .map_err(|m| corrupt(format!("toml: {m} at line {line}")))?;
        }
        let key = &dotted[dotted.len() - 1];
        let table = node
            .as_table_mut()
            .ok_or_else(|| corrupt(format!("toml: cannot assign key into non-table at line {line}")))?;
        if table.iter().any(|(k, _)| k == key) {
            return Err(corrupt(format!("toml: duplicate key '{key}' at line {line}")));
        }
        table.push((key.clone(), value));
        Ok(())
    }

    // ---- keys ----

    /// Parse a dotted key path: `a.b.c`. Stops before `=`, `]`, or EOL.
    ///
    /// The number of segments is capped at [`MAX_TOML_DEPTH`]: a dotted path of
    /// length N descends (and, for inline tables, builds) an N-deep table chain,
    /// so an unbounded path would otherwise overflow the stack (in parsing AND in
    /// the recursive drop of the built structure). Capping here bounds every
    /// dotted-key form — table headers, bare key-values, and inline tables.
    fn parse_key_path(&mut self) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        loop {
            self.skip_inline_ws();
            let key = self.parse_key_component()?;
            keys.push(key);
            if keys.len() >= MAX_TOML_DEPTH {
                return Err(self.err("toml: dotted key path too deep"));
            }
            self.skip_inline_ws();
            if self.peek() == Some(b'.') {
                self.pos += 1;
                continue;
            }
            break;
        }
        Ok(keys)
    }

    fn parse_key_component(&mut self) -> Result<String> {
        match self.peek() {
            Some(b'"') => self.parse_basic_string(),
            Some(b'\'') => self.parse_literal_string(),
            Some(c) if is_bare_key_char(c) => {
                let start = self.pos;
                while matches!(self.peek(), Some(c) if is_bare_key_char(c)) {
                    self.pos += 1;
                }
                Ok(core::str::from_utf8(&self.bytes[start..self.pos])
                    .map_err(|_| self.err("invalid utf-8 in key"))?
                    .to_string())
            }
            _ => Err(self.err("expected key")),
        }
    }

    // ---- values ----

    fn parse_value(&mut self, depth: usize) -> Result<Toml> {
        match self.peek() {
            None => Err(self.err("expected value")),
            Some(b'"') => {
                if self.peek_at(1) == Some(b'"') && self.peek_at(2) == Some(b'"') {
                    self.parse_multiline_basic_string().map(Toml::Str)
                } else {
                    self.parse_basic_string().map(Toml::Str)
                }
            }
            Some(b'\'') => {
                if self.peek_at(1) == Some(b'\'') && self.peek_at(2) == Some(b'\'') {
                    self.parse_multiline_literal_string().map(Toml::Str)
                } else {
                    self.parse_literal_string().map(Toml::Str)
                }
            }
            Some(b'[') => self.parse_array(depth),
            Some(b'{') => self.parse_inline_table(depth),
            Some(b't') | Some(b'f') => self.parse_bool(),
            Some(_) => self.parse_number_or_datetime(),
        }
    }

    fn parse_bool(&mut self) -> Result<Toml> {
        if self.bytes[self.pos..].starts_with(b"true") {
            self.pos += 4;
            Ok(Toml::Bool(true))
        } else if self.bytes[self.pos..].starts_with(b"false") {
            self.pos += 5;
            Ok(Toml::Bool(false))
        } else {
            Err(self.err("invalid boolean"))
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<Toml> {
        let depth = self.enter(depth)?;
        self.pos += 1; // consume '['
        let mut items = Vec::new();
        loop {
            self.skip_array_ws();
            match self.peek() {
                None => return Err(self.err("unterminated array")),
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Toml::Array(items));
                }
                _ => {}
            }
            let v = self.parse_value(depth)?;
            items.push(v);
            self.skip_array_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Toml::Array(items));
                }
                _ => return Err(self.err("expected ',' or ']' in array")),
            }
        }
    }

    /// Inside arrays, newlines and comments are allowed between elements.
    fn skip_array_ws(&mut self) {
        loop {
            self.skip_inline_ws();
            match self.peek() {
                Some(b'\n') | Some(b'\r') => {
                    self.bump();
                }
                Some(b'#') => self.skip_comment(),
                _ => break,
            }
        }
    }

    fn parse_inline_table(&mut self, depth: usize) -> Result<Toml> {
        let depth = self.enter(depth)?;
        self.pos += 1; // consume '{'
        let mut table: Vec<(String, Toml)> = Vec::new();
        self.skip_inline_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Toml::Table(table));
        }
        loop {
            self.skip_inline_ws();
            let dotted = self.parse_key_path()?;
            self.skip_inline_ws();
            if self.peek() != Some(b'=') {
                return Err(self.err("expected '=' in inline table"));
            }
            self.pos += 1;
            self.skip_inline_ws();
            let value = self.parse_value(depth)?;
            insert_dotted_inline(&mut table, &dotted, value)
                .map_err(|m| corrupt(format!("toml: {m} at line {}", self.line)))?;
            self.skip_inline_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    continue;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Toml::Table(table));
                }
                _ => return Err(self.err("expected ',' or '}' in inline table")),
            }
        }
    }

    fn enter(&self, depth: usize) -> Result<usize> {
        if depth >= MAX_TOML_DEPTH {
            return Err(self.err("maximum nesting depth exceeded"));
        }
        Ok(depth + 1)
    }

    // ---- strings ----

    fn parse_basic_string(&mut self) -> Result<String> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            match self.peek() {
                None | Some(b'\n') => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    self.parse_basic_escape(&mut out)?;
                }
                Some(b) if b < 0x20 && b != b'\t' => {
                    return Err(self.err("control character in string"));
                }
                Some(b) => {
                    self.push_scalar(b, &mut out)?;
                }
            }
        }
    }

    fn parse_multiline_basic_string(&mut self) -> Result<String> {
        self.pos += 3; // opening """
        // A newline immediately after the opening delimiter is trimmed.
        self.trim_leading_newline();
        let mut out = String::new();
        loop {
            // Closing delimiter.
            if self.peek() == Some(b'"')
                && self.peek_at(1) == Some(b'"')
                && self.peek_at(2) == Some(b'"')
            {
                self.pos += 3;
                return Ok(out);
            }
            match self.peek() {
                None => return Err(self.err("unterminated multiline string")),
                Some(b'\\') => {
                    // Line-ending backslash: trim trailing ws + newline + ws.
                    if self.is_line_ending_backslash() {
                        self.pos += 1;
                        self.skip_ws_and_newlines();
                    } else {
                        self.pos += 1;
                        self.parse_basic_escape(&mut out)?;
                    }
                }
                Some(b'\n') => {
                    out.push('\n');
                    self.bump();
                }
                Some(b'\r') => {
                    // CRLF and bare CR both normalize to LF, matching the
                    // post-hoc `replace("\r\n", "\n")` in the literal-string
                    // form so multiline basic and literal strings agree on
                    // line endings.
                    self.pos += 1;
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                    out.push('\n');
                }
                Some(b) => {
                    self.push_scalar(b, &mut out)?;
                }
            }
        }
    }

    fn parse_literal_string(&mut self) -> Result<String> {
        self.pos += 1; // opening '
        let start = self.pos;
        loop {
            match self.peek() {
                None | Some(b'\n') => return Err(self.err("unterminated literal string")),
                Some(b'\'') => {
                    let s = core::str::from_utf8(&self.bytes[start..self.pos])
                        .map_err(|_| self.err("invalid utf-8 in literal string"))?
                        .to_string();
                    self.pos += 1;
                    return Ok(s);
                }
                Some(b) if b < 0x20 && b != b'\t' => {
                    return Err(self.err("control character in literal string"));
                }
                Some(_) => {
                    self.pos += 1;
                }
            }
        }
    }

    fn parse_multiline_literal_string(&mut self) -> Result<String> {
        self.pos += 3; // opening '''
        self.trim_leading_newline();
        let start = self.pos;
        loop {
            if self.peek() == Some(b'\'')
                && self.peek_at(1) == Some(b'\'')
                && self.peek_at(2) == Some(b'\'')
            {
                let s = core::str::from_utf8(&self.bytes[start..self.pos])
                    .map_err(|_| self.err("invalid utf-8 in literal string"))?
                    .replace("\r\n", "\n");
                self.pos += 3;
                return Ok(s);
            }
            if self.bump().is_none() {
                return Err(self.err("unterminated multiline literal string"));
            }
        }
    }

    fn parse_basic_escape(&mut self, out: &mut String) -> Result<()> {
        let e = match self.peek() {
            Some(e) => e,
            None => return Err(self.err("unterminated escape")),
        };
        self.pos += 1;
        match e {
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            b'b' => out.push('\u{0008}'),
            b'f' => out.push('\u{000c}'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'u' => self.push_unicode(4, out)?,
            b'U' => self.push_unicode(8, out)?,
            _ => return Err(self.err("invalid escape sequence")),
        }
        Ok(())
    }

    fn push_unicode(&mut self, n: usize, out: &mut String) -> Result<()> {
        if self.pos + n > self.bytes.len() {
            return Err(self.err("truncated unicode escape"));
        }
        let mut v: u32 = 0;
        for _ in 0..n {
            let b = self.bytes[self.pos];
            let d = match b {
                b'0'..=b'9' => (b - b'0') as u32,
                b'a'..=b'f' => (b - b'a' + 10) as u32,
                b'A'..=b'F' => (b - b'A' + 10) as u32,
                _ => return Err(self.err("invalid hex digit in unicode escape")),
            };
            v = (v << 4) | d;
            self.pos += 1;
        }
        match char::from_u32(v) {
            Some(c) => {
                out.push(c);
                Ok(())
            }
            None => Err(self.err("invalid unicode scalar in escape")),
        }
    }

    fn push_scalar(&mut self, b: u8, out: &mut String) -> Result<()> {
        let len = utf8_len(b);
        let end = self.pos + len;
        if end > self.bytes.len() {
            return Err(self.err("truncated utf-8"));
        }
        match core::str::from_utf8(&self.bytes[self.pos..end]) {
            Ok(s) => out.push_str(s),
            Err(_) => return Err(self.err("invalid utf-8")),
        }
        self.pos = end;
        Ok(())
    }

    /// True if a backslash at `pos` is followed only by whitespace then a newline.
    fn is_line_ending_backslash(&self) -> bool {
        let mut i = self.pos + 1;
        while matches!(self.bytes.get(i), Some(b' ' | b'\t' | b'\r')) {
            i += 1;
        }
        matches!(self.bytes.get(i), Some(b'\n'))
    }

    fn skip_ws_and_newlines(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.bump();
        }
    }

    fn trim_leading_newline(&mut self) {
        if self.peek() == Some(b'\r') && self.peek_at(1) == Some(b'\n') {
            self.pos += 2;
            self.line += 1;
        } else if self.peek() == Some(b'\n') {
            self.bump();
        }
    }

    // ---- numbers / datetimes / bools ----

    fn parse_number_or_datetime(&mut self) -> Result<Toml> {
        let start = self.pos;
        // Collect the token up to a delimiter.
        while let Some(b) = self.peek() {
            if matches!(b, b',' | b']' | b'}' | b'\n' | b'\r' | b'#') || b == b' ' || b == b'\t' {
                break;
            }
            self.pos += 1;
        }
        let raw = core::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| self.err("invalid utf-8 in value"))?;
        let token = raw.trim();
        if token.is_empty() {
            return Err(self.err("expected value"));
        }
        classify_scalar(token).ok_or_else(|| self.err("invalid value"))
    }

    fn expect_eol(&mut self) -> Result<()> {
        self.skip_inline_ws();
        match self.peek() {
            None => Ok(()),
            Some(b'\n') => {
                self.bump();
                Ok(())
            }
            Some(b'\r') => {
                self.bump();
                Ok(())
            }
            Some(b'#') => {
                self.skip_comment();
                self.expect_eol()
            }
            _ => Err(self.err("unexpected trailing content on line")),
        }
    }
}

/// Descend into `node[key]`, creating an empty table there if absent.
///
/// The caller is responsible for following an array-of-tables slot down to its
/// last element via [`follow_last_array_element`] when appropriate. Returns an
/// error string when `node` is not a table.
fn descend_or_create_table<'n>(
    node: &'n mut Toml,
    key: &str,
) -> core::result::Result<&'n mut Toml, String> {
    let table = match node {
        Toml::Table(t) => t,
        _ => return Err(format!("cannot index into non-table at key '{key}'")),
    };
    match table.iter().position(|(k, _)| k == key) {
        Some(idx) => Ok(&mut table[idx].1),
        None => {
            table.push((key.to_string(), Toml::Table(Vec::new())));
            let last = table.len() - 1;
            Ok(&mut table[last].1)
        }
    }
}

/// If `node` is an array-of-tables, return its last element; else `node`.
fn follow_last_array_element(node: &mut Toml) -> &mut Toml {
    // Decide up front whether to descend, to keep the borrow checker happy.
    let descend = match node {
        Toml::Array(arr) => matches!(arr.last(), Some(Toml::Table(_))),
        _ => false,
    };
    if descend {
        if let Toml::Array(arr) = node {
            let n = arr.len() - 1;
            return &mut arr[n];
        }
    }
    node
}

/// Insert a (possibly dotted) key into an inline-table being built.
///
/// Iterative (walks the slot chain with a loop) rather than recursive, so a
/// long dotted key cannot overflow the stack regardless of the parser's depth
/// cap — `parse_key_path` already bounds the segment count, and this keeps the
/// builder safe independently.
fn insert_dotted_inline(
    table: &mut Vec<(String, Toml)>,
    dotted: &[String],
    value: Toml,
) -> core::result::Result<(), String> {
    debug_assert!(!dotted.is_empty());
    let mut current = table;
    for (i, key) in dotted.iter().enumerate() {
        let is_last = i == dotted.len() - 1;
        if is_last {
            if current.iter().any(|(k, _)| k == key) {
                return Err(format!("duplicate key '{key}' in inline table"));
            }
            current.push((key.clone(), value));
            return Ok(());
        }
        // Descend (creating an intermediate table if absent).
        let idx = match current.iter().position(|(k, _)| k == key) {
            Some(idx) => idx,
            None => {
                current.push((key.clone(), Toml::Table(Vec::new())));
                current.len() - 1
            }
        };
        current = match &mut current[idx].1 {
            Toml::Table(inner) => inner,
            _ => return Err(format!("dotted key '{key}' conflicts with a value")),
        };
    }
    Ok(())
}

/// Classify a bare scalar token (no strings/arrays/inline tables here).
fn classify_scalar(token: &str) -> Option<Toml> {
    match token {
        "true" => return Some(Toml::Bool(true)),
        "false" => return Some(Toml::Bool(false)),
        "inf" | "+inf" => return Some(Toml::Float(f64::INFINITY)),
        "-inf" => return Some(Toml::Float(f64::NEG_INFINITY)),
        "nan" | "+nan" | "-nan" => return Some(Toml::Float(f64::NAN)),
        _ => {}
    }
    // Datetime heuristic: contains '-' past index 0, or ':' (time), or 'T'/'Z'.
    if looks_like_datetime(token) {
        return Some(Toml::Datetime(token.to_string()));
    }
    parse_toml_integer(token)
        .map(Toml::Int)
        .or_else(|| parse_toml_float(token).map(Toml::Float))
}

fn looks_like_datetime(token: &str) -> bool {
    // YYYY-MM-DD or contains time separator. Avoid catching plain ints/floats
    // or negative numbers (a leading '-' is sign, not a date dash).
    let bytes = token.as_bytes();
    if bytes.len() >= 10
        && bytes[0].is_ascii_digit()
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[1].is_ascii_digit()
    {
        return true;
    }
    // Local time HH:MM:SS
    if bytes.len() >= 8 && bytes[2] == b':' && bytes[5] == b':' {
        return true;
    }
    false
}

/// Parse a TOML integer (decimal with optional `_` separators and sign, or
/// `0x`/`0o`/`0b` prefixed). Returns `None` if not an integer.
fn parse_toml_integer(token: &str) -> Option<i64> {
    let (radix, body, neg) = if let Some(rest) = token.strip_prefix("0x") {
        (16, rest, false)
    } else if let Some(rest) = token.strip_prefix("0o") {
        (8, rest, false)
    } else if let Some(rest) = token.strip_prefix("0b") {
        (2, rest, false)
    } else {
        let (neg, body) = if let Some(r) = token.strip_prefix('-') {
            (true, r)
        } else if let Some(r) = token.strip_prefix('+') {
            (false, r)
        } else {
            (false, token)
        };
        (10, body, neg)
    };
    if body.is_empty() {
        return None;
    }
    // Underscores must be between digits; reject leading/trailing/double.
    let cleaned = clean_underscores(body)?;
    let magnitude = i64::from_str_radix(&cleaned, radix).ok()?;
    if neg {
        magnitude.checked_neg()
    } else {
        Some(magnitude)
    }
}

fn parse_toml_float(token: &str) -> Option<f64> {
    // Floats may contain underscores between digits; strip after validating.
    // A valid float must contain '.', 'e', or 'E' (already handled inf/nan).
    if !token.contains('.') && !token.contains('e') && !token.contains('E') {
        return None;
    }
    let cleaned = clean_float_underscores(token)?;
    cleaned.parse::<f64>().ok()
}

/// Remove `_` separators from an integer body, rejecting malformed placement.
fn clean_underscores(body: &str) -> Option<String> {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut prev_digit = false;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'_' {
            if !prev_digit || i + 1 >= bytes.len() || !bytes[i + 1].is_ascii_alphanumeric() {
                return None;
            }
            prev_digit = false;
            continue;
        }
        if !b.is_ascii_alphanumeric() {
            return None;
        }
        out.push(b as char);
        prev_digit = true;
    }
    Some(out)
}

/// Like [`clean_underscores`] but also permits `.`, `e`, `E`, `+`, `-`.
fn clean_float_underscores(body: &str) -> Option<String> {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'_' {
            let prev_digit = i > 0 && bytes[i - 1].is_ascii_digit();
            let next_digit = i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit();
            if !prev_digit || !next_digit {
                return None;
            }
            continue;
        }
        out.push(b as char);
    }
    Some(out)
}

fn is_bare_key_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

/// Length in bytes of the UTF-8 scalar whose first byte is `b`.
fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_and_types() {
        let t = parse_toml(
            r#"
            name = "prism"
            count = 42
            ratio = 2.5
            neg = -7
            big = 1_000_000
            hex = 0xFF
            yes = true
            no = false
            "#,
        )
        .unwrap();
        assert_eq!(t.get("name").and_then(Toml::as_str), Some("prism"));
        assert_eq!(t.get("count").and_then(Toml::as_i64), Some(42));
        assert_eq!(t.get("ratio").and_then(Toml::as_f64), Some(2.5));
        assert_eq!(t.get("neg").and_then(Toml::as_i64), Some(-7));
        assert_eq!(t.get("big").and_then(Toml::as_i64), Some(1_000_000));
        assert_eq!(t.get("hex").and_then(Toml::as_i64), Some(255));
        assert_eq!(t.get("yes").and_then(Toml::as_bool), Some(true));
        assert_eq!(t.get("no").and_then(Toml::as_bool), Some(false));
    }

    #[test]
    fn strings_basic_literal_multiline() {
        let t = parse_toml(
            "a = \"esc\\t\\n\\\"x\"\n\
             b = 'literal \\n nope'\n\
             c = \"\"\"\nmulti\nline\"\"\"\n\
             d = '''raw\nmulti'''\n",
        )
        .unwrap();
        assert_eq!(t.get("a").and_then(Toml::as_str), Some("esc\t\n\"x"));
        assert_eq!(t.get("b").and_then(Toml::as_str), Some("literal \\n nope"));
        assert_eq!(t.get("c").and_then(Toml::as_str), Some("multi\nline"));
        assert_eq!(t.get("d").and_then(Toml::as_str), Some("raw\nmulti"));
    }

    #[test]
    fn line_ending_backslash_in_multiline() {
        let t = parse_toml("a = \"\"\"one \\\n   two\"\"\"\n").unwrap();
        assert_eq!(t.get("a").and_then(Toml::as_str), Some("one two"));
    }

    #[test]
    fn multiline_basic_normalizes_crlf_and_bare_cr() {
        // CRLF normalized to LF in multiline BASIC strings (basic and literal
        // forms now agree on line-ending semantics).
        let t = parse_toml("a = \"\"\"one\r\ntwo\"\"\"\n").unwrap();
        assert_eq!(t.get("a").and_then(Toml::as_str), Some("one\ntwo"));
        // Bare CR also normalizes to LF (previously dropped silently, producing
        // "onetwo" — a data-loss bug on documents authored on classic-Mac line
        // endings or pasted through transports that strip the LF).
        let t = parse_toml("a = \"\"\"one\rtwo\"\"\"\n").unwrap();
        assert_eq!(t.get("a").and_then(Toml::as_str), Some("one\ntwo"));
    }

    #[test]
    fn tables_and_nesting() {
        let t = parse_toml(
            r#"
            [package]
            name = "demo"
            version = "0.1.0"

            [dependencies]
            serde = "1"

            [dependencies.tokio]
            version = "1.0"
            features = ["full", "macros"]
            optional = true
            "#,
        )
        .unwrap();
        assert_eq!(
            t.path(["package", "name"]).and_then(Toml::as_str),
            Some("demo")
        );
        assert_eq!(
            t.path(["dependencies", "serde"]).and_then(Toml::as_str),
            Some("1")
        );
        let tokio = t.path(["dependencies", "tokio"]).unwrap();
        assert_eq!(tokio.get("version").and_then(Toml::as_str), Some("1.0"));
        assert_eq!(tokio.get("optional").and_then(Toml::as_bool), Some(true));
        let feats = tokio.get("features").and_then(Toml::as_array).unwrap();
        assert_eq!(feats.len(), 2);
        assert_eq!(feats[0].as_str(), Some("full"));
    }

    #[test]
    fn dotted_keys() {
        let t = parse_toml("a.b.c = 1\na.b.d = 2\n").unwrap();
        assert_eq!(t.path(["a", "b", "c"]).and_then(Toml::as_i64), Some(1));
        assert_eq!(t.path(["a", "b", "d"]).and_then(Toml::as_i64), Some(2));
    }

    #[test]
    fn inline_tables() {
        let t = parse_toml(r#"dep = { version = "1.0", features = ["x"], optional = true }"#)
            .unwrap();
        let dep = t.get("dep").unwrap();
        assert_eq!(dep.get("version").and_then(Toml::as_str), Some("1.0"));
        assert_eq!(dep.get("optional").and_then(Toml::as_bool), Some(true));
        assert_eq!(
            dep.get("features").and_then(Toml::as_array).map(|a| a.len()),
            Some(1)
        );
    }

    #[test]
    fn inline_table_dotted_keys() {
        let t = parse_toml(r#"x = { a.b = 1, a.c = 2 }"#).unwrap();
        let x = t.get("x").unwrap();
        assert_eq!(x.path(["a", "b"]).and_then(Toml::as_i64), Some(1));
        assert_eq!(x.path(["a", "c"]).and_then(Toml::as_i64), Some(2));
    }

    #[test]
    fn nested_arrays() {
        let t = parse_toml("m = [[1, 2], [3, 4], []]\n").unwrap();
        let m = t.get("m").and_then(Toml::as_array).unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!(m[1].index(0).and_then(Toml::as_i64), Some(3));
        assert_eq!(m[2].as_array().map(|a| a.len()), Some(0));
    }

    #[test]
    fn array_with_newlines_and_comments() {
        let t = parse_toml(
            "a = [\n  1, # one\n  2,\n  3,\n]\n",
        )
        .unwrap();
        let a = t.get("a").and_then(Toml::as_array).unwrap();
        assert_eq!(a.len(), 3);
    }

    #[test]
    fn array_of_tables() {
        let t = parse_toml(
            r#"
            [[package]]
            name = "a"
            version = "1.0.0"

            [[package]]
            name = "b"
            version = "2.0.0"
            dependencies = ["a"]
            "#,
        )
        .unwrap();
        let pkgs = t.get("package").and_then(Toml::as_array).unwrap();
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].get("name").and_then(Toml::as_str), Some("a"));
        assert_eq!(pkgs[1].get("name").and_then(Toml::as_str), Some("b"));
        let deps = pkgs[1].get("dependencies").and_then(Toml::as_array).unwrap();
        assert_eq!(deps[0].as_str(), Some("a"));
    }

    #[test]
    fn subtable_under_array_of_tables() {
        let t = parse_toml(
            r#"
            [[fruit]]
            name = "apple"

            [fruit.physical]
            color = "red"
            "#,
        )
        .unwrap();
        let fruit = t.get("fruit").and_then(Toml::as_array).unwrap();
        assert_eq!(fruit.len(), 1);
        assert_eq!(
            fruit[0].path(["physical", "color"]).and_then(Toml::as_str),
            Some("red")
        );
    }

    #[test]
    fn real_cargo_toml() {
        let src = r#"
            [package]
            name = "prism"
            version = "0.1.0"
            edition = "2021"
            authors = ["Someone <a@b.c>"]

            [dependencies]
            serde = { version = "1", features = ["derive"], optional = true }
            log = "0.4"

            [dependencies.tokio]
            version = "1"
            features = ["full"]

            [features]
            default = ["std"]
            std = []
            extra = ["serde"]

            [target.'cfg(unix)'.dependencies]
            libc = "0.2"

            [target."cfg(windows)".dependencies]
            winapi = { version = "0.3" }
        "#;
        let t = parse_toml(src).unwrap();
        assert_eq!(t.path(["package", "name"]).and_then(Toml::as_str), Some("prism"));
        assert_eq!(t.path(["package", "edition"]).and_then(Toml::as_str), Some("2021"));
        let serde = t.path(["dependencies", "serde"]).unwrap();
        assert_eq!(serde.get("optional").and_then(Toml::as_bool), Some(true));
        assert_eq!(
            serde.get("features").and_then(Toml::as_array).and_then(|a| a.first()).and_then(Toml::as_str),
            Some("derive")
        );
        assert_eq!(
            t.path(["dependencies", "tokio", "version"]).and_then(Toml::as_str),
            Some("1")
        );
        assert_eq!(
            t.path(["features", "default"]).and_then(Toml::as_array).and_then(|a| a.first()).and_then(Toml::as_str),
            Some("std")
        );
        // Quoted cfg keys in target table headers.
        assert_eq!(
            t.path(["target", "cfg(unix)", "dependencies", "libc"]).and_then(Toml::as_str),
            Some("0.2")
        );
        assert_eq!(
            t.path(["target", "cfg(windows)", "dependencies", "winapi", "version"]).and_then(Toml::as_str),
            Some("0.3")
        );
    }

    #[test]
    fn real_cargo_lock() {
        let src = r#"
            # This file is automatically @generated by Cargo.
            version = 3

            [[package]]
            name = "prism"
            version = "0.1.0"
            dependencies = [
             "serde",
             "tokio",
            ]

            [[package]]
            name = "serde"
            version = "1.0.197"
            source = "registry+https://github.com/rust-lang/crates.io-index"
            checksum = "3fb1c873e1b9b056a4dc4c0c198b9c0a5b0a3a1b0c9d8e7f6a5b4c3d2e1f0a9b"

            [[package]]
            name = "tokio"
            version = "1.36.0"
            source = "registry+https://github.com/rust-lang/crates.io-index"
            dependencies = [
             "serde",
            ]
        "#;
        let t = parse_toml(src).unwrap();
        assert_eq!(t.get("version").and_then(Toml::as_i64), Some(3));
        let pkgs = t.get("package").and_then(Toml::as_array).unwrap();
        assert_eq!(pkgs.len(), 3);
        assert_eq!(pkgs[0].get("name").and_then(Toml::as_str), Some("prism"));
        let deps = pkgs[0].get("dependencies").and_then(Toml::as_array).unwrap();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].as_str(), Some("serde"));
        assert_eq!(pkgs[1].get("name").and_then(Toml::as_str), Some("serde"));
        assert!(pkgs[1]
            .get("source")
            .and_then(Toml::as_str)
            .unwrap()
            .starts_with("registry+"));
        assert_eq!(pkgs[1].get("checksum").and_then(Toml::as_str).map(|s| s.len()), Some(64));
    }

    #[test]
    fn datetime_passthrough() {
        let t = parse_toml("d = 2024-01-02T03:04:05Z\nld = 2024-01-02\n").unwrap();
        assert!(matches!(t.get("d"), Some(Toml::Datetime(_))));
        assert_eq!(
            t.get("d").map(|v| matches!(v, Toml::Datetime(s) if s == "2024-01-02T03:04:05Z")),
            Some(true)
        );
        assert!(matches!(t.get("ld"), Some(Toml::Datetime(_))));
    }

    #[test]
    fn comments_everywhere() {
        let t = parse_toml(
            "# top comment\n\
             a = 1 # trailing\n\
             # standalone\n\
             [t] # header comment\n\
             b = 2\n",
        )
        .unwrap();
        assert_eq!(t.get("a").and_then(Toml::as_i64), Some(1));
        assert_eq!(t.path(["t", "b"]).and_then(Toml::as_i64), Some(2));
    }

    #[test]
    fn error_cases_not_panics() {
        assert!(parse_toml("a = ").is_err()); // missing value
        assert!(parse_toml("a = \"unterminated\n").is_err());
        assert!(parse_toml("= 1\n").is_err()); // missing key
        assert!(parse_toml("[unclosed\n").is_err()); // bad header
        assert!(parse_toml("a = 1\na = 2\n").is_err()); // duplicate key
        assert!(parse_toml("a = [1, 2\n").is_err()); // unterminated array
        assert!(parse_toml("a = { b = 1\n").is_err()); // unterminated inline table
        assert!(parse_toml("a = truex\n").is_err()); // invalid bool token
        assert!(parse_toml("a 1\n").is_err()); // missing '='
        assert!(parse_toml("[]\n").is_err()); // empty table header
    }

    #[test]
    fn quoted_keys() {
        let t = parse_toml("\"key.with.dots\" = 1\n'literal key' = 2\n").unwrap();
        assert_eq!(t.get("key.with.dots").and_then(Toml::as_i64), Some(1));
        assert_eq!(t.get("literal key").and_then(Toml::as_i64), Some(2));
    }

    #[test]
    fn deep_nesting_is_graceful_error_not_overflow() {
        let n = MAX_TOML_DEPTH + 50;
        let mut s = String::from("a = ");
        for _ in 0..n {
            s.push('[');
        }
        for _ in 0..n {
            s.push(']');
        }
        s.push('\n');
        let err = parse_toml(&s).unwrap_err();
        assert!(matches!(err, PrismError::Corrupt { .. }));

        // Deeply nested inline tables, too.
        let mut o = String::from("a = ");
        for _ in 0..n {
            o.push_str("{ b = ");
        }
        o.push('1');
        for _ in 0..n {
            o.push_str(" }");
        }
        o.push('\n');
        assert!(parse_toml(&o).is_err());
    }

    #[test]
    fn deep_dotted_key_is_graceful_error_not_overflow() {
        // The dotted-key chain (inline-table, table-header, and bare key-val
        // forms) must be depth-bounded — an unbounded chain previously overflowed
        // the stack. Run on a deliberately small-stack thread so a regression
        // aborts here as a thread panic rather than passing by luck on a big stack.
        let n = MAX_TOML_DEPTH + 5000;
        let body = move || {
            let segs = (0..n).map(|i| format!("k{i}")).collect::<Vec<_>>().join(".");
            // inline-table dotted key: a = { k0.k1.....kN = 1 }
            assert!(matches!(
                parse_toml(&format!("a = {{ {segs} = 1 }}\n")),
                Err(PrismError::Corrupt { .. })
            ));
            // bare dotted key-val: k0.k1.....kN = 1
            assert!(matches!(
                parse_toml(&format!("{segs} = 1\n")),
                Err(PrismError::Corrupt { .. })
            ));
            // dotted table header: [k0.k1.....kN]
            assert!(matches!(
                parse_toml(&format!("[{segs}]\n")),
                Err(PrismError::Corrupt { .. })
            ));
        };
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(body)
            .unwrap()
            .join()
            .expect("must return a typed error, not overflow the stack");
    }

    #[test]
    fn at_depth_limit_ok() {
        let n = MAX_TOML_DEPTH;
        let mut s = String::from("a = ");
        for _ in 0..n {
            s.push('[');
        }
        s.push('1');
        for _ in 0..n {
            s.push(']');
        }
        s.push('\n');
        assert!(parse_toml(&s).is_ok());
    }

    #[test]
    fn empty_input_is_empty_table() {
        let t = parse_toml("").unwrap();
        assert_eq!(t, Toml::Table(vec![]));
        let t2 = parse_toml("\n  \n# only comments\n").unwrap();
        assert_eq!(t2, Toml::Table(vec![]));
    }
}

