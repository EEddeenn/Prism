//! Immutable CSR graph snapshots (architecture doc, *CSR Graph Snapshots*).
//!
//! A [`GraphSnapshot`] owns the compressed-sparse-row arrays for one resolver
//! run. v0.1 loads snapshots into owned arrays (simpler, crash-safe, `Send +
//! Sync`); the little-endian layout still permits a later zero-copy mmap reader
//! without a format change.
//!
//! Node and edge indices are dense, snapshot-local, and meaningful only inside
//! this run; they must be carried with the [`RunId`] at any external boundary.

use prism_model::{
    GraphEdgeId, GraphNodeId, NodeFlags, PrismError, RequirementId, Result, RunId, StringId,
    VersionId,
};

/// One immutable resolved dependency graph in compressed-sparse-row form.
///
/// Invariants (validated by the builder and on read):
/// - `out_offsets`/`in_offsets` have length `node_count + 1`, are non-decreasing,
///   and end at `edge_count`.
/// - every `out_targets`/`in_sources` entry is a valid node index `< node_count`.
/// - all per-edge arrays have length `edge_count`.
#[derive(Clone, Debug)]
pub struct GraphSnapshot {
    run_id: RunId,
    node_count: u32,
    edge_count: u32,

    // Node arrays (length N).
    node_version_id: Vec<VersionId>,
    node_depth: Vec<u32>,
    node_flags: Vec<NodeFlags>,
    /// Optional per-node occurrence-key attribute (interned `StringId`), absent
    /// when no node carries one.
    node_attr: Option<Vec<StringId>>,

    // Forward CSR (offsets length N+1, target/edge length M).
    out_offsets: Vec<u32>,
    out_targets: Vec<GraphNodeId>,
    out_edge_ids: Vec<GraphEdgeId>,

    // Reverse CSR.
    in_offsets: Vec<u32>,
    in_sources: Vec<GraphNodeId>,
    in_edge_ids: Vec<GraphEdgeId>,

    // Edge arrays (length M), indexed by dense `GraphEdgeId`.
    edge_ordinal: Vec<u32>,
    edge_requirement_id: Vec<RequirementId>,
    edge_kind: Vec<u16>,
    edge_scope: Vec<StringId>,
    edge_flags: Vec<u32>,
}

/// The raw column arrays of a [`GraphSnapshot`], used by the builder and the
/// `.pcsr` reader to construct a validated snapshot in one place.
pub(crate) struct SnapshotParts {
    pub run_id: RunId,
    pub node_version_id: Vec<VersionId>,
    pub node_depth: Vec<u32>,
    pub node_flags: Vec<NodeFlags>,
    pub node_attr: Option<Vec<StringId>>,
    pub out_offsets: Vec<u32>,
    pub out_targets: Vec<GraphNodeId>,
    pub out_edge_ids: Vec<GraphEdgeId>,
    pub in_offsets: Vec<u32>,
    pub in_sources: Vec<GraphNodeId>,
    pub in_edge_ids: Vec<GraphEdgeId>,
    pub edge_ordinal: Vec<u32>,
    pub edge_requirement_id: Vec<RequirementId>,
    pub edge_kind: Vec<u16>,
    pub edge_scope: Vec<StringId>,
    pub edge_flags: Vec<u32>,
}

fn corrupt(detail: impl Into<String>) -> PrismError {
    PrismError::Corrupt {
        detail: detail.into(),
    }
}

impl GraphSnapshot {
    /// Build and validate a snapshot from raw column arrays.
    ///
    /// Returns [`PrismError::Corrupt`] if any structural invariant is violated:
    /// mismatched array lengths, an over-cap node/edge count, a non-monotonic or
    /// wrongly-terminated offset array, or an out-of-range neighbor index.
    pub(crate) fn from_parts(p: SnapshotParts) -> Result<Self> {
        let n = p.node_version_id.len();
        let m = p.out_targets.len();
        if n as u64 > GraphNodeId::MAX_NODES as u64 {
            return Err(corrupt(format!("node_count {n} exceeds MAX_NODES")));
        }
        if m as u64 > GraphEdgeId::MAX_EDGES as u64 {
            return Err(corrupt(format!("edge_count {m} exceeds MAX_EDGES")));
        }

        // Node-array lengths.
        if p.node_depth.len() != n || p.node_flags.len() != n {
            return Err(corrupt("node array length mismatch"));
        }
        if let Some(attr) = &p.node_attr {
            if attr.len() != n {
                return Err(corrupt("node_attr length mismatch"));
            }
        }

        // Edge-array lengths.
        if p.out_edge_ids.len() != m
            || p.in_sources.len() != m
            || p.in_edge_ids.len() != m
            || p.edge_ordinal.len() != m
            || p.edge_requirement_id.len() != m
            || p.edge_kind.len() != m
            || p.edge_scope.len() != m
            || p.edge_flags.len() != m
        {
            return Err(corrupt("edge array length mismatch"));
        }

        let node_count = n as u32;
        let edge_count = m as u32;
        validate_offsets(&p.out_offsets, n, m, "out")?;
        validate_offsets(&p.in_offsets, n, m, "in")?;
        validate_neighbors(&p.out_targets, node_count, "out_targets")?;
        validate_neighbors(&p.in_sources, node_count, "in_sources")?;

        Ok(Self {
            run_id: p.run_id,
            node_count,
            edge_count,
            node_version_id: p.node_version_id,
            node_depth: p.node_depth,
            node_flags: p.node_flags,
            node_attr: p.node_attr,
            out_offsets: p.out_offsets,
            out_targets: p.out_targets,
            out_edge_ids: p.out_edge_ids,
            in_offsets: p.in_offsets,
            in_sources: p.in_sources,
            in_edge_ids: p.in_edge_ids,
            edge_ordinal: p.edge_ordinal,
            edge_requirement_id: p.edge_requirement_id,
            edge_kind: p.edge_kind,
            edge_scope: p.edge_scope,
            edge_flags: p.edge_flags,
        })
    }

