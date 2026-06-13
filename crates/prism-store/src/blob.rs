//! Content-addressed blob store (architecture doc, *Blob Store*).
//!
//! Blobs hold raw evidence and large/rarely-queried payloads, addressed by the
//! SHA-256 of their bytes. The caller is responsible for redaction *before*
//! handing bytes here: [`BlobStore::put`] hashes exactly the bytes it is given,
//! so the no-secrets-in-blobs invariant holds as long as inputs are pre-redacted.
//!
//! Integrity is verified on read by re-hashing. Garbage collection is an explicit
//! mark-and-sweep against a live set, run only by the single writer.

use crate::fsutil;
use prism_codec::sha256;
use prism_model::{BlobId, HashDigest, PrismError, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A content-addressed blob store rooted at a `blobs/` directory.
pub struct BlobStore {
    dir: PathBuf,
}

impl BlobStore {
    /// Open (or lazily create) a blob store at `blobs_dir`.
    pub fn new(blobs_dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: blobs_dir.into(),
        }
    }

    fn path_for(&self, id: &BlobId) -> PathBuf {
        self.dir.join(id.shard_path())
    }

    /// Store `bytes` (already redacted) and return its content address. Idempotent:
    /// re-storing identical bytes is a no-op.
    pub fn put(&self, bytes: &[u8]) -> Result<BlobId> {
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
        let path = self.path_for(id);
        let bytes = fsutil::read_file(&path).map_err(|_| PrismError::NotFound)?;
        if &sha256(&bytes) != id.digest() {
            return Err(PrismError::ChecksumMismatch {
                detail: format!("blob {}", id.digest()),
            });
        }
        Ok(bytes)
    }

    /// True if blob `id` is present (without reading/verifying it).
    pub fn has(&self, id: &BlobId) -> bool {
        self.path_for(id).exists()
    }

    /// Mark-and-sweep: delete every stored blob whose id is not in `live`.
    /// Returns the number of blobs removed. Run only by the single writer.
    pub fn gc(&self, live: &HashSet<BlobId>) -> Result<usize> {
        let mut removed = 0;
        for id in self.iter_blob_ids()? {
            if !live.contains(&id) {
                let _ = std::fs::remove_file(self.path_for(&id));
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Enumerate the ids of all stored blobs (parsed from their shard filenames).
    pub fn iter_blob_ids(&self) -> Result<Vec<BlobId>> {
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
    use crate::test_support::TempDir;

    #[test]
    fn put_get_roundtrip_and_dedup() {
        let td = TempDir::new("blob");
        let bs = BlobStore::new(td.path().join("blobs"));
        let id1 = bs.put(b"raw registry response").unwrap();
        let id2 = bs.put(b"raw registry response").unwrap();
        assert_eq!(id1, id2); // content-addressed: identical bytes -> same id
        assert!(bs.has(&id1));
        assert_eq!(bs.get(&id1).unwrap(), b"raw registry response");
    }

    #[test]
    fn missing_blob_is_not_found() {
        let td = TempDir::new("blob-missing");
        let bs = BlobStore::new(td.path().join("blobs"));
        let id = BlobId(sha256(b"never stored"));
        assert!(!bs.has(&id));
        assert!(matches!(bs.get(&id), Err(PrismError::NotFound)));
    }

    #[test]
    fn corrupted_blob_fails_integrity_check() {
        let td = TempDir::new("blob-corrupt");
        let bs = BlobStore::new(td.path().join("blobs"));
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
        let bs = BlobStore::new(td.path().join("blobs"));
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
}
