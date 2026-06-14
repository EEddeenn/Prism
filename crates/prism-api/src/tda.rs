//! TDA over a persisted run (architecture doc, *TDA Architecture*).
//!
//! [`Prism::tda_run`] builds the depth filtration of a run's CSR snapshot,
//! computes persistent homology, persists the `.ptda` via [`Store::publish_tda`],
//! and returns a [`BarcodeView`] (per-dimension birth/death counts). The pipeline
//! is exactly the one `prism-tda` documents:
//! `depth_filtration → Complex::build → PersistenceResult::compute → write_ptda`.
//!
//! [`Prism::tda_support`] reads back a persisted `.ptda` and maps one pair's
//! generator chain to graph nodes/edges, resolving each into a display object.

use std::collections::{BTreeMap, HashMap};

use prism_graph::{
    forward_reachable, roots as graph_roots, EdgeKindMask, GraphSnapshot, TraversalOptions,
    PRODUCTION_KINDS,
};
use prism_model::{FiltrationId, GraphEdgeId, GraphNodeId, PairId, PrismError, Result, RunId};
use prism_tda::{
    depth_filtration, read_ptda, write_ptda, Complex, Filtration, FiltrationType, ObjectType,
    PersistenceResult, Ptda,
};

use crate::tda_views::{BarcodeDim, BarcodeView, PairView, SupportObject, SupportView};
use crate::Prism;

impl Prism {
    /// Compute and persist a TDA result for `run` under the named `filtration`,
    /// returning a barcode summary.
    ///
    /// Only `"depth"` is supported in v0.1 (it is also the default); any other
    /// name is a [`PrismError::Other`]. The `.ptda` is stored via
    /// [`Store::publish_tda`] keyed by a freshly-allocated, store-global
    /// [`FiltrationId`].
    pub fn tda_run(&mut self, run: RunId, filtration: &str) -> Result<BarcodeView> {
        self.ensure_writable()?;
        // Load the snapshot, build the filtered complex, compute homology.
        let g = self.load_graph(run)?;
        let filt = self.build_filtration(&g, filtration)?;
        let complex = Complex::build(&g, filt.values())?;

        // A fresh, store-global id for this filtration (the object counter is the
        // single monotonic source the store persists; it never reuses ids).
        let filtration_id = FiltrationId::new(self.store.alloc_run_id().get());
        // PairIds must be globally unique across filtrations: pack the filtration
        // id into the high 32 bits and the in-file pair ordinal into the low 32, so
        // each filtration owns a disjoint 2^32 block (the old base = filtration_id
        // incremented by 1, so consecutive filtrations' ranges overlapped).
        // Filtration ids and per-file pair counts each fit in 32 bits in practice.
        let pair_id_base = filtration_id.get() << 32;
        let persistence = PersistenceResult::compute(&complex, pair_id_base)?;

        let barcode = barcode_from(run, filtration_id, &persistence);

        // Serialize and persist the .ptda image.
        let ptda = Ptda::new(filtration_id, run, &filt, &complex, persistence);
        let image = write_ptda(&ptda);
        self.store.publish_tda(filtration_id, run, &image)?;

        Ok(barcode)
    }

    /// The filtration ids with a persisted `.ptda` for `run`.
    pub fn tda_filtrations(&self, run: RunId) -> Vec<FiltrationId> {
        self.store.tda_filtrations_for_run(run)
    }

    /// Read back the persisted barcode summary for `run`'s `filtration`.
    pub fn tda_barcode(&self, run: RunId, filtration: FiltrationId) -> Result<BarcodeView> {
        let doc = self.load_ptda(run, filtration)?;
        Ok(barcode_from(run, filtration, &doc.persistence()))
    }

    /// List the individual persistence pairs of `run`'s `filtration`.
    ///
    /// Where [`Prism::tda_barcode`] returns only per-dimension *counts*, this
    /// returns one [`PairView`] per bar (id, dimension, birth/death values) so a
    /// caller can present a feature list and then resolve a chosen pair's
    /// generators with [`Prism::tda_support`]. Pairs are returned in the `.ptda`'s
    /// stored emission order (H0 finite, H0 essential, H1 …).
    pub fn tda_pairs(&self, run: RunId, filtration: FiltrationId) -> Result<Vec<PairView>> {
        let doc = self.load_ptda(run, filtration)?;
        let result = doc.persistence();
        Ok(result
            .pairs()
            .iter()
            .map(|p| PairView {
                pair: p.pair_id,
                dim: p.dim,
                birth: p.birth_value,
                death: p.death_value,
            })
            .collect())
    }

