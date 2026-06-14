//! Pluggable byte-source seam used by [`Store`] to read its published files.
//!
//! [`FileSource`] is the read seam between the store's verification + index
//! logic and the *origin* of the bytes. Two concrete implementations exist:
//!
//! - [`FsFileSource`]: the disk-backed store. Every read becomes one
//!   [`crate::fsutil::read_file`] call against `<prism_dir>/<rel_path>`.
//! - [`BundleFileSource`]: an in-memory `.prismbundle` byte image. Reads are
//!   slices into a single shared `Arc<Vec<u8>>` plus one `to_vec` (one memcpy
//!   into a fresh `Vec`, no per-entry payload duplication held by the index).
//!
//! Integrity checks (length, digest) stay in the [`Store`] caller; this trait
//! is *just* the byte fetcher.
//!
//! ## Why this exists
//!
//! Before S2 the store called [`std::fs`] directly at six sites and had no
//! constructor for opening a bundle without first unpacking it to disk. Routing
//! reads through [`FileSource`] lets [`Store::open_in_memory`] hand the store a
//! workspace whose payloads live in WASM linear memory — exactly what the
//! browser-side WASM glue needs.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use prism_model::{PrismError, Result};

/// A read-only byte source addressed by paths relative to a workspace's
/// `.prism/` directory.
///
/// Implementations must be cheap to clone via `Arc` and safe to share across
/// threads — the store wraps them in `Arc<dyn FileSource>` and may hand them to
/// blob and segment helpers.
///
/// The `Send + Sync` bounds are required so [`Store`] keeps working under the
/// existing `Arc<Mutex<Prism>>` pattern. WASM itself is single-threaded; these
/// bounds are inherited but never exercised concurrently in the browser.
pub(crate) trait FileSource: Send + Sync {
    /// Read the whole file at `rel_path` (relative to `.prism/`) into a fresh
    /// `Vec<u8>`. Implementations must not panic on a missing or malformed
    /// path: surface a [`PrismError`] instead.
    fn read(&self, rel_path: &str) -> Result<Vec<u8>>;
}

/// Disk-backed source: reads `<prism_dir>/<rel_path>` via [`crate::fsutil`].
pub(crate) struct FsFileSource {
    pub prism_dir: PathBuf,
}

impl FileSource for FsFileSource {
    fn read(&self, rel: &str) -> Result<Vec<u8>> {
        crate::fsutil::read_file(&self.prism_dir.join(rel))
    }
}

/// In-memory bundle source: looks `rel_path` up in a TOC index and slices the
/// payload out of a shared `Arc<Vec<u8>>`.
///
/// One bundle byte image is held by exactly one `Arc`; cloning the source
/// (which the store does to share it with the blob helper) only bumps that
/// `Arc`'s ref-count. Each [`FileSource::read`] call performs one `to_vec`
/// (a single memcpy into a fresh `Vec`) — the store needs an owned buffer to
/// hand to manifest / index / segment parsers that take `Vec<u8>`.
pub(crate) struct BundleFileSource {
    /// The full `.prismbundle` byte image. Kept in one `Arc` so the bytes
    /// live in WASM linear memory until the workspace drops.
    pub bytes: Arc<Vec<u8>>,
    /// TOC index: `rel_path -> (payload_offset, payload_len)`. Offsets are
    /// absolute into `bytes`; `prism_bundle::read_summary` has already
    /// validated that every range falls inside the payload region.
    pub index: HashMap<String, (usize, usize)>,
}

impl FileSource for BundleFileSource {
    fn read(&self, rel: &str) -> Result<Vec<u8>> {
        let (off, len) = self
            .index
            .get(rel)
            .copied()
            .ok_or(PrismError::NotFound)?;
        // Defense in depth: even though prism_bundle validated every range,
        // we re-check here so a stray off-by-one in this crate cannot panic on
        // a slice out of bounds.
        let end = off
            .checked_add(len)
            .ok_or_else(|| PrismError::Corrupt {
                detail: format!("bundle entry {rel}: offset + len overflows usize"),
            })?;
        if end > self.bytes.len() {
            return Err(PrismError::OutOfBounds {
                detail: format!(
                    "bundle entry {rel}: end {end} exceeds bundle length {}",
                    self.bytes.len()
                ),
            });
        }
        Ok(self.bytes[off..end].to_vec())
    }
}
