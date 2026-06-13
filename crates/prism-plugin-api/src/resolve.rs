//! Resolver-adapter value types (architecture doc, *ResolverAdapter
//! Responsibilities*).
//!
//! A resolver adapter turns requirements into a resolved graph. The output here
//! is a *plain in-memory* [`ResolvedGraph`] — a flat list of node occurrences
//! and edges — **not** the compressed-sparse-row snapshot, which is built later
//! in `prism-graph`. Keeping the adapter output un-indexed lets native and
//! synthetic adapters share one shape and lets the graph builder own occurrence
//! interning and CSR layout.

use prism_model::{
    DependencyKind, EcosystemId, EdgeFlags, EnvironmentId, GraphFidelity, NodeFlags, RequirementId,
    SnapshotId, VersionId,
};

use crate::activation::ActivationContext;

/// A resolution policy the adapter should apply when a choice is free
/// (architecture doc lists: locked, latest, lowest, …).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvePolicy {
    /// Read selections verbatim from a lockfile/native tool; make no choices.
    Locked,
    /// Prefer the newest satisfying version.
    Latest,
    /// Prefer the oldest satisfying version.
    Lowest,
    /// Prefer the newest non-prerelease satisfying version.
    HighestNonPrerelease,
    /// Resolve as of a pinned timestamp (Unix seconds).
    TimeTravelAt(i64),
    /// Prefer versions a security policy ranks higher.
    SecurityPreferred,
    /// Sample one resolution at random (seeded for reproducibility).
    RandomSample(u64),
    /// Maximize version diversity across the graph.
    MaxDiversity,
}

/// The request handed to [`crate::ResolverAdapter::resolve`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveRequest {
    /// Ecosystem being resolved.
    pub ecosystem: EcosystemId,
    /// Root requirement ids that seed the resolution.
    pub roots: Vec<RequirementId>,
    /// Policy to apply where the selection is unconstrained.
    pub policy: ResolvePolicy,
    /// Optional cap on resolved nodes; over-budget runs are truncated.
    pub max_nodes: Option<u32>,
}

/// Read-only context shared across a resolution: the pinned environment and
/// snapshot, plus the already-resolved activation closure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveContext {
    /// Content-addressed environment (platform, language version, markers).
    pub environment: EnvironmentId,
    /// Registry snapshot pinning candidate metadata for reproducibility.
    pub snapshot: SnapshotId,
    /// Resolved feature/extra activation closure (Cargo features, PyPI extras).
    pub activation: ActivationContext,
}

/// One occurrence of a package version in the resolved graph.
///
/// This is the *plain* analogue of [`prism_model::GraphNode`]: it omits the
/// snapshot-local `GraphNodeId` (assigned by the CSR builder) and instead
/// carries a builder-local `local_id` for edges to reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedNode {
    /// Adapter-local node index, referenced by [`ResolvedEdge`]. Dense from 0.
    pub local_id: u32,
    /// The concrete version this occurrence selects.
    pub version_id: VersionId,
    /// Minimum discovery depth (BFS) of this occurrence.
    pub depth: u32,
    /// Occurrence-distinguishing key string (path/feature/platform context),
    /// canonicalized by the plugin's occurrence-identity function.
    pub occurrence_key: String,
    /// Node status flags.
    pub flags: NodeFlags,
}

/// A resolved dependency edge between two [`ResolvedNode`]s, by `local_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedEdge {
    /// `local_id` of the source occurrence.
    pub from: u32,
    /// `local_id` of the target occurrence.
    pub to: u32,
    /// Position within the source's dependency list (stable ordering).
    pub ordinal: u32,
    /// The declared requirement this edge resolves, if known.
    pub requirement_id: Option<RequirementId>,
    /// Dependency category.
    pub kind: DependencyKind,
    /// Resolution-outcome flags.
    pub flags: EdgeFlags,
}

