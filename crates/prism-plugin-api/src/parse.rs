//! Canonical fact events emitted by parsers (architecture doc, *Parser
//! Responsibilities*).
//!
//! A parser converts a [`crate::RawDocument`] into a stream of small,
//! ecosystem-agnostic *facts*. Parsers are deterministic and side-effect free:
//! they never write to the store. A separate writer consumes a
//! [`CanonicalBatch`] and allocates model IDs / blobs. Because the parser runs
//! before any store IDs exist, facts carry *raw strings and references*, not
//! [`prism_model`] integer IDs.
//!
//! Parsers consume hostile bytes, so they return [`ParseError`] rather than
//! panicking, and surface non-fatal issues as [`ParseWarning`] events.

use prism_model::{ConditionKind, DependencyKind, HashDigest};

use crate::coord::PackageName;

/// The trust class of a checksum (architecture doc, *Integrity and trust
/// model*). Higher is more trustworthy; a registry-provided hash proves
/// transport integrity, not authenticity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChecksumTrust {
    /// Advertised in registry metadata only (lowest trust).
    RegistryMetadata,
    /// From a checksum database or transparency log.
    TransparencyLog,
    /// Pinned by the user's own lockfile (highest trust).
    UserPinned,
}

/// A named hash algorithm for a [`ChecksumFact`]. SHA-256 is the canonical
/// content-address; others are recorded as advertised for cross-checking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HashAlgorithm {
    /// SHA-256.
    Sha256,
    /// SHA-512.
    Sha512,
    /// SHA-1 (legacy; npm integrity, git).
    Sha1,
    /// MD5 (legacy; weak — recorded, never trusted for integrity).
    Md5,
}

/// A package family was observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageFact {
    /// Raw (un-normalized) package name as written in the document.
    pub name: PackageName,
    /// Optional namespace/scope (npm scope, Maven groupId).
    pub namespace: Option<String>,
    /// Upstream repository URL, if advertised (caller redacts before storage).
    pub repository_url: Option<String>,
    /// Project homepage URL, if advertised.
    pub homepage_url: Option<String>,
}

/// A concrete published version of a package was observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionFact {
    /// Raw (un-normalized) package name this version belongs to.
    pub name: PackageName,
    /// Raw version string exactly as published.
    pub version_raw: String,
    /// Publication time, if known.
    pub released_at_unix: Option<i64>,
    /// SPDX-ish license expression, if declared.
    pub license_expr: Option<String>,
    /// True if this version is a prerelease per the ecosystem's scheme.
    pub is_prerelease: bool,
}

/// A declared dependency requirement was observed on a version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequirementFact {
    /// Raw name of the version declaring this requirement.
    pub src_name: PackageName,
    /// Raw version string of the declaring version.
    pub src_version_raw: String,
    /// Raw target coordinate exactly as written.
    pub declared_target: String,
    /// Optional local alias (Cargo renamed deps, `package` field).
    pub target_alias: Option<String>,
    /// Raw requirement/constraint expression.
    pub raw_requirement: String,
    /// Dependency category.
    pub kind: DependencyKind,
    /// Optional ecosystem scope (Maven scope, npm bundle marker, …).
    pub scope: Option<String>,
    /// Whether the condition gates on environment or activation closure.
    pub condition_kind: ConditionKind,
    /// Raw condition expression (marker / `cfg(...)` / profile activation).
    pub raw_condition: Option<String>,
    /// Raw features/extras requested on the target (e.g. `["derive"]`).
    pub features_or_extras: Vec<String>,
}

/// A distribution file (wheel, sdist, tarball, jar) for a version was observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistributionFileFact {
    /// Raw package name.
    pub name: PackageName,
    /// Raw version string.
    pub version_raw: String,
    /// The distribution filename as published.
    pub filename: String,
    /// Redacted download URL of the file (caller redacts before storage).
    pub url: Option<String>,
    /// Compatibility tag(s): wheel tags, target triple, packaging type.
    pub compatibility_tags: Vec<String>,
    /// `Requires-Python` / `rust-version`-style interpreter constraint.
    pub requires_runtime: Option<String>,
    /// True if this individual file is yanked (PyPI per-file yank).
    pub is_yanked: bool,
}

/// A version's yanked/retracted status was observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YankedFact {
    /// Raw package name.
    pub name: PackageName,
    /// Raw version string.
    pub version_raw: String,
    /// Whether the version is yanked.
    pub yanked: bool,
    /// Optional reason string (Go `retract` rationale, npm/Cargo note).
    pub reason: Option<String>,
}

/// A version's deprecation status was observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeprecationFact {
    /// Raw package name.
    pub name: PackageName,
    /// Raw version string, or `None` when the whole family is deprecated.
    pub version_raw: Option<String>,
    /// Human-readable deprecation message, if any.
    pub message: Option<String>,
}

/// A checksum for an artifact or distribution file was observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChecksumFact {
    /// Raw package name.
    pub name: PackageName,
    /// Raw version string.
    pub version_raw: String,
    /// The distribution filename this checksum is for, if file-scoped.
    pub filename: Option<String>,
    /// Hash algorithm.
    pub algorithm: HashAlgorithm,
    /// The raw digest bytes (length depends on `algorithm`).
    pub digest: Vec<u8>,
    /// SHA-256 digest in the canonical model form, when `algorithm` is SHA-256.
    pub sha256: Option<HashDigest>,
    /// Trust class of this checksum.
    pub trust: ChecksumTrust,
}

