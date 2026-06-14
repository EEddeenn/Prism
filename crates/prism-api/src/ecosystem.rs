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
use prism_plugin_uv::UvPlugin;

/// The fixed [`EcosystemId`] for Cargo within a Prism workspace.
pub const CARGO_ECOSYSTEM_ID: EcosystemId = EcosystemId::new(1);
/// The fixed [`EcosystemId`] for npm within a Prism workspace.
pub const NPM_ECOSYSTEM_ID: EcosystemId = EcosystemId::new(2);
/// The fixed [`EcosystemId`] for PyPI (uv) within a Prism workspace.
pub const PYPI_ECOSYSTEM_ID: EcosystemId = EcosystemId::new(3);

/// One of the ecosystems the facade can ingest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ecosystem {
    /// Rust's Cargo (`Cargo.toml` / `Cargo.lock`).
    Cargo,
    /// npm (`package.json` / `package-lock.json`).
    Npm,
    /// Bun (`bun.lock`). Bun installs npm packages, so it shares npm's store
    /// identity, purls, and semver semantics; only the lockfile format differs.
    Bun,
    /// pnpm (`pnpm-lock.yaml`). Like Bun, pnpm installs npm packages and shares
    /// npm's store identity, purls, and semver semantics; only the lockfile
    /// format (and its already-flat resolved graph) differ.
    Pnpm,
    /// uv / PyPI (`uv.lock`). Python packages with PEP 440/503/508 semantics; its
    /// own ecosystem id and `pkg:pypi/...` purls.
    Uv,
}

/// Map a stored [`EcosystemId`] back to an [`Ecosystem`]. Because Bun and pnpm
/// share [`NPM_ECOSYSTEM_ID`], the npm id resolves to [`Ecosystem::Npm`] (npm,
/// bun, and pnpm graphs are indistinguishable once ingested, and analyze
/// identically).
pub(crate) fn ecosystem_of(id: EcosystemId) -> Ecosystem {
    if id == CARGO_ECOSYSTEM_ID {
        Ecosystem::Cargo
    } else if id == PYPI_ECOSYSTEM_ID {
        Ecosystem::Uv
    } else {
        Ecosystem::Npm
    }
}

impl Ecosystem {
    /// The store-stable [`EcosystemId`] for this ecosystem.
    ///
    /// Bun deliberately shares [`NPM_ECOSYSTEM_ID`]: a `bun.lock` and a
    /// `package-lock.json` of the same project resolve the same npm packages, so
    /// their interned identities and `pkg:npm/...` purls must coincide.
    pub const fn id(self) -> EcosystemId {
        match self {
            Ecosystem::Cargo => CARGO_ECOSYSTEM_ID,
            Ecosystem::Npm | Ecosystem::Bun | Ecosystem::Pnpm => NPM_ECOSYSTEM_ID,
            Ecosystem::Uv => PYPI_ECOSYSTEM_ID,
        }
    }

    /// A short stable label (used in resolver-run names and ingest messages).
    pub const fn label(self) -> &'static str {
        match self {
            Ecosystem::Cargo => "cargo",
            Ecosystem::Npm => "npm",
            Ecosystem::Bun => "bun",
            Ecosystem::Pnpm => "pnpm",
            Ecosystem::Uv => "uv",
        }
    }

    /// The purl scheme for this ecosystem's `pkg:<scheme>/...` keys.
    pub const fn purl_scheme(self) -> &'static str {
        match self {
            Ecosystem::Cargo => "cargo",
            // Bun and pnpm install npm packages.
            Ecosystem::Npm | Ecosystem::Bun | Ecosystem::Pnpm => "npm",
            Ecosystem::Uv => "pypi",
        }
    }

    /// Detect the owning ecosystem from a path's *file name*.
    ///
    /// `Cargo.toml`/`Cargo.lock` → [`Ecosystem::Cargo`];
    /// `package.json`/`package-lock.json` → [`Ecosystem::Npm`]; `bun.lock` →
    /// [`Ecosystem::Bun`]; `uv.lock` → [`Ecosystem::Uv`]. Returns `None` for any
    /// other name (the caller maps this to a clear error).
    pub fn detect(path: &Path) -> Option<Ecosystem> {
        let name = path.file_name()?.to_str()?;
        match name {
            "Cargo.toml" | "Cargo.lock" => Some(Ecosystem::Cargo),
            "package.json" | "package-lock.json" | "npm-shrinkwrap.json" => Some(Ecosystem::Npm),
            "bun.lock" => Some(Ecosystem::Bun),
            "pnpm-lock.yaml" => Some(Ecosystem::Pnpm),
            "uv.lock" => Some(Ecosystem::Uv),
            _ => None,
        }
    }

    /// True if `path`'s file name is a lockfile for this ecosystem.
    pub fn is_lockfile(path: &Path) -> bool {
        matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some(
                "Cargo.lock"
                    | "package-lock.json"
                    | "npm-shrinkwrap.json"
                    | "bun.lock"
                    | "pnpm-lock.yaml"
                    | "uv.lock"
            )
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
    ///
    /// Bun reuses this plugin's npm semantics (name/version/requirement); a Bun
    /// `bun.lock` is reshaped to an npm document in [`crate::parse`], so no
    /// separate Bun plugin instance is needed here.
    pub(crate) fn npm_plugin() -> NpmPlugin {
        NpmPlugin::new(NPM_ECOSYSTEM_ID)
    }

    /// Construct the uv / PyPI plugin for this ecosystem id.
    pub(crate) fn uv_plugin() -> UvPlugin {
        UvPlugin::new(PYPI_ECOSYSTEM_ID)
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
        assert_eq!(Ecosystem::detect(&p("/x/bun.lock")), Some(Ecosystem::Bun));
        assert_eq!(Ecosystem::detect(&p("/x/uv.lock")), Some(Ecosystem::Uv));
        assert_eq!(Ecosystem::detect(&p("/x/README.md")), None);
        assert_eq!(Ecosystem::detect(&p("/x")), None);
    }

    #[test]
    fn uv_is_its_own_pypi_ecosystem() {
        assert_ne!(Ecosystem::Uv.id(), Ecosystem::Npm.id());
        assert_ne!(Ecosystem::Uv.id(), Ecosystem::Cargo.id());
        assert_eq!(Ecosystem::Uv.label(), "uv");
        assert_eq!(Ecosystem::Uv.purl_scheme(), "pypi");
        assert!(Ecosystem::is_lockfile(&p("uv.lock")));
        assert!(!Ecosystem::is_manifest(&p("uv.lock")));
    }

    #[test]
    fn bun_shares_npm_identity_but_keeps_its_label() {
        // A bun.lock and a package-lock.json must intern under the same id (so
        // their pkg:npm purls coincide) while still labelling as "bun".
        assert_eq!(Ecosystem::Bun.id(), Ecosystem::Npm.id());
        assert_eq!(Ecosystem::Bun.label(), "bun");
        assert!(Ecosystem::is_lockfile(&p("bun.lock")));
        assert!(!Ecosystem::is_manifest(&p("bun.lock")));
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
