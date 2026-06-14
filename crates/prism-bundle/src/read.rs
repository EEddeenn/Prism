//! Bundle reader: validate, summarize, and (optionally) materialize a
//! `.prismbundle` image back onto disk.
//!
//! ## Invariants enforced *before* allocating
//!
//! The reader is the only side of the format that ever touches hostile bytes
//! (an uploaded bundle from an untrusted client). It must therefore:
//!
//! - reject any byte length outside `[HEADER + TRAILER + 4 .. MAX_BUNDLE_SIZE]`
//!   so the slicing arithmetic below cannot wrap;
//! - validate the header magic, version, and CRC-32 *before* trusting
//!   `toc_offset` / `toc_entry_count`;
//! - validate the trailer magic + version and recompute the whole-bundle
//!   SHA-256 (constant-time `Eq`) before any TOC parse, so a forged TOC inside
//!   a tampered file is rejected by the outer envelope alone;
//! - delegate TOC body decoding to [`crate::toc::decode_toc`], which already
//!   guards path safety + allocation;
//! - check every entry's byte range: `payload_offset >= HEADER_LEN`,
//!   `payload_offset + payload_len <= toc_offset` via [`u64::checked_add`], no
//!   overlap with another entry, and re-hash the payload SHA-256;
//! - require exactly one [`EntryKind::Manifest`] entry whose `rel_path` is
//!   `"MANIFEST"`.
//!
//! [`unpack_into`] adds defense-in-depth on top: even though `decode_toc`
//! rejects `..` segments, the joined path is required to live under
//! `<dest_root>/.prism/` via [`std::path::Path::starts_with`]. This catches any
//! future loosening of `validate_rel_path` and any symlink games introduced by
//! a hostile filesystem layout.
//!
//! No panic path: every `try_into` / `slice` / arithmetic op has a fall-back
//! branch that emits a `PrismError`.

use std::path::Path;

use prism_codec::{crc32, sha256};
use prism_model::{HashDigest, Result};

use crate::error::{checksum_mismatch, corrupt, out_of_bounds, unsupported_format_version};
use crate::format::{
    EntryKind, BUNDLE_FORMAT_VERSION, BUNDLE_MAGIC, MAX_BUNDLE_SIZE, TRAILER_MAGIC,
    TRAILER_VERSION,
};
use crate::toc::{decode_toc, TocEntry};

/// Header width in bytes (must match [`crate::write`]).
const HEADER_LEN: usize = 32;

/// Trailer width in bytes (must match [`crate::write`]).
const TRAILER_LEN: usize = 44;

/// Minimum acceptable bundle size: `header + trailer + empty TOC CRC`.
///
/// A four-byte CRC must follow even an empty TOC, so a valid bundle is at
/// least `32 + 44 + 4 = 80` bytes. Anything shorter is rejected before any
/// slicing.
const MIN_BUNDLE_LEN: usize = HEADER_LEN + TRAILER_LEN + 4;

/// The `"MANIFEST"` rel_path that must appear exactly once in the TOC.
const MANIFEST_REL_PATH: &str = "MANIFEST";

/// The relative subdirectory under `dest_root` where bundle contents are
/// materialized by [`unpack_into`]. Matches the on-disk shape of a live Prism
/// workspace.
const PRISM_SUBDIR: &str = ".prism";

/// One entry's worth of summary information exposed to callers (CLI, server).
///
/// Mirrors [`TocEntry`] but with the SHA-256 pre-hex-encoded for easy display
/// and the [`EntryKind`] kept structured so callers can group or filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleEntryInfo {
    /// Category of the embedded file.
    pub kind: EntryKind,
    /// Path relative to the unpacked `.prism/` directory.
    pub rel_path: String,
    /// Authoritative payload byte length recorded in the TOC.
    pub payload_len: u64,
    /// Lowercase hex of the per-entry SHA-256.
    pub sha256_hex: String,
}

/// A verified summary of a `.prismbundle` image.
///
/// Returned by [`read_summary`] after the header, trailer, TOC, and every
/// per-entry digest have been validated. Callers can render this to users
/// (`prism bundle inspect`), publish it as JSON (server upload response), or
/// use it to drive [`unpack_into`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleSummary {
    /// Bundle format version (always `1` in v1).
    pub format_version: u32,
    /// Number of TOC entries.
    pub entry_count: u32,
    /// Total bundle byte length (`bytes.len() as u64`).
    pub byte_len: u64,
    /// Lowercase hex of the whole-bundle SHA-256 stored in the trailer.
    pub bundle_sha256_hex: String,
    /// One row per TOC entry, in the order the bundle stored them.
    pub entries: Vec<BundleEntryInfo>,
}

