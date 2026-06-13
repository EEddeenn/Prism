//! Small ecosystem-agnostic coordinate value types used across the plugin API.
//!
//! These are deliberately *unnormalized* string carriers: the plugin's
//! [`crate::NameNormalizer`] and [`crate::VersionScheme`] turn them into the
//! canonical forms stored in the model. They exist so fetch and parse request
//! types do not have to traffic in bare `String`s with ambiguous meaning.

/// A package name as written by a user or read from a document, *before*
/// normalization. The canonical key is produced by [`crate::NameNormalizer`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageName(pub String);

impl PackageName {
    /// Wrap a raw name.
    #[inline]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The raw, un-normalized name string.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PackageName {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl core::fmt::Display for PackageName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a fetch is keyed to within a package: either a concrete version or a
/// requirement range the fetcher should expand against the index.
///
/// The raw strings are interpreted by the ecosystem's [`crate::VersionScheme`]
/// and [`crate::RequirementParser`]; this type carries them without judging
/// their syntax.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum VersionReqOrExact {
    /// A single, fully-pinned version (e.g. `1.2.3`).
    Exact(String),
    /// A requirement expression (e.g. `^18`, `>=1,<2`, `~=2.31`).
    Requirement(String),
}

impl VersionReqOrExact {
    /// The underlying raw string, regardless of which arm.
    #[inline]
    pub fn as_str(&self) -> &str {
        match self {
            VersionReqOrExact::Exact(s) | VersionReqOrExact::Requirement(s) => s,
        }
    }

    /// True if this is a single pinned [`VersionReqOrExact::Exact`] version.
    #[inline]
    pub fn is_exact(&self) -> bool {
        matches!(self, VersionReqOrExact::Exact(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_name_roundtrip() {
        let n = PackageName::from("@scope/pkg");
        assert_eq!(n.as_str(), "@scope/pkg");
        assert_eq!(n.to_string(), "@scope/pkg");
        assert_eq!(PackageName::new("x"), PackageName("x".to_string()));
    }

    #[test]
    fn version_req_or_exact_accessors() {
        let e = VersionReqOrExact::Exact("1.2.3".into());
        let r = VersionReqOrExact::Requirement("^1".into());
        assert!(e.is_exact());
        assert!(!r.is_exact());
        assert_eq!(e.as_str(), "1.2.3");
        assert_eq!(r.as_str(), "^1");
    }
}
