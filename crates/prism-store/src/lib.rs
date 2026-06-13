//! Prism's purpose-built local store.
//!
//! This crate owns the on-disk workspace: append-only object segments, the
//! atomically-published manifest, and (in later milestones) typed indexes, the
//! blob store, and compaction. The unit of durability is a batch: records and the
//! files that reference them are staged in `tmp/`, fsync'd, renamed into place,
//! and then published by a single atomic manifest swap.
//!
//! See the architecture design document, *Storage Architecture* and *Crash Safety*.

pub mod batch;
pub mod blob;
pub mod fsutil;
pub mod index;
pub mod lock;
pub mod manifest;
pub mod paths;
pub mod records;
pub mod segment;

#[cfg(test)]
mod test_support;

pub use batch::WriteBatch;
pub use blob::BlobStore;
pub use index::{IndexReader, IndexSet, Memtable};
pub use lock::WorkspaceLock;
pub use manifest::{FileEntry, FileKind, Manifest};
pub use paths::Layout;
pub use records::record_type;
pub use segment::{SegmentReader, SegmentWriter};

use index::keys;
use prism_codec::sha256;
use prism_model::{
    FiltrationId, Package, PackageId, PackageVersion, PrismError, Requirement, RequirementId,
    ResolverRun, Result, RunId, StringId, VersionId,
};
use prism_model::EcosystemId;
use std::path::Path;

/// A record to be written into a segment during a publish. `logical_ts` is
/// assigned by the store from the recovered high-water mark.
#[derive(Clone, Debug)]
pub struct RawRecord {
    pub record_type: u16,
    pub flags: u16,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

impl RawRecord {
    /// Convenience constructor.
    pub fn new(record_type: u16, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Self {
        Self {
            record_type,
            flags: 0,
            key: key.into(),
            value: value.into(),
        }
    }
}

/// An open Prism store. Holds the workspace write lock for its lifetime.
pub struct Store {
    layout: Layout,
    manifest: Manifest,
    /// Merged read view over all published [`FileKind::Index`] segments.
    index: IndexSet,
    _lock: WorkspaceLock,
}

impl Store {
    /// Initialize a new workspace at `root`, creating `.prism/` and publishing an
    /// empty manifest. Fails if a workspace already exists there.
    pub fn init(root: impl AsRef<Path>) -> Result<Store> {
        let layout = Layout::new(root.as_ref());
        if layout.manifest().exists() {
            return Err(PrismError::Other(format!(
                "workspace already initialized at {}",
                layout.prism_dir().display()
            )));
        }
        fsutil::ensure_dir(&layout.prism_dir())?;
        for dir in layout.all_dirs() {
            fsutil::ensure_dir(&dir)?;
        }
        let lock = WorkspaceLock::acquire(&layout.lock())?;
        let manifest = Manifest::empty();
        fsutil::atomic_write(&layout.manifest(), &manifest.encode())?;
        Ok(Store {
            layout,
            manifest,
            index: IndexSet::new(),
            _lock: lock,
        })
    }

    /// Open an existing workspace at `root`: acquire the lock, load and verify the
    /// manifest's file set, and clean any stale `tmp/` staging files.
    pub fn open(root: impl AsRef<Path>) -> Result<Store> {
        let layout = Layout::new(root.as_ref());
        if !layout.manifest().exists() {
            return Err(PrismError::Other(format!(
                "no workspace at {}",
                layout.prism_dir().display()
            )));
        }
        let lock = WorkspaceLock::acquire(&layout.lock())?;
        let manifest = Manifest::decode(&fsutil::read_file(&layout.manifest())?)?;

        // Verify every referenced file: present, correct length, matching digest.
        for entry in &manifest.files {
            let path = layout.prism_dir().join(&entry.rel_path);
            let bytes = fsutil::read_file(&path).map_err(|_| PrismError::Corrupt {
                detail: format!("manifest references missing file {}", entry.rel_path),
            })?;
            if bytes.len() as u64 != entry.len {
                return Err(PrismError::Corrupt {
                    detail: format!(
                        "file {} length {} != manifest {}",
                        entry.rel_path,
                        bytes.len(),
                        entry.len
                    ),
                });
            }
            if sha256(&bytes) != entry.digest {
                return Err(PrismError::ChecksumMismatch {
                    detail: entry.rel_path.clone(),
                });
            }
        }

        // tmp/ cleanup is performed only by the writer under the lock.
        Self::clean_tmp(&layout)?;

        // Load all FileKind::Index segments into the merged read view. Files are
        // listed in publish order (oldest first); push_newest inserts at the front,
        // so iterating in order leaves the newest segment first.
        let mut index = IndexSet::new();
        for entry in &manifest.files {
            if entry.kind != FileKind::Index {
                continue;
            }
            let path = layout.prism_dir().join(&entry.rel_path);
            let reader = IndexReader::parse(&fsutil::read_file(&path)?)?;
            index.push_newest(reader);
        }

        Ok(Store {
            layout,
            manifest,
            index,
            _lock: lock,
        })
    }

    fn clean_tmp(layout: &Layout) -> Result<()> {
        let tmp = layout.tmp_dir();
        if tmp.exists() {
            for entry in std::fs::read_dir(&tmp)? {
                let entry = entry?;
                let _ = std::fs::remove_file(entry.path());
            }
            fsutil::fsync_dir(&tmp)?;
        }
        Ok(())
    }

