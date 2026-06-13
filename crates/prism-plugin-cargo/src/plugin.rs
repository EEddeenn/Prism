//! The [`CargoPlugin`] aggregator wiring every Cargo capability together.
//!
//! Mandatory capabilities (name/version/requirement/condition) are always
//! present; the optional manifest, lockfile, project-detection, and
//! candidate-enumeration capabilities are too. There is no registry fetcher or
//! resolver adapter in offline v0.1, so those accessors return `None` and their
//! flags are not advertised (keeping [`EcosystemPlugin::self_consistent`] true).

use prism_model::{EcosystemCapabilities, EcosystemId};
use prism_plugin_api::{
    CandidateProvider, ConditionEvaluator, EcosystemPlugin, LockfileParser, ManifestParser,
    NameNormalizer, ProjectDetector, RequirementParser, VersionScheme,
};

use crate::candidates::CargoCandidates;
use crate::condition::CargoConditions;
use crate::detect::CargoDetector;
use crate::lockfile::CargoLockfile;
use crate::manifest::CargoManifest;
use crate::names::CargoNames;
use crate::req::CargoReqs;
use crate::version::CargoVersions;

/// The Cargo ecosystem plugin (offline/local-only, v0.1).
pub struct CargoPlugin {
    id: EcosystemId,
    names: CargoNames,
    versions: CargoVersions,
    reqs: CargoReqs,
    conds: CargoConditions,
    detector: CargoDetector,
    manifest: CargoManifest,
    lockfile: CargoLockfile,
    candidates: CargoCandidates,
}

impl CargoPlugin {
    /// Construct the plugin with a caller-assigned [`EcosystemId`] and an empty
    /// offline candidate index.
    pub fn new(id: EcosystemId) -> Self {
        Self::with_candidates(id, CargoCandidates::new())
    }

    /// Construct the plugin with a pre-populated candidate index (e.g. seeded
    /// from a vendored mirror or a lockfile).
    pub fn with_candidates(id: EcosystemId, candidates: CargoCandidates) -> Self {
        Self {
            id,
            names: CargoNames,
            versions: CargoVersions,
            reqs: CargoReqs,
            conds: CargoConditions,
            detector: CargoDetector,
            manifest: CargoManifest,
            lockfile: CargoLockfile,
            candidates,
        }
    }
}

impl EcosystemPlugin for CargoPlugin {
    fn id(&self) -> EcosystemId {
        self.id
    }

    fn capabilities(&self) -> EcosystemCapabilities {
        EcosystemCapabilities::PARSE_MANIFEST
            | EcosystemCapabilities::PARSE_LOCKFILE
            | EcosystemCapabilities::ENUM_CANDIDATES
            | EcosystemCapabilities::SUPPORTS_WORKSPACE
            | EcosystemCapabilities::SUPPORTS_FEATURES
            | EcosystemCapabilities::SUPPORTS_PLATFORM
            | EcosystemCapabilities::SUPPORTS_YANKED
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
    fn candidate_provider(&self) -> Option<&dyn CandidateProvider> {
        Some(&self.candidates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Cargo ecosystem id used in tests.
    const CARGO_ECO: EcosystemId = EcosystemId::new(2);

    fn boxed() -> Box<dyn EcosystemPlugin> {
        Box::new(CargoPlugin::new(CARGO_ECO))
    }

    #[test]
    fn stores_as_boxed_trait_object() {
        let p: Box<dyn EcosystemPlugin> = boxed();
        assert_eq!(p.id(), CARGO_ECO);
    }

    #[test]
    fn capability_flags_are_self_consistent() {
        let p = boxed();
        assert!(p.self_consistent());
        let caps = p.capabilities();
        assert!(caps.contains(EcosystemCapabilities::PARSE_MANIFEST));
        assert!(caps.contains(EcosystemCapabilities::PARSE_LOCKFILE));
        assert!(caps.contains(EcosystemCapabilities::ENUM_CANDIDATES));
        assert!(caps.contains(EcosystemCapabilities::SUPPORTS_FEATURES));
        assert!(caps.contains(EcosystemCapabilities::SUPPORTS_PLATFORM));
        // Offline: no fetcher / native resolve advertised.
        assert!(!caps.contains(EcosystemCapabilities::FETCH_REGISTRY));
        assert!(!caps.contains(EcosystemCapabilities::NATIVE_RESOLVE));
        // Accessors agree with flags.
        assert!(p.registry_fetcher().is_none());
        assert!(p.resolver_adapter().is_none());
        assert!(p.manifest_parser().is_some());
        assert!(p.candidate_provider().is_some());
    }

    #[test]
    fn mandatory_capabilities_reachable_through_dyn() {
        let p = boxed();
        assert_eq!(
            p.name_normalizer().normalize("Serde-JSON").unwrap(),
            "serde_json"
        );
        assert!(p.version_scheme().is_prerelease("1.0.0-rc.1").unwrap());
        assert!(p.requirement_parser().matches("^1", "1.4.0").unwrap());
        assert_eq!(
            p.condition_evaluator()
                .classify("cfg(unix)")
                .unwrap(),
            prism_model::ConditionKind::Environment
        );
    }
}
