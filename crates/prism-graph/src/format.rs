//! The `.pcsr` graph-snapshot file format (architecture doc, *Appendix B:
//! PRISMCSR* and *CSR Graph Snapshots*).
//!
//! Layout (all little-endian, matching `prism-codec`):
//!
//! ```text
//! magic           "PRISMCSR" (8 bytes)
//! format_version  u32
//! run_id          u64
//! node_count      u32   (N)
//! edge_count      u32   (M)
//! flags           u32   (bit0 = node_attr column present)
//! node_version_id [u64; N]
//! node_depth      [u32; N]
//! node_flags      [u32; N]
//! node_attr       [u32; N]   (only if flags bit0 set)
//! out_offsets     [u32; N+1]
//! out_targets     [u32; M]
//! out_edge_ids    [u32; M]
//! in_offsets      [u32; N+1]
//! in_sources      [u32; M]
//! in_edge_ids     [u32; M]
//! edge_ordinal    [u32; M]
//! edge_req_id     [u64; M]
//! edge_kind       [u16; M]
//! edge_scope      [u32; M]
//! edge_flags      [u32; M]
//! footer_crc32    u32   (crc32 over every preceding byte)
//! ```
//!
//! The footer CRC covers the whole image; a roundtrip is byte-exact and any
//! single-byte corruption is detected on read as [`PrismError::ChecksumMismatch`].

use prism_codec::{crc32, ByteReader, ByteWriter};
use prism_model::{
    GraphEdgeId, GraphNodeId, NodeFlags, PrismError, RequirementId, Result, RunId, StringId,
    VersionId,
};

use crate::snapshot::{GraphSnapshot, SnapshotParts};

/// File magic: `PRISMCSR`.
pub const PCSR_MAGIC: &[u8; 8] = b"PRISMCSR";
/// The format version this build reads and writes.
pub const PCSR_FORMAT_VERSION: u32 = 1;

/// `flags` bit: the `node_attr` column is present.
const FLAG_HAS_ATTR: u32 = 1 << 0;

/// Serialize a [`GraphSnapshot`] into a self-contained `.pcsr` byte image.
pub fn write_pcsr(snapshot: &GraphSnapshot) -> Vec<u8> {
    let p = snapshot.parts_ref();
    let n = p.node_version_id.len();
    let m = p.out_targets.len();

    let mut flags = 0u32;
    if p.node_attr.is_some() {
        flags |= FLAG_HAS_ATTR;
    }

    let mut w = ByteWriter::with_capacity(64 + n * 16 + m * 28);
    w.write_bytes(PCSR_MAGIC);
    w.write_u32(PCSR_FORMAT_VERSION);
    w.write_u64(p.run_id.get());
    w.write_u32(n as u32);
    w.write_u32(m as u32);
    w.write_u32(flags);

    for v in p.node_version_id {
        w.write_u64(v.get());
    }
    for &d in p.node_depth {
        w.write_u32(d);
    }
    for f in p.node_flags {
        w.write_u32(f.bits());
    }
    if let Some(attr) = p.node_attr {
        for s in attr {
            w.write_u32(s.get());
        }
    }

    for &o in p.out_offsets {
        w.write_u32(o);
    }
    for t in p.out_targets {
        w.write_u32(t.get());
    }
    for e in p.out_edge_ids {
        w.write_u32(e.get());
    }
    for &o in p.in_offsets {
        w.write_u32(o);
    }
    for s in p.in_sources {
        w.write_u32(s.get());
    }
    for e in p.in_edge_ids {
        w.write_u32(e.get());
    }

    for &ord in p.edge_ordinal {
        w.write_u32(ord);
    }
    for r in p.edge_requirement_id {
        w.write_u64(r.get());
    }
    for &k in p.edge_kind {
        w.write_u16(k);
    }
    for s in p.edge_scope {
        w.write_u32(s.get());
    }
    for &f in p.edge_flags {
        w.write_u32(f);
    }

    let crc = crc32(w.as_slice());
    w.write_u32(crc);
    w.into_vec()
}

fn read_u32_vec(r: &mut ByteReader<'_>, n: usize) -> Result<Vec<u32>> {
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(r.read_u32()?);
    }
    Ok(v)
}

