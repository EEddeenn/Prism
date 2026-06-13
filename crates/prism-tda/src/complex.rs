//! The filtered 1-complex built from a [`GraphSnapshot`] (architecture doc,
//! *Graph to Complex Construction* and *Filtration functions must be monotone*).
//!
//! v0.1 takes the **underlying undirected graph** as a 1-dimensional cell
//! complex:
//!
//! ```text
//! 0-cells: one per graph node
//! 1-cells: one per unique undirected {u,v} edge (direction & duplicates dropped)
//! ```
//!
//! Each 0-cell carries a caller-supplied scalar `f(v)`; each 1-cell's value is the
//! **lower-star / max rule** `f(edge) = max(f(u), f(v))`, which guarantees
//! `f(face) <= f(coface)` (monotonicity). The cells are then laid out in the
//! **total reduction order** `(birth_value, dim, cell_id)`, with faces strictly
//! before cofaces — see [`Complex::build`].

use prism_model::{GraphEdgeId, GraphNodeId, PrismError, Result};
use prism_graph::GraphSnapshot;

/// The kind of graph object a cell (or generator entry) maps back to.
///
/// The numeric discriminants are stable and persisted in `.ptda`
/// (`object_type`). v0.1 only emits `Node`/`Edge`; the remaining variants are
/// reserved so the on-disk vocabulary matches the architecture doc.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ObjectType {
    /// A graph node; `object_id` is the dense [`GraphNodeId`] (as `u64`).
    Node = 0,
    /// A graph edge; `object_id` is the dense [`GraphEdgeId`] (as `u64`).
    Edge = 1,
    /// A package version (reserved for richer back-mapping).
    Version = 2,
    /// A package family (reserved).
    Package = 3,
    /// A requirement (reserved).
    Requirement = 4,
}

impl ObjectType {
    /// Stable persisted discriminant.
    #[inline]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Inverse of [`ObjectType::to_u8`]; rejects unknown codes.
    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => ObjectType::Node,
            1 => ObjectType::Edge,
            2 => ObjectType::Version,
            3 => ObjectType::Package,
            4 => ObjectType::Requirement,
            other => {
                return Err(PrismError::Corrupt {
                    detail: format!("unknown object_type discriminant {other}"),
                })
            }
        })
    }
}

/// One cell of the filtered complex, in reduction order.
///
/// `id` is the cell's position in [`Complex::cells`] (this is its filtration-local
/// `CellId`). For a 0-cell, `vertices` is the single owning node and `faces` is
/// empty. For a 1-cell, `vertices` are its two endpoint nodes and `faces` are the
/// two boundary 0-cells (as cell ids), both with Z/2 sign `1`.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    /// Cell dimension: `0` (node) or `1` (edge).
    pub dim: u8,
    /// Filtration scalar (birth value) of this cell.
    pub birth_value: f64,
    /// Owning graph nodes (one for a 0-cell, two for a 1-cell).
    pub vertices: Vec<GraphNodeId>,
    /// Codim-1 boundary faces, as cell ids into [`Complex::cells`].
    pub faces: Vec<u64>,
    /// Source graph object this cell maps back to (`Node` or `Edge`).
    pub source_type: ObjectType,
    /// Logical id of the source object (node index, or edge index for a 1-cell).
    pub source_id: u64,
}

/// A filtered undirected 1-complex over a [`GraphSnapshot`].
///
/// Cells are stored in the documented total order `(birth_value, dim, cell_id)`
/// with faces strictly before cofaces; [`Complex::cells`] indices are the
/// filtration-local cell ids consumed by the persistence reduction.
#[derive(Clone, Debug)]
pub struct Complex {
    cells: Vec<Cell>,
    /// Number of underlying graph nodes (== number of 0-cells).
    node_count: usize,
}

/// A deduplicated undirected edge `{u, v}` with `u <= v`, plus a representative
/// directed [`GraphEdgeId`] (the first occurrence) for display back-mapping.
struct UndirectedEdge {
    lo: u32,
    hi: u32,
    repr_edge: GraphEdgeId,
}

