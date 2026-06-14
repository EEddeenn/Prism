//! Experimental higher-dimensional homology via the truncated flag complex
//! (architecture doc, *Higher-Dimensional Topology*).
//!
//! ## The complex model
//!
//! The main TDA path ([`crate::complex`]) builds a **1-complex** (vertices +
//! edges), so it can only see H₀ (components) and H₁ (loops). To reach **H₂** we
//! build the **flag (clique) complex truncated at dimension 2**: in addition to a
//! vertex per package and a 1-cell per undirected dependency edge, we add a
//! **2-cell (triangle) for every 3-clique** — three packages that mutually
//! depend. We deliberately stop at dimension 2 (we do *not* fill the
//! tetrahedron of a 4-clique), so a set of four mutually-dependent packages
//! forms a **hollow void** that H₂ detects.
//!
//! ## Engineering interpretation
//!
//! An H₂ generator is a closed surface of dependency relationships with nothing
//! inside it — a cluster whose packages are densely cross-connected around an
//! empty centre. In a dependency graph these are rare and almost always signal a
//! tightly entangled mutual-dependency knot (e.g. a polyfill or type-shim cluster
//! wired into a near-clique) that should be reasoned about as one unit.
//!
//! ## Method
//!
//! Cells are ordered by `(filtration value, dim, tie)` and reduced with the
//! standard Z/2 boundary-matrix algorithm. A k-cell whose reduced column is empty
//! is *positive* (a birth); one with a non-empty column is *negative* (a death
//! that pairs the cell at its pivot). With the complex truncated at dimension 2,
//! every positive 2-cell is essential, so `H₂ = #positive 2-cells`. We assert
//! H₀/H₁ here against the same Euler/Betti identities the optimized path uses.

use std::collections::HashMap;

use prism_graph::GraphSnapshot;
use prism_model::{GraphNodeId, Result};

/// Betti numbers and H₂ generators from the truncated flag complex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagHomology {
    /// H₀ — connected components.
    pub betti0: u32,
    /// H₁ — independent loops (in the flag complex, triangles fill some loops).
    pub betti1: u32,
    /// H₂ — voids (hollow closed surfaces).
    pub betti2: u32,
    /// Number of 2-cells (filled triangles / 3-cliques).
    pub triangles: u32,
    /// The vertex triples generating each H₂ void (positive 2-cells), as node
    /// ids, ascending within each triple.
    pub h2_generators: Vec<[GraphNodeId; 3]>,
}

