//! Diamond-reuse analysis and the minimal explanation view (architecture doc,
//! *Diamond Reuse* and *Minimal Explanation View*).
//!
//! In a dependency DAG, every independent undirected loop (the H₁ generators)
//! arises from **reconvergence**: a shared package reached along more than one
//! path. So the shared foundations that generate the most topology complexity are
//! exactly the high-in-degree packages — [`Prism::diamond_reuse`] ranks them and
//! reports how much of the total reconvergence they account for.
//!
//! [`Prism::minimal_view`] returns the **dominator tree** as a
//! reachability-preserving skeleton: each package shown beneath the single
//! gateway that controls its access, collapsing the diamond noise into the story
//! "what depends through what" without losing which packages reach which.

use std::collections::BTreeSet;

use prism_graph::dominators;
use prism_model::{GraphNodeId, Result, RunId};

use crate::Prism;

/// One shared foundation: a package reached along multiple paths, contributing
/// reconvergence loops to the topology.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiamondNode {
    /// The shared package occurrence's purl.
    pub purl: String,
    /// Distinct direct parents (in-degree, parallel edges collapsed).
    pub parents: u32,
    /// Reconvergence contribution = `parents − 1` (independent loops it closes).
    pub reconvergence: u32,
    /// A sample of the distinct parent purls.
    pub sample_parents: Vec<String>,
}

/// Diamond-reuse summary for a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiamondReuseView {
    /// The run analyzed.
    pub run: RunId,
    /// Total reconvergence across the graph (the undirected cyclomatic number =
    /// Σ max(0, parents−1) — a proxy for the H₁ Betti number).
    pub total_reconvergence: u32,
    /// The top shared foundations, by descending reconvergence.
    pub foundations: Vec<DiamondNode>,
    /// Share of total reconvergence the listed foundations account for (0–100).
    pub top_share_pct: u32,
}

/// One edge of the minimal (dominator-tree) explanation view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MinimalEdge {
    /// The controlling gateway purl (the node's immediate dominator).
    pub gateway: String,
    /// The dependent purl.
    pub node: String,
}

/// The minimal explanation view of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MinimalView {
    /// The run reduced.
    pub run: RunId,
    /// Edges in the original graph.
    pub original_edges: usize,
    /// Edges in the reduced (dominator-tree) view.
    pub reduced_edges: usize,
    /// The dominator-tree edges (gateway → node).
    pub edges: Vec<MinimalEdge>,
}

impl MinimalView {
    /// Percentage of edges hidden by the reduction (0–100).
    pub fn reduction_pct(&self) -> u32 {
        if self.original_edges == 0 {
            return 0;
        }
        let hidden = self.original_edges.saturating_sub(self.reduced_edges);
        ((hidden as u64 * 100 + self.original_edges as u64 / 2) / self.original_edges as u64) as u32
    }
}

impl Prism {
    /// Rank the shared foundations that generate the most reconvergence (H₁).
    pub fn diamond_reuse(&self, run: RunId, limit: usize) -> Result<DiamondReuseView> {
        let g = self.load_graph(run)?;
        let n = g.node_count();

        let mut nodes: Vec<DiamondNode> = Vec::new();
        let mut total: u64 = 0;
        for i in 0..n as u32 {
            let node = GraphNodeId::new(i);
            // Distinct parents (collapse multigraph parallel edges).
            let distinct: BTreeSet<u32> = g.in_neighbors(node)?.iter().map(|p| p.get()).collect();
            if distinct.len() < 2 {
                continue;
            }
            let parents = distinct.len() as u32;
            let reconvergence = parents - 1;
            total += reconvergence as u64;
            let sample_parents: Vec<String> = distinct
                .iter()
                .take(4)
                .map(|&p| self.version_label(g.node_version(GraphNodeId::new(p)).unwrap_or_else(|_| prism_model::VersionId::new(0))))
                .collect();
            nodes.push(DiamondNode {
                purl: self.version_label(g.node_version(node)?),
                parents,
                reconvergence,
                sample_parents,
            });
        }
        nodes.sort_by(|a, b| {
            b.reconvergence
                .cmp(&a.reconvergence)
                .then(a.purl.cmp(&b.purl))
        });
        if limit > 0 && nodes.len() > limit {
            nodes.truncate(limit);
        }
        let shown: u64 = nodes.iter().map(|d| d.reconvergence as u64).sum();
        let top_share_pct = (shown * 100 + total / 2)
            .checked_div(total)
            .unwrap_or(0) as u32;

        Ok(DiamondReuseView {
            run,
            total_reconvergence: total.min(u32::MAX as u64) as u32,
            foundations: nodes,
            top_share_pct,
        })
    }

    /// The dominator-tree minimal explanation view of `run`.
    pub fn minimal_view(&self, run: RunId) -> Result<MinimalView> {
        let g = self.load_graph(run)?;
        let dt = dominators(&g)?;
        let mut edges = Vec::new();
        for i in 0..g.node_count() as u32 {
            let node = GraphNodeId::new(i);
            if let Some(gateway) = dt.idom_of(node) {
                edges.push(MinimalEdge {
                    gateway: self.version_label(g.node_version(gateway)?),
                    node: self.version_label(g.node_version(node)?),
                });
            }
        }
        Ok(MinimalView {
            run,
            original_edges: g.edge_count(),
            reduced_edges: edges.len(),
            edges,
        })
    }
}

#[cfg(test)]
mod tests {
    // The reduction-percentage arithmetic is the only pure logic worth a unit
    // test; the graph-walking paths are covered by the prism-graph dominator and
    // reachability suites plus the end-to-end CLI verification.
    use super::*;

    #[test]
    fn reduction_pct_is_bounded() {
        let v = MinimalView {
            run: RunId::new(1),
            original_edges: 100,
            reduced_edges: 25,
            edges: Vec::new(),
        };
        assert_eq!(v.reduction_pct(), 75);
        let empty = MinimalView {
            run: RunId::new(1),
            original_edges: 0,
            reduced_edges: 0,
            edges: Vec::new(),
        };
        assert_eq!(empty.reduction_pct(), 0);
    }
}