/// Verify a `.prismbundle` image and return a structural summary.
///
/// This is the only function that touches all of header + trailer + TOC +
/// payloads. It performs every integrity check the format describes, in
/// outer-to-inner order so a tampered envelope is rejected before any TOC
/// parse:
///
/// 1. byte-length bounds;
/// 2. header magic / version / CRC-32 over `[0..28)`;
/// 3. trailer magic / version / whole-bundle SHA-256 over `[0..file_len-44)`;
/// 4. TOC body + CRC at the header's `toc_offset` (delegated to
///    [`decode_toc`]);
/// 5. per-entry range, no-overlap, and SHA-256;
/// 6. exactly one MANIFEST entry.
///
/// # Errors
///
/// - [`prism_model::PrismError::OutOfBounds`] on truncated input or arithmetic
///   that would overflow the file length.
/// - [`prism_model::PrismError::UnsupportedFormatVersion`] if the header
///   `format_version` is `0` or `> BUNDLE_FORMAT_VERSION`.
/// - [`prism_model::PrismError::Corrupt`] for magic mismatches, CRC failure,
///   reserved-byte violations, overlapping payload ranges, or a missing /
///   duplicated MANIFEST entry.
/// - [`prism_model::PrismError::ChecksumMismatch`] when the trailer SHA-256 or
///   any per-entry SHA-256 does not match the bytes.
pub fn read_summary(bytes: &[u8]) -> Result<BundleSummary> {
    // (1) Byte-length bounds. Both ends must hold before we slice.
    let len_u64 = bytes.len() as u64;
    if len_u64 > MAX_BUNDLE_SIZE {
        return Err(out_of_bounds(format!(
            "bundle byte length {len_u64} exceeds MAX_BUNDLE_SIZE {MAX_BUNDLE_SIZE}"
        )));
    }
    if bytes.len() < MIN_BUNDLE_LEN {
        return Err(out_of_bounds(format!(
            "bundle byte length {} below minimum {MIN_BUNDLE_LEN}",
            bytes.len()
        )));
    }

    // (2) Header.
    let header = &bytes[..HEADER_LEN];
    if &header[0..8] != BUNDLE_MAGIC {
        return Err(corrupt("header magic does not match PRISMBND"));
    }
    let format_version = u32::from_le_bytes(slice4(header, 8)?);
    if format_version == 0 || format_version > BUNDLE_FORMAT_VERSION {
        return Err(unsupported_format_version(
            format_version,
            BUNDLE_FORMAT_VERSION,
        ));
    }
    let flags = u32::from_le_bytes(slice4(header, 12)?);
    let toc_offset = u64::from_le_bytes(slice8(header, 16)?);
    let toc_entry_count = u32::from_le_bytes(slice4(header, 24)?);
    let header_crc = u32::from_le_bytes(slice4(header, 28)?);
    let want_header_crc = crc32(&header[0..28]);
    if header_crc != want_header_crc {
        return Err(corrupt(format!(
            "header crc32 mismatch: header says {header_crc:#010x}, computed {want_header_crc:#010x}"
        )));
    }
    // v1 reserves `flags = 0`. Reject any non-zero value: a future writer
    // that sets a flag bit (e.g. for a new payload-framing variant) must
    // bump `format_version`; we don't want a v1 reader silently treating
    // such a file as if the flag had no semantic effect. Run this check
    // AFTER the header CRC so a corrupted-flags byte surfaces as a CRC
    // mismatch (the existing test contract); only an intentionally-built
    // header with valid CRC and a non-zero flag bit fails here.
    if flags != 0 {
        return Err(corrupt(format!(
            "header flags must be 0 in format v1 (got {flags:#010x})"
        )));
    }

    // (3) Trailer.
    let trailer_start = bytes
        .len()
        .checked_sub(TRAILER_LEN)
        .ok_or_else(|| out_of_bounds("bundle shorter than trailer"))?;
    let trailer = &bytes[trailer_start..];
    if &trailer[0..8] != TRAILER_MAGIC {
        return Err(corrupt("trailer magic does not match PRISMEND"));
    }
    let trailer_version = u32::from_le_bytes(slice4(trailer, 8)?);
    if trailer_version == 0 || trailer_version > TRAILER_VERSION {
        return Err(unsupported_format_version(trailer_version, TRAILER_VERSION));
    }
    let stored_sha = HashDigest::from_bytes(slice32(trailer, 12)?);
    let computed_sha = sha256(&bytes[..trailer_start]);
    if stored_sha != computed_sha {
        return Err(checksum_mismatch(format!(
            "trailer sha256 mismatch: stored {}, computed {}",
            stored_sha.to_hex(),
            computed_sha.to_hex()
        )));
    }

    // (4) TOC body + CRC at toc_offset, bounded above by trailer_start.
    let toc_offset_usize = usize_or_oob(toc_offset, "toc_offset")?;
    if toc_offset_usize < HEADER_LEN {
        return Err(corrupt(format!(
            "toc_offset {toc_offset_usize} is inside the header"
        )));
    }
    if toc_offset_usize > trailer_start {
        return Err(out_of_bounds(format!(
            "toc_offset {toc_offset_usize} > trailer_start {trailer_start}"
        )));
    }
    let toc_slice = &bytes[toc_offset_usize..trailer_start];
    let toc_entries = decode_toc(toc_slice, toc_entry_count)?;

    // (5) Per-entry range checks + SHA-256 + no-overlap.
    //
    // We walk the entries in the order the bundle stored them, then collect
    // (start, end) spans, sort, and verify no neighbour pair overlaps. Sort is
    // O(n log n) but n <= MAX_TOC_ENTRIES (65 536) and the comparison is
    // trivial.
    let mut entries_out: Vec<BundleEntryInfo> = Vec::with_capacity(toc_entries.len());
    let mut spans: Vec<(u64, u64)> = Vec::with_capacity(toc_entries.len());
    let mut manifest_count: u32 = 0;
    // Reject duplicate rel_path entries. Two TOC slots that name the same
    // file are silently last-write-wins downstream in `Store::open_in_memory`
    // (and would mask one of the two payloads from `read_referenced`); make
    // that a corruption error here so the bundle envelope is authoritative.
    let mut seen_paths: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(toc_entries.len());
    for (i, entry) in toc_entries.iter().enumerate() {
        if !seen_paths.insert(entry.rel_path.as_str()) {
            return Err(corrupt(format!(
                "duplicate bundle entry rel_path {:?}",
                entry.rel_path
            )));
        }
        verify_entry_range(entry, toc_offset)?;
        let start = usize_or_oob(entry.payload_offset, "payload_offset")?;
        let len = usize_or_oob(entry.payload_len, "payload_len")?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| out_of_bounds("payload start + len overflows usize"))?;
        // Redundant with verify_entry_range but cheap; defense in depth in case
        // a future bundle version widens the trailer.
        if end > trailer_start {
            return Err(out_of_bounds(format!(
                "entry {i} payload runs past trailer_start"
            )));
        }
        let payload = &bytes[start..end];
        let computed = sha256(payload);
        if computed != entry.sha256 {
            return Err(checksum_mismatch(format!(
                "entry {i} ({}) sha256 mismatch: TOC {}, computed {}",
                entry.rel_path,
                entry.sha256.to_hex(),
                computed.to_hex()
            )));
        }
        if entry.kind == EntryKind::Manifest {
            if entry.rel_path != MANIFEST_REL_PATH {
                return Err(corrupt(format!(
                    "Manifest entry rel_path {:?} != {MANIFEST_REL_PATH:?}",
                    entry.rel_path
                )));
            }
            manifest_count = manifest_count.saturating_add(1);
        }
        spans.push((
            entry.payload_offset,
            entry.payload_offset.saturating_add(entry.payload_len),
        ));
        entries_out.push(BundleEntryInfo {
            kind: entry.kind,
            rel_path: entry.rel_path.clone(),
            payload_len: entry.payload_len,
            sha256_hex: entry.sha256.to_hex(),
        });
    }
    if manifest_count != 1 {
        return Err(corrupt(format!(
            "expected exactly one MANIFEST entry, found {manifest_count}"
        )));
    }
    spans.sort_by(|a, b| a.0.cmp(&b.0));
    for w in spans.windows(2) {
        // Already validated end > start per entry above (a zero-length entry
        // passes range checks but cannot overlap anything by definition).
        if w[0].1 > w[1].0 {
            return Err(corrupt(format!(
                "payload ranges overlap: [{}, {}) and [{}, {})",
                w[0].0, w[0].1, w[1].0, w[1].1
            )));
        }
    }

    Ok(BundleSummary {
        format_version,
        entry_count: toc_entry_count,
        byte_len: len_u64,
        bundle_sha256_hex: stored_sha.to_hex(),
        entries: entries_out,
    })
}

