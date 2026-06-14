//! Structural analysis over CSR snapshots: dominators / chokepoints, per-scope
//! reachability footprints, and "why is this here?" path explanation
//! (architecture doc, *Structural Control* and *Scope-Aware Analysis*).
//!
//! These are pure graph algorithms over a validated [`GraphSnapshot`]; they
//! perform no I/O and never panic on a well-formed snapshot. Results are keyed by
//! snapshot-local [`GraphNodeId`]s — the human-facing purl/name mapping lives one
//! layer up in `prism-api`.
//!
//! The three primitives, and what they unlock:
//!
//! - [`dominators`] builds the **dominator tree** rooted at a virtual super-source
//!   joined to every [`NodeFlags::ROOT`] node. A node's *dominated count* is how
//!   many occurrences are reachable from the roots **only** through it — the
//!   structural notion of a chokepoint, distinct from raw reverse reachability.
//! - [`scope_footprint`] computes, per [`DependencyKind`], the set of occurrences
//!   pulled into the graph **because of** dependencies of that kind, so runtime /
//!   dev / build / test / … risk can be summarized separately.
//! - [`explain`] answers "why is this package here?": its direct introducers, a
//!   few shortest dependency paths from the roots, the nearest first-party owner,
//!   and whether it is reachable through production scopes.

use std::collections::VecDeque;

use prism_model::{DependencyKind, GraphNodeId, NodeFlags, PrismError, Result};

use crate::builder::kind_from_u16;
use crate::snapshot::GraphSnapshot;
use crate::traversal::EdgeKindMask;

/// Dependency kinds treated as "production": an occurrence reachable from a root
/// along only these edges ships (or is required to build the shipped artifact).
/// Workspace edges are first-party glue and are always production-relevant.
pub const PRODUCTION_KINDS: &[DependencyKind] =
    &[DependencyKind::Runtime, DependencyKind::Workspace];

