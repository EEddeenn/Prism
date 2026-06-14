//! A facade-level argument bag for view dispatchers.
//!
//! [`Args`] is what [`crate::dispatch::dispatch`] hands every view handler:
//! a flat key/value map of typed accessors, populated either from a pre-parsed
//! key/value sequence (via [`Args::from_pairs`]) or directly from a
//! `{"k":v,...}` JSON object shipped over the WebAssembly boundary (via
//! [`Args::from_flat_json`]).
//!
//! The accessor set covers: string get, `u32`/`u64`, and a "default-when-
//! absent-but-still-fail-on-garbage" variant for optional bounds like
//! `depth`/`limit` where missing and malformed must differ.

use prism_model::PrismError;

/// A flat key/value bag — values are stored as strings and parsed on demand by
/// the typed accessors. Order is preserved (first-seen), so emitting a string
/// representation back out is deterministic.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    pairs: Vec<(String, String)>,
}

impl Args {
    /// A fresh, empty argument bag.
    pub fn new() -> Args {
        Args { pairs: Vec::new() }
    }

    /// Build an [`Args`] from an iterator of `(key, value)` strings. Useful
    /// for adapters that forward a pre-parsed key/value sequence into the
    /// facade. Duplicate keys keep the first value (stable first-seen
    /// semantics).
    pub fn from_pairs<I, K, V>(pairs: I) -> Args
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut out: Vec<(String, String)> = Vec::new();
        for (k, v) in pairs {
            let key = k.into();
            let val = v.into();
            if !out.iter().any(|(ek, _)| ek == &key) {
                out.push((key, val));
            }
        }
        Args { pairs: out }
    }

    /// Parse a *flat* JSON object — `{"key": value, ...}` — into an [`Args`].
    /// This is the parser the WebAssembly entry point calls, so it accepts:
    ///
    /// - Top-level object only (no top-level array/scalar).
    /// - JSON string values (with the usual `\"`, `\\`, `\/`, `\b`, `\f`,
    ///   `\n`, `\r`, `\t`, and `\uXXXX` escapes).
    /// - JSON number values (integer or float; whatever the caller wrote is
    ///   re-stringified into the bag, so typed accessors parse it again).
    /// - JSON `true` / `false` (stringified to `"true"` / `"false"`).
    /// - JSON `null` (stringified to `""` — same shape as a bare query key).
    /// - Trailing commas are rejected; standard JSON whitespace is skipped.
    ///
    /// No nested objects, no arrays — every value must be a primitive. The
    /// parser is panic-free on any input; failures return a
    /// [`PrismError::Other`] with a one-line diagnostic.
    pub fn from_flat_json(s: &str) -> Result<Args, PrismError> {
        Parser::new(s).parse_object()
    }

    /// The raw string value for `key`, if present.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// `key` parsed as a `u64`, or `None` if missing/non-numeric/empty.
    pub fn u64(&self, key: &str) -> Option<u64> {
        self.get(key).and_then(|v| v.parse::<u64>().ok())
    }

    /// `key` parsed as a `u32`, or `None` if missing/non-numeric/empty.
    pub fn u32(&self, key: &str) -> Option<u32> {
        self.get(key).and_then(|v| v.parse::<u32>().ok())
    }

    /// `key` parsed as a `u32`, defaulting to `default` when absent or empty,
    /// but failing (`None`) when present-but-unparseable. Used for optional
    /// bounds like `depth`/`limit` where "absent" and "garbage" must differ.
    pub fn u32_or(&self, key: &str, default: u32) -> Option<u32> {
        match self.get(key) {
            None => Some(default),
            Some("") => Some(default),
            Some(v) => v.parse::<u32>().ok(),
        }
    }
}

