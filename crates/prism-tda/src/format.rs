//! The `.ptda` TDA-result file format (architecture doc, *TDA Store*).
//!
//! One `.ptda` image holds a complete filtration result for a single run: the
//! filtration header, the filtered cell complex (in reduction order), the
//! persistence pairs, and the per-pair generator support. All arrays are
//! little-endian, matching `prism-codec`; a footer CRC-32 covers the whole image.
//!
//! Layout (all little-endian):
//!
//! ```text
//! magic               "PRISMTDA" (8 bytes)
//! format_version      u32
//! filtration_id       u64
//! run_id              u64
//! filtration_type     u16
//! direction           u8
//! max_dimension       u8
//! cell_count          u64   (C)
//! pair_count          u64   (P)
//! gen_count           u64   (G)
//!
//! cell_dim            [u8;  C]
//! cell_birth_value    [f64; C]
//! cell_vertex_start   [u64; C+1]
//! cell_vertices       [u32; total_vertices]   (GraphNodeId)
//! cell_boundary_start [u64; C+1]
//! cell_boundary_faces [u64; total_faces]      (CellId)
//! cell_boundary_signs [i8;  total_faces]      (Z/2: always 1)
//! cell_source_type    [u8;  C]                (ObjectType)
//! cell_source_id      [u64; C]                (logical id)
//!
//! pair_id             [u64; P]
//! pair_dim            [u8;  P]
//! birth_cell          [u64; P]
//! death_cell          [u64; P]  (u64::MAX = NULL / essential)
//! birth_value         [f64; P]
//! death_value         [f64; P]  (+inf = essential)
//!
//! gen_pair_id         [u64; G]
//! gen_object_type     [u8;  G]
//! gen_object_id       [u64; G]
//! gen_contribution    [i8;  G]
//!
//! footer_crc32        u32   (crc32 over every preceding byte)
//! ```
//!
//! The footer CRC covers the whole image; a roundtrip is byte-exact and any
//! single-byte corruption is detected on read as [`PrismError::ChecksumMismatch`].

use prism_codec::{crc32, ByteReader, ByteWriter};
use prism_model::{
    CellId, FiltrationId, GraphNodeId, PairId, PrismError, Result, RunId,
};

use crate::complex::{Cell, Complex, ObjectType};
use crate::filtration::{Direction, Filtration, FiltrationType};
use crate::persistence::{GeneratorEntry, PersistencePair, PersistenceResult, NULL_CELL};

/// File magic: `PRISMTDA`.
pub const PTDA_MAGIC: &[u8; 8] = b"PRISMTDA";
/// The format version this build reads and writes.
pub const PTDA_FORMAT_VERSION: u32 = 1;

/// An in-memory `.ptda` document: a filtration result bound to one run snapshot.
///
/// Build one with [`Ptda::new`] from a [`Filtration`], its [`Complex`], and the
/// [`PersistenceResult`]; serialize with [`write_ptda`] and parse with
/// [`read_ptda`].
#[derive(Clone, Debug)]
pub struct Ptda {
    filtration_id: FiltrationId,
    run_id: RunId,
    filtration_type: FiltrationType,
    direction: Direction,
    max_dimension: u8,
    cells: Vec<Cell>,
    node_count: usize,
    pairs: Vec<PersistencePair>,
    generators: Vec<GeneratorEntry>,
}

impl Ptda {
    /// Assemble a `.ptda` document from its parts.
    pub fn new(
        filtration_id: FiltrationId,
        run_id: RunId,
        filtration: &Filtration,
        complex: &Complex,
        persistence: PersistenceResult,
    ) -> Self {
        let max_dimension = complex.cells().iter().map(|c| c.dim).max().unwrap_or(0);
        Self {
            filtration_id,
            run_id,
            filtration_type: filtration.filtration_type(),
            direction: filtration.direction(),
            max_dimension,
            cells: complex.cells().to_vec(),
            node_count: complex.node_count(),
            pairs: persistence.pairs().to_vec(),
            generators: persistence.generators().to_vec(),
        }
    }

    /// The filtration id of this document.
    #[inline]
    pub fn filtration_id(&self) -> FiltrationId {
        self.filtration_id
    }

    /// The run this filtration is bound to.
    #[inline]
    pub fn run_id(&self) -> RunId {
        self.run_id
    }

    /// The filtration kind.
    #[inline]
    pub fn filtration_type(&self) -> FiltrationType {
        self.filtration_type
    }

