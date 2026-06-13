//! The manifest: the authoritative, atomically-published description of a store
//! epoch (architecture doc, *Crash Safety* and *Store Invariants*).
//!
//! It records the epoch counter, the recovered high-water marks for the monotonic
//! ID allocators, and one entry per referenced file (kind, name, length, and a
//! whole-file SHA-256 digest). Recovery trusts each file only up to its recorded
//! length and verifies its digest.

use prism_codec::{ByteReader, ByteWriter};
use prism_model::{HashDigest, PrismError, Result};

/// Manifest file magic: `b"PRISMMAN"`.
pub const MANIFEST_MAGIC: &[u8; 8] = b"PRISMMAN";
/// Current manifest format version.
///
/// v2 adds the `next_string_id` high-water mark for the [`StringId`] interner.
/// v1 images still decode (the counter defaults to `1`).
pub const MANIFEST_FORMAT_VERSION: u32 = 2;

/// The category of a referenced file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FileKind {
    Segment = 0,
    Index = 1,
    Graph = 2,
    Tda = 3,
    Candidate = 4,
}

impl FileKind {
    fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => FileKind::Segment,
            1 => FileKind::Index,
            2 => FileKind::Graph,
            3 => FileKind::Tda,
            4 => FileKind::Candidate,
            other => {
                return Err(PrismError::Corrupt {
                    detail: format!("unknown file kind {other}"),
                })
            }
        })
    }
}

/// One referenced file in the published set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub kind: FileKind,
    /// Path relative to `.prism/` (e.g. `segments/000001.pseg`).
    pub rel_path: String,
    /// Authoritative valid length in bytes.
    pub len: u64,
    /// Whole-file SHA-256 digest.
    pub digest: HashDigest,
}

/// The full manifest contents.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    /// Increments on every compaction; identifies the store epoch.
    pub epoch: u64,
    /// High-water mark of the `logical_ts` allocator (recovered, never regresses).
    pub logical_ts_hwm: u64,
    /// Next segment id to allocate (segment ids are never reused).
    pub next_segment_id: u32,
    /// High-water mark of the store-global object-id allocator (packages, versions, …).
    pub next_object_id: u64,
    /// High-water mark of the store-global [`prism_model::StringId`] interner.
    pub next_string_id: u32,
    /// The referenced file set.
    pub files: Vec<FileEntry>,
}

impl Manifest {
    /// An empty manifest for a freshly initialized workspace.
    pub fn empty() -> Self {
        Self {
            epoch: 0,
            logical_ts_hwm: 0,
            next_segment_id: 1,
            next_object_id: 1,
            next_string_id: 1,
            files: Vec::new(),
        }
    }

    /// Look up a file entry by its relative path.
    pub fn file(&self, rel_path: &str) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.rel_path == rel_path)
    }

    /// Serialize to the on-disk byte image (trailing CRC over all prior bytes).
    pub fn encode(&self) -> Vec<u8> {
        let mut w = ByteWriter::new();
        w.write_bytes(MANIFEST_MAGIC);
        w.write_u32(MANIFEST_FORMAT_VERSION);
        w.write_u64(self.epoch);
        w.write_u64(self.logical_ts_hwm);
        w.write_u32(self.next_segment_id);
        w.write_u64(self.next_object_id);
        w.write_u32(self.next_string_id);
        w.write_u32(self.files.len() as u32);
        for f in &self.files {
            w.write_u8(f.kind as u8);
            let path = f.rel_path.as_bytes();
            w.write_u32(path.len() as u32);
            w.write_bytes(path);
            w.write_u64(f.len);
            w.write_bytes(f.digest.as_bytes());
        }
        let crc = prism_codec::crc32(w.as_slice());
        w.write_u32(crc);
        w.into_vec()
    }

    /// Parse an on-disk manifest image, validating magic, version, and CRC.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 4 {
            return Err(PrismError::Corrupt {
                detail: "manifest too short".into(),
            });
        }
        let (body, crc_bytes) = bytes.split_at(bytes.len() - 4);
        let expected = u32::from_le_bytes(crc_bytes.try_into().unwrap());
        if prism_codec::crc32(body) != expected {
            return Err(PrismError::ChecksumMismatch {
                detail: "manifest crc".into(),
            });
        }

        let mut r = ByteReader::new(body);
        let magic = r.read_bytes(MANIFEST_MAGIC.len())?;
        if magic != MANIFEST_MAGIC {
            return Err(PrismError::Corrupt {
                detail: "bad manifest magic".into(),
            });
        }
        let version = r.read_u32()?;
        if version == 0 || version > MANIFEST_FORMAT_VERSION {
            return Err(PrismError::UnsupportedFormatVersion {
                found: version,
                max: MANIFEST_FORMAT_VERSION,
            });
        }
        let epoch = r.read_u64()?;
        let logical_ts_hwm = r.read_u64()?;
        let next_segment_id = r.read_u32()?;
        let next_object_id = r.read_u64()?;
        // v2 added the interner high-water mark; v1 images default it to 1.
        let next_string_id = if version >= 2 { r.read_u32()? } else { 1 };
        let count = r.read_u32()? as usize;

        // Don't pre-reserve from an unvalidated count: a `FileEntry` encodes to at
        // least 45 bytes (1 kind + 4 path_len + 8 len + 32 digest), so a count that
        // could not possibly fit in the remaining body is rejected, and the
        // reservation is bounded by what is actually there. (The CRC above is not a
        // MAC, so a forged count must not drive a huge allocation.)
        const MIN_ENTRY_BYTES: usize = 1 + 4 + 8 + 32;
        if count > r.remaining() / MIN_ENTRY_BYTES {
            return Err(PrismError::Corrupt {
                detail: format!("manifest file count {count} exceeds remaining bytes"),
            });
        }
        let mut files = Vec::with_capacity(count);
        for _ in 0..count {
            let kind = FileKind::from_u8(r.read_u8()?)?;
            let path_len = r.read_u32()? as usize;
            let path_bytes = r.read_bytes(path_len)?;
            let rel_path = core::str::from_utf8(path_bytes)
                .map_err(|_| PrismError::Corrupt {
                    detail: "non-utf8 manifest path".into(),
                })?
                .to_owned();
            validate_rel_path(&rel_path)?;
            let len = r.read_u64()?;
            let digest = HashDigest::from_bytes(r.read_array()?);
            files.push(FileEntry {
                kind,
                rel_path,
                len,
                digest,
            });
        }

        Ok(Self {
            epoch,
            logical_ts_hwm,
            next_segment_id,
            next_object_id,
            next_string_id,
            files,
        })
    }
}

