//! Append-only object segments (`.pseg`).
//!
//! Layout: a fixed [`SegmentHeader`], a run of records (framed by `prism-codec`),
//! then a [`SegmentFooter`]. The segment's authoritative valid extent is the byte
//! length recorded in the manifest; the footer is a secondary self-check.

use prism_codec::{read_record, write_record, ByteReader, ByteWriter, Record};
use prism_model::{PrismError, Result};

/// Segment file magic: `b"PRISMPSEG"`.
pub const SEGMENT_MAGIC: &[u8; 9] = b"PRISMPSEG";
/// Footer magic marking the end of the record run: `b"PSEGEND\0"`.
pub const FOOTER_MAGIC: &[u8; 8] = b"PSEGEND\0";
/// Current segment format version.
pub const SEGMENT_FORMAT_VERSION: u32 = 1;

/// The fixed header at the start of every `.pseg` file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentHeader {
    /// On-disk format version.
    pub format_version: u32,
    /// Globally-unique, monotonic, never-reused segment id.
    pub segment_id: u32,
    /// Creation time (unix seconds).
    pub created_at: i64,
    /// Reserved flags.
    pub flags: u32,
}

/// Builds a `.pseg` byte image in memory, then hands it to the store for durable,
/// atomic placement.
pub struct SegmentWriter {
    header: SegmentHeader,
    body: ByteWriter,
    record_count: u64,
}

impl SegmentWriter {
    /// Start a new segment with the given id and creation time.
    pub fn new(segment_id: u32, created_at: i64) -> Self {
        Self {
            header: SegmentHeader {
                format_version: SEGMENT_FORMAT_VERSION,
                segment_id,
                created_at,
                flags: 0,
            },
            body: ByteWriter::new(),
            record_count: 0,
        }
    }

    /// Append a record. The byte offset of this record within the finished
    /// segment is returned so the caller can build index `RecordRef`s. Errors if a
    /// key/value is too large for the `u32` length fields (see `write_record`).
    pub fn append(
        &mut self,
        record_type: u16,
        flags: u16,
        logical_ts: u64,
        key: &[u8],
        value: &[u8],
    ) -> Result<u64> {
        let offset = (Self::header_len() + self.body.len()) as u64;
        write_record(&mut self.body, record_type, flags, logical_ts, key, value)?;
        self.record_count += 1;
        Ok(offset)
    }

    /// Number of records appended so far.
    pub fn record_count(&self) -> u64 {
        self.record_count
    }

    const fn header_len() -> usize {
        SEGMENT_MAGIC.len() + 4 + 4 + 8 + 4
    }

    /// Finish the segment, returning its complete byte image.
    pub fn finish(self) -> Vec<u8> {
        let body = self.body.into_vec();
        let body_crc = prism_codec::crc32(&body);

        let mut out = ByteWriter::with_capacity(Self::header_len() + body.len() + 24);
        out.write_bytes(SEGMENT_MAGIC);
        out.write_u32(self.header.format_version);
        out.write_u32(self.header.segment_id);
        out.write_i64(self.header.created_at);
        out.write_u32(self.header.flags);
        out.write_bytes(&body);
        // Footer
        out.write_bytes(FOOTER_MAGIC);
        out.write_u64(self.record_count);
        out.write_u32(body_crc);
        out.into_vec()
    }
}

/// A parsed, in-memory view of a `.pseg` file.
pub struct SegmentReader {
    header: SegmentHeader,
    bytes: Vec<u8>,
    body_range: core::ops::Range<usize>,
    record_count: u64,
}

impl SegmentReader {
    /// Parse and validate a segment image (header magic, footer magic, body crc).
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        let mut r = ByteReader::new(&bytes);
        let magic = r.read_bytes(SEGMENT_MAGIC.len())?;
        if magic != SEGMENT_MAGIC {
            return Err(PrismError::Corrupt {
                detail: "bad segment magic".into(),
            });
        }
        let format_version = r.read_u32()?;
        if format_version != SEGMENT_FORMAT_VERSION {
            return Err(PrismError::UnsupportedFormatVersion {
                found: format_version,
                max: SEGMENT_FORMAT_VERSION,
            });
        }
        let segment_id = r.read_u32()?;
        let created_at = r.read_i64()?;
        let flags = r.read_u32()?;
        let body_start = r.position();

        // Footer is the last 8 + 8 + 4 bytes.
        const FOOTER_LEN: usize = 8 + 8 + 4;
        if bytes.len() < body_start + FOOTER_LEN {
            return Err(PrismError::Corrupt {
                detail: "segment shorter than header + footer".into(),
            });
        }
        let body_end = bytes.len() - FOOTER_LEN;
        let mut fr = ByteReader::new(&bytes[body_end..]);
        let fmagic = fr.read_bytes(FOOTER_MAGIC.len())?;
        if fmagic != FOOTER_MAGIC {
            return Err(PrismError::Corrupt {
                detail: "bad segment footer magic".into(),
            });
        }
        let record_count = fr.read_u64()?;
        let body_crc = fr.read_u32()?;
        if prism_codec::crc32(&bytes[body_start..body_end]) != body_crc {
            return Err(PrismError::ChecksumMismatch {
                detail: "segment body crc".into(),
            });
        }

        Ok(Self {
            header: SegmentHeader {
                format_version,
                segment_id,
                created_at,
                flags,
            },
            body_range: body_start..body_end,
            record_count,
            bytes,
        })
    }

    /// The segment header.
    pub fn header(&self) -> &SegmentHeader {
        &self.header
    }

    /// Declared record count (from the footer).
    pub fn record_count(&self) -> u64 {
        self.record_count
    }

    /// Iterate the records in append order.
    pub fn records(&self) -> RecordIter<'_> {
        RecordIter {
            reader: ByteReader::new(&self.bytes[self.body_range.clone()]),
            done: false,
        }
    }
}

/// Iterator over the records in a segment body.
pub struct RecordIter<'a> {
    reader: ByteReader<'a>,
    done: bool,
}

impl<'a> Iterator for RecordIter<'a> {
    type Item = Result<Record<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.reader.is_empty() {
            return None;
        }
        match read_record(&mut self.reader) {
            Ok(rec) => Some(Ok(rec)),
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_roundtrip() {
        let mut w = SegmentWriter::new(3, 1_700_000_000);
        let off0 = w.append(1, 0, 10, b"k0", b"v0").unwrap();
        let off1 = w.append(2, 0, 11, b"k1", b"value-one").unwrap();
        assert!(off1 > off0);
        assert_eq!(w.record_count(), 2);
        let bytes = w.finish();

        let seg = SegmentReader::parse(bytes).unwrap();
        assert_eq!(seg.header().segment_id, 3);
        assert_eq!(seg.record_count(), 2);
        let recs: Vec<_> = seg.records().map(|r| r.unwrap()).collect();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].key, b"k0");
        assert_eq!(recs[1].value, b"value-one");
        assert_eq!(recs[1].header.logical_ts, 11);
    }

    #[test]
    fn corrupt_body_detected() {
        let mut w = SegmentWriter::new(1, 0);
        w.append(1, 0, 1, b"k", b"v").unwrap();
        let mut bytes = w.finish();
        // Flip a byte inside the record body (header is 29 bytes; body starts there).
        bytes[40] ^= 0xFF;
        assert!(SegmentReader::parse(bytes).is_err());
    }

    #[test]
    fn empty_segment_is_valid() {
        let w = SegmentWriter::new(9, 0);
        let seg = SegmentReader::parse(w.finish()).unwrap();
        assert_eq!(seg.record_count(), 0);
        assert_eq!(seg.records().count(), 0);
    }
}
