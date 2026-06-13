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
pub const MANIFEST_FORMAT_VERSION: u32 = 1;

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
        if version != MANIFEST_FORMAT_VERSION {
            return Err(PrismError::UnsupportedFormatVersion {
                found: version,
                max: MANIFEST_FORMAT_VERSION,
            });
        }
        let epoch = r.read_u64()?;
        let logical_ts_hwm = r.read_u64()?;
        let next_segment_id = r.read_u32()?;
        let next_object_id = r.read_u64()?;
        let count = r.read_u32()? as usize;

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
            files,
        })
    }
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
}
