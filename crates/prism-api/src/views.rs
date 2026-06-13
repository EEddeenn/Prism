//! Display-oriented view structs returned by the facade's graph/TDA queries.
//!
//! These are *resolved* projections: snapshot-local [`prism_model::GraphNodeId`]s
//! are paired with the human-facing purl/label the store interned, so a caller
//! (CLI, binding) never has to touch raw ids or re-open the store to render a
//! result. They are plain data; the facade owns all the lookups.

use prism_model::{GraphNodeId, PackageId, RunId, VersionId};

/// One node in a query result, resolved for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphNodeView {
    /// Snapshot-local node id (meaningful only within `run`).
    pub id: GraphNodeId,
    /// The package version this occurrence resolves to.
    pub version: VersionId,
    /// The package family of `version`, if known to the store.
    pub package: Option<PackageId>,
    /// The purl/label for display (interned `purl`, else a synthetic label).
    pub purl: String,
    /// Minimum BFS discovery depth of this occurrence.
    pub depth: u32,
}

/// One edge in a query result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphEdgeView {
    /// Source node id.
    pub source: GraphNodeId,
    /// Target node id.
    pub target: GraphNodeId,
    /// Raw `u16` dependency-kind discriminant (see `prism_graph::kind_from_u16`).
    pub kind: u16,
    /// Raw `u32` edge flags (see `prism_model::EdgeFlags`).
    pub flags: u32,
}

/// Whole-graph summary statistics for a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphStatsView {
    /// The run these stats describe.
    pub run: RunId,
    /// Number of occurrence nodes.
    pub nodes: usize,
    /// Number of directed edges (multigraph).
    pub edges: usize,
    /// Number of strongly-connected components (condensation size).
    pub scc_count: usize,
    /// True if any SCC spans more than one node (a real cycle).
    pub has_cycle: bool,
    /// Maximum BFS discovery depth across all nodes.
    pub max_depth: u32,
    /// Number of exact versions that occur at more than one node.
    pub duplicate_version_count: usize,
}

/// A bounded neighborhood / impact result: the resolved nodes and the edges
/// induced among them, plus a truncation flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeighborhoodView {
    /// The center node id (always first, resolved for display).
    pub center: GraphNodeView,
    /// The reached nodes (excluding the center), resolved for display.
    pub nodes: Vec<GraphNodeView>,
    /// Edges induced among `{center} ∪ nodes`.
    pub edges: Vec<GraphEdgeView>,
    /// True if the traversal stopped early because a node budget was hit.
    pub truncated: bool,
}

/// A strongly-connected component, resolved for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SccView {
    /// The member nodes of this component.
    pub members: Vec<GraphNodeView>,
}

impl SccView {
    /// Number of nodes in the component.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// True if the component is empty (never produced by the facade).
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// True if this component is a multi-node cycle.
    pub fn is_cycle(&self) -> bool {
        self.members.len() > 1
    }
}

/// A duplicate-version cluster: one exact version appearing at several nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateClusterView {
    /// The shared version id.
    pub version: VersionId,
    /// The package family, if known.
    pub package: Option<PackageId>,
    /// The purl/label for display.
    pub purl: String,
    /// The occurrence nodes (length >= 2).
    pub occurrences: Vec<GraphNodeId>,
    /// Minimum BFS depth across the occurrences.
    pub min_depth: u32,
    /// Maximum BFS depth across the occurrences.
    pub max_depth: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scc_view_predicates() {
        let n = GraphNodeView {
            id: GraphNodeId::new(0),
            version: VersionId::new(1),
            package: None,
            purl: "x".into(),
            depth: 0,
        };
        let solo = SccView {
            members: vec![n.clone()],
        };
        assert_eq!(solo.len(), 1);
        assert!(!solo.is_cycle());
        assert!(!solo.is_empty());
        let cyc = SccView {
            members: vec![n.clone(), n],
        };
        assert!(cyc.is_cycle());
    }
}
