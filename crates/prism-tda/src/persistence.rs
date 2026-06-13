//! Ordinary persistent homology of the undirected 1-complex (architecture doc,
//! *Backend decision (Open Question 5)* and *Persistence Mapping*).
//!
//! - **H0** is computed with a union-find over the 0-cells as the 1-cells are
//!   added in filtration order. Adding an edge that unites two distinct
//!   components kills the **younger** component (the one whose representative
//!   0-cell was born later) — a finite `(birth_cell, death_cell)` pair. Each
//!   surviving component is an **essential** class (death = +inf).
//! - **H1** is computed with the standard Z/2 boundary-matrix reduction over the
//!   1-cells. In v0.1 there are no 2-cells, so every 1-cycle is **essential**
//!   (death = +inf); its birth value is the max edge value of the cycle.
//!
//! Generator support maps each pair's representative chain back to graph objects
//! (nodes/edges) with a Z/2 contribution. Representatives are **one valid
//! choice** under the documented cell order, not the unique minimal one.

use prism_model::{CellId, PairId, Result};

use crate::complex::{Complex, ObjectType};

/// `death_cell` / `death_value` sentinel meaning "essential class" (never dies).
pub(crate) const NULL_CELL: u64 = u64::MAX;

/// One persistence pair: a topological feature born at `birth_cell` and either
/// killed at `death_cell` (finite) or essential (`death_cell == None`).
#[derive(Clone, Debug, PartialEq)]
pub struct PersistencePair {
    /// Globally-unique pair id (see [`PersistenceResult`] for allocation).
    pub pair_id: PairId,
    /// Homology dimension (0 or 1).
    pub dim: u8,
    /// Birth cell (filtration-local [`CellId`]); a dim-`k` cell.
    pub birth_cell: CellId,
    /// Death cell (a dim-`k+1` cell) for a finite pair, else `None` (essential).
    pub death_cell: Option<CellId>,
    /// Birth filtration value.
    pub birth_value: f64,
    /// Death filtration value, or `None` (== +inf) for an essential class.
    pub death_value: Option<f64>,
}

/// A back-mapping of one cell in a pair's representative chain to a graph object.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratorEntry {
    /// The pair this generator entry belongs to.
    pub pair_id: PairId,
    /// What kind of graph object `object_id` names.
    pub object_type: ObjectType,
    /// Logical id (node index or edge index), qualified by the run.
    pub object_id: u64,
    /// Z/2 membership coefficient (always `1` in v0.1: presence).
    pub contribution: i8,
}

/// The full persistence result for one filtration: the diagram (pairs) plus the
/// per-pair generator support, ready to serialize to `.ptda`.
#[derive(Clone, Debug)]
pub struct PersistenceResult {
    pairs: Vec<PersistencePair>,
    generators: Vec<GeneratorEntry>,
}

impl PersistenceResult {
    /// The persistence pairs (H0 then H1), in stable emission order.
    #[inline]
    pub fn pairs(&self) -> &[PersistencePair] {
        &self.pairs
    }

    /// All generator-support entries (flattened across pairs; group by `pair_id`).
    #[inline]
    pub fn generators(&self) -> &[GeneratorEntry] {
        &self.generators
    }

    /// Generator entries for a specific pair.
    pub fn generators_for(&self, pair_id: PairId) -> impl Iterator<Item = &GeneratorEntry> {
        self.generators.iter().filter(move |g| g.pair_id == pair_id)
    }

    /// Number of essential (infinite) pairs of dimension `dim`.
    pub fn essential_count(&self, dim: u8) -> usize {
        self.pairs
            .iter()
            .filter(|p| p.dim == dim && p.death_cell.is_none())
            .count()
    }

    /// Construct from already-computed pairs/generators (used by the reader).
    pub(crate) fn from_parts(
        pairs: Vec<PersistencePair>,
        generators: Vec<GeneratorEntry>,
    ) -> Self {
        Self { pairs, generators }
    }
}

