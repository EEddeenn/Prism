//! Canonical domain model for Prism.
//!
//! This crate defines the typed identifiers, bitflags, and core entity structs
//! shared across every other crate. It is intentionally pure data: no I/O, no
//! external dependencies, no ecosystem-specific logic. See the architecture
//! design document, sections *Canonical Domain Model* and *Identity and ID Model*.
//!
//! ## Identity model
//!
//! Every identifier is an explicit newtype with a normative width and stability
//! scope (see [`ids`]). Store-global integer IDs are allocated from monotonic
//! counters whose high-water marks are recovered from the durably published
//! manifest. [`GraphNodeId`]/[`GraphEdgeId`] are dense, snapshot-local indices and
//! are meaningless outside their run; [`GraphNodeId::NULL`] is the reserved sentinel.

#[macro_use]
mod macros;

pub mod entities;
pub mod error;
pub mod flags;
pub mod ids;

pub use entities::*;
pub use error::{PrismError, Result};
pub use flags::{EcosystemCapabilities, EdgeFlags, FidelityCaveats, NodeFlags, VersionFlags};
pub use ids::{
    BlobId, CellId, EcosystemId, EnvironmentId, FiltrationId, GraphEdgeId, GraphNodeId, HashDigest,
    PackageId, PairId, RecordRef, RequirementId, RunId, SnapshotId, StringId, VersionId,
};
