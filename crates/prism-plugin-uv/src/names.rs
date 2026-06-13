//! PyPI name normalization (PEP 503).
//!
//! PEP 503 defines the canonical form of a project name: lowercase, with every
//! run of `-`, `_`, or `.` replaced by a single `-`. So `Foo.Bar`, `foo_bar`, and
//! `foo--bar` all normalize to `foo-bar` and denote the same project. uv already
//! writes normalized names into `uv.lock`, but normalizing here keeps the
//! ecosystem's identity rule authoritative.

use prism_plugin_api::{NameNormalizer, ParseError};

/// PEP 503 normalization for PyPI project names.
pub struct UvNames;

/// Normalize a raw project name to its PEP 503 canonical form. Returns `None` for
/// an empty name (or one that is only separators), which is not a valid project.
pub fn normalize_pep503(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut prev_sep = false;
    for ch in raw.chars() {
        if matches!(ch, '-' | '_' | '.') {
            // Collapse a run of separators to a single '-', but never lead with one.
            if !out.is_empty() && !prev_sep {
                out.push('-');
            }
            prev_sep = true;
        } else {
            out.push(ch.to_ascii_lowercase());
            prev_sep = false;
        }
    }
    // Drop a trailing '-' produced by a run of separators at the end.
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

impl NameNormalizer for UvNames {
    fn normalize(&self, raw: &str) -> Result<String, ParseError> {
        normalize_pep503(raw)
            .ok_or_else(|| ParseError::new("empty-name", "project name is empty after PEP 503 fold"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case_and_separators() {
        let n = UvNames;
        assert_eq!(n.normalize("Foo.Bar").unwrap(), "foo-bar");
        assert_eq!(n.normalize("foo_bar").unwrap(), "foo-bar");
        assert_eq!(n.normalize("foo--bar").unwrap(), "foo-bar");
        assert_eq!(n.normalize("typing_extensions").unwrap(), "typing-extensions");
        assert_eq!(n.normalize("PyYAML").unwrap(), "pyyaml");
        // All the separator spellings collide.
        assert!(n.names_equal("foo.bar", "foo_bar"));
        assert!(n.names_equal("Foo-Bar", "foo--bar"));
    }

    #[test]
    fn trims_edge_separators_and_rejects_empty() {
        let n = UvNames;
        assert_eq!(n.normalize("_foo_").unwrap(), "foo");
        assert!(n.normalize("").is_err());
        assert!(n.normalize("___").is_err());
    }
}
