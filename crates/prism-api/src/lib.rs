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
//! - [`Prism::resolve_synthetic`] — a clearly-marked thin wrapper over
//!   `prism_resolve::Engine` (the solid path in v0.1 is lockfile ingestion).
//!
//! Untrusted bytes never panic: every parse/lookup failure surfaces as a
//! [`prism_model::PrismError`].

mod ecosystem;
mod ingest;
mod parse;
mod query;
mod raw;
mod resolve;
mod tda;
mod tda_views;
mod views;

#[cfg(test)]
mod test_util;

pub use ecosystem::{Ecosystem, CARGO_ECOSYSTEM_ID, NPM_ECOSYSTEM_ID};
pub use ingest::{LockfileIngest, ManifestIngest};
pub use tda_views::{BarcodeDim, BarcodeView, SupportObject, SupportView};
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
