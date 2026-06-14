//! Future-compat boundary: a bundle whose header advertises
//! `format_version = 2` must be rejected with [`PrismError::UnsupportedFormatVersion`]
//! and *not* misclassified as `Corrupt`. This is the only path readers should
//! take when handed a bundle from a newer writer.

use prism_codec::{crc32, sha256, ByteWriter};
use prism_model::PrismError;

const HEADER_LEN: usize = 32;
const TRAILER_LEN: usize = 44;
const BUNDLE_MAGIC: &[u8; 8] = b"PRISMBND";
const TRAILER_MAGIC: &[u8; 8] = b"PRISMEND";
const TRAILER_VERSION: u32 = 1;

/// Build the simplest valid-shape bundle we can — one MANIFEST entry,
/// header magic intact — and pin the format version to `bumped_version`.
fn build_with_version(bumped_version: u32) -> Vec<u8> {
    let manifest_payload = b"PRISMMAN-fake".to_vec();
    let manifest_sha = sha256(&manifest_payload);
    let manifest_offset = HEADER_LEN as u64;

    let mut w = ByteWriter::new();
    w.write_bytes(&[0u8; HEADER_LEN]);
    w.write_bytes(&manifest_payload);
    let toc_offset = w.len() as u64;

    // One TOC entry: Manifest.
    let mut tocw = ByteWriter::new();
    tocw.write_u8(5); // Manifest
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
    header[8..12].copy_from_slice(&bumped_version.to_le_bytes());
    header[12..16].copy_from_slice(&0u32.to_le_bytes());
    header[16..24].copy_from_slice(&toc_offset.to_le_bytes());
    header[24..28].copy_from_slice(&1u32.to_le_bytes()); // entry_count = 1
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
fn future_format_version_yields_unsupported_format_version() {
    let bytes = build_with_version(2);
    let err = prism_bundle::read_summary(&bytes).expect_err("v2 must fail");
    match err {
        PrismError::UnsupportedFormatVersion { found, max } => {
            assert_eq!(found, 2);
            assert_eq!(max, prism_bundle::BUNDLE_FORMAT_VERSION);
        }
        other => panic!("expected UnsupportedFormatVersion, got {other:?}"),
    }
}

#[test]
fn far_future_format_version_still_yields_version_error_not_corrupt() {
    let bytes = build_with_version(u32::MAX);
    let err = prism_bundle::read_summary(&bytes).expect_err("max must fail");
    assert!(
        matches!(err, PrismError::UnsupportedFormatVersion { .. }),
        "got {err:?}"
    );
}

#[test]
fn zero_format_version_is_unsupported() {
    let bytes = build_with_version(0);
    let err = prism_bundle::read_summary(&bytes).expect_err("v0 must fail");
    assert!(
        matches!(err, PrismError::UnsupportedFormatVersion { .. }),
        "got {err:?}"
    );
}