    /// Map a persistence pair's generator support to graph nodes/edges.
    ///
    /// `run`/`filtration` identify the persisted `.ptda`; `pair` selects one of
    /// its pairs. Each generator entry is resolved into a [`SupportObject`]
    /// (node view, or edge with endpoints). Returns [`PrismError::NotFound`] if
    /// the pair is absent.
    pub fn tda_support(
        &self,
        run: RunId,
        filtration: FiltrationId,
        pair: PairId,
    ) -> Result<SupportView> {
        let doc = self.load_ptda(run, filtration)?;
        let result = doc.persistence();
        let dim = result
            .pairs()
            .iter()
            .find(|p| p.pair_id == pair)
            .map(|p| p.dim)
            .ok_or(PrismError::NotFound)?;

        let g = self.load_graph(run)?;
        // Pre-index every edge id -> (from, to) once for endpoint lookups.
        let endpoints = edge_endpoints(&g)?;

        let mut objects = Vec::new();
        for gen in result.generators_for(pair) {
            match gen.object_type {
                ObjectType::Node => {
                    // Defensive: a generator that names no real node (a legacy or
                    // corrupt `.ptda`, or an id past this snapshot) is skipped
                    // rather than failing the whole request. A correct `.ptda`
                    // never hits this.
                    if gen.object_id >= g.node_count() as u64 {
                        continue;
                    }
                    let node = GraphNodeId::new(gen.object_id as u32);
                    objects.push(SupportObject::Node(self.node_view_for(&g, node)?));
                }
                ObjectType::Edge => {
                    let eid = GraphEdgeId::new(gen.object_id as u32);
                    let (source, target) = endpoints
                        .get(&gen.object_id)
                        .copied()
                        .unwrap_or((GraphNodeId::NULL, GraphNodeId::NULL));
                    objects.push(SupportObject::Edge {
                        id: eid,
                        source,
                        target,
                    });
                }
                // v0.1 only emits Node/Edge generators; richer types are skipped.
                _ => {}
            }
        }
        Ok(SupportView {
            pair,
            dim,
            objects,
        })
    }

    /// Compare a run's topology under several filtration **lenses**.
    ///
    /// The underlying 1-complex — and therefore the H₀/H₁ Betti numbers — is the
    /// same under every filtration; what changes is the *persistence* (how
    /// long-lived each feature is). So this reports, per lens, the total and
    /// maximum persistence of the finite bars: under `depth`, deep-tree loops are
    /// long-lived; under `centrality`, loops among high-dependent hubs are; under
    /// `scope`, production structure is born before non-production structure.
    pub fn tda_compare(&self, run: RunId) -> Result<Vec<FiltrationSummary>> {
        let g = self.load_graph(run)?;
        let mut out = Vec::new();
        for name in ["depth", "centrality", "scope"] {
            let filt = self.build_filtration(&g, name)?;
            let complex = Complex::build(&g, filt.values())?;
            let persistence = PersistenceResult::compute(&complex, 0)?;
            let (mut h0_finite, mut h1_finite, mut h0_ess, mut h1_ess) = (0u32, 0u32, 0u32, 0u32);
            let mut total_persistence = 0.0f64;
            let mut max_persistence = 0.0f64;
            for p in persistence.pairs() {
                let essential = p.death_cell.is_none();
                match (p.dim, essential) {
                    (0, true) => h0_ess += 1,
                    (0, false) => h0_finite += 1,
                    (1, true) => h1_ess += 1,
                    (1, false) => h1_finite += 1,
                    _ => {}
                }
                if let Some(death) = p.death_value {
                    let life = death - p.birth_value;
                    if life > 0.0 {
                        total_persistence += life;
                        if life > max_persistence {
                            max_persistence = life;
                        }
                    }
                }
            }
            out.push(FiltrationSummary {
                filtration: name,
                h0_components: h0_ess,
                h1_loops: h1_ess + h1_finite,
                finite_bars: h0_finite + h1_finite,
                total_persistence,
                max_persistence,
            });
        }
        Ok(out)
    }

    // --- internal -----------------------------------------------------------

