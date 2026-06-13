//! Prism's synthetic, two-phase fixpoint resolver and policy engine.
//!
//! This crate implements the *synthetic* construction path from the architecture
//! doc's *Resolution and Graph Construction*: a research resolver that turns
//! declared requirements into a plain [`prism_plugin_api::ResolvedGraph`]
//! (occurrences + edges), which `prism-graph` later compresses into a CSR
//! snapshot. It operates against two **abstract inputs** so it is testable
//! standalone; `prism-api` wires the store-backed implementations later:
//!
//! - [`RequirementSource`] — declared requirements of a version (Phase-1 input).
//! - [`CandidateSource`] — version selection under a policy (Phase-2 operator).
//!
//! The [`Engine`] does BFS materialization with three guarantees the doc
//! requires: a **package-family cycle guard** (back-edges instead of infinite
//! recursion), **depth/node budgets** (truncation + `BUDGET_TRUNCATED` caveat),
//! and a **feature/extra activation fixpoint** (re-resolve while the
//! [`prism_plugin_api::ActivationContext`] keeps growing). Condition gating is
//! split into environment vs activation via [`prism_plugin_api::GateInput`].
//!
//! [`VersionCatalog`] is a ready-made [`CandidateSource`] implementing the
//! locked / lowest / latest / highest-non-prerelease policies over an in-memory
//! version list. The engine never panics on its inputs: every fallible path
//! returns [`prism_plugin_api::ResolveError`], and expansion is bounded.

mod engine;
mod policy;
mod source;

#[cfg(test)]
mod tests;

pub use engine::{AlwaysActive, ConditionGate, Engine, ResolveBudget, Resolution};
pub use policy::VersionCatalog;
pub use source::{CandidateSource, DeclaredRequirement, RequirementSource};

// Re-export the shared output/policy types so callers depend on one crate.
pub use prism_plugin_api::{
    ActivationContext, GateInput, ResolveError, ResolvePolicy, ResolvedEdge, ResolvedGraph,
    ResolvedNode,
};
