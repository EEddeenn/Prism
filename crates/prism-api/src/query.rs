//! Graph queries over a persisted run (architecture doc, *Graph Algorithms*).
//!
//! Each query loads the run's CSR [`GraphSnapshot`] (`open_graph` → `read_pcsr`),
//! runs a pure `prism-graph` algorithm, and resolves the snapshot-local ids into
//! display [views](crate::views) via the store's interned purls.

use std::collections::BTreeSet;

use prism_graph::{
    family_clusters, forward_reachable, reverse_reachable, tarjan_scc, GraphSnapshot,
    TraversalOptions,
};
use prism_model::{GraphNodeId, PackageId, PrismError, Result, RunId, VersionId};

use crate::views::{
    DuplicateClusterView, DuplicateVersion, GraphEdgeView, GraphNodeView, GraphStatsView,
    NeighborhoodView, SccView,
};
use crate::Prism;

impl Prism {
    /// Whole-graph summary statistics for `run`.
    pub fn graph_stats(&self, run: RunId) -> Result<GraphStatsView> {
        let g = self.load_graph(run)?;
        let scc = tarjan_scc(&g)?;
        let mut max_depth = 0u32;
        for i in 0..g.node_count() as u32 {
            max_depth = max_depth.max(g.node_depth(GraphNodeId::new(i))?);
        }
        // "Duplicate versions" = package families present at more than one
        // distinct version (architecture doc, *Duplicate Version Clusters*). This
        // groups by family, not by version, so e.g. `windows-sys` at four versions
        // counts once.
        let duplicate_version_count = family_clusters(&g, |v| self.package_of(v))?
            .into_iter()
            .filter(|c| c.is_version_fragmented())
            .count();
        Ok(GraphStatsView {
            run,
            nodes: g.node_count(),
            edges: g.edge_count(),
            scc_count: scc.len(),
            has_cycle: scc.has_multinode_cycle(),
            max_depth,
            duplicate_version_count,
        })
    }

    /// Forward neighborhood of `node` in `run`: descendants within `depth`,
    /// capped at `limit` nodes, plus the edges induced among the result.
    pub fn neighborhood(
        &self,
        run: RunId,
        node: GraphNodeId,
        depth: u32,
        limit: u32,
    ) -> Result<NeighborhoodView> {
        let g = self.load_graph(run)?;
        let opts = TraversalOptions {
            max_depth: depth,
            max_nodes: limit,
            ..Default::default()
        };
        let reached = forward_reachable(&g, node, opts)?;
        self.assemble_neighborhood(&g, node, &reached.nodes, reached.truncated)
    }

    /// Reverse impact of `node` in `run`: ancestors that (transitively) depend on
    /// `node`, within `depth`. The center is the queried node.
    pub fn impact(&self, run: RunId, node: GraphNodeId, depth: u32) -> Result<NeighborhoodView> {
        let g = self.load_graph(run)?;
        let opts = TraversalOptions {
            max_depth: depth,
            ..Default::default()
        };
        let reached = reverse_reachable(&g, node, opts)?;
        self.assemble_neighborhood(&g, node, &reached.nodes, reached.truncated)
    }

    /// Reverse impact of an entire package family: every node whose version is in
    /// `package`, unioned with all their ancestors within `depth`. Useful for
    /// "what breaks if this package changes" across all its occurrences.
    pub fn impact_package(
        &self,
        run: RunId,
        package: PackageId,
        depth: u32,
    ) -> Result<Vec<GraphNodeView>> {
        let g = self.load_graph(run)?;
        let opts = TraversalOptions {
            max_depth: depth,
            ..Default::default()
        };
        let mut result: BTreeSet<u32> = BTreeSet::new();
        for i in 0..g.node_count() as u32 {
            let nid = GraphNodeId::new(i);
            if self.package_of(g.node_version(nid)?) == Some(package) {
                result.insert(i);
                for anc in reverse_reachable(&g, nid, opts)?.nodes {
                    result.insert(anc.get());
                }
            }
        }
        result
            .into_iter()
            .map(|i| self.node_view(&g, GraphNodeId::new(i)))
            .collect()
    }

