//! CSR finalization from transient build nodes/edges (architecture doc,
//! *Resolution and Graph Construction → CSR finalization*).
//!
//! The builder takes a flat list of nodes and `(from, to, ...)` edges and
//! finalizes them into a [`GraphSnapshot`]: a **stable** sort of edges by `from`
//! yields the forward CSR (preserving each source's declaration order via
//! `ordinal`), a separate stable sort of a copy by `to` yields the reverse CSR,
//! and dense `GraphEdgeId`s are assigned by forward-CSR position.

use prism_model::{
    DependencyKind, EdgeFlags, GraphEdgeId, GraphNodeId, NodeFlags, PrismError, RequirementId,
    Result, RunId, StringId, VersionId,
};

use crate::snapshot::{GraphSnapshot, SnapshotParts, NO_REQUIREMENT};

/// A transient build node: an occurrence to be assigned a dense `GraphNodeId`
/// equal to its insertion index.
#[derive(Clone, Debug)]
pub struct BuildNode {
    /// The package version this occurrence resolves to.
    pub version_id: VersionId,
    /// Minimum BFS discovery depth.
    pub depth: u32,
    /// Status flags (root / duplicate / truncated).
    pub flags: NodeFlags,
    /// Optional interned occurrence-key attribute.
    pub attr: Option<StringId>,
}

impl BuildNode {
    /// A plain node at `depth` with no flags or attribute.
    pub fn new(version_id: VersionId, depth: u32) -> Self {
        Self {
            version_id,
            depth,
            flags: NodeFlags::EMPTY,
            attr: None,
        }
    }
}

/// A transient build edge between two existing build-node indices.
#[derive(Clone, Debug)]
pub struct BuildEdge {
    /// Source node index.
    pub from: GraphNodeId,
    /// Target node index.
    pub to: GraphNodeId,
    /// Declaration-order position within the source's dependency list.
    pub ordinal: u32,
    /// Requirement that produced this edge, if any.
    pub requirement_id: Option<RequirementId>,
    /// Dependency kind.
    pub kind: DependencyKind,
    /// Interned scope string (use `StringId::new(0)` when none).
    pub scope: StringId,
    /// Resolution-outcome flags.
    pub flags: EdgeFlags,
}

impl BuildEdge {
    /// A resolved runtime edge with the given ordinal and no requirement/scope.
    pub fn new(from: GraphNodeId, to: GraphNodeId, ordinal: u32) -> Self {
        Self {
            from,
            to,
            ordinal,
            requirement_id: None,
            kind: DependencyKind::Runtime,
            scope: StringId::new(0),
            flags: EdgeFlags::RESOLVED,
        }
    }
}

/// Map a [`DependencyKind`] to the stable `u16` discriminant stored in the CSR.
pub fn kind_to_u16(kind: DependencyKind) -> u16 {
    match kind {
        DependencyKind::Runtime => 0,
        DependencyKind::Dev => 1,
        DependencyKind::Build => 2,
        DependencyKind::Peer => 3,
        DependencyKind::Optional => 4,
        DependencyKind::Test => 5,
        DependencyKind::Plugin => 6,
        DependencyKind::Workspace => 7,
    }
}

/// Inverse of [`kind_to_u16`]; returns [`PrismError::Corrupt`] on an unknown code.
pub fn kind_from_u16(v: u16) -> Result<DependencyKind> {
    Ok(match v {
        0 => DependencyKind::Runtime,
        1 => DependencyKind::Dev,
        2 => DependencyKind::Build,
        3 => DependencyKind::Peer,
        4 => DependencyKind::Optional,
        5 => DependencyKind::Test,
        6 => DependencyKind::Plugin,
        7 => DependencyKind::Workspace,
        other => {
            return Err(PrismError::Corrupt {
                detail: format!("unknown edge kind discriminant {other}"),
            })
        }
    })
}

/// Accumulates build nodes and edges, then finalizes them into a CSR snapshot.
#[derive(Debug, Default)]
pub struct GraphBuilder {
    nodes: Vec<BuildNode>,
    edges: Vec<BuildEdge>,
}

impl GraphBuilder {
    /// A new empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a node and return its dense [`GraphNodeId`].
    ///
    /// Returns [`PrismError::OutOfBounds`] if the node count would exceed
    /// [`GraphNodeId::MAX_NODES`].
    pub fn add_node(&mut self, node: BuildNode) -> Result<GraphNodeId> {
        let id = self.nodes.len() as u64;
        if id >= GraphNodeId::MAX_NODES as u64 {
            return Err(PrismError::OutOfBounds {
                detail: "node count exceeds MAX_NODES".into(),
            });
        }
        self.nodes.push(node);
        Ok(GraphNodeId::new(id as u32))
    }