/// A non-fatal issue encountered during parsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseWarning {
    /// Stable machine-readable code (e.g. `"unknown-field"`).
    pub code: String,
    /// Human-readable detail.
    pub detail: String,
    /// Optional location hint (line, json pointer, xpath).
    pub location: Option<String>,
}

impl ParseWarning {
    /// Construct a warning with no location.
    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: detail.into(),
            location: None,
        }
    }
}

/// One canonical fact emitted by a parser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanonicalEvent {
    /// A package family was seen.
    PackageSeen(PackageFact),
    /// A concrete version was seen.
    VersionSeen(VersionFact),
    /// A declared requirement was seen.
    RequirementSeen(RequirementFact),
    /// A distribution file was seen.
    DistFileSeen(DistributionFileFact),
    /// A yank/retract status was seen.
    YankedStatusSeen(YankedFact),
    /// A deprecation status was seen.
    DeprecationSeen(DeprecationFact),
    /// A checksum was seen.
    ChecksumSeen(ChecksumFact),
    /// A non-fatal parse issue.
    Warning(ParseWarning),
}

/// An ordered batch of canonical events produced from one [`crate::RawDocument`].
///
/// A batch is an in-memory accumulator: parsers `push` facts and warnings, and a
/// downstream writer drains the events into the store. Events keep insertion
/// order so provenance and ordinals are reproducible.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalBatch {
    /// Events in emission order.
    pub events: Vec<CanonicalEvent>,
}

impl CanonicalBatch {
    /// A new empty batch.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of events collected.
    #[inline]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True if no events have been collected.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Append one event.
    #[inline]
    pub fn push(&mut self, event: CanonicalEvent) {
        self.events.push(event);
    }

    /// Append a [`ParseWarning`] as a [`CanonicalEvent::Warning`].
    #[inline]
    pub fn warn(&mut self, warning: ParseWarning) {
        self.events.push(CanonicalEvent::Warning(warning));
    }

    /// Iterate over the events.
    pub fn iter(&self) -> core::slice::Iter<'_, CanonicalEvent> {
        self.events.iter()
    }

    /// Count only the non-warning fact events.
    pub fn fact_count(&self) -> usize {
        self.events
            .iter()
            .filter(|e| !matches!(e, CanonicalEvent::Warning(_)))
            .count()
    }
}

impl Extend<CanonicalEvent> for CanonicalBatch {
    fn extend<T: IntoIterator<Item = CanonicalEvent>>(&mut self, iter: T) {
        self.events.extend(iter);
    }
}

/// A fatal parse failure. Distinct from [`ParseWarning`], which is recoverable.
///
/// Carries a stable code plus detail and an optional location, and converts into
/// the shared [`prism_model::PrismError`] (as `Corrupt`) at the crate boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// Stable machine-readable code (e.g. `"invalid-json"`, `"budget-exceeded"`).
    pub code: String,
    /// Human-readable detail.
    pub detail: String,
    /// Optional location hint.
    pub location: Option<String>,
}

impl ParseError {
    /// Construct a parse error with no location.
    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: detail.into(),
            location: None,
        }
    }

    /// Attach a location hint.
    pub fn at(mut self, location: impl Into<String>) -> Self {
        self.location = Some(location.into());
        self
    }
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "parse error [{}]: {}", self.code, self.detail)?;
        if let Some(loc) = &self.location {
            write!(f, " (at {loc})")?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseError {}

impl From<ParseError> for prism_model::PrismError {
    fn from(e: ParseError) -> Self {
        prism_model::PrismError::Corrupt {
            detail: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_collects_in_order() {
        let mut b = CanonicalBatch::new();
        assert!(b.is_empty());
        b.push(CanonicalEvent::PackageSeen(PackageFact {
            name: PackageName::from("serde"),
            namespace: None,
            repository_url: None,
            homepage_url: None,
        }));
        b.push(CanonicalEvent::VersionSeen(VersionFact {
            name: PackageName::from("serde"),
            version_raw: "1.0.0".into(),
            released_at_unix: None,
            license_expr: Some("MIT".into()),
            is_prerelease: false,
        }));
        b.warn(ParseWarning::new("unknown-field", "ignored `foo`"));
        assert_eq!(b.len(), 3);
        assert_eq!(b.fact_count(), 2);
        assert!(!b.is_empty());
        // Order preserved.
        assert!(matches!(b.events[0], CanonicalEvent::PackageSeen(_)));
        assert!(matches!(b.events[2], CanonicalEvent::Warning(_)));
    }

    #[test]
    fn batch_extend_appends() {
        let mut b = CanonicalBatch::new();
        b.extend([
            CanonicalEvent::Warning(ParseWarning::new("a", "x")),
            CanonicalEvent::Warning(ParseWarning::new("b", "y")),
        ]);
        assert_eq!(b.len(), 2);
        assert_eq!(b.iter().count(), 2);
    }

    #[test]
    fn parse_error_displays_and_converts() {
        let e = ParseError::new("invalid-json", "unexpected eof").at("line 3");
        let s = e.to_string();
        assert!(s.contains("invalid-json"));
        assert!(s.contains("line 3"));
        let pe: prism_model::PrismError = e.into();
        assert!(matches!(pe, prism_model::PrismError::Corrupt { .. }));
    }

    #[test]
    fn checksum_trust_orders_user_pinned_highest() {
        assert!(ChecksumTrust::UserPinned > ChecksumTrust::TransparencyLog);
        assert!(ChecksumTrust::TransparencyLog > ChecksumTrust::RegistryMetadata);
    }
}
