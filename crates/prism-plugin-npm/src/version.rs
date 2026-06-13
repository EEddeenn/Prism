//! npm version scheme: a thin delegation to [`prism_semver`].
//!
//! npm versions are SemVer 2.0, optionally written with a leading `v`
//! (`v1.2.3`). [`prism_semver::Version`] handles the rest, including the
//! prerelease-aware total order and the byte sort key that becomes
//! `PackageVersion::version_sort_key`.

use prism_plugin_api::{ParseError, VersionOrdering, VersionScheme};
use prism_semver::Version;

/// Strip an optional leading `v`/`V` (npm tolerates `v1.2.3`).
fn strip_v(raw: &str) -> &str {
    let t = raw.trim();
    t.strip_prefix('v').or_else(|| t.strip_prefix('V')).unwrap_or(t)
}

fn parse(raw: &str) -> Result<Version, ParseError> {
    Version::parse(strip_v(raw)).map_err(|e| ParseError::new("bad-version", e.to_string()))
}

/// npm [`VersionScheme`] backed by [`prism_semver`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NpmVersions;

impl VersionScheme for NpmVersions {
    fn parse(&self, raw: &str) -> Result<String, ParseError> {
        // Normalized form drops the leading `v` and build metadata is preserved
        // by `Version::Display`.
        Ok(parse(raw)?.to_string())
    }

    fn compare(&self, a: &str, b: &str) -> Result<VersionOrdering, ParseError> {
        Ok(parse(a)?.cmp(&parse(b)?))
    }

    fn sort_key(&self, raw: &str) -> Result<Vec<u8>, ParseError> {
        Ok(parse(raw)?.sort_key())
    }

    fn is_prerelease(&self, raw: &str) -> Result<bool, ParseError> {
        Ok(parse(raw)?.is_prerelease())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn parses_and_strips_leading_v() {
        let vs = NpmVersions;
        assert_eq!(vs.parse("1.2.3").unwrap(), "1.2.3");
        assert_eq!(vs.parse("v1.2.3").unwrap(), "1.2.3");
        assert_eq!(vs.parse("  v0.1.0  ").unwrap(), "0.1.0");
        assert_eq!(vs.parse("1.0.0-rc.1").unwrap(), "1.0.0-rc.1");
        assert!(vs.parse("not-a-version").is_err());
    }

    #[test]
    fn ordering_is_semver_correct() {
        let vs = NpmVersions;
        assert_eq!(vs.compare("1.2.0", "1.10.0").unwrap(), Ordering::Less);
        assert_eq!(vs.compare("2.0.0", "2.0.0").unwrap(), Ordering::Equal);
        // Prerelease sorts before its release.
        assert_eq!(vs.compare("1.0.0-rc.1", "1.0.0").unwrap(), Ordering::Less);
    }

    #[test]
    fn sort_key_agrees_with_compare() {
        let vs = NpmVersions;
        let mut versions = ["1.10.0", "1.2.0", "2.0.0", "1.0.0-rc.1", "1.0.0"];
        versions.sort_by(|a, b| vs.compare(a, b).unwrap());
        assert_eq!(
            versions,
            ["1.0.0-rc.1", "1.0.0", "1.2.0", "1.10.0", "2.0.0"]
        );
        // Byte keys order the same way.
        let ka = vs.sort_key("1.2.0").unwrap();
        let kb = vs.sort_key("1.10.0").unwrap();
        assert!(ka < kb);
    }

    #[test]
    fn prerelease_detection() {
        let vs = NpmVersions;
        assert!(vs.is_prerelease("1.0.0-rc.1").unwrap());
        assert!(!vs.is_prerelease("1.0.0").unwrap());
        assert!(!vs.is_prerelease("v2.3.4").unwrap());
    }
}