    /// The workspace layout.
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The current (in-memory) manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Publish a new object segment containing `records`, assigning each a fresh
    /// monotonic `logical_ts`. Returns the new segment's id.
    ///
    /// Follows the write protocol: stage the segment in `tmp/`, fsync, rename into
    /// `segments/`, fsync the dir, then atomically swap the manifest.
    pub fn publish_segment(&mut self, records: &[RawRecord]) -> Result<u32> {
        let segment_id = self.manifest.next_segment_id;
        let created_at = self.manifest.logical_ts_hwm as i64; // monotone stand-in clock

        let mut writer = SegmentWriter::new(segment_id, created_at);
        let mut ts = self.manifest.logical_ts_hwm;
        for rec in records {
            ts += 1;
            writer.append(rec.record_type, rec.flags, ts, &rec.key, &rec.value);
        }
        let image = writer.finish();
        let digest = sha256(&image);

        // Stage in tmp/, fsync, then rename into segments/ and fsync that dir.
        let staged = self.layout.tmp_dir().join(Layout::segment_file_name(segment_id));
        fsutil::atomic_write(&staged, &image)?; // durable write into tmp/
        let final_path = self.layout.segment_file(segment_id);
        std::fs::rename(&staged, &final_path)?;
        fsutil::fsync_dir(&self.layout.segments_dir())?;

        // Build the next manifest and publish it atomically.
        let mut next = self.manifest.clone();
        next.next_segment_id += 1;
        next.logical_ts_hwm = ts;
        next.files.push(FileEntry {
            kind: FileKind::Segment,
            rel_path: format!("segments/{}", Layout::segment_file_name(segment_id)),
            len: image.len() as u64,
            digest,
        });
        fsutil::atomic_write(&self.layout.manifest(), &next.encode())?;
        self.manifest = next;
        Ok(segment_id)
    }

    /// Read every record across all published segments, in segment then append
    /// order. (A convenience for tests and `prism status`; real reads go through
    /// indexes once those land.)
    pub fn read_all_records(&self) -> Result<Vec<StoredRecord>> {
        let mut out = Vec::new();
        for entry in &self.manifest.files {
            if entry.kind != FileKind::Segment {
                continue;
            }
            let path = self.layout.prism_dir().join(&entry.rel_path);
            let seg = SegmentReader::parse(fsutil::read_file(&path)?)?;
            for rec in seg.records() {
                let rec = rec?;
                out.push(StoredRecord {
                    record_type: rec.header.record_type,
                    key: rec.key.to_vec(),
                    value: rec.value.to_vec(),
                    logical_ts: rec.header.logical_ts,
                });
            }
        }
        Ok(out)
    }

    // --- ID allocation (from manifest high-water counters) ---------------------

    /// Allocate the next store-global object id (used for packages, versions,
    /// requirements, runs — all share one monotonic counter). Persisted on the next
    /// publish via [`Manifest::next_object_id`].
    fn alloc_object_id(&mut self) -> u64 {
        let id = self.manifest.next_object_id;
        self.manifest.next_object_id += 1;
        id
    }

    /// Allocate a fresh [`PackageId`].
    pub fn alloc_package_id(&mut self) -> PackageId {
        PackageId::new(self.alloc_object_id())
    }

    /// Allocate a fresh [`VersionId`].
    pub fn alloc_version_id(&mut self) -> VersionId {
        VersionId::new(self.alloc_object_id())
    }

    /// Allocate a fresh [`RequirementId`].
    pub fn alloc_requirement_id(&mut self) -> RequirementId {
        RequirementId::new(self.alloc_object_id())
    }

    /// Allocate a fresh [`RunId`].
    pub fn alloc_run_id(&mut self) -> RunId {
        RunId::new(self.alloc_object_id())
    }

    /// Allocate a fresh [`StringId`] from the dedicated interner counter.
    fn alloc_string_id(&mut self) -> StringId {
        let id = StringId::new(self.manifest.next_string_id);
        self.manifest.next_string_id += 1;
        id
    }

    // --- Atomic batch commit ---------------------------------------------------

    /// Start a new empty write batch.
    pub fn batch(&self) -> WriteBatch {
        WriteBatch::new()
    }

    /// Commit a [`WriteBatch`]: write a `.pseg` for its records and a `.pidx` for
    /// its index entries, publishing both in one manifest swap (architecture doc,
    /// *Crash Safety*). Empty batches are a no-op. Returns the segment id (or `None`
    /// for an empty batch).
    ///
    /// Both the records and the index entries draw their `logical_ts` from one
    /// monotonic sweep over the recovered high-water mark, so "newest wins" is
    /// consistent across the segment and its index.
    pub fn commit_batch(&mut self, batch: WriteBatch) -> Result<Option<u32>> {
        if batch.is_empty() {
            return Ok(None);
        }

        let mut next = self.manifest.clone();
        let mut ts = next.logical_ts_hwm;
        let created_at = next.logical_ts_hwm as i64; // monotone stand-in clock
        let mut staged: Vec<(std::path::PathBuf, std::path::PathBuf, FileEntry)> = Vec::new();

        // Build the .pseg (may be empty if the batch only writes index entries).
        let seg_id = next.next_segment_id;
        next.next_segment_id += 1;
        let mut writer = SegmentWriter::new(seg_id, created_at);
        for rec in &batch.records {
            ts += 1;
            writer.append(rec.record_type, rec.flags, ts, &rec.key, &rec.value);
        }
        let seg_image = writer.finish();
        let seg_rel = format!("segments/{}", Layout::segment_file_name(seg_id));
        staged.push((
            self.layout.tmp_dir().join(Layout::segment_file_name(seg_id)),
            self.layout.segment_file(seg_id),
            FileEntry {
                kind: FileKind::Segment,
                rel_path: seg_rel,
                len: seg_image.len() as u64,
                digest: sha256(&seg_image),
            },
        ));

        // Build the .pidx over a memtable; the entries share the same ts sweep.
        let idx_id = next.next_segment_id;
        next.next_segment_id += 1;
        let mut mt = Memtable::new();
        for op in &batch.index_ops {
            ts += 1;
            if op.tombstone {
                mt.delete(op.key.clone(), ts);
            } else {
                mt.put(op.key.clone(), op.value.clone(), ts);
            }
        }
        let idx_image = mt.freeze(idx_id, created_at);
        let idx_rel = format!("indexes/{}", Layout::index_file_name(idx_id));
        staged.push((
            self.layout.tmp_dir().join(Layout::index_file_name(idx_id)),
            self.layout.index_file(idx_id),
            FileEntry {
                kind: FileKind::Index,
                rel_path: idx_rel,
                len: idx_image.len() as u64,
                digest: sha256(&idx_image),
            },
        ));

        next.logical_ts_hwm = ts;
        let images = [seg_image, idx_image];
        self.publish(&mut next, &staged, &images)?;

        // Make the new index visible to readers immediately.
        let reader = IndexReader::parse(&images[1])?;
        self.index.push_newest(reader);
        self.manifest = next;
        Ok(Some(seg_id))
    }

