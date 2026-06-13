//! The npm ecosystem plugin (architecture doc, *npm Plugin*).
//!
//! Offline / local-only for v0.1: it parses `package.json` and
//! `package-lock.json` (v2/v3) entirely in-process via [`prism_textfmt`], with
//! no network access. It bundles the capability traits required by
//! [`prism_plugin_api::EcosystemPlugin`]:
//!
//! - [`NpmNames`] — name normalization: lowercases the `@scope`, preserves
//!   unscoped case, and exposes a case-folded [`NpmNames::collision_key`].
//! - [`NpmVersions`] — SemVer 2.0 scheme delegated to [`prism_semver`].
//! - [`NpmReqs`] — npm range syntax via [`prism_semver::VersionReq`], plus
//!   non-range protocol classification ([`req::ReqProtocol`]).
//! - [`NpmConditions`] — `engines`/`os`/`cpu` as **environment** conditions.
//! - [`NpmDetector`] — a project is npm iff `package.json` is present.
//! - [`NpmManifest`] — `package.json` → [`prism_plugin_api::CanonicalBatch`],
//!   with `peerDependencies` marked [`prism_model::DependencyKind::Peer`].
//! - [`NpmLockfile`] — `package-lock.json` → facts, and
//!   [`lockfile::parse_package_lock`] → a [`prism_plugin_api::ResolvedGraph`]
//!   at [`prism_model::FidelityTier::LockfileExact`].
//! - [`LockfileResolver`] — a native [`prism_plugin_api::ResolverAdapter`] that
//!   replays a captured lockfile.
//!
//! [`NpmPlugin`] is the object-safe aggregator; store it as
//! `Box<dyn EcosystemPlugin>`.

mod cond;
mod detect;
mod manifest;
mod name;
pub mod lockfile;
pub mod req;
mod resolver;
mod version;

pub use cond::NpmConditions;
pub use detect::NpmDetector;
pub use lockfile::{
    lockfile_facts_value, name_from_path, parse_package_lock, parse_package_lock_value, NpmLockfile,
};
pub use manifest::NpmManifest;
pub use name::NpmNames;
pub use req::NpmReqs;
pub use resolver::LockfileResolver;
pub use version::NpmVersions;

use prism_model::{EcosystemCapabilities, EcosystemId};
use prism_plugin_api::{
    ConditionEvaluator, EcosystemPlugin, LockfileParser, ManifestParser, NameNormalizer,
    ProjectDetector, RequirementParser, VersionScheme,
};

/// The npm ecosystem plugin: the object-safe aggregator over the npm capability
/// implementations.
///
/// Construct with [`NpmPlugin::new`] (allocating a process-local
/// [`EcosystemId`]). The resolver adapter is *not* exposed through the trait —
/// a native lockfile resolution needs the lockfile bytes, so build a
/// [`LockfileResolver`] per document instead.
#[derive(Debug, Clone)]
pub struct NpmPlugin {
    id: EcosystemId,
    names: NpmNames,
    versions: NpmVersions,
    reqs: NpmReqs,
    conds: NpmConditions,
    detector: NpmDetector,
    manifest: NpmManifest,
    lockfile: NpmLockfile,
}

impl NpmPlugin {
    /// Construct the plugin with the given [`EcosystemId`].
    pub fn new(id: EcosystemId) -> Self {
        Self {
            id,
            names: NpmNames,
            versions: NpmVersions,
            reqs: NpmReqs,
            conds: NpmConditions,
            detector: NpmDetector,
            manifest: NpmManifest,
            lockfile: NpmLockfile,
        }
    }
}

impl EcosystemPlugin for NpmPlugin {
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

/// Compile-time object-safety assertions. These never run; they only type-check
/// that each trait object is well-formed (a broken object-safety would fail the
/// build here, mirroring `prism-plugin-api`'s own checks).
#[allow(dead_code)]
mod object_safety {
    use super::*;

