//! Object-segment record framing (architecture doc, *Object Segments*).
//!
//! A record is a fixed `RecordHeader` followed by `key_bytes` then `value_bytes`.
//! The header's `crc32` covers the concatenated key and value. Decoding is fully
//! bounds-checked: a corrupt length or crc is reported, never trusted.

use crate::bytes::{ByteReader, ByteWriter};
use crate::checksum::Crc32;
use prism_model::{PrismError, Result};

/// Per-record magic (`b"PREC"` as a little-endian `u32`), distinguishing a valid
/// record start from a torn append tail.
pub const RECORD_MAGIC: u32 = u32::from_le_bytes(*b"PREC");

/// The fixed-size header preceding every object-segment record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordHeader {
    /// Discriminates the record's logical type (see `prism-store` record types).
    pub record_type: u16,
    /// Record-level flags (e.g. tombstone).
    pub flags: u16,
    /// Key length in bytes.
    pub key_len: u32,
    /// Value length in bytes.
    pub value_len: u32,
    /// Monotonic logical timestamp; the "newest wins" total order.
    pub logical_ts: u64,
    /// CRC-32 of `key_bytes ++ value_bytes`.
    pub crc32: u32,
}

impl RecordHeader {
    /// Encoded size of the header in bytes.
    pub const ENCODED_LEN: usize = 4 /*magic*/ + 2 + 2 + 4 + 4 + 8 + 4;
}

/// Encode a complete record (header + key + value) into `w`.
pub fn write_record(
    w: &mut ByteWriter,
    record_type: u16,
    flags: u16,
    logical_ts: u64,
    key: &[u8],
    value: &[u8],
) {
    let mut crc = Crc32::new();
    crc.update(key);
    crc.update(value);

    w.write_u32(RECORD_MAGIC);
    w.write_u16(record_type);
    w.write_u16(flags);
    w.write_u32(key.len() as u32);
    w.write_u32(value.len() as u32);
    w.write_u64(logical_ts);
    w.write_u32(crc.finalize());
    w.write_bytes(key);
    w.write_bytes(value);
}

/// A decoded record borrowed from the underlying buffer.
#[derive(Clone, Debug)]
pub struct Record<'a> {
    /// The parsed header.
    pub header: RecordHeader,
    /// The record key.
    pub key: &'a [u8],
    /// The record value.
    pub value: &'a [u8],
}

/// Decode one record at the reader's current position, advancing past it.
///
/// Validates magic, bounds-checks both lengths against the remaining buffer
/// (so a torn tail cannot trigger a huge allocation), and verifies the crc.
pub fn read_record<'a>(r: &mut ByteReader<'a>) -> Result<Record<'a>> {
    let magic = r.read_u32()?;
    if magic != RECORD_MAGIC {
        return Err(PrismError::Corrupt {
            detail: format!("bad record magic {magic:#010x}"),
        });
    }
    let record_type = r.read_u16()?;
    let flags = r.read_u16()?;
    let key_len = r.read_u32()? as usize;
    let value_len = r.read_u32()? as usize;
    let logical_ts = r.read_u64()?;
    let crc = r.read_u32()?;

    // Bounds-check before reading the variable-length payload.
    if key_len + value_len > r.remaining() {
        return Err(PrismError::OutOfBounds {
            detail: format!(
                "record payload {} exceeds remaining {}",
                key_len + value_len,
                r.remaining()
            ),
        });
    }
    let key = r.read_bytes(key_len)?;
    let value = r.read_bytes(value_len)?;

    let mut acc = Crc32::new();
    acc.update(key);
    acc.update(value);
    if acc.finalize() != crc {
        return Err(PrismError::ChecksumMismatch {
            detail: "record crc32".into(),
        });
    }

    Ok(Record {
        header: RecordHeader {
            record_type,
            flags,
            key_len: key_len as u32,
            value_len: value_len as u32,
            logical_ts,
            crc32: crc,
        },
        key,
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_roundtrip() {
        let mut w = ByteWriter::new();
        write_record(&mut w, 7, 0, 99, b"key-bytes", b"value-bytes-here");
        let buf = w.into_vec();

        let mut r = ByteReader::new(&buf);
        let rec = read_record(&mut r).unwrap();
        assert_eq!(rec.header.record_type, 7);
        assert_eq!(rec.header.logical_ts, 99);
        assert_eq!(rec.key, b"key-bytes");
        assert_eq!(rec.value, b"value-bytes-here");
        assert!(r.is_empty());
    }

    #[test]
    fn multiple_records_in_sequence() {
        let mut w = ByteWriter::new();
        write_record(&mut w, 1, 0, 1, b"a", b"1");
        write_record(&mut w, 2, 0, 2, b"bb", b"22");
        let buf = w.into_vec();
        let mut r = ByteReader::new(&buf);
        assert_eq!(read_record(&mut r).unwrap().key, b"a");
        let second = read_record(&mut r).unwrap();
        assert_eq!(second.key, b"bb");
        assert_eq!(second.header.logical_ts, 2);
        assert!(r.is_empty());
    }

    #[test]
    fn corrupted_value_fails_crc() {
        let mut w = ByteWriter::new();
        write_record(&mut w, 1, 0, 1, b"k", b"value");
        let mut buf = w.into_vec();
        *buf.last_mut().unwrap() ^= 0xFF; // flip a byte in the value
        let mut r = ByteReader::new(&buf);
        assert!(matches!(
            read_record(&mut r),
            Err(PrismError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn bogus_length_does_not_panic() {
        let mut w = ByteWriter::new();
        write_record(&mut w, 1, 0, 1, b"k", b"v");
        let mut buf = w.into_vec();
        // Corrupt value_len (offset 12..16) to a huge number.
        buf[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut r = ByteReader::new(&buf);
        assert!(read_record(&mut r).is_err());
    }
}