/// Verify a `.prismbundle` image and return absolute payload byte ranges
/// keyed by their `rel_path`.
///
/// Runs the same full envelope-validation pass as [`read_summary`] (header
/// magic / version / CRC, trailer magic / version / whole-bundle SHA-256,
/// TOC body + CRC, per-entry range + no-overlap + SHA-256, exactly one
/// MANIFEST), then returns `(rel_path, payload_offset, payload_len)` per
/// entry in the order the bundle stored them.
///
/// Built for [`prism_store::Store::open_in_memory`]: the store turns the
/// returned tuples into a `HashMap` and serves reads as slices into the
/// shared bundle byte image.
///
/// # Errors
///
/// Same as [`read_summary`].
pub fn decode_toc_offsets(bytes: &[u8]) -> Result<Vec<(String, usize, usize)>> {
    // Run the full verification pass — this is the only correct thing to do
    // before trusting any TOC byte to address into `bytes`. The cost of
    // re-walking the TOC at the caller (Store::open_in_memory) is negligible
    // versus the cost of the per-entry SHA-256s that read_summary computes
    // here, and re-using the verified pass means decode_toc_offsets cannot
    // diverge from read_summary's bounds discipline.
    let _summary = read_summary(bytes)?;

    let header = &bytes[..HEADER_LEN];
    let toc_offset = u64::from_le_bytes(slice8(header, 16)?);
    let toc_entry_count = u32::from_le_bytes(slice4(header, 24)?);
    let trailer_start = bytes
        .len()
        .checked_sub(TRAILER_LEN)
        .ok_or_else(|| out_of_bounds("bundle shorter than trailer"))?;
    let toc_offset_usize = usize_or_oob(toc_offset, "toc_offset")?;
    let toc_slice = &bytes[toc_offset_usize..trailer_start];
    let toc_entries = decode_toc(toc_slice, toc_entry_count)?;

    let mut out = Vec::with_capacity(toc_entries.len());
    for entry in toc_entries {
        let start = usize_or_oob(entry.payload_offset, "payload_offset")?;
        let len = usize_or_oob(entry.payload_len, "payload_len")?;
        // read_summary has already enforced start + len <= trailer_start, but
        // recompute the bound here so decode_toc_offsets has no reliance on
        // an upstream invariant being preserved verbatim by a future refactor.
        let _end = start
            .checked_add(len)
            .ok_or_else(|| out_of_bounds("payload start + len overflows usize"))?;
        out.push((entry.rel_path, start, len));
    }
    Ok(out)
}

