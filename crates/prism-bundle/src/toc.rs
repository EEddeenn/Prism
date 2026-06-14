//! Flat table of contents (TOC) for a `.prismbundle`.
//!
//! The TOC is written once, near the end of the bundle (between the payload
//! region and the trailer). Each entry describes one embedded file: its
//! [`EntryKind`], a UTF-8 relative path matching the on-disk layout of `.prism/`
//! (e.g. `"MANIFEST"`, `"segments/000001.pseg"`), its absolute byte range inside
//! the bundle, and a SHA-256 of the payload bytes so corruption of any single
//! file is caught even if the whole-bundle hash were forged.
//!
//! Encoding is a fixed-shape little-endian record per entry:
//!
//! ```text
//! [u8 kind][u8 0 reserved][u16 path_len][path_len bytes path]
//! [u64 payload_offset][u64 payload_len][32 bytes sha256]
//! ```
//!
//! followed by a single `[u32 crc32]` over the entry bytes that precede it.
//!
//! ## Untrusted-input discipline
//!
//! [`decode_toc`] enforces every bound *before* allocating:
//!
//! - the trailing CRC must be present and match;
//! - the advertised `expected_count` must fit into the remaining bytes at the
//!   minimum per-entry width (52 bytes) — i.e. we refuse to `Vec::with_capacity`
//!   a count a hostile producer could never have written;
//! - each entry's path length is capped at [`MAX_PATH_LEN`];
//! - each path is validated by [`validate_rel_path`] (mirrors
//!   `prism_store::manifest::validate_rel_path` and additionally rejects any
//!   NUL byte) so a malicious TOC cannot drive the unpacker outside `.prism/`.
//!
//! All arithmetic on lengths uses `checked_*` so an adversarial entry cannot
//! wrap a `usize` / `u64` past the buffer end. Range and overlap checks live in
//! the reader (this module is purely structural).

use prism_codec::{crc32, ByteReader, ByteWriter};
use prism_model::{HashDigest, Result};

use crate::error::{corrupt, out_of_bounds};
use crate::format::{EntryKind, MAX_PATH_LEN, MAX_TOC_ENTRIES};

/// Minimum number of bytes any single TOC entry occupies on disk.
///
/// Computed from the fixed-shape encoding:
/// `kind (1) + reserved (1) + path_len (2) + path (0+) + offset (8) + len (8) + sha256 (32)`
/// = 52 bytes, attained when the path is empty. Empty paths are themselves
/// rejected by [`validate_rel_path`]; this bound is solely for the
/// allocation-guard so a bogus `expected_count` cannot DoS the decoder.
const MIN_ENTRY_BYTES: usize = 1 + 1 + 2 + 0 + 8 + 8 + 32;

/// Width, in bytes, of the trailing CRC-32 footer that follows the entry body.
const TOC_CRC_LEN: usize = 4;

/// One row of the bundle table of contents.
///
/// `rel_path` always uses forward slashes and mirrors the path shape the store
/// writes (`"MANIFEST"`, `"segments/NNNNNN.pseg"`, `"graphs/run_NNNNNNNN.pcsr"`,
/// `"tda/filtration_NNNNNNNN.ptda"`, `"indexes/NNNNNN.pidx"`).
///
/// `payload_offset` and `payload_len` are **absolute** into the bundle file: the
/// reader verifies they fall entirely within the payload region (after the
/// 32-byte header and before the TOC) without overlap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocEntry {
    /// What category of file this entry references.
    pub kind: EntryKind,
    /// UTF-8 path relative to the unpacked `.prism/` directory.
    pub rel_path: String,
    /// Absolute byte offset of the payload in the bundle file.
    pub payload_offset: u64,
    /// Authoritative payload byte length (also validated against the SHA-256).
    pub payload_len: u64,
    /// SHA-256 of `bytes[payload_offset .. payload_offset + payload_len]`.
    pub sha256: HashDigest,
}

/// Encode a list of TOC entries into the on-disk bytes (body + trailing CRC-32).
///
/// Callers are responsible for ordering `entries` deterministically (see the
/// writer); this function preserves whatever order it is given. No bounds are
/// enforced here beyond `path_len`'s `u16` representation — the writer is the
/// trusted side and the reader does the validation.
pub fn encode_toc(entries: &[TocEntry]) -> Vec<u8> {
    // Conservative pre-size: every fixed field + average-ish path length.
    let avg_path = 32usize;
    let mut w = ByteWriter::with_capacity(entries.len() * (MIN_ENTRY_BYTES + avg_path) + TOC_CRC_LEN);
    for e in entries {
        w.write_u8(e.kind.as_u8());
        w.write_u8(0); // reserved
        // path_len is bounded by the writer; for safety we truncate the upper
        // bits at the u16 width — caller is expected to have validated.
        let path_bytes = e.rel_path.as_bytes();
        w.write_u16(path_bytes.len() as u16);
        w.write_bytes(path_bytes);
        w.write_u64(e.payload_offset);
        w.write_u64(e.payload_len);
        w.write_bytes(e.sha256.as_bytes());
    }
    let body = w.into_vec();
    let crc = crc32(&body);
    let mut out = body;
    out.reserve_exact(TOC_CRC_LEN);
    out.extend_from_slice(&crc.to_le_bytes());
    out
}

