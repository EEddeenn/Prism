//! The pnpm ecosystem plugin.
//!
//! pnpm installs the *same* registry packages as npm with the *same* SemVer
//! semantics — only the lockfile differs (`pnpm-lock.yaml`, a YAML file whose
//! `snapshots:` section is already a flat, fully-resolved dependency graph). So
//! this plugin parses that graph **directly** (one node per resolved unit, edges
//! verbatim) rather than reshaping it through npm's nested `node_modules` tree,
//! and reuses the npm plugin for everything else:
//!
//! - [`parse_pnpm_graph`] → a [`prism_plugin_api::ResolvedGraph`] at
//!   [`prism_model::FidelityTier::LockfileExact`]; [`parse_pnpm_units`] is the
//!   aligned identity column; [`parse_pnpm_batch`] the canonical facts.
//! - name normalization, version ordering, requirement matching, condition
//!   evaluation, and the `package.json` manifest are npm's
//!   ([`NpmNames`]/[`NpmVersions`]/[`NpmReqs`]/[`NpmConditions`]/[`NpmManifest`]).
//!
//! Each peer/patch context in `pnpm-lock.yaml` is a distinct resolved unit, so the
//! graph reproduces pnpm's topology exactly. Workspace packages are collapsed into
//! one synthetic root (the analysis is of the external dependency topology).
//!
//! Because pnpm packages are npm packages, [`PnpmPlugin`] is constructed with the
//! same [`EcosystemId`] as npm so interned identities and `pkg:npm/...` purls line
//! up across a `pnpm-lock.yaml`, a `bun.lock`, and a `package-lock.json` of the
//! same project. [`PnpmPlugin`] is object-safe; store it as `Box<dyn EcosystemPlugin>`.

mod detect;
mod lockfile;

pub use detect::PnpmDetector;
pub use lockfile::{parse_pnpm_batch, parse_pnpm_graph, parse_pnpm_units, PnpmLockfile, PnpmUnit};

use prism_model::{EcosystemCapabilities, EcosystemId};
use prism_plugin_api::{
    ConditionEvaluator, EcosystemPlugin, LockfileParser, ManifestParser, NameNormalizer,
    ProjectDetector, RequirementParser, VersionScheme,
};
use prism_plugin_npm::{NpmConditions, NpmManifest, NpmNames, NpmReqs, NpmVersions};

/// The pnpm ecosystem plugin: npm capability implementations with a
/// `pnpm-lock.yaml` lockfile parser and detector.
///
/// Construct with [`PnpmPlugin::new`], passing the same [`EcosystemId`] used for
/// npm so a `pnpm-lock.yaml` and a `package-lock.json` of the same project share
/// interned identities and purls.
#[derive(Debug, Clone)]
pub struct PnpmPlugin {
    id: EcosystemId,
    names: NpmNames,
    versions: NpmVersions,
    reqs: NpmReqs,
    conds: NpmConditions,
    detector: PnpmDetector,
    manifest: NpmManifest,
    lockfile: PnpmLockfile,
}

impl PnpmPlugin {
    /// Construct the plugin with the given [`EcosystemId`].
    pub fn new(id: EcosystemId) -> Self {
        Self {
            id,
            names: NpmNames,
            versions: NpmVersions,
            reqs: NpmReqs,
            conds: NpmConditions,
            detector: PnpmDetector,
            manifest: NpmManifest,
            lockfile: PnpmLockfile,
        }
    }
}

impl EcosystemPlugin for PnpmPlugin {
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

/// Compile-time object-safety assertions (mirrors the other plugins' checks).
#[allow(dead_code)]
mod object_safety {
    use super::*;

    fn _plugin(_: &dyn EcosystemPlugin) {}
    fn _is_boxable() {
        let _: Box<dyn EcosystemPlugin> = Box::new(PnpmPlugin::new(EcosystemId::new(2)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_safe_and_npm_semantics() {
        let p: Box<dyn EcosystemPlugin> = Box::new(PnpmPlugin::new(EcosystemId::new(2)));
        assert!(p.self_consistent());
        assert!(p.project_detector().is_some());
        assert!(p.lockfile_parser().is_some());
        assert!(p.manifest_parser().is_some());
        let caps = p.capabilities();
        assert!(caps.contains(EcosystemCapabilities::PARSE_LOCKFILE));
        // npm semantics, through the trait object.
        assert_eq!(p.version_scheme().parse("v1.2.3").unwrap(), "1.2.3");
        assert!(p.requirement_parser().matches("^1.2.0", "1.5.0").unwrap());
    }
}