    /// The run this snapshot belongs to; node/edge ids are valid only within it.
    #[inline]
    pub fn run_id(&self) -> RunId {
        self.run_id
    }

    /// Number of nodes (occurrences) in the snapshot.
    #[inline]
    pub fn node_count(&self) -> usize {
        self.node_count as usize
    }

    /// Number of directed edges in the snapshot (a multigraph: parallel edges
    /// between the same pair are counted separately).
    #[inline]
    pub fn edge_count(&self) -> usize {
        self.edge_count as usize
    }

    #[inline]
    fn node_idx(&self, node: GraphNodeId) -> Result<usize> {
        match node.index() {
            Some(i) if i < self.node_count() => Ok(i),
            _ => Err(PrismError::OutOfBounds {
                detail: format!("node {node:?} out of range (node_count {})", self.node_count()),
            }),
        }
    }

    /// The package version recorded at `node`.
    pub fn node_version(&self, node: GraphNodeId) -> Result<VersionId> {
        Ok(self.node_version_id[self.node_idx(node)?])
    }

    /// The BFS discovery depth of `node` (minimum over discovery paths).
    pub fn node_depth(&self, node: GraphNodeId) -> Result<u32> {
        Ok(self.node_depth[self.node_idx(node)?])
    }

    /// The status flags of `node`.
    pub fn node_flags(&self, node: GraphNodeId) -> Result<NodeFlags> {
        Ok(self.node_flags[self.node_idx(node)?])
    }

    /// The optional interned occurrence-key attribute of `node`, if the snapshot
    /// carries the attribute column.
    pub fn node_attr(&self, node: GraphNodeId) -> Result<Option<StringId>> {
        let i = self.node_idx(node)?;
        Ok(self.node_attr.as_ref().map(|a| a[i]))
    }

    /// All node version ids, in dense node order.
    #[inline]
    pub fn node_versions(&self) -> &[VersionId] {
        &self.node_version_id
    }

    /// Forward (dependency) neighbors of `node`: a slice of target node ids.
    /// May contain duplicate targets (multigraph); dedup before set algebra.
    pub fn out_neighbors(&self, node: GraphNodeId) -> Result<&[GraphNodeId]> {
        let i = self.node_idx(node)?;
        let start = self.out_offsets[i] as usize;
        let end = self.out_offsets[i + 1] as usize;
        Ok(&self.out_targets[start..end])
    }

    /// Reverse (dependent) neighbors of `node`: a slice of source node ids.
    pub fn in_neighbors(&self, node: GraphNodeId) -> Result<&[GraphNodeId]> {
        let i = self.node_idx(node)?;
        let start = self.in_offsets[i] as usize;
        let end = self.in_offsets[i + 1] as usize;
        Ok(&self.in_sources[start..end])
    }

    /// The dense edge ids of the outgoing edges of `node`, aligned with
    /// [`out_neighbors`](Self::out_neighbors).
    pub fn out_edge_ids(&self, node: GraphNodeId) -> Result<&[GraphEdgeId]> {
        let i = self.node_idx(node)?;
        let start = self.out_offsets[i] as usize;
        let end = self.out_offsets[i + 1] as usize;
        Ok(&self.out_edge_ids[start..end])
    }

    /// The dense edge ids of the incoming edges of `node`, aligned with
    /// [`in_neighbors`](Self::in_neighbors).
    pub fn in_edge_ids(&self, node: GraphNodeId) -> Result<&[GraphEdgeId]> {
        let i = self.node_idx(node)?;
        let start = self.in_offsets[i] as usize;
        let end = self.in_offsets[i + 1] as usize;
        Ok(&self.in_edge_ids[start..end])
    }