    /// The filtration direction.
    #[inline]
    pub fn direction(&self) -> Direction {
        self.direction
    }

    /// The max cell dimension present (0 or 1 in v0.1).
    #[inline]
    pub fn max_dimension(&self) -> u8 {
        self.max_dimension
    }

    /// The cells, in reduction order.
    #[inline]
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// The persistence pairs.
    #[inline]
    pub fn pairs(&self) -> &[PersistencePair] {
        &self.pairs
    }

    /// The generator-support entries.
    #[inline]
    pub fn generators(&self) -> &[GeneratorEntry] {
        &self.generators
    }

    /// Recover the filtered [`Complex`] (re-validates monotonicity / cell order).
    pub fn complex(&self) -> Result<Complex> {
        Complex::from_cells(self.cells.clone(), self.node_count)
    }

    /// Recover the [`PersistenceResult`].
    pub fn persistence(&self) -> PersistenceResult {
        PersistenceResult::from_parts(self.pairs.clone(), self.generators.clone())
    }
}

/// Serialize a [`Ptda`] into a self-contained `.ptda` byte image.
pub fn write_ptda(doc: &Ptda) -> Vec<u8> {
    let c = doc.cells.len();
    let p = doc.pairs.len();
    let g = doc.generators.len();

    let total_vertices: usize = doc.cells.iter().map(|cell| cell.vertices.len()).sum();
    let total_faces: usize = doc.cells.iter().map(|cell| cell.faces.len()).sum();

    let mut w = ByteWriter::with_capacity(
        64 + c * 26 + total_vertices * 4 + total_faces * 9 + p * 41 + g * 18,
    );
    w.write_bytes(PTDA_MAGIC);
    w.write_u32(PTDA_FORMAT_VERSION);
    w.write_u64(doc.filtration_id.get());
    w.write_u64(doc.run_id.get());
    w.write_u16(doc.filtration_type.to_u16());
    w.write_u8(doc.direction.to_u8());
    w.write_u8(doc.max_dimension);
    w.write_u64(c as u64);
    w.write_u64(p as u64);
    w.write_u64(g as u64);

    // Cell arrays.
    for cell in &doc.cells {
        w.write_u8(cell.dim);
    }
    for cell in &doc.cells {
        w.write_f64(cell.birth_value);
    }
    // vertex CSR
    let mut acc = 0u64;
    w.write_u64(acc);
    for cell in &doc.cells {
        acc += cell.vertices.len() as u64;
        w.write_u64(acc);
    }
    for cell in &doc.cells {
        for v in &cell.vertices {
            w.write_u32(v.get());
        }
    }
    // boundary (face-incidence) CSR
    let mut bacc = 0u64;
    w.write_u64(bacc);
    for cell in &doc.cells {
        bacc += cell.faces.len() as u64;
        w.write_u64(bacc);
    }
    for cell in &doc.cells {
        for &face in &cell.faces {
            w.write_u64(face);
        }
    }
    for cell in &doc.cells {
        for _ in &cell.faces {
            w.write_u8(1i8 as u8); // Z/2 orientation sign is always +1
        }
    }
    for cell in &doc.cells {
        w.write_u8(cell.source_type.to_u8());
    }
    for cell in &doc.cells {
        w.write_u64(cell.source_id);
    }

    // Persistence arrays.
    for pair in &doc.pairs {
        w.write_u64(pair.pair_id.get());
    }
    for pair in &doc.pairs {
        w.write_u8(pair.dim);
    }
    for pair in &doc.pairs {
        w.write_u64(pair.birth_cell.get());
    }
    for pair in &doc.pairs {
        w.write_u64(pair.death_cell.map(|d| d.get()).unwrap_or(NULL_CELL));
    }
    for pair in &doc.pairs {
        w.write_f64(pair.birth_value);
    }
    for pair in &doc.pairs {
        w.write_f64(pair.death_value.unwrap_or(f64::INFINITY));
    }

    // Generator arrays.
    for gen in &doc.generators {
        w.write_u64(gen.pair_id.get());
    }
    for gen in &doc.generators {
        w.write_u8(gen.object_type.to_u8());
    }
    for gen in &doc.generators {
        w.write_u64(gen.object_id);
    }
    for gen in &doc.generators {
        w.write_u8(gen.contribution as u8);
    }

    let crc = crc32(w.as_slice());
    w.write_u32(crc);
    w.into_vec()
}