/// The first-party root occurrences of a snapshot: nodes flagged
/// [`NodeFlags::ROOT`] (workspace members / the manifest under analysis).
pub fn roots(g: &GraphSnapshot) -> Result<Vec<GraphNodeId>> {
    let mut out = Vec::new();
    for i in 0..g.node_count() {
        let id = GraphNodeId::new(i as u32);
        if g.node_flags(id)?.contains(NodeFlags::ROOT) {
            out.push(id);
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------------- //
// Dominator tree / chokepoints
// ------------------------------------------------------------------------- //

/// The dominator tree of a snapshot, rooted at a virtual super-source connected
/// to every [`NodeFlags::ROOT`] node.
///
/// Computed with the Cooper–Harvey–Kennedy iterative algorithm ("A Simple, Fast
/// Dominance Algorithm"). Nodes not reachable from any root are absent from the
/// tree (`idom_of` returns `None`, `dominated_count` returns `0`).
#[derive(Clone, Debug)]
pub struct DomTree {
    /// `node_count` of the underlying snapshot (the virtual root has index
    /// `node_count`).
    node_count: usize,
    /// Immediate dominator of each node, as a node index, or `None` if the node
    /// is unreachable from the roots. The virtual root dominates itself and is
    /// encoded as `node_count`.
    idom: Vec<Option<u32>>,
    /// Size of the dominator-tree subtree rooted at each node (1 = dominates only
    /// itself). `0` for unreachable nodes.
    subtree: Vec<u32>,
}

/// One structurally-controlling occurrence: a non-root node that is the sole
/// gateway from the roots to `dominated` other occurrences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chokepoint {
    /// The gateway occurrence.
    pub node: GraphNodeId,
    /// How many *other* occurrences are reachable from the roots only through
    /// `node` (its dominator-subtree size minus itself).
    pub dominated: u32,
}

impl DomTree {
    /// The immediate dominator of `node`, or `None` if `node` is the virtual root
    /// or is unreachable from any root. A root occurrence's idom is the virtual
    /// root, reported here as `None` (it has no real-node dominator).
    pub fn idom_of(&self, node: GraphNodeId) -> Option<GraphNodeId> {
        let i = node.index()?;
        match self.idom.get(i).copied().flatten() {
            Some(p) if (p as usize) < self.node_count => Some(GraphNodeId::new(p)),
            _ => None, // virtual root or unreachable
        }
    }

    /// How many occurrences (excluding itself) are reachable from the roots only
    /// through `node`. `0` for leaves and unreachable nodes.
    pub fn dominated_count(&self, node: GraphNodeId) -> u32 {
        node.index()
            .and_then(|i| self.subtree.get(i).copied())
            .map(|s| s.saturating_sub(1))
            .unwrap_or(0)
    }

    /// Chokepoints ranked by descending dominated count, excluding root nodes and
    /// nodes that dominate fewer than `min_dominated` others. Ties break by node
    /// id for determinism.
    pub fn chokepoints(&self, g: &GraphSnapshot, min_dominated: u32) -> Result<Vec<Chokepoint>> {
        let mut out = Vec::new();
        for i in 0..self.node_count {
            let id = GraphNodeId::new(i as u32);
            if g.node_flags(id)?.contains(NodeFlags::ROOT) {
                continue;
            }
            let dominated = self.subtree[i].saturating_sub(1);
            if dominated >= min_dominated && dominated > 0 {
                out.push(Chokepoint {
                    node: id,
                    dominated,
                });
            }
        }
        out.sort_by(|a, b| {
            b.dominated
                .cmp(&a.dominated)
                .then(a.node.get().cmp(&b.node.get()))
        });
        Ok(out)
    }
}

/// Build the dominator tree of `g` rooted at a virtual super-source joined to
/// every [`NodeFlags::ROOT`] node.
///
/// If a snapshot has no root nodes (e.g. a bare lockfile slice), every node is
/// unreachable from the virtual root and the tree is empty.
pub fn dominators(g: &GraphSnapshot) -> Result<DomTree> {
    let n = g.node_count();
    let vroot = n; // virtual super-source index
    let total = n + 1;
    let root_list = roots(g)?;

    // --- Postorder DFS from the virtual root over forward edges. ---
    // `succ(vroot) = roots`, `succ(real) = out_neighbors` (dedup via `visited`).
    let mut visited = vec![false; total];
    let mut post_num = vec![u32::MAX; total]; // postorder index; MAX = unreachable
    let mut post_seq: Vec<usize> = Vec::with_capacity(total);

    struct Frame {
        node: usize,
        child: usize,
    }
    visited[vroot] = true;
    let mut stack = vec![Frame {
        node: vroot,
        child: 0,
    }];
    while let Some(top) = stack.last() {
        let node = top.node;
        let child = top.child;
        // Successors of the current node.
        let succ: &[GraphNodeId] = if node == vroot {
            &root_list
        } else {
            g.out_neighbors(GraphNodeId::new(node as u32))?
        };
        if child < succ.len() {
            let w = succ[child].get() as usize;
            stack.last_mut().expect("non-empty").child += 1;
            if !visited[w] {
                visited[w] = true;
                stack.push(Frame { node: w, child: 0 });
            }
        } else {
            post_num[node] = post_seq.len() as u32;
            post_seq.push(node);
            stack.pop();
        }
    }

    // --- Iterative dominance (Cooper–Harvey–Kennedy). ---
    let mut idom: Vec<Option<u32>> = vec![None; total];
    idom[vroot] = Some(vroot as u32); // the virtual root dominates itself

    // Reverse postorder, excluding the virtual root.
    let rpo: Vec<usize> = post_seq.iter().rev().copied().collect();

    // Predecessors of a real node `b`: its (deduped) in-neighbors that are
    // reachable, plus the virtual root when `b` is a root occurrence.
    let is_root: Vec<bool> = {
        let mut v = vec![false; n];
        for r in &root_list {
            v[r.get() as usize] = true;
        }
        v
    };

    let mut changed = true;
    while changed {
        changed = false;
        for &b in &rpo {
            if b == vroot {
                continue;
            }
            // `b` is always a real node here (vroot has no in-edges).
            let mut new_idom: Option<usize> = None;
            // Virtual-root predecessor for root occurrences.
            if is_root[b] {
                new_idom = Some(vroot);
            }
            let preds = g.in_neighbors(GraphNodeId::new(b as u32))?;
            let mut last_pred = u32::MAX;
            for &p in preds {
                let pi = p.get();
                if pi == last_pred {
                    continue; // dedup parallel reverse edges (sorted by source)
                }
                last_pred = pi;
                let pu = pi as usize;
                if post_num[pu] == u32::MAX || idom[pu].is_none() {
                    continue; // unreachable, or not yet processed
                }
                new_idom = Some(match new_idom {
                    None => pu,
                    Some(cur) => intersect(pu, cur, &idom, &post_num),
                });
            }
            if let Some(ni) = new_idom {
                if idom[b] != Some(ni as u32) {
                    idom[b] = Some(ni as u32);
                    changed = true;
                }
            }
        }
    }

    // --- Subtree sizes: process in increasing postorder (children first). ---
    let mut subtree = vec![0u32; total];
    for &node in &post_seq {
        subtree[node] = 1; // reachable nodes start at 1
    }
    for &node in &post_seq {
        if node == vroot {
            continue;
        }
        if let Some(p) = idom[node] {
            let pu = p as usize;
            subtree[pu] = subtree[pu].saturating_add(subtree[node]);
        }
    }

    // Drop the virtual-root slot from the public arrays.
    subtree.truncate(n);
    idom.truncate(n);

    Ok(DomTree {
        node_count: n,
        idom,
        subtree,
    })
}

/// Walk two dominator-tree fingers toward the root until they meet, using
/// postorder numbers (the node with the smaller number is deeper and climbs).
fn intersect(mut a: usize, mut b: usize, idom: &[Option<u32>], post: &[u32]) -> usize {
    while a != b {
        while post[a] < post[b] {
            a = idom[a].expect("processed node has idom") as usize;
        }
        while post[b] < post[a] {
            b = idom[b].expect("processed node has idom") as usize;
        }
    }
    a
}

// ------------------------------------------------------------------------- //
// Scope footprints
// ------------------------------------------------------------------------- //

/// The dependency-graph footprint attributable to one [`DependencyKind`].
///
/// Footprints are **root-anchored**, mirroring how package managers resolve: a
/// scope's footprint is everything reachable from the roots when edges of that
/// kind are allowed *in addition to* the always-followed production edges
/// ([`PRODUCTION_KINDS`]). The `exclusive` set is what the scope adds beyond the
/// production base — the packages present *solely* to serve this scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeFootprint {
    /// The dependency kind this footprint summarizes.
    pub kind: DependencyKind,
    /// Number of edges of this kind anywhere in the graph.
    pub direct_edges: u32,
    /// Distinct occurrences that are the direct target of an edge of this kind.
    pub direct_targets: Vec<GraphNodeId>,
    /// Occurrences reachable from the roots under the production base plus this
    /// kind (roots themselves excluded). For a production kind this equals the
    /// production footprint.
    pub footprint: Vec<GraphNodeId>,
    /// `footprint` minus the production-only reachable set: occurrences present
    /// solely because of this scope. Empty for production kinds.
    pub exclusive: Vec<GraphNodeId>,
    /// Greatest BFS distance from the roots within the footprint.
    pub max_depth: u32,
}

impl ScopeFootprint {
    /// Total occurrences in this scope's footprint.
    pub fn footprint_count(&self) -> usize {
        self.footprint.len()
    }

    /// Occurrences attributable solely to this scope (beyond production).
    pub fn exclusive_count(&self) -> usize {
        self.exclusive.len()
    }
}

/// Multi-source forward BFS from `sources` over edges admitted by `mask`,
/// excluding the sources themselves from the result. Returns a membership
/// bitset, the reached nodes in BFS order, and the greatest depth reached.
fn reachable_set(
    g: &GraphSnapshot,
    sources: &[GraphNodeId],
    mask: EdgeKindMask,
) -> Result<(Vec<bool>, Vec<GraphNodeId>, u32)> {
    let n = g.node_count();
    let mut seen = vec![false; n];
    let mut member = vec![false; n];
    let mut reached = Vec::new();
    let mut q: VecDeque<(GraphNodeId, u32)> = VecDeque::new();
    let mut max_depth = 0u32;
    for &s in sources {
        let si = s.get() as usize;
        if !seen[si] {
            seen[si] = true; // visited, but a source is not part of the footprint
            q.push_back((s, 0));
        }
    }
    while let Some((u, depth)) = q.pop_front() {
        let neigh = g.out_neighbors(u)?;
        let eids = g.out_edge_ids(u)?;
        for (k, &v) in neigh.iter().enumerate() {
            if !mask.admits_raw(g.edge_kind(eids[k])?) {
                continue;
            }
            let vi = v.get() as usize;
            if !seen[vi] {
                seen[vi] = true;
                member[vi] = true;
                reached.push(v);
                let d = depth + 1;
                if d > max_depth {
                    max_depth = d;
                }
                q.push_back((v, d));
            }
        }
    }
    Ok((member, reached, max_depth))
}

/// Compute the root-anchored footprint of one dependency kind: everything the
/// roots reach when this kind's edges are allowed alongside production edges,
/// plus the subset present solely because of this scope.
pub fn scope_footprint(g: &GraphSnapshot, kind: DependencyKind) -> Result<ScopeFootprint> {
    let n = g.node_count();
    let want = crate::builder::kind_to_u16(kind);
    let root_list = roots(g)?;

    // Direct edges/targets of this kind (anywhere in the graph).
    let mut is_direct = vec![false; n];
    let mut direct_targets = Vec::new();
    let mut direct_edges: u32 = 0;
    for u in 0..n {
        let uid = GraphNodeId::new(u as u32);
        let neigh = g.out_neighbors(uid)?;
        let eids = g.out_edge_ids(uid)?;
        for (k, &v) in neigh.iter().enumerate() {
            if g.edge_kind(eids[k])? == want {
                direct_edges = direct_edges.saturating_add(1);
                let vi = v.get() as usize;
                if !is_direct[vi] {
                    is_direct[vi] = true;
                    direct_targets.push(v);
                }
            }
        }
    }

    // Production base, and this scope's footprint (production + this kind).
    let prod_mask = EdgeKindMask::from_kinds(PRODUCTION_KINDS);
    let (prod_member, _, _) = reachable_set(g, &root_list, prod_mask)?;
    let scope_mask = prod_mask.with(kind);
    let (_, footprint, max_depth) = reachable_set(g, &root_list, scope_mask)?;

    let exclusive: Vec<GraphNodeId> = footprint
        .iter()
        .copied()
        .filter(|v| !prod_member[v.get() as usize])
        .collect();

    Ok(ScopeFootprint {
        kind,
        direct_edges,
        direct_targets,
        footprint,
        exclusive,
        max_depth,
    })
}

/// Footprints for every dependency kind that appears as at least one edge, in a
/// stable kind order.
pub fn scope_summary(g: &GraphSnapshot) -> Result<Vec<ScopeFootprint>> {
    const ALL: &[DependencyKind] = &[
        DependencyKind::Runtime,
        DependencyKind::Dev,
        DependencyKind::Build,
        DependencyKind::Peer,
        DependencyKind::Optional,
        DependencyKind::Test,
        DependencyKind::Plugin,
        DependencyKind::Workspace,
    ];
    let mut out = Vec::new();
    for &k in ALL {
        let fp = scope_footprint(g, k)?;
        if fp.direct_edges > 0 {
            out.push(fp);
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------------- //
// "Why is this here?" explanation
// ------------------------------------------------------------------------- //

/// A direct introducer of a target occurrence: a parent plus the dependency
/// kinds by which it pulls the target in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Introducer {
    /// The parent occurrence depending directly on the target.
    pub node: GraphNodeId,
    /// The distinct dependency kinds on edges from `node` to the target.
    pub kinds: Vec<DependencyKind>,
}

/// The structural explanation of why a target occurrence is present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Explanation {
    /// The occurrence being explained.
    pub target: GraphNodeId,
    /// Direct parents that depend on the target, with the kinds involved.
    pub introducers: Vec<Introducer>,
    /// A few shortest dependency paths from a root to the target, each `root …
    /// target` inclusive. Empty if the target is unreachable from any root.
    pub paths: Vec<Vec<GraphNodeId>>,
    /// The nearest [`NodeFlags::ROOT`] ancestor (the first-party owner closest to
    /// the target), or `None` if no root reaches it. Equals the target itself
    /// when the target is a root.
    pub nearest_owner: Option<GraphNodeId>,
    /// Distance in edges from `nearest_owner` to the target (`0` when the target
    /// is itself a root).
    pub owner_distance: u32,
    /// True if the target is reachable from a root through production scopes only
    /// ([`PRODUCTION_KINDS`]).
    pub production_reachable: bool,
    /// True if the target is reachable from a root at all but *not* through
    /// production scopes (it exists solely to serve dev/test/build tooling).
    pub non_production_only: bool,
}

/// Explain why `target` is in the graph: its introducers, up to `max_paths`
/// shortest paths from the roots, its nearest first-party owner, and its
/// production reachability.
pub fn explain(g: &GraphSnapshot, target: GraphNodeId, max_paths: usize) -> Result<Explanation> {
    let n = g.node_count();
    let ti = match target.index() {
        Some(i) if i < n => i,
        _ => {
            return Err(PrismError::OutOfBounds {
                detail: format!("explain target {target:?} out of range (node_count {n})"),
            })
        }
    };

    // --- Introducers: group reverse edges by source, collect distinct kinds. ---
    let preds = g.in_neighbors(target)?;
    let pred_eids = g.in_edge_ids(target)?;
    let mut introducers: Vec<Introducer> = Vec::new();
    for (k, &p) in preds.iter().enumerate() {
        let kind = kind_from_u16(g.edge_kind(pred_eids[k])?)?;
        match introducers.iter_mut().find(|i| i.node == p) {
            Some(intro) => {
                if !intro.kinds.contains(&kind) {
                    intro.kinds.push(kind);
                }
            }
            None => introducers.push(Introducer {
                node: p,
                kinds: vec![kind],
            }),
        }
    }

    // --- Shortest paths: multi-source BFS from roots over forward edges. ---
    let root_list = roots(g)?;
    let mut dist = vec![u32::MAX; n];
    let mut parent = vec![GraphNodeId::NULL; n];
    let mut q: VecDeque<GraphNodeId> = VecDeque::new();
    for &r in &root_list {
        let ri = r.get() as usize;
        if dist[ri] == u32::MAX {
            dist[ri] = 0;
            q.push_back(r);
        }
    }
    while let Some(u) = q.pop_front() {
        let du = dist[u.get() as usize];
        for &v in g.out_neighbors(u)? {
            let vi = v.get() as usize;
            if dist[vi] == u32::MAX {
                dist[vi] = du + 1;
                parent[vi] = u;
                q.push_back(v);
            }
        }
    }

    let mut paths: Vec<Vec<GraphNodeId>> = Vec::new();
    if dist[ti] != u32::MAX {
        if dist[ti] == 0 {
            paths.push(vec![target]); // the target is itself a root
        } else {
            // Canonical shortest path via BFS parents.
            paths.push(reconstruct(target, &parent));
            // Additional shortest paths through other on-shortest-path introducers.
            for intro in &introducers {
                if paths.len() >= max_paths {
                    break;
                }
                let pi = intro.node.get() as usize;
                if intro.node == parent[ti] {
                    continue; // already the canonical path
                }
                if dist[pi] == dist[ti].saturating_sub(1) {
                    let mut path = reconstruct(intro.node, &parent);
                    path.push(target);
                    paths.push(path);
                }
            }
        }
        if paths.len() > max_paths {
            paths.truncate(max_paths);
        }
    }

    // --- Nearest first-party owner: reverse BFS to the closest ROOT node. ---
    let (nearest_owner, owner_distance) = nearest_root_owner(g, target)?;

    // --- Production reachability: BFS from roots under the production mask. ---
    let prod_mask = EdgeKindMask::from_kinds(PRODUCTION_KINDS);
    let production_reachable = reachable_from_roots(g, &root_list, target, prod_mask)?;
    let any_reachable = dist[ti] != u32::MAX;
    let non_production_only = any_reachable && !production_reachable;

    Ok(Explanation {
        target,
        introducers,
        paths,
        nearest_owner,
        owner_distance,
        production_reachable,
        non_production_only,
    })
}

/// Reconstruct the root→node path from BFS parent pointers (inclusive of both).
fn reconstruct(node: GraphNodeId, parent: &[GraphNodeId]) -> Vec<GraphNodeId> {
    let mut path = vec![node];
    let mut cur = node;
    while let Some(i) = cur.index() {
        let p = parent[i];
        if p.is_null() {
            break;
        }
        path.push(p);
        cur = p;
    }
    path.reverse();
    path
}

/// Find the nearest [`NodeFlags::ROOT`] ancestor of `target` by reverse BFS, and
/// its edge distance. Returns `(Some(target), 0)` when the target is itself a
/// root, `(None, 0)` when no root can reach it.
fn nearest_root_owner(
    g: &GraphSnapshot,
    target: GraphNodeId,
) -> Result<(Option<GraphNodeId>, u32)> {
    if g.node_flags(target)?.contains(NodeFlags::ROOT) {
        return Ok((Some(target), 0));
    }
    let n = g.node_count();
    let mut seen = vec![false; n];
    let mut q: VecDeque<(GraphNodeId, u32)> = VecDeque::new();
    seen[target.get() as usize] = true;
    q.push_back((target, 0));
    while let Some((u, d)) = q.pop_front() {
        for &p in g.in_neighbors(u)? {
            let pi = p.get() as usize;
            if !seen[pi] {
                seen[pi] = true;
                if g.node_flags(p)?.contains(NodeFlags::ROOT) {
                    return Ok((Some(p), d + 1));
                }
                q.push_back((p, d + 1));
            }
        }
    }
    Ok((None, 0))
}

/// Whether `target` is reachable from any node in `sources` following only edges
/// admitted by `mask`.
fn reachable_from_roots(
    g: &GraphSnapshot,
    sources: &[GraphNodeId],
    target: GraphNodeId,
    mask: EdgeKindMask,
) -> Result<bool> {
    let n = g.node_count();
    let mut seen = vec![false; n];
    let mut q: VecDeque<GraphNodeId> = VecDeque::new();
    for &s in sources {
        let si = s.get() as usize;
        if !seen[si] {
            seen[si] = true;
            if s == target {
                return Ok(true);
            }
            q.push_back(s);
        }
    }
    while let Some(u) = q.pop_front() {
        let neigh = g.out_neighbors(u)?;
        let eids = g.out_edge_ids(u)?;
        for (k, &v) in neigh.iter().enumerate() {
            if !mask.admits_raw(g.edge_kind(eids[k])?) {
                continue;
            }
            let vi = v.get() as usize;
            if !seen[vi] {
                seen[vi] = true;
                if v == target {
                    return Ok(true);
                }
                q.push_back(v);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{BuildEdge, BuildNode, GraphBuilder};
    use prism_model::{NodeFlags, RunId, VersionId};

    fn nid(i: u32) -> GraphNodeId {
        GraphNodeId::new(i)
    }

    /// Build a snapshot from `(from,to,kind)` edges; `roots` are flagged ROOT.
    fn build(
        n: u32,
        roots: &[u32],
        edges: &[(u32, u32, DependencyKind)],
    ) -> GraphSnapshot {
        let mut b = GraphBuilder::new();
        for i in 0..n {
            let mut node = BuildNode::new(VersionId::new(i as u64), 0);
            if roots.contains(&i) {
                node.flags = NodeFlags::ROOT;
            }
            b.add_node(node).unwrap();
        }
        for (ord, &(f, t, k)) in edges.iter().enumerate() {
            let mut e = BuildEdge::new(nid(f), nid(t), ord as u32);
            e.kind = k;
            b.add_edge(e).unwrap();
        }
        b.finalize(RunId::new(1)).unwrap()
    }

    fn rt(f: u32, t: u32) -> (u32, u32, DependencyKind) {
        (f, t, DependencyKind::Runtime)
    }

    #[test]
    fn chokepoint_dominates_subtree() {
        // root 0 -> 1 -> {2,3}; 1 is the sole gateway to 2 and 3.
        let g = build(4, &[0], &[rt(0, 1), rt(1, 2), rt(1, 3)]);
        let dt = dominators(&g).unwrap();
        assert_eq!(dt.dominated_count(nid(1)), 2, "1 dominates 2 and 3");
        assert_eq!(dt.dominated_count(nid(2)), 0);
        let cps = dt.chokepoints(&g, 1).unwrap();
        assert_eq!(cps[0].node, nid(1));
        assert_eq!(cps[0].dominated, 2);
        // roots are excluded from the ranking.
        assert!(cps.iter().all(|c| c.node != nid(0)));
    }

    #[test]
    fn shared_target_is_not_dominated_by_one_parent() {
        // root 0 -> {1,2}; both 1 and 2 -> 3. Neither 1 nor 2 dominates 3.
        let g = build(4, &[0], &[rt(0, 1), rt(0, 2), rt(1, 3), rt(2, 3)]);
        let dt = dominators(&g).unwrap();
        assert_eq!(dt.dominated_count(nid(1)), 0);
        assert_eq!(dt.dominated_count(nid(2)), 0);
        // Every path to 3 runs through root 0, so 3's immediate dominator is 0.
        assert_eq!(dt.idom_of(nid(3)), Some(nid(0)));
    }

    #[test]
    fn dominators_handle_cycles() {
        // root 0 -> 1 -> 2 -> 1 (cycle) -> 3 off 2. 1 dominates 2 and 3.
        let g = build(4, &[0], &[rt(0, 1), rt(1, 2), rt(2, 1), rt(2, 3)]);
        let dt = dominators(&g).unwrap();
        assert_eq!(dt.dominated_count(nid(1)), 2);
    }

    #[test]
    fn scope_footprint_separates_dev_from_runtime() {
        // root 0 -runtime-> 1 -> 2 ; root 0 -dev-> 3 -> 4. Shared: none.
        let g = build(
            5,
            &[0],
            &[
                rt(0, 1),
                rt(1, 2),
                (0, 3, DependencyKind::Dev),
                rt(3, 4),
            ],
        );
        let rt_fp = scope_footprint(&g, DependencyKind::Runtime).unwrap();
        // Runtime/production footprint from root 0: {1,2}; nothing exclusive.
        assert_eq!(rt_fp.footprint_count(), 2);
        assert_eq!(rt_fp.exclusive_count(), 0);
        let dev_fp = scope_footprint(&g, DependencyKind::Dev).unwrap();
        // Dev footprint adds {3,4} on top of production: total 4, exclusive 2.
        assert_eq!(dev_fp.footprint_count(), 4);
        assert_eq!(dev_fp.exclusive_count(), 2);
        let dev_excl: std::collections::HashSet<u32> =
            dev_fp.exclusive.iter().map(|v| v.get()).collect();
        assert_eq!(dev_excl, std::collections::HashSet::from([3, 4]));
        let summary = scope_summary(&g).unwrap();
        let kinds: Vec<_> = summary.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, vec![DependencyKind::Runtime, DependencyKind::Dev]);
    }

    #[test]
    fn explain_reports_introducers_and_path() {
        // root 0 -> 1 -> 3 ; root 0 -> 2 -> 3. 3 has two introducers.
        let g = build(4, &[0], &[rt(0, 1), rt(0, 2), rt(1, 3), rt(2, 3)]);
        let ex = explain(&g, nid(3), 4).unwrap();
        let intro_nodes: std::collections::HashSet<u32> =
            ex.introducers.iter().map(|i| i.node.get()).collect();
        assert_eq!(intro_nodes, std::collections::HashSet::from([1, 2]));
        // Two distinct shortest paths root..3.
        assert_eq!(ex.paths.len(), 2);
        for p in &ex.paths {
            assert_eq!(*p.first().unwrap(), nid(0));
            assert_eq!(*p.last().unwrap(), nid(3));
        }
        assert_eq!(ex.nearest_owner, Some(nid(0)));
        assert_eq!(ex.owner_distance, 2);
        assert!(ex.production_reachable);
        assert!(!ex.non_production_only);
    }

    #[test]
    fn explain_flags_dev_only_dependency() {
        // root 0 -dev-> 1 -> 2. 2 is reachable only through a dev edge.
        let g = build(3, &[0], &[(0, 1, DependencyKind::Dev), rt(1, 2)]);
        let ex = explain(&g, nid(2), 4).unwrap();
        assert!(!ex.production_reachable);
        assert!(ex.non_production_only);
        assert_eq!(ex.nearest_owner, Some(nid(0)));
    }

    #[test]
    fn explain_root_is_its_own_owner() {
        let g = build(2, &[0], &[rt(0, 1)]);
        let ex = explain(&g, nid(0), 4).unwrap();
        assert_eq!(ex.nearest_owner, Some(nid(0)));
        assert_eq!(ex.owner_distance, 0);
        assert!(ex.production_reachable);
    }

    #[test]
    fn explain_out_of_range_errors() {
        let g = build(2, &[0], &[rt(0, 1)]);
        assert!(explain(&g, nid(99), 4).is_err());
    }
}