    /// Append an edge between two existing node ids.
    ///
    /// Returns [`PrismError::OutOfBounds`] if either endpoint is not a node that
    /// has been added, or if the edge count would exceed [`GraphEdgeId::MAX_EDGES`].
    pub fn add_edge(&mut self, edge: BuildEdge) -> Result<()> {
        let n = self.nodes.len() as u32;
        if edge.from.get() >= n || edge.to.get() >= n {
            return Err(PrismError::OutOfBounds {
                detail: "edge endpoint references a node that does not exist".into(),
            });
        }
        if self.edges.len() as u64 >= GraphEdgeId::MAX_EDGES as u64 {
            return Err(PrismError::OutOfBounds {
                detail: "edge count exceeds MAX_EDGES".into(),
            });
        }
        self.edges.push(edge);
        Ok(())
    }

    /// Number of nodes added so far.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Number of edges added so far.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Finalize into an immutable [`GraphSnapshot`] for `run_id`.
    ///
    /// Forward CSR: edges are stably sorted by `from`, so each source's edges
    /// keep their insertion order (and thus `ordinal` order if inserted that
    /// way); dense `GraphEdgeId` = forward position. Reverse CSR: a copy is
    /// stably sorted by `to`. If any node carries an `attr`, the snapshot keeps
    /// the attribute column (absent entries default to `StringId::new(0)`).
    pub fn finalize(self, run_id: RunId) -> Result<GraphSnapshot> {
        let n = self.nodes.len();
        let m = self.edges.len();

        // Node columns.
        let mut node_version_id = Vec::with_capacity(n);
        let mut node_depth = Vec::with_capacity(n);
        let mut node_flags = Vec::with_capacity(n);
        let any_attr = self.nodes.iter().any(|nd| nd.attr.is_some());
        let mut node_attr = if any_attr {
            Some(Vec::with_capacity(n))
        } else {
            None
        };
        for nd in &self.nodes {
            node_version_id.push(nd.version_id);
            node_depth.push(nd.depth);
            node_flags.push(nd.flags);
            if let Some(col) = node_attr.as_mut() {
                col.push(nd.attr.unwrap_or(StringId::new(0)));
            }
        }

        // Forward CSR: stable-sort edge indices by `from`. `Vec::sort_by_key` is
        // stable, so insertion order (declaration order) is preserved per source.
        let mut fwd: Vec<usize> = (0..m).collect();
        fwd.sort_by_key(|&e| self.edges[e].from.get());

        let mut out_offsets = vec![0u32; n + 1];
        for &e in &fwd {
            out_offsets[self.edges[e].from.get() as usize + 1] += 1;
        }
        for i in 0..n {
            out_offsets[i + 1] += out_offsets[i];
        }

        let mut out_targets = Vec::with_capacity(m);
        let mut out_edge_ids = Vec::with_capacity(m);
        let mut edge_ordinal = Vec::with_capacity(m);
        let mut edge_requirement_id = Vec::with_capacity(m);
        let mut edge_kind = Vec::with_capacity(m);
        let mut edge_scope = Vec::with_capacity(m);
        let mut edge_flags = Vec::with_capacity(m);

        // `old_to_new[e]` = dense GraphEdgeId of original edge `e` (= forward pos).
        let mut old_to_new = vec![0u32; m];
        for (new_id, &e) in fwd.iter().enumerate() {
            let edge = &self.edges[e];
            old_to_new[e] = new_id as u32;
            out_targets.push(edge.to);
            out_edge_ids.push(GraphEdgeId::new(new_id as u32));
            edge_ordinal.push(edge.ordinal);
            edge_requirement_id.push(
                edge.requirement_id
                    .unwrap_or(RequirementId::new(NO_REQUIREMENT)),
            );
            edge_kind.push(kind_to_u16(edge.kind));
            edge_scope.push(edge.scope);
            edge_flags.push(edge.flags.bits());
        }

        // Reverse CSR: stable-sort a copy of original indices by `to`.
        let mut rev: Vec<usize> = (0..m).collect();
        rev.sort_by_key(|&e| self.edges[e].to.get());

        let mut in_offsets = vec![0u32; n + 1];
        for &e in &rev {
            in_offsets[self.edges[e].to.get() as usize + 1] += 1;
        }
        for i in 0..n {
            in_offsets[i + 1] += in_offsets[i];
        }

        let mut in_sources = Vec::with_capacity(m);
        let mut in_edge_ids = Vec::with_capacity(m);
        for &e in &rev {
            let edge = &self.edges[e];
            in_sources.push(edge.from);
            in_edge_ids.push(GraphEdgeId::new(old_to_new[e]));
        }

        GraphSnapshot::from_parts(SnapshotParts {
            run_id,
            node_version_id,
            node_depth,
            node_flags,
            node_attr,
            out_offsets,
            out_targets,
            out_edge_ids,
            in_offsets,
            in_sources,
            in_edge_ids,
            edge_ordinal,
            edge_requirement_id,
            edge_kind,
            edge_scope,
            edge_flags,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nid(i: u32) -> GraphNodeId {
        GraphNodeId::new(i)
    }

    #[test]
    fn kind_roundtrip_all() {
        for k in [
            DependencyKind::Runtime,
            DependencyKind::Dev,
            DependencyKind::Build,
            DependencyKind::Peer,
            DependencyKind::Optional,
            DependencyKind::Test,
            DependencyKind::Plugin,
            DependencyKind::Workspace,
        ] {
            assert_eq!(kind_from_u16(kind_to_u16(k)).unwrap(), k);
        }
        assert!(kind_from_u16(99).is_err());
    }

    #[test]
    fn finalize_builds_consistent_csr() {
        // 0 -> 1, 0 -> 2, 1 -> 2, 2 -> 0 (a cycle 0->2->0).
        let mut b = GraphBuilder::new();
        for i in 0..3 {
            b.add_node(BuildNode::new(VersionId::new(100 + i), i as u32))
                .unwrap();
        }
        b.add_edge(BuildEdge::new(nid(0), nid(1), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(0), nid(2), 1)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(2), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(2), nid(0), 0)).unwrap();
        let g = b.finalize(RunId::new(7)).unwrap();

        assert_eq!(g.node_count(), 3);
        assert_eq!(g.edge_count(), 4);
        assert_eq!(g.run_id(), RunId::new(7));

        // Forward neighbors.
        let o0: Vec<u32> = g.out_neighbors(nid(0)).unwrap().iter().map(|x| x.get()).collect();
        assert_eq!(o0, vec![1, 2]);
        assert_eq!(g.out_neighbors(nid(1)).unwrap(), &[nid(2)]);
        assert_eq!(g.out_neighbors(nid(2)).unwrap(), &[nid(0)]);

        // Reverse neighbors: who points at 2? -> 0 and 1.
        let mut i2: Vec<u32> = g.in_neighbors(nid(2)).unwrap().iter().map(|x| x.get()).collect();
        i2.sort_unstable();
        assert_eq!(i2, vec![0, 1]);
        // Who points at 0? -> 2.
        assert_eq!(g.in_neighbors(nid(0)).unwrap(), &[nid(2)]);

        // Ordinals preserved per source (declaration order).
        let e0 = g.out_edge_ids(nid(0)).unwrap();
        assert_eq!(g.edge_ordinal(e0[0]).unwrap(), 0);
        assert_eq!(g.edge_ordinal(e0[1]).unwrap(), 1);
    }

    #[test]
    fn edge_endpoint_out_of_range_rejected() {
        let mut b = GraphBuilder::new();
        b.add_node(BuildNode::new(VersionId::new(1), 0)).unwrap();
        assert!(b.add_edge(BuildEdge::new(nid(0), nid(5), 0)).is_err());
    }

    #[test]
    fn empty_graph_finalizes() {
        let g = GraphBuilder::new().finalize(RunId::new(1)).unwrap();
        assert_eq!(g.node_count(), 0);
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn attr_column_kept_only_when_present() {
        let mut b = GraphBuilder::new();
        let mut nd = BuildNode::new(VersionId::new(1), 0);
        nd.attr = Some(StringId::new(42));
        b.add_node(nd).unwrap();
        b.add_node(BuildNode::new(VersionId::new(2), 1)).unwrap();
        let g = b.finalize(RunId::new(1)).unwrap();
        assert_eq!(g.node_attr(nid(0)).unwrap(), Some(StringId::new(42)));
        // Node without an explicit attr defaults to StringId(0).
        assert_eq!(g.node_attr(nid(1)).unwrap(), Some(StringId::new(0)));
    }
}
