//! Prism's purpose-built local store.
//!
//! This crate owns the on-disk workspace: append-only object segments, the
//! atomically-published manifest, and (in later milestones) typed indexes, the
//! blob store, and compaction. The unit of durability is a batch: records and the
//! files that reference them are staged in `tmp/`, fsync'd, renamed into place,
//! and then published by a single atomic manifest swap.
//!
//! See the architecture design document, *Storage Architecture* and *Crash Safety*.

pub mod fsutil;
pub mod lock;
pub mod manifest;
pub mod paths;
pub mod segment;

#[cfg(test)]
mod test_support;

pub use lock::WorkspaceLock;
pub use manifest::{FileEntry, FileKind, Manifest};
pub use paths::Layout;
pub use segment::{SegmentReader, SegmentWriter};

use prism_codec::sha256;
use prism_model::{PrismError, Result};
use std::path::Path;

/// A record to be written into a segment during a publish. `logical_ts` is
/// assigned by the store from the recovered high-water mark.
#[derive(Clone, Debug)]
pub struct RawRecord {
    pub record_type: u16,
    pub flags: u16,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

impl RawRecord {
    /// Convenience constructor.
    pub fn new(record_type: u16, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Self {
        Self {
            record_type,
            flags: 0,
            key: key.into(),
            value: value.into(),
        }
    }
}

/// An open Prism store. Holds the workspace write lock for its lifetime.
pub struct Store {
    layout: Layout,
    manifest: Manifest,
    _lock: WorkspaceLock,
}

impl Store {
    /// Initialize a new workspace at `root`, creating `.prism/` and publishing an
    /// empty manifest. Fails if a workspace already exists there.
    pub fn init(root: impl AsRef<Path>) -> Result<Store> {
        let layout = Layout::new(root.as_ref());
        if layout.manifest().exists() {
            return Err(PrismError::Other(format!(
                "workspace already initialized at {}",
                layout.prism_dir().display()
            )));
        }
        fsutil::ensure_dir(&layout.prism_dir())?;
        for dir in layout.all_dirs() {
            fsutil::ensure_dir(&dir)?;
        }
        let lock = WorkspaceLock::acquire(&layout.lock())?;
        let manifest = Manifest::empty();
        fsutil::atomic_write(&layout.manifest(), &manifest.encode())?;
        Ok(Store {
            layout,
            manifest,
            _lock: lock,
        })
    }

    /// Open an existing workspace at `root`: acquire the lock, load and verify the
    /// manifest's file set, and clean any stale `tmp/` staging files.
    pub fn open(root: impl AsRef<Path>) -> Result<Store> {
        let layout = Layout::new(root.as_ref());
        if !layout.manifest().exists() {
            return Err(PrismError::Other(format!(
                "no workspace at {}",
                layout.prism_dir().display()
            )));
        }
        let lock = WorkspaceLock::acquire(&layout.lock())?;
        let manifest = Manifest::decode(&fsutil::read_file(&layout.manifest())?)?;

        // Verify every referenced file: present, correct length, matching digest.
        for entry in &manifest.files {
            let path = layout.prism_dir().join(&entry.rel_path);
            let bytes = fsutil::read_file(&path).map_err(|_| PrismError::Corrupt {
                detail: format!("manifest references missing file {}", entry.rel_path),
            })?;
            if bytes.len() as u64 != entry.len {
                return Err(PrismError::Corrupt {
                    detail: format!(
                        "file {} length {} != manifest {}",
                        entry.rel_path,
                        bytes.len(),
                        entry.len
                    ),
                });
            }
            if sha256(&bytes) != entry.digest {
                return Err(PrismError::ChecksumMismatch {
                    detail: entry.rel_path.clone(),
                });
            }
        }

        // tmp/ cleanup is performed only by the writer under the lock.
        Self::clean_tmp(&layout)?;

        Ok(Store {
            layout,
            manifest,
            _lock: lock,
        })
    }

    fn clean_tmp(layout: &Layout) -> Result<()> {
        let tmp = layout.tmp_dir();
        if tmp.exists() {
            for entry in std::fs::read_dir(&tmp)? {
                let entry = entry?;
                let _ = std::fs::remove_file(entry.path());
            }
            fsutil::fsync_dir(&tmp)?;
        }
        Ok(())
    }

    /// The workspace layout.
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The current (in-memory) manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Publish a new object segment containing `records`, assigning each a fresh
    /// monotonic `logical_ts`. Returns the new segment's id.
    ///
    /// Follows the write protocol: stage the segment in `tmp/`, fsync, rename into
    /// `segments/`, fsync the dir, then atomically swap the manifest.
    pub fn publish_segment(&mut self, records: &[RawRecord]) -> Result<u32> {
        let segment_id = self.manifest.next_segment_id;
        let created_at = self.manifest.logical_ts_hwm as i64; // monotone stand-in clock

        let mut writer = SegmentWriter::new(segment_id, created_at);
        let mut ts = self.manifest.logical_ts_hwm;
        for rec in records {
            ts += 1;
            writer.append(rec.record_type, rec.flags, ts, &rec.key, &rec.value);
        }
        let image = writer.finish();
        let digest = sha256(&image);

        // Stage in tmp/, fsync, then rename into segments/ and fsync that dir.
        let staged = self.layout.tmp_dir().join(Layout::segment_file_name(segment_id));
        fsutil::atomic_write(&staged, &image)?; // durable write into tmp/
        let final_path = self.layout.segment_file(segment_id);
        std::fs::rename(&staged, &final_path)?;
        fsutil::fsync_dir(&self.layout.segments_dir())?;

        // Build the next manifest and publish it atomically.
        let mut next = self.manifest.clone();
        next.next_segment_id += 1;
        next.logical_ts_hwm = ts;
        next.files.push(FileEntry {
            kind: FileKind::Segment,
            rel_path: format!("segments/{}", Layout::segment_file_name(segment_id)),
            len: image.len() as u64,
            digest,
        });
        fsutil::atomic_write(&self.layout.manifest(), &next.encode())?;
        self.manifest = next;
        Ok(segment_id)
    }

