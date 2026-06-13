//! Strongly-connected components via Tarjan's algorithm (architecture doc,
//! *Strongly Connected Components*).
//!
//! Tarjan emits SCCs in **reverse topological order** of the condensation as a
//! byproduct, so a separate Kahn pass is unnecessary when only the order is
//! needed. [`tarjan_scc`] returns the components in that emission order;
//! [`condensation_topo_order`] reverses it to give condensation topological
//! order. The implementation is iterative to avoid deep recursion on large
//! dependency graphs. Cycles are preserved: every member of a cycle lands in one
//! component.

use prism_model::{GraphNodeId, Result};

use crate::snapshot::GraphSnapshot;

/// The strongly-connected components of a snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SccResult {
    /// Components in Tarjan emission order (reverse topological order of the
    /// condensation). Each inner `Vec` holds the node ids of one component.
    pub components: Vec<Vec<GraphNodeId>>,
    /// `component_of[node]` = index into `components` for that node.
    pub component_of: Vec<u32>,
}

impl SccResult {
    /// Number of components (= number of nodes in the condensation DAG).
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// True if the graph has no nodes.
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    /// The component index containing `node`.
    pub fn component_of(&self, node: GraphNodeId) -> Option<u32> {
        node.index()
            .and_then(|i| self.component_of.get(i).copied())
    }

    /// Components in **condensation topological order** (every edge `u -> v` with
    /// `u`, `v` in distinct components has `comp(u)` before `comp(v)`). This is
    /// the reverse of the Tarjan emission order in `components`.
    pub fn topo_order(&self) -> Vec<&Vec<GraphNodeId>> {
        self.components.iter().rev().collect()
    }

    /// True if any component contains more than one node, i.e. the graph has a
    /// cycle spanning multiple occurrences. (A single-node self-loop is also a
    /// cycle but is detected separately.)
    pub fn has_multinode_cycle(&self) -> bool {
        self.components.iter().any(|c| c.len() > 1)
    }
}

/// Compute strongly-connected components with iterative Tarjan.
///
/// Returns components in reverse topological order of the condensation (the
/// natural Tarjan emission order). For a DAG every component is a singleton and
/// the emission order is a reverse topological order of the nodes themselves.
pub fn tarjan_scc(g: &GraphSnapshot) -> Result<SccResult> {
    let n = g.node_count();
    const UNVISITED: u32 = u32::MAX;

    let mut index = vec![UNVISITED; n]; // discovery index
    let mut lowlink = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut comp_of = vec![u32::MAX; n];

    let mut tarjan_stack: Vec<usize> = Vec::new();
    let mut components: Vec<Vec<GraphNodeId>> = Vec::new();
    let mut next_index: u32 = 0;

    // Iterative DFS frame: the node plus its next-neighbor cursor.
    struct Frame {
        node: usize,
        child: usize,
    }

    for start in 0..n {
        if index[start] != UNVISITED {
            continue;
        }
        let mut call_stack: Vec<Frame> = vec![Frame {
            node: start,
            child: 0,
        }];

        // On first entry to a node we assign its index/lowlink and push it.
        index[start] = next_index;
        lowlink[start] = next_index;
        next_index += 1;
        tarjan_stack.push(start);
        on_stack[start] = true;

        while let Some(frame) = call_stack.last_mut() {
            let u = frame.node;
            let neighbors = g.out_neighbors(GraphNodeId::new(u as u32))?;

            if frame.child < neighbors.len() {
                let w = neighbors[frame.child].get() as usize;
                frame.child += 1;
                if index[w] == UNVISITED {
                    // Recurse into w.
                    index[w] = next_index;
                    lowlink[w] = next_index;
                    next_index += 1;
                    tarjan_stack.push(w);
                    on_stack[w] = true;
                    call_stack.push(Frame { node: w, child: 0 });
                } else if on_stack[w] {
                    // Back-edge to a node still on the Tarjan stack.
                    if index[w] < lowlink[u] {
                        lowlink[u] = index[w];
                    }
                }
                // else: cross-edge into a finished component; ignore.
            } else {
                // All neighbors processed: u is fully explored.
                if lowlink[u] == index[u] {
                    // Root of an SCC: pop the stack down to u.
                    let comp_id = components.len() as u32;
                    let mut comp = Vec::new();
                    loop {
                        let w = tarjan_stack.pop().expect("tarjan stack non-empty at root");
                        on_stack[w] = false;
                        comp_of[w] = comp_id;
                        comp.push(GraphNodeId::new(w as u32));
                        if w == u {
                            break;
                        }
                    }
                    components.push(comp);
                }
                // Pop this frame and propagate lowlink to the parent.
                call_stack.pop();
                if let Some(parent) = call_stack.last() {
                    let p = parent.node;
                    if lowlink[u] < lowlink[p] {
                        lowlink[p] = lowlink[u];
                    }
                }
            }
        }
    }

    Ok(SccResult {
        components,
        component_of: comp_of,
    })
}