/// Reject a manifest `rel_path` that could escape the workspace directory when
/// joined to `.prism/`. The path is always built by the store from a fixed set of
/// templates, so a hostile value can only arrive via a crafted (but CRC-valid)
/// MANIFEST opened read-only; this is defense-in-depth so the store never `read`s
/// outside its own directory regardless.
fn validate_rel_path(rel_path: &str) -> Result<()> {
    let bad = rel_path.is_empty()
        || rel_path.starts_with('/')
        || rel_path.contains('\\')
        || rel_path.contains(':') // drive-letter / scheme forms
        || rel_path.split('/').any(|c| c == ".." || c == ".");
    // Absolute Windows-style or UNC paths are caught by the ':' / '\\' checks.
    if bad {
        return Err(PrismError::Corrupt {
            detail: format!("unsafe manifest path {rel_path:?}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            epoch: 4,
            logical_ts_hwm: 99,
            next_segment_id: 7,
            next_object_id: 1234,
            next_string_id: 55,
            files: vec![
                FileEntry {
                    kind: FileKind::Segment,
                    rel_path: "segments/000001.pseg".into(),
                    len: 512,
                    digest: prism_codec::sha256(b"one"),
                },
                FileEntry {
                    kind: FileKind::Graph,
                    rel_path: "graphs/run_00000001.pcsr".into(),
                    len: 2048,
                    digest: prism_codec::sha256(b"two"),
                },
            ],
        }
    }

    #[test]
    fn manifest_roundtrip() {
        let m = sample();
        let bytes = m.encode();
        assert_eq!(Manifest::decode(&bytes).unwrap(), m);
    }

    #[test]
    fn empty_manifest_roundtrip() {
        let m = Manifest::empty();
        assert_eq!(Manifest::decode(&m.encode()).unwrap(), m);
        assert_eq!(m.next_segment_id, 1);
    }

    #[test]
    fn crc_detects_corruption() {
        let mut bytes = sample().encode();
        bytes[20] ^= 0xFF;
        assert!(matches!(
            Manifest::decode(&bytes),
            Err(PrismError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn file_lookup() {
        let m = sample();
        assert!(m.file("segments/000001.pseg").is_some());
        assert!(m.file("nope").is_none());
    }

    #[test]
    fn v1_image_decodes_with_default_string_hwm() {
        // Hand-assemble a v1 manifest body (no next_string_id field) and verify
        // it decodes with the interner counter defaulted to 1.
        let mut w = ByteWriter::new();
        w.write_bytes(MANIFEST_MAGIC);
        w.write_u32(1); // format version 1
        w.write_u64(2); // epoch
        w.write_u64(3); // logical_ts_hwm
        w.write_u32(4); // next_segment_id
        w.write_u64(5); // next_object_id
        w.write_u32(0); // file count
        let crc = prism_codec::crc32(w.as_slice());
        w.write_u32(crc);
        let m = Manifest::decode(&w.into_vec()).unwrap();
        assert_eq!(m.next_string_id, 1);
        assert_eq!(m.next_object_id, 5);
        assert_eq!(m.epoch, 2);
    }
}
