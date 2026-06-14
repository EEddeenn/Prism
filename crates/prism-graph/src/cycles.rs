//! Structural classification of dependency cycles (architecture doc, *Richer
//! Cycle Analysis*).
//!
//! [`crate::scc::tarjan_scc`] finds the strongly-connected components; this
//! module enriches each non-trivial one (a multi-node SCC, or a single-node
//! self-loop) with the signals an engineer needs to triage it: which dependency
//! scopes wire it together, whether it is runtime-relevant, how much of the graph
//! depends on it (external blast radius), whether a first-party root sits inside
//! it, and where the rest of the graph enters it.
//!
//! Ownership beyond "touches a root" (first-party vs third-party by registry
//! namespace) needs purls and is layered on in `prism-api`; here we report only
//! what the CSR structure proves.

use std::collections::VecDeque;

use prism_model::{DependencyKind, GraphNodeId, NodeFlags, Result};

use crate::builder::kind_from_u16;
use crate::scc::tarjan_scc;
use crate::snapshot::GraphSnapshot;

/// A coarse, structurally-derived triage label for a cycle. The interpretation
/// is refined by ownership in `prism-api`; this captures the graph-only verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CycleClass {
    /// Runtime-relevant and depended upon from outside: a coupling risk that
    /// ships, must be reasoned about as a unit, and ripples on any change.
    RuntimeCoupling,
    /// Runtime-relevant but with little or no external blast radius: a contained
    /// cluster, still best upgraded together.
    RuntimeCluster,
    /// Not runtime-relevant (dev/build/test only): tooling cycles that do not
    /// reach production.
    ToolingCycle,
}

/// One classified cycle: an SCC of size ≥ 2, or a single node with a self-loop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CycleInfo {
    /// Member occurrences, in ascending node-id order.
    pub members: Vec<GraphNodeId>,
    /// Directed edges whose endpoints are both members (the cycle's internal
    /// wiring, counting parallel edges separately).
    pub internal_edges: u32,
    /// Distinct dependency kinds on the internal edges.
    pub kinds: Vec<DependencyKind>,
    /// True if any internal edge is a production kind (runtime or workspace).
    pub runtime_relevant: bool,
    /// True if any member is a first-party [`NodeFlags::ROOT`] occurrence.
    pub touches_root: bool,
    /// Distinct occurrences **outside** the cycle that can reach a member (the
    /// external blast radius — who is exposed if the cycle breaks or a member is
    /// yanked).
    pub external_dependents: u32,
    /// Members that have at least one incoming edge from a non-member: where the
    /// rest of the graph enters the cycle.
    pub entry_points: Vec<GraphNodeId>,
    /// The structural triage label.
    pub class: CycleClass,
}

impl CycleInfo {
    /// Number of occurrences in the cycle.
    pub fn size(&self) -> usize {
        self.members.len()
    }
}

/// Whether a node has a self-loop (an out-edge back to itself).
fn has_self_loop(g: &GraphSnapshot, node: GraphNodeId) -> Result<bool> {
    Ok(g.out_neighbors(node)?.contains(&node))
}

/// Classify every non-trivial cycle in `g`, largest first.
pub fn classify_cycles(g: &GraphSnapshot) -> Result<Vec<CycleInfo>> {
    let n = g.node_count();
    let scc = tarjan_scc(g)?;

    // Component membership lookup for "is this neighbor inside my cycle?".
    let comp_of = &scc.component_of;

    let mut out = Vec::new();
    for (cid, comp) in scc.components.iter().enumerate() {
        let is_cycle = comp.len() > 1 || (comp.len() == 1 && has_self_loop(g, comp[0])?);
        if !is_cycle {
            continue;
        }
        let cid = cid as u32;

        // Mark members for O(1) "inside this cycle" tests.
        let mut in_cycle = vec![false; n];
        for &m in comp {
            in_cycle[m.get() as usize] = true;
        }

        // Internal edges, kinds, runtime relevance, entry points.
        let mut internal_edges: u32 = 0;
        let mut kinds: Vec<DependencyKind> = Vec::new();
        let mut runtime_relevant = false;
        let mut touches_root = false;
        let mut entry_points = Vec::new();
        for &m in comp {
            if g.node_flags(m)?.contains(NodeFlags::ROOT) {
                touches_root = true;
            }
            // Internal out-edges of `m`.
            let neigh = g.out_neighbors(m)?;
            let eids = g.out_edge_ids(m)?;
            for (k, &v) in neigh.iter().enumerate() {
                if comp_of[v.get() as usize] != cid {
                    continue; // leaves the cycle
                }
                internal_edges = internal_edges.saturating_add(1);
                let kind = kind_from_u16(g.edge_kind(eids[k])?)?;
                if matches!(kind, DependencyKind::Runtime | DependencyKind::Workspace) {
                    runtime_relevant = true;
                }
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                }
            }
            // Entry point: any in-edge from outside the cycle.
            if g
                .in_neighbors(m)?
                .iter()
                .any(|&p| comp_of[p.get() as usize] != cid)
            {
                entry_points.push(m);
            }
        }

        // External blast radius: reverse BFS from all members, count non-members.
        let external_dependents = external_dependents(g, comp, &in_cycle)?;

        let class = if runtime_relevant && external_dependents > 0 {
            CycleClass::RuntimeCoupling
        } else if runtime_relevant {
            CycleClass::RuntimeCluster
        } else {
            CycleClass::ToolingCycle
        };

        let mut members = comp.clone();
        members.sort_by_key(|x| x.get());
        entry_points.sort_by_key(|x| x.get());

        out.push(CycleInfo {
            members,
            internal_edges,
            kinds,
            runtime_relevant,
            touches_root,
            external_dependents,
            entry_points,
            class,
        });
    }

    // Largest cycles first; ties by smallest member id for determinism.
    out.sort_by(|a, b| {
        b.size()
            .cmp(&a.size())
            .then(a.members[0].get().cmp(&b.members[0].get()))
    });
    Ok(out)
}

