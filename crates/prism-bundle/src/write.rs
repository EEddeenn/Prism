//! Bundle writer: pack a workspace's published file set into a
//! single-file `.prismbundle` image.
//!
//! ## Algorithm (single-pass with one header patch)
//!
//! 1. Read the workspace `MANIFEST` (which lists every published `.prism/` file
//!    with its authoritative length + SHA-256) via [`std::fs::read`].
//! 2. Decode the manifest with [`prism_store::Manifest::decode`] (this re-runs
//!    its CRC + path-safety checks; if it cannot decode, neither can a reader,
//!    so refuse to pack).
//! 3. Build the [`TocEntry`] list in the fixed bundle order so two packs of the
//!    same workspace are byte-identical (and the SHA-256 in the trailer is
//!    therefore reproducible):
//!
//!    - the `MANIFEST` (kind [`EntryKind::Manifest`]) is always first;
//!    - then every [`FileKind::Segment`] in ascending `rel_path` order;
//!    - then every [`FileKind::Index`];
//!    - then every [`FileKind::Graph`];
//!    - then every [`FileKind::Tda`];
//!    - [`FileKind::Candidate`] entries are skipped (staging-only, never live).
//!
//!    For each entry the writer reads the bytes off disk, recomputes the
//!    SHA-256 (defense-in-depth: never trust the in-memory manifest digest if
//!    the file on disk has rotted), and records the absolute byte range it
//!    will occupy in the bundle.
//! 4. Emit a 32-byte zero placeholder where the header will go; then each
//!    payload byte-for-byte; then the encoded TOC + 4-byte CRC; then the
//!    44-byte trailer (8-byte magic + 4-byte version + 32-byte SHA-256 over
//!    everything before the trailer).
//! 5. Patch the header in place with magic / version / flags / `toc_offset` /
//!    `toc_entry_count` / `header_crc32`.
//!
//! All offset/length arithmetic uses [`u64::checked_add`] so a hostile or
//! degenerate workspace cannot wrap a counter past the buffer end. The final
//! size is bounded by [`MAX_BUNDLE_SIZE`] and the entry count by
//! [`MAX_TOC_ENTRIES`].
//!
//! [`pack_workspace_to`] is the on-disk entry point: it writes the bundle to a
//! `.tmp` sibling, calls `sync_all`, then renames into place — so a crash
//! mid-write never leaves a half-written `.prismbundle` for anyone to load.

use std::path::Path;

use prism_codec::{crc32, sha256, ByteWriter};
use prism_model::{HashDigest, PrismError, Result};
use prism_store::{FileKind, Layout, Manifest};

use crate::error::{corrupt, out_of_bounds};
use crate::format::{
    EntryKind, BUNDLE_FORMAT_VERSION, BUNDLE_MAGIC, MAX_BUNDLE_SIZE, MAX_TOC_ENTRIES,
    TRAILER_MAGIC, TRAILER_VERSION,
};
use crate::toc::{encode_toc, TocEntry};

/// Bundle header width in bytes (see [`crate::format`]).
const HEADER_LEN: usize = 32;

/// Bundle trailer width in bytes.
const TRAILER_LEN: usize = 44;

/// The `"MANIFEST"` rel_path used for the bundle's manifest entry. Matches the
/// file name the store writes at `<root>/.prism/MANIFEST`.
const MANIFEST_REL_PATH: &str = "MANIFEST";

