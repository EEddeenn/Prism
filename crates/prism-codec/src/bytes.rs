//! Bounds-checked little-endian readers and writers over byte buffers.
//!
//! [`ByteReader`] never panics on malformed input: every read validates length
//! against the remaining buffer and returns [`PrismError::OutOfBounds`] on
//! underflow. This is the first line of defense for decoding untrusted segments.

use prism_model::{PrismError, Result};

/// A forward-only, bounds-checked reader over a byte slice.
#[derive(Clone, Debug)]
pub struct ByteReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    /// Wrap a byte slice at position 0.
    #[inline]
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Current read position.
    #[inline]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Bytes remaining.
    #[inline]
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// True if the reader is exhausted.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(PrismError::OutOfBounds {
                detail: format!("need {n} bytes, have {}", self.remaining()),
            });
        }
        let out = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    /// Read `n` raw bytes.
    #[inline]
    pub fn read_bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        self.take(n)
    }

    /// Read a fixed-size byte array.
    pub fn read_array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    /// Read a single byte.
    #[inline]
    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// Read a little-endian `u16`.
    #[inline]
    pub fn read_u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.read_array()?))
    }

    /// Read a little-endian `u32`.
    #[inline]
    pub fn read_u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.read_array()?))
    }

    /// Read a little-endian `u64`.
    #[inline]
    pub fn read_u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.read_array()?))
    }

    /// Read a little-endian `i64`.
    #[inline]
    pub fn read_i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.read_array()?))
    }

    /// Read a little-endian `f64`.
    #[inline]
    pub fn read_f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.read_array()?))
    }
}

/// A growable little-endian writer over an owned `Vec<u8>`.
#[derive(Clone, Debug, Default)]
pub struct ByteWriter {
    buf: Vec<u8>,
}

impl ByteWriter {
    /// A new empty writer.
    #[inline]
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// A new writer with reserved capacity.
    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }

    /// Number of bytes written so far.
    #[inline]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// True if nothing has been written.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Borrow the written bytes.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    /// Consume the writer, returning the buffer.
    #[inline]
    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }

    /// Append raw bytes.
    #[inline]
    pub fn write_bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }

    /// Append a single byte.
    #[inline]
    pub fn write_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    /// Append a little-endian `u16`.
    #[inline]
    pub fn write_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Append a little-endian `u32`.
    #[inline]
    pub fn write_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Append a little-endian `u64`.
    #[inline]
    pub fn write_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Append a little-endian `i64`.
    #[inline]
    pub fn write_i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Append a little-endian `f64`.
    #[inline]
    pub fn write_f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_widths() {
        let mut w = ByteWriter::new();
        w.write_u8(0xAB);
        w.write_u16(0x1234);
        w.write_u32(0xDEAD_BEEF);
        w.write_u64(0x0102_0304_0506_0708);
        w.write_i64(-42);
        w.write_f64(3.5);
        w.write_bytes(b"hi");

        let buf = w.into_vec();
        let mut r = ByteReader::new(&buf);
        assert_eq!(r.read_u8().unwrap(), 0xAB);
        assert_eq!(r.read_u16().unwrap(), 0x1234);
        assert_eq!(r.read_u32().unwrap(), 0xDEAD_BEEF);
        assert_eq!(r.read_u64().unwrap(), 0x0102_0304_0506_0708);
        assert_eq!(r.read_i64().unwrap(), -42);
        assert_eq!(r.read_f64().unwrap(), 3.5);
        assert_eq!(r.read_bytes(2).unwrap(), b"hi");
        assert!(r.is_empty());
    }

    #[test]
    fn underflow_is_an_error_not_a_panic() {
        let buf = [0u8; 3];
        let mut r = ByteReader::new(&buf);
        assert!(r.read_u32().is_err());
        // position is unchanged after a failed read
        assert_eq!(r.position(), 0);
        assert_eq!(r.remaining(), 3);
    }

    #[test]
    fn little_endian_byte_order() {
        let mut w = ByteWriter::new();
        w.write_u32(1);
        assert_eq!(w.as_slice(), &[1, 0, 0, 0]);
    }
}
