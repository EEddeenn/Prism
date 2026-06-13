//! Cargo requirement parser: delegates to [`prism_semver::VersionReq`].
//!
//! Cargo's dependency `version` strings use the Cargo range dialect (`^1`,
//! `~1.2`, `=1.0.0`, `>=1, <2`, `*`, hyphen ranges, …) plus the *default-caret*
//! rule: a bare version with **no operator** (`1`, `1.2`, `1.2.3`) means `^1` /
//! `^1.2` / `^1.2.3`. The shared [`VersionReq`] engine treats a bare full
//! version as exact, so this parser rewrites operator-less, non-wildcard tokens
//! to caret form before delegating, then forwards to the engine.
//!
//! `canonicalize` returns the engine's `Display` form so equivalent
//! requirements collapse to one `RequirementId`.

use prism_plugin_api::{ParseError, RequirementParser};
use prism_semver::{Version, VersionReq};

/// Cargo requirement-expression semantics.
pub struct CargoReqs;

impl CargoReqs {
    fn parse_req(raw: &str) -> Result<VersionReq, ParseError> {
        let rewritten = apply_default_caret(raw);
        VersionReq::parse(&rewritten)
            .map_err(|e| ParseError::new("bad-requirement", e.to_string()))
    }
}

/// Apply Cargo's default-caret rule across an AND-set of comparator tokens.
///
/// Each comma/whitespace-separated token that starts with a digit (i.e. has no
/// explicit operator) and is not a wildcard (`*`/`x`) is prefixed with `^`, so
/// `1.2` becomes `^1.2`. Tokens that already carry an operator, wildcards, and
/// hyphen ranges are passed through untouched.
fn apply_default_caret(raw: &str) -> String {
    let s = raw.trim();
    // Hyphen ranges (`A - B`) and bare `*` are left to the engine verbatim.
    if s.is_empty() || s == "*" || s.contains(" - ") {
        return s.to_string();
    }
    // Split on commas, preserving the comma joins, and caret-ify each token.
    let mut out = String::with_capacity(s.len() + 4);
    for (i, token) in s.split(',').enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        let t = token.trim();
        out.push_str(&caretify_token(t));
    }
    out
}

/// Prefix a single token with `^` if it is an operator-less, non-wildcard
/// version. Wildcards and explicit-operator tokens are returned unchanged.
fn caretify_token(t: &str) -> String {
    if t.is_empty() {
        return String::new();
    }
    let first = t.as_bytes()[0];
    let has_operator = matches!(first, b'^' | b'~' | b'>' | b'<' | b'=');
    let is_wildcard = t == "*"
        || t.split('.')
            .any(|p| matches!(p, "*" | "x" | "X"));
    if first.is_ascii_digit() && !has_operator && !is_wildcard {
        format!("^{t}")
    } else {
        t.to_string()
    }
}

impl RequirementParser for CargoReqs {
    fn canonicalize(&self, raw: &str) -> Result<String, ParseError> {
        // The engine's Display is a normalized, comma-joined comparator form;
        // two equivalent inputs (`1`, `^1`) canonicalize to the same string.
        Ok(Self::parse_req(raw)?.to_string())
    }

    fn matches(&self, raw: &str, version: &str) -> Result<bool, ParseError> {
        let req = Self::parse_req(raw)?;
        let v = Version::parse(version)
            .map_err(|e| ParseError::new("bad-version", e.to_string()))?;
        Ok(req.matches(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_default_for_bare_version() {
        let r = CargoReqs;
        // Cargo treats a bare `1` as `^1`.
        assert!(r.matches("1", "1.5.0").unwrap());
        assert!(!r.matches("1", "2.0.0").unwrap());
        assert!(r.matches("^1.2.3", "1.9.0").unwrap());
        assert!(!r.matches("^1.2.3", "2.0.0").unwrap());
    }

    #[test]
    fn comparator_sets_and_wildcards() {
        let r = CargoReqs;
        assert!(r.matches(">=1, <2", "1.7.0").unwrap());
        assert!(!r.matches(">=1, <2", "2.0.0").unwrap());
        assert!(r.matches("*", "9.9.9").unwrap());
        assert!(r.matches("=1.0.0", "1.0.0").unwrap());
        assert!(!r.matches("=1.0.0", "1.0.1").unwrap());
    }

    #[test]
    fn canonicalizes_equivalent_requirements() {
        let r = CargoReqs;
        let a = r.canonicalize("1").unwrap();
        let b = r.canonicalize("^1").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn rejects_garbage() {
        let r = CargoReqs;
        assert!(r.matches("@@@", "1.0.0").is_err());
    }

    #[test]
    fn full_bare_version_is_caret_not_exact() {
        let r = CargoReqs;
        // Cargo: `1.2.3` means `^1.2.3`, so 1.5.0 satisfies it.
        assert!(r.matches("1.2.3", "1.5.0").unwrap());
        assert!(!r.matches("1.2.3", "2.0.0").unwrap());
        // Explicit `=1.2.3` stays exact.
        assert!(!r.matches("=1.2.3", "1.5.0").unwrap());
    }

    #[test]
    fn wildcards_and_tilde_pass_through() {
        let r = CargoReqs;
        assert!(r.matches("1.*", "1.9.0").unwrap());
        assert!(!r.matches("1.*", "2.0.0").unwrap());
        assert!(r.matches("~1.2", "1.2.9").unwrap());
        assert!(!r.matches("~1.2", "1.3.0").unwrap());
    }
}
