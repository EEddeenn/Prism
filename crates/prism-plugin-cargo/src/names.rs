//! crates.io name normalization (architecture doc, *Cargo Plugin*).
//!
//! crates.io accepts names of 1..=64 bytes drawn from ASCII letters, digits,
//! `-`, and `_`, and the first character must be an ASCII letter. For collision
//! detection crates.io folds case and treats `-` and `_` as equivalent, so the
//! canonical identity key lowercases and maps every `-` to `_`. The *raw* name a
//! manifest writes (`serde-json` vs `serde_json`) is preserved by callers; only
//! the canonical key is normalized here.

use prism_plugin_api::{NameNormalizer, ParseError};

/// Maximum crate-name length enforced by crates.io.
pub const MAX_NAME_LEN: usize = 64;

/// crates.io name rules.
pub struct CargoNames;

impl CargoNames {
    /// Validate a raw name against crates.io's character/length rules without
    /// canonicalizing, returning the offending detail on rejection.
    fn validate(raw: &str) -> Result<(), ParseError> {
        if raw.is_empty() {
            return Err(ParseError::new("empty-name", "crate name is empty"));
        }
        if raw.len() > MAX_NAME_LEN {
            return Err(ParseError::new(
                "name-too-long",
                format!("crate name exceeds {MAX_NAME_LEN} bytes: {raw}"),
            ));
        }
        let mut chars = raw.bytes();
        let first = chars.next().expect("non-empty checked above");
        if !first.is_ascii_alphabetic() {
            return Err(ParseError::new(
                "bad-name-start",
                format!("crate name must start with an ASCII letter: {raw}"),
            ));
        }
        for b in raw.bytes() {
            if !(b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
                return Err(ParseError::new(
                    "bad-name-char",
                    format!("invalid character in crate name: {raw}"),
                ));
            }
        }
        Ok(())
    }
}

impl NameNormalizer for CargoNames {
    fn normalize(&self, raw: &str) -> Result<String, ParseError> {
        Self::validate(raw)?;
        // Canonical identity: lowercase, `-`/`_` unified to `_` (crates.io's
        // collision rule). `serde-json` and `serde_json` share one key.
        let mut out = String::with_capacity(raw.len());
        for b in raw.bytes() {
            let c = if b == b'-' { b'_' } else { b.to_ascii_lowercase() };
            out.push(c as char);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercases_and_unifies_separators() {
        let n = CargoNames;
        assert_eq!(n.normalize("Serde-JSON").unwrap(), "serde_json");
        assert_eq!(n.normalize("serde_json").unwrap(), "serde_json");
        // Hyphen and underscore forms collide under normalization.
        assert!(n.names_equal("serde-json", "serde_json"));
        assert!(n.names_equal("Tokio", "tokio"));
    }

    #[test]
    fn rejects_invalid_names() {
        let n = CargoNames;
        assert!(n.normalize("").is_err());
        assert!(n.normalize("1abc").is_err()); // must start with a letter
        assert!(n.normalize("foo bar").is_err()); // space
        assert!(n.normalize("foo/bar").is_err()); // slash
        assert!(n.normalize(&"a".repeat(MAX_NAME_LEN + 1)).is_err());
    }

    #[test]
    fn accepts_valid_names() {
        let n = CargoNames;
        assert_eq!(n.normalize("a").unwrap(), "a");
        assert_eq!(n.normalize("my-crate_2").unwrap(), "my_crate_2");
    }
}
