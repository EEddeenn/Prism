//! npm requirement parsing (architecture doc, *npm Plugin*).
//!
//! npm requirement strings are mostly the SemVer range dialect handled by
//! [`prism_semver::VersionReq`] (exact, comparators, `^`, `~`, `*`, hyphen and
//! AND ranges, `x`-wildcards). But npm also allows **non-range protocols** that
//! do not constrain a registry version at all:
//!
//! - dist-tags: `latest`, `next`, … (a bare alphanumeric word)
//! - `file:` / `link:` / `portal:` local paths
//! - `workspace:` workspace protocol (`workspace:*`, `workspace:^1.2.3`)
//! - `git`, `git+ssh`, `github:owner/repo`, `http(s):` tarball URLs
//! - `npm:alias@range` aliasing
//!
//! For v0.1 (offline) we **canonicalize** every requirement to a stable string
//! (so `RequirementId` is deterministic) and only run real range matching on the
//! semver-range subset. Non-range protocols canonicalize to their trimmed form
//! and [`NpmReqs::matches`] reports `false` (no registry version satisfies a
//! local/git pin) rather than erroring, which is the honest answer in offline
//! mode. The kind/category of a protocol is exposed via [`ReqProtocol`].

use prism_plugin_api::{ParseError, RequirementParser};
use prism_semver::{Version, VersionReq};

/// The protocol class of an npm requirement string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReqProtocol {
    /// A SemVer range (`^1.2.3`, `>=1, <2`, `1.2.x`, `*`, `latest`-less).
    SemverRange,
    /// A dist-tag word (`latest`, `next`).
    DistTag,
    /// A `workspace:` protocol requirement.
    Workspace,
    /// A `file:` / `link:` / `portal:` local path.
    Local,
    /// A `git`/`github:`/URL source.
    Git,
    /// An `http(s):` tarball URL.
    Url,
    /// An `npm:alias@range` aliased requirement.
    Alias,
}

impl ReqProtocol {
    /// Classify a trimmed requirement string.
    pub fn classify(raw: &str) -> ReqProtocol {
        let s = raw.trim();
        if let Some(rest) = s.strip_prefix("npm:") {
            // `npm:name@range` — alias unless it is just `npm:` noise.
            if rest.contains('@') || !rest.is_empty() {
                return ReqProtocol::Alias;
            }
        }
        if s.starts_with("workspace:") {
            return ReqProtocol::Workspace;
        }
        if s.starts_with("file:") || s.starts_with("link:") || s.starts_with("portal:") {
            return ReqProtocol::Local;
        }
        if s.starts_with("git:")
            || s.starts_with("git+")
            || s.starts_with("github:")
            || s.starts_with("bitbucket:")
            || s.starts_with("gitlab:")
            || s.starts_with("gist:")
        {
            return ReqProtocol::Git;
        }
        if s.starts_with("http://") || s.starts_with("https://") {
            return ReqProtocol::Url;
        }
        // A bare alphanumeric word that is not a range is a dist-tag.
        if is_dist_tag(s) {
            return ReqProtocol::DistTag;
        }
        ReqProtocol::SemverRange
    }

    /// True if this protocol pins a registry version space we can range-match.
    pub fn is_registry_range(self) -> bool {
        matches!(self, ReqProtocol::SemverRange)
    }
}

/// Strip a leading `v`/`V` that directly precedes a digit (npm accepts `v1.2.3`
/// as an exact-version requirement). A real range (`^1.2.3`) or a dist-tag
/// (`vue`, `latest`) is returned unchanged.
fn strip_v(s: &str) -> &str {
    match s.strip_prefix(['v', 'V']) {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest,
        _ => s,
    }
}

