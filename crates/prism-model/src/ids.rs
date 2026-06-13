//! Typed identifiers.
//!
//! Widths and stability scope are normative (architecture doc, *Identity and ID
//! Model*). Store-global integer IDs are monotonic and never reused;
//! [`GraphNodeId`]/[`GraphEdgeId`] are dense and snapshot-local.

id_newtype!(
    /// Interned UTF-8 atom handle. Store-global, `u32`.
    StringId(u32)
);
id_newtype!(
    /// Ecosystem registry handle (npm, cargo, …). Process-global, `u32`.
    EcosystemId(u32)
);
id_newtype!(
    /// Package family identifier. Store-global, `u64`.
    PackageId(u64)
);
id_newtype!(
    /// Concrete package-version identifier. Store-global, `u64`.
    VersionId(u64)
);
id_newtype!(
    /// Canonicalized requirement identifier. Store-global, `u64`.
    RequirementId(u64)
);
id_newtype!(
    /// Content-addressed environment identifier (truncated hash). Store-global, `u64`.
    EnvironmentId(u64)
);
id_newtype!(
    /// Registry-metadata snapshot identifier. Store-global, `u64`.
    SnapshotId(u64)
);
id_newtype!(
    /// Resolver-run identifier. Store-global, `u64`.
    RunId(u64)
);
id_newtype!(
    /// Filtration identifier. Store-global, `u64`.
    FiltrationId(u64)
);
id_newtype!(
    /// Globally-unique persistence-pair identifier. Store-global, `u64`.
    PairId(u64)
);
id_newtype!(
    /// TDA cell identifier. Filtration-local, `u64`.
    CellId(u64)
);

id_newtype!(
    /// Dense, **snapshot-local** graph node index. Meaningless across runs.
    /// [`GraphNodeId::NULL`] is the reserved sentinel.
    GraphNodeId(u32)
);
id_newtype!(
    /// Dense, **snapshot-local** graph edge index (= CSR position).
    GraphEdgeId(u32)
);

impl GraphNodeId {
    /// Reserved NULL sentinel (`u32::MAX`); never a valid node.
    pub const NULL: GraphNodeId = GraphNodeId(u32::MAX);
    /// Maximum addressable nodes in one snapshot (`u32::MAX - 1`).
    pub const MAX_NODES: u32 = u32::MAX - 1;

    /// True if this is the [`GraphNodeId::NULL`] sentinel.
    #[inline]
    pub const fn is_null(self) -> bool {
        self.0 == u32::MAX
    }

    /// Index into a `node_count`-length array, or `None` for the sentinel.
    #[inline]
    pub const fn index(self) -> Option<usize> {
        if self.is_null() {
            None
        } else {
            Some(self.0 as usize)
        }
    }
}

impl GraphEdgeId {
    /// Reserved NULL sentinel (`u32::MAX`).
    pub const NULL: GraphEdgeId = GraphEdgeId(u32::MAX);
    /// Maximum addressable edges in one snapshot (`u32::MAX - 1`).
    pub const MAX_EDGES: u32 = u32::MAX - 1;
}

/// A SHA-256 digest. The content-address used for blobs and artifact hashes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct HashDigest(pub [u8; 32]);

impl HashDigest {
    /// Wrap raw 32 bytes.
    #[inline]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw bytes.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hex encoding (64 chars).
    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
            s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
        }
        s
    }
}

impl core::fmt::Debug for HashDigest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "HashDigest({})", self.to_hex())
    }
}

impl core::fmt::Display for HashDigest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Content-addressed blob handle (the SHA-256 of the *redacted* bytes).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BlobId(pub HashDigest);

impl BlobId {
    /// The underlying digest.
    #[inline]
    pub const fn digest(&self) -> &HashDigest {
        &self.0
    }

    /// The `sha256/ab/abcdef…` relative path fragment for this blob: the first
    /// hex byte is the shard directory, the full hex is the file stem.
    pub fn shard_path(&self) -> String {
        let hex = self.0.to_hex();
        format!("sha256/{}/{}", &hex[0..2], hex)
    }
}

impl core::fmt::Debug for BlobId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "BlobId({})", self.0.to_hex())
    }
}

/// A physical record location, **stable only within a store epoch** (compaction
/// reassigns segment ids and offsets). Never persisted inside object records or
/// immutable derived files — only logical IDs cross those boundaries.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct RecordRef {
    /// Owning segment id (globally unique, monotonic, never reused).
    pub segment_id: u32,
    /// Byte offset of the record within the segment.
    pub offset: u64,
    /// Record length in bytes.
    pub len: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_roundtrip_and_display() {
        let p = PackageId::new(42);
        assert_eq!(p.get(), 42);
        assert_eq!(PackageId::from(42u64), p);
        assert_eq!(p.to_string(), "42");
        assert_eq!(format!("{p:?}"), "PackageId(42)");
    }

    #[test]
    fn ids_order_numerically() {
        assert!(VersionId::new(2) < VersionId::new(10));
        let mut v = [VersionId::new(10), VersionId::new(2), VersionId::new(7)];
        v.sort();
        assert_eq!(v, [VersionId::new(2), VersionId::new(7), VersionId::new(10)]);
    }

    #[test]
    fn graph_node_null_sentinel() {
        assert!(GraphNodeId::NULL.is_null());
        assert_eq!(GraphNodeId::NULL.index(), None);
        assert_eq!(GraphNodeId::new(3).index(), Some(3));
        assert!(!GraphNodeId::new(0).is_null());
        assert_eq!(GraphNodeId::MAX_NODES, u32::MAX - 1);
    }

    #[test]
    fn hash_hex_and_shard_path() {
        let mut bytes = [0u8; 32];
        bytes[0] = 0xab;
        bytes[1] = 0xcd;
        let h = HashDigest::from_bytes(bytes);
        let hex = h.to_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.starts_with("abcd"));
        let blob = BlobId(h);
        assert!(blob.shard_path().starts_with("sha256/ab/abcd"));
    }
}
