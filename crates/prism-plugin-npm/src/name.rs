//! npm name normalization (architecture doc, *npm Plugin*).
//!
//! npm names come in two shapes: unscoped (`left-pad`) and scoped
//! (`@scope/name`). The registry treats the **scope** case-insensitively but
//! preserves the case of an unscoped package's published name; in practice npm
//! has rejected new mixed-case names since 2017, yet historical mixed-case names
//! still exist. We therefore keep two notions:
//!
//! - [`NpmNames::normalize`] — the *canonical identity key*. Scope is always
//!   lowercased; the unscoped portion's case is **preserved** so two distinct
//!   historical packages that differ only in case do not collide.
//! - [`NpmNames::collision_key`] — a fully case-folded key used to *detect*
//!   typosquat/dedup collisions (`React` and `react` share one collision key but
//!   two identities).

use prism_plugin_api::ParseError;
use prism_plugin_api::NameNormalizer;

/// Maximum length of an npm package name (registry limit is 214 bytes).
pub const MAX_NAME_LEN: usize = 214;

/// npm [`NameNormalizer`]: lowercases the scope, preserves unscoped case, and
/// exposes a separate case-folded collision key.
#[derive(Debug, Default, Clone, Copy)]
pub struct NpmNames;

impl NpmNames {
    /// The fully case-folded collision key for `raw` (scope **and** name
    /// lowercased). Two raw names share a collision key iff npm would consider
    /// them a case-insensitive squat of each other. Returns the same
    /// [`ParseError`]s as [`NpmNames::normalize`] for structurally invalid names.
    pub fn collision_key(&self, raw: &str) -> Result<String, ParseError> {
        Ok(self.normalize(raw)?.to_ascii_lowercase())
    }

    /// Split a scoped name into `(scope, name)` without the leading `@`/`/`.
    /// Returns `None` for an unscoped name.
    pub fn split_scope(raw: &str) -> Option<(&str, &str)> {
        let rest = raw.strip_prefix('@')?;
        let (scope, name) = rest.split_once('/')?;
        Some((scope, name))
    }

    /// True if `raw` is a scoped name (`@scope/name`).
    pub fn is_scoped(raw: &str) -> bool {
        Self::split_scope(raw).is_some()
    }
}

/// Validate a single name segment (scope or unscoped package name).
///
/// npm's URL-safe rules are stricter than this, but for offline ingestion we
/// only reject what would make the canonical key ambiguous or unsafe: empty,
/// whitespace, control chars, path separators, or a leading dot.
fn check_segment(seg: &str, what: &str) -> Result<(), ParseError> {
    if seg.is_empty() {
        return Err(ParseError::new("empty-name", format!("{what} is empty")));
    }
    if seg.starts_with('.') {
        return Err(ParseError::new(
            "bad-name",
            format!("{what} may not begin with a dot"),
        ));
    }
    for ch in seg.chars() {
        if ch.is_control() || ch.is_whitespace() {
            return Err(ParseError::new(
                "bad-name",
                format!("{what} contains a control or whitespace character"),
            ));
        }
        // A bare segment must not contain a path separator; scope is handled
        // separately by the caller.
        if ch == '/' || ch == '\\' || ch == '@' {
            return Err(ParseError::new(
                "bad-name",
                format!("{what} contains an invalid character `{ch}`"),
            ));
        }
    }
    Ok(())
}

impl NameNormalizer for NpmNames {
    fn normalize(&self, raw: &str) -> Result<String, ParseError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(ParseError::new("empty-name", "name is empty"));
        }
        if raw.len() > MAX_NAME_LEN {
            return Err(ParseError::new(
                "name-too-long",
                format!("name exceeds {MAX_NAME_LEN} bytes"),
            )
            .at(raw.to_string()));
        }

        if let Some((scope, name)) = Self::split_scope(raw) {
            check_segment(scope, "scope")?;
            check_segment(name, "package name")?;
            // Scope is case-folded; the package name's case is preserved.
            Ok(format!("@{}/{}", scope.to_ascii_lowercase(), name))
        } else {
            if raw.starts_with('@') {
                return Err(ParseError::new(
                    "bad-name",
                    "scoped name is missing a `/name` part",
                )
                .at(raw.to_string()));
            }
            check_segment(raw, "package name")?;
            Ok(raw.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_is_lowercased_name_case_preserved() {
        let n = NpmNames;
        assert_eq!(n.normalize("@MyOrg/MyPkg").unwrap(), "@myorg/MyPkg");
        assert_eq!(n.normalize("left-pad").unwrap(), "left-pad");
        // Unscoped historical mixed case is preserved as identity.
        assert_eq!(n.normalize("React").unwrap(), "React");
    }

    #[test]
    fn collision_key_folds_everything() {
        let n = NpmNames;
        assert_eq!(n.collision_key("React").unwrap(), "react");
        assert_eq!(n.collision_key("@MyOrg/MyPkg").unwrap(), "@myorg/mypkg");
        // Distinct identities, same collision key.
        assert_ne!(
            n.normalize("React").unwrap(),
            n.normalize("react").unwrap()
        );
        assert_eq!(
            n.collision_key("React").unwrap(),
            n.collision_key("react").unwrap()
        );
    }

    #[test]
    fn names_equal_uses_identity_not_collision() {
        let n = NpmNames;
        // names_equal is identity equality: case-different unscoped names differ.
        assert!(!n.names_equal("React", "react"));
        // Scope case does not matter for identity.
        assert!(n.names_equal("@Org/pkg", "@org/pkg"));
    }

    #[test]
    fn split_scope_and_is_scoped() {
        assert_eq!(NpmNames::split_scope("@a/b"), Some(("a", "b")));
        assert_eq!(NpmNames::split_scope("a"), None);
        assert!(NpmNames::is_scoped("@a/b"));
        assert!(!NpmNames::is_scoped("a"));
    }

    #[test]
    fn rejects_malformed_names() {
        let n = NpmNames;
        assert!(n.normalize("").is_err());
        assert!(n.normalize("   ").is_err());
        assert!(n.normalize("@scope-only").is_err());
        assert!(n.normalize("@/name").is_err());
        assert!(n.normalize("@scope/").is_err());
        assert!(n.normalize(".hidden").is_err());
        assert!(n.normalize("has space").is_err());
        assert!(n.normalize(&"x".repeat(MAX_NAME_LEN + 1)).is_err());
    }

    #[test]
    fn trims_surrounding_whitespace() {
        let n = NpmNames;
        assert_eq!(n.normalize("  left-pad  ").unwrap(), "left-pad");
    }
}
