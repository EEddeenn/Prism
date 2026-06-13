//! Defensive `application/x-www-form-urlencoded` query-string parsing.
//!
//! A query string is `key=value&key=value`; `+` decodes to a space and `%XX` to
//! a byte. Bad escapes are tolerated by falling back to the raw text for that
//! token (a query value never aborts a request the way a bad *path* escape
//! does — it just yields a value that won't match an integer parse). Typed
//! accessors return `None` on a missing/unparseable key so the router answers
//! `400`.

use crate::http::percent_decode;

/// Decoded key/value pairs from a query string, in first-seen order. Duplicate
/// keys keep the first value (deterministic, and matches our single-valued API).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pairs: Vec<(String, String)>,
}

impl Query {
    /// Parse a raw query string (the part after `?`, without it). An empty input
    /// yields an empty map; malformed pairs are skipped, never panic.
    pub fn parse(raw: &str) -> Query {
        let mut pairs = Vec::new();
        if raw.is_empty() {
            return Query { pairs };
        }
        for part in raw.split('&') {
            if part.is_empty() {
                continue;
            }
            let (k, v) = match part.split_once('=') {
                Some((k, v)) => (k, v),
                None => (part, ""), // bare key -> empty value
            };
            let key = decode_component(k);
            let val = decode_component(v);
            if !pairs.iter().any(|(ek, _): &(String, String)| ek == &key) {
                pairs.push((key, val));
            }
        }
        Query { pairs }
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

/// Decode one query component: `+` -> space, then percent-decode. On a bad
/// percent escape, return the (plus-substituted) text unchanged rather than
/// failing the whole request.
fn decode_component(s: &str) -> String {
    let plus_to_space: String = s.chars().map(|c| if c == '+' { ' ' } else { c }).collect();
    percent_decode(&plus_to_space).unwrap_or(plus_to_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_is_empty() {
        let q = Query::parse("");
        assert_eq!(q.get("x"), None);
    }

    #[test]
    fn parses_pairs() {
        let q = Query::parse("run=7&node=3&depth=2");
        assert_eq!(q.u64("run"), Some(7));
        assert_eq!(q.u32("node"), Some(3));
        assert_eq!(q.u32("depth"), Some(2));
    }

    #[test]
    fn missing_and_garbage_numbers() {
        let q = Query::parse("run=abc");
        assert_eq!(q.u64("run"), None); // non-numeric
        assert_eq!(q.u64("absent"), None); // missing
    }

    #[test]
    fn defaulted_distinguishes_absent_from_garbage() {
        let q = Query::parse("depth=&limit=oops");
        assert_eq!(q.u32_or("depth", 2), Some(2)); // empty -> default
        assert_eq!(q.u32_or("missing", 5), Some(5)); // absent -> default
        assert_eq!(q.u32_or("limit", 1), None); // present garbage -> fail
    }

    #[test]
    fn plus_is_space_and_percent_decodes() {
        let q = Query::parse("name=a+b%20c");
        assert_eq!(q.get("name"), Some("a b c"));
    }

    #[test]
    fn bare_key_has_empty_value() {
        let q = Query::parse("flag&run=1");
        assert_eq!(q.get("flag"), Some(""));
        assert_eq!(q.u64("run"), Some(1));
    }

    #[test]
    fn duplicate_key_keeps_first() {
        let q = Query::parse("run=1&run=2");
        assert_eq!(q.u64("run"), Some(1));
    }

    #[test]
    fn bad_escape_falls_back_to_raw() {
        // %ZZ is not valid; the value is kept verbatim rather than aborting.
        let q = Query::parse("x=%ZZ");
        assert_eq!(q.get("x"), Some("%ZZ"));
    }

    #[test]
    fn leading_and_empty_segments_skipped() {
        let q = Query::parse("&&run=1&&");
        assert_eq!(q.u64("run"), Some(1));
    }
}