/// Pack a workspace into an in-memory `.prismbundle` image.
///
/// `layout` must point at a published workspace whose `MANIFEST` decodes and
/// whose referenced files are present on disk with their recorded lengths.
///
/// The returned bytes are a self-describing, hash-sealed bundle. Two packs of
/// the same workspace produce byte-identical bundles (deterministic ordering
/// and no timestamps) so the trailer SHA-256 is a stable share-id.
///
/// # Errors
///
/// - [`PrismError::Io`] for any read failure on `MANIFEST` or a referenced file.
/// - [`PrismError::Corrupt`] if the manifest decodes but lists a file whose
///   on-disk length does not match (so we never bundle a torn-write).
/// - [`PrismError::Other`] if the resulting bundle would exceed
///   [`MAX_BUNDLE_SIZE`] or the entry count would exceed [`MAX_TOC_ENTRIES`],
///   or if an offset arithmetic overflow is detected.
pub fn pack_workspace(layout: &Layout) -> Result<Vec<u8>> {
    // (1) Read MANIFEST bytes.
    let manifest_path = layout.manifest();
    let manifest_bytes = std::fs::read(&manifest_path).map_err(PrismError::Io)?;

    // (2) Decode to recover the file list. A non-decodable MANIFEST is itself a
    // packing error: we refuse to bundle a workspace a reader could not open.
    let manifest = Manifest::decode(&manifest_bytes)?;

    // (3) Collect TocEntry candidates. We materialize the payload bytes only
    // after we know the order and counts (one read per file, no rebuffering).
    let mut planned: Vec<PlannedEntry> = Vec::with_capacity(manifest.files.len() + 1);

    // (3a) Manifest first.
    let manifest_sha = sha256(&manifest_bytes);
    planned.push(PlannedEntry {
        kind: EntryKind::Manifest,
        rel_path: MANIFEST_REL_PATH.to_string(),
        bytes: manifest_bytes,
        sha256: manifest_sha,
    });

    // (3b) Then segments → indexes → graphs → tda, each group ascending by
    // rel_path. Candidate entries are skipped (they are staging-only and
    // should never appear in a healthy manifest, but defense in depth).
    for kind in [FileKind::Segment, FileKind::Index, FileKind::Graph, FileKind::Tda] {
        let mut group: Vec<&prism_store::FileEntry> = manifest
            .files
            .iter()
            .filter(|f| f.kind == kind)
            .collect();
        group.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

        for entry in group {
            // entry.rel_path is validated by Manifest::decode itself; we still
            // never trust it past joining against prism_dir().
            let path = layout.prism_dir().join(&entry.rel_path);
            let bytes = std::fs::read(&path).map_err(PrismError::Io)?;
            if bytes.len() as u64 != entry.len {
                return Err(corrupt(format!(
                    "{}: on-disk length {} != manifest length {}",
                    entry.rel_path,
                    bytes.len(),
                    entry.len
                )));
            }
            let sha = sha256(&bytes);
            // We could compare to entry.digest here, but the load path
            // already does that and refusing to pack on rotted bytes is a
            // separate user-visible signal. Trust on-disk content for the
            // bundle digest and let the reader catch any mismatch.
            let _ = entry.digest;
            planned.push(PlannedEntry {
                kind: bundle_kind_for(kind)?,
                rel_path: entry.rel_path.clone(),
                bytes,
                sha256: sha,
            });
        }
    }

    if planned.len() as u64 > MAX_TOC_ENTRIES as u64 {
        return Err(PrismError::Other(format!(
            "bundle: toc entry count {} exceeds MAX_TOC_ENTRIES {}",
            planned.len(),
            MAX_TOC_ENTRIES
        )));
    }

    // (4) Emit header placeholder + payloads + TOC + trailer into a single
    // ByteWriter. We patch the header bytes at the very end once we know the
    // toc_offset.
    let total_payload_len: u64 = planned
        .iter()
        .try_fold(0u64, |acc, p| acc.checked_add(p.bytes.len() as u64))
        .ok_or_else(|| out_of_bounds("payload byte total overflowed u64"))?;
    // Conservative pre-size: header + payload + an upper-bound TOC + trailer.
    let toc_upper_bound: usize = planned
        .iter()
        .map(|p| 52 + p.rel_path.len())
        .sum::<usize>()
        + 4;
    let cap = HEADER_LEN
        .saturating_add(usize::try_from(total_payload_len).unwrap_or(usize::MAX))
        .saturating_add(toc_upper_bound)
        .saturating_add(TRAILER_LEN);
    let mut w = ByteWriter::with_capacity(cap);

    // (4a) Header placeholder.
    w.write_bytes(&[0u8; HEADER_LEN]);

    // (4b) Payloads, recording the absolute offset of each.
    let mut toc_entries: Vec<TocEntry> = Vec::with_capacity(planned.len());
    for p in &planned {
        let offset = w.len() as u64;
        // Sanity: offset + len must not overflow and must stay under the cap
        // we will enforce at the end. (The MAX_BUNDLE_SIZE check below is the
        // authoritative one — this is purely an arithmetic-overflow guard.)
        let _end = offset
            .checked_add(p.bytes.len() as u64)
            .ok_or_else(|| out_of_bounds("payload end offset overflowed u64"))?;
        w.write_bytes(&p.bytes);
        toc_entries.push(TocEntry {
            kind: p.kind,
            rel_path: p.rel_path.clone(),
            payload_offset: offset,
            payload_len: p.bytes.len() as u64,
            sha256: p.sha256,
        });
    }

    // (4c) TOC at toc_offset.
    let toc_offset = w.len() as u64;
    let toc_bytes = encode_toc(&toc_entries);
    w.write_bytes(&toc_bytes);

    // (5) Patch the header *now*, BEFORE computing the trailer SHA. The
    // trailer SHA must cover the final on-disk bytes [0 .. trailer_start),
    // which includes the real (non-zero) header.
    let mut header = [0u8; HEADER_LEN];
    header[0..8].copy_from_slice(BUNDLE_MAGIC);
    header[8..12].copy_from_slice(&BUNDLE_FORMAT_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&0u32.to_le_bytes()); // flags
    header[16..24].copy_from_slice(&toc_offset.to_le_bytes());
    let toc_entry_count: u32 = u32::try_from(toc_entries.len()).map_err(|_| {
        PrismError::Other(format!(
            "bundle: toc entry count {} does not fit in u32",
            toc_entries.len()
        ))
    })?;
    header[24..28].copy_from_slice(&toc_entry_count.to_le_bytes());
    let header_crc = crc32(&header[0..28]);
    header[28..32].copy_from_slice(&header_crc.to_le_bytes());

    let mut out = w.into_vec();
    out[..HEADER_LEN].copy_from_slice(&header);

    // (4d) Trailer: magic + version + sha256 over everything-before-trailer
    // (i.e. the patched header, payloads, and TOC). Append after the patch
    // so the digest is over the final image.
    let trailer_start = out.len();
    let bundle_sha = sha256(&out); // bytes [0 .. trailer_start)
    out.extend_from_slice(TRAILER_MAGIC);
    out.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
    out.extend_from_slice(bundle_sha.as_bytes());
    debug_assert_eq!(out.len() - trailer_start, TRAILER_LEN);

    // Final size bound. This is the last-line defense; payload reads are
    // bounded above by the workspace contents, but a giant workspace would
    // otherwise produce a bundle no reader will accept.
    if out.len() as u64 > MAX_BUNDLE_SIZE {
        return Err(PrismError::Other(format!(
            "bundle: total size {} exceeds MAX_BUNDLE_SIZE {}",
            out.len(),
            MAX_BUNDLE_SIZE
        )));
    }

    Ok(out)
}

