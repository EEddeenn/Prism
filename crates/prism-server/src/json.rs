//! A tiny hand-rolled JSON string builder (no external crates).
//!
//! Mirrors `prism-cli`'s writer: object/array nesting, string/number/bool
//! fields, and RFC 8259 string escaping. Output is compact (no incidental
//! whitespace) and deterministic in field order. Used to serialize the facade's
//! views and `{"error": ...}` bodies.

/// A growable JSON document built field-by-field.
#[derive(Default)]
pub struct JsonWriter {
    buf: String,
    /// True once the current container has at least one element (so the next
    /// element gets a leading comma).
    needs_comma: Vec<bool>,
}

impl JsonWriter {
    /// A fresh, empty writer.
    pub fn new() -> Self {
        JsonWriter {
            buf: String::new(),
            needs_comma: Vec::new(),
        }
    }

    /// Consume the writer, returning the JSON text.
    pub fn finish(self) -> String {
        self.buf
    }

    fn sep(&mut self) {
        if let Some(last) = self.needs_comma.last_mut() {
            if *last {
                self.buf.push(',');
            } else {
                *last = true;
            }
        }
    }

    /// Open an object `{`.
    pub fn begin_object(&mut self) {
        self.sep();
        self.buf.push('{');
        self.needs_comma.push(false);
    }

    /// Close an object `}`.
    pub fn end_object(&mut self) {
        self.buf.push('}');
        self.needs_comma.pop();
    }

    /// Open an array `[`.
    pub fn begin_array(&mut self) {
        self.sep();
        self.buf.push('[');
        self.needs_comma.push(false);
    }

    /// Close an array `]`.
    pub fn end_array(&mut self) {
        self.buf.push(']');
        self.needs_comma.pop();
    }

    /// Write an object key (followed by `:`). Call between `begin_object` and the
    /// value method.
    pub fn key(&mut self, name: &str) {
        self.sep();
        write_escaped(&mut self.buf, name);
        self.buf.push(':');
        // The value that follows is the *first* token after a key, so it must not
        // emit its own separator comma.
        if let Some(last) = self.needs_comma.last_mut() {
            *last = false;
        }
    }

    /// Write a string value (escaped).
    pub fn str_value(&mut self, s: &str) {
        self.sep();
        write_escaped(&mut self.buf, s);
    }

    /// Write an unsigned integer value.
    pub fn u64_value(&mut self, n: u64) {
        self.sep();
        self.buf.push_str(&n.to_string());
    }

    /// Write a boolean value.
    pub fn bool_value(&mut self, b: bool) {
        self.sep();
        self.buf.push_str(if b { "true" } else { "false" });
    }

    /// Convenience: `key` then a string value.
    pub fn field_str(&mut self, name: &str, s: &str) {
        self.key(name);
        self.str_value(s);
    }

    /// Convenience: `key` then a `u64` value.
    pub fn field_u64(&mut self, name: &str, n: u64) {
        self.key(name);
        self.u64_value(n);
    }

    /// Convenience: `key` then a bool value.
    pub fn field_bool(&mut self, name: &str, b: bool) {
        self.key(name);
        self.bool_value(b);
    }
}

/// A one-field `{"error": <message>}` document, the body of every error response.
pub fn error_body(message: &str) -> String {
    let mut w = JsonWriter::new();
    w.begin_object();
    w.field_str("error", message);
    w.end_object();
    w.finish()
}

/// Append `s` as a quoted, escaped JSON string (RFC 8259 §7).
fn write_escaped(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_object() {
        let mut w = JsonWriter::new();
        w.begin_object();
        w.end_object();
        assert_eq!(w.finish(), "{}");
    }

    #[test]
    fn flat_object_fields() {
        let mut w = JsonWriter::new();
        w.begin_object();
        w.field_u64("run", 7);
        w.field_str("eco", "cargo");
        w.field_bool("cycle", false);
        w.end_object();
        assert_eq!(w.finish(), r#"{"run":7,"eco":"cargo","cycle":false}"#);
    }

    #[test]
    fn nested_array_of_objects() {
        let mut w = JsonWriter::new();
        w.begin_object();
        w.key("nodes");
        w.begin_array();
        for i in 0..2u64 {
            w.begin_object();
            w.field_u64("id", i);
            w.end_object();
        }
        w.end_array();
        w.end_object();
        assert_eq!(w.finish(), r#"{"nodes":[{"id":0},{"id":1}]}"#);
    }

    #[test]
    fn string_escaping() {
        let mut w = JsonWriter::new();
        let s = format!("a{}b{}c{}d{}e", '"', '\\', '\n', '\t');
        w.str_value(&s);
        assert_eq!(w.finish(), r#""a\"b\\c\nd\te""#);
    }

    #[test]
    fn control_char_escaped_as_unicode() {
        let mut w = JsonWriter::new();
        let s = format!("x{}y", '\u{01}');
        w.str_value(&s);
        let expected = format!("\"x{}y\"", "\\u0001");
        assert_eq!(w.finish(), expected);
    }

    #[test]
    fn backspace_and_formfeed_escaped() {
        let mut w = JsonWriter::new();
        let s = format!("{}{}", '\u{08}', '\u{0C}');
        w.str_value(&s);
        assert_eq!(w.finish(), r#""\b\f""#);
    }

    #[test]
    fn error_body_is_one_field() {
        assert_eq!(error_body("not found"), r#"{"error":"not found"}"#);
        // The message is escaped like any other string.
        assert_eq!(error_body("a\"b"), r#"{"error":"a\"b"}"#);
    }
}
