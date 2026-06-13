//! Capability sub-traits that make up an [`crate::EcosystemPlugin`].
//!
//! Each trait is a single, focused capability (name normalization, version
//! scheme, parsing, fetching, resolving). They are all reached through
//! `&dyn Trait` from the aggregator, so **every method is object-safe**: no
//! generic parameters, no `Self`-returning methods, no `async fn`. The one
//! asynchronous capability ([`ResolverAdapter::resolve`]) is written as an
//! explicit boxed `Send` future to stay object-safe without `async-trait`.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use prism_model::{ConditionKind, GraphFidelity};

use crate::activation::GateInput;
use crate::fetch::{FetchRequest, RawDocument};
use crate::parse::{CanonicalBatch, ParseError};
use crate::resolve::{ResolveContext, ResolveRequest, ResolveError, ResolvedGraph};

/// The total order of two values under an ecosystem's version scheme.
pub type VersionOrdering = core::cmp::Ordering;

/// Normalizes raw package names to the single canonical identity key for a
/// family (architecture doc: the `Package.canonical_name` is the output of this).
pub trait NameNormalizer: Send + Sync {
    /// Canonicalize a raw name (e.g. PyPI's PEP 503 fold, npm scope handling).
    /// Returns [`ParseError`] for names the ecosystem rejects outright.
    fn normalize(&self, raw: &str) -> Result<String, ParseError>;

    /// True if two raw names denote the same family after normalization.
    fn names_equal(&self, a: &str, b: &str) -> bool {
        match (self.normalize(a), self.normalize(b)) {
            (Ok(na), Ok(nb)) => na == nb,
            _ => false,
        }
    }
}

/// Parses, normalizes, compares, and totally-orders an ecosystem's versions.
///
/// `sort_key` yields the ecosystem-specific byte key stored in
/// [`prism_model::PackageVersion::version_sort_key`]; comparing two keys
/// lexicographically MUST agree with [`VersionScheme::compare`].
pub trait VersionScheme: Send + Sync {
    /// Validate a raw version string, returning its normalized form.
    fn parse(&self, raw: &str) -> Result<String, ParseError>;

    /// Normalize a raw version (e.g. strip a leading `v`, pad implicit zeros).
    /// Equivalent to [`VersionScheme::parse`] but the name documents intent.
    fn normalize(&self, raw: &str) -> Result<String, ParseError> {
        self.parse(raw)
    }

    /// Total order between two raw versions. Implementations MUST be consistent
    /// with the byte ordering produced by [`VersionScheme::sort_key`].
    fn compare(&self, a: &str, b: &str) -> Result<VersionOrdering, ParseError>;

    /// The total-order byte key for a version (lexicographic == version order).
    fn sort_key(&self, raw: &str) -> Result<Vec<u8>, ParseError>;

    /// True if the version is a prerelease per this scheme.
    fn is_prerelease(&self, raw: &str) -> Result<bool, ParseError>;
}

/// Parses a raw requirement/constraint expression and tests membership.
///
/// The parsed AST is ecosystem-specific and content-addressed by the writer, so
/// this trait does not expose it directly; it exposes the queries the resolver
/// needs (`matches`) plus a normalized canonical form for `RequirementId`.
pub trait RequirementParser: Send + Sync {
    /// Validate a raw requirement, returning a canonical normalized string used
    /// for `RequirementId` canonicalization.
    fn canonicalize(&self, raw: &str) -> Result<String, ParseError>;

    /// True if `version` (raw) satisfies the requirement `raw`.
    fn matches(&self, raw: &str, version: &str) -> Result<bool, ParseError>;
}

/// Evaluates a requirement's condition against the appropriate gate state
/// (architecture doc: environment markers vs feature/extra activation).
pub trait ConditionEvaluator: Send + Sync {
    /// Evaluate the raw condition `raw` against `input`. A `None` condition is
    /// always active; the evaluator may assert that `input.kind()` matches the
    /// declared [`ConditionKind`] of the requirement.
    fn evaluate(&self, raw: Option<&str>, input: GateInput<'_>) -> Result<bool, ParseError>;

    /// Classify a raw condition expression into the gate kind it must be
    /// evaluated under (e.g. `extra == "x"` → [`ConditionKind::Activation`],
    /// `sys_platform == "linux"` → [`ConditionKind::Environment`]).
    fn classify(&self, raw: &str) -> Result<ConditionKind, ParseError>;
}

/// Detects whether a directory is a project this ecosystem owns and which input
/// files drive it (optional capability).
pub trait ProjectDetector: Send + Sync {
    /// True if `root` looks like a project for this ecosystem.
    fn detects(&self, root: &Path) -> bool;

    /// Manifest filenames this ecosystem reads, in preference order.
    fn manifest_filenames(&self) -> &'static [&'static str];

    /// Lockfile filenames this ecosystem reads, in preference order.
    fn lockfile_filenames(&self) -> &'static [&'static str];
}

/// Parses a manifest document into canonical events (declared requirements,
/// features, workspace info). Optional capability.
pub trait ManifestParser: Send + Sync {
    /// Parse a manifest [`RawDocument`] into a [`CanonicalBatch`].
    fn parse_manifest(&self, raw: &RawDocument) -> Result<CanonicalBatch, ParseError>;
}