/// Pack a workspace and atomically publish the bundle to `out_path`.
///
/// Writes to `<out_path>.tmp`, calls `File::sync_all` (so the bundle bytes are
/// on stable storage), then renames the staged file over `out_path` so a crash
/// mid-write can never leave a half-written `.prismbundle` for someone else to
/// load.
///
/// The parent directory of `out_path` must already exist.
pub fn pack_workspace_to(out_path: &Path, layout: &Layout) -> Result<()> {
    let bytes = pack_workspace(layout)?;
    let tmp_path = staging_path(out_path);

    // Best-effort cleanup of any stale .tmp from a previous crashed run; we
    // ignore failures because the create-and-rename below is the source of
    // truth.
    let _ = std::fs::remove_file(&tmp_path);

    use std::fs::OpenOptions;
    use std::io::Write;
    let mut f = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp_path)
        .map_err(PrismError::Io)?;
    f.write_all(&bytes).map_err(PrismError::Io)?;
    f.sync_all().map_err(PrismError::Io)?;
    drop(f);

    std::fs::rename(&tmp_path, out_path).map_err(PrismError::Io)?;
    Ok(())
}

/// Compute the staging-file name `<out>.tmp` next to `out_path`. The file
/// lives in the same directory as `out_path` so the final `rename` is
/// guaranteed to stay on a single filesystem.
fn staging_path(out_path: &Path) -> std::path::PathBuf {
    let mut name = out_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    out_path
        .parent()
        .map(|p| p.join(&name))
        .unwrap_or_else(|| std::path::PathBuf::from(&name))
}

