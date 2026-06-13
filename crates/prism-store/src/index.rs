//! Typed index segments (architecture doc, *Typed Index Segments*).
//!
//! Lookups read an in-memory memtable first, then immutable sorted `.pidx`
//! segments newest-to-oldest; the first hit wins (newer segments carry newer
//! `logical_ts`). Deletes are tombstones that hide older entries until compaction.
//!
//! Keys are opaque sorted byte strings; the [`keys`] helpers build the typed,
//! big-endian-encoded keys for the concrete indexes so that lexical order matches
//! numeric order for IDs.

use prism_codec::{read_uvarint, write_uvarint, ByteReader, ByteWriter};
use prism_model::{PrismError, Result};
use std::collections::BTreeMap;

/// Index file magic: `b"PRISMPIDX"`.
pub const INDEX_MAGIC: &[u8; 9] = b"PRISMPIDX";
/// Current index format version.
pub const INDEX_FORMAT_VERSION: u32 = 1;

const FLAG_TOMBSTONE: u8 = 1;

/// A value bound to a key, with its version and liveness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexEntry {
    pub value: Vec<u8>,
    pub logical_ts: u64,
    pub tombstone: bool,
}

/// The in-memory write buffer. Sorted by key for deterministic flush.
#[derive(Default)]
pub struct Memtable {
    map: BTreeMap<Vec<u8>, IndexEntry>,
}

impl Memtable {
    /// A new empty memtable.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or overwrite `key` with `value` at `logical_ts`.
    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>, logical_ts: u64) {
        self.map.insert(
            key,
            IndexEntry {
                value,
                logical_ts,
                tombstone: false,
            },
        );
    }

    /// Record a tombstone for `key` at `logical_ts`.
    pub fn delete(&mut self, key: Vec<u8>, logical_ts: u64) {
        self.map.insert(
            key,
            IndexEntry {
                value: Vec::new(),
                logical_ts,
                tombstone: true,
            },
        );
    }

    /// Look up `key` within this memtable only.
    pub fn get(&self, key: &[u8]) -> Option<&IndexEntry> {
        self.map.get(key)
    }

    /// Number of buffered entries.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True if empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Iterate entries in sorted key order.
    pub fn iter(&self) -> impl Iterator<Item = (&Vec<u8>, &IndexEntry)> {
        self.map.iter()
    }

    /// Iterate `(key, entry)` pairs whose key starts with `prefix`, in sorted order.
    pub fn scan_prefix<'a>(
        &'a self,
        prefix: &'a [u8],
    ) -> impl Iterator<Item = (&'a Vec<u8>, &'a IndexEntry)> {
        self.map
            .range(prefix.to_vec()..)
            .take_while(move |(k, _)| k.starts_with(prefix))
    }

    /// Serialize this memtable to an immutable `.pidx` image.
    pub fn freeze(&self, index_id: u32, created_at: i64) -> Vec<u8> {
        let mut w = ByteWriter::new();
        w.write_bytes(INDEX_MAGIC);
        w.write_u32(INDEX_FORMAT_VERSION);
        w.write_u32(index_id);
        w.write_i64(created_at);
        w.write_u32(self.map.len() as u32);
        for (key, entry) in &self.map {
            write_uvarint(&mut w, key.len() as u64);
            w.write_bytes(key);
            w.write_u8(if entry.tombstone { FLAG_TOMBSTONE } else { 0 });
            w.write_u64(entry.logical_ts);
            write_uvarint(&mut w, entry.value.len() as u64);
            w.write_bytes(&entry.value);
        }
        let crc = prism_codec::crc32(w.as_slice());
        w.write_u32(crc);
        w.into_vec()
    }
}

/// A parsed immutable index segment, sorted by key for binary search.
pub struct IndexReader {
    entries: Vec<(Vec<u8>, IndexEntry)>,
}