/// Decode TOC bytes (body + trailing CRC-32) into [`TocEntry`] records.
///
/// `expected_count` is the entry count advertised by the bundle header.
/// `body_with_crc` must contain exactly the bytes between `toc_offset` and the
/// start of the trailer (44 bytes from end-of-file).
///
/// Returns [`prism_model::PrismError::Corrupt`] on:
/// - missing or mismatched CRC,
/// - a count too large to fit at [`MIN_ENTRY_BYTES`] per entry,
/// - an [`EntryKind`] with reserved/unknown discriminant,
/// - a `path_len` greater than [`MAX_PATH_LEN`],
/// - a non-UTF-8 path,
/// - a path rejected by [`validate_rel_path`] (empty / leading `/` / contains
///   `\` / `:` / `..` or `.` segment / any NUL byte).
///
/// Returns [`prism_model::PrismError::OutOfBounds`] on truncated underlying
/// reads. **Never panics** on hostile input.
pub fn decode_toc(body_with_crc: &[u8], expected_count: u32) -> Result<Vec<TocEntry>> {
    if expected_count > MAX_TOC_ENTRIES {
        return Err(corrupt(format!(
            "toc entry count {expected_count} exceeds MAX_TOC_ENTRIES {MAX_TOC_ENTRIES}"
        )));
    }
    if body_with_crc.len() < TOC_CRC_LEN {
        return Err(out_of_bounds(format!(
            "toc bytes too short: need at least {TOC_CRC_LEN}, have {}",
            body_with_crc.len()
        )));
    }
    let split = body_with_crc.len() - TOC_CRC_LEN;
    let body = &body_with_crc[..split];
    let crc_bytes: [u8; 4] = body_with_crc[split..]
        .try_into()
        .expect("split point leaves exactly 4 bytes");
    let want_crc = u32::from_le_bytes(crc_bytes);
    let have_crc = crc32(body);
    if want_crc != have_crc {
        return Err(corrupt(format!(
            "toc crc32 mismatch: header says {want_crc:#010x}, computed {have_crc:#010x}"
        )));
    }

    let mut r = ByteReader::new(body);
    let count = expected_count as usize;

    // Guard before allocating: a hostile count must not fit if there are not
    // even MIN_ENTRY_BYTES * count bytes to feed it.
    if count > r.remaining() / MIN_ENTRY_BYTES {
        return Err(corrupt(format!(
            "toc entry count {count} exceeds remaining bytes {}",
            r.remaining()
        )));
    }

    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let kind_raw = r.read_u8()?;
        let kind = EntryKind::from_u8(kind_raw)?;
        let reserved = r.read_u8()?;
        if reserved != 0 {
            return Err(corrupt(format!(
                "toc entry {i}: reserved byte must be 0, got {reserved}"
            )));
        }
        let path_len = r.read_u16()?;
        if path_len > MAX_PATH_LEN {
            return Err(corrupt(format!(
                "toc entry {i}: path_len {path_len} exceeds MAX_PATH_LEN {MAX_PATH_LEN}"
            )));
        }
        let path_bytes = r.read_bytes(path_len as usize)?;
        let rel_path = core::str::from_utf8(path_bytes)
            .map_err(|_| corrupt(format!("toc entry {i}: path is not utf-8")))?
            .to_owned();
        validate_rel_path(&rel_path)?;
        let payload_offset = r.read_u64()?;
        let payload_len = r.read_u64()?;
        let sha256 = HashDigest::from_bytes(r.read_array()?);
        entries.push(TocEntry {
            kind,
            rel_path,
            payload_offset,
            payload_len,
            sha256,
        });
    }

    if !r.is_empty() {
        return Err(corrupt(format!(
            "toc has {} trailing bytes after {count} entries",
            r.remaining()
        )));
    }

    Ok(entries)
}

