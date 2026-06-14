//! The high-level Prism facade (architecture doc, *Rust API Design*).
//!
//! [`Prism`] wraps a [`prism_store::Store`] plus the offline plugin registry
//! (Cargo + npm) and exposes the end-to-end vertical slice the CLI and bindings
//! call into:
//!
//! - [`Prism::init`] / [`Prism::open`] — create or open a workspace.
//! - [`Prism::ingest_lockfile`] — detect the ecosystem by filename, parse the
//!   lockfile to a [`prism_plugin_api::ResolvedGraph`], intern each occurrence's
//!   real `(name, version)` into store ids, build and persist a CSR
//!   [`prism_graph::GraphSnapshot`], and record a `LockfileExact` [`ResolverRun`]
//!   plus the `Package`/`PackageVersion` records.
//! - [`Prism::ingest_manifest`] — parse a manifest into `Package`/
//!   `PackageVersion`/`Requirement` records and indexes.
//! - Graph queries over a [`RunId`]: [`Prism::graph_stats`],
//!   [`Prism::neighborhood`], [`Prism::impact`], [`Prism::scc`],
//!   [`Prism::duplicate_clusters`].
//! - TDA: [`Prism::tda_run`] persists a `.ptda` and returns a barcode summary;
//!   [`Prism::tda_support`] maps a pair's generators back to graph nodes/edges.
//! - Candidate analysis: [`Prism::candidates_for`] / [`Prism::candidates`] compute
//!   ordinal candidate ranges over a package's sorted versions, and
//!   [`Prism::conflict_pressure`] reports same-package requirement conflicts and a
//!   tightness score.
//! - Synthetic resolution: [`Prism::resolve_manifest`] /
//!   [`Prism::resolve_synthetic_from_roots`] run `prism_resolve::Engine` over
//!   store-backed adapters and persist a `ModelResolved` run;
//!   [`Prism::resolve_synthetic`] persists a caller-produced `ResolvedGraph`.
//!
//! Untrusted bytes never panic: every parse/lookup failure surfaces as a
//! [`prism_model::PrismError`].

mod candidate_views;
mod candidates;
mod ci_topology;
mod clusters;
mod duplicate_rootcause;
mod ecosystem;
mod fidelity_notes;
mod ingest;
mod parse;
mod query;
mod raw;
mod overlap_views;
mod overlay_views;
mod resolve;
mod resolve_store;
mod reuse_views;
mod risk_views;
mod run_diff;
mod structure_views;
mod tda;
mod tda_views;
mod views;

#[cfg(test)]
mod test_util;

pub use candidate_views::{
    CandidateExclusion, CandidateVersion, CandidateView, ConflictPair, ConflictReportView,
};
pub use ci_topology::{
    BudgetReport, BudgetViolation, MetricDelta, ShapeSignature, TopologyDistanceView,
};
pub use clusters::{Subsystem, WorkspaceRegion};
pub use duplicate_rootcause::{
    ConstraintIntroducer, DuplicateRootCauseView, DuplicateVersionCause,
};
pub use ecosystem::{Ecosystem, CARGO_ECOSYSTEM_ID, NPM_ECOSYSTEM_ID};
pub use fidelity_notes::{FidelityNote, FidelityReport};
pub use ingest::{LockfileIngest, ManifestIngest};
pub use overlap_views::{OverlapPair, OverlapView, SharedSurface};
pub use overlay_views::{OverlayFinding, OverlayReportView};
pub use resolve::SyntheticResolve;
pub use reuse_views::{DiamondNode, DiamondReuseView, MinimalEdge, MinimalView};
pub use risk_views::{FragmentationView, IslandView, RiskDimension, RiskProfileView};
pub use run_diff::{CycleDiffEntry, HotspotShift, RunDiffView, ScopeShift};
pub use structure_views::{
    kind_label, ChokepointView, CycleView, ExplanationView, IntroducerView, ScopeSummaryView,
};
pub use tda::FiltrationSummary;
pub use tda_views::{BarcodeDim, BarcodeView, PairView, SupportObject, SupportView};
pub use views::{
    DuplicateClusterView, GraphEdgeView, GraphNodeView, GraphStatsView, NeighborhoodView, SccView,
};

