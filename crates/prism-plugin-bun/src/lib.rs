//! The Bun ecosystem plugin.
//!
//! Bun installs the *same* registry packages as npm with the *same* SemVer
//! semantics — only the lockfile format differs (`bun.lock`, a JSONC text file,
//! vs npm's `package-lock.json`). So this plugin is thin: it reshapes a
//! `bun.lock` into an npm `package-lock.json` value ([`bun_lock_to_npm_value`])
//! and reuses the npm plugin for everything downstream:
//!
//! - [`parse_bun_lock`] → a [`prism_plugin_api::ResolvedGraph`] via
//!   [`prism_plugin_npm::parse_package_lock_value`].
//! - [`BunLockfile`] → version/requirement facts via
//!   [`prism_plugin_npm::lockfile_facts_value`].
//! - name normalization, version ordering, requirement matching, and condition
//!   evaluation are npm's ([`NpmNames`]/[`NpmVersions`]/[`NpmReqs`]/
//!   [`NpmConditions`]); the manifest is npm's `package.json` ([`NpmManifest`]).
//!
//! Because the reshaped document is an exact npm lockfile, a Bun run is
//! [`prism_model::FidelityTier::LockfileExact`] with the same
//! `NO_PEER_PLACEMENT` caveat npm carries.
//!
//! [`BunPlugin`] is the object-safe aggregator; store it as
//! `Box<dyn EcosystemPlugin>`.

mod convert;
mod detect;

pub use convert::{bun_lock_to_npm_value, strip_jsonc};
pub use detect::BunDetector;

use prism_model::{EcosystemCapabilities, EcosystemId};
use prism_plugin_api::{
    CanonicalBatch, ConditionEvaluator, EcosystemPlugin, LockfileParser, ManifestParser,
    NameNormalizer, ParseError, ProjectDetector, RawDocument, RequirementParser, ResolvedGraph,
    VersionScheme,
};
use prism_plugin_npm::{
    lockfile_facts_value, parse_package_lock_value, NpmConditions, NpmManifest, NpmNames, NpmReqs,
    NpmVersions,
};

/// Parse a `bun.lock` document into a resolved graph (reusing npm's resolver).
pub fn parse_bun_lock(raw: &RawDocument) -> Result<ResolvedGraph, ParseError> {
    let value = bun_lock_to_npm_value(&raw.bytes)?;
    parse_package_lock_value(&value)
}

/// Bun [`LockfileParser`] for `bun.lock`: reshape to npm, then emit npm facts.
#[derive(Debug, Default, Clone, Copy)]
pub struct BunLockfile;

impl LockfileParser for BunLockfile {
    fn parse_lockfile(&self, raw: &RawDocument) -> Result<CanonicalBatch, ParseError> {
        let value = bun_lock_to_npm_value(&raw.bytes)?;
        lockfile_facts_value(&value)
    }
}

/// The Bun ecosystem plugin: npm capability implementations with a `bun.lock`
/// lockfile parser and detector.
///
/// Construct with [`BunPlugin::new`]. Because Bun packages are npm packages,
/// pass the same [`EcosystemId`] used for npm so interned identities and purls
/// line up across a `bun.lock` and a `package-lock.json` of the same project.
#[derive(Debug, Clone)]
pub struct BunPlugin {
    id: EcosystemId,
    names: NpmNames,
    versions: NpmVersions,
    reqs: NpmReqs,
    conds: NpmConditions,
    detector: BunDetector,
    manifest: NpmManifest,
    lockfile: BunLockfile,
}

impl BunPlugin {
    /// Construct the plugin with the given [`EcosystemId`].
    pub fn new(id: EcosystemId) -> Self {
        Self {
            id,
            names: NpmNames,
            versions: NpmVersions,
            reqs: NpmReqs,
            conds: NpmConditions,
            detector: BunDetector,
            manifest: NpmManifest,
            lockfile: BunLockfile,
        }
    }
}