/// Convenience: the condensation's topological order as owned component vectors.
///
/// Equivalent to `tarjan_scc(g)?.components` reversed.
pub fn condensation_topo_order(g: &GraphSnapshot) -> Result<Vec<Vec<GraphNodeId>>> {
    let mut r = tarjan_scc(g)?;
    r.components.reverse();
    Ok(r.components)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{BuildEdge, BuildNode, GraphBuilder};
    use prism_model::{RunId, VersionId};
    use std::collections::HashSet;

    fn nid(i: u32) -> GraphNodeId {
        GraphNodeId::new(i)
    }

    fn build(edges: &[(u32, u32)], n: u32) -> GraphSnapshot {
        let mut b = GraphBuilder::new();
        for i in 0..n {
            b.add_node(BuildNode::new(VersionId::new(i as u64), i))
                .unwrap();
        }
        for (ord, &(f, t)) in edges.iter().enumerate() {
            b.add_edge(BuildEdge::new(nid(f), nid(t), ord as u32)).unwrap();
        }
        b.finalize(RunId::new(1)).unwrap()
    }

    fn comp_set(c: &[GraphNodeId]) -> HashSet<u32> {
        c.iter().map(|x| x.get()).collect()
    }

    #[test]
    fn cycle_is_one_scc() {
        // 0 -> 1 -> 2 -> 0 forms one SCC; 3 hangs off 2.
        let g = build(&[(0, 1), (1, 2), (2, 0), (2, 3)], 4);
        let r = tarjan_scc(&g).unwrap();
        // Find the component containing 0.
        let big = r
            .components
            .iter()
            .find(|c| c.contains(&nid(0)))
            .unwrap();
        assert_eq!(comp_set(big), HashSet::from([0, 1, 2]));
        assert!(r.has_multinode_cycle());
        // 3 is its own singleton.
        let solo = r.components.iter().find(|c| c.contains(&nid(3))).unwrap();
        assert_eq!(comp_set(solo), HashSet::from([3]));
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn dag_topo_orders() {
        // 0 -> 1 -> 2, 0 -> 2. A DAG: all singletons.
        let g = build(&[(0, 1), (1, 2), (0, 2)], 3);
        let r = tarjan_scc(&g).unwrap();
        assert_eq!(r.len(), 3);
        assert!(!r.has_multinode_cycle());

        // Topo order: every edge goes from an earlier component to a later one.
        let order = r.topo_order();
        let mut position = [0usize; 3];
        for (pos, comp) in order.iter().enumerate() {
            for nd in comp.iter() {
                position[nd.get() as usize] = pos;
            }
        }
        for &(f, t) in &[(0u32, 1u32), (1, 2), (0, 2)] {
            assert!(
                position[f as usize] < position[t as usize],
                "edge {f}->{t} violates topo order"
            );
        }
    }

    #[test]
    fn topo_order_across_sccs() {
        // SCC A = {0,1} (0<->1); SCC B = {2,3} (2<->3); A -> B via 1 -> 2.
        let g = build(&[(0, 1), (1, 0), (1, 2), (2, 3), (3, 2)], 4);
        let r = tarjan_scc(&g).unwrap();
        assert_eq!(r.len(), 2);
        let order = r.topo_order();
        // The component containing 0 must come before the one containing 2.
        let pos_a = order.iter().position(|c| c.contains(&nid(0))).unwrap();
        let pos_b = order.iter().position(|c| c.contains(&nid(2))).unwrap();
        assert!(pos_a < pos_b);
        // condensation_topo_order agrees.
        let owned = condensation_topo_order(&g).unwrap();
        let pa = owned.iter().position(|c| c.contains(&nid(0))).unwrap();
        let pb = owned.iter().position(|c| c.contains(&nid(2))).unwrap();
        assert!(pa < pb);
    }

    #[test]
    fn self_loop_is_singleton_component() {
        let g = build(&[(0, 0)], 1);
        let r = tarjan_scc(&g).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(comp_set(&r.components[0]), HashSet::from([0]));
    }

    #[test]
    fn empty_graph() {
        let g = GraphBuilder::new().finalize(RunId::new(1)).unwrap();
        let r = tarjan_scc(&g).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn every_node_assigned_one_component() {
        let g = build(&[(0, 1), (1, 2), (2, 0), (2, 3), (3, 4)], 5);
        let r = tarjan_scc(&g).unwrap();
        for i in 0..5u32 {
            assert!(r.component_of(nid(i)).is_some());
        }
        // Total nodes across components = node count.
        let total: usize = r.components.iter().map(|c| c.len()).sum();
        assert_eq!(total, 5);
    }
}