/// Parse a `.pcsr` byte image into a validated [`GraphSnapshot`].
///
/// Errors:
/// - [`PrismError::Corrupt`] — wrong magic, truncated, or a structural invariant
///   violated (offsets, lengths, out-of-range neighbor).
/// - [`PrismError::UnsupportedFormatVersion`] — a newer on-disk format.
/// - [`PrismError::ChecksumMismatch`] — the whole-image CRC does not match.
pub fn read_pcsr(bytes: &[u8]) -> Result<GraphSnapshot> {
    // The trailing 4 bytes are the CRC; everything before is covered by it.
    if bytes.len() < 4 {
        return Err(PrismError::Corrupt {
            detail: "pcsr image too short for footer".into(),
        });
    }
    let (body, crc_bytes) = bytes.split_at(bytes.len() - 4);
    let stored_crc = u32::from_le_bytes([crc_bytes[0], crc_bytes[1], crc_bytes[2], crc_bytes[3]]);
    let actual_crc = crc32(body);
    if stored_crc != actual_crc {
        return Err(PrismError::ChecksumMismatch {
            detail: format!(".pcsr crc mismatch (stored {stored_crc:#x}, actual {actual_crc:#x})"),
        });
    }

    let mut r = ByteReader::new(body);
    let magic = r.read_array::<8>()?;
    if &magic != PCSR_MAGIC {
        return Err(PrismError::Corrupt {
            detail: "bad .pcsr magic".into(),
        });
    }
    let version = r.read_u32()?;
    if version > PCSR_FORMAT_VERSION {
        return Err(PrismError::UnsupportedFormatVersion {
            found: version,
            max: PCSR_FORMAT_VERSION,
        });
    }
    let run_id = RunId::new(r.read_u64()?);
    let n = r.read_u32()? as usize;
    let m = r.read_u32()? as usize;
    let flags = r.read_u32()?;

    // Bound N and M against the model caps before allocating, so a corrupt huge
    // count cannot trigger an enormous allocation.
    if n as u64 > GraphNodeId::MAX_NODES as u64 {
        return Err(PrismError::Corrupt {
            detail: format!("pcsr node_count {n} exceeds MAX_NODES"),
        });
    }
    if m as u64 > GraphEdgeId::MAX_EDGES as u64 {
        return Err(PrismError::Corrupt {
            detail: format!("pcsr edge_count {m} exceeds MAX_EDGES"),
        });
    }
    // Also bound the counts against the bytes that actually remain: each node
    // contributes at least 16 body bytes (version 8 + depth 4 + flags 4) and each
    // edge at least 2 (edge_kind u16), so a forged count far larger than the file
    // cannot drive a huge `Vec::with_capacity` before the element reads fail. (The
    // CRC is not a MAC; this is the allocation guard the comment above promised.)
    let body = r.remaining();
    if n > body / 16 || m > body / 2 {
        return Err(PrismError::Corrupt {
            detail: format!("pcsr counts (n={n}, m={m}) exceed remaining {body} bytes"),
        });
    }

    let mut node_version_id = Vec::with_capacity(n);
    for _ in 0..n {
        node_version_id.push(VersionId::new(r.read_u64()?));
    }
    let node_depth = read_u32_vec(&mut r, n)?;
    let mut node_flags = Vec::with_capacity(n);
    for _ in 0..n {
        node_flags.push(NodeFlags::from_bits_truncate(r.read_u32()?));
    }
    let node_attr = if flags & FLAG_HAS_ATTR != 0 {
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(StringId::new(r.read_u32()?));
        }
        Some(v)
    } else {
        None
    };

    let out_offsets = read_u32_vec(&mut r, n + 1)?;
    let out_targets = read_u32_vec(&mut r, m)?
        .into_iter()
        .map(GraphNodeId::new)
        .collect();
    let out_edge_ids = read_u32_vec(&mut r, m)?
        .into_iter()
        .map(GraphEdgeId::new)
        .collect();
    let in_offsets = read_u32_vec(&mut r, n + 1)?;
    let in_sources = read_u32_vec(&mut r, m)?
        .into_iter()
        .map(GraphNodeId::new)
        .collect();
    let in_edge_ids = read_u32_vec(&mut r, m)?
        .into_iter()
        .map(GraphEdgeId::new)
        .collect();

    let edge_ordinal = read_u32_vec(&mut r, m)?;
    let mut edge_requirement_id = Vec::with_capacity(m);
    for _ in 0..m {
        edge_requirement_id.push(RequirementId::new(r.read_u64()?));
    }
    let mut edge_kind = Vec::with_capacity(m);
    for _ in 0..m {
        let k = r.read_u16()?;
        // Reject an unknown dependency-kind discriminant at load time (consistent
        // with the other structural validation) rather than silently round-tripping
        // a value that BFS would never admit.
        crate::builder::kind_from_u16(k)?;
        edge_kind.push(k);
    }
    let edge_scope = read_u32_vec(&mut r, m)?
        .into_iter()
        .map(StringId::new)
        .collect();
    let edge_flags = read_u32_vec(&mut r, m)?;

    if !r.is_empty() {
        return Err(PrismError::Corrupt {
            detail: format!("pcsr has {} trailing bytes", r.remaining()),
        });
    }

    GraphSnapshot::from_parts(SnapshotParts {
        run_id,
        node_version_id,
        node_depth,
        node_flags,
        node_attr,
        out_offsets,
        out_targets,
        out_edge_ids,
        in_offsets,
        in_sources,
        in_edge_ids,
        edge_ordinal,
        edge_requirement_id,
        edge_kind,
        edge_scope,
        edge_flags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{BuildEdge, BuildNode, GraphBuilder};

    fn nid(i: u32) -> GraphNodeId {
        GraphNodeId::new(i)
    }

    fn sample() -> GraphSnapshot {
        let mut b = GraphBuilder::new();
        for i in 0..4 {
            let mut nd = BuildNode::new(VersionId::new(1000 + i), i as u32);
            if i == 2 {
                nd.attr = Some(StringId::new(77));
            }
            b.add_node(nd).unwrap();
        }
        b.add_edge(BuildEdge::new(nid(0), nid(1), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(0), nid(2), 1)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(3), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(2), nid(3), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(3), nid(0), 0)).unwrap(); // back edge / cycle
        b.finalize(RunId::new(99)).unwrap()
    }

    #[test]
    fn roundtrip_is_exact() {
        let g = sample();
        let bytes = write_pcsr(&g);
        let g2 = read_pcsr(&bytes).unwrap();

        assert_eq!(g2.run_id(), g.run_id());
        assert_eq!(g2.node_count(), g.node_count());
        assert_eq!(g2.edge_count(), g.edge_count());
        assert_eq!(g2.node_versions(), g.node_versions());

        for i in 0..g.node_count() as u32 {
            let v = nid(i);
            assert_eq!(g2.out_neighbors(v).unwrap(), g.out_neighbors(v).unwrap());
            assert_eq!(g2.in_neighbors(v).unwrap(), g.in_neighbors(v).unwrap());
            assert_eq!(g2.node_depth(v).unwrap(), g.node_depth(v).unwrap());
            assert_eq!(g2.node_attr(v).unwrap(), g.node_attr(v).unwrap());
        }
        for e in 0..g.edge_count() as u32 {
            let id = GraphEdgeId::new(e);
            assert_eq!(g2.edge_ordinal(id).unwrap(), g.edge_ordinal(id).unwrap());
            assert_eq!(g2.edge_kind(id).unwrap(), g.edge_kind(id).unwrap());
            assert_eq!(g2.edge_flags(id).unwrap(), g.edge_flags(id).unwrap());
        }

        // A second serialization must be byte-identical.
        assert_eq!(write_pcsr(&g2), bytes);
    }

    #[test]
    fn empty_graph_roundtrip() {
        let g = GraphBuilder::new().finalize(RunId::new(1)).unwrap();
        let bytes = write_pcsr(&g);
        let g2 = read_pcsr(&bytes).unwrap();
        assert_eq!(g2.node_count(), 0);
        assert_eq!(g2.edge_count(), 0);
    }

    #[test]
    fn corruption_is_detected() {
        let g = sample();
        let mut bytes = write_pcsr(&g);
        // Flip a byte in the body (not the footer).
        let idx = bytes.len() / 2;
        bytes[idx] ^= 0xFF;
        match read_pcsr(&bytes) {
            Err(PrismError::ChecksumMismatch { .. }) => {}
            other => panic!("expected ChecksumMismatch, got {other:?}"),
        }
    }

    #[test]
    fn truncated_footer_is_corrupt() {
        let g = sample();
        let bytes = write_pcsr(&g);
        assert!(matches!(
            read_pcsr(&bytes[..2]),
            Err(PrismError::Corrupt { .. })
        ));
    }

    #[test]
    fn bad_magic_rejected() {
        let g = sample();
        let mut bytes = write_pcsr(&g);
        bytes[0] = b'X';
        // Recompute CRC so we get past the checksum to the magic check.
        let body_len = bytes.len() - 4;
        let crc = crc32(&bytes[..body_len]);
        bytes[body_len..].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(read_pcsr(&bytes), Err(PrismError::Corrupt { .. })));
    }

    #[test]
    fn newer_version_rejected() {
        let g = sample();
        let mut bytes = write_pcsr(&g);
        // format_version is the 4 bytes after the 8-byte magic.
        bytes[8..12].copy_from_slice(&(PCSR_FORMAT_VERSION + 1).to_le_bytes());
        let body_len = bytes.len() - 4;
        let crc = crc32(&bytes[..body_len]);
        bytes[body_len..].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(
            read_pcsr(&bytes),
            Err(PrismError::UnsupportedFormatVersion { .. })
        ));
    }
}
