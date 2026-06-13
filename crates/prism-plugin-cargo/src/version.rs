//! Cargo version scheme: a thin delegation to [`prism_semver::Version`].
//!
//! Cargo versions are SemVer 2.0, so this capability simply forwards to the
//! shared engine. `sort_key` returns [`prism_semver::Version::sort_key`], whose
//! lexicographic byte order matches [`Version`]'s total [`Ord`] exactly, so it
//! satisfies the [`VersionScheme`] contract (`compare` ⇔ `sort_key`).

use prism_plugin_api::{ParseError, VersionOrdering, VersionScheme};
use prism_semver::Version;

/// SemVer-2.0 version semantics for Cargo.
pub struct CargoVersions;

impl CargoVersions {
    /// Parse a raw string into a [`Version`], mapping the model error into a
    /// [`ParseError`] so the plugin boundary stays in the parse-error domain.
    fn parse_version(raw: &str) -> Result<Version, ParseError> {
        Version::parse(raw).map_err(|e| ParseError::new("bad-version", e.to_string()))
    }
}

impl VersionScheme for CargoVersions {
    fn parse(&self, raw: &str) -> Result<String, ParseError> {
        // Re-serialize via the normalized core; we keep the trimmed canonical
        // string (drops surrounding whitespace, validates structure).
        let v = Self::parse_version(raw)?;
        // Reconstruct a canonical `major.minor.patch[-pre][+build]` string.
        Ok(canonical(&v))
    }

    fn compare(&self, a: &str, b: &str) -> Result<VersionOrdering, ParseError> {
        Ok(Self::parse_version(a)?.cmp(&Self::parse_version(b)?))
    }

    fn sort_key(&self, raw: &str) -> Result<Vec<u8>, ParseError> {
        Ok(Self::parse_version(raw)?.sort_key())
    }

    fn is_prerelease(&self, raw: &str) -> Result<bool, ParseError> {
        Ok(Self::parse_version(raw)?.is_prerelease())
    }
}

/// Render a [`Version`] back to its canonical normalized string.
fn canonical(v: &Version) -> String {
    let mut s = format!("{}.{}.{}", v.major, v.minor, v.patch);
    if !v.pre.is_empty() {
        s.push('-');
        for (i, id) in v.pre.iter().enumerate() {
            if i > 0 {
                s.push('.');
            }
            s.push_str(&id.to_string());
        }
    }
    if !v.build.is_empty() {
        s.push('+');
        for (i, b) in v.build.iter().enumerate() {
            if i > 0 {
                s.push('.');
            }
            s.push_str(b);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn parses_and_normalizes() {
        let vs = CargoVersions;
        assert_eq!(vs.parse("1.2.3").unwrap(), "1.2.3");
        assert_eq!(vs.parse("  1.0.0-rc.1+build  ").unwrap(), "1.0.0-rc.1+build");
        assert!(vs.parse("not-a-version").is_err());
    }

    #[test]
    fn ordering_matches_semver() {
        let vs = CargoVersions;
        assert_eq!(vs.compare("1.2.0", "1.10.0").unwrap(), Ordering::Less);
        assert_eq!(vs.compare("2.0.0", "2.0.0").unwrap(), Ordering::Equal);
        // Prerelease sorts below its release.
        assert_eq!(vs.compare("1.0.0-rc.1", "1.0.0").unwrap(), Ordering::Less);
    }

    #[test]
    fn sort_key_agrees_with_compare() {
        let vs = CargoVersions;
        let mut versions = ["1.10.0", "1.2.0", "2.0.0", "1.0.0-rc.1", "1.0.0"];
        versions.sort_by(|a, b| vs.compare(a, b).unwrap());
        assert_eq!(
            versions,
            ["1.0.0-rc.1", "1.0.0", "1.2.0", "1.10.0", "2.0.0"]
        );
        // Byte keys order identically.
        let ka = vs.sort_key("1.2.0").unwrap();
        let kb = vs.sort_key("1.10.0").unwrap();
        assert!(ka < kb);
    }

    #[test]
    fn detects_prerelease() {
        let vs = CargoVersions;
        assert!(vs.is_prerelease("1.0.0-alpha").unwrap());
        assert!(!vs.is_prerelease("1.0.0").unwrap());
    }
}