    fn _plugin(_: &dyn EcosystemPlugin) {}
    fn _boxed(p: Box<dyn EcosystemPlugin>) -> Box<dyn EcosystemPlugin> {
        p
    }
    fn _is_boxable() {
        let _: Box<dyn EcosystemPlugin> = Box::new(NpmPlugin::new(EcosystemId::new(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_model::DependencyKind;
    use prism_plugin_api::{
        CanonicalEvent, Provenance, RawDocument, RequirementFact, SafeUrl,
    };

    fn plugin() -> Box<dyn EcosystemPlugin> {
        Box::new(NpmPlugin::new(EcosystemId::new(42)))
    }

    fn doc(bytes: &str) -> RawDocument {
        RawDocument {
            source_key: SafeUrl::new("file:///x.json"),
            media_type: Some("application/json".into()),
            content_hash: prism_model::HashDigest::default(),
            bytes: bytes.as_bytes().to_vec(),
            fetched_at_unix: 0,
            etag: None,
            last_modified: None,
            provenance: Provenance {
                source_id: "local".into(),
                retrieved_from: SafeUrl::new("file:///x.json"),
                from_cache: false,
                snapshot: None,
            },
        }
    }

    #[test]
    fn object_safe_and_boxable() {
        let p = plugin();
        assert_eq!(p.id(), EcosystemId::new(42));
        // self_consistent verifies the advertised flags match the Some() accessors.
        assert!(p.self_consistent());
        let caps = p.capabilities();
        assert!(caps.contains(EcosystemCapabilities::PARSE_MANIFEST));
        assert!(caps.contains(EcosystemCapabilities::PARSE_LOCKFILE));
        assert!(caps.contains(EcosystemCapabilities::SUPPORTS_WORKSPACE));
        // Not advertised → no resolver/fetch/candidate accessor.
        assert!(p.resolver_adapter().is_none());
        assert!(p.registry_fetcher().is_none());
        assert!(p.candidate_provider().is_none());
    }

    #[test]
    fn mandatory_capabilities_through_dyn() {
        let p = plugin();
        // Scoped-name normalization through the trait object.
        assert_eq!(
            p.name_normalizer().normalize("@MyOrg/Pkg").unwrap(),
            "@myorg/Pkg"
        );
        assert!(p.name_normalizer().names_equal("@A/b", "@a/b"));
        assert_eq!(p.version_scheme().parse("v1.2.3").unwrap(), "1.2.3");
        assert!(p.requirement_parser().matches("^1.2.0", "1.5.0").unwrap());
    }

    #[test]
    fn manifest_parser_marks_peer_kind() {
        let src = r#"{
          "name": "app",
          "version": "1.0.0",
          "dependencies": { "left-pad": "^1.3.0" },
          "devDependencies": { "jest": "~29.0.0" },
          "optionalDependencies": { "fsevents": "^2.3.0" },
          "peerDependencies": { "react": ">=17" }
        }"#;
        let p = plugin();
        let batch = p.manifest_parser().unwrap().parse_manifest(&doc(src)).unwrap();
        let reqs: Vec<&RequirementFact> = batch
            .iter()
            .filter_map(|e| match e {
                CanonicalEvent::RequirementSeen(r) => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(reqs.len(), 4);
        let kinds: Vec<(&str, DependencyKind)> =
            reqs.iter().map(|r| (r.declared_target.as_str(), r.kind)).collect();
        assert!(kinds.contains(&("left-pad", DependencyKind::Runtime)));
        assert!(kinds.contains(&("jest", DependencyKind::Dev)));
        assert!(kinds.contains(&("fsevents", DependencyKind::Optional)));
        // The peer dependency is DependencyKind::Peer.
        assert!(kinds.contains(&("react", DependencyKind::Peer)));
    }

    #[test]
    fn lockfile_to_resolved_graph_through_helper() {
        let src = r#"{
          "lockfileVersion": 3,
          "packages": {
            "": { "version": "1.0.0", "dependencies": { "left-pad": "^1.3.0" } },
            "node_modules/left-pad": { "version": "1.3.0" }
          }
        }"#;
        let g = parse_package_lock(&doc(src)).unwrap();
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 1);
        assert_eq!(
            g.fidelity.unwrap().tier,
            prism_model::FidelityTier::LockfileExact
        );
        // And the LockfileParser capability emits facts for the same input.
        let p = plugin();
        let batch = p.lockfile_parser().unwrap().parse_lockfile(&doc(src)).unwrap();
        assert!(batch.fact_count() >= 1);
    }
}
