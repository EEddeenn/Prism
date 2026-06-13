//! The uv / PyPI ecosystem plugin.
//!
//! uv resolves Python (PyPI) packages and writes a TOML `uv.lock`. Python is not
//! SemVer, so this plugin implements the Python semantics itself rather than
//! delegating to [`prism_semver`]:
//!
//! - [`UvNames`] — PEP 503 name normalization (lowercase, collapse `-_.` to `-`).
//! - [`UvVersions`] — PEP 440 versions (epoch/release/pre/post/dev/local).
//! - [`UvReqs`] — PEP 440 version-specifier matching (`~=`, `==X.*`, …).
//! - [`UvConditions`] — PEP 508 markers (`extra ==` is activation, else environment).
//! - [`UvDetector`] — a project is uv iff it has a `uv.lock`.
//! - [`UvLockfile`] — `uv.lock` → facts, and [`parse_uv_graph`] → a
//!   [`prism_plugin_api::ResolvedGraph`] at [`prism_model::FidelityTier::LockfileExact`].
//!
//! There is no manifest parser (a `pyproject.toml` is ambiguous across tools) and
//! no registry fetch/native-resolve in offline v0.1, so those accessors are
//! `None` and their flags are not advertised.
//!
//! [`UvPlugin`] is the object-safe aggregator; store it as
//! `Box<dyn EcosystemPlugin>`.

mod condition;
mod detect;
mod lockfile;
mod names;
mod req;
mod version;

pub use condition::UvConditions;
pub use detect::UvDetector;
pub use lockfile::{parse_uv_batch, parse_uv_graph, parse_uv_packages, UvLockfile, UvDep, UvPackage};
pub use names::{normalize_pep503, UvNames};
pub use req::UvReqs;
pub use version::UvVersions;

use prism_model::{EcosystemCapabilities, EcosystemId};
use prism_plugin_api::{
    ConditionEvaluator, EcosystemPlugin, LockfileParser, NameNormalizer, ProjectDetector,
    RequirementParser, VersionScheme,
};

/// The uv / PyPI ecosystem plugin (offline/local-only, v0.1).
pub struct UvPlugin {
    id: EcosystemId,
    names: UvNames,
    versions: UvVersions,
    reqs: UvReqs,
    conds: UvConditions,
    detector: UvDetector,
    lockfile: UvLockfile,
}

impl UvPlugin {
    /// Construct the plugin with a caller-assigned [`EcosystemId`].
    pub fn new(id: EcosystemId) -> Self {
        Self {
            id,
            names: UvNames,
            versions: UvVersions,
            reqs: UvReqs,
            conds: UvConditions,
            detector: UvDetector,
            lockfile: UvLockfile,
        }
    }
}

impl EcosystemPlugin for UvPlugin {
    fn id(&self) -> EcosystemId {
        self.id
    }

    fn capabilities(&self) -> EcosystemCapabilities {
        EcosystemCapabilities::PARSE_LOCKFILE
            | EcosystemCapabilities::SUPPORTS_WORKSPACE
            | EcosystemCapabilities::SUPPORTS_FEATURES
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
    fn lockfile_parser(&self) -> Option<&dyn LockfileParser> {
        Some(&self.lockfile)
    }
}

/// Compile-time object-safety assertions (mirrors the other plugins' checks).
#[allow(dead_code)]
mod object_safety {
    use super::*;

    fn _plugin(_: &dyn EcosystemPlugin) {}
    fn _is_boxable() {
        let _: Box<dyn EcosystemPlugin> = Box::new(UvPlugin::new(EcosystemId::new(3)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed() -> Box<dyn EcosystemPlugin> {
        Box::new(UvPlugin::new(EcosystemId::new(3)))
    }

    #[test]
    fn object_safe_and_consistent() {
        let p = boxed();
        assert_eq!(p.id(), EcosystemId::new(3));
        assert!(p.self_consistent());
        let caps = p.capabilities();
        assert!(caps.contains(EcosystemCapabilities::PARSE_LOCKFILE));
        assert!(caps.contains(EcosystemCapabilities::SUPPORTS_PLATFORM));
        // Offline, lockfile-only: no manifest / candidate / fetch / resolve.
        assert!(!caps.contains(EcosystemCapabilities::PARSE_MANIFEST));
        assert!(p.manifest_parser().is_none());
        assert!(p.candidate_provider().is_none());
        assert!(p.registry_fetcher().is_none());
        assert!(p.resolver_adapter().is_none());
    }

    #[test]
    fn python_semantics_through_dyn() {
        let p = boxed();
        assert_eq!(p.name_normalizer().normalize("Typing_Extensions").unwrap(), "typing-extensions");
        assert!(p.version_scheme().is_prerelease("1.0rc1").unwrap());
        // PEP 440 ordering through the trait object.
        assert_eq!(
            p.version_scheme().compare("1.9", "1.10").unwrap(),
            std::cmp::Ordering::Less
        );
        assert!(p.requirement_parser().matches("~=1.2", "1.9").unwrap());
        assert_eq!(
            p.condition_evaluator().classify("extra == 'cli'").unwrap(),
            prism_model::ConditionKind::Activation
        );
    }
}