    /// Stage `images` (paired with their destination paths and manifest entries),
    /// fsync them, rename into place, fsync each destination dir, append the entries
    /// to `next`, and atomically swap the manifest. Shared by every publish path.
    fn publish(
        &self,
        next: &mut Manifest,
        staged: &[(std::path::PathBuf, std::path::PathBuf, FileEntry)],
        images: &[Vec<u8>],
    ) -> Result<()> {
        debug_assert_eq!(staged.len(), images.len());
        // 1-3: durably write each file into tmp/.
        for ((tmp, _, _), image) in staged.iter().zip(images) {
            fsutil::atomic_write(tmp, image)?;
        }
        // 4: rename into the destination dirs and fsync those dirs.
        let mut dirs: Vec<std::path::PathBuf> = Vec::new();
        for (tmp, final_path, _) in staged {
            std::fs::rename(tmp, final_path)?;
            if let Some(parent) = final_path.parent() {
                if !dirs.iter().any(|d| d == parent) {
                    dirs.push(parent.to_path_buf());
                }
            }
        }
        for dir in &dirs {
            fsutil::fsync_dir(dir)?;
        }
        // 5-7: append entries and atomically swap the manifest.
        for (_, _, entry) in staged {
            next.files.push(entry.clone());
        }
        fsutil::atomic_write(&self.layout.manifest(), &next.encode())?;
        Ok(())
    }

    // --- String interning ------------------------------------------------------

    /// Intern `text`, returning a stable [`StringId`]. If `text` was interned before
    /// (in this store or a prior session, recovered from the index), the existing id
    /// is returned; otherwise a new atom record and its `STRING_BY_ID`/
    /// `STRING_BY_TEXT` index entries are committed.
    pub fn intern_string(&mut self, text: &str) -> Result<StringId> {
        let mut batch = self.batch();
        let id = self.intern_string_in(&mut batch, text)?;
        self.commit_batch(batch)?;
        Ok(id)
    }

    /// Batch-aware interning: like [`Store::intern_string`] but stages the new atom
    /// into `batch` instead of committing immediately, so a whole ingest interns
    /// many strings and publishes them in **one** segment + index. Deduplicates
    /// against both already-committed atoms and atoms staged earlier in `batch`.
    pub fn intern_string_in(&mut self, batch: &mut WriteBatch, text: &str) -> Result<StringId> {
        let text_hash = *sha256(text.as_bytes()).as_bytes();
        let rev_key = keys::string_by_text(&text_hash);

        // Already committed in a prior epoch/batch?
        if let Some(bytes) = self.index.get(&rev_key) {
            let arr: [u8; 4] = bytes.try_into().map_err(|_| PrismError::Corrupt {
                detail: "string_by_text value not 4 bytes".into(),
            })?;
            return Ok(StringId::new(u32::from_be_bytes(arr)));
        }
        // Already staged earlier in this same uncommitted batch?
        if let Some(&id) = batch.pending_strings.get(&text_hash) {
            return Ok(id);
        }

        let id = self.alloc_string_id();
        let value = records::encode_string_atom(text);
        // The atom record is keyed by its StringId so STRING_BY_ID can point at it
        // and resolve_string can read the value back.
        let id_key = keys::string_by_id(id);
        batch.put_record(RawRecord::new(
            record_type::STRING_ATOM,
            id_key.clone(),
            value.clone(),
        ));
        batch.put_index(id_key, value);
        batch.put_index(rev_key, id.get().to_be_bytes().to_vec());
        batch.pending_strings.insert(text_hash, id);
        Ok(id)
    }

    /// Resolve a [`StringId`] back to its text, or `None` if unknown.
    pub fn resolve_string(&self, id: StringId) -> Option<String> {
        let bytes = self.index.get(&keys::string_by_id(id))?;
        records::decode_string_atom(bytes).ok()
    }

    // --- Index reads -----------------------------------------------------------

    /// Resolve a typed index `key` to its live value (memtable → newest → older),
    /// or `None` if absent or tombstoned.
    pub fn get_index(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.index.get(key).map(|v| v.to_vec())
    }

    /// The merged read view over published index segments.
    pub fn index(&self) -> &IndexSet {
        &self.index
    }

    // --- Entity writes (staged into a batch) -----------------------------------

