//! Cargo project detection: a `Cargo.toml` in the directory root.

use std::path::Path;

use prism_plugin_api::ProjectDetector;

/// Manifest filenames Cargo reads.
const MANIFESTS: &[&str] = &["Cargo.toml"];
/// Lockfile filenames Cargo reads.
const LOCKFILES: &[&str] = &["Cargo.lock"];

/// Detects a Cargo project by the presence of `Cargo.toml`.
pub struct CargoDetector;

impl ProjectDetector for CargoDetector {
    fn detects(&self, root: &Path) -> bool {
        root.join("Cargo.toml").is_file()
    }

    fn manifest_filenames(&self) -> &'static [&'static str] {
        MANIFESTS
    }

    fn lockfile_filenames(&self) -> &'static [&'static str] {
        LOCKFILES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertises_cargo_filenames() {
        let d = CargoDetector;
        assert_eq!(d.manifest_filenames(), &["Cargo.toml"]);
        assert_eq!(d.lockfile_filenames(), &["Cargo.lock"]);
    }

    #[test]
    fn does_not_detect_empty_dir() {
        let d = CargoDetector;
        // A path that does not contain a Cargo.toml file.
        assert!(!d.detects(Path::new("/nonexistent-prism-cargo-test-dir")));
    }
}