fn read_u64_vec(r: &mut ByteReader<'_>, n: usize) -> Result<Vec<u64>> {
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(r.read_u64()?);
    }
    Ok(v)
}

/// Parse a `.ptda` byte image into a validated [`Ptda`] document.
///
/// Errors:
/// - [`PrismError::Corrupt`] — wrong magic, truncated, a bad discriminant, or a
///   structural invariant violated (offsets, lengths, out-of-range cell).
/// - [`PrismError::UnsupportedFormatVersion`] — a newer on-disk format.
/// - [`PrismError::ChecksumMismatch`] — the whole-image CRC does not match.
pub fn read_ptda(bytes: &[u8]) -> Result<Ptda> {
    if bytes.len() < 4 {
        return Err(PrismError::Corrupt {
            detail: "ptda image too short for footer".into(),
        });
    }
    let (body, crc_bytes) = bytes.split_at(bytes.len() - 4);
    let stored_crc = u32::from_le_bytes([crc_bytes[0], crc_bytes[1], crc_bytes[2], crc_bytes[3]]);
    let actual_crc = crc32(body);
    if stored_crc != actual_crc {
        return Err(PrismError::ChecksumMismatch {
            detail: format!(".ptda crc mismatch (stored {stored_crc:#x}, actual {actual_crc:#x})"),
        });
    }

    let mut r = ByteReader::new(body);
    let magic = r.read_array::<8>()?;
    if &magic != PTDA_MAGIC {
        return Err(PrismError::Corrupt {
            detail: "bad .ptda magic".into(),
        });
    }
    let version = r.read_u32()?;
    if version > PTDA_FORMAT_VERSION {
        return Err(PrismError::UnsupportedFormatVersion {
            found: version,
            max: PTDA_FORMAT_VERSION,
        });
    }
    let filtration_id = FiltrationId::new(r.read_u64()?);
    let run_id = RunId::new(r.read_u64()?);
    let filtration_type = FiltrationType::from_u16(r.read_u16()?);
    let direction = Direction::from_u8(r.read_u8()?)?;
    let max_dimension = r.read_u8()?;
    let c = read_count(r.read_u64()?, "cell_count")?;
    let p = read_count(r.read_u64()?, "pair_count")?;
    let g = read_count(r.read_u64()?, "gen_count")?;

    // Cell arrays.
    let mut cell_dim = Vec::with_capacity(c);
    for _ in 0..c {
        cell_dim.push(r.read_u8()?);
    }
    let mut cell_birth = Vec::with_capacity(c);
    for _ in 0..c {
        cell_birth.push(r.read_f64()?);
    }
    let vertex_start = read_u64_vec(&mut r, c + 1)?;
    validate_csr(&vertex_start, "cell_vertex_start")?;
    let total_vertices = vertex_start[c] as usize;
    let mut cell_vertices = Vec::with_capacity(total_vertices);
    for _ in 0..total_vertices {
        cell_vertices.push(GraphNodeId::new(r.read_u32()?));
    }
    let boundary_start = read_u64_vec(&mut r, c + 1)?;
    validate_csr(&boundary_start, "cell_boundary_start")?;
    let total_faces = boundary_start[c] as usize;
    let mut boundary_faces = Vec::with_capacity(total_faces);
    for _ in 0..total_faces {
        boundary_faces.push(r.read_u64()?);
    }
    let mut boundary_signs = Vec::with_capacity(total_faces);
    for _ in 0..total_faces {
        boundary_signs.push(r.read_u8()? as i8);
    }
    let mut cell_source_type = Vec::with_capacity(c);
    for _ in 0..c {
        cell_source_type.push(ObjectType::from_u8(r.read_u8()?)?);
    }
    let mut cell_source_id = Vec::with_capacity(c);
    for _ in 0..c {
        cell_source_id.push(r.read_u64()?);
    }

    // Reassemble cells; recover node_count from the highest 0-cell node index + 1.
    let mut cells = Vec::with_capacity(c);
    let mut node_count = 0usize;
    for i in 0..c {
        let vs = vertex_start[i] as usize;
        let ve = vertex_start[i + 1] as usize;
        let bs = boundary_start[i] as usize;
        let be = boundary_start[i + 1] as usize;
        let vertices = cell_vertices[vs..ve].to_vec();
        let faces = boundary_faces[bs..be].to_vec();
        // Validate face indices are in range and signs are Z/2 (+1).
        for &face in &faces {
            if face >= c as u64 {
                return Err(PrismError::Corrupt {
                    detail: format!("cell {i} face {face} out of range"),
                });
            }
        }
        for &s in &boundary_signs[bs..be] {
            if s != 1 {
                return Err(PrismError::Corrupt {
                    detail: format!("cell {i} has non-Z/2 boundary sign {s}"),
                });
            }
        }
        if cell_dim[i] == 0 {
            if let Some(v0) = vertices.first() {
                node_count = node_count.max(v0.get() as usize + 1);
            }
        }
        cells.push(Cell {
            dim: cell_dim[i],
            birth_value: cell_birth[i],
            vertices,
            faces,
            source_type: cell_source_type[i],
            source_id: cell_source_id[i],
        });
    }

    // Persistence arrays.
    let pair_id = read_u64_vec(&mut r, p)?;
    let mut pair_dim = Vec::with_capacity(p);
    for _ in 0..p {
        pair_dim.push(r.read_u8()?);
    }
    let birth_cell = read_u64_vec(&mut r, p)?;
    let death_cell = read_u64_vec(&mut r, p)?;
    let mut birth_value = Vec::with_capacity(p);
    for _ in 0..p {
        birth_value.push(r.read_f64()?);
    }
    let mut death_value = Vec::with_capacity(p);
    for _ in 0..p {
        death_value.push(r.read_f64()?);
    }
    let mut pairs = Vec::with_capacity(p);
    for i in 0..p {
        let dc = death_cell[i];
        let dv = death_value[i];
        // Invariant: NULL death cell iff infinite death value.
        let is_essential = dc == NULL_CELL;
        if is_essential != dv.is_infinite() {
            return Err(PrismError::Corrupt {
                detail: format!(
                    "pair {i}: death_cell NULL ({is_essential}) disagrees with death_value inf"
                ),
            });
        }
        pairs.push(PersistencePair {
            pair_id: PairId::new(pair_id[i]),
            dim: pair_dim[i],
            birth_cell: CellId::new(birth_cell[i]),
            death_cell: if is_essential {
                None
            } else {
                Some(CellId::new(dc))
            },
            birth_value: birth_value[i],
            death_value: if is_essential { None } else { Some(dv) },
        });
    }

    // Generator arrays.
    let gen_pair_id = read_u64_vec(&mut r, g)?;
    let mut gen_object_type = Vec::with_capacity(g);
    for _ in 0..g {
        gen_object_type.push(ObjectType::from_u8(r.read_u8()?)?);
    }
    let gen_object_id = read_u64_vec(&mut r, g)?;
    let mut gen_contribution = Vec::with_capacity(g);
    for _ in 0..g {
        gen_contribution.push(r.read_u8()? as i8);
    }
    let mut generators = Vec::with_capacity(g);
    for i in 0..g {
        generators.push(GeneratorEntry {
            pair_id: PairId::new(gen_pair_id[i]),
            object_type: gen_object_type[i],
            object_id: gen_object_id[i],
            contribution: gen_contribution[i],
        });
    }

    if !r.is_empty() {
        return Err(PrismError::Corrupt {
            detail: format!("ptda has {} trailing bytes", r.remaining()),
        });
    }

    // Re-validate the cell complex (monotonicity / cell order) via the reader.
    let _ = Complex::from_cells(cells.clone(), node_count)?;

    Ok(Ptda {
        filtration_id,
        run_id,
        filtration_type,
        direction,
        max_dimension,
        cells,
        node_count,
        pairs,
        generators,
    })
}