impl Complex {
    /// Build the filtered 1-complex from `graph` and a per-node scalar
    /// `node_values` (length must equal `graph.node_count()`).
    ///
    /// 0-cells are created per node; 1-cells per unique undirected `{u, v}` edge
    /// (direction dropped, duplicates and self-loops removed). Edge values use the
    /// lower-star / max rule; the monotonicity invariant `f(face) <= f(coface)` is
    /// asserted and a violation is rejected as [`PrismError::Corrupt`].
    ///
    /// ## Total order / tie-break
    ///
    /// Cells are sorted by the key `(birth_value, dim, source_key)`:
    /// `birth_value` ascending (NaN rejected), then dimension (0 before 1, so a
    /// face never sorts after its coface even at equal value), then a stable
    /// per-dimension key — node index for 0-cells, `(lo, hi)` endpoint pair for
    /// 1-cells — to make the order total and deterministic. The final `cell_id`
    /// is the post-sort position, so `(birth_value, dim, cell_id)` is exactly the
    /// reduction order and faces are strictly before cofaces.
    pub fn build(graph: &GraphSnapshot, node_values: &[f64]) -> Result<Self> {
        let n = graph.node_count();
        if node_values.len() != n {
            return Err(PrismError::Corrupt {
                detail: format!(
                    "filtration value count {} != node_count {n}",
                    node_values.len()
                ),
            });
        }
        for (i, &v) in node_values.iter().enumerate() {
            if v.is_nan() {
                return Err(PrismError::Corrupt {
                    detail: format!("filtration value for node {i} is NaN"),
                });
            }
        }

        // Collect unique undirected edges. We scan the forward CSR only (every
        // directed edge appears there exactly once), normalize each to `lo <= hi`,
        // drop self-loops, and keep the first directed edge id as the display
        // representative.
        let mut undirected: Vec<UndirectedEdge> = Vec::new();
        let mut seen: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
        for u in 0..n as u32 {
            let node = GraphNodeId::new(u);
            let targets = graph.out_neighbors(node)?;
            let edge_ids = graph.out_edge_ids(node)?;
            for (t, e) in targets.iter().zip(edge_ids.iter()) {
                let v = t.get();
                if v == u {
                    continue; // self-loop: contributes no homology in a 1-complex
                }
                let (lo, hi) = if u < v { (u, v) } else { (v, u) };
                if seen.insert((lo, hi)) {
                    undirected.push(UndirectedEdge {
                        lo,
                        hi,
                        repr_edge: *e,
                    });
                }
            }
        }

        // Stage cells with a sort key but without final ids yet.
        enum Staged {
            Vertex { node: u32 },
            Edge { lo: u32, hi: u32, repr: GraphEdgeId },
        }
        struct Entry {
            value: f64,
            dim: u8,
            tie: (u32, u32),
            staged: Staged,
        }
        let mut entries: Vec<Entry> = Vec::with_capacity(n + undirected.len());
        for u in 0..n as u32 {
            entries.push(Entry {
                value: node_values[u as usize],
                dim: 0,
                tie: (u, 0),
                staged: Staged::Vertex { node: u },
            });
        }
        for e in &undirected {
            let fu = node_values[e.lo as usize];
            let fv = node_values[e.hi as usize];
            // lower-star / max rule
            let value = if fu >= fv { fu } else { fv };
            entries.push(Entry {
                value,
                dim: 1,
                tie: (e.lo, e.hi),
                staged: Staged::Edge {
                    lo: e.lo,
                    hi: e.hi,
                    repr: e.repr_edge,
                },
            });
        }

        // Total order: (birth_value, dim, tie). f64 has no Ord; we already
        // rejected NaN, so total_cmp is a safe total order here and agrees with
        // the natural numeric order for finite values.
        entries.sort_by(|a, b| {
            a.value
                .total_cmp(&b.value)
                .then(a.dim.cmp(&b.dim))
                .then(a.tie.cmp(&b.tie))
        });

        // Assign final cell ids and resolve 1-cell faces against the now-fixed
        // 0-cell positions.
        let mut vertex_cell: Vec<u64> = vec![u64::MAX; n];
        for (idx, e) in entries.iter().enumerate() {
            if e.dim == 0 {
                if let Staged::Vertex { node } = e.staged {
                    vertex_cell[node as usize] = idx as u64;
                }
            }
        }

        let mut cells = Vec::with_capacity(entries.len());
        for e in entries {
            match e.staged {
                Staged::Vertex { node } => {
                    cells.push(Cell {
                        dim: 0,
                        birth_value: e.value,
                        vertices: vec![GraphNodeId::new(node)],
                        faces: Vec::new(),
                        source_type: ObjectType::Node,
                        source_id: node as u64,
                    });
                }
                Staged::Edge { lo, hi, repr } => {
                    let f_lo = vertex_cell[lo as usize];
                    let f_hi = vertex_cell[hi as usize];
                    // Defensive: vertices always exist, but never index blindly.
                    if f_lo == u64::MAX || f_hi == u64::MAX {
                        return Err(PrismError::Corrupt {
                            detail: "1-cell references a missing 0-cell".into(),
                        });
                    }
                    cells.push(Cell {
                        dim: 1,
                        birth_value: e.value,
                        vertices: vec![GraphNodeId::new(lo), GraphNodeId::new(hi)],
                        // Faces sorted ascending for a canonical reduced column.
                        faces: if f_lo <= f_hi {
                            vec![f_lo, f_hi]
                        } else {
                            vec![f_hi, f_lo]
                        },
                        source_type: ObjectType::Edge,
                        source_id: repr.get() as u64,
                    });
                }
            }
        }

        let complex = Complex {
            cells,
            node_count: n,
        };
        complex.assert_monotone()?;
        Ok(complex)
    }

