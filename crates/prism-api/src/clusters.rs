//! Ecosystem / workspace / subsystem clustering (architecture doc,
//! *Clustering*).
//!
//! Two granularities:
//! - [`Prism::workspace_regions`] is **cross-run**: it lists every run in the
//!   workspace as a comparable dependency region (one per ecosystem in a polyglot
//!   repo), with side-by-side structural stats.
//! - [`Prism::subsystems`] is **within-run**: it partitions one graph into the
//!   subsystems owned by each top-level (depth-1) dependency, using the dominator
//!   tree to size each one's privately-owned subtree.

use std::collections::BTreeSet;

use prism_graph::{classify_cycles, dominators, family_clusters, scope_summary};
use prism_model::{GraphNodeId, NodeFlags, Result, RunId};
use prism_store::{record_type, records};

use crate::ecosystem::ecosystem_of;
use crate::Prism;

/// One dependency region of the workspace (normally one ecosystem's run).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceRegion {
    /// The run backing this region.
    pub run: RunId,
    /// Ecosystem label.
    pub ecosystem: &'static str,
    /// Occurrence nodes.
    pub nodes: usize,
    /// Edges.
    pub edges: usize,
    /// Production (runtime) footprint reachable from the roots.
    pub runtime_footprint: usize,
    /// Number of dependency cycles.
    pub cycles: usize,
    /// Version-fragmented families.
    pub duplicate_families: usize,
    /// Overall structural risk band label.
    pub risk_band: &'static str,
    /// Overall structural risk score (0–100).
    pub risk_score: u32,
}

/// A subsystem of one run: a top-level dependency and the subtree it owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subsystem {
    /// The owning top-level dependency's purl.
    pub owner_purl: String,
    /// Total occurrences in the owner's dominated subtree (including itself).
    pub size: u32,
    /// Occurrences owned exclusively by this subsystem (dominated by the owner —
    /// not shared with any sibling subsystem).
    pub exclusive: u32,
    /// True if the owner is introduced by a production-kind edge.
    pub runtime: bool,
}

impl Prism {
    /// Every run in the workspace as a comparable dependency region.
    pub fn workspace_regions(&self) -> Result<Vec<WorkspaceRegion>> {
        let mut runs: Vec<RunId> = Vec::new();
        let mut seen: BTreeSet<u64> = BTreeSet::new();
        for rec in self.store.read_all_records()? {
            if rec.record_type == record_type::RESOLVER_RUN {
                if let Ok(run) = records::decode_resolver_run(&rec.value) {
                    if seen.insert(run.id.get()) {
                        runs.push(run.id);
                    }
                }
            }
        }
        runs.sort_by_key(|r| r.get());

        let mut out = Vec::with_capacity(runs.len());
        for run in runs {
            let record = self.store.get_run(run)?;
            let eco = ecosystem_of(record.ecosystem);
            let g = self.load_graph(run)?;
            let cycles = classify_cycles(&g)?.len();
            let duplicate_families = family_clusters(&g, |v| self.package_of(v))?
                .into_iter()
                .filter(|c| c.is_version_fragmented())
                .count();
            let runtime_footprint = scope_summary(&g)?
                .iter()
                .find(|fp| fp.kind == prism_model::DependencyKind::Runtime)
                .map(|fp| fp.footprint.len())
                .unwrap_or(0);
            let risk = self.structural_risk(run)?;
            out.push(WorkspaceRegion {
                run,
                ecosystem: eco.label(),
                nodes: g.node_count(),
                edges: g.edge_count(),
                runtime_footprint,
                cycles,
                duplicate_families,
                risk_band: risk.band,
                risk_score: risk.overall,
            });
        }
        Ok(out)
    }

    /// Partition `run` into subsystems owned by each top-level dependency.
    pub fn subsystems(&self, run: RunId) -> Result<Vec<Subsystem>> {
        let g = self.load_graph(run)?;
        let dt = dominators(&g)?;

        // Top-level deps = distinct non-root out-neighbors of root nodes, with the
        // kind of the edge that introduces them.
        let mut top: Vec<(GraphNodeId, bool)> = Vec::new();
        let mut seen: BTreeSet<u32> = BTreeSet::new();
        for i in 0..g.node_count() as u32 {
            let node = GraphNodeId::new(i);
            if !g.node_flags(node)?.contains(NodeFlags::ROOT) {
                continue;
            }
            let neigh = g.out_neighbors(node)?;
            let eids = g.out_edge_ids(node)?;
            for (k, &v) in neigh.iter().enumerate() {
                if g.node_flags(v)?.contains(NodeFlags::ROOT) {
                    continue; // skip root→root workspace links
                }
                if seen.insert(v.get()) {
                    let kind = prism_graph::kind_from_u16(g.edge_kind(eids[k])?)?;
                    let runtime = matches!(
                        kind,
                        prism_model::DependencyKind::Runtime | prism_model::DependencyKind::Workspace
                    );
                    top.push((v, runtime));
                }
            }
        }

        let mut out = Vec::with_capacity(top.len());
        for (owner, runtime) in top {
            let exclusive = dt.dominated_count(owner);
            out.push(Subsystem {
                owner_purl: self.version_label(g.node_version(owner)?),
                size: exclusive.saturating_add(1),
                exclusive,
                runtime,
            });
        }
        // Largest subsystems first.
        out.sort_by(|a, b| {
            b.size
                .cmp(&a.size)
                .then(a.owner_purl.cmp(&b.owner_purl))
        });
        Ok(out)
    }
}
