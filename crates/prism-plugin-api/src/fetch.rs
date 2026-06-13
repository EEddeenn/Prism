//! Fetch-layer value types (architecture doc, *Fetcher Responsibilities*).
//!
//! A fetcher retrieves *raw* facts. It writes no graph objects and makes no
//! resolution decisions. These types describe what to fetch ([`FetchRequest`] /
//! [`FetchTarget`]), where from ([`RegistrySource`]), under what caching policy
//! ([`CachePolicy`]), and what comes back ([`RawDocument`] / [`Provenance`]).
//!
//! All URLs on this path are [`SafeUrl`] so credentials can never be stored,
//! logged, or surfaced; secrets are referenced indirectly by [`AuthRef`] and
//! resolved at fetch time from outside the workspace.

use prism_model::{EcosystemId, HashDigest, SnapshotId};
use std::path::PathBuf;

use crate::coord::{PackageName, VersionReqOrExact};
use crate::url::SafeUrl;

/// An **indirect** handle to a credential — never the secret itself.
///
/// The named secret is resolved at fetch time from an OS keychain, an
/// environment variable, or an external secrets file that lives *outside*
/// `.prism/` (architecture doc, *Credential handling*). Only the lookup key is
/// carried here, so an `AuthRef` is safe to keep in a request struct.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuthRef {
    /// Opaque lookup key (e.g. `"npm:registry.example"`), never a token value.
    pub key: String,
}

impl AuthRef {
    /// Construct an indirect credential handle from its lookup key.
    #[inline]
    pub fn new(key: impl Into<String>) -> Self {
        Self { key: key.into() }
    }
}

/// A registry endpoint to fetch from. The base URL is always a [`SafeUrl`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrySource {
    /// Stable identifier for this source (e.g. `"crates.io"`, `"npmjs"`).
    pub id: String,
    /// Redacted base URL of the registry/index.
    pub base_url: SafeUrl,
    /// Whether this source is a mirror permitted to use plain `http`.
    pub is_mirror: bool,
}

impl RegistrySource {
    /// A non-mirror HTTPS source.
    pub fn new(id: impl Into<String>, base_url: impl Into<SafeUrl>) -> Self {
        Self {
            id: id.into(),
            base_url: base_url.into(),
            is_mirror: false,
        }
    }
}

/// What to retrieve. Mirrors the three input modes plus artifact retrieval and
/// local-file ingestion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchTarget {
    /// The package-level index document (all versions of a family).
    PackageIndex {
        /// Un-normalized package name.
        package: PackageName,
    },
    /// Metadata for one version or requirement range of a package.
    VersionMetadata {
        /// Un-normalized package name.
        package: PackageName,
        /// Concrete version or requirement to resolve against the index.
        version: VersionReqOrExact,
    },
    /// Metadata *about* a distribution artifact (not its bytes).
    ArtifactMetadata {
        /// Redacted artifact URL (untrusted: from registry metadata).
        artifact_url: SafeUrl,
        /// Expected content hash, if the registry advertised one.
        expected_hash: Option<HashDigest>,
    },
    /// The artifact bytes themselves.
    Artifact {
        /// Redacted artifact URL (untrusted: from registry metadata).
        artifact_url: SafeUrl,
        /// Expected content hash, if the registry advertised one.
        expected_hash: Option<HashDigest>,
    },
    /// A document already present on disk (manifest, lockfile, …).
    LocalFile {
        /// Filesystem path to read.
        path: PathBuf,
    },
}

/// How a fetch may use (or must bypass) the content-addressed cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CachePolicy {
    /// Use cache if fresh, otherwise revalidate/fetch (the default).
    #[default]
    Default,
    /// Never touch the network; fail if not cached (offline mode).
    OfflineOnly,
    /// Ignore any cached copy and fetch fresh, then update the cache.
    Refresh,
    /// Use any cached copy unconditionally, even if stale.
    PreferCache,
}

/// One unit of work for the fetcher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchRequest {
    /// Owning ecosystem.
    pub ecosystem: EcosystemId,
    /// Where to fetch from.
    pub source: RegistrySource,
    /// What to fetch.
    pub target: FetchTarget,
    /// Indirect credential handle, if this source needs authentication.
    pub auth: Option<AuthRef>,
    /// Snapshot this fetch is pinned to, for reproducible runs.
    pub snapshot: Option<SnapshotId>,
    /// Caching behavior for this request.
    pub cache_policy: CachePolicy,
}

/// Where a [`RawDocument`] came from, for audit and reproducibility. Carries no
/// credentials by construction (all URLs are [`SafeUrl`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    /// The registry/source id this document was retrieved from.
    pub source_id: String,
    /// The redacted URL or path key it was retrieved by.
    pub retrieved_from: SafeUrl,
    /// True if served from the local content-addressed cache, not the network.
    pub from_cache: bool,
    /// Snapshot the fetch was pinned to, if any.
    pub snapshot: Option<SnapshotId>,
}

/// A raw fetched document handed to a parser. Deterministic content-addressing
/// (`content_hash`) lets the cache and store dedup identical payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawDocument {
    /// Stable key identifying this document within its source (redacted).
    pub source_key: SafeUrl,
    /// MIME type if the transport reported one.
    pub media_type: Option<String>,
    /// SHA-256 of `bytes` (the content address).
    pub content_hash: HashDigest,
    /// The raw document bytes.
    pub bytes: Vec<u8>,
    /// Unix timestamp the document was fetched.
    pub fetched_at_unix: i64,
    /// HTTP `ETag` for conditional requests, if provided.
    pub etag: Option<String>,
    /// HTTP `Last-Modified` for conditional requests, if provided.
    pub last_modified: Option<String>,
    /// Where this document came from.
    pub provenance: Provenance,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_ref_is_just_a_key() {
        let a = AuthRef::new("npm:registry.example");
        assert_eq!(a.key, "npm:registry.example");
        // The handle itself reveals no secret value.
        assert!(!format!("{a:?}").contains("token"));
    }

    #[test]
    fn cache_policy_default_is_default() {
        assert_eq!(CachePolicy::default(), CachePolicy::Default);
    }

    #[test]
    fn registry_source_base_url_is_redacted() {
        let s = RegistrySource::new("npmjs", SafeUrl::new("https://user:tok@reg.example/"));
        assert_eq!(s.base_url.as_str(), "https://reg.example/");
        assert!(!s.is_mirror);
    }

    #[test]
    fn fetch_target_variants_construct() {
        let _ = FetchTarget::PackageIndex {
            package: PackageName::from("serde"),
        };
        let _ = FetchTarget::VersionMetadata {
            package: PackageName::from("serde"),
            version: VersionReqOrExact::Exact("1.0.0".into()),
        };
        let t = FetchTarget::Artifact {
            artifact_url: SafeUrl::new("https://reg/x.tgz"),
            expected_hash: Some(HashDigest::default()),
        };
        if let FetchTarget::Artifact { artifact_url, .. } = t {
            assert_eq!(artifact_url.as_str(), "https://reg/x.tgz");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn fetch_request_builds() {
        let req = FetchRequest {
            ecosystem: EcosystemId::new(1),
            source: RegistrySource::new("crates.io", SafeUrl::new("https://index.crates.io/")),
            target: FetchTarget::PackageIndex {
                package: PackageName::from("serde"),
            },
            auth: None,
            snapshot: Some(SnapshotId::new(7)),
            cache_policy: CachePolicy::OfflineOnly,
        };
        assert_eq!(req.cache_policy, CachePolicy::OfflineOnly);
        assert_eq!(req.snapshot, Some(SnapshotId::new(7)));
    }
}