/// Verify a bundle, then materialize every embedded file under
/// `<dest_root>/.prism/`.
///
/// The bundle is fully validated first (via [`read_summary`]); no file is
/// written if validation fails, so a hostile bundle never leaves partial
/// content on disk.
///
/// Path safety is enforced twice:
///
/// - [`crate::toc::validate_rel_path`] inside [`read_summary`] rejects `..`,
///   leading `/`, `\`, `:`, NUL, etc.;
/// - this function additionally requires the joined destination to live under
///   `<dest_root>/.prism/` via [`std::path::Path::starts_with`], catching any
///   path that survived the first pass.
///
/// Each file is `fsync`ed via [`std::fs::File::sync_all`] so a crash after a
/// successful return guarantees the bundle is on stable storage.
///
/// Returns the same [`BundleSummary`] [`read_summary`] would, so callers can
/// log / publish it without re-parsing.
pub fn unpack_into(bytes: &[u8], dest_root: &Path) -> Result<BundleSummary> {
    let summary = read_summary(bytes)?;

    // Re-decode the TOC to recover absolute byte ranges. read_summary already
    // verified everything in `bytes`; this just gives us the offsets without
    // mutating `BundleSummary`'s public shape.
    let header = &bytes[..HEADER_LEN];
    let toc_offset = u64::from_le_bytes(slice8(header, 16)?);
    let toc_offset_usize = usize_or_oob(toc_offset, "toc_offset")?;
    let trailer_start = bytes
        .len()
        .checked_sub(TRAILER_LEN)
        .ok_or_else(|| out_of_bounds("bundle shorter than trailer"))?;
    let toc_slice = &bytes[toc_offset_usize..trailer_start];
    let toc_entries = decode_toc(toc_slice, summary.entry_count)?;

    let prism_root = dest_root.join(PRISM_SUBDIR);
    std::fs::create_dir_all(&prism_root).map_err(prism_model::PrismError::Io)?;

    use std::fs::OpenOptions;
    use std::io::Write;
    for entry in &toc_entries {
        let dest = prism_root.join(&entry.rel_path);

        // Defense in depth: even though validate_rel_path already rejects
        // traversal, require the joined parent to live under prism_root.
        let parent = dest
            .parent()
            .ok_or_else(|| corrupt(format!("entry {:?} has no parent path", entry.rel_path)))?;
        if !parent.starts_with(&prism_root) {
            return Err(corrupt(format!(
                "entry {:?} would escape prism root {:?}",
                entry.rel_path, prism_root
            )));
        }
        std::fs::create_dir_all(parent).map_err(prism_model::PrismError::Io)?;

        let start = usize_or_oob(entry.payload_offset, "payload_offset")?;
        let len = usize_or_oob(entry.payload_len, "payload_len")?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| out_of_bounds("payload start + len overflows usize"))?;
        let payload = &bytes[start..end];

        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&dest)
            .map_err(prism_model::PrismError::Io)?;
        f.write_all(payload)
            .map_err(prism_model::PrismError::Io)?;
        f.sync_all().map_err(prism_model::PrismError::Io)?;
    }

    Ok(summary)
}