/// Parses a lockfile document into canonical events (exact resolved versions).
/// Optional capability.
pub trait LockfileParser: Send + Sync {
    /// Parse a lockfile [`RawDocument`] into a [`CanonicalBatch`].
    fn parse_lockfile(&self, raw: &RawDocument) -> Result<CanonicalBatch, ParseError>;
}

/// Retrieves raw registry facts. Optional capability.
///
/// `fetch` is asynchronous and reached via `&dyn`, so it is written as a boxed
/// `Send` future to remain object-safe and runtime-agnostic.
pub trait RegistryFetcher: Send + Sync {
    /// Fetch one [`FetchRequest`], returning the raw bytes plus provenance.
    fn fetch<'a>(
        &'a self,
        request: FetchRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RawDocument, FetchError>> + Send + 'a>>;
}

/// Parses registry metadata documents into canonical events (architecture doc,
/// *Parser Responsibilities*). Optional capability.
pub trait RegistryMetadataParser: Send + Sync {
    /// Parse a package-level index document.
    fn parse_package_index(&self, raw: &RawDocument) -> Result<CanonicalBatch, ParseError>;

    /// Parse a single version's metadata.
    fn parse_version_metadata(&self, raw: &RawDocument) -> Result<CanonicalBatch, ParseError>;

    /// Parse an artifact-metadata document (wheel METADATA, nuspec, …).
    fn parse_artifact_metadata(&self, raw: &RawDocument) -> Result<CanonicalBatch, ParseError>;
}

/// Enumerates candidate versions for a requirement (Candidate Version Space
/// mode). Optional capability.
pub trait CandidateProvider: Send + Sync {
    /// The sorted (ascending) list of raw version strings known for a
    /// normalized package name, as of the provider's pinned snapshot.
    fn versions_for(&self, canonical_name: &str) -> Result<Vec<String>, ParseError>;
}

/// Produces a resolved graph from requirements. Optional capability.
///
/// Native adapters parse lockfiles / call ecosystem tools; synthetic adapters
/// implement Prism's resolver policies. `resolve` is async and reached via
/// `&dyn`, so it is a boxed `Send` future to stay object-safe.
pub trait ResolverAdapter: Send + Sync {
    /// The fidelity this adapter achieves (tier + dropped-semantics caveats).
    fn fidelity(&self) -> GraphFidelity;

    /// Resolve `request` under `ctx`, producing a plain [`ResolvedGraph`].
    fn resolve<'a>(
        &'a self,
        request: ResolveRequest,
        ctx: &'a ResolveContext,
    ) -> Pin<Box<dyn Future<Output = Result<ResolvedGraph, ResolveError>> + Send + 'a>>;
}

/// A fatal fetch failure. Kept separate from [`ParseError`]/[`ResolveError`] so
/// the fetch path can carry transport-specific detail; converts into the shared
/// [`prism_model::PrismError`] at the boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    /// The target was not found at the source.
    NotFound,
    /// A network/transport-level failure (message is already redacted).
    Transport {
        /// Human-readable detail (must not contain credentials).
        detail: String,
    },
    /// The request was blocked by the SSRF/security guard.
    Blocked {
        /// Human-readable detail.
        detail: String,
    },
    /// A content hash did not match the expected value.
    ChecksumMismatch {
        /// Human-readable detail.
        detail: String,
    },
    /// Offline mode and the document was not in the cache.
    OfflineMiss,
    /// A fallback for failures without a dedicated variant.
    Other(String),
}

impl core::fmt::Display for FetchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FetchError::NotFound => write!(f, "fetch target not found"),
            FetchError::Transport { detail } => write!(f, "transport error: {detail}"),
            FetchError::Blocked { detail } => write!(f, "blocked by security guard: {detail}"),
            FetchError::ChecksumMismatch { detail } => write!(f, "checksum mismatch: {detail}"),
            FetchError::OfflineMiss => write!(f, "offline: document not cached"),
            FetchError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for FetchError {}

impl From<FetchError> for prism_model::PrismError {
    fn from(e: FetchError) -> Self {
        match e {
            FetchError::NotFound | FetchError::OfflineMiss => prism_model::PrismError::NotFound,
            FetchError::ChecksumMismatch { detail } => {
                prism_model::PrismError::ChecksumMismatch { detail }
            }
            other => prism_model::PrismError::Other(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_error_converts() {
        assert!(matches!(
            prism_model::PrismError::from(FetchError::NotFound),
            prism_model::PrismError::NotFound
        ));
        assert!(matches!(
            prism_model::PrismError::from(FetchError::OfflineMiss),
            prism_model::PrismError::NotFound
        ));
        assert!(matches!(
            prism_model::PrismError::from(FetchError::ChecksumMismatch { detail: "x".into() }),
            prism_model::PrismError::ChecksumMismatch { .. }
        ));
    }

    #[test]
    fn fetch_error_displays() {
        assert!(FetchError::OfflineMiss.to_string().contains("offline"));
        assert!(FetchError::Blocked { detail: "rfc1918".into() }
            .to_string()
            .contains("blocked"));
    }
}
