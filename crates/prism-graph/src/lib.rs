//! Immutable CSR graph snapshots and the algorithms over them.
//!
//! This crate owns the **resolved-graph** layer (architecture doc, *CSR Graph
//! Snapshots*, *Graph Algorithms*). A resolver run produces a
//! [`GraphSnapshot`]: an immutable, owned set of compressed-sparse-row arrays —
//! forward CSR for dependency traversal, reverse CSR for impact analysis, and
//! per-node / per-edge attribute columns. Node and edge indices are dense and
//! snapshot-local; they are meaningful only within their [`prism_model::RunId`].
//!
//! ## Pipeline
//!
//! 1. [`GraphBuilder`] accumulates transient [`BuildNode`]s and [`BuildEdge`]s
//!    and [`GraphBuilder::finalize`]s them into a validated [`GraphSnapshot`],
//!    sorting edges by source (forward CSR, ordinals preserved) and by target
//!    (reverse CSR) and assigning dense [`prism_model::GraphEdgeId`]s.
//! 2. [`write_pcsr`] / [`read_pcsr`] serialize a snapshot to/from the
//!    little-endian `.pcsr` image with a whole-image CRC-32; roundtrips are exact
//!    and any corruption is detected on read.
//! 3. Algorithms run over the immutable snapshot: bounded BFS
//!    ([`forward_reachable`] / [`reverse_reachable`]), Tarjan SCC + condensation
//!    order ([`tarjan_scc`]), and duplicate-version analysis
//!    ([`version_clusters`] / [`family_clusters`]).
//!
//! Everything is pure safe Rust over [`prism_model`] types and the
//! [`prism_codec`] byte primitives; no I/O is performed here (callers own files).

pub mod builder;
pub mod cycles;
pub mod duplicate;
pub mod format;
pub mod scc;
pub mod snapshot;
pub mod structure;
pub mod traversal;

pub use builder::{kind_from_u16, kind_to_u16, BuildEdge, BuildNode, GraphBuilder};
pub use cycles::{classify_cycles, CycleClass, CycleInfo};
pub use duplicate::{family_clusters, version_clusters, FamilyCluster, VersionCluster};
pub use format::{read_pcsr, write_pcsr, PCSR_FORMAT_VERSION, PCSR_MAGIC};
pub use scc::{condensation_topo_order, tarjan_scc, SccResult};
pub use snapshot::GraphSnapshot;
pub use structure::{
    dominators, explain, roots, scope_footprint, scope_summary, weak_components, Chokepoint,
    DomTree, Explanation, Introducer, ScopeFootprint, PRODUCTION_KINDS,
};
pub use traversal::{
    forward_reachable, reverse_reachable, EdgeKindMask, Reachable, TraversalOptions,
};

#[cfg(test)]
mod integration_tests {
    //! End-to-end: build a graph with a cycle, round-trip it through `.pcsr`,
    //! and confirm traversal / SCC / duplicate views agree across the roundtrip.

    use crate::*;
    use prism_model::{GraphNodeId, RunId, VersionId};

    fn nid(i: u32) -> GraphNodeId {
        GraphNodeId::new(i)
    }

    // A small graph that exercises every feature:
    //   root 0 -> 1, 0 -> 2
    //   1 -> 3, 2 -> 3
    //   3 -> 1   (back edge: 1 -> 3 -> 1 is a cycle)
    //   4 isolated, sharing version with node 1 (duplicate version).
    fn build_graph() -> GraphSnapshot {
        let mut b = GraphBuilder::new();
        // versions: n0=v10, n1=v20, n2=v30, n3=v40, n4=v20 (dup of n1).
        let specs = [
            (10u64, 0u32),
            (20, 1),
            (30, 1),
            (40, 2),
            (20, 3),
        ];
        for (v, d) in specs {
            b.add_node(BuildNode::new(VersionId::new(v), d)).unwrap();
        }
        b.add_edge(BuildEdge::new(nid(0), nid(1), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(0), nid(2), 1)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(3), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(2), nid(3), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(3), nid(1), 0)).unwrap(); // cycle 1<->3
        b.finalize(RunId::new(42)).unwrap()
    }

    #[test]
    fn end_to_end() {
        let g = build_graph();

        // Roundtrip through .pcsr is exact.
        let bytes = write_pcsr(&g);
        let g = read_pcsr(&bytes).unwrap();
        assert_eq!(write_pcsr(&g), bytes);

        // Forward reachability from root 0 (root excluded): 1, 2, 3.
        let fwd = forward_reachable(&g, nid(0), TraversalOptions::default()).unwrap();
        let mut got: Vec<u32> = fwd.nodes.iter().map(|x| x.get()).collect();
        got.sort_unstable();
        assert_eq!(got, vec![1, 2, 3]);
        assert!(!fwd.nodes.contains(&nid(0)));

        // Reverse reachability from 3: ancestors 0, 1, 2.
        let rev = reverse_reachable(&g, nid(3), TraversalOptions::default()).unwrap();
        let mut anc: Vec<u32> = rev.nodes.iter().map(|x| x.get()).collect();
        anc.sort_unstable();
        assert_eq!(anc, vec![0, 1, 2]);

        // SCC: {1,3} is the cycle; 0, 2, 4 are singletons.
        let scc = tarjan_scc(&g).unwrap();
        assert!(scc.has_multinode_cycle());
        let cyc = scc.components.iter().find(|c| c.len() > 1).unwrap();
        let mut cyc_ids: Vec<u32> = cyc.iter().map(|x| x.get()).collect();
        cyc_ids.sort_unstable();
        assert_eq!(cyc_ids, vec![1, 3]);

        // Duplicate version: v20 occurs at nodes 1 and 4.
        let dups: Vec<_> = version_clusters(&g)
            .unwrap()
            .into_iter()
            .filter(|c| c.is_duplicated())
            .collect();
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].version_id, VersionId::new(20));
        assert_eq!(dups[0].occurrence_count(), 2);
    }
}
