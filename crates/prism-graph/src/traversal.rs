//! Bounded BFS traversal over CSR snapshots (architecture doc, *Forward
//! Reachability* and *Reverse Impact Analysis*).
//!
//! [`forward_reachable`] walks the forward CSR and yields strictly-reachable
//! descendants; [`reverse_reachable`] mirrors it over the reverse CSR to yield
//! ancestors. By convention the **root is excluded** from the result unless
//! [`TraversalOptions::include_root`] is set. Both honor `max_depth`, `max_nodes`
//! (setting a `truncated` flag when hit), and an edge-kind allowlist.

use std::collections::VecDeque;

use prism_model::{DependencyKind, GraphNodeId, Result};

use crate::builder::kind_to_u16;
use crate::snapshot::GraphSnapshot;

/// An allowlist of [`DependencyKind`]s, encoded as a bitmask over the stable
/// `u16` kind discriminants. [`EdgeKindMask::ALL`] admits every kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeKindMask(u16);

impl EdgeKindMask {
    /// The empty mask: admits no edge.
    pub const NONE: EdgeKindMask = EdgeKindMask(0);
    /// Admits every dependency kind.
    pub const ALL: EdgeKindMask = EdgeKindMask(u16::MAX);

    /// A mask containing exactly the given kinds.
    pub fn from_kinds(kinds: &[DependencyKind]) -> Self {
        let mut bits = 0u16;
        for &k in kinds {
            bits |= 1 << kind_to_u16(k);
        }
        EdgeKindMask(bits)
    }

    /// Add a kind to the mask.
    pub fn with(mut self, kind: DependencyKind) -> Self {
        self.0 |= 1 << kind_to_u16(kind);
        self
    }

    /// True if `kind` (the raw `u16` discriminant from a CSR edge) is admitted.
    #[inline]
    pub fn admits_raw(self, kind: u16) -> bool {
        // Discriminants are < 16; shifting by >= 16 would be UB-ish, so guard.
        kind < 16 && (self.0 & (1 << kind)) != 0
    }

    /// Raw bits.
    #[inline]
    pub fn bits(self) -> u16 {
        self.0
    }
}

impl Default for EdgeKindMask {
    fn default() -> Self {
        EdgeKindMask::ALL
    }
}

/// Knobs bounding a traversal (the safety-critical limits).
#[derive(Clone, Copy, Debug)]
pub struct TraversalOptions {
    /// Maximum BFS depth from the root; edges out of a node at this depth are
    /// not followed.
    pub max_depth: u32,
    /// Maximum number of result nodes to collect before stopping and flagging
    /// the result `truncated`.
    pub max_nodes: u32,
    /// Edge-kind allowlist.
    pub kinds: EdgeKindMask,
    /// Whether to include the root in the result (default `false`).
    pub include_root: bool,
}

impl Default for TraversalOptions {
    fn default() -> Self {
        Self {
            max_depth: u32::MAX,
            max_nodes: u32::MAX,
            kinds: EdgeKindMask::ALL,
            include_root: false,
        }
    }
}

/// The result of a bounded reachability query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reachable {
    /// Reached node ids in BFS discovery order (root first only when
    /// `include_root` was set).
    pub nodes: Vec<GraphNodeId>,
    /// True if the traversal stopped early because `max_nodes` was hit.
    pub truncated: bool,
}

fn bfs(
    g: &GraphSnapshot,
    root: GraphNodeId,
    opts: TraversalOptions,
    forward: bool,
) -> Result<Reachable> {
    let node_count = g.node_count();
    // Validate the root is in range (also covers the NULL sentinel).
    let root_idx = match root.index() {
        Some(i) if i < node_count => i,
        _ => {
            return Err(prism_model::PrismError::OutOfBounds {
                detail: format!("traversal root {root:?} out of range"),
            })
        }
    };

    let mut seen = vec![false; node_count];
    let mut out: Vec<GraphNodeId> = Vec::new();
    let mut truncated = false;
    let mut q: VecDeque<(GraphNodeId, u32)> = VecDeque::new();

    seen[root_idx] = true;
    // `max_nodes == 0` is a zero budget: nothing fits. We still explore so we can
    // report `truncated` honestly (there was reachable content we could not emit).
    if opts.include_root && opts.max_nodes > 0 {
        out.push(root);
    }
    q.push_back((root, 0));

    'outer: while let Some((u, depth)) = q.pop_front() {
        if depth >= opts.max_depth {
            continue;
        }
        let neighbors = if forward {
            g.out_neighbors(u)?
        } else {
            g.in_neighbors(u)?
        };
        let edge_ids = if forward {
            g.out_edge_ids(u)?
        } else {
            g.in_edge_ids(u)?
        };
        for (k, &v) in neighbors.iter().enumerate() {
            // Edge-kind filter.
            let kind = g.edge_kind(edge_ids[k])?;
            if !opts.kinds.admits_raw(kind) {
                continue;
            }
            let vi = v.get() as usize;
            if !seen[vi] {
                // Check the budget BEFORE adding, so the result never exceeds
                // `max_nodes` (the old post-push check over-shot by one whenever
                // `include_root` had already consumed a slot), and so hitting the
                // cap with reachable nodes left is correctly reported as truncated.
                if out.len() as u64 >= opts.max_nodes as u64 {
                    truncated = true;
                    break 'outer;
                }
                seen[vi] = true;
                out.push(v);
                q.push_back((v, depth + 1));
            }
        }
    }

    Ok(Reachable {
        nodes: out,
        truncated,
    })
}

