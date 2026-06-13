//! Synthetic resolution (best-effort) over the store-backed facade.
//!
//! The solid, fully-wired path in v0.1 is lockfile ingestion
//! ([`Prism::ingest_lockfile`]). Synthetic resolution via
//! [`prism_resolve::Engine`] needs store-backed [`RequirementSource`] /
//! [`CandidateSource`] implementations; that wiring is intentionally left for a
//! later milestone. What the facade *does* offer today is the persistence half:
//! given any [`ResolvedGraph`] whose nodes already carry **real** store
//! [`VersionId`]s (e.g. the `graph` of a [`prism_resolve::Resolution`]), persist
//! it as a run exactly like an ingested lockfile.
//!
//! [`Prism::resolve_synthetic`] is the documented entry point: it persists a
//! caller-produced `ResolvedGraph`. A future revision will own the engine wiring
//! and produce that graph internally.

use prism_graph::{write_pcsr, BuildEdge, BuildNode, GraphBuilder};
use prism_model::{
    EcosystemId, EnvironmentId, FidelityTier, GraphFidelity, GraphNodeId, Result, RunId, RunStatus,
    SnapshotId, StringId,
};
use prism_plugin_api::ResolvedGraph;

use crate::ecosystem::Ecosystem;
use crate::Prism;

impl Prism {
    /// Persist a synthetic [`ResolvedGraph`] (real [`VersionId`]s) as a new run,
    /// returning its [`RunId`].
    ///
    /// This is the best-effort synthetic path: the caller supplies the resolved
    /// graph (e.g. from [`prism_resolve::Engine::resolve`]), and the facade
    /// builds + persists the CSR snapshot and records the resolver run. The graph
    /// nodes' `version_id`s are taken **as-is** — they must already be real store
    /// ids, not synthetic ones. (Lockfile ingestion, which interns synthetic ids,
    /// remains the recommended path.)
    ///
    /// `resolver_name` labels the run (e.g. `"synthetic"`); `fidelity` defaults to
    /// the graph's own fidelity, or [`FidelityTier::ModelResolved`] if unset.
    pub fn resolve_synthetic(
        &mut self,
        eco: Ecosystem,
        graph: &ResolvedGraph,
        resolver_name: &str,
    ) -> Result<RunId> {
        let run = self.store.alloc_run_id();

        let mut builder = GraphBuilder::new();
        for node in &graph.nodes {
            builder.add_node(BuildNode {
                version_id: node.version_id,
                depth: node.depth,
                flags: node.flags,
                attr: None,
            })?;
        }
        for edge in &graph.edges {
            builder.add_edge(BuildEdge {
                from: GraphNodeId::new(edge.from),
                to: GraphNodeId::new(edge.to),
                ordinal: edge.ordinal,
                requirement_id: edge.requirement_id,
                kind: edge.kind,
                scope: StringId::new(0),
                flags: edge.flags,
            })?;
        }
        let snapshot = builder.finalize(run)?;
        let image = write_pcsr(&snapshot);
        self.store.publish_graph(run, &image)?;

        let fidelity = graph
            .fidelity
            .unwrap_or_else(|| GraphFidelity::exact(FidelityTier::ModelResolved));
        self.write_synthetic_run(run, eco.id(), resolver_name, fidelity)?;
        Ok(run)
    }

    /// Write the `ResolverRun` record for a synthetic resolution.
    fn write_synthetic_run(
        &mut self,
        run: RunId,
        eco: EcosystemId,
        resolver_name: &str,
        fidelity: GraphFidelity,
    ) -> Result<()> {
        let name_id = self.store.intern_string(resolver_name)?;
        let record = prism_model::ResolverRun {
            id: run,
            ecosystem: eco,
            root_set_hash: prism_codec::sha256(resolver_name.as_bytes()),
            resolver_name: name_id,
            resolver_version: None,
            environment_id: EnvironmentId::new(0),
            snapshot_id: SnapshotId::new(0),
            input_manifest_ref: None,
            input_lockfile_ref: None,
            status: RunStatus::Complete,
            fidelity,
            reference_unix: 0,
            created_at_unix: 0,
            metadata_ref: None,
        };
        let mut batch = self.store.batch();
        self.store.put_run(&mut batch, &record);
        self.store.commit_batch(batch)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;
    use prism_model::{EdgeFlags, NodeFlags, VersionId};
    use prism_plugin_api::ResolvedEdge;

    #[test]
    fn persists_a_synthetic_graph_as_a_run() {
        let td = TempDir::new("api-resolve");
        let mut prism = Prism::init(td.path()).unwrap();

        // A tiny graph with two real version ids and one edge.
        let mut g = ResolvedGraph::new();
        let a = g.add_node(VersionId::new(101), 0, "root", NodeFlags::ROOT);
        let b = g.add_node(VersionId::new(202), 1, "dep", NodeFlags::EMPTY);
        g.add_edge(ResolvedEdge {
            from: a,
            to: b,
            ordinal: 0,
            requirement_id: None,
            kind: prism_model::DependencyKind::Runtime,
            flags: EdgeFlags::RESOLVED,
        });

        let run = prism
            .resolve_synthetic(Ecosystem::Cargo, &g, "synthetic")
            .unwrap();
        let stats = prism.graph_stats(run).unwrap();
        assert_eq!(stats.nodes, 2);
        assert_eq!(stats.edges, 1);
    }
}
