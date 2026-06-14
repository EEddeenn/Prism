//! Content-addressed blob store (architecture doc, *Blob Store*).
//!
//! Blobs hold raw evidence and large/rarely-queried payloads, addressed by the
//! SHA-256 of their bytes. The caller is responsible for redaction *before*
//! handing bytes here: [`BlobStore::put`] hashes exactly the bytes it is given,
//! so the no-secrets-in-blobs invariant holds as long as inputs are pre-redacted.
//!
//! Integrity is verified on read by re-hashing. Garbage collection is an explicit
//! mark-and-sweep against a live set, run only by the single writer.

use crate::file_source::FileSource;
use crate::fsutil;
use prism_codec::sha256;
use prism_model::{BlobId, HashDigest, PrismError, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A content-addressed blob store rooted at a `blobs/` directory.
///
/// `dir` is the absolute on-disk path used by mutating operations (put / gc /
/// iter_blob_ids); `file_source` is the read seam used by `get`. For a
/// disk-backed store both point at the same `<.prism>/blobs/` tree; for a
/// bundle-backed (read-only) store `dir` is a sentinel that never gets touched
/// because bundles do not include blobs.
///
/// `read_only` mirrors the parent [`Store`]'s read-only flag: when true,
/// every writing or filesystem-probing entry point fails fast (or returns
/// an empty / negative answer for the read-only-friendly probes). This
/// keeps a bundle-backed store from poking the CWD-relative
/// `.prism/blobs/...` of whatever directory the WASM host or CLI happened
/// to launch from.
pub struct BlobStore {
    dir: PathBuf,
    file_source: Arc<dyn FileSource>,
    read_only: bool,
}

impl BlobStore {
    /// Open (or lazily create) a blob store at `blobs_dir`, reading through
    /// `file_source` for `get`. `read_only=true` rejects every mutation and
    /// every direct-filesystem probe (`has` / `iter_blob_ids` / `gc`).
    pub(crate) fn new(
        blobs_dir: impl Into<PathBuf>,
        file_source: Arc<dyn FileSource>,
        read_only: bool,
    ) -> Self {
        Self {
            dir: blobs_dir.into(),
            file_source,
            read_only,
        }
    }

    fn ensure_writable(&self) -> Result<()> {
        if self.read_only {
            return Err(PrismError::Other(
                "blob store is read-only (bundle-backed or read-only workspace)".into(),
            ));
        }
        Ok(())
    }

    fn path_for(&self, id: &BlobId) -> PathBuf {
        self.dir.join(id.shard_path())
    }

    /// Relative path under `.prism/` for blob `id` (`blobs/sha256/<ab>/<hex>`).
    fn rel_for(id: &BlobId) -> String {
        format!("blobs/{}", id.shard_path())
    }

    /// Store `bytes` (already redacted) and return its content address. Idempotent:
    /// re-storing identical bytes is a no-op. Refused on a read-only store.
    pub fn put(&self, bytes: &[u8]) -> Result<BlobId> {
        self.ensure_writable()?;
        let id = BlobId(sha256(bytes));
        let path = self.path_for(&id);
        if path.exists() {
            return Ok(id);
        }
        if let Some(parent) = path.parent() {
            fsutil::ensure_dir(parent)?;
        }
        fsutil::atomic_write(&path, bytes)?;
        Ok(id)
    }

    /// Read blob `id`, verifying its content address by re-hashing.
    pub fn get(&self, id: &BlobId) -> Result<Vec<u8>> {
        let bytes = self
            .file_source
            .read(&Self::rel_for(id))
            .map_err(|_| PrismError::NotFound)?;
        if &sha256(&bytes) != id.digest() {
            return Err(PrismError::ChecksumMismatch {
                detail: format!("blob {}", id.digest()),
            });
        }
        Ok(bytes)
    }

    /// True if blob `id` is present (without reading/verifying it). Always
    /// false on a read-only store — bundles do not carry blobs, and we must
    /// not probe a CWD-relative `.prism/blobs/...` from a bundle-backed
    /// store opened in an arbitrary directory.
    pub fn has(&self, id: &BlobId) -> bool {
        if self.read_only {
            return false;
        }
        self.path_for(id).exists()
    }

    /// Mark-and-sweep: delete every stored blob whose id is not in `live`.
    /// Returns the number of blobs removed. Run only by the single writer.
    /// Refused on a read-only store.
    pub fn gc(&self, live: &HashSet<BlobId>) -> Result<usize> {
        self.ensure_writable()?;
        let mut removed = 0;
        for id in self.iter_blob_ids()? {
            if !live.contains(&id) {
                let _ = std::fs::remove_file(self.path_for(&id));
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Enumerate the ids of all stored blobs (parsed from their shard
    /// filenames). Returns the empty list on a read-only store — same
    /// reasoning as [`BlobStore::has`].
    pub fn iter_blob_ids(&self) -> Result<Vec<BlobId>> {
        if self.read_only {
            return Ok(Vec::new());
        }
        let sha_dir = self.dir.join("sha256");
        let mut out = Vec::new();
        if !sha_dir.exists() {
            return Ok(out);
        }
        for shard in read_dir_sorted(&sha_dir)? {
            if !shard.is_dir() {
                continue;
            }
            for file in read_dir_sorted(&shard)? {
                if let Some(name) = file.file_name().and_then(|n| n.to_str()) {
                    if let Some(id) = parse_hex_blob_id(name) {
                        out.push(id);
                    }
                }
            }
        }
        Ok(out)
    }
}

fn read_dir_sorted(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        paths.push(entry?.path());
    }
    paths.sort();
    Ok(paths)
}

/// Parse a 64-char lowercase-hex blob filename into a [`BlobId`].
fn parse_hex_blob_id(name: &str) -> Option<BlobId> {
    if name.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        let hi = (name.as_bytes()[i * 2] as char).to_digit(16)?;
        let lo = (name.as_bytes()[i * 2 + 1] as char).to_digit(16)?;
        *b = ((hi << 4) | lo) as u8;
    }
    Some(BlobId(HashDigest::from_bytes(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_source::FsFileSource;
    use crate::test_support::TempDir;

    fn make_store(td: &TempDir) -> BlobStore {
        // Disk-backed FileSource rooted at `<td>/`: the BlobStore writes to
        // `<td>/blobs/sha256/...` and reads via FileSource(rel="blobs/sha256/...").
        let fs: Arc<dyn FileSource> = Arc::new(FsFileSource {
            prism_dir: td.path().to_path_buf(),
        });
        BlobStore::new(td.path().join("blobs"), fs, false)
    }

    #[test]
    fn put_get_roundtrip_and_dedup() {
        let td = TempDir::new("blob");
        let bs = make_store(&td);
        let id1 = bs.put(b"raw registry response").unwrap();
        let id2 = bs.put(b"raw registry response").unwrap();
        assert_eq!(id1, id2); // content-addressed: identical bytes -> same id
        assert!(bs.has(&id1));
        assert_eq!(bs.get(&id1).unwrap(), b"raw registry response");
    }

    #[test]
    fn missing_blob_is_not_found() {
        let td = TempDir::new("blob-missing");
        let bs = make_store(&td);
        let id = BlobId(sha256(b"never stored"));
        assert!(!bs.has(&id));
        assert!(matches!(bs.get(&id), Err(PrismError::NotFound)));
    }

    #[test]
    fn corrupted_blob_fails_integrity_check() {
        let td = TempDir::new("blob-corrupt");
        let bs = make_store(&td);
        let id = bs.put(b"trust me").unwrap();
        let path = td.path().join("blobs").join(id.shard_path());
        std::fs::write(&path, b"tampered!").unwrap();
        assert!(matches!(
            bs.get(&id),
            Err(PrismError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn gc_sweeps_unreferenced() {
        let td = TempDir::new("blob-gc");
        let bs = make_store(&td);
        let live_id = bs.put(b"keep me").unwrap();
        let dead_id = bs.put(b"collect me").unwrap();
        assert_eq!(bs.iter_blob_ids().unwrap().len(), 2);

        let live: HashSet<BlobId> = [live_id].into_iter().collect();
        assert_eq!(bs.gc(&live).unwrap(), 1);
        assert!(bs.has(&live_id));
        assert!(!bs.has(&dead_id));
    }

    #[test]
    fn hex_filename_roundtrip() {
        let id = BlobId(sha256(b"x"));
        let hex = id.digest().to_hex();
        assert_eq!(parse_hex_blob_id(&hex), Some(id));
        assert_eq!(parse_hex_blob_id("short"), None);
    }

    #[test]
    fn read_only_store_rejects_writes_and_probes() {
        // A read-only BlobStore (the shape a bundle-backed `Store` produces)
        // must refuse `put` and `gc` and must NOT poke the filesystem in
        // `has` / `iter_blob_ids` — its `dir` is a CWD-relative sentinel
        // because bundles do not include blobs.
        let td = TempDir::new("blob-ro");
        let fs: Arc<dyn FileSource> = Arc::new(FsFileSource {
            prism_dir: td.path().to_path_buf(),
        });
        // Sentinel-like `""` blobs dir mirrors what `Store::open_in_memory`
        // hands BlobStore via Layout::new("").
        let bs = BlobStore::new(PathBuf::from(""), fs, true);

        let put_err = bs.put(b"data").unwrap_err();
        assert!(matches!(put_err, PrismError::Other(m) if m.contains("read-only")));
        let gc_err = bs.gc(&HashSet::new()).unwrap_err();
        assert!(matches!(gc_err, PrismError::Other(m) if m.contains("read-only")));
        assert_eq!(bs.iter_blob_ids().unwrap(), Vec::<BlobId>::new());
        let id = BlobId(sha256(b"absent"));
        assert!(!bs.has(&id));
        // `get` goes through FileSource and would just NotFound — exercised by
        // the bundle-backed integration tests, not here.
    }
}