/// Forward reachability: descendants of `root` reachable along admitted edges,
/// bounded by `opts`. The root is excluded unless `opts.include_root`.
pub fn forward_reachable(
    g: &GraphSnapshot,
    root: GraphNodeId,
    opts: TraversalOptions,
) -> Result<Reachable> {
    bfs(g, root, opts, true)
}

/// Reverse reachability: ancestors of `root` along admitted edges, bounded by
/// `opts`. The root is excluded unless `opts.include_root`.
pub fn reverse_reachable(
    g: &GraphSnapshot,
    root: GraphNodeId,
    opts: TraversalOptions,
) -> Result<Reachable> {
    bfs(g, root, opts, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{BuildEdge, BuildNode, GraphBuilder};
    use prism_model::{RunId, VersionId};

    fn nid(i: u32) -> GraphNodeId {
        GraphNodeId::new(i)
    }

    fn sorted(mut v: Vec<GraphNodeId>) -> Vec<u32> {
        v.sort_by_key(|x| x.get());
        v.into_iter().map(|x| x.get()).collect()
    }

    // 0 -> 1 -> 2 -> 0 (cycle), 1 -> 3, plus a parallel 0 -> 1 edge (multigraph).
    fn graph() -> GraphSnapshot {
        let mut b = GraphBuilder::new();
        for i in 0..4 {
            b.add_node(BuildNode::new(VersionId::new(i), i as u32))
                .unwrap();
        }
        b.add_edge(BuildEdge::new(nid(0), nid(1), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(0), nid(1), 1)).unwrap(); // parallel dup
        b.add_edge(BuildEdge::new(nid(1), nid(2), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(2), nid(0), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(3), 1)).unwrap();
        b.finalize(RunId::new(1)).unwrap()
    }

    #[test]
    fn forward_excludes_root_by_default() {
        let g = graph();
        let r = forward_reachable(&g, nid(0), TraversalOptions::default()).unwrap();
        // From 0 we reach 1, 2, 3 (and 0 via cycle, but it's the excluded root).
        assert_eq!(sorted(r.nodes.clone()), vec![1, 2, 3]);
        assert!(!r.nodes.contains(&nid(0)), "root must be excluded");
        assert!(!r.truncated);
    }

    #[test]
    fn forward_includes_root_when_requested() {
        let g = graph();
        let opts = TraversalOptions {
            include_root: true,
            ..Default::default()
        };
        let r = forward_reachable(&g, nid(0), opts).unwrap();
        assert_eq!(r.nodes[0], nid(0), "root first when included");
        assert_eq!(sorted(r.nodes), vec![0, 1, 2, 3]);
    }

    #[test]
    fn dedup_handles_parallel_edges() {
        let g = graph();
        let r = forward_reachable(&g, nid(0), TraversalOptions::default()).unwrap();
        // Node 1 reached via two parallel edges must appear once.
        let ones = r.nodes.iter().filter(|&&x| x == nid(1)).count();
        assert_eq!(ones, 1);
    }

    #[test]
    fn reverse_yields_ancestors() {
        let g = graph();
        // Ancestors of 3: who can reach 3? 1 (direct), 0 (->1), 2 (->0->1).
        let r = reverse_reachable(&g, nid(3), TraversalOptions::default()).unwrap();
        assert!(!r.nodes.contains(&nid(3)), "root excluded in reverse too");
        assert_eq!(sorted(r.nodes), vec![0, 1, 2]);
    }

    #[test]
    fn max_depth_bounds_traversal() {
        let g = graph();
        let opts = TraversalOptions {
            max_depth: 1,
            ..Default::default()
        };
        // Depth 1 from 0 reaches only direct neighbor 1.
        let r = forward_reachable(&g, nid(0), opts).unwrap();
        assert_eq!(sorted(r.nodes), vec![1]);
    }

    #[test]
    fn max_nodes_sets_truncated() {
        let g = graph();
        let opts = TraversalOptions {
            max_nodes: 1,
            ..Default::default()
        };
        let r = forward_reachable(&g, nid(0), opts).unwrap();
        assert_eq!(r.nodes.len(), 1);
        assert!(r.truncated);
    }

    #[test]
    fn kind_filter_excludes_nonadmitted() {
        let g = graph();
        // Default builder edges are Runtime; an empty mask reaches nothing.
        let opts = TraversalOptions {
            kinds: EdgeKindMask::NONE,
            ..Default::default()
        };
        let r = forward_reachable(&g, nid(0), opts).unwrap();
        assert!(r.nodes.is_empty());

        let opts = TraversalOptions {
            kinds: EdgeKindMask::from_kinds(&[DependencyKind::Runtime]),
            ..Default::default()
        };
        let r = forward_reachable(&g, nid(0), opts).unwrap();
        assert_eq!(sorted(r.nodes), vec![1, 2, 3]);
    }

    #[test]
    fn out_of_range_root_errors() {
        let g = graph();
        assert!(forward_reachable(&g, nid(99), TraversalOptions::default()).is_err());
        assert!(forward_reachable(&g, GraphNodeId::NULL, TraversalOptions::default()).is_err());
    }
}
