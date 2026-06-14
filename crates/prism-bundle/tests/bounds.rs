//! Header advertises absurd counts / lengths. Every variant must produce a
//! defined [`PrismError`] (no panic, no large allocation).

use prism_codec::{crc32, sha256, ByteWriter};
use prism_model::PrismError;

const HEADER_LEN: usize = 32;
const TRAILER_LEN: usize = 44;
const BUNDLE_MAGIC: &[u8; 8] = b"PRISMBND";
const TRAILER_MAGIC: &[u8; 8] = b"PRISMEND";
const BUNDLE_FORMAT_VERSION: u32 = 1;
const TRAILER_VERSION: u32 = 1;

/// Build a valid-shape bundle with one MANIFEST entry, then patch the header
/// in place to claim `claimed_entry_count`. Used by the
/// `claims_u32_max_entries` test to drive the count-bounds path without
/// requiring an actually-huge TOC.
fn build_with_claimed_count(claimed_entry_count: u32) -> Vec<u8> {
    let manifest_payload = b"PRISMMAN-fake".to_vec();
    let manifest_sha = sha256(&manifest_payload);
    let manifest_offset = HEADER_LEN as u64;

    let mut w = ByteWriter::new();
    w.write_bytes(&[0u8; HEADER_LEN]);
    w.write_bytes(&manifest_payload);
    let toc_offset = w.len() as u64;

    let mut tocw = ByteWriter::new();
    tocw.write_u8(5);
    tocw.write_u8(0);
    let path = b"MANIFEST";
    tocw.write_u16(path.len() as u16);
    tocw.write_bytes(path);
    tocw.write_u64(manifest_offset);
    tocw.write_u64(manifest_payload.len() as u64);
    tocw.write_bytes(manifest_sha.as_bytes());
    let toc_body = tocw.into_vec();
    let toc_crc = crc32(&toc_body);
    w.write_bytes(&toc_body);
    w.write_bytes(&toc_crc.to_le_bytes());

    let mut header = [0u8; HEADER_LEN];
    header[0..8].copy_from_slice(BUNDLE_MAGIC);
    header[8..12].copy_from_slice(&BUNDLE_FORMAT_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&0u32.to_le_bytes());
    header[16..24].copy_from_slice(&toc_offset.to_le_bytes());
    header[24..28].copy_from_slice(&claimed_entry_count.to_le_bytes());
    let header_crc = crc32(&header[0..28]);
    header[28..32].copy_from_slice(&header_crc.to_le_bytes());

    let mut out = w.into_vec();
    out[..HEADER_LEN].copy_from_slice(&header);
    let bundle_sha = sha256(&out);
    out.extend_from_slice(TRAILER_MAGIC);
    out.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
    out.extend_from_slice(bundle_sha.as_bytes());
    debug_assert_eq!(out.len() - (out.len() - TRAILER_LEN), TRAILER_LEN);
    out
}

#[test]
fn claims_u32_max_entries_yields_corrupt_without_panic() {
    // u32::MAX entries claimed but the body has only one. This must trip the
    // MAX_TOC_ENTRIES check (which fires before any allocation) and never
    // panic. The MAX_TOC_ENTRIES guard fires first.
    let bytes = build_with_claimed_count(u32::MAX);
    let err = prism_bundle::read_summary(&bytes).expect_err("must reject");
    assert!(matches!(err, PrismError::Corrupt { .. }), "got {err:?}");
}

#[test]
fn claims_one_above_max_toc_entries_yields_corrupt() {
    let bytes = build_with_claimed_count(prism_bundle::MAX_TOC_ENTRIES + 1);
    let err = prism_bundle::read_summary(&bytes).expect_err("must reject");
    assert!(matches!(err, PrismError::Corrupt { .. }), "got {err:?}");
}