/// Translate a manifest [`FileKind`] into its bundle [`EntryKind`].
///
/// `FileKind::Candidate` is staging-only and must never reach a published
/// bundle. The caller filters it before we get here; defense-in-depth, we
/// return [`PrismError::Other`] rather than papering over a future
/// refactor by silently aliasing it to `Manifest` (which would surface as
/// a second-MANIFEST corruption error at read time, far from the cause).
fn bundle_kind_for(kind: FileKind) -> Result<EntryKind> {
    Ok(match kind {
        FileKind::Segment => EntryKind::Segment,
        FileKind::Index => EntryKind::Index,
        FileKind::Graph => EntryKind::Graph,
        FileKind::Tda => EntryKind::Tda,
        FileKind::Candidate => {
            return Err(PrismError::Other(
                "bundle: Candidate file kind must not be packed (staging only)".into(),
            ));
        }
    })
}

/// In-memory record bridging "what I want to write" and "what I write".
struct PlannedEntry {
    kind: EntryKind,
    rel_path: String,
    bytes: Vec<u8>,
    sha256: HashDigest,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toc::decode_toc;
    use prism_store::{RawRecord, Store};

    #[test]
    fn bundle_kind_for_translates_publishable_kinds() {
        assert!(matches!(bundle_kind_for(FileKind::Segment), Ok(EntryKind::Segment)));
        assert!(matches!(bundle_kind_for(FileKind::Index), Ok(EntryKind::Index)));
        assert!(matches!(bundle_kind_for(FileKind::Graph), Ok(EntryKind::Graph)));
        assert!(matches!(bundle_kind_for(FileKind::Tda), Ok(EntryKind::Tda)));
    }

    #[test]
    fn bundle_kind_for_rejects_candidate() {
        // Defense-in-depth: a future change to the caller filter must NOT
        // silently smuggle a Candidate (staging-only) entry into a bundle.
        match bundle_kind_for(FileKind::Candidate) {
            Err(PrismError::Other(m)) => assert!(m.contains("Candidate"), "{m}"),
            other => panic!("expected Other(\"Candidate ...\"), got {other:?}"),
        }
    }

    /// A throwaway directory rooted under `std::env::temp_dir()`. We do not
    /// pull in any dev-dep, so this is a hand-rolled `TempDir` that mirrors
    /// the one prism-store's own tests use.
    struct TempDir {
        path: std::path::PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> TempDir {
            // Pid + nanos give a per-process, per-call unique name without
            // needing rand. Matches prism_store::test_support.
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "prism-bundle-write-{label}-{}-{nanos}",
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

    fn build_tiny_store(td: &TempDir) {
        let mut s = Store::init(td.path()).unwrap();
        s.publish_segment(&[
            RawRecord::new(1, b"react".to_vec(), b"npm:react".to_vec()),
            RawRecord::new(1, b"serde".to_vec(), b"cargo:serde".to_vec()),
        ])
        .unwrap();
    }

    #[test]
    fn pack_emits_header_magic_version_and_crc() {
        let td = TempDir::new("hdr");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let bytes = pack_workspace(&layout).expect("pack");

        // Header magic + version.
        assert_eq!(&bytes[0..8], BUNDLE_MAGIC);
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            BUNDLE_FORMAT_VERSION
        );
        // Flags = 0 in v1.
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 0);
        // Header CRC over [0..28).
        let want = crc32(&bytes[0..28]);
        assert_eq!(u32::from_le_bytes(bytes[28..32].try_into().unwrap()), want);
    }