/// The plain in-memory result of a resolution: occurrences plus edges, with the
/// fidelity the adapter achieved. `prism-graph` consumes this to build the CSR
/// snapshot and assign `GraphNodeId`/`GraphEdgeId`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedGraph {
    /// Node occurrences, indexed by their `local_id` (== position).
    pub nodes: Vec<ResolvedNode>,
    /// Edges referencing nodes by `local_id`.
    pub edges: Vec<ResolvedEdge>,
    /// Fidelity tier and dropped-semantics caveats for this resolution.
    pub fidelity: Option<GraphFidelity>,
    /// True if the run was cut short by a budget (`max_nodes`).
    pub truncated: bool,
}

impl ResolvedGraph {
    /// A new empty graph.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a node, assigning the next dense `local_id`, and return that id.
    pub fn add_node(
        &mut self,
        version_id: VersionId,
        depth: u32,
        occurrence_key: impl Into<String>,
        flags: NodeFlags,
    ) -> u32 {
        let local_id = self.nodes.len() as u32;
        self.nodes.push(ResolvedNode {
            local_id,
            version_id,
            depth,
            occurrence_key: occurrence_key.into(),
            flags,
        });
        local_id
    }

    /// Push an edge between two existing `local_id`s.
    pub fn add_edge(&mut self, edge: ResolvedEdge) {
        self.edges.push(edge);
    }

    /// Number of node occurrences.
    #[inline]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Number of edges.
    #[inline]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

/// A fatal resolution failure. Converts into the shared
/// [`prism_model::PrismError`] at the crate boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// A requirement could not be satisfied under the snapshot/environment.
    Unsatisfiable {
        /// The offending requirement.
        requirement: RequirementId,
        /// Human-readable detail.
        detail: String,
    },
    /// Required registry metadata was missing for the pinned snapshot.
    MissingMetadata {
        /// Human-readable detail.
        detail: String,
    },
    /// A native tool or lockfile produced output the adapter could not map.
    NativeToolFailed {
        /// Human-readable detail.
        detail: String,
    },
    /// The resolution exceeded a configured budget and could not continue.
    BudgetExceeded {
        /// Human-readable detail.
        detail: String,
    },
    /// A fallback for failures without a dedicated variant.
    Other(String),
}

impl core::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ResolveError::Unsatisfiable {
                requirement,
                detail,
            } => write!(f, "unsatisfiable requirement {requirement}: {detail}"),
            ResolveError::MissingMetadata { detail } => write!(f, "missing metadata: {detail}"),
            ResolveError::NativeToolFailed { detail } => write!(f, "native tool failed: {detail}"),
            ResolveError::BudgetExceeded { detail } => write!(f, "budget exceeded: {detail}"),
            ResolveError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for ResolveError {}

impl From<ResolveError> for prism_model::PrismError {
    fn from(e: ResolveError) -> Self {
        match e {
            ResolveError::BudgetExceeded { detail } => prism_model::PrismError::OutOfBounds { detail },
            other => prism_model::PrismError::Other(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_builds_nodes_and_edges() {
        let mut g = ResolvedGraph::new();
        let a = g.add_node(VersionId::new(1), 0, "", NodeFlags::ROOT);
        let b = g.add_node(VersionId::new(2), 1, "node_modules/b", NodeFlags::EMPTY);
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        g.add_edge(ResolvedEdge {
            from: a,
            to: b,
            ordinal: 0,
            requirement_id: Some(RequirementId::new(9)),
            kind: DependencyKind::Runtime,
            flags: EdgeFlags::RESOLVED,
        });
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 1);
        assert_eq!(g.nodes[1].occurrence_key, "node_modules/b");
        assert!(!g.truncated);
    }

    #[test]
    fn resolve_error_display_and_convert() {
        let e = ResolveError::Unsatisfiable {
            requirement: RequirementId::new(3),
            detail: "no candidate".into(),
        };
        assert!(e.to_string().contains("unsatisfiable"));
        let pe: prism_model::PrismError = ResolveError::BudgetExceeded {
            detail: "too big".into(),
        }
        .into();
        assert!(matches!(pe, prism_model::PrismError::OutOfBounds { .. }));
    }

    #[test]
    fn policy_variants_carry_params() {
        assert_eq!(
            ResolvePolicy::TimeTravelAt(1700000000),
            ResolvePolicy::TimeTravelAt(1700000000)
        );
        assert_ne!(ResolvePolicy::RandomSample(1), ResolvePolicy::RandomSample(2));
    }
}
