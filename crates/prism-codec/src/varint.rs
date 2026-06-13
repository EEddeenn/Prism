//! LEB128 unsigned variable-length integers.
//!
//! Used for compact, length-delimited encoding of small counts and IDs inside
//! records. Decoding is bounds- and overflow-checked.

use crate::bytes::{ByteReader, ByteWriter};
use prism_model::{PrismError, Result};

/// Append `v` as an unsigned LEB128 varint.
pub fn write_uvarint(w: &mut ByteWriter, mut v: u64) {
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        w.write_u8(byte);
        if v == 0 {
            break;
        }
    }
}

/// Read an unsigned LEB128 varint.
///
/// Returns [`PrismError::OutOfBounds`] on truncation, and
/// [`PrismError::Corrupt`] if the encoding would overflow `u64` (more than 10
/// bytes, or a 10th byte with bits above bit 63 set) or is **overlong** — a
/// non-minimal multi-byte encoding whose terminating group is `0x00`. Rejecting
/// overlong forms keeps the decoder injective, so a re-encoded segment cannot map
/// a value to bytes that fail to round-trip (canonical byte form is the basis of
/// content addressing).
pub fn read_uvarint(r: &mut ByteReader<'_>) -> Result<u64> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        let byte = r.read_u8()?;
        if shift >= 64 {
            return Err(PrismError::Corrupt {
                detail: "uvarint overflows u64".into(),
            });
        }
        let payload = (byte & 0x7f) as u64;
        // Guard the final group so we never shift bits off the top of u64.
        if shift == 63 && payload > 1 {
            return Err(PrismError::Corrupt {
                detail: "uvarint overflows u64".into(),
            });
        }
        result |= payload << shift;
        if byte & 0x80 == 0 {
            // A terminating `0x00` continuation group past the first byte is a
            // non-minimal (overlong) encoding.
            if shift > 0 && byte == 0 {
                return Err(PrismError::Corrupt {
                    detail: "overlong (non-canonical) uvarint encoding".into(),
                });
            }
            return Ok(result);
        }
        shift += 7;
    }
}

/// The number of bytes `write_uvarint` would emit for `v`.
pub fn uvarint_len(v: u64) -> usize {
    let mut n = 1;
    let mut v = v >> 7;
    while v != 0 {
        n += 1;
        v >>= 7;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(v: u64) {
        let mut w = ByteWriter::new();
        write_uvarint(&mut w, v);
        assert_eq!(w.len(), uvarint_len(v), "len mismatch for {v}");
        let buf = w.into_vec();
        let mut r = ByteReader::new(&buf);
        assert_eq!(read_uvarint(&mut r).unwrap(), v, "roundtrip for {v}");
        assert!(r.is_empty());
    }

    #[test]
    fn roundtrip_boundaries() {
        for v in [0u64, 1, 127, 128, 255, 300, 16_383, 16_384, u32::MAX as u64, u64::MAX] {
            roundtrip(v);
        }
    }

    #[test]
    fn known_encodings() {
        let mut w = ByteWriter::new();
        write_uvarint(&mut w, 300);
        assert_eq!(w.as_slice(), &[0xAC, 0x02]);
    }

    #[test]
    fn overlong_encoding_is_rejected() {
        // [0x80, 0x00] and [0x81, 0x00] decode to 0 / 1 but are non-minimal; the
        // minimal forms are [0x00] and [0x01]. Decoding must reject the overlong
        // forms so the byte representation stays canonical.
        for bytes in [&[0x80u8, 0x00][..], &[0x81, 0x00][..], &[0xAC, 0x80, 0x00][..]] {
            let mut r = ByteReader::new(bytes);
            assert!(
                matches!(read_uvarint(&mut r), Err(PrismError::Corrupt { .. })),
                "overlong {bytes:?} must be rejected"
            );
        }
        // The minimal single-byte zero is still fine.
        let mut r = ByteReader::new(&[0x00]);
        assert_eq!(read_uvarint(&mut r).unwrap(), 0);
    }

    #[test]
    fn truncated_is_error() {
        let buf = [0x80u8]; // continuation bit set, no follow-up byte
        let mut r = ByteReader::new(&buf);
        assert!(read_uvarint(&mut r).is_err());
    }

    #[test]
    fn overlong_overflow_is_error() {
        // 11 bytes all with continuation bit set -> overflow
        let buf = [0x80u8; 11];
        let mut r = ByteReader::new(&buf);
        assert!(read_uvarint(&mut r).is_err());
    }
}
