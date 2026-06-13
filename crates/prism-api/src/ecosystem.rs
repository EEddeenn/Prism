//! Ecosystem identity and input-file detection (architecture doc, *Rust API
//! Design → Plugin registry*).
//!
//! The facade owns a tiny registry of the two offline plugins shipped in v0.1
//! (Cargo and npm). Each is assigned a fixed, store-stable [`EcosystemId`] so
//! interned package keys are comparable across sessions. Detection is purely
//! lexical: it keys off the input file's *name*, never its contents.

use std::path::Path;

use prism_model::EcosystemId;
use prism_plugin_cargo::CargoPlugin;
use prism_plugin_npm::NpmPlugin;

/// The fixed [`EcosystemId`] for Cargo within a Prism workspace.
pub const CARGO_ECOSYSTEM_ID: EcosystemId = EcosystemId::new(1);
/// The fixed [`EcosystemId`] for npm within a Prism workspace.
pub const NPM_ECOSYSTEM_ID: EcosystemId = EcosystemId::new(2);

/// One of the ecosystems the facade can ingest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ecosystem {
    /// Rust's Cargo (`Cargo.toml` / `Cargo.lock`).
    Cargo,
    /// npm (`package.json` / `package-lock.json`).
    Npm,
}

impl Ecosystem {
    /// The store-stable [`EcosystemId`] for this ecosystem.
    pub const fn id(self) -> EcosystemId {
        match self {
            Ecosystem::Cargo => CARGO_ECOSYSTEM_ID,
            Ecosystem::Npm => NPM_ECOSYSTEM_ID,
        }
    }

    /// A short stable label (used in resolver-run names and purls).
    pub const fn label(self) -> &'static str {
        match self {
            Ecosystem::Cargo => "cargo",
            Ecosystem::Npm => "npm",
        }
    }

    /// Detect the owning ecosystem from a path's *file name*.
    ///
    /// `Cargo.toml`/`Cargo.lock` → [`Ecosystem::Cargo`];
    /// `package.json`/`package-lock.json` → [`Ecosystem::Npm`]. Returns `None`
    /// for any other name (the caller maps this to a clear error).
    pub fn detect(path: &Path) -> Option<Ecosystem> {
        let name = path.file_name()?.to_str()?;
        match name {
            "Cargo.toml" | "Cargo.lock" => Some(Ecosystem::Cargo),
            "package.json" | "package-lock.json" | "npm-shrinkwrap.json" => Some(Ecosystem::Npm),
            _ => None,
        }
    }

    /// True if `path`'s file name is a lockfile for this ecosystem.
    pub fn is_lockfile(path: &Path) -> bool {
        matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some("Cargo.lock" | "package-lock.json" | "npm-shrinkwrap.json")
        )
    }

    /// True if `path`'s file name is a manifest for this ecosystem.
    pub fn is_manifest(path: &Path) -> bool {
        matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some("Cargo.toml" | "package.json")
        )
    }

    /// Construct the boxed Cargo plugin for this ecosystem id.
    pub(crate) fn cargo_plugin() -> CargoPlugin {
        CargoPlugin::new(CARGO_ECOSYSTEM_ID)
    }

    /// Construct the boxed npm plugin for this ecosystem id.
    pub(crate) fn npm_plugin() -> NpmPlugin {
        NpmPlugin::new(NPM_ECOSYSTEM_ID)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn detect_by_filename() {
        assert_eq!(Ecosystem::detect(&p("/x/Cargo.toml")), Some(Ecosystem::Cargo));
        assert_eq!(Ecosystem::detect(&p("/x/Cargo.lock")), Some(Ecosystem::Cargo));
        assert_eq!(Ecosystem::detect(&p("/x/package.json")), Some(Ecosystem::Npm));
        assert_eq!(
            Ecosystem::detect(&p("/x/package-lock.json")),
            Some(Ecosystem::Npm)
        );
        assert_eq!(Ecosystem::detect(&p("/x/README.md")), None);
        assert_eq!(Ecosystem::detect(&p("/x")), None);
    }

    #[test]
    fn lockfile_vs_manifest() {
        assert!(Ecosystem::is_lockfile(&p("Cargo.lock")));
        assert!(Ecosystem::is_lockfile(&p("package-lock.json")));
        assert!(!Ecosystem::is_lockfile(&p("Cargo.toml")));
        assert!(Ecosystem::is_manifest(&p("Cargo.toml")));
        assert!(Ecosystem::is_manifest(&p("package.json")));
        assert!(!Ecosystem::is_manifest(&p("Cargo.lock")));
    }

    #[test]
    fn ids_are_distinct_and_stable() {
        assert_ne!(Ecosystem::Cargo.id(), Ecosystem::Npm.id());
        assert_eq!(Ecosystem::Cargo.id(), CARGO_ECOSYSTEM_ID);
        assert_eq!(Ecosystem::Cargo.label(), "cargo");
        assert_eq!(Ecosystem::Npm.label(), "npm");
    }
}