    /// Verify `f(face) <= f(coface)` for every boundary incidence and that faces
    /// strictly precede cofaces in the cell order. Rejects violations as
    /// [`PrismError::Corrupt`].
    fn assert_monotone(&self) -> Result<()> {
        for (cid, cell) in self.cells.iter().enumerate() {
            // Structural validation: `compute` indexes `cell.vertices[0]`/`[1]` and
            // a union-find of length `node_count`, so a wrong arity or an
            // out-of-range vertex (which the `.ptda` reader cannot catch — it
            // derives node_count from 0-cells only) would panic. Reject here.
            match cell.dim {
                0 if cell.vertices.len() != 1 => {
                    return Err(PrismError::Corrupt {
                        detail: format!("0-cell {cid} must have exactly 1 vertex"),
                    });
                }
                1 if cell.vertices.len() != 2 => {
                    return Err(PrismError::Corrupt {
                        detail: format!("1-cell {cid} must have exactly 2 vertices"),
                    });
                }
                d if d > 1 => {
                    return Err(PrismError::Corrupt {
                        detail: format!("cell {cid} has unsupported dimension {d}"),
                    });
                }
                _ => {}
            }
            for v in &cell.vertices {
                if v.get() as usize >= self.node_count {
                    return Err(PrismError::Corrupt {
                        detail: format!("cell {cid} vertex {} >= node_count", v.get()),
                    });
                }
            }
            // A non-finite birth value would make filtration ordering undefined and
            // can slip past the face-based monotonicity check on a faceless 0-cell.
            if !cell.birth_value.is_finite() {
                return Err(PrismError::Corrupt {
                    detail: format!("cell {cid} has non-finite birth value"),
                });
            }
            for &face in &cell.faces {
                let fi = face as usize;
                if fi >= self.cells.len() {
                    return Err(PrismError::Corrupt {
                        detail: format!("cell {cid} face {face} out of range"),
                    });
                }
                if face as usize >= cid {
                    return Err(PrismError::Corrupt {
                        detail: format!(
                            "cell order broken: face {face} not strictly before coface {cid}"
                        ),
                    });
                }
                let fv = self.cells[fi].birth_value;
                // `f(face) <= f(coface)` must hold. Use `partial_cmp` so a NaN
                // (incomparable) value is rejected rather than silently passing a
                // negated `<=`.
                match fv.partial_cmp(&cell.birth_value) {
                    Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal) => {}
                    _ => {
                        return Err(PrismError::Corrupt {
                            detail: format!(
                                "monotonicity violated: f(face {face})={fv} > f(coface {cid})={}",
                                cell.birth_value
                            ),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// The cells in reduction order; index == filtration-local cell id.
    #[inline]
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// Number of cells.
    #[inline]
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// Number of underlying graph nodes (== number of 0-cells).
    #[inline]
    pub fn node_count(&self) -> usize {
        self.node_count
    }

    /// Construct directly from an ordered cell list (used by the `.ptda` reader).
    /// Validates the monotonicity / cell-order invariant.
    pub(crate) fn from_cells(cells: Vec<Cell>, node_count: usize) -> Result<Self> {
        let c = Complex { cells, node_count };
        c.assert_monotone()?;
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_graph::{BuildEdge, BuildNode, GraphBuilder, GraphSnapshot};
    use prism_model::{RunId, VersionId};

    fn nid(i: u32) -> GraphNodeId {
        GraphNodeId::new(i)
    }

    // Triangle 0-1-2 plus a duplicate directed edge and a reversed edge, to prove
    // undirected dedup. Edges: 0->1, 1->0 (dup of {0,1}), 1->2, 0->2, plus a
    // self-loop 2->2 that must be dropped.
    fn messy_triangle() -> GraphSnapshot {
        let mut b = GraphBuilder::new();
        for i in 0..3u64 {
            b.add_node(BuildNode::new(VersionId::new(i), i as u32)).unwrap();
        }
        b.add_edge(BuildEdge::new(nid(0), nid(1), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(0), 0)).unwrap(); // reverse dup
        b.add_edge(BuildEdge::new(nid(1), nid(2), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(0), nid(2), 1)).unwrap();
        b.add_edge(BuildEdge::new(nid(2), nid(2), 0)).unwrap(); // self-loop
        b.finalize(RunId::new(1)).unwrap()
    }

    #[test]
    fn undirected_edges_dedup_and_self_loops_drop() {
        let g = messy_triangle();
        // depth 0,1,2.
        let c = Complex::build(&g, &[0.0, 1.0, 2.0]).unwrap();
        let edges = c.cells().iter().filter(|x| x.dim == 1).count();
        let verts = c.cells().iter().filter(|x| x.dim == 0).count();
        assert_eq!(verts, 3);
        // {0,1}, {1,2}, {0,2} — 3 unique undirected edges; dup & self-loop gone.
        assert_eq!(edges, 3);
    }

    #[test]
    fn edge_value_is_max_of_endpoints() {
        let g = messy_triangle();
        let c = Complex::build(&g, &[0.0, 1.0, 2.0]).unwrap();
        // The {1,2} edge has value max(1,2)=2; {0,2} -> 2; {0,1} -> 1.
        let mut edge_vals: Vec<f64> = c
            .cells()
            .iter()
            .filter(|x| x.dim == 1)
            .map(|x| x.birth_value)
            .collect();
        edge_vals.sort_by(f64::total_cmp);
        assert_eq!(edge_vals, vec![1.0, 2.0, 2.0]);
    }

    #[test]
    fn total_order_tie_break_faces_before_cofaces() {
        // All nodes share birth value 5.0: ties are broken by (value, dim, key),
        // so every 0-cell sorts before every 1-cell, and a face is strictly before
        // its coface even at equal value.
        let g = messy_triangle();
        let c = Complex::build(&g, &[5.0, 5.0, 5.0]).unwrap();
        let cells = c.cells();
        // All 0-cells come first.
        let first_edge = cells.iter().position(|x| x.dim == 1).unwrap();
        assert!(cells[..first_edge].iter().all(|x| x.dim == 0));
        assert!(cells[first_edge..].iter().all(|x| x.dim == 1));
        // Every face id is strictly less than its coface id.
        for (cid, cell) in cells.iter().enumerate() {
            for &f in &cell.faces {
                assert!((f as usize) < cid);
            }
        }
    }

    #[test]
    fn wrong_value_count_rejected() {
        let g = messy_triangle();
        assert!(Complex::build(&g, &[0.0, 1.0]).is_err());
    }

    #[test]
    fn reader_path_rejects_nonmonotone_cells() {
        // Hand-build a bad cell list: a 1-cell whose face has a HIGHER value.
        let cells = vec![
            Cell {
                dim: 0,
                birth_value: 9.0, // face value > coface value -> violation
                vertices: vec![nid(0)],
                faces: vec![],
                source_type: ObjectType::Node,
                source_id: 0,
            },
            Cell {
                dim: 0,
                birth_value: 0.0,
                vertices: vec![nid(1)],
                faces: vec![],
                source_type: ObjectType::Node,
                source_id: 1,
            },
            Cell {
                dim: 1,
                birth_value: 0.0,
                vertices: vec![nid(0), nid(1)],
                faces: vec![0, 1],
                source_type: ObjectType::Edge,
                source_id: 0,
            },
        ];
        assert!(Complex::from_cells(cells, 2).is_err());
    }

    #[test]
    fn reader_path_rejects_face_after_coface() {
        // A 1-cell at index 0 referencing a face at index 2 (face not before it).
        let cells = vec![Cell {
            dim: 1,
            birth_value: 0.0,
            vertices: vec![nid(0), nid(1)],
            faces: vec![2],
            source_type: ObjectType::Edge,
            source_id: 0,
        }];
        assert!(Complex::from_cells(cells, 2).is_err());
    }

    #[test]
    fn object_type_roundtrip() {
        for t in [
            ObjectType::Node,
            ObjectType::Edge,
            ObjectType::Version,
            ObjectType::Package,
            ObjectType::Requirement,
        ] {
            assert_eq!(ObjectType::from_u8(t.to_u8()).unwrap(), t);
        }
        assert!(ObjectType::from_u8(200).is_err());
    }
}