    /// Stage a [`Package`] record plus its `PKG_BY_ID` and `PKG_BY_NAME` index
    /// entries into `batch`.
    pub fn put_package(&self, batch: &mut WriteBatch, pkg: &Package) {
        let id_key = keys::pkg_by_id(pkg.id);
        batch.put_record(RawRecord::new(
            record_type::PACKAGE,
            id_key.clone(),
            records::encode_package(pkg),
        ));
        batch.put_index(id_key, records::encode_package(pkg));
        batch.put_index(
            keys::pkg_by_name(pkg.ecosystem, pkg.namespace, pkg.canonical_name),
            pkg.id.get().to_be_bytes().to_vec(),
        );
    }

    /// Stage a [`PackageVersion`] record plus its `VER_BY_ID`, `VER_BY_PURL`, and a
    /// `VERS_BY_PACKAGE` list fragment into `batch`.
    pub fn put_version(&self, batch: &mut WriteBatch, ver: &PackageVersion) {
        let id_key = keys::ver_by_id(ver.id);
        batch.put_record(RawRecord::new(
            record_type::PACKAGE_VERSION,
            id_key.clone(),
            records::encode_package_version(ver),
        ));
        batch.put_index(id_key, records::encode_package_version(ver));
        batch.put_index(keys::ver_by_purl(ver.purl), ver.id.get().to_be_bytes().to_vec());
        batch.put_index(
            keys::vers_by_package_entry(ver.package_id, ver.id),
            ver.id.get().to_be_bytes().to_vec(),
        );
    }

    /// Stage a [`Requirement`] record plus its `REQ_BY_ID`, a `REQS_BY_SRC_VER`
    /// fragment, and (when the target is resolved) a `REQS_BY_DST_PACKAGE` fragment.
    pub fn put_requirement(&self, batch: &mut WriteBatch, req: &Requirement) {
        let id_key = keys::req_by_id(req.id);
        batch.put_record(RawRecord::new(
            record_type::REQUIREMENT,
            id_key.clone(),
            records::encode_requirement(req),
        ));
        batch.put_index(id_key, records::encode_requirement(req));
        batch.put_index(
            keys::reqs_by_src_ver(req.src_version_id, req.id),
            req.id.get().to_be_bytes().to_vec(),
        );
        if let Some(dst) = req.dst_package_id {
            batch.put_index(
                keys::reqs_by_dst_package(dst, req.id),
                req.id.get().to_be_bytes().to_vec(),
            );
        }
    }

    /// Stage a [`ResolverRun`] record plus its `RUN_BY_ID` index entry.
    pub fn put_run(&self, batch: &mut WriteBatch, run: &ResolverRun) {
        let id_key = keys::run_by_id(run.id);
        batch.put_record(RawRecord::new(
            record_type::RESOLVER_RUN,
            id_key.clone(),
            records::encode_resolver_run(run),
        ));
        batch.put_index(id_key, records::encode_resolver_run(run));
    }

    // --- Typed lookups ---------------------------------------------------------

    /// Find a package family by its canonical `(ecosystem, namespace, name)` key.
    pub fn find_package(
        &self,
        eco: EcosystemId,
        namespace: Option<StringId>,
        canonical_name: StringId,
    ) -> Option<PackageId> {
        let bytes = self.index.get(&keys::pkg_by_name(eco, namespace, canonical_name))?;
        let arr: [u8; 8] = bytes.try_into().ok()?;
        Some(PackageId::new(u64::from_be_bytes(arr)))
    }

    /// Read a [`Package`] by id.
    pub fn get_package(&self, id: PackageId) -> Result<Package> {
        let bytes = self.index.get(&keys::pkg_by_id(id)).ok_or(PrismError::NotFound)?;
        records::decode_package(bytes)
    }

