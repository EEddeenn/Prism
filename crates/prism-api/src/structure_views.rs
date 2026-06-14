//! Resolved views for structural analysis: scope-aware summaries, chokepoints,
//! "why is this here?" explanations, and classified cycles (architecture doc,
//! *Scope-Aware Analysis*, *Structural Control*, *Richer Cycle Analysis*).
//!
//! Each method loads the run's CSR snapshot once, runs a pure `prism-graph`
//! structural algorithm, and resolves the snapshot-local ids into display views
//! via the store's interned purls — mirroring [`crate::query`].

use prism_graph::{
    classify_cycles, dominators, explain, scope_summary, CycleClass, GraphSnapshot,
};
use prism_model::{DependencyKind, GraphNodeId, PrismError, Result, RunId};

use crate::views::GraphNodeView;
use crate::Prism;

/// Stable lowercase label for a dependency kind (matches manifest vocabulary).
pub fn kind_label(kind: DependencyKind) -> &'static str {
    match kind {
        DependencyKind::Runtime => "runtime",
        DependencyKind::Dev => "dev",
        DependencyKind::Build => "build",
        DependencyKind::Peer => "peer",
        DependencyKind::Optional => "optional",
        DependencyKind::Test => "test",
        DependencyKind::Plugin => "plugin",
        DependencyKind::Workspace => "workspace",
    }
}

/// Per-scope footprint summary, resolved for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeSummaryView {
    /// The dependency-kind label (`runtime`, `dev`, …).
    pub kind: &'static str,
    /// Edges of this kind anywhere in the graph.
    pub direct_edges: u32,
    /// Distinct occurrences directly targeted by an edge of this kind.
    pub direct_targets: usize,
    /// Occurrences reachable from the roots under production + this kind.
    pub footprint: usize,
    /// Occurrences present solely because of this scope (beyond production).
    pub exclusive: usize,
    /// Greatest BFS depth from the roots within the footprint.
    pub max_depth: u32,
}

/// A structurally-controlling occurrence (a dominator gateway).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChokepointView {
    /// The gateway occurrence, resolved for display.
    pub node: GraphNodeView,
    /// How many other occurrences are reachable from the roots only through it.
    pub dominated: u32,
}

/// A direct introducer of a target occurrence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntroducerView {
    /// The parent occurrence.
    pub node: GraphNodeView,
    /// The dependency-kind labels by which it pulls the target in.
    pub kinds: Vec<&'static str>,
}

/// "Why is this here?" — resolved explanation of a target occurrence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplanationView {
    /// The occurrence being explained.
    pub target: GraphNodeView,
    /// Direct parents that depend on the target.
    pub introducers: Vec<IntroducerView>,
    /// A few shortest dependency paths from a root to the target.
    pub paths: Vec<Vec<GraphNodeView>>,
    /// The nearest first-party (root) owner, if any.
    pub nearest_owner: Option<GraphNodeView>,
    /// Edge distance from `nearest_owner` to the target.
    pub owner_distance: u32,
    /// True if reachable from a root through production scopes only.
    pub production_reachable: bool,
    /// True if reachable but only through non-production scopes.
    pub non_production_only: bool,
}

/// A classified cycle, resolved for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CycleView {
    /// Member occurrences.
    pub members: Vec<GraphNodeView>,
    /// Internal edge count (the cycle's wiring).
    pub internal_edges: u32,
    /// Dependency-kind labels on the internal edges.
    pub kinds: Vec<&'static str>,
    /// True if any internal edge is a production kind.
    pub runtime_relevant: bool,
    /// True if a first-party root occurrence is inside the cycle.
    pub touches_root: bool,
    /// External occurrences depending on the cycle (blast radius).
    pub external_dependents: u32,
    /// Members entered from outside the cycle.
    pub entry_points: Vec<GraphNodeView>,
    /// Structural triage label.
    pub class: &'static str,
    /// One-line engineering interpretation of `class`.
    pub advice: &'static str,
}

fn cycle_class_label(c: CycleClass) -> (&'static str, &'static str) {
    match c {
        CycleClass::RuntimeCoupling => (
            "runtime-coupling",
            "ships and is depended upon externally — treat as one architectural unit; \
             any member change ripples across its blast radius",
        ),
        CycleClass::RuntimeCluster => (
            "runtime-cluster",
            "ships but is contained — safe to upgrade as a single locked group",
        ),
        CycleClass::ToolingCycle => (
            "tooling-cycle",
            "dev/build/test only — does not reach production; low runtime risk",
        ),
    }
}