/// A union-find with the standard path-compression / union-by-rank, plus a
/// per-component "birth" cell id used to decide which component is younger.
struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u32>,
    /// The 0-cell id at which each root component was born (its oldest member).
    birth: Vec<u64>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
            birth: (0..n as u64).collect(),
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    /// Union the components of `a` and `b`. Returns `Some(younger_birth_cell)` if
    /// they were distinct (the younger component is killed at this birth cell),
    /// else `None`. The surviving root keeps the *older* (smaller) birth cell — an
    /// elder rule that makes H0 pairing deterministic.
    fn union(&mut self, a: usize, b: usize) -> Option<u64> {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return None;
        }
        let (older_root, younger_root) = if self.birth[ra] <= self.birth[rb] {
            (ra, rb)
        } else {
            (rb, ra)
        };
        let older_birth = self.birth[older_root];
        let younger_birth = self.birth[younger_root];

        // Union by rank; then force the merged root to carry the older birth.
        let merged = if self.rank[ra] < self.rank[rb] {
            self.parent[ra] = rb;
            rb
        } else if self.rank[ra] > self.rank[rb] {
            self.parent[rb] = ra;
            ra
        } else {
            self.parent[rb] = ra;
            self.rank[ra] += 1;
            ra
        };
        self.birth[merged] = older_birth;
        Some(younger_birth)
    }
}