    /// List every version id of a package family (list fragments merged across
    /// segments), sorted by [`VersionId`].
    pub fn versions_by_package(&self, id: PackageId) -> Vec<VersionId> {
        let prefix = keys::vers_by_package(id);
        let mut out: Vec<VersionId> = self
            .index
            .scan_prefix(&prefix)
            .into_iter()
            .filter_map(|(_, v)| {
                let arr: [u8; 8] = v.try_into().ok()?;
                Some(VersionId::new(u64::from_be_bytes(arr)))
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Read a [`PackageVersion`] by id.
    pub fn get_version(&self, id: VersionId) -> Result<PackageVersion> {
        let bytes = self.index.get(&keys::ver_by_id(id)).ok_or(PrismError::NotFound)?;
        records::decode_package_version(bytes)
    }

    /// Find a version id by its interned purl.
    pub fn find_version_by_purl(&self, purl: StringId) -> Option<VersionId> {
        let bytes = self.index.get(&keys::ver_by_purl(purl))?;
        let arr: [u8; 8] = bytes.try_into().ok()?;
        Some(VersionId::new(u64::from_be_bytes(arr)))
    }

    /// List every requirement id declared by a source version (merged fragments),
    /// sorted by [`RequirementId`].
    pub fn requirements_by_source_version(&self, src: VersionId) -> Vec<RequirementId> {
        let prefix = keys::reqs_by_src_ver_prefix(src);
        let mut out: Vec<RequirementId> = self
            .index
            .scan_prefix(&prefix)
            .into_iter()
            .filter_map(|(_, v)| {
                let arr: [u8; 8] = v.try_into().ok()?;
                Some(RequirementId::new(u64::from_be_bytes(arr)))
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// List every requirement id whose resolved target is `dst` (merged fragments).
    pub fn requirements_by_dst_package(&self, dst: PackageId) -> Vec<RequirementId> {
        let prefix = keys::reqs_by_dst_package_prefix(dst);
        let mut out: Vec<RequirementId> = self
            .index
            .scan_prefix(&prefix)
            .into_iter()
            .filter_map(|(_, v)| {
                let arr: [u8; 8] = v.try_into().ok()?;
                Some(RequirementId::new(u64::from_be_bytes(arr)))
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Read a [`Requirement`] by id.
    pub fn get_requirement(&self, id: RequirementId) -> Result<Requirement> {
        let bytes = self.index.get(&keys::req_by_id(id)).ok_or(PrismError::NotFound)?;
        records::decode_requirement(bytes)
    }

    /// Read a [`ResolverRun`] by id.
    pub fn get_run(&self, id: RunId) -> Result<ResolverRun> {
        let bytes = self.index.get(&keys::run_by_id(id)).ok_or(PrismError::NotFound)?;
        records::decode_resolver_run(bytes)
    }

    // --- Derived-file persistence (graphs, tda) --------------------------------

    /// Publish a resolved graph snapshot (an opaque `.pcsr` image from `prism-graph`)
    /// for `run`: stage the file in `graphs/`, then in one manifest swap add a
    /// [`FileKind::Graph`] entry and a `GRAPH_BY_RUN` index entry pointing at it.
    pub fn publish_graph(&mut self, run: RunId, pcsr: &[u8]) -> Result<()> {
        let rel = format!("graphs/{}", Layout::graph_file_name(run.get()));
        let mut next = self.manifest.clone();
        let mut ts = next.logical_ts_hwm;
        let created_at = ts as i64;
        let staged = vec![(
            self.layout.tmp_dir().join(Layout::graph_file_name(run.get())),
            self.layout.graph_file(run.get()),
            FileEntry {
                kind: FileKind::Graph,
                rel_path: rel.clone(),
                len: pcsr.len() as u64,
                digest: sha256(pcsr),
            },
        )];

        // GRAPH_BY_RUN index entry lives in its own .pidx published in the same swap.
        let idx_id = next.next_segment_id;
        next.next_segment_id += 1;
        let mut mt = Memtable::new();
        ts += 1;
        mt.put(keys::graph_by_run(run), rel.into_bytes(), ts);
        let idx_image = mt.freeze(idx_id, created_at);
        let idx_staged = (
            self.layout.tmp_dir().join(Layout::index_file_name(idx_id)),
            self.layout.index_file(idx_id),
            FileEntry {
                kind: FileKind::Index,
                rel_path: format!("indexes/{}", Layout::index_file_name(idx_id)),
                len: idx_image.len() as u64,
                digest: sha256(&idx_image),
            },
        );

        next.logical_ts_hwm = ts;
        let mut all_staged = staged;
        all_staged.push(idx_staged);
        let images = vec![pcsr.to_vec(), idx_image];
        self.publish(&mut next, &all_staged, &images)?;
        self.index.push_newest(IndexReader::parse(&images[1])?);
        self.manifest = next;
        Ok(())
    }

    /// Read back the graph snapshot image published for `run`.
    pub fn open_graph(&self, run: RunId) -> Result<Vec<u8>> {
        let rel = self
            .get_index(&keys::graph_by_run(run))
            .ok_or(PrismError::NotFound)?;
        let rel = String::from_utf8(rel).map_err(|_| PrismError::Corrupt {
            detail: "graph path not utf8".into(),
        })?;
        self.read_referenced(&rel)
    }

    /// Publish a TDA result (an opaque `.ptda` image from `prism-tda`) for
    /// `filtration` (which belongs to `run`): stage the file in `tda/`, then in one
    /// manifest swap add a [`FileKind::Tda`] entry and a `TDA_BY_RUN` list fragment.
    pub fn publish_tda(&mut self, filtration: FiltrationId, run: RunId, ptda: &[u8]) -> Result<()> {
        let rel = format!("tda/{}", Layout::tda_file_name(filtration.get()));
        let mut next = self.manifest.clone();
        let mut ts = next.logical_ts_hwm;
        let created_at = ts as i64;
        let staged = (
            self.layout.tmp_dir().join(Layout::tda_file_name(filtration.get())),
            self.layout.tda_file(filtration.get()),
            FileEntry {
                kind: FileKind::Tda,
                rel_path: rel.clone(),
                len: ptda.len() as u64,
                digest: sha256(ptda),
            },
        );

        let idx_id = next.next_segment_id;
        next.next_segment_id += 1;
        let mut mt = Memtable::new();
        ts += 1;
        mt.put(keys::tda_by_run(run, filtration), rel.into_bytes(), ts);
        let idx_image = mt.freeze(idx_id, created_at);
        let idx_staged = (
            self.layout.tmp_dir().join(Layout::index_file_name(idx_id)),
            self.layout.index_file(idx_id),
            FileEntry {
                kind: FileKind::Index,
                rel_path: format!("indexes/{}", Layout::index_file_name(idx_id)),
                len: idx_image.len() as u64,
                digest: sha256(&idx_image),
            },
        );

        next.logical_ts_hwm = ts;
        let all_staged = vec![staged, idx_staged];
        let images = vec![ptda.to_vec(), idx_image];
        self.publish(&mut next, &all_staged, &images)?;
        self.index.push_newest(IndexReader::parse(&images[1])?);
        self.manifest = next;
        Ok(())
    }

    /// Read back the TDA result image published for `filtration` under `run`.
    pub fn open_tda(&self, filtration: FiltrationId, run: RunId) -> Result<Vec<u8>> {
        let key = keys::tda_by_run(run, filtration);
        let rel = self.get_index(&key).ok_or(PrismError::NotFound)?;
        let rel = String::from_utf8(rel).map_err(|_| PrismError::Corrupt {
            detail: "tda path not utf8".into(),
        })?;
        self.read_referenced(&rel)
    }

    /// List the filtration ids that have a TDA result for `run` (merged fragments).
    pub fn tda_filtrations_for_run(&self, run: RunId) -> Vec<FiltrationId> {
        let prefix = keys::tda_by_run_prefix(run);
        let mut out: Vec<FiltrationId> = self
            .index
            .scan_prefix(&prefix)
            .into_iter()
            .filter_map(|(k, _)| {
                // key = prefix(9) + filtration(8 BE)
                let tail = k.get(9..17)?;
                let arr: [u8; 8] = tail.try_into().ok()?;
                Some(FiltrationId::new(u64::from_be_bytes(arr)))
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Read a manifest-referenced derived file by its `.prism/`-relative path,
    /// verifying it is present, the recorded length, and the recorded digest.
    fn read_referenced(&self, rel_path: &str) -> Result<Vec<u8>> {
        let entry = self.manifest.file(rel_path).ok_or(PrismError::NotFound)?;
        let path = self.layout.prism_dir().join(rel_path);
        let bytes = fsutil::read_file(&path)?;
        if bytes.len() as u64 != entry.len {
            return Err(PrismError::Corrupt {
                detail: format!("{rel_path} length mismatch"),
            });
        }
        if sha256(&bytes) != entry.digest {
            return Err(PrismError::ChecksumMismatch {
                detail: rel_path.to_string(),
            });
        }
        Ok(bytes)
    }

    // --- Blob store ------------------------------------------------------------

    /// A handle to this workspace's content-addressed blob store.
    pub fn blobs(&self) -> BlobStore {
        BlobStore::new(self.layout.blobs_dir())
    }
}

/// An owned record read back from a segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRecord {
    pub record_type: u16,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub logical_ts: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::TempDir;

    #[test]
    fn init_open_publish_roundtrip() {
        let td = TempDir::new("store");
        {
            let mut s = Store::init(td.path()).unwrap();
            s.publish_segment(&[
                RawRecord::new(1, b"react".to_vec(), b"npm:react".to_vec()),
                RawRecord::new(1, b"serde".to_vec(), b"cargo:serde".to_vec()),
            ])
            .unwrap();
        } // drop releases the lock

        let s = Store::open(td.path()).unwrap();
        let recs = s.read_all_records().unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].key, b"react");
        assert_eq!(recs[1].value, b"cargo:serde");
        // logical_ts is monotonic and recovered.
        assert_eq!(recs[0].logical_ts, 1);
        assert_eq!(recs[1].logical_ts, 2);
        assert_eq!(s.manifest().logical_ts_hwm, 2);
        assert_eq!(s.manifest().next_segment_id, 2);
    }

    #[test]
    fn multiple_publishes_continue_counters() {
        let td = TempDir::new("store-multi");
        let mut s = Store::init(td.path()).unwrap();
        let id1 = s.publish_segment(&[RawRecord::new(1, b"a".to_vec(), b"1".to_vec())]).unwrap();
        let id2 = s.publish_segment(&[RawRecord::new(1, b"b".to_vec(), b"2".to_vec())]).unwrap();
        assert_eq!((id1, id2), (1, 2));
        assert_eq!(s.read_all_records().unwrap().len(), 2);
        assert_eq!(s.manifest().logical_ts_hwm, 2);
    }

    #[test]
    fn recovery_ignores_tmp_and_orphans() {
        let td = TempDir::new("store-recover");
        {
            let mut s = Store::init(td.path()).unwrap();
            s.publish_segment(&[RawRecord::new(1, b"k".to_vec(), b"v".to_vec())]).unwrap();
        }
        let layout = Layout::new(td.path());
        // Simulate an interrupted publish: a stray staged file and a stray MANIFEST.tmp.
        std::fs::write(layout.tmp_dir().join("000099.pseg"), b"garbage").unwrap();
        std::fs::write(layout.manifest_tmp(), b"half-written").unwrap();
        // An orphan segment file not referenced by the manifest.
        std::fs::write(layout.segment_file(42), b"orphan").unwrap();

        let s = Store::open(td.path()).unwrap();
        // The committed data is intact and the stray tmp file was cleaned.
        assert_eq!(s.read_all_records().unwrap().len(), 1);
        assert!(!layout.tmp_dir().join("000099.pseg").exists());
    }

    #[test]
    fn corrupted_published_segment_is_detected_on_open() {
        let td = TempDir::new("store-corrupt");
        {
            let mut s = Store::init(td.path()).unwrap();
            s.publish_segment(&[RawRecord::new(1, b"k".to_vec(), b"v".to_vec())]).unwrap();
        }
        // Corrupt the published segment in place.
        let layout = Layout::new(td.path());
        let seg = layout.segment_file(1);
        let mut bytes = std::fs::read(&seg).unwrap();
        let n = bytes.len();
        bytes[n / 2] ^= 0xFF;
        std::fs::write(&seg, &bytes).unwrap();

        assert!(matches!(
            Store::open(td.path()),
            Err(PrismError::ChecksumMismatch { .. }) | Err(PrismError::Corrupt { .. })
        ));
    }

    #[test]
    fn double_init_fails() {
        let td = TempDir::new("store-double");
        let _s = Store::init(td.path()).unwrap();
        // lock is held; a second init must fail (already initialized or locked).
        assert!(Store::init(td.path()).is_err());
    }

    // --- Extended tests: records, interning, indexes, derived files ------------

    use prism_model::{
        DependencyKind, EcosystemId, FidelityTier, GraphFidelity, Package, PackageVersion,
        Requirement, ResolverRun, RunStatus, VersionFlags,
    };

    fn make_package(s: &mut Store, eco: EcosystemId, name: &str) -> Package {
        let name_id = s.intern_string(name).unwrap();
        let canon = s.intern_string(&name.to_lowercase()).unwrap();
        let purl = s.intern_string(&format!("pkg:generic/{name}")).unwrap();
        Package {
            id: s.alloc_package_id(),
            ecosystem: eco,
            namespace: None,
            name: name_id,
            canonical_name: canon,
            purl_base: purl,
            repository_url: None,
            homepage_url: None,
            metadata_ref: None,
        }
    }

    fn make_version(s: &mut Store, pkg: PackageId, ver: &str, sort: Vec<u8>) -> PackageVersion {
        let raw = s.intern_string(ver).unwrap();
        let purl = s.intern_string(&format!("pkg:generic/x@{ver}")).unwrap();
        PackageVersion {
            id: s.alloc_version_id(),
            package_id: pkg,
            version_raw: raw,
            version_norm: raw,
            version_sort_key: sort,
            purl,
            released_at_unix: Some(1_700_000_000),
            flags: VersionFlags::EMPTY,
            license_expr: None,
            artifact_hash: None,
            metadata_ref: None,
        }
    }

    #[test]
    fn entity_records_roundtrip_across_reopen() {
        let td = TempDir::new("store-entities");
        let eco = EcosystemId::new(1);
        let (pkg_id, ver_id, req_id, run_id);
        {
            let mut s = Store::init(td.path()).unwrap();
            let pkg = make_package(&mut s, eco, "Serde");
            pkg_id = pkg.id;
            let ver = make_version(&mut s, pkg_id, "1.0.0", vec![1, 0, 0]);
            ver_id = ver.id;
            let constraint = s.blobs().put(b">=1,<2").unwrap();
            let req = Requirement {
                id: s.alloc_requirement_id(),
                src_version_id: ver_id,
                declared_target: s.intern_string("serde_derive").unwrap(),
                dst_package_id: Some(pkg_id),
                target_alias: None,
                raw_requirement: s.intern_string(">=1,<2").unwrap(),
                constraint_ref: constraint,
                managed_version_ref: None,
                kind: DependencyKind::Runtime,
                scope: None,
                condition_kind: prism_model::ConditionKind::None,
                condition_ref: None,
                features_or_extras_ref: None,
                metadata_ref: None,
            };
            req_id = req.id;
            let run = ResolverRun {
                id: s.alloc_run_id(),
                ecosystem: eco,
                root_set_hash: sha256(b"roots"),
                resolver_name: s.intern_string("locked").unwrap(),
                resolver_version: None,
                environment_id: prism_model::EnvironmentId::new(7),
                snapshot_id: prism_model::SnapshotId::new(9),
                input_manifest_ref: None,
                input_lockfile_ref: None,
                status: RunStatus::Complete,
                fidelity: GraphFidelity::exact(FidelityTier::LockfileExact),
                reference_unix: 1_600_000_000,
                created_at_unix: 1_700_000_000,
                metadata_ref: None,
            };
            run_id = run.id;

            let mut b = s.batch();
            s.put_package(&mut b, &pkg);
            s.put_version(&mut b, &ver);
            s.put_requirement(&mut b, &req);
            s.put_run(&mut b, &run);
            s.commit_batch(b).unwrap();

            // Visible immediately within the same session.
            assert_eq!(s.get_package(pkg_id).unwrap().name, pkg.name);
        }

        // Reopen and query everything back.
        let s = Store::open(td.path()).unwrap();
        let canon = s.find_package(eco, None, sha_canon(&s, "serde")).unwrap();
        assert_eq!(canon, pkg_id);
        let pkg = s.get_package(pkg_id).unwrap();
        assert_eq!(s.resolve_string(pkg.name).unwrap(), "Serde");

        assert_eq!(s.versions_by_package(pkg_id), vec![ver_id]);
        let v = s.get_version(ver_id).unwrap();
        assert_eq!(v.version_sort_key, vec![1, 0, 0]);
        assert_eq!(s.find_version_by_purl(v.purl), Some(ver_id));

        assert_eq!(s.requirements_by_source_version(ver_id), vec![req_id]);
        assert_eq!(s.requirements_by_dst_package(pkg_id), vec![req_id]);
        let req = s.get_requirement(req_id).unwrap();
        assert_eq!(req.kind, DependencyKind::Runtime);

        let run = s.get_run(run_id).unwrap();
        assert_eq!(run.status, RunStatus::Complete);
        assert_eq!(s.resolve_string(run.resolver_name).unwrap(), "locked");
    }

    // Helper: look up the StringId of an already-interned canonical name.
    fn sha_canon(s: &Store, text: &str) -> StringId {
        let h = *sha256(text.as_bytes()).as_bytes();
        let bytes = s.get_index(&keys::string_by_text(&h)).unwrap();
        StringId::new(u32::from_be_bytes(bytes.try_into().unwrap()))
    }

    #[test]
    fn string_interning_is_stable_across_reopen() {
        let td = TempDir::new("store-intern");
        let (a, b, c);
        {
            let mut s = Store::init(td.path()).unwrap();
            a = s.intern_string("react").unwrap();
            b = s.intern_string("vue").unwrap();
            c = s.intern_string("react").unwrap(); // dedup -> same id as `a`
            assert_eq!(a, c);
            assert_ne!(a, b);
            assert_eq!(s.resolve_string(a).unwrap(), "react");
        }
        let mut s = Store::open(td.path()).unwrap();
        // Existing ids resolve, and re-interning returns the same ids (recovered
        // from the reverse index, counter recovered from the manifest).
        assert_eq!(s.resolve_string(a).unwrap(), "react");
        assert_eq!(s.resolve_string(b).unwrap(), "vue");
        assert_eq!(s.intern_string("react").unwrap(), a);
        assert_eq!(s.intern_string("vue").unwrap(), b);
        // A brand-new string gets a fresh id strictly above the high-water mark.
        let d = s.intern_string("svelte").unwrap();
        assert!(d.get() > a.get() && d.get() > b.get());
    }

    #[test]
    fn index_publish_merge_and_override_across_reopen() {
        let td = TempDir::new("store-index-merge");
        let key = b"app/feature".to_vec();
        {
            let mut s = Store::init(td.path()).unwrap();
            let mut b = s.batch();
            b.put_index(key.clone(), b"v1".to_vec());
            s.commit_batch(b).unwrap();
            assert_eq!(s.get_index(&key).as_deref(), Some(b"v1".as_slice()));
        }
        {
            // A later publish overrides the same key in a newer segment.
            let mut s = Store::open(td.path()).unwrap();
            assert_eq!(s.get_index(&key).as_deref(), Some(b"v1".as_slice()));
            let mut b = s.batch();
            b.put_index(key.clone(), b"v2".to_vec());
            s.commit_batch(b).unwrap();
            assert_eq!(s.get_index(&key).as_deref(), Some(b"v2".as_slice()));
        }
        // After reopen the override (newest segment) wins the merge.
        let s = Store::open(td.path()).unwrap();
        assert_eq!(s.get_index(&key).as_deref(), Some(b"v2".as_slice()));
    }

    #[test]
    fn versions_list_merges_fragments_across_batches() {
        let td = TempDir::new("store-verlist");
        let eco = EcosystemId::new(1);
        let mut s = Store::init(td.path()).unwrap();
        let pkg = make_package(&mut s, eco, "lib");
        let v1 = make_version(&mut s, pkg.id, "1.0.0", vec![1]);
        let v2 = make_version(&mut s, pkg.id, "2.0.0", vec![2]);
        // Two separate batches -> two list fragments that must union on read.
        let mut b = s.batch();
        s.put_package(&mut b, &pkg);
        s.put_version(&mut b, &v1);
        s.commit_batch(b).unwrap();
        let mut b = s.batch();
        s.put_version(&mut b, &v2);
        s.commit_batch(b).unwrap();
        assert_eq!(s.versions_by_package(pkg.id), vec![v1.id, v2.id]);
    }

    #[test]
    fn publish_graph_and_open_roundtrip() {
        let td = TempDir::new("store-graph");
        let run = RunId::new(42);
        let image = b"PRISMCSR-fake-graph-bytes".to_vec();
        {
            let mut s = Store::init(td.path()).unwrap();
            s.publish_graph(run, &image).unwrap();
            assert_eq!(s.open_graph(run).unwrap(), image);
            assert!(matches!(s.open_graph(RunId::new(99)), Err(PrismError::NotFound)));
        }
        let s = Store::open(td.path()).unwrap();
        assert_eq!(s.open_graph(run).unwrap(), image);
    }

    #[test]
    fn publish_tda_and_open_roundtrip() {
        let td = TempDir::new("store-tda");
        let run = RunId::new(7);
        let f1 = FiltrationId::new(1);
        let f2 = FiltrationId::new(2);
        let img1 = b"PRISMTDA-one".to_vec();
        let img2 = b"PRISMTDA-two".to_vec();
        {
            let mut s = Store::init(td.path()).unwrap();
            s.publish_tda(f1, run, &img1).unwrap();
            s.publish_tda(f2, run, &img2).unwrap();
            assert_eq!(s.open_tda(f1, run).unwrap(), img1);
            assert_eq!(s.open_tda(f2, run).unwrap(), img2);
            assert_eq!(s.tda_filtrations_for_run(run), vec![f1, f2]);
        }
        let s = Store::open(td.path()).unwrap();
        assert_eq!(s.open_tda(f2, run).unwrap(), img2);
        assert_eq!(s.tda_filtrations_for_run(run), vec![f1, f2]);
    }

    #[test]
    fn high_water_marks_persist_across_reopen() {
        let td = TempDir::new("store-hwm");
        let (last_obj, last_str);
        {
            let mut s = Store::init(td.path()).unwrap();
            let _ = s.intern_string("a").unwrap();
            let p = s.alloc_package_id();
            let v = s.alloc_version_id();
            // Commit so the counters are persisted in the manifest.
            let mut b = s.batch();
            b.put_index(b"x".to_vec(), b"y".to_vec());
            s.commit_batch(b).unwrap();
            last_obj = v.get();
            last_str = s.manifest().next_string_id;
            assert!(p.get() < v.get());
        }
        let mut s = Store::open(td.path()).unwrap();
        // New allocations continue strictly above the recovered high-water marks.
        assert!(s.alloc_run_id().get() > last_obj);
        assert!(s.intern_string("brand-new").unwrap().get() >= last_str);
    }

    #[test]
    fn recovery_still_works_with_indexes_and_derived_files() {
        let td = TempDir::new("store-recover-ext");
        let run = RunId::new(3);
        {
            let mut s = Store::init(td.path()).unwrap();
            let _ = s.intern_string("keep").unwrap();
            s.publish_graph(run, b"graph-bytes").unwrap();
        }
        let layout = Layout::new(td.path());
        // Simulate an interrupted publish and an orphan index file.
        std::fs::write(layout.tmp_dir().join("000099.pidx"), b"garbage").unwrap();
        std::fs::write(layout.manifest_tmp(), b"half").unwrap();
        std::fs::write(layout.index_file(998), b"orphan").unwrap();

        let s = Store::open(td.path()).unwrap();
        // Committed data intact; tmp cleaned; graph still readable.
        assert_eq!(s.open_graph(run).unwrap(), b"graph-bytes");
        assert!(!layout.tmp_dir().join("000099.pidx").exists());
    }
}