impl Prism {
    /// Scope-aware footprint summaries for `run`, one row per dependency kind
    /// that appears, in a stable kind order.
    pub fn scope_summary(&self, run: RunId) -> Result<Vec<ScopeSummaryView>> {
        let g = self.load_graph(run)?;
        Ok(scope_summary(&g)?
            .into_iter()
            .map(|fp| ScopeSummaryView {
                kind: kind_label(fp.kind),
                direct_edges: fp.direct_edges,
                direct_targets: fp.direct_targets.len(),
                footprint: fp.footprint.len(),
                exclusive: fp.exclusive.len(),
                max_depth: fp.max_depth,
            })
            .collect())
    }

    /// Chokepoints of `run`: non-root occurrences that structurally control
    /// access to large parts of the graph, ranked by descending dominated count
    /// and capped at `limit` (0 = no cap).
    pub fn chokepoints(&self, run: RunId, limit: usize) -> Result<Vec<ChokepointView>> {
        let g = self.load_graph(run)?;
        let dt = dominators(&g)?;
        let mut cps = dt.chokepoints(&g, 1)?;
        if limit > 0 && cps.len() > limit {
            cps.truncate(limit);
        }
        cps.into_iter()
            .map(|c| {
                Ok(ChokepointView {
                    node: self.node_view(&g, c.node)?,
                    dominated: c.dominated,
                })
            })
            .collect()
    }

    /// Explain why `node` is present in `run`: introducers, shortest paths from
    /// the roots, nearest first-party owner, and production reachability.
    pub fn explain_node(&self, run: RunId, node: GraphNodeId, max_paths: usize) -> Result<ExplanationView> {
        let g = self.load_graph(run)?;
        if node.index().map_or(true, |i| i >= g.node_count()) {
            return Err(PrismError::OutOfBounds {
                detail: format!("node {node:?} out of range for run {}", run.get()),
            });
        }
        let ex = explain(&g, node, max_paths.max(1))?;
        let introducers = ex
            .introducers
            .iter()
            .map(|i| {
                Ok(IntroducerView {
                    node: self.node_view(&g, i.node)?,
                    kinds: i.kinds.iter().map(|&k| kind_label(k)).collect(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut paths = Vec::with_capacity(ex.paths.len());
        for p in &ex.paths {
            let mut row = Vec::with_capacity(p.len());
            for &nid in p {
                row.push(self.node_view(&g, nid)?);
            }
            paths.push(row);
        }
        let nearest_owner = match ex.nearest_owner {
            Some(o) => Some(self.node_view(&g, o)?),
            None => None,
        };
        Ok(ExplanationView {
            target: self.node_view(&g, node)?,
            introducers,
            paths,
            nearest_owner,
            owner_distance: ex.owner_distance,
            production_reachable: ex.production_reachable,
            non_production_only: ex.non_production_only,
        })
    }

    /// Classified cycles of `run`, largest first, resolved for display.
    pub fn classified_cycles(&self, run: RunId) -> Result<Vec<CycleView>> {
        let g = self.load_graph(run)?;
        classify_cycles(&g)?
            .into_iter()
            .map(|c| self.cycle_view(&g, c))
            .collect()
    }

    fn cycle_view(&self, g: &GraphSnapshot, c: prism_graph::CycleInfo) -> Result<CycleView> {
        let members = c
            .members
            .iter()
            .map(|&n| self.node_view(g, n))
            .collect::<Result<Vec<_>>>()?;
        let entry_points = c
            .entry_points
            .iter()
            .map(|&n| self.node_view(g, n))
            .collect::<Result<Vec<_>>>()?;
        let (class, advice) = cycle_class_label(c.class);
        Ok(CycleView {
            members,
            internal_edges: c.internal_edges,
            kinds: c.kinds.iter().map(|&k| kind_label(k)).collect(),
            runtime_relevant: c.runtime_relevant,
            touches_root: c.touches_root,
            external_dependents: c.external_dependents,
            entry_points,
            class,
            advice,
        })
    }
}