use std::path::Path;

use prism_graph::{read_pcsr, GraphSnapshot};
use prism_model::{Result, RunId};
use prism_store::Store;

/// The Prism facade: a [`Store`] plus the offline plugin registry.
///
/// Holds the workspace write lock for its lifetime (via the inner [`Store`]), so
/// only one `Prism` may be open per workspace at a time.
pub struct Prism {
    store: Store,
}

impl Prism {
    /// Initialize a brand-new workspace at `path`, creating `.prism/`.
    ///
    /// Fails if a workspace already exists there (mirrors [`Store::init`]).
    pub fn init(path: impl AsRef<Path>) -> Result<Prism> {
        Ok(Prism {
            store: Store::init(path)?,
        })
    }

    /// Open an existing workspace at `path`, acquiring the write lock and
    /// verifying the published file set.
    pub fn open(path: impl AsRef<Path>) -> Result<Prism> {
        Ok(Prism {
            store: Store::open(path)?,
        })
    }

    /// Open an existing workspace at `path` **read-only**: take no write lock and
    /// reject every mutation (ingest/resolve/tda). Multiple read-only opens — and
    /// a concurrent writer — can coexist, since the published file set is immutable
    /// and content-checksummed.
    ///
    /// This is the right entry point for a viewer such as `prism-serve`: it must
    /// not block the CLI, and a crashed viewer must never strand a stale lock.
    pub fn open_readonly(path: impl AsRef<Path>) -> Result<Prism> {
        Ok(Prism {
            store: Store::open_readonly(path)?,
        })
    }

    /// True if this facade was opened read-only (see [`Prism::open_readonly`]).
    pub fn is_read_only(&self) -> bool {
        self.store.is_read_only()
    }

    /// Reject a mutating operation on a read-only workspace at the facade boundary,
    /// BEFORE any side effect (e.g. a content-addressed blob write happens before
    /// the store's own commit-time guard). Every ingest/resolve/tda entry point
    /// calls this first.
    pub(crate) fn ensure_writable(&self) -> Result<()> {
        if self.store.is_read_only() {
            return Err(prism_model::PrismError::Other(
                "workspace is read-only; this operation requires the write lock".into(),
            ));
        }
        Ok(())
    }

    /// The underlying store (read-only borrow), for advanced callers.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The underlying store (mutable borrow), for advanced callers.
    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    /// Detect the ecosystem that owns `path` from its file name.
    pub fn detect(path: impl AsRef<Path>) -> Option<Ecosystem> {
        Ecosystem::detect(path.as_ref())
    }

    // --- Internal helpers shared by the query/TDA paths. ---------------------

    /// Open the persisted CSR snapshot for `run` (`open_graph` → `read_pcsr`).
    pub(crate) fn load_graph(&self, run: RunId) -> Result<GraphSnapshot> {
        let bytes = self.store.open_graph(run)?;
        read_pcsr(&bytes)
    }

    /// Best-effort display label for a version: its interned purl, else a
    /// synthetic `version:<id>` placeholder so output is never empty.
    pub(crate) fn version_label(&self, version: prism_model::VersionId) -> String {
        if let Ok(v) = self.store.get_version(version) {
            if let Some(purl) = self.store.resolve_string(v.purl) {
                return purl;
            }
        }
        format!("version:{}", version.get())
    }

    /// The package family of a version, if the store knows it.
    pub(crate) fn package_of(&self, version: prism_model::VersionId) -> Option<prism_model::PackageId> {
        self.store.get_version(version).ok().map(|v| v.package_id)
    }
}

#[cfg(test)]
mod end_to_end_tests;