impl PersistenceResult {
    /// Compute persistent homology of `complex`. Pair ids are allocated globally
    /// starting at `pair_id_base` in emission order (H0 finite, then H0 essential,
    /// then H1 essential), so `(filtration_id, base)` makes them globally unique.
    pub fn compute(complex: &Complex, pair_id_base: u64) -> Result<Self> {
        let cells = complex.cells();
        let n = complex.node_count();

        let mut pairs: Vec<PersistencePair> = Vec::new();
        let mut generators: Vec<GeneratorEntry> = Vec::new();
        let mut next_pair = pair_id_base;

        // --- H0 via union-find over 0-cells, edges added in filtration order. ---
        //
        // Map each cell id -> its node (for 0-cells) and the inverse node -> its
        // 0-cell id. Cells are sorted by `(birth_value, dim, …)`, so a 0-cell id
        // is NOT the node index under a non-uniform filtration.
        let mut cell_to_node: Vec<usize> = vec![usize::MAX; cells.len()];
        let mut node_to_cell0: Vec<u64> = vec![0; n];
        for (cid, cell) in cells.iter().enumerate() {
            if cell.dim == 0 {
                let node = cell.vertices[0].get() as usize;
                cell_to_node[cid] = node;
                node_to_cell0[node] = cid as u64;
            }
        }
        // Seed the union-find so each node's component is "born" at its 0-cell id
        // (already in filtration order). Without this, `birth` holds node indices,
        // which differ from 0-cell ids whenever the filtration is non-uniform —
        // corrupting H0 birth values, the older/younger pairing, and the mapped
        // generator node (a node index used as a cell id could even hit the
        // `usize::MAX` 1-cell sentinel, which truncates to `GraphNodeId::NULL`).
        let mut uf = UnionFind::new(n);
        uf.birth = node_to_cell0;

        // Track which 1-cells were "positive" (created a cycle) vs "negative"
        // (merged components) so H1 reduction can focus on positive ones. Here we
        // just record finite H0 deaths.
        let mut edge_is_negative = vec![false; cells.len()];

        for (cid, cell) in cells.iter().enumerate() {
            if cell.dim != 1 {
                continue;
            }
            let u = cell.vertices[0].get() as usize;
            let v = cell.vertices[1].get() as usize;
            // A `Some` result means the edge united two distinct components; the
            // younger one dies. `None` means both endpoints were already merged.
            if let Some(younger_birth_cell) = uf.union(u, v) {
                // Younger component dies, killed by this edge.
                edge_is_negative[cid] = true;
                let birth_node = cell_to_node[younger_birth_cell as usize];
                let birth_value = cells[younger_birth_cell as usize].birth_value;
                let death_value = cell.birth_value;
                let pair_id = PairId::new(next_pair);
                next_pair += 1;
                pairs.push(PersistencePair {
                    pair_id,
                    dim: 0,
                    birth_cell: CellId::new(younger_birth_cell),
                    death_cell: Some(CellId::new(cid as u64)),
                    birth_value,
                    death_value: Some(death_value),
                });
                // Generator support for an H0 pair: the younger component's birth
                // node and the edge that merged it. One valid representative.
                generators.push(GeneratorEntry {
                    pair_id,
                    object_type: ObjectType::Node,
                    object_id: birth_node as u64,
                    contribution: 1,
                });
                generators.push(GeneratorEntry {
                    pair_id,
                    object_type: ObjectType::Edge,
                    object_id: cell.source_id,
                    contribution: 1,
                });
            }
        }

        // Essential H0 classes: one per surviving component, born at its oldest
        // 0-cell. Emit in ascending birth-cell order for determinism.
        let mut essential_births: Vec<u64> = Vec::new();
        let mut roots_seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for node in 0..n {
            let r = uf.find(node);
            if roots_seen.insert(r) {
                essential_births.push(uf.birth[r]);
            }
        }
        essential_births.sort_unstable();
        for birth_cell in essential_births {
            let birth_node = cell_to_node[birth_cell as usize];
            let birth_value = cells[birth_cell as usize].birth_value;
            let pair_id = PairId::new(next_pair);
            next_pair += 1;
            pairs.push(PersistencePair {
                pair_id,
                dim: 0,
                birth_cell: CellId::new(birth_cell),
                death_cell: None,
                birth_value,
                death_value: None,
            });
            generators.push(GeneratorEntry {
                pair_id,
                object_type: ObjectType::Node,
                object_id: birth_node as u64,
                contribution: 1,
            });
        }

        // --- H1 via Z/2 boundary-matrix reduction over the 1-cells. ---
        //
        // Each 1-cell column is the Z/2 chain of its boundary 0-cells (`faces`).
        // We reduce columns left-to-right keyed by their lowest set row (the
        // "low"). A 1-cell whose column reduces to empty is a *positive* cell: it
        // creates an essential 1-cycle (no 2-cells means it never dies). We track
        // the set of contributing 1-cells per column (`support`) so the cycle's
        // edges can be recovered as the representative chain.
        //
        // `low_to_col[r]` = index (into `reduced_cols`/`reduced_support`) of the
        // reduced column currently owning low-row `r`.
        let mut low_to_col: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
        let mut reduced_cols: ColStore = Vec::new();
        let mut reduced_support: SupportStore = Vec::new();

        for (cid, cell) in cells.iter().enumerate() {
            if cell.dim != 1 {
                continue;
            }
            // Working column over 0-cell rows (sorted, deduped, Z/2).
            let mut col: Vec<u64> = cell.faces.clone();
            col.sort_unstable();
            col.dedup();
            // 1-cells contributing to this column (for the representative cycle).
            let mut support: Vec<usize> = vec![cid];

            while let Some(&low) = col.last() {
                if let Some(&other) = low_to_col.get(&low) {
                    // Add the owning column (Z/2 symmetric difference) and merge
                    // its support.
                    col = sym_diff(&col, &reduced_cols[other]);
                    support = sym_diff_usize(&support, &reduced_support[other]);
                } else {
                    break;
                }
            }

            if let Some(&low) = col.last() {
                // Negative 1-cell: it kills an H0 component (already recorded by
                // union-find). Register it as the owner of its low row.
                debug_assert!(edge_is_negative[cid]);
                low_to_col.insert(low, reduced_cols.len());
                reduced_cols.push(col);
                reduced_support.push(support);
            } else {
                // Positive 1-cell: column reduced to empty -> essential 1-cycle.
                let pair_id = PairId::new(next_pair);
                next_pair += 1;
                // Birth value = max edge value over the cycle's 1-cells. The
                // birth cell is this 1-cell (the one that completed the cycle).
                let birth_value = support
                    .iter()
                    .map(|&c| cells[c].birth_value)
                    .fold(f64::NEG_INFINITY, f64::max);
                pairs.push(PersistencePair {
                    pair_id,
                    dim: 1,
                    birth_cell: CellId::new(cid as u64),
                    death_cell: None,
                    birth_value,
                    death_value: None,
                });
                // Generator support: each 1-cell in the cycle (its edge) plus the
                // nodes it touches. Emit edges (the cycle) and the distinct nodes.
                let mut node_set: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
                let mut sorted_support = support.clone();
                sorted_support.sort_unstable();
                for &c in &sorted_support {
                    let ecell = &cells[c];
                    generators.push(GeneratorEntry {
                        pair_id,
                        object_type: ObjectType::Edge,
                        object_id: ecell.source_id,
                        contribution: 1,
                    });
                    for vtx in &ecell.vertices {
                        node_set.insert(vtx.get() as u64);
                    }
                }
                for node in node_set {
                    generators.push(GeneratorEntry {
                        pair_id,
                        object_type: ObjectType::Node,
                        object_id: node,
                        contribution: 1,
                    });
                }
            }
        }

        Ok(PersistenceResult { pairs, generators })
    }
}