/// Count distinct non-member occurrences that can reach any member of the cycle.
fn external_dependents(
    g: &GraphSnapshot,
    members: &[GraphNodeId],
    in_cycle: &[bool],
) -> Result<u32> {
    let n = g.node_count();
    let mut seen = vec![false; n];
    let mut q: VecDeque<GraphNodeId> = VecDeque::new();
    for &m in members {
        seen[m.get() as usize] = true; // members are not "external"
        q.push_back(m);
    }
    let mut count: u32 = 0;
    while let Some(u) = q.pop_front() {
        for &p in g.in_neighbors(u)? {
            let pi = p.get() as usize;
            if !seen[pi] {
                seen[pi] = true;
                if !in_cycle[pi] {
                    count = count.saturating_add(1);
                }
                q.push_back(p);
            }
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{BuildEdge, BuildNode, GraphBuilder};
    use prism_model::{RunId, VersionId};

    fn nid(i: u32) -> GraphNodeId {
        GraphNodeId::new(i)
    }

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
    fn runtime_cycle_with_external_dependents_is_coupling() {
        // root 0 -> 1 -> 2 -> 3 -> 1 (cycle {1,2,3}); 0 and the root depend in.
        let g = build(4, &[0], &[rt(0, 1), rt(1, 2), rt(2, 3), rt(3, 1)]);
        let cycles = classify_cycles(&g).unwrap();
        assert_eq!(cycles.len(), 1);
        let c = &cycles[0];
        assert_eq!(c.members, vec![nid(1), nid(2), nid(3)]);
        assert!(c.runtime_relevant);
        assert_eq!(c.external_dependents, 1, "only root 0 is external");
        assert_eq!(c.entry_points, vec![nid(1)]);
        assert_eq!(c.class, CycleClass::RuntimeCoupling);
        assert!(!c.touches_root);
    }

    #[test]
    fn tooling_cycle_is_classified_separately() {
        // root 0 -dev-> 1 <-> 2 (dev cycle), nothing else depends on it.
        let g = build(
            3,
            &[0],
            &[
                (0, 1, DependencyKind::Dev),
                (1, 2, DependencyKind::Dev),
                (2, 1, DependencyKind::Dev),
            ],
        );
        let cycles = classify_cycles(&g).unwrap();
        assert_eq!(cycles.len(), 1);
        let c = &cycles[0];
        assert_eq!(c.members, vec![nid(1), nid(2)]);
        assert!(!c.runtime_relevant);
        assert_eq!(c.kinds, vec![DependencyKind::Dev]);
        assert_eq!(c.class, CycleClass::ToolingCycle);
    }

    #[test]
    fn self_loop_counts_as_cycle() {
        let g = build(2, &[0], &[rt(0, 1), rt(1, 1)]);
        let cycles = classify_cycles(&g).unwrap();
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].members, vec![nid(1)]);
        assert_eq!(cycles[0].internal_edges, 1);
    }

    #[test]
    fn dag_has_no_cycles() {
        let g = build(3, &[0], &[rt(0, 1), rt(1, 2), rt(0, 2)]);
        assert!(classify_cycles(&g).unwrap().is_empty());
    }

    #[test]
    fn first_party_cycle_is_flagged() {
        // roots 0 and 1 mutually depend (workspace cycle).
        let g = build(
            2,
            &[0, 1],
            &[
                (0, 1, DependencyKind::Workspace),
                (1, 0, DependencyKind::Workspace),
            ],
        );
        let cycles = classify_cycles(&g).unwrap();
        assert_eq!(cycles.len(), 1);
        assert!(cycles[0].touches_root);
        assert!(cycles[0].runtime_relevant);
    }
}