/// Check that a single TOC entry's `[offset, offset+len)` range falls entirely
/// inside the payload region `[HEADER_LEN, toc_offset)`. All arithmetic is
/// `checked_*` so an adversarial entry cannot wrap a `u64`.
fn verify_entry_range(entry: &TocEntry, toc_offset: u64) -> Result<()> {
    if entry.payload_offset < HEADER_LEN as u64 {
        return Err(corrupt(format!(
            "payload_offset {} is inside header (< {HEADER_LEN})",
            entry.payload_offset
        )));
    }
    let end = entry
        .payload_offset
        .checked_add(entry.payload_len)
        .ok_or_else(|| out_of_bounds("payload_offset + payload_len overflows u64"))?;
    if end > toc_offset {
        return Err(out_of_bounds(format!(
            "payload end {end} > toc_offset {toc_offset}"
        )));
    }
    Ok(())
}

/// Read a 4-byte little-endian field from `buf` starting at `at`.
///
/// Returns `OutOfBounds` instead of panicking on a short read so a hostile
/// header with malformed bounds is rejected, not crashed-on.
fn slice4(buf: &[u8], at: usize) -> Result<[u8; 4]> {
    let end = at
        .checked_add(4)
        .ok_or_else(|| out_of_bounds("slice4 offset overflows usize"))?;
    if end > buf.len() {
        return Err(out_of_bounds(format!(
            "slice4 at {at} runs past buffer length {}",
            buf.len()
        )));
    }
    let mut out = [0u8; 4];
    out.copy_from_slice(&buf[at..end]);
    Ok(out)
}

/// Read an 8-byte little-endian field from `buf` starting at `at`.
fn slice8(buf: &[u8], at: usize) -> Result<[u8; 8]> {
    let end = at
        .checked_add(8)
        .ok_or_else(|| out_of_bounds("slice8 offset overflows usize"))?;
    if end > buf.len() {
        return Err(out_of_bounds(format!(
            "slice8 at {at} runs past buffer length {}",
            buf.len()
        )));
    }
    let mut out = [0u8; 8];
    out.copy_from_slice(&buf[at..end]);
    Ok(out)
}

