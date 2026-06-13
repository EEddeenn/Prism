//! Topological data analysis over a [`prism_graph::GraphSnapshot`] (architecture
//! doc, *TDA Architecture*, *TDA Store*, *Persistence Mapping*).
//!
//! v0.1 computes **ordinary persistent homology of the underlying undirected
//! 1-complex**: 0-cells are graph nodes, 1-cells are unique undirected edges
//! (direction and duplicates dropped). A caller-supplied per-node scalar drives
//! the filtration; edge values follow the lower-star / max rule so the filtration
//! is monotone. `H0` is connected components (union-find); `H1` is the cycle rank
//! (Z/2 boundary reduction) — with no 2-cells, every 1-cycle is essential.
//!
//! ## Pipeline
//!
//! 1. [`Filtration`] supplies one scalar per node ([`depth_filtration`] is the
//!    node-depth convenience).
//! 2. [`Complex::build`] turns a snapshot + scalars into a filtered 1-complex in
//!    the total reduction order `(birth_value, dim, cell_id)` (faces strictly
//!    before cofaces), asserting monotonicity.
//! 3. [`PersistenceResult::compute`] produces [`PersistencePair`]s and their
//!    [`GeneratorEntry`] back-mapping to graph nodes/edges.
//! 4. [`write_ptda`] / [`read_ptda`] serialize the whole filtration + persistence
//!    result to the little-endian `.ptda` image with a whole-image CRC-32.
//!
//! Pure safe Rust over [`prism_model`] types and [`prism_codec`] byte primitives;
//! no I/O is performed here.

pub mod complex;
pub mod filtration;
pub mod format;
pub mod persistence;

pub use complex::{Cell, Complex, ObjectType};
pub use filtration::{depth_filtration, Direction, Filtration, FiltrationType};
pub use format::{read_ptda, write_ptda, Ptda, PTDA_FORMAT_VERSION, PTDA_MAGIC};
pub use persistence::{GeneratorEntry, PersistencePair, PersistenceResult};

#[cfg(test)]
mod integration_tests {
    //! Known-answer tests over small graphs built with `prism_graph::GraphBuilder`.

    use super::*;
    use prism_graph::{BuildEdge, BuildNode, GraphBuilder, GraphSnapshot};
    use prism_model::{FiltrationId, GraphNodeId, RunId, VersionId};

    fn nid(i: u32) -> GraphNodeId {
        GraphNodeId::new(i)
    }

    /// Two disjoint trees: nodes {0-1-2} and {3-4}, edges only within each tree.
    fn two_trees() -> GraphSnapshot {
        let mut b = GraphBuilder::new();
        for i in 0..5u64 {
            b.add_node(BuildNode::new(VersionId::new(100 + i), i as u32))
                .unwrap();
        }
        b.add_edge(BuildEdge::new(nid(0), nid(1), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(2), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(3), nid(4), 0)).unwrap();
        b.finalize(RunId::new(1)).unwrap()
    }

    /// A single 4-cycle 0->1->2->3->0.
    fn single_cycle() -> GraphSnapshot {
        let mut b = GraphBuilder::new();
        for i in 0..4u64 {
            b.add_node(BuildNode::new(VersionId::new(200 + i), i as u32))
                .unwrap();
        }
        b.add_edge(BuildEdge::new(nid(0), nid(1), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(2), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(2), nid(3), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(3), nid(0), 0)).unwrap();
        b.finalize(RunId::new(2)).unwrap()
    }

    /// A plain tree (path 0-1-2-3).
    fn tree() -> GraphSnapshot {
        let mut b = GraphBuilder::new();
        for i in 0..4u64 {
            b.add_node(BuildNode::new(VersionId::new(300 + i), i as u32))
                .unwrap();
        }
        b.add_edge(BuildEdge::new(nid(0), nid(1), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(2), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(2), nid(3), 0)).unwrap();
        b.finalize(RunId::new(3)).unwrap()
    }