    /// Build the named filtration scalar over `g`.
    ///
    /// `depth` (distance from roots), `centrality` (−direct-dependents, so hubs
    /// are born first), and `scope` (0 for production-reachable, 1 otherwise) are
    /// supported; an unknown name is a [`PrismError::Other`].
    fn build_filtration(&self, g: &GraphSnapshot, name: &str) -> Result<Filtration> {
        match name {
            "depth" | "" | "default" => depth_filtration(g),
            "centrality" => {
                let mut values = Vec::with_capacity(g.node_count());
                for i in 0..g.node_count() as u32 {
                    let node = GraphNodeId::new(i);
                    // Distinct direct dependents (collapse parallel edges).
                    let mut preds: Vec<u32> = g.in_neighbors(node)?.iter().map(|p| p.get()).collect();
                    preds.sort_unstable();
                    preds.dedup();
                    values.push(-(preds.len() as f64));
                }
                Ok(Filtration::new(FiltrationType::Centrality, values))
            }
            "scope" => {
                let prod = production_reachable(g)?;
                let values = prod.iter().map(|&r| if r { 0.0 } else { 1.0 }).collect();
                Ok(Filtration::new(FiltrationType::Scope, values))
            }
            other => Err(PrismError::Other(format!(
                "unsupported filtration {other:?} (use depth, centrality, or scope)"
            ))),
        }
    }

    /// Read back and parse the `.ptda` for `(run, filtration)`.
    fn load_ptda(&self, run: RunId, filtration: FiltrationId) -> Result<Ptda> {
        let bytes = self.store.open_tda(filtration, run)?;
        read_ptda(&bytes)
    }

    /// Resolve a node into a display view (shared with the query path's logic).
    fn node_view_for(&self, g: &GraphSnapshot, node: GraphNodeId) -> Result<crate::views::GraphNodeView> {
        let version = g.node_version(node)?;
        Ok(crate::views::GraphNodeView {
            id: node,
            version,
            package: self.package_of(version),
            purl: self.version_label(version),
            depth: g.node_depth(node)?,
        })
    }
}

/// A run's topology summary under one filtration lens.
#[derive(Clone, Debug, PartialEq)]
pub struct FiltrationSummary {
    /// The filtration name (`depth`/`centrality`/`scope`).
    pub filtration: &'static str,
    /// H₀ Betti number (connected components) — invariant across lenses.
    pub h0_components: u32,
    /// H₁ Betti number (independent loops) — invariant across lenses.
    pub h1_loops: u32,
    /// Number of finite (born-and-died) bars under this lens.
    pub finite_bars: u32,
    /// Total persistence (Σ death−birth over finite bars) under this lens.
    pub total_persistence: f64,
    /// Longest single bar's persistence under this lens.
    pub max_persistence: f64,
}

/// Production-reachable mask: nodes reachable from any root along production
/// ([`PRODUCTION_KINDS`]) edges, including the roots themselves.
fn production_reachable(g: &GraphSnapshot) -> Result<Vec<bool>> {
    let mut reached = vec![false; g.node_count()];
    let mask = EdgeKindMask::from_kinds(PRODUCTION_KINDS);
    for root in graph_roots(g)? {
        let opts = TraversalOptions {
            kinds: mask,
            include_root: true,
            ..Default::default()
        };
        for nd in forward_reachable(g, root, opts)?.nodes {
            reached[nd.get() as usize] = true;
        }
    }
    Ok(reached)
}

/// Build a [`BarcodeView`] of per-dimension finite/essential counts.
fn barcode_from(run: RunId, filtration: FiltrationId, result: &PersistenceResult) -> BarcodeView {
    // dim -> (finite, essential)
    let mut by_dim: BTreeMap<u8, (usize, usize)> = BTreeMap::new();
    for p in result.pairs() {
        let entry = by_dim.entry(p.dim).or_insert((0, 0));
        if p.death_cell.is_none() {
            entry.1 += 1;
        } else {
            entry.0 += 1;
        }
    }
    let dims = by_dim
        .into_iter()
        .map(|(dim, (finite, essential))| BarcodeDim {
            dim,
            finite,
            essential,
        })
        .collect();
    BarcodeView {
        run,
        filtration,
        dims,
    }
}

/// Index every dense edge id to its `(from, to)` endpoints from the forward CSR.
fn edge_endpoints(g: &GraphSnapshot) -> Result<HashMap<u64, (GraphNodeId, GraphNodeId)>> {
    let mut map = HashMap::new();
    for i in 0..g.node_count() as u32 {
        let from = GraphNodeId::new(i);
        let targets = g.out_neighbors(from)?;
        let edge_ids = g.out_edge_ids(from)?;
        for (k, &to) in targets.iter().enumerate() {
            map.insert(edge_ids[k].get() as u64, (from, to));
        }
    }
    Ok(map)
}