/// Bound a length read from untrusted bytes against `u32::MAX` before allocating,
/// so a corrupt huge count cannot trigger an enormous allocation.
fn read_count(v: u64, which: &str) -> Result<usize> {
    if v > u32::MAX as u64 {
        return Err(PrismError::Corrupt {
            detail: format!("ptda {which} {v} exceeds limit"),
        });
    }
    Ok(v as usize)
}

/// Validate a CSR offset array: non-decreasing and starting at 0.
fn validate_csr(offsets: &[u64], which: &str) -> Result<()> {
    if offsets.first() != Some(&0) {
        return Err(PrismError::Corrupt {
            detail: format!("{which}[0] must be 0"),
        });
    }
    for w in offsets.windows(2) {
        if w[0] > w[1] {
            return Err(PrismError::Corrupt {
                detail: format!("{which} not non-decreasing"),
            });
        }
    }
    if offsets[offsets.len() - 1] > u32::MAX as u64 {
        return Err(PrismError::Corrupt {
            detail: format!("{which} total exceeds limit"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filtration::depth_filtration;
    use prism_graph::{BuildEdge, BuildNode, GraphBuilder, GraphSnapshot};
    use prism_model::{GraphNodeId, RunId, VersionId};

    fn nid(i: u32) -> GraphNodeId {
        GraphNodeId::new(i)
    }

    fn cycle_graph() -> GraphSnapshot {
        let mut b = GraphBuilder::new();
        for i in 0..4u64 {
            b.add_node(BuildNode::new(VersionId::new(10 + i), i as u32))
                .unwrap();
        }
        b.add_edge(BuildEdge::new(nid(0), nid(1), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(2), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(2), nid(3), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(3), nid(0), 0)).unwrap();
        b.finalize(RunId::new(5)).unwrap()
    }

    fn sample_doc() -> Ptda {
        let g = cycle_graph();
        let f = depth_filtration(&g).unwrap();
        let c = Complex::build(&g, f.values()).unwrap();
        let r = PersistenceResult::compute(&c, 1000).unwrap();
        Ptda::new(FiltrationId::new(3), g.run_id(), &f, &c, r)
    }

    #[test]
    fn roundtrip_is_exact() {
        let doc = sample_doc();
        let bytes = write_ptda(&doc);
        let back = read_ptda(&bytes).unwrap();
        assert_eq!(back.filtration_id(), doc.filtration_id());
        assert_eq!(back.run_id(), doc.run_id());
        assert_eq!(back.cells().len(), doc.cells().len());
        assert_eq!(back.pairs(), doc.pairs());
        assert_eq!(back.generators(), doc.generators());
        // Byte-identical second serialization.
        assert_eq!(write_ptda(&back), bytes);
    }

    #[test]
    fn corruption_is_detected() {
        let doc = sample_doc();
        let mut bytes = write_ptda(&doc);
        let idx = bytes.len() / 2;
        bytes[idx] ^= 0xFF;
        assert!(matches!(
            read_ptda(&bytes),
            Err(PrismError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn truncated_footer_is_corrupt() {
        let doc = sample_doc();
        let bytes = write_ptda(&doc);
        assert!(matches!(
            read_ptda(&bytes[..2]),
            Err(PrismError::Corrupt { .. })
        ));
    }

    #[test]
    fn bad_magic_rejected() {
        let doc = sample_doc();
        let mut bytes = write_ptda(&doc);
        bytes[0] = b'X';
        let body_len = bytes.len() - 4;
        let crc = crc32(&bytes[..body_len]);
        bytes[body_len..].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(read_ptda(&bytes), Err(PrismError::Corrupt { .. })));
    }

    #[test]
    fn newer_version_rejected() {
        let doc = sample_doc();
        let mut bytes = write_ptda(&doc);
        bytes[8..12].copy_from_slice(&(PTDA_FORMAT_VERSION + 1).to_le_bytes());
        let body_len = bytes.len() - 4;
        let crc = crc32(&bytes[..body_len]);
        bytes[body_len..].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(
            read_ptda(&bytes),
            Err(PrismError::UnsupportedFormatVersion { .. })
        ));
    }

    #[test]
    fn empty_graph_roundtrips() {
        let g = GraphBuilder::new().finalize(RunId::new(9)).unwrap();
        let f = depth_filtration(&g).unwrap();
        let c = Complex::build(&g, f.values()).unwrap();
        let r = PersistenceResult::compute(&c, 0).unwrap();
        let doc = Ptda::new(FiltrationId::new(1), g.run_id(), &f, &c, r);
        let bytes = write_ptda(&doc);
        let back = read_ptda(&bytes).unwrap();
        assert_eq!(back.cells().len(), 0);
        assert_eq!(back.pairs().len(), 0);
    }

    #[test]
    fn recovered_complex_and_persistence_match() {
        let doc = sample_doc();
        let bytes = write_ptda(&doc);
        let back = read_ptda(&bytes).unwrap();
        let c = back.complex().unwrap();
        assert_eq!(c.cell_count(), back.cells().len());
        let r = back.persistence();
        assert_eq!(r.essential_count(1), 1);
    }
}
