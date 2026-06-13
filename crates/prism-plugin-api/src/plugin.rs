//! The [`EcosystemPlugin`] aggregator trait (architecture doc, *Plugin Trait*).
//!
//! A plugin is not merely a parser: it is a bundle of ecosystem-specific
//! capabilities. The aggregator hands out each capability as a borrowed trait
//! object — `&dyn Cap` for the mandatory four, `Option<&dyn Cap>` for the rest —
//! and advertises which capabilities it actually implements via
//! [`prism_model::EcosystemCapabilities`]. The whole trait is object-safe, so a
//! registry can hold `Box<dyn EcosystemPlugin>`.

use prism_model::{EcosystemCapabilities, EcosystemId};

use crate::capabilities::{
    CandidateProvider, ConditionEvaluator, LockfileParser, ManifestParser, NameNormalizer,
    ProjectDetector, RegistryFetcher, RegistryMetadataParser, RequirementParser, ResolverAdapter,
    VersionScheme,
};

/// An ecosystem plugin: the object-safe aggregator over capability sub-traits.
///
/// The four name/version/requirement/condition capabilities are always present;
/// parsing, fetching, candidate-enumeration, and resolution are optional and
/// returned as `Option<&dyn ...>`. `capabilities()` MUST agree with which
/// optional accessors return `Some` (see [`EcosystemPlugin::self_consistent`]).
pub trait EcosystemPlugin: Send + Sync {
    /// This plugin's ecosystem id.
    fn id(&self) -> EcosystemId;

    /// The capability flags this plugin advertises.
    fn capabilities(&self) -> EcosystemCapabilities;

    // --- mandatory capabilities ---

    /// Name normalization (always present).
    fn name_normalizer(&self) -> &dyn NameNormalizer;

    /// Version parsing/ordering (always present).
    fn version_scheme(&self) -> &dyn VersionScheme;

    /// Requirement parsing/matching (always present).
    fn requirement_parser(&self) -> &dyn RequirementParser;

    /// Condition evaluation (always present).
    fn condition_evaluator(&self) -> &dyn ConditionEvaluator;

    // --- optional capabilities ---

    /// Project detection, if supported.
    fn project_detector(&self) -> Option<&dyn ProjectDetector> {
        None
    }

    /// Manifest parsing, if supported.
    fn manifest_parser(&self) -> Option<&dyn ManifestParser> {
        None
    }

    /// Lockfile parsing, if supported.
    fn lockfile_parser(&self) -> Option<&dyn LockfileParser> {
        None
    }

    /// Registry fetching, if supported.
    fn registry_fetcher(&self) -> Option<&dyn RegistryFetcher> {
        None
    }

    /// Registry-metadata parsing, if supported.
    fn metadata_parser(&self) -> Option<&dyn RegistryMetadataParser> {
        None
    }

    /// Candidate enumeration, if supported.
    fn candidate_provider(&self) -> Option<&dyn CandidateProvider> {
        None
    }

    /// Resolution, if supported.
    fn resolver_adapter(&self) -> Option<&dyn ResolverAdapter> {
        None
    }

    /// Check that the advertised [`EcosystemCapabilities`] flags match the
    /// optional accessors that actually return `Some`. A registry can assert
    /// this at load time to catch misconfigured plugins.
    fn self_consistent(&self) -> bool {
        let caps = self.capabilities();
        let pairs = [
            (
                EcosystemCapabilities::PARSE_MANIFEST,
                self.manifest_parser().is_some(),
            ),
            (
                EcosystemCapabilities::PARSE_LOCKFILE,
                self.lockfile_parser().is_some(),
            ),
            (
                EcosystemCapabilities::FETCH_REGISTRY,
                self.registry_fetcher().is_some(),
            ),
            (
                EcosystemCapabilities::ENUM_CANDIDATES,
                self.candidate_provider().is_some(),
            ),
            (
                EcosystemCapabilities::SYNTHETIC_RESOLVE,
                self.resolver_adapter().is_some(),
            ),
        ];
        pairs
            .into_iter()
            .all(|(flag, present)| caps.contains(flag) == present)
    }
}

#[cfg(test)]
mod tests {
    // Behavioral tests for a concrete plugin live in `crate::mock`; this module
    // only asserts the trait is object-safe via the type alias below.
    use super::*;

    #[allow(dead_code)]
    type BoxedPlugin = Box<dyn EcosystemPlugin>;

    /// A plugin that advertises whatever capabilities it is told to, while
    /// exposing NO accessors — so any advertised flag (from our checked set) is
    /// inconsistent.
    struct LyingPlugin(EcosystemCapabilities);
    impl EcosystemPlugin for LyingPlugin {
        fn id(&self) -> EcosystemId {
            EcosystemId::new(999)
        }
        fn capabilities(&self) -> EcosystemCapabilities {
            self.0
        }
        fn name_normalizer(&self) -> &dyn NameNormalizer {
            unimplemented!("not needed for self_consistent")
        }
        fn version_scheme(&self) -> &dyn VersionScheme {
            unimplemented!("not needed for self_consistent")
        }
        fn requirement_parser(&self) -> &dyn RequirementParser {
            unimplemented!("not needed for self_consistent")
        }
        fn condition_evaluator(&self) -> &dyn ConditionEvaluator {
            unimplemented!("not needed for self_consistent")
        }
    }

    #[test]
    fn self_consistent_rejects_advertised_flag_without_accessor() {
        // Advertises each checked capability but returns no accessor -> inconsistent.
        for flag in [
            EcosystemCapabilities::PARSE_MANIFEST,
            EcosystemCapabilities::PARSE_LOCKFILE,
            EcosystemCapabilities::FETCH_REGISTRY,
            EcosystemCapabilities::ENUM_CANDIDATES,
            EcosystemCapabilities::SYNTHETIC_RESOLVE,
        ] {
            assert!(
                !LyingPlugin(flag).self_consistent(),
                "advertising {flag:?} without an accessor must be inconsistent"
            );
        }
        // Advertising nothing (and exposing nothing) IS consistent.
        assert!(LyingPlugin(EcosystemCapabilities::EMPTY).self_consistent());
    }
}