// ----- Flat-JSON parser --------------------------------------------------
//
// A hand-rolled recursive-descent parser over byte indices, panic-free on any
// input. Only accepts the shape `{"k":v,...}` with primitive values.

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Parser<'a> {
        Parser {
            bytes: s.as_bytes(),
            pos: 0,
        }
    }

    fn err<S: Into<String>>(msg: S) -> PrismError {
        PrismError::Other(format!("invalid args json: {}", msg.into()))
    }

    /// Skip JSON whitespace (space, tab, CR, LF).
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn expect(&mut self, b: u8) -> Result<(), PrismError> {
        match self.peek() {
            Some(c) if c == b => {
                self.pos += 1;
                Ok(())
            }
            Some(c) => Err(Self::err(format!(
                "expected '{}' at byte {}, got '{}'",
                b as char, self.pos, c as char
            ))),
            None => Err(Self::err(format!(
                "expected '{}' at byte {}, got end of input",
                b as char, self.pos
            ))),
        }
    }

    fn parse_object(mut self) -> Result<Args, PrismError> {
        self.skip_ws();
        self.expect(b'{')?;
        self.skip_ws();
        let mut pairs: Vec<(String, String)> = Vec::new();
        // Empty object?
        if self.peek() == Some(b'}') {
            self.pos += 1;
            self.skip_ws();
            if self.pos != self.bytes.len() {
                return Err(Self::err(format!(
                    "trailing bytes at position {}",
                    self.pos
                )));
            }
            return Ok(Args { pairs });
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let val = self.parse_value()?;
            if !pairs.iter().any(|(ek, _): &(String, String)| ek == &key) {
                pairs.push((key, val));
            }
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                    // Reject trailing comma `{,}` and `{"k":1,}`.
                    if self.peek() == Some(b'}') {
                        return Err(Self::err("trailing comma before '}'"));
                    }
                }
                Some(b'}') => {
                    self.pos += 1;
                    self.skip_ws();
                    if self.pos != self.bytes.len() {
                        return Err(Self::err(format!(
                            "trailing bytes at position {}",
                            self.pos
                        )));
                    }
                    return Ok(Args { pairs });
                }
                Some(c) => {
                    return Err(Self::err(format!(
                        "expected ',' or '}}' at byte {}, got '{}'",
                        self.pos, c as char
                    )));
                }
                None => return Err(Self::err("unterminated object")),
            }
        }
    }

    /// Parse a JSON string token (the opening `"` must be at `self.pos`).
    fn parse_string(&mut self) -> Result<String, PrismError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let b = self
                .peek()
                .ok_or_else(|| Self::err("unterminated string"))?;
            self.pos += 1;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self
                        .peek()
                        .ok_or_else(|| Self::err("unterminated escape"))?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let cp = self.parse_u_escape()?;
                            // Surrogate-pair handling: a high surrogate must
                            // be followed by `\uXXXX` carrying a low surrogate.
                            if (0xD800..=0xDBFF).contains(&cp) {
                                if self.peek() == Some(b'\\') {
                                    self.pos += 1;
                                    let next = self
                                        .peek()
                                        .ok_or_else(|| Self::err("unterminated escape"))?;
                                    if next != b'u' {
                                        return Err(Self::err("expected '\\u' after high surrogate"));
                                    }
                                    self.pos += 1;
                                    let lo = self.parse_u_escape()?;
                                    if !(0xDC00..=0xDFFF).contains(&lo) {
                                        return Err(Self::err(
                                            "invalid low surrogate after high surrogate",
                                        ));
                                    }
                                    let combined: u32 =
                                        0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                    let ch = char::from_u32(combined).ok_or_else(|| {
                                        Self::err("surrogate pair is not a valid scalar")
                                    })?;
                                    out.push(ch);
                                } else {
                                    return Err(Self::err("lone high surrogate"));
                                }
                            } else if (0xDC00..=0xDFFF).contains(&cp) {
                                return Err(Self::err("lone low surrogate"));
                            } else {
                                let ch = char::from_u32(cp)
                                    .ok_or_else(|| Self::err("invalid '\\u' escape"))?;
                                out.push(ch);
                            }
                        }
                        other => {
                            return Err(Self::err(format!(
                                "invalid escape '\\{}'",
                                other as char
                            )));
                        }
                    }
                }
                c if c < 0x20 => {
                    return Err(Self::err(format!(
                        "control byte 0x{:02x} in string",
                        c
                    )));
                }
                _ => {
                    // ASCII fast path: just push.
                    if b < 0x80 {
                        out.push(b as char);
                    } else {
                        // Multi-byte UTF-8: rewind one and decode.
                        self.pos -= 1;
                        let start = self.pos;
                        // Walk forward until next `"` or `\` while staying on
                        // valid UTF-8 by relying on `str::from_utf8`.
                        // Find the next byte that is `"` or `\` to bound the slice.
                        let mut end = start;
                        while end < self.bytes.len() {
                            let bb = self.bytes[end];
                            if bb == b'"' || bb == b'\\' {
                                break;
                            }
                            if bb < 0x20 {
                                break;
                            }
                            end += 1;
                        }
                        let chunk = std::str::from_utf8(&self.bytes[start..end])
                            .map_err(|_| Self::err("invalid utf-8 in string"))?;
                        out.push_str(chunk);
                        self.pos = end;
                    }
                }
            }
        }
    }

    /// Parse a 4-hex-digit `\uXXXX` escape (the `\u` has already been consumed),
    /// returning the raw `u32` code point so callers can handle surrogates.
    fn parse_u_escape(&mut self) -> Result<u32, PrismError> {
        let mut acc: u32 = 0;
        for _ in 0..4 {
            let d = self
                .peek()
                .ok_or_else(|| Self::err("truncated '\\u' escape"))?;
            self.pos += 1;
            let nib = match d {
                b'0'..=b'9' => (d - b'0') as u32,
                b'a'..=b'f' => (d - b'a' + 10) as u32,
                b'A'..=b'F' => (d - b'A' + 10) as u32,
                _ => return Err(Self::err("non-hex digit in '\\u' escape")),
            };
            acc = (acc << 4) | nib;
        }
        Ok(acc)
    }

    /// Parse a primitive value (string / number / bool / null) into the
    /// string form stored in the bag.
    fn parse_value(&mut self) -> Result<String, PrismError> {
        match self
            .peek()
            .ok_or_else(|| Self::err("expected value, got end of input"))?
        {
            b'"' => self.parse_string(),
            b't' => self.match_literal(b"true", "true".to_string()),
            b'f' => self.match_literal(b"false", "false".to_string()),
            b'n' => self.match_literal(b"null", String::new()),
            b'-' | b'0'..=b'9' => self.parse_number(),
            c => Err(Self::err(format!(
                "expected primitive value at byte {}, got '{}'",
                self.pos, c as char
            ))),
        }
    }

    fn match_literal(&mut self, lit: &[u8], out: String) -> Result<String, PrismError> {
        let end = self.pos.checked_add(lit.len()).ok_or_else(|| {
            Self::err("arithmetic overflow while matching literal")
        })?;
        if end > self.bytes.len() || &self.bytes[self.pos..end] != lit {
            return Err(Self::err(format!(
                "expected literal {:?} at byte {}",
                std::str::from_utf8(lit).unwrap_or(""),
                self.pos
            )));
        }
        self.pos = end;
        Ok(out)
    }

    fn parse_number(&mut self) -> Result<String, PrismError> {
        let start = self.pos;
        // Optional leading minus.
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        // Integer part: at least one digit. Leading zero forbids more digits.
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                while let Some(b'0'..=b'9') = self.peek() {
                    self.pos += 1;
                }
            }
            _ => return Err(Self::err("invalid number: missing integer part")),
        }
        // Optional fraction.
        if self.peek() == Some(b'.') {
            self.pos += 1;
            let frac_start = self.pos;
            while let Some(b'0'..=b'9') = self.peek() {
                self.pos += 1;
            }
            if self.pos == frac_start {
                return Err(Self::err("invalid number: missing fractional digits"));
            }
        }
        // Optional exponent.
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let exp_start = self.pos;
            while let Some(b'0'..=b'9') = self.peek() {
                self.pos += 1;
            }
            if self.pos == exp_start {
                return Err(Self::err("invalid number: missing exponent digits"));
            }
        }
        let slice = &self.bytes[start..self.pos];
        let s = std::str::from_utf8(slice)
            .map_err(|_| Self::err("number is not valid utf-8"))?;
        Ok(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_object() {
        let a = Args::from_flat_json("{}").unwrap();
        assert_eq!(a.get("x"), None);
    }

    #[test]
    fn primitives_and_typed_access() {
        let a = Args::from_flat_json(
            r#"{"run":7,"node":42,"depth":3,"name":"hello","flag":true,"missing":null}"#,
        )
        .unwrap();
        assert_eq!(a.u64("run"), Some(7));
        assert_eq!(a.u32("node"), Some(42));
        assert_eq!(a.u32_or("depth", 64), Some(3));
        assert_eq!(a.get("name"), Some("hello"));
        assert_eq!(a.get("flag"), Some("true"));
        // null serialises as "" — same as a bare query key.
        assert_eq!(a.get("missing"), Some(""));
    }

    #[test]
    fn defaulted_distinguishes_absent_from_garbage() {
        let a = Args::from_flat_json(r#"{"depth":""}"#).unwrap();
        // empty string -> default
        assert_eq!(a.u32_or("depth", 2), Some(2));
        // absent -> default
        assert_eq!(a.u32_or("limit", 5), Some(5));
    }

    #[test]
    fn rejects_arrays_and_objects_as_values() {
        assert!(Args::from_flat_json(r#"{"x":[1,2]}"#).is_err());
        assert!(Args::from_flat_json(r#"{"x":{"y":1}}"#).is_err());
    }

    #[test]
    fn rejects_non_object_top_level() {
        assert!(Args::from_flat_json("[]").is_err());
        assert!(Args::from_flat_json("42").is_err());
        assert!(Args::from_flat_json("\"x\"").is_err());
    }

    #[test]
    fn rejects_trailing_comma_and_garbage_tail() {
        assert!(Args::from_flat_json(r#"{"x":1,}"#).is_err());
        assert!(Args::from_flat_json(r#"{"x":1}garbage"#).is_err());
    }

    #[test]
    fn duplicate_key_keeps_first() {
        let a = Args::from_flat_json(r#"{"run":1,"run":2}"#).unwrap();
        assert_eq!(a.u64("run"), Some(1));
    }

    #[test]
    fn handles_string_escapes() {
        let a = Args::from_flat_json(
            r#"{"s":"a\"b\\c\nd\te","u":"A"}"#,
        )
        .unwrap();
        assert_eq!(a.get("s"), Some("a\"b\\c\nd\te"));
        assert_eq!(a.get("u"), Some("A"));
    }

    #[test]
    fn handles_surrogate_pair() {
        // 😀 is the emoji 😀 (U+1F600).
        let a = Args::from_flat_json(r#"{"e":"😀"}"#).unwrap();
        assert_eq!(a.get("e"), Some("😀"));
    }

    #[test]
    fn rejects_control_char_in_string() {
        let bad = format!("{{\"x\":\"a{}b\"}}", '\u{01}');
        assert!(Args::from_flat_json(&bad).is_err());
    }

    #[test]
    fn from_pairs_dedupes_first_wins() {
        let a = Args::from_pairs(vec![("run", "1"), ("run", "2")]);
        assert_eq!(a.u64("run"), Some(1));
    }

    #[test]
    fn negative_integer_serialises_as_string_form() {
        let a = Args::from_flat_json(r#"{"x":-5}"#).unwrap();
        // u32/u64 parses reject negatives.
        assert_eq!(a.u32("x"), None);
        assert_eq!(a.get("x"), Some("-5"));
    }

    #[test]
    fn float_value_round_trips_to_string() {
        let a = Args::from_flat_json(r#"{"x":1.5}"#).unwrap();
        assert_eq!(a.get("x"), Some("1.5"));
    }

    #[test]
    fn parse_does_not_panic_on_truncated_input() {
        // A grab-bag of pathological prefixes that must all error cleanly.
        for input in [
            "",
            "{",
            "{\"k\"",
            "{\"k\":",
            "{\"k\":1",
            "{\"k\":1,",
            "{\"k\":tr",
            "{\"k\":\"abc",
            "{\"k\":\"\\u00",
            "{\"k\":\"\\u00ZZ\"}",
            "{\"k\":1.}",
            "{\"k\":1e}",
        ] {
            assert!(Args::from_flat_json(input).is_err(), "input was {input:?}");
        }
    }
}
