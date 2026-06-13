//! Binary encoding primitives for Prism's on-disk formats.
//!
//! All multi-byte integers are encoded **little-endian** with natural framing so
//! that, when memory-mapping is later enabled, fixed-width arrays can be sliced
//! zero-copy (architecture doc, *Storage Architecture*). This crate is pure and
//! deterministic; it performs no I/O of its own beyond reading/writing byte buffers.

pub mod bytes;
pub mod checksum;
pub mod section;
pub mod varint;

pub use bytes::{ByteReader, ByteWriter};
pub use checksum::crc32;
pub use section::{RecordHeader, RECORD_MAGIC};
pub use varint::{read_uvarint, uvarint_len, write_uvarint};
