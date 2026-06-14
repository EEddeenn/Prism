//! End-to-end round-trip: build a tiny workspace via `prism_store::Store`,
//! pack it into a `.prismbundle`, unpack into a fresh directory, and confirm
//! `Store::open_readonly` succeeds on the unpacked workspace with byte-identical
//! manifest + segment bytes.
//!
//! We deliberately avoid pulling in `prism-api` so the test surface is the
//! bundle format + the store, nothing else.

use std::path::PathBuf;

use prism_bundle::{pack_workspace, pack_workspace_to, read_summary, unpack_into, EntryKind};
use prism_store::{Layout, RawRecord, Store};

/// Hand-rolled tempdir — no external dev-deps allowed in this hermetic crate.
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
        let seq = TEMPDIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "prism-bundle-it-roundtrip-{label}-{}-{nanos}-{seq}",
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

/// Seed a workspace with two records plus a graph so the manifest exercises
/// every file class the bundle pack/unpack pipeline cares about (Manifest +
/// Segment + Index + Graph).
fn seed(td: &TempDir) {
    let mut s = Store::init(td.path()).expect("init");
    s.publish_segment(&[
        RawRecord::new(1, b"react".to_vec(), b"npm:react".to_vec()),
        RawRecord::new(1, b"serde".to_vec(), b"cargo:serde".to_vec()),
    ])
    .expect("publish");
    // publish_graph appends both a Graph file and a fresh Index segment so
    // the round-trip touches every group the bundle writer orders.
    s.publish_graph(prism_model::RunId::new(1), b"PRISMCSR-fake-graph-bytes")
        .expect("publish graph");
}

#[test]
fn pack_then_unpack_yields_byte_identical_files() {
    let src = TempDir::new("src");
    seed(&src);
    let src_layout = Layout::new(src.path());

    // (1) Pack via the in-memory writer.
    let bundle_bytes = pack_workspace(&src_layout).expect("pack");

    // (2) Sanity: read_summary accepts the writer's output.
    let summary = read_summary(&bundle_bytes).expect("read_summary");
    assert!(summary.entries.iter().any(|e| e.kind == EntryKind::Manifest));
    assert!(summary.entries.iter().any(|e| e.kind == EntryKind::Segment));
    assert!(summary.entries.iter().any(|e| e.kind == EntryKind::Index));

    // (3) Unpack into a fresh directory tree.
    let dst = TempDir::new("dst");
    let unpacked_summary = unpack_into(&bundle_bytes, dst.path()).expect("unpack");
    assert_eq!(unpacked_summary.bundle_sha256_hex, summary.bundle_sha256_hex);

    // (4) Every file the bundle TOC named must exist under <dst>/.prism/ and
    // its bytes must equal the source's bytes (byte-verbatim contract).
    for entry in &summary.entries {
        let in_src = if entry.rel_path == "MANIFEST" {
            std::fs::read(src_layout.manifest()).expect("read src manifest")
        } else {
            std::fs::read(src_layout.prism_dir().join(&entry.rel_path)).expect("read src file")
        };
        let in_dst = if entry.rel_path == "MANIFEST" {
            std::fs::read(dst.path().join(".prism/MANIFEST")).expect("read dst manifest")
        } else {
            std::fs::read(dst.path().join(".prism").join(&entry.rel_path))
                .expect("read dst file")
        };
        assert_eq!(
            in_src, in_dst,
            "byte mismatch for {}: src len {}, dst len {}",
            entry.rel_path,
            in_src.len(),
            in_dst.len()
        );
    }
}

#[test]
fn unpacked_workspace_reopens_via_store() {
    let src = TempDir::new("reopen-src");
    seed(&src);
    let src_layout = Layout::new(src.path());

    let bundle_bytes = pack_workspace(&src_layout).expect("pack");
    let dst = TempDir::new("reopen-dst");
    unpack_into(&bundle_bytes, dst.path()).expect("unpack");

    // The store's load path re-verifies the manifest, per-file lengths, and
    // per-file SHA-256. If any of that fails the open returns an error and
    // this test fails.
    let store = Store::open_readonly(dst.path()).expect("open_readonly");
    assert!(store.is_read_only());
    assert!(!store.manifest().files.is_empty());

    // Source and destination manifests must agree on the file list.
    let src_store = Store::open_readonly(src.path()).expect("open src");
    assert_eq!(
        src_store.manifest().files.len(),
        store.manifest().files.len()
    );
}

#[test]
fn pack_workspace_to_file_matches_in_memory_pack() {
    let src = TempDir::new("ondisk");
    seed(&src);
    let src_layout = Layout::new(src.path());

    let in_mem = pack_workspace(&src_layout).expect("pack");

    let dst = TempDir::new("ondisk-out");
    let out = dst.path().join("snapshot.prismbundle");
    pack_workspace_to(&out, &src_layout).expect("pack_to");
    let on_disk = std::fs::read(&out).expect("read packed");
    assert_eq!(in_mem, on_disk);
}
