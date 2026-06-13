//! Abstract resolver inputs: the two hooks the synthetic engine pulls on.
//!
//! The engine is deliberately store-agnostic. Instead of reaching into a store
//! it asks two traits for facts:
//!
//! - [`RequirementSource`] — *what does this version declare?* Given a
//!   [`VersionId`] it returns the declared [`DeclaredRequirement`]s (target
//!   family, an opaque matcher string, kind, condition split, and an optional
//!   feature/extra gate). This is the Phase-1 closure input.
//! - [`CandidateSource`] — *which version satisfies this?* Given a package, a
//!   matcher, the pinned [`EnvironmentId`], and the current [`ActivationContext`]
//!   it selects one [`VersionId`] under a [`ResolvePolicy`]. This is the Phase-2
//!   selection operator.
//!
//! Keeping both abstract lets the crate be tested standalone (see the in-crate
//! mocks) while `prism-api` later wires the store-backed implementations.

use prism_model::{ConditionKind, DependencyKind, EnvironmentId, PackageId, RequirementId, VersionId};
use prism_plugin_api::{ActivationContext, ResolveError, ResolvePolicy};

/// A single declared dependency edge out of a concrete version.
///
/// This is the resolver-facing analogue of [`prism_model::Requirement`]: it
/// carries only what the engine needs to gate and select, with the matcher and
/// condition kept as opaque strings the [`CandidateSource`]/gate interprets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredRequirement {
    /// Canonicalized requirement identity (recorded on the resolved edge).
    pub requirement_id: RequirementId,
    /// Target package family this requirement points at.
    pub target: PackageId,
    /// Opaque version matcher (e.g. `^1`, `*`, `=1.2.3`) the candidate source
    /// interprets; the engine never parses it.
    pub matcher: String,
    /// Dependency category, copied onto the resolved edge.
    pub kind: DependencyKind,
    /// Whether [`Self::condition`] gates on the environment or on activation.
    pub condition_kind: ConditionKind,
    /// Opaque condition expression, evaluated under [`Self::condition_kind`].
    /// `None` means unconditional.
    pub condition: Option<String>,
    /// Features/extras this requirement *activates on its target* when the edge
    /// is taken (Cargo `serde/derive`, PyPI `requests[socks]`). Drives the
    /// activation fixpoint.
    pub activates: Vec<String>,
}

impl DeclaredRequirement {
    /// A minimal unconditional runtime requirement with no activation effect.
    pub fn runtime(
        requirement_id: RequirementId,
        target: PackageId,
        matcher: impl Into<String>,
    ) -> Self {
        Self {
            requirement_id,
            target,
            matcher: matcher.into(),
            kind: DependencyKind::Runtime,
            condition_kind: ConditionKind::None,
            condition: None,
            activates: Vec::new(),
        }
    }

    /// True if this requirement carries an activation gate (must be checked
    /// against the running [`ActivationContext`], not the environment).
    #[inline]
    pub fn is_activation_gated(&self) -> bool {
        self.condition_kind == ConditionKind::Activation && self.condition.is_some()
    }
}

/// Source of declared requirements for a version (Phase-1 closure input).
pub trait RequirementSource {
    /// The declared requirements of `version`, in stable declaration order. The
    /// order is preserved as the resolved-edge `ordinal`.
    fn requirements(&self, version: VersionId) -> Result<Vec<DeclaredRequirement>, ResolveError>;

    /// The package family `version` belongs to. The cycle guard keys on this, so
    /// it must agree with the [`CandidateSource`]'s notion of family identity.
    fn package_of(&self, version: VersionId) -> Result<PackageId, ResolveError>;

    /// Features/extras a version activates *on itself* unconditionally (e.g. a
    /// crate's `default` feature). Empty for ecosystems without features.
    fn default_activations(&self, _version: VersionId) -> Result<Vec<String>, ResolveError> {
        Ok(Vec::new())
    }
}

/// Source of concrete version selections (Phase-2 selection operator).
///
/// Implementations apply the run's [`ResolvePolicy`] (locked / lowest /
/// highest-non-prerelease / …) over the candidate set, consulting the
/// environment and the live activation closure where relevant.
pub trait CandidateSource {
    /// Select one version of `package` satisfying `matcher` under `policy`,
    /// given the pinned `environment` and the current `activation` closure.
    ///
    /// Returns `Ok(None)` when the requirement is *gated off* (no candidate is
    /// wanted), and `Err(ResolveError::Unsatisfiable)` when a candidate is
    /// required but none exists.
    fn select(
        &self,
        package: PackageId,
        matcher: &str,
        environment: EnvironmentId,
        activation: &ActivationContext,
        policy: &ResolvePolicy,
    ) -> Result<Option<VersionId>, ResolveError>;
}
