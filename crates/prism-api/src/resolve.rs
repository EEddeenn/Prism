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
    EcosystemId, EnvironmentId, FidelityTier, GraphFidelity, GraphNodeId, PrismError, Result, RunId,
    RunStatus, SnapshotId, StringId, VersionId,
};
use prism_plugin_api::ResolvedGraph;
use prism_resolve::{Engine, ResolveBudget};

use crate::ecosystem::Ecosystem;
use crate::resolve_store::{policy_from_label, StoreCandidateSource, StoreRequirementSource};
use crate::Prism;

/// The outcome of a synthetic resolution: the run plus its graph size and the
/// policy applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntheticResolve {
    /// The run id whose graph was persisted.
    pub run: RunId,
    /// Number of occurrence nodes in the persisted graph.
    pub nodes: usize,
    /// Number of edges in the persisted graph.
    pub edges: usize,
    /// True if a budget truncated the resolution.
    pub truncated: bool,
}

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
        self.ensure_writable()?;
        self.persist_resolved_graph(eco, graph, resolver_name)
            .map(|r| r.run)
    }

    /// Ingest a manifest at `path`, then run the synthetic resolver from the
    /// manifest's declaring version(s) under `policy`. Returns the run details.
    ///
    /// This is the end-to-end synthetic entry point the CLI's `resolve` command
    /// uses: it populates `Requirement` records (so the engine has a closure to
    /// expand) and then resolves from the roots those requirements declare.
    pub fn resolve_manifest(
        &mut self,
        path: impl AsRef<std::path::Path>,
        policy: &str,
    ) -> Result<SyntheticResolve> {
        let path = path.as_ref();
        let eco = Ecosystem::detect(path).ok_or_else(|| {
            PrismError::Other(format!(
                "cannot detect ecosystem from file name {:?}",
                path.file_name()
            ))
        })?;
        let ingest = self.ingest_manifest(path)?;
        debug_assert_eq!(ingest.ecosystem, eco);
        let roots = self.declaring_versions(eco)?;
        if roots.is_empty() {
            return Err(PrismError::Other(
                "manifest declared no requirements to resolve from".into(),
            ));
        }
        self.resolve_synthetic_from_roots_detailed(eco, &roots, policy)
    }

    /// The distinct source versions of requirements in this ecosystem — the
    /// natural synthetic-resolution roots (a manifest's declaring version is the
    /// source of every requirement it declares).
    fn declaring_versions(&self, eco: Ecosystem) -> Result<Vec<VersionId>> {
        use std::collections::BTreeSet;
        let mut roots: BTreeSet<u64> = BTreeSet::new();
        for rec in self.store().read_all_records()? {
            if rec.record_type != prism_store::record_type::REQUIREMENT {
                continue;
            }
            let Ok(req) = prism_store::records::decode_requirement(&rec.value) else {
                continue;
            };
            // Restrict to this ecosystem via the source version's package.
            if let Ok(v) = self.store().get_version(req.src_version_id) {
                if let Ok(p) = self.store().get_package(v.package_id) {
                    if p.ecosystem == eco.id() {
                        roots.insert(req.src_version_id.get());
                    }
                }
            }
        }
        Ok(roots.into_iter().map(VersionId::new).collect())
    }

    /// Run the synthetic [`Engine`] from a set of root [`VersionId`]s under
    /// `policy`, persisting the resulting graph as a `ModelResolved` run.
    ///
    /// Requirements must already be in the store (ingest a manifest first). The
    /// engine pulls declared requirements from [`StoreRequirementSource`] and
    /// selects versions with [`StoreCandidateSource`] under the policy; the
    /// resulting [`ResolvedGraph`] already carries **real** store [`VersionId`]s,
    /// so it is persisted via the same CSR-build + `publish_graph` path
    /// `ingest_lockfile` uses. Returns the new [`RunId`].
    pub fn resolve_synthetic_from_roots(
        &mut self,
        eco: Ecosystem,
        roots: &[VersionId],
        policy: &str,
    ) -> Result<RunId> {
        self.resolve_synthetic_from_roots_detailed(eco, roots, policy)
            .map(|r| r.run)
    }

    /// Like [`Prism::resolve_synthetic_from_roots`] but returns node/edge counts.
    pub fn resolve_synthetic_from_roots_detailed(
        &mut self,
        eco: Ecosystem,
        roots: &[VersionId],
        policy: &str,
    ) -> Result<SyntheticResolve> {
        self.ensure_writable()?;
        let resolve_policy =
            policy_from_label(policy).map_err(|e| PrismError::Other(e.to_string()))?;

        // Build the store-backed engine inputs in a borrow scope so the immutable
        // store borrows end before we take the mutable store borrow to persist.
        let resolution = {
            let req_src = StoreRequirementSource::new(self.store());
            let mut cand_src = StoreCandidateSource::new(self.store());
            // The `locked` policy needs pins; seed them from the versions on hand.
            if matches!(resolve_policy, prism_resolve::ResolvePolicy::Locked) {
                cand_src.seed_locks_from_store(roots);
            }
            let engine = Engine::new(req_src, cand_src, resolve_policy, EnvironmentId::new(0))
                .with_budget(ResolveBudget::default());
            engine
                .resolve(roots)
                .map_err(|e| PrismError::Other(format!("synthetic resolve failed: {e}")))?
        };

        let label = format!("synthetic-{policy}");
        let detailed = self.persist_resolved_graph(eco, &resolution.graph, &label)?;
        Ok(detailed)
    }

    /// Build + publish a [`ResolvedGraph`] (real [`VersionId`]s) as a new run,
    /// returning its size. Shared by the from-roots path and the public
    /// [`Prism::resolve_synthetic`] convenience.
    fn persist_resolved_graph(
        &mut self,
        eco: Ecosystem,
        graph: &ResolvedGraph,
        resolver_name: &str,
    ) -> Result<SyntheticResolve> {
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
        let nodes = snapshot.node_count();
        let edges = snapshot.edge_count();
        let image = write_pcsr(&snapshot);
        self.store.publish_graph(run, &image)?;

        let fidelity = graph
            .fidelity
            .unwrap_or_else(|| GraphFidelity::exact(FidelityTier::ModelResolved));
        self.write_synthetic_run(run, eco.id(), resolver_name, fidelity)?;
        Ok(SyntheticResolve {
            run,
            nodes,
            edges,
            truncated: graph.truncated,
        })
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
