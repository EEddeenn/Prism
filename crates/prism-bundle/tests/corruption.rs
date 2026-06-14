//! Pack a real bundle and tamper with one byte in each region. Every
//! tamper must produce a specific [`PrismError`] variant — not a panic,
//! not silent success.

use std::path::PathBuf;

use prism_bundle::{pack_workspace, read_summary};
use prism_model::PrismError;
use prism_store::{Layout, RawRecord, Store};

struct TempDir {
    path: PathBuf,
}

static TEMPDIR_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl TempDir {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        // Atomic counter avoids collisions when two tests resolve `now()`
        // to the same nanos (rare but observed on fast CI hardware).
        let seq = TEMPDIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "prism-bundle-it-corruption-{label}-{}-{nanos}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create tempdir");
        TempDir { path }
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Build a sample bundle bound to `label`. Each caller passes a distinct
/// label so concurrent tests do not collide on the tempdir path (the nanos
/// suffix alone is not enough — same process, sub-microsecond test scheduling).
/// We must also hold the `TempDir` until *after* `pack_workspace` returns, so
/// the workspace files still exist when we read them.
fn pack_sample(label: &str) -> Vec<u8> {
    let td = TempDir::new(label);
    let mut s = Store::init(td.path()).expect("init");
    s.publish_segment(&[
        RawRecord::new(1, b"react".to_vec(), b"npm:react".to_vec()),
        RawRecord::new(1, b"serde".to_vec(), b"cargo:serde".to_vec()),
    ])
    .expect("publish");
    drop(s);
    let layout = Layout::new(td.path());
    let bytes = pack_workspace(&layout).expect("pack");
    // td is dropped here; the bundle bytes are now in memory and detached.
    bytes
}

#[test]
fn header_magic_flip_yields_corrupt() {
    let mut bytes = pack_sample("hmagic");
    // Flip a magic byte. Must be Corrupt (the envelope check fires before any
    // checksum recompute).
    bytes[0] ^= 0xff;
    let err = read_summary(&bytes).expect_err("must reject");
    assert!(matches!(err, PrismError::Corrupt { .. }), "got {err:?}");
}

#[test]
fn payload_middle_byte_flip_yields_checksum_mismatch() {
    let mut bytes = pack_sample("pmiddle");
    // The trailer envelope (whole-bundle SHA-256) catches any payload tamper
    // before the per-entry SHA loop runs.
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0x01;
    let err = read_summary(&bytes).expect_err("must reject");
    assert!(
        matches!(err, PrismError::ChecksumMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn toc_body_byte_flip_yields_checksum_or_corrupt() {
    let mut bytes = pack_sample("tocbody");
    // Locate the TOC region from the header.
    let toc_offset = u64::from_le_bytes(bytes[16..24].try_into().unwrap()) as usize;
    // Flip a byte inside the TOC body. The whole-bundle SHA in the trailer
    // catches this first (ChecksumMismatch); even if it did not, the TOC's own
    // CRC32 would fire as Corrupt. Either is acceptable defensive behavior.
    bytes[toc_offset + 4] ^= 0x80;
    let err = read_summary(&bytes).expect_err("must reject");
    assert!(
        matches!(
            err,
            PrismError::ChecksumMismatch { .. } | PrismError::Corrupt { .. }
        ),
        "got {err:?}"
    );
}

#[test]
fn trailer_sha_byte_flip_yields_checksum_mismatch() {
    let mut bytes = pack_sample("tsha");
    let n = bytes.len();
    // Flip the last byte of the trailer SHA-256.
    bytes[n - 1] ^= 0x01;
    let err = read_summary(&bytes).expect_err("must reject");
    assert!(
        matches!(err, PrismError::ChecksumMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn trailer_magic_flip_yields_corrupt() {
    let mut bytes = pack_sample("tmagic");
    let n = bytes.len();
    bytes[n - 44] ^= 0xff;
    let err = read_summary(&bytes).expect_err("must reject");
    assert!(matches!(err, PrismError::Corrupt { .. }), "got {err:?}");
}

#[test]
fn header_crc_byte_flip_yields_corrupt() {
    let mut bytes = pack_sample("hcrc");
    // Flip the last header-CRC byte.
    bytes[31] ^= 0x01;
    let err = read_summary(&bytes).expect_err("must reject");
    assert!(matches!(err, PrismError::Corrupt { .. }), "got {err:?}");
}
