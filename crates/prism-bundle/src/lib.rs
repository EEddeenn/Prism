//! `.prismbundle` v1: a single-file, byte-verbatim package of a Prism workspace's
//! published file set (MANIFEST + segments + indexes + graphs + TDA filtrations).
//!
//! ## Why verbatim?
//!
//! Every `.prism/` file is already self-framed (magic + version + CRC + digested
//! by the MANIFEST). Embedding each file byte-for-byte means every existing
//! defensive check is re-used on the load side and there is no codec drift. The
//! bundle adds one extra layer on top: a flat table of contents with per-entry
//! SHA-256 + a whole-bundle SHA-256 in the trailer.
//!
//! ## Top-level layout
//!
//! ```text
//! +-------------------+
//! | Header (32 bytes) |
//! +-------------------+
//! | Payload region    |  concatenated, byte-identical .prism/ files
//! +-------------------+
//! | TOC               |  flat table sorted by file class then path
//! +-------------------+
//! | Trailer (44 bytes)|  whole-bundle sha256 over [0 .. trailer_start)
//! +-------------------+
//! ```
//!
//! See [`format`] for the exact byte-level layout, magic constants, and version
//! pins. The reader path enforces every bound *before* allocating, matching the
//! hermetic-untrusted-input discipline of the rest of the codebase.
//!
//! v1 deliberately omits compression, content-addressed blobs, multi-analysis
//! containers, streaming, and signing.

#![forbid(unsafe_code)]

pub mod error;
pub mod format;
pub mod read;
pub mod toc;
pub mod write;

pub use format::{
    EntryKind, BUNDLE_FORMAT_VERSION, BUNDLE_MAGIC, MAX_BUNDLE_SIZE, MAX_PATH_LEN,
    MAX_TOC_ENTRIES, TRAILER_MAGIC, TRAILER_VERSION,
};
pub use read::{decode_toc_offsets, read_summary, unpack_into, BundleEntryInfo, BundleSummary};
pub use write::{pack_workspace, pack_workspace_to};
