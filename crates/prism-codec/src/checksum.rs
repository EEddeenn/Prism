//! CRC-32 (IEEE 802.3) for per-record integrity checks.
//!
//! This is corruption detection only — not tamper resistance. Whole-file
//! authenticity uses the SHA-256 digests recorded in the manifest.

/// Precomputed CRC-32/IEEE lookup table (reflected, polynomial 0xEDB88320).
const TABLE: [u32; 256] = build_table();

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 == 1 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// Compute the CRC-32/IEEE checksum of `data`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        let idx = ((crc ^ b as u32) & 0xFF) as usize;
        crc = (crc >> 8) ^ TABLE[idx];
    }
    crc ^ 0xFFFF_FFFF
}

/// Incremental CRC-32 accumulator for streaming over multiple chunks.
#[derive(Clone, Copy, Debug)]
pub struct Crc32 {
    state: u32,
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    /// A fresh accumulator.
    #[inline]
    pub fn new() -> Self {
        Self {
            state: 0xFFFF_FFFF,
        }
    }

    /// Feed a chunk.
    pub fn update(&mut self, data: &[u8]) {
        let mut crc = self.state;
        for &b in data {
            let idx = ((crc ^ b as u32) & 0xFF) as usize;
            crc = (crc >> 8) ^ TABLE[idx];
        }
        self.state = crc;
    }

    /// Finalize and return the checksum.
    #[inline]
    pub fn finalize(self) -> u32 {
        self.state ^ 0xFFFF_FFFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        // The canonical CRC-32/IEEE check value for "123456789".
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn streaming_matches_oneshot() {
        let data = b"the quick brown fox jumps over the lazy dog";
        let mut acc = Crc32::new();
        acc.update(&data[..10]);
        acc.update(&data[10..]);
        assert_eq!(acc.finalize(), crc32(data));
    }
}