impl EcosystemPlugin for BunPlugin {
    fn id(&self) -> EcosystemId {
        self.id
    }

    fn capabilities(&self) -> EcosystemCapabilities {
        EcosystemCapabilities::PARSE_MANIFEST
            | EcosystemCapabilities::PARSE_LOCKFILE
            | EcosystemCapabilities::SUPPORTS_WORKSPACE
            | EcosystemCapabilities::SUPPORTS_PLATFORM
    }

    fn name_normalizer(&self) -> &dyn NameNormalizer {
        &self.names
    }
    fn version_scheme(&self) -> &dyn VersionScheme {
        &self.versions
    }
    fn requirement_parser(&self) -> &dyn RequirementParser {
        &self.reqs
    }
    fn condition_evaluator(&self) -> &dyn ConditionEvaluator {
        &self.conds
    }
    fn project_detector(&self) -> Option<&dyn ProjectDetector> {
        Some(&self.detector)
    }
    fn manifest_parser(&self) -> Option<&dyn ManifestParser> {
        Some(&self.manifest)
    }
    fn lockfile_parser(&self) -> Option<&dyn LockfileParser> {
        Some(&self.lockfile)
    }
}

/// Compile-time object-safety assertions (mirrors the npm plugin's own check).
#[allow(dead_code)]
mod object_safety {
    use super::*;

    fn _plugin(_: &dyn EcosystemPlugin) {}
    fn _is_boxable() {
        let _: Box<dyn EcosystemPlugin> = Box::new(BunPlugin::new(EcosystemId::new(2)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_plugin_api::{CanonicalEvent, Provenance, SafeUrl};

    fn doc(bytes: &str) -> RawDocument {
        RawDocument {
            source_key: SafeUrl::new("file:///bun.lock"),
            media_type: Some("text/plain".into()),
            content_hash: prism_model::HashDigest::default(),
            bytes: bytes.as_bytes().to_vec(),
            fetched_at_unix: 0,
            etag: None,
            last_modified: None,
            provenance: Provenance {
                source_id: "local".into(),
                retrieved_from: SafeUrl::new("file:///bun.lock"),
                from_cache: false,
                snapshot: None,
            },
        }
    }

    const BUN_LOCK: &str = r#"{
      "lockfileVersion": 1,
      "workspaces": {
        "": { "name": "app", "dependencies": { "left-pad": "1.3.0" }, "devDependencies": { "typescript": "5.0.0" } },
      },
      "packages": {
        "left-pad": ["left-pad@1.3.0", "", {}, "sha512-a"],
        "typescript": ["typescript@5.0.0", "", {}, "sha512-b"],
      },
    }"#;

    #[test]
    fn plugin_is_object_safe_and_consistent() {
        let p: Box<dyn EcosystemPlugin> = Box::new(BunPlugin::new(EcosystemId::new(2)));
        assert!(p.self_consistent());
        assert!(p.project_detector().is_some());
        assert!(p.lockfile_parser().is_some());
        // npm semantics, through the trait object.
        assert_eq!(p.version_scheme().parse("v1.2.3").unwrap(), "1.2.3");
        assert!(p.requirement_parser().matches("^1.2.0", "1.5.0").unwrap());
    }

    #[test]
    fn parses_graph_and_facts() {
        let g = parse_bun_lock(&doc(BUN_LOCK)).unwrap();
        // root + left-pad + typescript
        assert_eq!(g.node_count(), 3);
        assert_eq!(
            g.fidelity.unwrap().tier,
            prism_model::FidelityTier::LockfileExact
        );

        let batch = BunLockfile.parse_lockfile(&doc(BUN_LOCK)).unwrap();
        let versions: Vec<&str> = batch
            .iter()
            .filter_map(|e| match e {
                CanonicalEvent::VersionSeen(v) => Some(v.name.as_str()),
                _ => None,
            })
            .collect();
        assert!(versions.contains(&"left-pad"));
        assert!(versions.contains(&"typescript"));
    }
}