impl IndexReader {
    /// Parse a `.pidx` image, validating magic, version, and trailing CRC.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 4 {
            return Err(PrismError::Corrupt {
                detail: "index too short".into(),
            });
        }
        let (body, crc_bytes) = bytes.split_at(bytes.len() - 4);
        let expected = u32::from_le_bytes(crc_bytes.try_into().unwrap());
        if prism_codec::crc32(body) != expected {
            return Err(PrismError::ChecksumMismatch {
                detail: "index crc".into(),
            });
        }

        let mut r = ByteReader::new(body);
        if r.read_bytes(INDEX_MAGIC.len())? != INDEX_MAGIC {
            return Err(PrismError::Corrupt {
                detail: "bad index magic".into(),
            });
        }
        let version = r.read_u32()?;
        if version != INDEX_FORMAT_VERSION {
            return Err(PrismError::UnsupportedFormatVersion {
                found: version,
                max: INDEX_FORMAT_VERSION,
            });
        }
        let _index_id = r.read_u32()?;
        let _created_at = r.read_i64()?;
        let count = r.read_u32()? as usize;

        // Each entry encodes to at least 11 bytes (1B klen varint + 1B flags + 8B
        // logical_ts + 1B vlen varint); reject a count that cannot fit so a forged
        // header cannot drive a multi-GB reservation before the element reads fail.
        const MIN_ENTRY_BYTES: usize = 11;
        if count > r.remaining() / MIN_ENTRY_BYTES {
            return Err(PrismError::Corrupt {
                detail: format!("index entry count {count} exceeds remaining bytes"),
            });
        }
        let mut entries = Vec::with_capacity(count);
        let mut prev: Option<Vec<u8>> = None;
        for _ in 0..count {
            let klen = read_uvarint(&mut r)? as usize;
            let key = r.read_bytes(klen)?.to_vec();
            if let Some(p) = &prev {
                if &key <= p {
                    return Err(PrismError::Corrupt {
                        detail: "index keys not strictly sorted".into(),
                    });
                }
            }
            prev = Some(key.clone());
            let flags = r.read_u8()?;
            let logical_ts = r.read_u64()?;
            let vlen = read_uvarint(&mut r)? as usize;
            let value = r.read_bytes(vlen)?.to_vec();
            entries.push((
                key,
                IndexEntry {
                    value,
                    logical_ts,
                    tombstone: flags & FLAG_TOMBSTONE != 0,
                },
            ));
        }
        Ok(Self { entries })
    }

    /// Look up `key` within this segment only.
    pub fn get(&self, key: &[u8]) -> Option<&IndexEntry> {
        self.entries
            .binary_search_by(|(k, _)| k.as_slice().cmp(key))
            .ok()
            .map(|i| &self.entries[i].1)
    }

    /// Iterate `(key, entry)` pairs whose key starts with `prefix`, in sorted key
    /// order (within this segment only).
    pub fn scan_prefix<'a>(
        &'a self,
        prefix: &'a [u8],
    ) -> impl Iterator<Item = (&'a [u8], &'a IndexEntry)> {
        // Binary-search the first key >= prefix, then walk while still prefixed.
        let start = self
            .entries
            .partition_point(|(k, _)| k.as_slice() < prefix);
        self.entries[start..]
            .iter()
            .take_while(move |(k, _)| k.starts_with(prefix))
            .map(|(k, e)| (k.as_slice(), e))
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The merged read view: memtable over immutable segments (newest first).
#[derive(Default)]
pub struct IndexSet {
    memtable: Memtable,
    /// Immutable segments ordered newest → oldest.
    readers: Vec<IndexReader>,
}

impl IndexSet {
    /// A new empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Mutable access to the memtable.
    pub fn memtable_mut(&mut self) -> &mut Memtable {
        &mut self.memtable
    }

    /// Add an immutable segment as the newest one (call in newest-first order).
    pub fn push_newest(&mut self, reader: IndexReader) {
        self.readers.insert(0, reader);
    }

    /// Add an immutable segment as the oldest one.
    pub fn push_oldest(&mut self, reader: IndexReader) {
        self.readers.push(reader);
    }

    /// Resolve `key` to its live value across the memtable and every segment.
    ///
    /// The winner is the entry with the highest `logical_ts` (the documented
    /// "newest wins" total order), not merely the first source that holds the key.
    /// Today production readers are only ever `push_newest` in publish order, so
    /// file order already equals `logical_ts` order — but comparing `logical_ts`
    /// keeps `get` correct (and consistent with `scan_prefix`) regardless of
    /// insertion order or any future compaction. Returns `None` if absent or the
    /// winning entry is a tombstone.
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        let mut best: Option<&IndexEntry> = self.memtable.get(key);
        for r in &self.readers {
            if let Some(e) = r.get(key) {
                if best.map_or(true, |b| e.logical_ts > b.logical_ts) {
                    best = Some(e);
                }
            }
        }
        match best {
            Some(e) if !e.tombstone => Some(e.value.as_slice()),
            _ => None,
        }
    }

    /// Merge all list-fragment entries whose key starts with `prefix` across the
    /// memtable and every segment (architecture doc, *List-valued keys*). For each
    /// distinct full key the entry with the highest `logical_ts` wins; tombstoned
    /// winners are dropped. Returns `(full_key, value)` pairs sorted by key.
    pub fn scan_prefix(&self, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
        use std::collections::BTreeMap;
        // For each distinct full key keep the entry with the highest logical_ts.
        let mut winners: BTreeMap<&[u8], &IndexEntry> = BTreeMap::new();
        for (k, e) in self.memtable.scan_prefix(prefix) {
            match winners.get(k.as_slice()) {
                Some(prev) if prev.logical_ts >= e.logical_ts => {}
                _ => {
                    winners.insert(k.as_slice(), e);
                }
            }
        }
        for r in &self.readers {
            for (k, e) in r.scan_prefix(prefix) {
                match winners.get(k) {
                    Some(prev) if prev.logical_ts >= e.logical_ts => {}
                    _ => {
                        winners.insert(k, e);
                    }
                }
            }
        }
        winners
            .into_iter()
            .filter(|(_, e)| !e.tombstone)
            .map(|(k, e)| (k.to_vec(), e.value.clone()))
            .collect()
    }
}