/// A dist-tag is a non-empty token of `[A-Za-z][A-Za-z0-9-_.]*` that is not the
/// `*`/empty "any" range and does not look like a comparator/number.
fn is_dist_tag(s: &str) -> bool {
    // `x`/`X` are the npm "any" wildcard, and `v1.2.3` is a version — both are
    // SemVer ranges, not dist-tags, so they must not be shadowed here.
    if s.eq_ignore_ascii_case("x") || strip_v(s) != s {
        return false;
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// The `workspace:` payload after the prefix (`*`, `~`, `^`, or a range).
fn workspace_inner(s: &str) -> &str {
    s.trim().strip_prefix("workspace:").unwrap_or(s).trim()
}

/// npm [`RequirementParser`] over [`prism_semver`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NpmReqs;

impl RequirementParser for NpmReqs {
    fn canonicalize(&self, raw: &str) -> Result<String, ParseError> {
        let s = raw.trim();
        match ReqProtocol::classify(s) {
            ReqProtocol::SemverRange => {
                let req = VersionReq::parse(strip_v(s))
                    .map_err(|e| ParseError::new("bad-requirement", e.to_string()))?;
                Ok(req.to_string())
            }
            ReqProtocol::Workspace => {
                // Canonicalize the inner range when it is a real range.
                let inner = workspace_inner(s);
                match inner {
                    "" | "*" | "^" | "~" => Ok(format!("workspace:{inner}")),
                    other => match VersionReq::parse(other) {
                        Ok(req) => Ok(format!("workspace:{req}")),
                        Err(_) => Ok(s.to_string()),
                    },
                }
            }
            // Non-range protocols are opaque; preserve a stable trimmed form.
            ReqProtocol::DistTag
            | ReqProtocol::Local
            | ReqProtocol::Git
            | ReqProtocol::Url
            | ReqProtocol::Alias => {
                if s.is_empty() {
                    return Err(ParseError::new("empty-requirement", "requirement is empty"));
                }
                Ok(s.to_string())
            }
        }
    }

    fn matches(&self, raw: &str, version: &str) -> Result<bool, ParseError> {
        let s = raw.trim();
        let proto = ReqProtocol::classify(s);
        match proto {
            ReqProtocol::SemverRange => {
                let req = VersionReq::parse(strip_v(s))
                    .map_err(|e| ParseError::new("bad-requirement", e.to_string()))?;
                let v = parse_version(version)?;
                Ok(req.matches(&v))
            }
            ReqProtocol::Workspace => {
                let inner = workspace_inner(s);
                // `workspace:*` accepts any version of the linked workspace pkg.
                let req = match inner {
                    "" | "*" => VersionReq::any(),
                    other => VersionReq::parse(other)
                        .map_err(|e| ParseError::new("bad-requirement", e.to_string()))?,
                };
                let v = parse_version(version)?;
                Ok(req.matches(&v))
            }
            // Offline: a local/git/url/tag/alias pin has no resolvable registry
            // version, so nothing "matches" in the range sense.
            _ => Ok(false),
        }
    }
}

fn parse_version(raw: &str) -> Result<Version, ParseError> {
    let t = raw.trim();
    let t = t.strip_prefix('v').or_else(|| t.strip_prefix('V')).unwrap_or(t);
    Version::parse(t).map_err(|e| ParseError::new("bad-version", e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x_wildcard_and_v_prefix_are_versions_not_dist_tags() {
        let r = NpmReqs;
        // `x`/`X` are the "any" wildcard, not a dist-tag.
        assert!(r.matches("x", "1.0.0").unwrap());
        assert!(r.matches("X", "9.9.9").unwrap());
        assert_eq!(r.canonicalize("x").unwrap(), "*");
        // Leading-`v` exact versions parse as ranges.
        assert!(r.matches("v1.2.3", "1.2.3").unwrap());
        assert!(!r.matches("v1.2.3", "1.2.4").unwrap());
        // Genuine dist-tags (alpha words) are still opaque.
        assert!(!r.matches("latest", "1.0.0").unwrap());
        assert!(!r.matches("vue", "1.0.0").unwrap());
        assert_eq!(r.canonicalize("latest").unwrap(), "latest");
        assert!(!is_dist_tag("x") && !is_dist_tag("X") && !is_dist_tag("v1.2.3"));
        assert!(is_dist_tag("latest") && is_dist_tag("vue") && is_dist_tag("next"));
    }

    #[test]
    fn semver_ranges_match() {
        let r = NpmReqs;
        assert!(r.matches("^1.2.3", "1.4.0").unwrap());
        assert!(!r.matches("^1.2.3", "2.0.0").unwrap());
        assert!(r.matches("~1.2.3", "1.2.9").unwrap());
        assert!(!r.matches("~1.2.3", "1.3.0").unwrap());
        assert!(r.matches("1.2.x", "1.2.7").unwrap());
        assert!(r.matches(">=1.0.0 <2.0.0", "1.9.9").unwrap());
        assert!(r.matches("*", "9.9.9").unwrap());
        assert!(r.matches("", "1.0.0").unwrap());
        assert!(r.matches("1.2.3", "1.2.3").unwrap());
    }

    #[test]
    fn canonicalize_is_stable() {
        let r = NpmReqs;
        // Equivalent spellings canonicalize identically.
        let a = r.canonicalize(">=1.2.0").unwrap();
        let b = r.canonicalize(">= 1.2.0").unwrap();
        assert_eq!(a, b);
        assert_eq!(r.canonicalize("*").unwrap(), "*");
    }

    #[test]
    fn protocol_classification() {
        assert_eq!(ReqProtocol::classify("^1.2.3"), ReqProtocol::SemverRange);
        assert_eq!(ReqProtocol::classify("1.2.x"), ReqProtocol::SemverRange);
        assert_eq!(ReqProtocol::classify("latest"), ReqProtocol::DistTag);
        assert_eq!(ReqProtocol::classify("next"), ReqProtocol::DistTag);
        assert_eq!(ReqProtocol::classify("workspace:*"), ReqProtocol::Workspace);
        assert_eq!(ReqProtocol::classify("file:../x"), ReqProtocol::Local);
        assert_eq!(ReqProtocol::classify("link:../x"), ReqProtocol::Local);
        assert_eq!(
            ReqProtocol::classify("git+ssh://git@host/x.git"),
            ReqProtocol::Git
        );
        assert_eq!(ReqProtocol::classify("github:owner/repo"), ReqProtocol::Git);
        assert_eq!(
            ReqProtocol::classify("https://x/y.tgz"),
            ReqProtocol::Url
        );
        assert_eq!(ReqProtocol::classify("npm:left-pad@^1"), ReqProtocol::Alias);
    }

    #[test]
    fn workspace_protocol_matches_and_canonicalizes() {
        let r = NpmReqs;
        assert!(r.matches("workspace:*", "9.9.9").unwrap());
        assert!(r.matches("workspace:^1.2.0", "1.4.0").unwrap());
        assert!(!r.matches("workspace:^1.2.0", "2.0.0").unwrap());
        assert_eq!(r.canonicalize("workspace:*").unwrap(), "workspace:*");
        // The inner range canonicalizes through VersionReq (caret expands).
        assert_eq!(
            r.canonicalize("workspace:^1.2.0").unwrap(),
            "workspace:>=1.2.0, <2.0.0"
        );
        // Equivalent spellings of the inner range share one canonical form.
        assert_eq!(
            r.canonicalize("workspace:^1.2.0").unwrap(),
            r.canonicalize("workspace:>=1.2.0 <2.0.0").unwrap()
        );
    }

    #[test]
    fn non_range_protocols_do_not_match_offline() {
        let r = NpmReqs;
        assert!(!r.matches("latest", "1.0.0").unwrap());
        assert!(!r.matches("file:../local", "1.0.0").unwrap());
        assert!(!r.matches("github:a/b", "1.0.0").unwrap());
        // …but they still canonicalize to a stable id.
        assert_eq!(r.canonicalize("latest").unwrap(), "latest");
        assert_eq!(r.canonicalize("file:../local").unwrap(), "file:../local");
    }

    #[test]
    fn bad_range_errors() {
        let r = NpmReqs;
        assert!(r.canonicalize(">=not.a.version").is_err());
        assert!(r.matches("^1.0.0", "garbage").is_err());
    }
}