    #[test]
    fn pack_emits_trailer_magic_version_and_whole_bundle_sha() {
        let td = TempDir::new("trailer");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let bytes = pack_workspace(&layout).expect("pack");

        let n = bytes.len();
        assert!(n >= TRAILER_LEN);
        let trailer = &bytes[n - TRAILER_LEN..];
        assert_eq!(&trailer[0..8], TRAILER_MAGIC);
        assert_eq!(
            u32::from_le_bytes(trailer[8..12].try_into().unwrap()),
            TRAILER_VERSION
        );
        // Trailer sha256 must match the digest of everything before the trailer.
        let want = sha256(&bytes[..n - TRAILER_LEN]);
        assert_eq!(&trailer[12..44], want.as_bytes());
    }

    #[test]
    fn toc_offset_in_header_points_at_decodable_toc() {
        let td = TempDir::new("toc");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let bytes = pack_workspace(&layout).expect("pack");

        let toc_offset = u64::from_le_bytes(bytes[16..24].try_into().unwrap()) as usize;
        let toc_count = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        let trailer_start = bytes.len() - TRAILER_LEN;
        assert!(toc_offset < trailer_start);

        let toc_slice = &bytes[toc_offset..trailer_start];
        let entries = decode_toc(toc_slice, toc_count).expect("decode toc");
        assert_eq!(entries.len() as u32, toc_count);

        // The first entry is always the MANIFEST.
        assert_eq!(entries[0].kind, EntryKind::Manifest);
        assert_eq!(entries[0].rel_path, MANIFEST_REL_PATH);
    }

    #[test]
    fn entries_in_fixed_order_segments_then_indexes_then_graphs_then_tda() {
        // Build a workspace with a graph, then ensure the bundle TOC orders
        // Manifest → Segments → Indexes → Graphs → (no Tda in this fixture).
        let td = TempDir::new("order");
        let mut s = Store::init(td.path()).unwrap();
        s.publish_segment(&[RawRecord::new(1, b"k".to_vec(), b"v".to_vec())]).unwrap();
        s.publish_graph(prism_model::RunId::new(42), b"PRISMCSR-fake").unwrap();
        drop(s);

        let layout = Layout::new(td.path());
        let bytes = pack_workspace(&layout).expect("pack");
        let toc_offset = u64::from_le_bytes(bytes[16..24].try_into().unwrap()) as usize;
        let toc_count = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        let trailer_start = bytes.len() - TRAILER_LEN;
        let entries = decode_toc(&bytes[toc_offset..trailer_start], toc_count).expect("decode");

        // First entry is the manifest.
        assert_eq!(entries[0].kind, EntryKind::Manifest);

        // Kinds must appear in this monotonically-ordered sequence:
        // 5 (Manifest), then all 0 (Segment), then all 1 (Index), then all 2
        // (Graph), then all 3 (Tda). We collect the kinds in order and check
        // the sequence is non-decreasing under that custom rank.
        let rank = |k: EntryKind| -> u8 {
            match k {
                EntryKind::Manifest => 0,
                EntryKind::Segment => 1,
                EntryKind::Index => 2,
                EntryKind::Graph => 3,
                EntryKind::Tda => 4,
            }
        };
        let ranks: Vec<u8> = entries.iter().map(|e| rank(e.kind)).collect();
        let mut sorted = ranks.clone();
        sorted.sort();
        assert_eq!(ranks, sorted, "TOC kinds must be in canonical order");

        // We must see at least one Segment, one Index, one Graph in this fixture.
        assert!(entries.iter().any(|e| e.kind == EntryKind::Segment));
        assert!(entries.iter().any(|e| e.kind == EntryKind::Index));
        assert!(entries.iter().any(|e| e.kind == EntryKind::Graph));
    }