/// Typed key builders for the concrete indexes. Prefix byte + big-endian payloads
/// so lexical order matches numeric order.
pub mod keys {
    use prism_model::{
        EcosystemId, FiltrationId, PackageId, RequirementId, RunId, StringId, VersionId,
    };

    /// Index prefixes (the first byte of every typed key).
    #[repr(u8)]
    pub enum Prefix {
        PkgByName = 1,
        PkgById = 2,
        VerById = 3,
        VerByPurl = 4,
        VersByPackage = 5,
        ReqById = 6,
        ReqsBySrcVer = 7,
        ReqsByDstPackage = 8,
        RunById = 9,
        /// `STRING_BY_ID`: interner `StringId -> atom record key`.
        StringById = 10,
        /// `STRING_BY_TEXT`: interner reverse lookup `hash(text) -> StringId`.
        StringByText = 11,
        /// `GRAPH_BY_RUN`: `RunId -> graph snapshot path`.
        GraphByRun = 12,
        /// `TDA_BY_RUN`: `RunId -> tda result path(s)`.
        TdaByRun = 13,
    }

    /// `PKG_BY_NAME`: ecosystem + namespace(StringId, 0 if none) + canonical_name.
    pub fn pkg_by_name(eco: EcosystemId, namespace: Option<StringId>, canonical: StringId) -> Vec<u8> {
        let mut k = Vec::with_capacity(1 + 4 + 4 + 4);
        k.push(Prefix::PkgByName as u8);
        k.extend_from_slice(&eco.get().to_be_bytes());
        k.extend_from_slice(&namespace.map(|s| s.get()).unwrap_or(0).to_be_bytes());
        k.extend_from_slice(&canonical.get().to_be_bytes());
        k
    }

    /// `PKG_BY_ID`.
    pub fn pkg_by_id(id: PackageId) -> Vec<u8> {
        let mut k = Vec::with_capacity(9);
        k.push(Prefix::PkgById as u8);
        k.extend_from_slice(&id.get().to_be_bytes());
        k
    }

    /// `VER_BY_ID`.
    pub fn ver_by_id(id: VersionId) -> Vec<u8> {
        let mut k = Vec::with_capacity(9);
        k.push(Prefix::VerById as u8);
        k.extend_from_slice(&id.get().to_be_bytes());
        k
    }

    /// `VERS_BY_PACKAGE`.
    pub fn vers_by_package(id: PackageId) -> Vec<u8> {
        let mut k = Vec::with_capacity(9);
        k.push(Prefix::VersByPackage as u8);
        k.extend_from_slice(&id.get().to_be_bytes());
        k
    }

    /// `VER_BY_PURL`: interned purl `StringId -> VersionId`.
    pub fn ver_by_purl(purl: StringId) -> Vec<u8> {
        let mut k = Vec::with_capacity(5);
        k.push(Prefix::VerByPurl as u8);
        k.extend_from_slice(&purl.get().to_be_bytes());
        k
    }

    /// `REQ_BY_ID`.
    pub fn req_by_id(id: RequirementId) -> Vec<u8> {
        let mut k = Vec::with_capacity(9);
        k.push(Prefix::ReqById as u8);
        k.extend_from_slice(&id.get().to_be_bytes());
        k
    }