    /// Read every record across all published segments, in segment then append
    /// order. (A convenience for tests and `prism status`; real reads go through
    /// indexes once those land.)
    pub fn read_all_records(&self) -> Result<Vec<StoredRecord>> {
        let mut out = Vec::new();
        for entry in &self.manifest.files {
            if entry.kind != FileKind::Segment {
                continue;
            }
            let path = self.layout.prism_dir().join(&entry.rel_path);
            let seg = SegmentReader::parse(fsutil::read_file(&path)?)?;
            for rec in seg.records() {
                let rec = rec?;
                out.push(StoredRecord {
                    record_type: rec.header.record_type,
                    key: rec.key.to_vec(),
                    value: rec.value.to_vec(),
                    logical_ts: rec.header.logical_ts,
                });
            }
        }
        Ok(out)
    }
}

/// An owned record read back from a segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRecord {
    pub record_type: u16,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub logical_ts: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::TempDir;

    #[test]
    fn init_open_publish_roundtrip() {
        let td = TempDir::new("store");
        {
            let mut s = Store::init(td.path()).unwrap();
            s.publish_segment(&[
                RawRecord::new(1, b"react".to_vec(), b"npm:react".to_vec()),
                RawRecord::new(1, b"serde".to_vec(), b"cargo:serde".to_vec()),
            ])
            .unwrap();
        } // drop releases the lock

        let s = Store::open(td.path()).unwrap();
        let recs = s.read_all_records().unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].key, b"react");
        assert_eq!(recs[1].value, b"cargo:serde");
        // logical_ts is monotonic and recovered.
        assert_eq!(recs[0].logical_ts, 1);
        assert_eq!(recs[1].logical_ts, 2);
        assert_eq!(s.manifest().logical_ts_hwm, 2);
        assert_eq!(s.manifest().next_segment_id, 2);
    }

    #[test]
    fn multiple_publishes_continue_counters() {
        let td = TempDir::new("store-multi");
        let mut s = Store::init(td.path()).unwrap();
        let id1 = s.publish_segment(&[RawRecord::new(1, b"a".to_vec(), b"1".to_vec())]).unwrap();
        let id2 = s.publish_segment(&[RawRecord::new(1, b"b".to_vec(), b"2".to_vec())]).unwrap();
        assert_eq!((id1, id2), (1, 2));
        assert_eq!(s.read_all_records().unwrap().len(), 2);
        assert_eq!(s.manifest().logical_ts_hwm, 2);
    }

    #[test]
    fn recovery_ignores_tmp_and_orphans() {
        let td = TempDir::new("store-recover");
        {
            let mut s = Store::init(td.path()).unwrap();
            s.publish_segment(&[RawRecord::new(1, b"k".to_vec(), b"v".to_vec())]).unwrap();
        }
        let layout = Layout::new(td.path());
        // Simulate an interrupted publish: a stray staged file and a stray MANIFEST.tmp.
        std::fs::write(layout.tmp_dir().join("000099.pseg"), b"garbage").unwrap();
        std::fs::write(layout.manifest_tmp(), b"half-written").unwrap();
        // An orphan segment file not referenced by the manifest.
        std::fs::write(layout.segment_file(42), b"orphan").unwrap();

        let s = Store::open(td.path()).unwrap();
        // The committed data is intact and the stray tmp file was cleaned.
        assert_eq!(s.read_all_records().unwrap().len(), 1);
        assert!(!layout.tmp_dir().join("000099.pseg").exists());
    }

    #[test]
    fn corrupted_published_segment_is_detected_on_open() {
        let td = TempDir::new("store-corrupt");
        {
            let mut s = Store::init(td.path()).unwrap();
            s.publish_segment(&[RawRecord::new(1, b"k".to_vec(), b"v".to_vec())]).unwrap();
        }
        // Corrupt the published segment in place.
        let layout = Layout::new(td.path());
        let seg = layout.segment_file(1);
        let mut bytes = std::fs::read(&seg).unwrap();
        let n = bytes.len();
        bytes[n / 2] ^= 0xFF;
        std::fs::write(&seg, &bytes).unwrap();

        assert!(matches!(
            Store::open(td.path()),
            Err(PrismError::ChecksumMismatch { .. }) | Err(PrismError::Corrupt { .. })
        ));
    }

    #[test]
    fn double_init_fails() {
        let td = TempDir::new("store-double");
        let _s = Store::init(td.path()).unwrap();
        // lock is held; a second init must fail (already initialized or locked).
        assert!(Store::init(td.path()).is_err());
    }
}