#[test]
fn payload_offset_inside_header_yields_corrupt() {
    // Hand-build a bundle where the TOC entry's payload_offset points inside
    // the header region. The reader must reject this in verify_entry_range.
    let manifest_payload = b"PRISMMAN-fake".to_vec();
    let manifest_sha = sha256(&manifest_payload);

    let mut w = ByteWriter::new();
    w.write_bytes(&[0u8; HEADER_LEN]);
    // Skip writing payload bytes — the TOC will lie about where they are.
    w.write_bytes(&manifest_payload);
    let toc_offset = w.len() as u64;

    let mut tocw = ByteWriter::new();
    tocw.write_u8(5);
    tocw.write_u8(0);
    let path = b"MANIFEST";
    tocw.write_u16(path.len() as u16);
    tocw.write_bytes(path);
    // Lie: claim payload starts at offset 8 (inside the header).
    tocw.write_u64(8);
    tocw.write_u64(manifest_payload.len() as u64);
    tocw.write_bytes(manifest_sha.as_bytes());
    let toc_body = tocw.into_vec();
    let toc_crc = crc32(&toc_body);
    w.write_bytes(&toc_body);
    w.write_bytes(&toc_crc.to_le_bytes());

    let mut header = [0u8; HEADER_LEN];
    header[0..8].copy_from_slice(BUNDLE_MAGIC);
    header[8..12].copy_from_slice(&BUNDLE_FORMAT_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&0u32.to_le_bytes());
    header[16..24].copy_from_slice(&toc_offset.to_le_bytes());
    header[24..28].copy_from_slice(&1u32.to_le_bytes());
    let header_crc = crc32(&header[0..28]);
    header[28..32].copy_from_slice(&header_crc.to_le_bytes());

    let mut out = w.into_vec();
    out[..HEADER_LEN].copy_from_slice(&header);
    let bundle_sha = sha256(&out);
    out.extend_from_slice(TRAILER_MAGIC);
    out.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
    out.extend_from_slice(bundle_sha.as_bytes());

    let err = prism_bundle::read_summary(&out).expect_err("must reject");
    // Must be Corrupt with a payload_offset complaint.
    match err {
        PrismError::Corrupt { detail } => {
            assert!(
                detail.contains("payload_offset") || detail.contains("inside header"),
                "detail: {detail}"
            );
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[test]
fn payload_offset_plus_len_overflowing_yields_out_of_bounds_or_corrupt() {
    // Hand-build a TOC entry with payload_offset = u64::MAX and a non-zero
    // payload_len. The checked_add must trip before any slicing.
    let manifest_payload = b"PRISMMAN-fake".to_vec();
    let manifest_sha = sha256(&manifest_payload);

    let mut w = ByteWriter::new();
    w.write_bytes(&[0u8; HEADER_LEN]);
    w.write_bytes(&manifest_payload);
    let toc_offset = w.len() as u64;

    let mut tocw = ByteWriter::new();
    tocw.write_u8(5);
    tocw.write_u8(0);
    let path = b"MANIFEST";
    tocw.write_u16(path.len() as u16);
    tocw.write_bytes(path);
    tocw.write_u64(u64::MAX);
    tocw.write_u64(1);
    tocw.write_bytes(manifest_sha.as_bytes());
    let toc_body = tocw.into_vec();
    let toc_crc = crc32(&toc_body);
    w.write_bytes(&toc_body);
    w.write_bytes(&toc_crc.to_le_bytes());

    let mut header = [0u8; HEADER_LEN];
    header[0..8].copy_from_slice(BUNDLE_MAGIC);
    header[8..12].copy_from_slice(&BUNDLE_FORMAT_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&0u32.to_le_bytes());
    header[16..24].copy_from_slice(&toc_offset.to_le_bytes());
    header[24..28].copy_from_slice(&1u32.to_le_bytes());
    let header_crc = crc32(&header[0..28]);
    header[28..32].copy_from_slice(&header_crc.to_le_bytes());

    let mut out = w.into_vec();
    out[..HEADER_LEN].copy_from_slice(&header);
    let bundle_sha = sha256(&out);
    out.extend_from_slice(TRAILER_MAGIC);
    out.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
    out.extend_from_slice(bundle_sha.as_bytes());

    let err = prism_bundle::read_summary(&out).expect_err("must reject");
    assert!(
        matches!(err, PrismError::OutOfBounds { .. } | PrismError::Corrupt { .. }),
        "got {err:?}"
    );
}

#[test]
fn header_advertises_toc_offset_past_trailer() {
    // toc_offset claims a position beyond the trailer start.
    let manifest_payload = b"PRISMMAN-fake".to_vec();
    let manifest_sha = sha256(&manifest_payload);

    let mut w = ByteWriter::new();
    w.write_bytes(&[0u8; HEADER_LEN]);
    w.write_bytes(&manifest_payload);
    let _real_toc_offset = w.len() as u64;

    let mut tocw = ByteWriter::new();
    tocw.write_u8(5);
    tocw.write_u8(0);
    let path = b"MANIFEST";
    tocw.write_u16(path.len() as u16);
    tocw.write_bytes(path);
    tocw.write_u64(HEADER_LEN as u64);
    tocw.write_u64(manifest_payload.len() as u64);
    tocw.write_bytes(manifest_sha.as_bytes());
    let toc_body = tocw.into_vec();
    let toc_crc = crc32(&toc_body);
    w.write_bytes(&toc_body);
    w.write_bytes(&toc_crc.to_le_bytes());

    // Header claims toc_offset = u64::MAX which is past the trailer.
    let mut header = [0u8; HEADER_LEN];
    header[0..8].copy_from_slice(BUNDLE_MAGIC);
    header[8..12].copy_from_slice(&BUNDLE_FORMAT_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&0u32.to_le_bytes());
    header[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
    header[24..28].copy_from_slice(&1u32.to_le_bytes());
    let header_crc = crc32(&header[0..28]);
    header[28..32].copy_from_slice(&header_crc.to_le_bytes());

    let mut out = w.into_vec();
    out[..HEADER_LEN].copy_from_slice(&header);
    let bundle_sha = sha256(&out);
    out.extend_from_slice(TRAILER_MAGIC);
    out.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
    out.extend_from_slice(bundle_sha.as_bytes());

    let err = prism_bundle::read_summary(&out).expect_err("must reject");
    assert!(
        matches!(err, PrismError::OutOfBounds { .. } | PrismError::Corrupt { .. }),
        "got {err:?}"
    );
}