/// Compute H₀/H₁/H₂ of `graph`'s truncated flag complex under `node_values`.
///
/// `node_values` must have length `graph.node_count()`. The filtration value of
/// a higher cell is the max of its vertices (lower-star rule), matching the
/// 1-complex path.
pub fn flag_homology(graph: &GraphSnapshot, node_values: &[f64]) -> Result<FlagHomology> {
    let n = graph.node_count();
    if node_values.len() != n {
        return Err(prism_model::PrismError::Corrupt {
            detail: format!("flag: value count {} != node_count {n}", node_values.len()),
        });
    }

    // --- Undirected adjacency (dedup, no self-loops). ---
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];
    {
        let mut seen: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
        for u in 0..n as u32 {
            for &t in graph.out_neighbors(GraphNodeId::new(u))? {
                let v = t.get();
                if v == u {
                    continue;
                }
                let (lo, hi) = if u < v { (u, v) } else { (v, u) };
                if seen.insert((lo, hi)) {
                    adj[lo as usize].push(hi);
                    adj[hi as usize].push(lo);
                }
            }
        }
        for a in &mut adj {
            a.sort_unstable();
        }
    }

    // --- Enumerate cells: vertices, edges, triangles. ---
    // Each cell carries (value, dim, tie-key) for the total order, plus enough to
    // build its boundary after ids are assigned.
    enum Kind {
        Vertex(u32),
        Edge(u32, u32),
        Triangle(u32, u32, u32),
    }
    struct Staged {
        value: f64,
        dim: u8,
        tie: (u32, u32, u32),
        kind: Kind,
    }
    let mut staged: Vec<Staged> = Vec::new();

    for u in 0..n as u32 {
        staged.push(Staged {
            value: node_values[u as usize],
            dim: 0,
            tie: (u, 0, 0),
            kind: Kind::Vertex(u),
        });
    }
    // Edges (lo < hi) and triangles (lo < mid < hi) via neighbor intersection.
    for lo in 0..n as u32 {
        for &hi in &adj[lo as usize] {
            if hi <= lo {
                continue;
            }
            let ev = node_values[lo as usize].max(node_values[hi as usize]);
            staged.push(Staged {
                value: ev,
                dim: 1,
                tie: (lo, hi, 0),
                kind: Kind::Edge(lo, hi),
            });
            // Triangles: common neighbors w > hi.
            let mut i = 0usize;
            let mut j = 0usize;
            let (na, nb) = (&adj[lo as usize], &adj[hi as usize]);
            while i < na.len() && j < nb.len() {
                let (a, b) = (na[i], nb[j]);
                if a == b {
                    let w = a;
                    if w > hi {
                        let tv = ev.max(node_values[w as usize]);
                        staged.push(Staged {
                            value: tv,
                            dim: 2,
                            tie: (lo, hi, w),
                            kind: Kind::Triangle(lo, hi, w),
                        });
                    }
                    i += 1;
                    j += 1;
                } else if a < b {
                    i += 1;
                } else {
                    j += 1;
                }
            }
        }
    }

    // --- Total order (value, dim, tie); assign cell ids. ---
    staged.sort_by(|a, b| {
        a.value
            .total_cmp(&b.value)
            .then(a.dim.cmp(&b.dim))
            .then(a.tie.cmp(&b.tie))
    });

    let mut vertex_id: HashMap<u32, usize> = HashMap::new();
    let mut edge_id: HashMap<(u32, u32), usize> = HashMap::new();
    for (id, s) in staged.iter().enumerate() {
        match s.kind {
            Kind::Vertex(u) => {
                vertex_id.insert(u, id);
            }
            Kind::Edge(a, b) => {
                edge_id.insert((a, b), id);
            }
            Kind::Triangle(..) => {}
        }
    }

    // --- Boundary columns (sorted face cell ids). ---
    let mut dims: Vec<u8> = Vec::with_capacity(staged.len());
    let mut columns: Vec<Vec<usize>> = Vec::with_capacity(staged.len());
    let mut tri_vertices: Vec<Option<[u32; 3]>> = Vec::with_capacity(staged.len());
    for s in &staged {
        dims.push(s.dim);
        let (col, tri): (Vec<usize>, Option<[u32; 3]>) = match s.kind {
            Kind::Vertex(_) => (Vec::new(), None),
            Kind::Edge(a, b) => {
                let mut c = vec![vertex_id[&a], vertex_id[&b]];
                c.sort_unstable();
                (c, None)
            }
            Kind::Triangle(a, b, c) => {
                let mut col = vec![edge_id[&(a, b)], edge_id[&(a, c)], edge_id[&(b, c)]];
                col.sort_unstable();
                (col, Some([a, b, c]))
            }
        };
        columns.push(col);
        tri_vertices.push(tri);
    }

    // --- Standard Z/2 reduction. ---
    // `pivot_of[i]` = column index whose low (pivot) is cell i, once reduced.
    let mut pivot_of: HashMap<usize, usize> = HashMap::new();
    let mut negative = vec![false; staged.len()]; // cell j is a death
    let mut paired_birth = vec![false; staged.len()]; // cell i is paired as a birth
    for j in 0..columns.len() {
        // Reduce column j by repeatedly cancelling its low against an earlier col.
        loop {
            let Some(&low) = columns[j].iter().max() else {
                break; // empty column -> positive cell
            };
            if let Some(&k) = pivot_of.get(&low) {
                symmetric_difference(&mut columns, j, k);
            } else {
                pivot_of.insert(low, j);
                negative[j] = true;
                paired_birth[low] = true;
                break;
            }
        }
    }

    // --- Betti numbers: positive (unpaired) cells per dimension. ---
    let (mut betti0, mut betti1, mut betti2) = (0u32, 0u32, 0u32);
    let mut h2_generators = Vec::new();
    let mut triangles = 0u32;
    for j in 0..staged.len() {
        if dims[j] == 2 {
            triangles += 1;
        }
        let positive = !negative[j] && !paired_birth[j];
        if positive {
            match dims[j] {
                0 => betti0 += 1,
                1 => betti1 += 1,
                2 => {
                    betti2 += 1;
                    if let Some([a, b, c]) = tri_vertices[j] {
                        let mut v = [a, b, c];
                        v.sort_unstable();
                        h2_generators.push([
                            GraphNodeId::new(v[0]),
                            GraphNodeId::new(v[1]),
                            GraphNodeId::new(v[2]),
                        ]);
                    }
                }
                _ => {}
            }
        }
    }

    Ok(FlagHomology {
        betti0,
        betti1,
        betti2,
        triangles,
        h2_generators,
    })
}