    #[inline]
    fn edge_idx(&self, edge: GraphEdgeId) -> Result<usize> {
        let i = edge.get() as usize;
        if edge.get() != u32::MAX && i < self.edge_count() {
            Ok(i)
        } else {
            Err(PrismError::OutOfBounds {
                detail: format!("edge {edge:?} out of range (edge_count {})", self.edge_count()),
            })
        }
    }

    /// The source-list ordinal of `edge` (preserves declaration order).
    pub fn edge_ordinal(&self, edge: GraphEdgeId) -> Result<u32> {
        Ok(self.edge_ordinal[self.edge_idx(edge)?])
    }

    /// The requirement that produced `edge`. [`RequirementId::new(u64::MAX)`] is
    /// the in-snapshot encoding of "no requirement".
    pub fn edge_requirement_id(&self, edge: GraphEdgeId) -> Result<RequirementId> {
        Ok(self.edge_requirement_id[self.edge_idx(edge)?])
    }

    /// The raw `u16` dependency-kind discriminant of `edge`.
    pub fn edge_kind(&self, edge: GraphEdgeId) -> Result<u16> {
        Ok(self.edge_kind[self.edge_idx(edge)?])
    }

    /// The interned scope string of `edge`.
    pub fn edge_scope(&self, edge: GraphEdgeId) -> Result<StringId> {
        Ok(self.edge_scope[self.edge_idx(edge)?])
    }

    /// The raw `u32` resolution-outcome flags of `edge`.
    pub fn edge_flags(&self, edge: GraphEdgeId) -> Result<u32> {
        Ok(self.edge_flags[self.edge_idx(edge)?])
    }

    // --- Raw column accessors for the `.pcsr` writer (crate-internal). ---

    pub(crate) fn parts_ref(&self) -> SnapshotPartsRef<'_> {
        SnapshotPartsRef {
            run_id: self.run_id,
            node_version_id: &self.node_version_id,
            node_depth: &self.node_depth,
            node_flags: &self.node_flags,
            node_attr: self.node_attr.as_deref(),
            out_offsets: &self.out_offsets,
            out_targets: &self.out_targets,
            out_edge_ids: &self.out_edge_ids,
            in_offsets: &self.in_offsets,
            in_sources: &self.in_sources,
            in_edge_ids: &self.in_edge_ids,
            edge_ordinal: &self.edge_ordinal,
            edge_requirement_id: &self.edge_requirement_id,
            edge_kind: &self.edge_kind,
            edge_scope: &self.edge_scope,
            edge_flags: &self.edge_flags,
        }
    }
}

/// Borrowed view over a snapshot's columns, used by the `.pcsr` writer.
pub(crate) struct SnapshotPartsRef<'a> {
    pub run_id: RunId,
    pub node_version_id: &'a [VersionId],
    pub node_depth: &'a [u32],
    pub node_flags: &'a [NodeFlags],
    pub node_attr: Option<&'a [StringId]>,
    pub out_offsets: &'a [u32],
    pub out_targets: &'a [GraphNodeId],
    pub out_edge_ids: &'a [GraphEdgeId],
    pub in_offsets: &'a [u32],
    pub in_sources: &'a [GraphNodeId],
    pub in_edge_ids: &'a [GraphEdgeId],
    pub edge_ordinal: &'a [u32],
    pub edge_requirement_id: &'a [RequirementId],
    pub edge_kind: &'a [u16],
    pub edge_scope: &'a [StringId],
    pub edge_flags: &'a [u32],
}

/// `RequirementId` sentinel meaning "this edge has no requirement". The model's
/// `GraphEdge.requirement_id` is `Option`; the CSR column packs `None` as this.
pub(crate) const NO_REQUIREMENT: u64 = u64::MAX;

fn validate_offsets(offsets: &[u32], n: usize, m: usize, which: &str) -> Result<()> {
    if offsets.len() != n + 1 {
        return Err(corrupt(format!(
            "{which}_offsets length {} != node_count+1 ({})",
            offsets.len(),
            n + 1
        )));
    }
    if offsets[0] != 0 {
        return Err(corrupt(format!("{which}_offsets[0] must be 0")));
    }
    if offsets[n] as usize != m {
        return Err(corrupt(format!(
            "{which}_offsets[N] {} != edge_count {m}",
            offsets[n]
        )));
    }
    for w in offsets.windows(2) {
        if w[0] > w[1] {
            return Err(corrupt(format!("{which}_offsets not non-decreasing")));
        }
    }
    Ok(())
}

fn validate_neighbors(neighbors: &[GraphNodeId], node_count: u32, which: &str) -> Result<()> {
    for &v in neighbors {
        if v.get() >= node_count {
            return Err(corrupt(format!(
                "{which} entry {} out of range (node_count {node_count})",
                v.get()
            )));
        }
    }
    Ok(())
}
