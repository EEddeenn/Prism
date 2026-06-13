//! Offline candidate enumeration over an in-memory version index.
//!
//! v0.1 is network-free, so there is no live crates.io index to enumerate. This
//! provider answers from a locally-populated map (e.g. seeded from a `Cargo.lock`
//! or a vendored mirror), returning versions sorted ascending by the Cargo
//! [`crate::version::CargoVersions`] scheme. An unknown crate yields an empty
//! list, never an error.

use std::collections::HashMap;

use prism_plugin_api::{CandidateProvider, ParseError, VersionScheme};

use crate::names::CargoNames;
use crate::version::CargoVersions;
use prism_plugin_api::NameNormalizer;

/// An offline candidate index keyed by canonical crate name.
#[derive(Default)]
pub struct CargoCandidates {
    /// `canonical_name -> ascending sorted raw versions`.
    index: HashMap<String, Vec<String>>,
}

impl CargoCandidates {
    /// An empty index (the default offline state).
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert the known versions of a crate, canonicalizing the name and sorting
    /// the versions ascending by the Cargo version scheme. Unparseable versions
    /// are dropped. Returns `Err` only if the crate name is invalid.
    pub fn insert(
        &mut self,
        raw_name: &str,
        versions: impl IntoIterator<Item = String>,
    ) -> Result<(), ParseError> {
        let canonical = CargoNames.normalize(raw_name)?;
        let scheme = CargoVersions;
        let mut vs: Vec<String> = versions
            .into_iter()
            .filter(|v| scheme.parse(v).is_ok())
            .collect();
        vs.sort_by(|a, b| {
            scheme
                .compare(a, b)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        self.index.insert(canonical, vs);
        Ok(())
    }
}

impl CandidateProvider for CargoCandidates {
    fn versions_for(&self, canonical_name: &str) -> Result<Vec<String>, ParseError> {
        Ok(self.index.get(canonical_name).cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_index_returns_no_versions() {
        let c = CargoCandidates::new();
        assert!(c.versions_for("serde").unwrap().is_empty());
    }

    #[test]
    fn returns_sorted_versions_by_canonical_name() {
        let mut c = CargoCandidates::new();
        c.insert(
            "Serde-JSON",
            ["1.0.10".into(), "1.0.2".into(), "1.0.100".into()],
        )
        .unwrap();
        // Looked up by the canonical (lowercase, `-`->`_`) key.
        let got = c.versions_for("serde_json").unwrap();
        assert_eq!(got, vec!["1.0.2", "1.0.10", "1.0.100"]);
    }

    #[test]
    fn drops_unparseable_versions() {
        let mut c = CargoCandidates::new();
        c.insert("x", ["1.0.0".into(), "garbage".into()]).unwrap();
        assert_eq!(c.versions_for("x").unwrap(), vec!["1.0.0"]);
    }

    #[test]
    fn invalid_name_is_an_error() {
        let mut c = CargoCandidates::new();
        assert!(c.insert("1bad", ["1.0.0".into()]).is_err());
    }
}
