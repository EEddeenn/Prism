//! Hermetic, hand-rolled JSON and (subset) TOML parsers for ingesting
//! manifests and lockfiles entirely offline.
//!
//! Prism never shells out to a package manager to read a `package.json`,
//! `package-lock.json`, `Cargo.toml`, or `Cargo.lock`; it parses them here with
//! zero external dependencies. Both parsers:
//!
//! - accept their full (JSON) / subset (TOML) grammar needed for ingestion;
//! - preserve object/table insertion order and integer-valued numbers;
//! - are bounded by a maximum input size and a recursion-depth cap, returning
//!   [`prism_model::PrismError::Corrupt`] on adversarial nesting rather than
//!   overflowing the stack;
//! - never panic on untrusted input.
//!
//! See the architecture doc's *Manifest Requirement Graph Mode* and
//! *Native Resolution Mode* sections.

pub mod json;
pub mod toml;

pub use json::{parse_json, Json, MAX_JSON_BYTES, MAX_JSON_DEPTH};
pub use toml::{parse_toml, Toml, MAX_TOML_BYTES, MAX_TOML_DEPTH};