/// Reduced boundary columns (sorted 0-cell row sets) per registered 1-cell.
type ColStore = Vec<Vec<u64>>;
/// 1-cell support sets aligned with [`ColStore`].
type SupportStore = Vec<Vec<usize>>;

/// Z/2 symmetric difference of two sorted, deduplicated `u64` row vectors.
fn sym_diff(a: &[u64], b: &[u64]) -> Vec<u64> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                out.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                out.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}

/// Z/2 symmetric difference of two sorted, deduplicated `usize` vectors.
fn sym_diff_usize(a: &[usize], b: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                out.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                out.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filtration::depth_filtration;
    use prism_graph::{BuildEdge, BuildNode, GraphBuilder, GraphSnapshot};
    use prism_model::{GraphNodeId, RunId, VersionId};

    fn nid(i: u32) -> GraphNodeId {
        GraphNodeId::new(i)
    }

    #[test]
    fn sym_diff_is_z2() {
        assert_eq!(sym_diff(&[1, 2, 3], &[2, 4]), vec![1, 3, 4]);
        assert_eq!(sym_diff(&[1, 2], &[1, 2]), Vec::<u64>::new());
        assert_eq!(sym_diff_usize(&[0, 5], &[5, 9]), vec![0, 9]);
    }

    #[test]
    fn union_find_elder_rule_kills_younger() {
        // Birth cells 0,1,2 (the elder is the smaller id). Uniting 0 and 2 must
        // kill cell 2 (younger). Uniting the merged set with 1 must kill 1.
        let mut uf = UnionFind::new(3);
        assert_eq!(uf.union(0, 2), Some(2));
        assert_eq!(uf.union(0, 1), Some(1));
        // Already connected -> no death.
        assert_eq!(uf.union(1, 2), None);
    }

    // Path 0-1-2 then closing edge 2-0 makes one cycle; depth filtration.
    fn triangle() -> GraphSnapshot {
        let mut b = GraphBuilder::new();
        for i in 0..3u64 {
            b.add_node(BuildNode::new(VersionId::new(i), i as u32)).unwrap();
        }
        b.add_edge(BuildEdge::new(nid(0), nid(1), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(2), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(2), nid(0), 0)).unwrap();
        b.finalize(RunId::new(1)).unwrap()
    }

    #[test]
    fn h0_finite_pairs_for_a_connected_tree() {
        // A 3-node path has 3 components initially; 2 merges -> 2 finite H0 pairs
        // plus 1 essential survivor.
        let mut b = GraphBuilder::new();
        for i in 0..3u64 {
            b.add_node(BuildNode::new(VersionId::new(i), i as u32)).unwrap();
        }
        b.add_edge(BuildEdge::new(nid(0), nid(1), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(2), 0)).unwrap();
        let g = b.finalize(RunId::new(1)).unwrap();
        let f = depth_filtration(&g).unwrap();
        let c = Complex::build(&g, f.values()).unwrap();
        let r = PersistenceResult::compute(&c, 0).unwrap();
        let finite_h0 = r
            .pairs()
            .iter()
            .filter(|p| p.dim == 0 && p.death_cell.is_some())
            .count();
        assert_eq!(finite_h0, 2);
        assert_eq!(r.essential_count(0), 1);
        // Every finite H0 pair obeys birth_value <= death_value.
        for p in r.pairs().iter().filter(|p| p.death_value.is_some()) {
            assert!(p.birth_value <= p.death_value.unwrap());
        }
    }

    #[test]
    fn h1_birth_is_max_edge_value_of_cycle() {
        let g = triangle();
        let f = depth_filtration(&g).unwrap(); // depths 0,1,2 -> edge vals 1,2,2
        let c = Complex::build(&g, f.values()).unwrap();
        let r = PersistenceResult::compute(&c, 0).unwrap();
        let h1 = r.pairs().iter().find(|p| p.dim == 1).unwrap();
        assert!(h1.death_cell.is_none(), "no 2-cells -> essential");
        // Max edge value over {0,1}=1,{1,2}=2,{0,2}=2 is 2.
        assert_eq!(h1.birth_value, 2.0);
    }

    #[test]
    fn generators_are_valid_nodes_under_nonuniform_filtration() {
        // Root = node 2 (depth 0) -> node 1 (depth 1) -> node 0 (depth 2). The
        // depth filtration sorts 0-cells by depth, so a 0-cell id is NOT the node
        // index and 1-cells interleave. This is the case that used to seed the
        // union-find with node indices and map an H0 generator onto a 1-cell
        // slot's `usize::MAX` sentinel -> `GraphNodeId::NULL` (out of range).
        let mut b = GraphBuilder::new();
        for i in 0..3u64 {
            b.add_node(BuildNode::new(VersionId::new(i), i as u32)).unwrap();
        }
        b.add_edge(BuildEdge::new(nid(2), nid(1), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(0), 0)).unwrap();
        let g = b.finalize(RunId::new(1)).unwrap();
        let f = depth_filtration(&g).unwrap();
        let c = Complex::build(&g, f.values()).unwrap();
        let r = PersistenceResult::compute(&c, 0).unwrap();

        // Every node generator must name a real node (0..node_count), never the
        // sentinel that truncates to GraphNodeId::NULL.
        let nc = c.node_count() as u64;
        for gen in r.generators() {
            if matches!(gen.object_type, ObjectType::Node) {
                assert!(gen.object_id < nc, "node generator out of range: {}", gen.object_id);
            }
        }
        // H0 births are the real filtration depths (0..=2), not a misindexed
        // cell's value.
        for p in r.pairs().iter().filter(|p| p.dim == 0) {
            assert!(
                (0.0..=2.0).contains(&p.birth_value),
                "bogus H0 birth_value {}",
                p.birth_value
            );
        }
    }

    #[test]
    fn pair_ids_are_globally_unique_with_base() {
        let g = triangle();
        let f = depth_filtration(&g).unwrap();
        let c = Complex::build(&g, f.values()).unwrap();
        let r = PersistenceResult::compute(&c, 5000).unwrap();
        let ids: std::collections::BTreeSet<u64> =
            r.pairs().iter().map(|p| p.pair_id.get()).collect();
        assert_eq!(ids.len(), r.pairs().len(), "all pair ids distinct");
        assert!(ids.iter().all(|&i| i >= 5000), "ids start at the base");
    }
}