    /// The strongly-connected components of `run`, resolved for display, in
    /// Tarjan emission order.
    pub fn scc(&self, run: RunId) -> Result<Vec<SccView>> {
        let g = self.load_graph(run)?;
        let scc = tarjan_scc(&g)?;
        let mut out = Vec::with_capacity(scc.components.len());
        for comp in &scc.components {
            let mut members = Vec::with_capacity(comp.len());
            for &nid in comp {
                members.push(self.node_view(&g, nid)?);
            }
            out.push(SccView { members });
        }
        Ok(out)
    }

    /// Duplicate-version clusters of `run`: each package *family* present at more
    /// than one distinct version, resolved for display, ordered by package id.
    pub fn duplicate_clusters(&self, run: RunId) -> Result<Vec<DuplicateClusterView>> {
        let g = self.load_graph(run)?;
        let mut out = Vec::new();
        for cluster in family_clusters(&g, |v| self.package_of(v))? {
            if !cluster.is_version_fragmented() {
                continue;
            }
            out.push(DuplicateClusterView {
                package: cluster.package_id,
                package_purl: self.package_purl(cluster.package_id),
                versions: cluster
                    .versions
                    .iter()
                    .map(|&v| DuplicateVersion {
                        version: v,
                        purl: self.version_label(v),
                    })
                    .collect(),
                occurrence_count: cluster.occurrence_count,
                min_depth: cluster.min_depth,
                max_depth: cluster.max_depth,
            });
        }
        Ok(out)
    }

    // --- internal -----------------------------------------------------------

    /// Resolve one snapshot-local node id into a display view.
    fn node_view(&self, g: &GraphSnapshot, node: GraphNodeId) -> Result<GraphNodeView> {
        let version = g.node_version(node)?;
        Ok(GraphNodeView {
            id: node,
            version,
            package: self.package_of(version),
            purl: self.version_label(version),
            depth: g.node_depth(node)?,
        })
    }

    /// Build a [`NeighborhoodView`] from a center and a set of reached nodes,
    /// including the edges induced among `{center} ∪ reached`.
    fn assemble_neighborhood(
        &self,
        g: &GraphSnapshot,
        center: GraphNodeId,
        reached: &[GraphNodeId],
        truncated: bool,
    ) -> Result<NeighborhoodView> {
        let center_view = self.node_view(g, center)?;
        let mut nodes = Vec::with_capacity(reached.len());
        for &nid in reached {
            nodes.push(self.node_view(g, nid)?);
        }

        // The set of nodes in scope (center + reached) for inducing edges.
        let mut in_scope: BTreeSet<u32> = reached.iter().map(|n| n.get()).collect();
        in_scope.insert(center.get());

        let mut edges = Vec::new();
        for &src in &in_scope {
            let from = GraphNodeId::new(src);
            let targets = g.out_neighbors(from)?;
            let edge_ids = g.out_edge_ids(from)?;
            for (k, &to) in targets.iter().enumerate() {
                if in_scope.contains(&to.get()) {
                    let eid = edge_ids[k];
                    edges.push(GraphEdgeView {
                        source: from,
                        target: to,
                        kind: g.edge_kind(eid)?,
                        flags: g.edge_flags(eid)?,
                    });
                }
            }
        }

        Ok(NeighborhoodView {
            center: center_view,
            nodes,
            edges,
            truncated,
        })
    }
}

/// Resolve a node by [`VersionId`] equality within a run (the first occurrence).
/// Convenience for callers that have a version, not a node id.
pub(crate) fn node_for_version(g: &GraphSnapshot, version: VersionId) -> Option<GraphNodeId> {
    (0..g.node_count() as u32)
        .map(GraphNodeId::new)
        .find(|&n| g.node_version(n).ok() == Some(version))
}

impl Prism {
    /// The first node in `run` whose version is `version`, if any.
    pub fn node_of_version(&self, run: RunId, version: VersionId) -> Result<Option<GraphNodeId>> {
        let g = self.load_graph(run)?;
        Ok(node_for_version(&g, version))
    }

    /// Resolve a node id in `run` to a display view (bounds-checked).
    pub fn node(&self, run: RunId, node: GraphNodeId) -> Result<GraphNodeView> {
        let g = self.load_graph(run)?;
        if node.index().map_or(true, |i| i >= g.node_count()) {
            return Err(PrismError::OutOfBounds {
                detail: format!("node {node:?} out of range for run {}", run.get()),
            });
        }
        self.node_view(&g, node)
    }
}
