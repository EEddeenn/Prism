//! Higher-order overlap analysis (architecture doc, *Higher-Order Overlap*).
//!
//! Each top-level dependency of the roots defines a subsystem — everything its
//! own subtree pulls in. When the same package sits in *many* subsystems'
//! footprints, those subsystems share a dependency surface. A high-spread package
//! that is not an obvious shared foundation is exactly the "multiple subsystems
//! depend on this without a clear common abstraction" smell.
//!
//! [`Prism::higher_order_overlap`] computes each subsystem's forward footprint,
//! then ranks packages by **spread** (how many distinct subsystems include them)
//! and reports the subsystem pairs with the largest shared surface.

use std::collections::{BTreeMap, BTreeSet};

use prism_graph::{forward_reachable, TraversalOptions};
use prism_model::{GraphNodeId, NodeFlags, Result, RunId};

use crate::Prism;

/// A package shared across multiple subsystems.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedSurface {
    /// The shared package purl.
    pub purl: String,
    /// How many distinct subsystems include it (its spread).
    pub spread: u32,
    /// A sample of the owning subsystems' purls.
    pub subsystems: Vec<String>,
}

/// A pair of subsystems with a notable shared dependency surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlapPair {
    /// First subsystem owner purl.
    pub a: String,
    /// Second subsystem owner purl.
    pub b: String,
    /// Number of packages in both footprints.
    pub shared: u32,
}

/// The higher-order overlap report for a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlapView {
    /// The run analyzed.
    pub run: RunId,
    /// Number of top-level subsystems considered.
    pub subsystems: usize,
    /// Packages shared by ≥ 2 subsystems, by descending spread.
    pub shared_surfaces: Vec<SharedSurface>,
    /// Subsystem pairs with the largest shared surface.
    pub top_pairs: Vec<OverlapPair>,
}

impl Prism {
    /// Higher-order overlap across the top-level subsystems of `run`.
    pub fn higher_order_overlap(&self, run: RunId, limit: usize) -> Result<OverlapView> {
        let g = self.load_graph(run)?;

        // Top-level subsystem owners = distinct non-root out-neighbors of roots.
        let mut owners: Vec<GraphNodeId> = Vec::new();
        let mut seen: BTreeSet<u32> = BTreeSet::new();
        for i in 0..g.node_count() as u32 {
            let node = GraphNodeId::new(i);
            if !g.node_flags(node)?.contains(NodeFlags::ROOT) {
                continue;
            }
            for &v in g.out_neighbors(node)? {
                if !g.node_flags(v)?.contains(NodeFlags::ROOT) && seen.insert(v.get()) {
                    owners.push(v);
                }
            }
        }

        // Each owner's forward footprint (including itself).
        let mut footprints: Vec<(GraphNodeId, BTreeSet<u32>)> = Vec::with_capacity(owners.len());
        // spread[node] = how many subsystems include it; owners_of for samples.
        let mut spread: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (idx, &owner) in owners.iter().enumerate() {
            let opts = TraversalOptions {
                include_root: true,
                ..Default::default()
            };
            let reached = forward_reachable(&g, owner, opts)?;
            let mut set = BTreeSet::new();
            for nd in &reached.nodes {
                set.insert(nd.get());
                spread.entry(nd.get()).or_default().push(idx);
            }
            footprints.push((owner, set));
        }

        // Shared surfaces: packages in ≥ 2 subsystems.
        let mut shared_surfaces: Vec<SharedSurface> = spread
            .iter()
            .filter(|(_, subs)| subs.len() >= 2)
            .map(|(&node, subs)| {
                let subsystems = subs
                    .iter()
                    .take(4)
                    .map(|&i| self.version_label(g.node_version(owners[i]).unwrap_or_else(|_| prism_model::VersionId::new(0))))
                    .collect();
                SharedSurface {
                    purl: self.version_label(g.node_version(GraphNodeId::new(node)).unwrap_or_else(|_| prism_model::VersionId::new(0))),
                    spread: subs.len() as u32,
                    subsystems,
                }
            })
            .collect();
        shared_surfaces.sort_by(|a, b| b.spread.cmp(&a.spread).then(a.purl.cmp(&b.purl)));
        if limit > 0 && shared_surfaces.len() > limit {
            shared_surfaces.truncate(limit);
        }

        // Pairwise overlap sizes (only over a bounded number of owners to stay
        // O(k^2) cheap; subsystems are top-level so k is small).
        let mut top_pairs: Vec<OverlapPair> = Vec::new();
        for i in 0..footprints.len() {
            for j in (i + 1)..footprints.len() {
                let shared = footprints[i].1.intersection(&footprints[j].1).count() as u32;
                if shared > 0 {
                    top_pairs.push(OverlapPair {
                        a: self.version_label(g.node_version(footprints[i].0)?),
                        b: self.version_label(g.node_version(footprints[j].0)?),
                        shared,
                    });
                }
            }
        }
        top_pairs.sort_by(|x, y| y.shared.cmp(&x.shared).then(x.a.cmp(&y.a)));
        if limit > 0 && top_pairs.len() > limit {
            top_pairs.truncate(limit);
        }

        Ok(OverlapView {
            run,
            subsystems: owners.len(),
            shared_surfaces,
            top_pairs,
        })
    }
}
