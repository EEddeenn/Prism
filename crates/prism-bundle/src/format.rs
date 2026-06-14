//! Byte-level constants and the bundle-local [`EntryKind`] enum.
//!
//! These constants are the single source of truth for the on-disk
//! [`.prismbundle`](crate) format. They live in their own module so the writer
//! and the reader cannot drift.
//!
//! `EntryKind` is deliberately bundle-local: it does *not* extend
//! [`prism_store::manifest::FileKind`]. The bundle adds one variant
//! ([`EntryKind::Manifest`], discriminant `5`) so the MANIFEST can appear in the
//! TOC like every other file, but the two enums are kept independent so the
//! store's on-disk numbering can evolve without breaking bundle compatibility.

use prism_model::{PrismError, Result};

/// Bundle file magic: `b"PRISMBND"`. Matches at offset 0.
pub const BUNDLE_MAGIC: &[u8; 8] = b"PRISMBND";

/// Trailer magic: `b"PRISMEND"`. Matches at offset `file_len - 44`.
pub const TRAILER_MAGIC: &[u8; 8] = b"PRISMEND";

/// Current bundle format version written by this binary.
///
/// Readers reject `0` and any value greater than this via
/// [`PrismError::UnsupportedFormatVersion`] so future-compat is explicit.
pub const BUNDLE_FORMAT_VERSION: u32 = 1;

/// Current trailer version. Bumped independently of the body version so the
/// integrity envelope can evolve without rewriting payload layout.
pub const TRAILER_VERSION: u32 = 1;

/// Maximum accepted bundle byte length (2 GiB).
///
/// Defense-in-depth cap so a malicious header cannot trick the reader into
/// allocating arbitrarily large buffers. Real bundles are kilobytes to a few
/// megabytes; the cap is set well above any plausible single-workspace size.
pub const MAX_BUNDLE_SIZE: u64 = 2 * 1024 * 1024 * 1024;

/// Maximum number of entries in the TOC.
///
/// A workspace has at most a few thousand files (segments + indexes + graphs +
/// TDA filtrations + MANIFEST), so 65 536 is plenty of headroom and keeps the
/// TOC-allocation guard cheap.
pub const MAX_TOC_ENTRIES: u32 = 65_536;

/// Maximum length, in bytes, of a TOC entry's relative path (UTF-8). 512 fits
/// every path Prism currently writes with margin.
pub const MAX_PATH_LEN: u16 = 512;

/// The category of a file referenced by a bundle TOC entry.
///
/// Discriminants are part of the on-disk format and must not change. v1
/// deliberately reserves `4` (`Candidate` in the store) and rejects it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum EntryKind {
    /// A `segments/NNNNNN.pseg` fact batch.
    Segment = 0,
    /// An `indexes/NNNNNN.pidx` Merkle index.
    Index = 1,
    /// A `graphs/run_NNNNNNNN.pcsr` resolved graph.
    Graph = 2,
    /// A `tda/filtration_NNNNNNNN.ptda` persistence filtration.
    Tda = 3,
    /// The workspace `MANIFEST`. Bundle-local kind with discriminant `5` so the
    /// store's `Candidate = 4` slot stays reserved.
    Manifest = 5,
}

impl EntryKind {
    /// Parse an [`EntryKind`] from its on-disk discriminant.
    ///
    /// Rejects `4` (`Candidate`, reserved) and any value greater than `5` with a
    /// [`PrismError::Corrupt`]. Never panics.
    pub fn from_u8(v: u8) -> Result<EntryKind> {
        match v {
            0 => Ok(EntryKind::Segment),
            1 => Ok(EntryKind::Index),
            2 => Ok(EntryKind::Graph),
            3 => Ok(EntryKind::Tda),
            5 => Ok(EntryKind::Manifest),
            4 => Err(PrismError::Corrupt {
                detail: "bundle entry kind 4 (Candidate) is reserved and not accepted in v1"
                    .to_string(),
            }),
            other => Err(PrismError::Corrupt {
                detail: format!("unknown bundle entry kind {other}"),
            }),
        }
    }

    /// The on-disk discriminant. Inverse of [`Self::from_u8`] for accepted
    /// variants.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_constants_are_exact_bytes() {
        assert_eq!(BUNDLE_MAGIC, b"PRISMBND");
        assert_eq!(TRAILER_MAGIC, b"PRISMEND");
        assert_eq!(BUNDLE_MAGIC.len(), 8);
        assert_eq!(TRAILER_MAGIC.len(), 8);
    }

    #[test]
    fn versions_are_pinned() {
        assert_eq!(BUNDLE_FORMAT_VERSION, 1);
        assert_eq!(TRAILER_VERSION, 1);
    }

    #[test]
    fn limits_are_documented_values() {
        assert_eq!(MAX_BUNDLE_SIZE, 2 * 1024 * 1024 * 1024);
        assert_eq!(MAX_TOC_ENTRIES, 65_536);
        assert_eq!(MAX_PATH_LEN, 512);
    }

    #[test]
    fn entry_kind_roundtrip_accepted_variants() {
        for k in [
            EntryKind::Segment,
            EntryKind::Index,
            EntryKind::Graph,
            EntryKind::Tda,
            EntryKind::Manifest,
        ] {
            let raw = k.as_u8();
            let parsed = EntryKind::from_u8(raw).expect("accepted variant");
            assert_eq!(parsed, k);
        }
    }

    #[test]
    fn entry_kind_rejects_reserved_four() {
        let err = EntryKind::from_u8(4).expect_err("v1 rejects 4");
        match err {
            PrismError::Corrupt { detail } => {
                assert!(detail.contains("Candidate"), "detail: {detail}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn entry_kind_rejects_unknown() {
        for v in [6u8, 7, 42, 255] {
            let err = EntryKind::from_u8(v).expect_err("unknown kind");
            assert!(matches!(err, PrismError::Corrupt { .. }));
        }
    }

    #[test]
    fn entry_kind_discriminants_are_stable() {
        // These numbers are part of the on-disk format; do not change them.
        assert_eq!(EntryKind::Segment.as_u8(), 0);
        assert_eq!(EntryKind::Index.as_u8(), 1);
        assert_eq!(EntryKind::Graph.as_u8(), 2);
        assert_eq!(EntryKind::Tda.as_u8(), 3);
        assert_eq!(EntryKind::Manifest.as_u8(), 5);
    }
}