/// Reject any relative path that could escape the unpacked `.prism/` directory.
///
/// Mirrors `prism_store::manifest::validate_rel_path` byte-for-byte and adds an
/// explicit NUL-byte rejection (defense in depth: a TOC entry hand-built by an
/// attacker could embed `\0` to truncate the path inside C-string syscalls on
/// some platforms, even though our writer never produces them).
///
/// Rejection rules:
/// - empty;
/// - starts with `/` (absolute Unix path);
/// - contains `\` (Windows path / UNC);
/// - contains `:` (Windows drive letter, URL scheme);
/// - any path segment is exactly `..` or `.`;
/// - any NUL byte.
pub fn validate_rel_path(rel_path: &str) -> Result<()> {
    let bad = rel_path.is_empty()
        || rel_path.starts_with('/')
        || rel_path.contains('\\')
        || rel_path.contains(':')
        || rel_path.contains('\0')
        || rel_path.split('/').any(|c| c == ".." || c == ".");
    if bad {
        return Err(corrupt(format!("unsafe path {rel_path:?}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(seed: u8) -> HashDigest {
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = seed.wrapping_add(i as u8);
        }
        HashDigest::from_bytes(bytes)
    }

    fn sample_pair() -> Vec<TocEntry> {
        vec![
            TocEntry {
                kind: EntryKind::Manifest,
                rel_path: "MANIFEST".into(),
                payload_offset: 32,
                payload_len: 128,
                sha256: digest(0x11),
            },
            TocEntry {
                kind: EntryKind::Segment,
                rel_path: "segments/000001.pseg".into(),
                payload_offset: 160,
                payload_len: 4096,
                sha256: digest(0x42),
            },
        ]
    }

    #[test]
    fn round_trip_two_entries_byte_identically() {
        let entries = sample_pair();
        let encoded = encode_toc(&entries);
        let decoded = decode_toc(&encoded, entries.len() as u32).expect("decode");
        assert_eq!(decoded, entries);

        // Re-encoding the decoded entries reproduces the same bytes — the
        // on-disk shape is deterministic.
        let re_encoded = encode_toc(&decoded);
        assert_eq!(re_encoded, encoded);
    }

    #[test]
    fn encoded_layout_matches_spec_for_first_entry() {
        let entries = vec![TocEntry {
            kind: EntryKind::Manifest,
            rel_path: "MANIFEST".into(),
            payload_offset: 32,
            payload_len: 128,
            sha256: digest(0x11),
        }];
        let encoded = encode_toc(&entries);
        // [u8 kind=5][u8 0][u16 path_len=8]["MANIFEST"][u64 offset=32][u64 len=128][32 sha][u32 crc]
        let expected_len = 1 + 1 + 2 + 8 + 8 + 8 + 32 + TOC_CRC_LEN;
        assert_eq!(encoded.len(), expected_len);
        assert_eq!(encoded[0], EntryKind::Manifest.as_u8());
        assert_eq!(encoded[1], 0);
        assert_eq!(u16::from_le_bytes([encoded[2], encoded[3]]), 8);
        assert_eq!(&encoded[4..12], b"MANIFEST");
    }

    #[test]
    fn rejects_path_with_dotdot_segment() {
        let entries = vec![TocEntry {
            kind: EntryKind::Segment,
            rel_path: "../escape".into(),
            payload_offset: 32,
            payload_len: 1,
            sha256: digest(0),
        }];
        let encoded = encode_toc(&entries);
        let err = decode_toc(&encoded, 1).expect_err("must reject ..");
        match err {
            prism_model::PrismError::Corrupt { detail } => {
                assert!(detail.contains("unsafe path"), "detail: {detail}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_path_with_dot_segment() {
        let entries = vec![TocEntry {
            kind: EntryKind::Segment,
            rel_path: "segments/./000001.pseg".into(),
            payload_offset: 32,
            payload_len: 1,
            sha256: digest(0),
        }];
        let encoded = encode_toc(&entries);
        let err = decode_toc(&encoded, 1).expect_err("must reject .");
        assert!(matches!(err, prism_model::PrismError::Corrupt { .. }));
    }

    #[test]
    fn rejects_absolute_path() {
        let entries = vec![TocEntry {
            kind: EntryKind::Segment,
            rel_path: "/etc/passwd".into(),
            payload_offset: 32,
            payload_len: 1,
            sha256: digest(0),
        }];
        let encoded = encode_toc(&entries);
        let err = decode_toc(&encoded, 1).expect_err("must reject leading /");
        assert!(matches!(err, prism_model::PrismError::Corrupt { .. }));
    }

    #[test]
    fn rejects_backslash_path() {
        let entries = vec![TocEntry {
            kind: EntryKind::Segment,
            rel_path: "segments\\000001.pseg".into(),
            payload_offset: 32,
            payload_len: 1,
            sha256: digest(0),
        }];
        let encoded = encode_toc(&entries);
        let err = decode_toc(&encoded, 1).expect_err("must reject \\");
        assert!(matches!(err, prism_model::PrismError::Corrupt { .. }));
    }

    #[test]
    fn rejects_colon_path() {
        let entries = vec![TocEntry {
            kind: EntryKind::Segment,
            rel_path: "C:/segments".into(),
            payload_offset: 32,
            payload_len: 1,
            sha256: digest(0),
        }];
        let encoded = encode_toc(&entries);
        let err = decode_toc(&encoded, 1).expect_err("must reject :");
        assert!(matches!(err, prism_model::PrismError::Corrupt { .. }));
    }

    #[test]
    fn rejects_empty_path() {
        let entries = vec![TocEntry {
            kind: EntryKind::Segment,
            rel_path: String::new(),
            payload_offset: 32,
            payload_len: 1,
            sha256: digest(0),
        }];
        let encoded = encode_toc(&entries);
        let err = decode_toc(&encoded, 1).expect_err("must reject empty path");
        assert!(matches!(err, prism_model::PrismError::Corrupt { .. }));
    }

    #[test]
    fn rejects_nul_byte_in_path() {
        let entries = vec![TocEntry {
            kind: EntryKind::Segment,
            rel_path: "segments/\0/000001.pseg".into(),
            payload_offset: 32,
            payload_len: 1,
            sha256: digest(0),
        }];
        let encoded = encode_toc(&entries);
        let err = decode_toc(&encoded, 1).expect_err("must reject NUL byte");
        assert!(matches!(err, prism_model::PrismError::Corrupt { .. }));
    }

    #[test]
    fn rejects_truncated_crc_footer() {
        let err = decode_toc(&[0u8; 3], 0).expect_err("crc footer too short");
        assert!(matches!(err, prism_model::PrismError::OutOfBounds { .. }));
    }

    #[test]
    fn rejects_bad_crc() {
        let entries = sample_pair();
        let mut encoded = encode_toc(&entries);
        let last = encoded.len() - 1;
        encoded[last] ^= 0xff;
        let err = decode_toc(&encoded, entries.len() as u32).expect_err("crc mismatch");
        match err {
            prism_model::PrismError::Corrupt { detail } => {
                assert!(detail.contains("crc32"), "detail: {detail}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_reserved_byte_nonzero() {
        let entries = vec![TocEntry {
            kind: EntryKind::Manifest,
            rel_path: "MANIFEST".into(),
            payload_offset: 32,
            payload_len: 1,
            sha256: digest(0),
        }];
        let mut encoded = encode_toc(&entries);
        // Flip the reserved byte (index 1) to 1 and re-CRC so we exercise the
        // explicit reserved check, not the CRC check.
        encoded[1] = 1;
        let body_end = encoded.len() - TOC_CRC_LEN;
        let new_crc = crc32(&encoded[..body_end]);
        encoded[body_end..].copy_from_slice(&new_crc.to_le_bytes());
        let err = decode_toc(&encoded, 1).expect_err("reserved must be 0");
        match err {
            prism_model::PrismError::Corrupt { detail } => {
                assert!(detail.contains("reserved"), "detail: {detail}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_path_len_over_max() {
        // Hand-build a TOC body with path_len = MAX_PATH_LEN + 1. We have to
        // include MIN_ENTRY_BYTES of real payload so the allocation-guard
        // (which fires earlier than the path_len check) lets us through and
        // we exercise the path_len rejection itself.
        let mut w = ByteWriter::new();
        w.write_u8(EntryKind::Segment.as_u8());
        w.write_u8(0);
        w.write_u16(MAX_PATH_LEN + 1);
        // Path bytes — we intentionally do not write that many; the path_len
        // check rejects the entry before any read of path_bytes.
        w.write_u64(0);
        w.write_u64(0);
        w.write_bytes(&[0u8; 32]);
        let body = w.into_vec();
        // Sanity: we wrote exactly MIN_ENTRY_BYTES of post-path-len framing,
        // so remaining()/MIN_ENTRY_BYTES >= 1 and the allocation guard
        // accepts expected_count = 1.
        assert!(body.len() >= MIN_ENTRY_BYTES);
        let crc = crc32(&body);
        let mut bytes = body;
        bytes.extend_from_slice(&crc.to_le_bytes());
        let err = decode_toc(&bytes, 1).expect_err("path_len overflows max");
        match err {
            prism_model::PrismError::Corrupt { detail } => {
                assert!(detail.contains("MAX_PATH_LEN"), "detail: {detail}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_count_over_max_toc_entries() {
        // CRC-valid empty body, but expected_count > MAX_TOC_ENTRIES.
        let crc = crc32(&[]);
        let bytes = crc.to_le_bytes().to_vec();
        let err = decode_toc(&bytes, MAX_TOC_ENTRIES + 1).expect_err("count over cap");
        match err {
            prism_model::PrismError::Corrupt { detail } => {
                assert!(detail.contains("MAX_TOC_ENTRIES"), "detail: {detail}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_count_that_would_overflow_remaining_bytes() {
        // CRC-valid 1-byte body claiming hundreds of entries — must reject
        // before allocating Vec::with_capacity(huge).
        let body = vec![0u8];
        let crc = crc32(&body);
        let mut bytes = body;
        bytes.extend_from_slice(&crc.to_le_bytes());
        let err = decode_toc(&bytes, 500).expect_err("count exceeds bytes");
        match err {
            prism_model::PrismError::Corrupt { detail } => {
                assert!(detail.contains("exceeds remaining"), "detail: {detail}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_entry_kind() {
        // Hand-build a one-entry body with kind = 6 (unknown), valid CRC.
        let mut w = ByteWriter::new();
        w.write_u8(6);
        w.write_u8(0);
        w.write_u16(1);
        w.write_u8(b'a');
        w.write_u64(0);
        w.write_u64(0);
        w.write_bytes(&[0u8; 32]);
        let body = w.into_vec();
        let crc = crc32(&body);
        let mut bytes = body;
        bytes.extend_from_slice(&crc.to_le_bytes());
        let err = decode_toc(&bytes, 1).expect_err("unknown kind");
        assert!(matches!(err, prism_model::PrismError::Corrupt { .. }));
    }

    #[test]
    fn rejects_reserved_candidate_kind() {
        // kind = 4 is reserved (Candidate); the bundle must refuse it.
        let mut w = ByteWriter::new();
        w.write_u8(4);
        w.write_u8(0);
        w.write_u16(1);
        w.write_u8(b'a');
        w.write_u64(0);
        w.write_u64(0);
        w.write_bytes(&[0u8; 32]);
        let body = w.into_vec();
        let crc = crc32(&body);
        let mut bytes = body;
        bytes.extend_from_slice(&crc.to_le_bytes());
        let err = decode_toc(&bytes, 1).expect_err("candidate reserved");
        match err {
            prism_model::PrismError::Corrupt { detail } => {
                assert!(detail.contains("Candidate"), "detail: {detail}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_non_utf8_path() {
        // path_len = 2, bytes 0xff 0xff (invalid UTF-8), valid CRC.
        let mut w = ByteWriter::new();
        w.write_u8(EntryKind::Segment.as_u8());
        w.write_u8(0);
        w.write_u16(2);
        w.write_bytes(&[0xff, 0xff]);
        w.write_u64(32);
        w.write_u64(1);
        w.write_bytes(&[0u8; 32]);
        let body = w.into_vec();
        let crc = crc32(&body);
        let mut bytes = body;
        bytes.extend_from_slice(&crc.to_le_bytes());
        let err = decode_toc(&bytes, 1).expect_err("non-utf8 path");
        match err {
            prism_model::PrismError::Corrupt { detail } => {
                assert!(detail.contains("utf-8"), "detail: {detail}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_trailing_bytes_after_advertised_count() {
        // Encode one entry but tell the decoder zero — leftover bytes must
        // be flagged so a producer cannot smuggle data past the count.
        let entries = sample_pair();
        let encoded = encode_toc(&entries);
        let err = decode_toc(&encoded, 0).expect_err("trailing bytes");
        match err {
            prism_model::PrismError::Corrupt { detail } => {
                assert!(detail.contains("trailing bytes"), "detail: {detail}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn empty_toc_roundtrips() {
        let encoded = encode_toc(&[]);
        // Just the 4-byte CRC of the empty body.
        assert_eq!(encoded.len(), TOC_CRC_LEN);
        let decoded = decode_toc(&encoded, 0).expect("empty decode");
        assert!(decoded.is_empty());
    }

    #[test]
    fn validate_rel_path_accepts_normal_shapes() {
        for p in [
            "MANIFEST",
            "segments/000001.pseg",
            "graphs/run_00000001.pcsr",
            "tda/filtration_00000001.ptda",
            "indexes/000001.pidx",
        ] {
            validate_rel_path(p).unwrap_or_else(|e| panic!("rejected {p:?}: {e:?}"));
        }
    }
}