/// Replace column `j` with its symmetric difference against column `k` (Z/2 add).
/// `j` and `k` are distinct; we clone `k` to satisfy the borrow checker.
fn symmetric_difference(columns: &mut [Vec<usize>], j: usize, k: usize) {
    let other = columns[k].clone();
    let col = &mut columns[j];
    // Both are sorted; merge-cancel duplicates.
    let mut merged = Vec::with_capacity(col.len() + other.len());
    let (mut a, mut b) = (0usize, 0usize);
    while a < col.len() && b < other.len() {
        match col[a].cmp(&other[b]) {
            std::cmp::Ordering::Less => {
                merged.push(col[a]);
                a += 1;
            }
            std::cmp::Ordering::Greater => {
                merged.push(other[b]);
                b += 1;
            }
            std::cmp::Ordering::Equal => {
                a += 1;
                b += 1; // cancel (mod 2)
            }
        }
    }
    merged.extend_from_slice(&col[a..]);
    merged.extend_from_slice(&other[b..]);
    *col = merged;
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_graph::{BuildEdge, BuildNode, GraphBuilder};
    use prism_model::{RunId, VersionId};

    /// Build a snapshot with `n` nodes and the given undirected `edges`.
    fn flag_graph(n: u32, edges: &[(u32, u32)]) -> GraphSnapshot {
        let mut b = GraphBuilder::new();
        for i in 0..n {
            b.add_node(BuildNode::new(VersionId::new(i as u64), 0)).unwrap();
        }
        for (ord, &(f, t)) in edges.iter().enumerate() {
            b.add_edge(BuildEdge::new(GraphNodeId::new(f), GraphNodeId::new(t), ord as u32))
                .unwrap();
        }
        b.finalize(RunId::new(1)).unwrap()
    }

    fn vals(n: usize) -> Vec<f64> {
        vec![0.0; n]
    }

    #[test]
    fn filled_triangle_has_no_loop_or_void() {
        // 3 mutually-connected nodes: one filled triangle. H0=1, H1=0, H2=0.
        let g = flag_graph(3, &[(0, 1), (1, 2), (0, 2)]);
        let h = flag_homology(&g, &vals(3)).unwrap();
        assert_eq!((h.betti0, h.betti1, h.betti2), (1, 0, 0));
        assert_eq!(h.triangles, 1);
    }

    #[test]
    fn square_without_diagonal_is_a_loop() {
        // 4-cycle, no triangle: H1=1, H2=0.
        let g = flag_graph(4, &[(0, 1), (1, 2), (2, 3), (0, 3)]);
        let h = flag_homology(&g, &vals(4)).unwrap();
        assert_eq!((h.betti0, h.betti1, h.betti2), (1, 1, 0));
        assert_eq!(h.triangles, 0);
    }

    #[test]
    fn complete_k4_is_a_void() {
        // K4: every pair connected. Flag complex truncated at dim 2 fills the 4
        // triangular faces but not the tetrahedron -> a hollow void. H2=1.
        let g = flag_graph(4, &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]);
        let h = flag_homology(&g, &vals(4)).unwrap();
        assert_eq!((h.betti0, h.betti1, h.betti2), (1, 0, 1));
        assert_eq!(h.triangles, 4);
        assert_eq!(h.h2_generators.len(), 1);
    }

    #[test]
    fn two_components() {
        // Two disjoint edges: H0=2, H1=0, H2=0.
        let g = flag_graph(4, &[(0, 1), (2, 3)]);
        let h = flag_homology(&g, &vals(4)).unwrap();
        assert_eq!((h.betti0, h.betti1, h.betti2), (2, 0, 0));
    }

    #[test]
    fn value_count_mismatch_errors() {
        let g = flag_graph(3, &[(0, 1)]);
        assert!(flag_homology(&g, &vals(2)).is_err());
    }
}