    #[test]
    fn payload_offsets_match_file_bytes_byte_for_byte() {
        let td = TempDir::new("offsets");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let bytes = pack_workspace(&layout).expect("pack");

        let toc_offset = u64::from_le_bytes(bytes[16..24].try_into().unwrap()) as usize;
        let toc_count = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        let trailer_start = bytes.len() - TRAILER_LEN;
        let entries = decode_toc(&bytes[toc_offset..trailer_start], toc_count).expect("decode");

        for e in &entries {
            let start = e.payload_offset as usize;
            let end = start + e.payload_len as usize;
            assert!(start >= HEADER_LEN, "payload starts before header end");
            assert!(end <= toc_offset, "payload runs into TOC");
            let in_bundle = &bytes[start..end];

            // Recompute sha and compare to TOC entry.
            assert_eq!(sha256(in_bundle), e.sha256, "TOC sha mismatch for {}", e.rel_path);

            // Compare to the live workspace bytes.
            let on_disk = if e.rel_path == MANIFEST_REL_PATH {
                std::fs::read(layout.manifest()).unwrap()
            } else {
                std::fs::read(layout.prism_dir().join(&e.rel_path)).unwrap()
            };
            assert_eq!(in_bundle, on_disk.as_slice(), "payload bytes != on-disk for {}", e.rel_path);
        }
    }

    #[test]
    fn pack_workspace_to_writes_atomically_and_matches_pack_workspace() {
        let td = TempDir::new("atomic");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());

        let in_mem = pack_workspace(&layout).expect("pack");

        // Pack to a sibling temp file.
        let out_dir = TempDir::new("out");
        let out = out_dir.path().join("snapshot.prismbundle");
        pack_workspace_to(&out, &layout).expect("pack_to");

        let on_disk = std::fs::read(&out).expect("read packed");
        assert_eq!(in_mem, on_disk);

        // The staging file must not exist after a successful publish.
        let staged = staging_path(&out);
        assert!(!staged.exists(), "tmp staging file left behind: {staged:?}");
    }

    #[test]
    fn two_packs_of_the_same_workspace_are_byte_identical() {
        // Determinism: the trailer sha is a stable share-id.
        let td = TempDir::new("determinism");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let a = pack_workspace(&layout).expect("pack a");
        let b = pack_workspace(&layout).expect("pack b");
        assert_eq!(a, b);
    }

    #[test]
    fn header_crc_validates_byte_layout() {
        let td = TempDir::new("crc");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let bytes = pack_workspace(&layout).expect("pack");
        // Flipping a header byte must break the CRC32 (we are not testing the
        // reader here, just that the writer's CRC covers the documented range).
        let mut perturbed = bytes.clone();
        perturbed[10] ^= 0x80;
        assert_ne!(crc32(&perturbed[0..28]), u32::from_le_bytes(perturbed[28..32].try_into().unwrap()));
    }

    #[test]
    fn manifest_sha_in_toc_matches_on_disk_manifest() {
        let td = TempDir::new("manifest-sha");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let bytes = pack_workspace(&layout).expect("pack");
        let toc_offset = u64::from_le_bytes(bytes[16..24].try_into().unwrap()) as usize;
        let toc_count = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        let trailer_start = bytes.len() - TRAILER_LEN;
        let entries = decode_toc(&bytes[toc_offset..trailer_start], toc_count).expect("decode");

        let manifest_entry = entries.iter().find(|e| e.kind == EntryKind::Manifest).expect("manifest entry");
        let manifest_bytes = std::fs::read(layout.manifest()).expect("read manifest");
        assert_eq!(manifest_entry.sha256, sha256(&manifest_bytes));
    }

    #[test]
    fn staging_path_lives_next_to_target() {
        let p = staging_path(Path::new("/tmp/foo/bar.prismbundle"));
        assert_eq!(p, Path::new("/tmp/foo/bar.prismbundle.tmp"));
        // No parent → still keeps the .tmp suffix in the current dir.
        let p2 = staging_path(Path::new("bare.prismbundle"));
        assert_eq!(p2, Path::new("bare.prismbundle.tmp"));
    }
}
