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

use prism_graph::GraphSnapshot;
use prism_model::{FiltrationId, GraphEdgeId, GraphNodeId, PairId, PrismError, Result, RunId};
use prism_tda::{
    depth_filtration, read_ptda, write_ptda, Complex, ObjectType, PersistenceResult, Ptda,
};

use crate::tda_views::{BarcodeDim, BarcodeView, SupportObject, SupportView};
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
        if !matches!(filtration, "depth" | "" | "default") {
            return Err(PrismError::Other(format!(
                "unsupported filtration {filtration:?} (only \"depth\" in v0.1)"
            )));
        }
        // Load the snapshot, build the filtered complex, compute homology.
        let g = self.load_graph(run)?;
        let filt = depth_filtration(&g)?;
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

    // --- internal -----------------------------------------------------------

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