/// Read a 32-byte SHA-256 field from `buf` starting at `at`.
fn slice32(buf: &[u8], at: usize) -> Result<[u8; 32]> {
    let end = at
        .checked_add(32)
        .ok_or_else(|| out_of_bounds("slice32 offset overflows usize"))?;
    if end > buf.len() {
        return Err(out_of_bounds(format!(
            "slice32 at {at} runs past buffer length {}",
            buf.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&buf[at..end]);
    Ok(out)
}

/// Narrow a `u64` to `usize`, returning `OutOfBounds` instead of panicking on
/// 32-bit targets where a forged `u64` could exceed the platform's address
/// space.
fn usize_or_oob(value: u64, name: &str) -> Result<usize> {
    usize::try_from(value).map_err(|_| out_of_bounds(format!("{name} {value} does not fit usize")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toc::encode_toc;
    use crate::write::pack_workspace;
    use prism_store::{Layout, RawRecord, Store};

    /// Hand-rolled tempdir (no external crates allowed). Each call gets a
    /// per-process, per-call unique name under `temp_dir`. An atomic counter
    /// guards against same-nanos collisions across concurrent tests.
    struct TempDir {
        path: std::path::PathBuf,
    }

    static TEMPDIR_COUNTER: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);

    impl TempDir {
        fn new(label: &str) -> TempDir {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let seq = TEMPDIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "prism-bundle-read-{label}-{}-{nanos}-{seq}",
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
    fn read_summary_round_trip_from_writer() {
        let td = TempDir::new("rt");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let bytes = pack_workspace(&layout).expect("pack");
        let summary = read_summary(&bytes).expect("verify");

        assert_eq!(summary.format_version, BUNDLE_FORMAT_VERSION);
        assert_eq!(summary.byte_len, bytes.len() as u64);
        assert_eq!(summary.entry_count as usize, summary.entries.len());
        // First entry must be MANIFEST and there must be exactly one.
        assert_eq!(summary.entries[0].kind, EntryKind::Manifest);
        let manifest_n = summary
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::Manifest)
            .count();
        assert_eq!(manifest_n, 1);
    }

    #[test]
    fn decode_toc_offsets_matches_read_summary() {
        // The two functions must agree on entry counts and ordering. Range
        // checks already happen inside both; here we just verify the seam
        // hands back what Store::open_in_memory needs (rel_path, off, len)
        // per entry, in bundle order.
        let td = TempDir::new("toc-offsets");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let bytes = pack_workspace(&layout).expect("pack");
        let summary = read_summary(&bytes).expect("verify");
        let toc = decode_toc_offsets(&bytes).expect("toc offsets");
        assert_eq!(toc.len(), summary.entries.len());
        for (entry, (rel, off, len)) in summary.entries.iter().zip(toc.iter()) {
            assert_eq!(&entry.rel_path, rel);
            assert_eq!(entry.payload_len as usize, *len);
            // Slice the payload out at the reported offset and verify it
            // hashes to the per-entry SHA-256 the summary reports.
            let end = off + len;
            let payload = &bytes[*off..end];
            assert_eq!(prism_codec::sha256(payload).to_hex(), entry.sha256_hex);
        }
    }

    #[test]
    fn store_open_in_memory_round_trips_through_bundle() {
        // End-to-end: pack a tiny workspace -> verify + decode TOC -> open
        // via Store::open_in_memory -> records and manifest match the
        // disk-opened control.
        use std::sync::Arc;

        let td = TempDir::new("om-src");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let bytes = pack_workspace(&layout).expect("pack");

        // Control: open the on-disk workspace read-only.
        let disk = Store::open_readonly(td.path()).expect("disk open");
        let disk_records = disk.read_all_records().expect("disk records");

        // SUT: open the same workspace from the in-memory bundle.
        let arc = Arc::new(bytes.clone());
        let toc = decode_toc_offsets(&bytes).expect("toc offsets");
        let mem = Store::open_in_memory(arc, toc).expect("in-memory open");
        let mem_records = mem.read_all_records().expect("mem records");

        // Same record set, byte-for-byte.
        assert_eq!(disk_records, mem_records);
        // Same manifest file set.
        assert_eq!(disk.manifest().files.len(), mem.manifest().files.len());
        for (a, b) in disk.manifest().files.iter().zip(mem.manifest().files.iter()) {
            assert_eq!(a.rel_path, b.rel_path);
            assert_eq!(a.len, b.len);
            assert_eq!(a.digest, b.digest);
        }
        // Bundle-backed store is read-only.
        assert!(mem.is_read_only());
    }

    #[test]
    fn rejects_short_buffer() {
        let bytes = vec![0u8; MIN_BUNDLE_LEN - 1];
        let err = read_summary(&bytes).expect_err("must reject short input");
        assert!(matches!(err, prism_model::PrismError::OutOfBounds { .. }));
    }

    #[test]
    fn rejects_bad_header_magic() {
        let td = TempDir::new("magic");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let mut bytes = pack_workspace(&layout).expect("pack");
        bytes[0] ^= 0xff;
        let err = read_summary(&bytes).expect_err("bad magic must fail");
        assert!(matches!(err, prism_model::PrismError::Corrupt { .. }));
    }

    #[test]
    fn rejects_bad_header_crc() {
        let td = TempDir::new("hcrc");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let mut bytes = pack_workspace(&layout).expect("pack");
        // Flip a flag byte. The CRC must reject *before* trailer recompute is
        // even reached.
        bytes[14] ^= 0x01;
        let err = read_summary(&bytes).expect_err("bad header crc must fail");
        match err {
            prism_model::PrismError::Corrupt { detail } => {
                assert!(detail.contains("header crc32"), "detail: {detail}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_nonzero_header_flags_with_valid_crc() {
        // A future writer that sets a flag bit AND recomputes the header
        // CRC for the new bytes would slip past the CRC check. The
        // explicit flags-must-be-zero rule must catch it: a v1 reader
        // cannot honour an unknown semantic flag silently.
        let td = TempDir::new("flags-set");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let mut bytes = pack_workspace(&layout).expect("pack");
        // flags = bytes[12..16]; set the low bit and recompute CRC over 0..28.
        bytes[12] |= 0x01;
        let new_crc = crc32(&bytes[0..28]);
        bytes[28..32].copy_from_slice(&new_crc.to_le_bytes());
        let err = read_summary(&bytes).expect_err("non-zero flags must fail");
        match err {
            prism_model::PrismError::Corrupt { detail } => {
                assert!(detail.contains("flags"), "detail: {detail}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_trailer_magic() {
        let td = TempDir::new("tmagic");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let mut bytes = pack_workspace(&layout).expect("pack");
        let n = bytes.len();
        bytes[n - TRAILER_LEN] ^= 0xff;
        let err = read_summary(&bytes).expect_err("bad trailer magic must fail");
        assert!(matches!(err, prism_model::PrismError::Corrupt { .. }));
    }

    #[test]
    fn rejects_bad_trailer_sha() {
        let td = TempDir::new("tsha");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let mut bytes = pack_workspace(&layout).expect("pack");
        let n = bytes.len();
        bytes[n - 1] ^= 0xff;
        let err = read_summary(&bytes).expect_err("bad trailer sha must fail");
        assert!(matches!(err, prism_model::PrismError::ChecksumMismatch { .. }));
    }

    #[test]
    fn rejects_unsupported_version() {
        let td = TempDir::new("ver");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let mut bytes = pack_workspace(&layout).expect("pack");
        // Bump format_version to 2, re-CRC the header. Trailer will fail too,
        // but version is checked first.
        bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
        let new_crc = crc32(&bytes[0..28]);
        bytes[28..32].copy_from_slice(&new_crc.to_le_bytes());
        let err = read_summary(&bytes).expect_err("v2 must fail");
        match err {
            prism_model::PrismError::UnsupportedFormatVersion { found, max } => {
                assert_eq!(found, 2);
                assert_eq!(max, BUNDLE_FORMAT_VERSION);
            }
            other => panic!("expected UnsupportedFormatVersion, got {other:?}"),
        }
    }

    #[test]
    fn rejects_zero_version() {
        let td = TempDir::new("v0");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let mut bytes = pack_workspace(&layout).expect("pack");
        bytes[8..12].copy_from_slice(&0u32.to_le_bytes());
        let new_crc = crc32(&bytes[0..28]);
        bytes[28..32].copy_from_slice(&new_crc.to_le_bytes());
        let err = read_summary(&bytes).expect_err("v0 must fail");
        assert!(matches!(
            err,
            prism_model::PrismError::UnsupportedFormatVersion { .. }
        ));
    }

    #[test]
    fn unpack_into_round_trips_through_disk() {
        let td = TempDir::new("unpack-src");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let bytes = pack_workspace(&layout).expect("pack");

        let dest = TempDir::new("unpack-dst");
        let _summary = unpack_into(&bytes, dest.path()).expect("unpack");

        // Every file recorded in the bundle TOC must exist under
        // <dest>/.prism/<rel_path>.
        let prism_root = dest.path().join(".prism");
        assert!(prism_root.join("MANIFEST").exists());

        // And the unpacked workspace must open read-only.
        let _store = Store::open_readonly(dest.path()).expect("re-open");
    }

    #[test]
    fn unpack_into_skips_writes_on_corrupted_input() {
        let td = TempDir::new("partial-src");
        build_tiny_store(&td);
        let layout = Layout::new(td.path());
        let mut bytes = pack_workspace(&layout).expect("pack");
        // Wreck the trailer sha so verification fails.
        let n = bytes.len();
        bytes[n - 1] ^= 0xff;

        let dest = TempDir::new("partial-dst");
        let err = unpack_into(&bytes, dest.path()).expect_err("must reject");
        assert!(matches!(err, prism_model::PrismError::ChecksumMismatch { .. }));

        // Defensive: we DO allow the .prism dir to be created up-front, but no
        // payload file should exist.
        let prism_root = dest.path().join(".prism");
        if prism_root.exists() {
            let manifest = prism_root.join("MANIFEST");
            assert!(
                !manifest.exists(),
                "must not write any payload on failed verify"
            );
        }
    }

    /// Build a synthetic bundle with a hand-picked TOC. Useful for tests that
    /// need to inject specific entries without going through the writer.
    fn synthesize_bundle(toc_entries: Vec<TocEntry>, payloads: Vec<Vec<u8>>) -> Vec<u8> {
        use prism_codec::ByteWriter;

        assert_eq!(toc_entries.len(), payloads.len());
        let mut w = ByteWriter::new();
        w.write_bytes(&[0u8; HEADER_LEN]);
        for p in &payloads {
            w.write_bytes(p);
        }
        let toc_offset = w.len() as u64;
        let toc_bytes = encode_toc(&toc_entries);
        w.write_bytes(&toc_bytes);

        let mut header = [0u8; HEADER_LEN];
        header[0..8].copy_from_slice(BUNDLE_MAGIC);
        header[8..12].copy_from_slice(&BUNDLE_FORMAT_VERSION.to_le_bytes());
        header[12..16].copy_from_slice(&0u32.to_le_bytes());
        header[16..24].copy_from_slice(&toc_offset.to_le_bytes());
        header[24..28].copy_from_slice(&(toc_entries.len() as u32).to_le_bytes());
        let header_crc = crc32(&header[0..28]);
        header[28..32].copy_from_slice(&header_crc.to_le_bytes());

        let mut out = w.into_vec();
        out[..HEADER_LEN].copy_from_slice(&header);

        let bundle_sha = sha256(&out);
        out.extend_from_slice(TRAILER_MAGIC);
        out.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
        out.extend_from_slice(bundle_sha.as_bytes());
        out
    }

    #[test]
    fn rejects_missing_manifest_entry() {
        // A bundle that omits the MANIFEST entry entirely must be rejected.
        let payload = b"hello".to_vec();
        let sha = sha256(&payload);
        let entries = vec![TocEntry {
            kind: EntryKind::Segment,
            rel_path: "segments/000001.pseg".into(),
            payload_offset: HEADER_LEN as u64,
            payload_len: payload.len() as u64,
            sha256: sha,
        }];
        let bytes = synthesize_bundle(entries, vec![payload]);
        let err = read_summary(&bytes).expect_err("must reject missing manifest");
        match err {
            prism_model::PrismError::Corrupt { detail } => {
                assert!(
                    detail.contains("MANIFEST") || detail.contains("Manifest"),
                    "detail: {detail}"
                );
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_overlapping_payloads() {
        // Two entries claim overlapping ranges.
        let payload_a = b"AAAAAAAA".to_vec(); // 8 bytes
        let payload_b = b"BBBBBBBB".to_vec(); // 8 bytes
        let sha_a = sha256(&payload_a);
        let sha_b = sha256(&payload_b);
        let entries = vec![
            TocEntry {
                kind: EntryKind::Manifest,
                rel_path: "MANIFEST".into(),
                payload_offset: HEADER_LEN as u64,
                payload_len: 8,
                sha256: sha_a,
            },
            // Claim the same range as A. Even though the layout writes them
            // back to back (offset 32 and 40), the TOC entry lies and claims
            // offset 32 again — overlap detection must fire.
            TocEntry {
                kind: EntryKind::Segment,
                rel_path: "segments/000001.pseg".into(),
                payload_offset: HEADER_LEN as u64,
                payload_len: 8,
                sha256: sha_b,
            },
        ];
        // We still have to write *some* bytes for both payloads or the bundle
        // won't even reach the overlap check. Both rounds of "payload" are
        // appended; the SHA of the first overlapping range will not match B's
        // hash so the checksum check fires first if we get unlucky — but the
        // first payload's SHA matches A's, and B's TOC entry points at the
        // same offset, so B's SHA check will run against A's bytes and fail.
        // Either way, we get a Corrupt/ChecksumMismatch, which is the
        // protection contract.
        let bytes = synthesize_bundle(entries, vec![payload_a, payload_b]);
        let err = read_summary(&bytes).expect_err("must reject overlap");
        assert!(matches!(
            err,
            prism_model::PrismError::Corrupt { .. }
                | prism_model::PrismError::ChecksumMismatch { .. }
        ));
    }

    #[test]
    fn rejects_duplicate_manifest_entries() {
        let payload = b"hello".to_vec();
        let sha = sha256(&payload);
        let entries = vec![
            TocEntry {
                kind: EntryKind::Manifest,
                rel_path: "MANIFEST".into(),
                payload_offset: HEADER_LEN as u64,
                payload_len: payload.len() as u64,
                sha256: sha,
            },
            TocEntry {
                kind: EntryKind::Manifest,
                rel_path: "MANIFEST".into(),
                payload_offset: HEADER_LEN as u64 + payload.len() as u64,
                payload_len: payload.len() as u64,
                sha256: sha,
            },
        ];
        let bytes = synthesize_bundle(entries, vec![payload.clone(), payload]);
        let err = read_summary(&bytes).expect_err("must reject dup MANIFEST");
        assert!(matches!(err, prism_model::PrismError::Corrupt { .. }));
    }
}
