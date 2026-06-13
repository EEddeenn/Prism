//! Ecosystem plugin API: the trait surface every package ecosystem (npm, Cargo,
//! PyPI, Maven, Go, …) implements, plus the canonical fact events parsers emit.
//!
//! See the architecture design document, sections *Fetcher, Parser, and Resolver
//! Architecture* and *Ecosystem Plugin Design*. This crate is interface-heavy:
//! it defines *what* a plugin must provide, not any ecosystem-specific logic.
//!
//! ## Shape
//!
//! - [`EcosystemPlugin`] is the object-safe aggregator. It hands out borrowed
//!   capability trait objects: `&dyn` for the four mandatory capabilities and
//!   `Option<&dyn>` for the optional ones, and advertises them via
//!   [`prism_model::EcosystemCapabilities`].
//! - Capability sub-traits ([`NameNormalizer`], [`VersionScheme`],
//!   [`RequirementParser`], [`ConditionEvaluator`], [`ProjectDetector`],
//!   [`ManifestParser`], [`LockfileParser`], [`RegistryFetcher`],
//!   [`RegistryMetadataParser`], [`CandidateProvider`], [`ResolverAdapter`]) are
//!   each **object-safe**: no generics, no `Self`-returning methods, no
//!   `async fn`. The two async capabilities use explicit boxed `Send` futures.
//! - Parsers emit [`CanonicalEvent`]s collected into a [`CanonicalBatch`]; a
//!   separate writer (outside this crate) drains them into the store.
//! - Resolver adapters return a *plain* [`ResolvedGraph`] (a flat node/edge
//!   list), **not** the CSR snapshot — that is built downstream in `prism-graph`.
//!
//! ## Security
//!
//! All fetch-path URLs are [`SafeUrl`], which strips credentials at
//! construction so secrets can never enter a store file, provenance record, log,
//! or UI response. [`AuthRef`] is an *indirect* handle to a secret resolved
//! outside the workspace — never the secret itself.

pub mod activation;
pub mod capabilities;
pub mod coord;
pub mod fetch;
pub mod parse;
pub mod plugin;
pub mod resolve;
pub mod url;

pub use activation::{ActivationContext, GateInput};
pub use capabilities::{
    CandidateProvider, ConditionEvaluator, FetchError, LockfileParser, ManifestParser,
    NameNormalizer, ProjectDetector, RegistryFetcher, RegistryMetadataParser, RequirementParser,
    ResolverAdapter, VersionOrdering, VersionScheme,
};
pub use coord::{PackageName, VersionReqOrExact};
pub use fetch::{
    AuthRef, CachePolicy, FetchRequest, FetchTarget, Provenance, RawDocument, RegistrySource,
};
pub use parse::{
    CanonicalBatch, CanonicalEvent, ChecksumFact, ChecksumTrust, DeprecationFact,
    DistributionFileFact, HashAlgorithm, PackageFact, ParseError, ParseWarning, RequirementFact,
    VersionFact, YankedFact,
};
pub use plugin::EcosystemPlugin;
pub use resolve::{
    ResolveContext, ResolveError, ResolveRequest, ResolvePolicy, ResolvedEdge, ResolvedGraph,
    ResolvedNode,
};
pub use url::SafeUrl;

/// Compile-time object-safety assertions.
///
/// Each function below only type-checks if the trait it references is
/// object-safe; a method that broke object-safety (a generic, a `Self` return,
/// an `async fn`) would make the corresponding `&dyn`/`Box<dyn>` ill-formed and
/// fail the build. These are never called.
#[allow(dead_code)]
mod object_safety {
    use super::*;

    fn _name_normalizer(_: &dyn NameNormalizer) {}
    fn _version_scheme(_: &dyn VersionScheme) {}
    fn _requirement_parser(_: &dyn RequirementParser) {}
    fn _condition_evaluator(_: &dyn ConditionEvaluator) {}
    fn _project_detector(_: &dyn ProjectDetector) {}
    fn _manifest_parser(_: &dyn ManifestParser) {}
    fn _lockfile_parser(_: &dyn LockfileParser) {}
    fn _registry_fetcher(_: &dyn RegistryFetcher) {}
    fn _registry_metadata_parser(_: &dyn RegistryMetadataParser) {}
    fn _candidate_provider(_: &dyn CandidateProvider) {}
    fn _resolver_adapter(_: &dyn ResolverAdapter) {}
    fn _ecosystem_plugin(_: &dyn EcosystemPlugin) {}

    // The aggregator must be storable as an owned trait object.
    fn _boxed_plugin(p: Box<dyn EcosystemPlugin>) -> Box<dyn EcosystemPlugin> {
        p
    }
}

#[cfg(test)]
mod mock;