    #[test]
    fn two_trees_have_two_essential_h0() {
        let g = two_trees();
        let f = depth_filtration(&g).unwrap();
        let c = Complex::build(&g, f.values()).unwrap();
        let r = PersistenceResult::compute(&c, 0).unwrap();
        assert_eq!(r.essential_count(0), 2, "two components -> two H0 classes");
        assert_eq!(r.essential_count(1), 0, "no cycles -> no H1");
    }

    #[test]
    fn single_cycle_has_h1_rank_one() {
        let g = single_cycle();
        let f = depth_filtration(&g).unwrap();
        let c = Complex::build(&g, f.values()).unwrap();
        let r = PersistenceResult::compute(&c, 0).unwrap();
        assert_eq!(r.essential_count(0), 1, "connected -> one H0 class");
        assert_eq!(r.essential_count(1), 1, "one cycle -> H1 rank 1");
    }

    #[test]
    fn tree_has_no_h1() {
        let g = tree();
        let f = depth_filtration(&g).unwrap();
        let c = Complex::build(&g, f.values()).unwrap();
        let r = PersistenceResult::compute(&c, 0).unwrap();
        assert_eq!(r.essential_count(0), 1);
        assert_eq!(r.essential_count(1), 0, "a tree has no cycles");
    }

    #[test]
    fn nan_filtration_value_is_rejected() {
        let g = tree();
        let mut vals = depth_filtration(&g).unwrap().values().to_vec();
        vals[0] = f64::NAN;
        assert!(Complex::build(&g, &vals).is_err());
    }

    #[test]
    fn ptda_roundtrip_is_exact_and_detects_corruption() {
        let g = single_cycle();
        let f = depth_filtration(&g).unwrap();
        let c = Complex::build(&g, f.values()).unwrap();
        let r = PersistenceResult::compute(&c, 0).unwrap();
        let ptda = Ptda::new(FiltrationId::new(7), g.run_id(), &f, &c, r);

        let bytes = write_ptda(&ptda);
        let back = read_ptda(&bytes).unwrap();
        // Re-serializing the parsed image is byte-identical.
        assert_eq!(write_ptda(&back), bytes);
        assert_eq!(back.pairs().len(), ptda.pairs().len());

        // Single-byte body corruption is detected.
        let mut bad = bytes.clone();
        let idx = bad.len() / 2;
        bad[idx] ^= 0xFF;
        assert!(read_ptda(&bad).is_err());
    }

    #[test]
    fn figure_eight_has_h1_rank_two() {
        // Two triangles sharing node 0: {0,1,2} and {0,3,4}. Betti-1 = 2.
        let mut b = GraphBuilder::new();
        for i in 0..5u64 {
            b.add_node(BuildNode::new(VersionId::new(i), i as u32)).unwrap();
        }
        b.add_edge(BuildEdge::new(nid(0), nid(1), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(2), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(2), nid(0), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(0), nid(3), 1)).unwrap();
        b.add_edge(BuildEdge::new(nid(3), nid(4), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(4), nid(0), 0)).unwrap();
        let g = b.finalize(RunId::new(8)).unwrap();
        let f = depth_filtration(&g).unwrap();
        let c = Complex::build(&g, f.values()).unwrap();
        let r = PersistenceResult::compute(&c, 0).unwrap();
        assert_eq!(r.essential_count(0), 1, "connected");
        assert_eq!(r.essential_count(1), 2, "two independent cycles");
    }

    #[test]
    fn cycle_generator_maps_to_cycle_edges() {
        let g = single_cycle();
        let f = depth_filtration(&g).unwrap();
        let c = Complex::build(&g, f.values()).unwrap();
        let r = PersistenceResult::compute(&c, 0).unwrap();
        // The single essential H1 pair.
        let h1 = r
            .pairs()
            .iter()
            .find(|p| p.dim == 1 && p.death_cell.is_none())
            .expect("one H1 generator");
        let edges: std::collections::BTreeSet<u64> = r
            .generators_for(h1.pair_id)
            .filter(|gen| gen.object_type == ObjectType::Edge)
            .map(|gen| gen.object_id)
            .collect();
        // All four edges of the 4-cycle support the generator.
        assert_eq!(edges.len(), 4, "cycle generator covers all 4 edges");
    }
}
