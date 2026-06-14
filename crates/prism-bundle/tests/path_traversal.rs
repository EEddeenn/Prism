//! Hand-craft a bundle whose TOC points at a path that would escape the
//! `.prism/` directory if blindly joined (e.g. `../../etc/passwd`,
//! `..\\..\\etc\\passwd`). The reader must reject these in `read_summary`
//! *before* anyone calls `unpack_into`.
//!
//! We bypass `encode_toc` here so the hostile path bytes reach the decoder
//! exactly as a malicious producer would have written them. The CRC and
//! per-payload SHA-256 are recomputed honestly so the *only* defense the
//! reader has against this attack is `validate_rel_path` — exactly what we
//! are testing.

use prism_codec::{crc32, sha256, ByteWriter};
use prism_model::PrismError;

const HEADER_LEN: usize = 32;
const TRAILER_LEN: usize = 44;
const BUNDLE_MAGIC: &[u8; 8] = b"PRISMBND";
const TRAILER_MAGIC: &[u8; 8] = b"PRISMEND";
const BUNDLE_FORMAT_VERSION: u32 = 1;
const TRAILER_VERSION: u32 = 1;

/// Build a bundle with two entries: a benign MANIFEST and one hostile-path
/// entry whose payload is real and whose CRC is correct.
fn build_hostile(rel_path: &str) -> Vec<u8> {
    let manifest_payload = b"PRISMMAN-fake-manifest-bytes".to_vec();
    let hostile_payload = b"# pretend this is /etc/passwd".to_vec();
    let manifest_sha = sha256(&manifest_payload);
    let hostile_sha = sha256(&hostile_payload);

    let manifest_offset = HEADER_LEN as u64;
    let hostile_offset = manifest_offset + manifest_payload.len() as u64;

    // Body bytes (everything before header is patched and before TOC CRC).
    let mut w = ByteWriter::new();
    w.write_bytes(&[0u8; HEADER_LEN]);
    w.write_bytes(&manifest_payload);
    w.write_bytes(&hostile_payload);
    let toc_offset = w.len() as u64;

    // --- Hand-encode the TOC ---
    let mut tocw = ByteWriter::new();
    // Entry 0: Manifest, rel_path = "MANIFEST"
    tocw.write_u8(5); // Manifest kind
    tocw.write_u8(0); // reserved
    let m_path = b"MANIFEST";
    tocw.write_u16(m_path.len() as u16);
    tocw.write_bytes(m_path);
    tocw.write_u64(manifest_offset);
    tocw.write_u64(manifest_payload.len() as u64);
    tocw.write_bytes(manifest_sha.as_bytes());

    // Entry 1: hostile path, kind = Segment (0). path bytes verbatim.
    let h_path = rel_path.as_bytes();
    tocw.write_u8(0); // Segment kind
    tocw.write_u8(0); // reserved
    tocw.write_u16(h_path.len() as u16);
    tocw.write_bytes(h_path);
    tocw.write_u64(hostile_offset);
    tocw.write_u64(hostile_payload.len() as u64);
    tocw.write_bytes(hostile_sha.as_bytes());

    let toc_body = tocw.into_vec();
    let toc_crc = crc32(&toc_body);

    w.write_bytes(&toc_body);
    w.write_bytes(&toc_crc.to_le_bytes());

    // Patch the header now we know toc_offset.
    let mut header = [0u8; HEADER_LEN];
    header[0..8].copy_from_slice(BUNDLE_MAGIC);
    header[8..12].copy_from_slice(&BUNDLE_FORMAT_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&0u32.to_le_bytes()); // flags
    header[16..24].copy_from_slice(&toc_offset.to_le_bytes());
    header[24..28].copy_from_slice(&2u32.to_le_bytes()); // 2 entries
    let header_crc = crc32(&header[0..28]);
    header[28..32].copy_from_slice(&header_crc.to_le_bytes());

    let mut out = w.into_vec();
    out[..HEADER_LEN].copy_from_slice(&header);

    // Trailer (sha covers everything before it).
    let bundle_sha = sha256(&out);
    out.extend_from_slice(TRAILER_MAGIC);
    out.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
    out.extend_from_slice(bundle_sha.as_bytes());
    debug_assert_eq!(out.len() - (out.len() - TRAILER_LEN), TRAILER_LEN);
    out
}

#[test]
fn unix_traversal_path_is_rejected_by_read_summary() {
    let bytes = build_hostile("../../etc/passwd");
    let err = prism_bundle::read_summary(&bytes).expect_err("must reject ..");
    match err {
        PrismError::Corrupt { detail } => {
            assert!(detail.contains("unsafe path"), "detail: {detail}");
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[test]
fn windows_traversal_path_is_rejected_by_read_summary() {
    // The decoder also rejects backslashes outright.
    let bytes = build_hostile("..\\..\\etc\\passwd");
    let err = prism_bundle::read_summary(&bytes).expect_err("must reject \\");
    match err {
        PrismError::Corrupt { detail } => {
            assert!(detail.contains("unsafe path"), "detail: {detail}");
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[test]
fn leading_slash_is_rejected_by_read_summary() {
    let bytes = build_hostile("/etc/passwd");
    let err = prism_bundle::read_summary(&bytes).expect_err("must reject /");
    assert!(matches!(err, PrismError::Corrupt { .. }), "got {err:?}");
}

#[test]
fn unpack_into_never_writes_when_path_is_hostile() {
    // Even if a future change loosened read_summary's path check,
    // unpack_into's starts_with(prism_root) guard would catch it. We re-run
    // unpack_into here for layered defense — the test passes whenever the
    // hostile path is rejected anywhere in the chain.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dest = std::env::temp_dir().join(format!(
        "prism-bundle-it-path-{}-{nanos}",
        std::process::id()
    ));
    let _cleanup = scopeguard(dest.clone());

    let bytes = build_hostile("../../etc/passwd");
    let err = prism_bundle::unpack_into(&bytes, &dest).expect_err("must reject");
    assert!(matches!(err, PrismError::Corrupt { .. }), "got {err:?}");

    // Confirm no escape file was created.
    assert!(!std::path::Path::new("/etc/passwd-pwned").exists());
}

/// Tiny RAII cleanup so we do not leak temp dirs on test failure.
fn scopeguard(p: std::path::PathBuf) -> impl Drop {
    struct G(std::path::PathBuf);
    impl Drop for G {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    G(p)
}