    /// `REQS_BY_SRC_VER`: per-`(VersionId, RequirementId)` membership entry. The
    /// requirement id is folded into the key so each requirement is its own
    /// append-only entry (a list fragment), unioned across segments at read time.
    pub fn reqs_by_src_ver(src: VersionId, req: RequirementId) -> Vec<u8> {
        let mut k = Vec::with_capacity(17);
        k.push(Prefix::ReqsBySrcVer as u8);
        k.extend_from_slice(&src.get().to_be_bytes());
        k.extend_from_slice(&req.get().to_be_bytes());
        k
    }

    /// The key prefix that all [`reqs_by_src_ver`] entries for `src` share, for a
    /// prefix scan over the index.
    pub fn reqs_by_src_ver_prefix(src: VersionId) -> Vec<u8> {
        let mut k = Vec::with_capacity(9);
        k.push(Prefix::ReqsBySrcVer as u8);
        k.extend_from_slice(&src.get().to_be_bytes());
        k
    }

    /// `REQS_BY_DST_PACKAGE`: per-`(PackageId, RequirementId)` membership entry.
    pub fn reqs_by_dst_package(dst: PackageId, req: RequirementId) -> Vec<u8> {
        let mut k = Vec::with_capacity(17);
        k.push(Prefix::ReqsByDstPackage as u8);
        k.extend_from_slice(&dst.get().to_be_bytes());
        k.extend_from_slice(&req.get().to_be_bytes());
        k
    }

    /// The shared key prefix of all [`reqs_by_dst_package`] entries for `dst`.
    pub fn reqs_by_dst_package_prefix(dst: PackageId) -> Vec<u8> {
        let mut k = Vec::with_capacity(9);
        k.push(Prefix::ReqsByDstPackage as u8);
        k.extend_from_slice(&dst.get().to_be_bytes());
        k
    }

    /// `RUN_BY_ID`.
    pub fn run_by_id(id: RunId) -> Vec<u8> {
        let mut k = Vec::with_capacity(9);
        k.push(Prefix::RunById as u8);
        k.extend_from_slice(&id.get().to_be_bytes());
        k
    }

    /// `VERS_BY_PACKAGE` per-`(PackageId, VersionId)` membership entry (list fragment).
    pub fn vers_by_package_entry(pkg: PackageId, ver: VersionId) -> Vec<u8> {
        let mut k = Vec::with_capacity(17);
        k.push(Prefix::VersByPackage as u8);
        k.extend_from_slice(&pkg.get().to_be_bytes());
        k.extend_from_slice(&ver.get().to_be_bytes());
        k
    }

    /// `STRING_BY_ID`: interner forward map `StringId -> atom`.
    pub fn string_by_id(id: StringId) -> Vec<u8> {
        let mut k = Vec::with_capacity(5);
        k.push(Prefix::StringById as u8);
        k.extend_from_slice(&id.get().to_be_bytes());
        k
    }

    /// `STRING_BY_TEXT`: interner reverse map keyed by a 32-byte content hash of the
    /// atom text (so the key length is fixed regardless of text length).
    pub fn string_by_text(text_hash: &[u8; 32]) -> Vec<u8> {
        let mut k = Vec::with_capacity(33);
        k.push(Prefix::StringByText as u8);
        k.extend_from_slice(text_hash);
        k
    }

    /// `GRAPH_BY_RUN`: `RunId -> graph snapshot path`.
    pub fn graph_by_run(id: RunId) -> Vec<u8> {
        let mut k = Vec::with_capacity(9);
        k.push(Prefix::GraphByRun as u8);
        k.extend_from_slice(&id.get().to_be_bytes());
        k
    }

    /// `TDA_BY_RUN`: per-`(RunId, FiltrationId)` membership entry (list fragment).
    pub fn tda_by_run(run: RunId, filtration: FiltrationId) -> Vec<u8> {
        let mut k = Vec::with_capacity(17);
        k.push(Prefix::TdaByRun as u8);
        k.extend_from_slice(&run.get().to_be_bytes());
        k.extend_from_slice(&filtration.get().to_be_bytes());
        k
    }

    /// The shared key prefix of all [`tda_by_run`] entries for `run`.
    pub fn tda_by_run_prefix(run: RunId) -> Vec<u8> {
        let mut k = Vec::with_capacity(9);
        k.push(Prefix::TdaByRun as u8);
        k.extend_from_slice(&run.get().to_be_bytes());
        k
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memtable_freeze_parse_roundtrip() {
        let mut mt = Memtable::new();
        mt.put(b"alpha".to_vec(), b"1".to_vec(), 10);
        mt.put(b"beta".to_vec(), b"2".to_vec(), 11);
        mt.delete(b"gamma".to_vec(), 12);
        let bytes = mt.freeze(1, 0);

        let r = IndexReader::parse(&bytes).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r.get(b"alpha").unwrap().value, b"1");
        assert!(r.get(b"gamma").unwrap().tombstone);
        assert!(r.get(b"missing").is_none());
    }

    #[test]
    fn merge_prefers_newer_and_honors_tombstone() {
        // Old segment: key=k -> "old"
        let mut old = Memtable::new();
        old.put(b"k".to_vec(), b"old".to_vec(), 1);
        old.put(b"keep".to_vec(), b"v".to_vec(), 1);
        let old_reader = IndexReader::parse(&old.freeze(1, 0)).unwrap();

        // Newer segment: key=k -> "new"
        let mut new = Memtable::new();
        new.put(b"k".to_vec(), b"new".to_vec(), 5);
        let new_reader = IndexReader::parse(&new.freeze(2, 0)).unwrap();

        let mut set = IndexSet::new();
        set.push_oldest(old_reader);
        set.push_newest(new_reader);
        assert_eq!(set.get(b"k"), Some(b"new".as_slice()));
        assert_eq!(set.get(b"keep"), Some(b"v".as_slice()));

        // A memtable tombstone hides everything older.
        set.memtable_mut().delete(b"k".to_vec(), 9);
        assert_eq!(set.get(b"k"), None);
        assert_eq!(set.get(b"keep"), Some(b"v".as_slice()));
    }

    #[test]
    fn corrupt_index_detected() {
        let mut mt = Memtable::new();
        mt.put(b"a".to_vec(), b"b".to_vec(), 1);
        let mut bytes = mt.freeze(1, 0);
        let n = bytes.len();
        bytes[n / 2] ^= 0xFF;
        assert!(IndexReader::parse(&bytes).is_err());
    }

    #[test]
    fn typed_keys_sort_numerically() {
        use prism_model::PackageId;
        let k2 = keys::pkg_by_id(PackageId::new(2));
        let k10 = keys::pkg_by_id(PackageId::new(10));
        assert!(k2 < k10, "big-endian keys must sort numerically");
    }

    #[test]
    fn scan_prefix_merges_fragments_newest_wins() {
        // Old segment: two members under prefix P, plus one unrelated key.
        let mut old = Memtable::new();
        old.put(b"P\x00\x01".to_vec(), b"a".to_vec(), 1);
        old.put(b"P\x00\x02".to_vec(), b"b".to_vec(), 2);
        old.put(b"Q\x00\x01".to_vec(), b"z".to_vec(), 3);
        let old_reader = IndexReader::parse(&old.freeze(1, 0)).unwrap();

        // Newer segment: overrides member 2, adds member 3, tombstones member 1.
        let mut new = Memtable::new();
        new.put(b"P\x00\x02".to_vec(), b"b2".to_vec(), 10);
        new.put(b"P\x00\x03".to_vec(), b"c".to_vec(), 11);
        new.delete(b"P\x00\x01".to_vec(), 12);
        let new_reader = IndexReader::parse(&new.freeze(2, 0)).unwrap();

        let mut set = IndexSet::new();
        set.push_oldest(old_reader);
        set.push_newest(new_reader);

        let got = set.scan_prefix(b"P\x00");
        // member 1 tombstoned -> dropped; member 2 overridden; members 2,3 live.
        assert_eq!(
            got,
            vec![
                (b"P\x00\x02".to_vec(), b"b2".to_vec()),
                (b"P\x00\x03".to_vec(), b"c".to_vec()),
            ]
        );

        // The unrelated prefix Q is untouched.
        assert_eq!(set.scan_prefix(b"Q\x00"), vec![(b"Q\x00\x01".to_vec(), b"z".to_vec())]);

        // A live memtable fragment participates and wins by logical_ts.
        set.memtable_mut().put(b"P\x00\x03".to_vec(), b"c-new".to_vec(), 20);
        let got = set.scan_prefix(b"P\x00");
        assert_eq!(got.last().unwrap().1, b"c-new");
    }
}
