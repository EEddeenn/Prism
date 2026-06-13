//! uv project detection: a `uv.lock` in the directory root.
//!
//! A bare `pyproject.toml` is ambiguous (poetry, pdm, hatch, flit all use it), so
//! detection keys on uv's own lockfile. `pyproject.toml` is still listed as the
//! manifest name for callers that walk a uv project.

use std::path::Path;

use prism_plugin_api::ProjectDetector;

/// Manifest filenames a uv project reads.
const MANIFESTS: &[&str] = &["pyproject.toml"];
/// Lockfile filenames uv writes.
const LOCKFILES: &[&str] = &["uv.lock"];

/// Detects a uv project by the presence of `uv.lock`.
pub struct UvDetector;

impl ProjectDetector for UvDetector {
    fn detects(&self, root: &Path) -> bool {
        root.join("uv.lock").is_file()
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
    fn lists_lockfile_and_manifest() {
        let d = UvDetector;
        assert_eq!(d.lockfile_filenames(), &["uv.lock"]);
        assert_eq!(d.manifest_filenames(), &["pyproject.toml"]);
    }

    #[test]
    fn detects_requires_uv_lock() {
        let d = UvDetector;
        let tmp = std::env::temp_dir().join("prism-uv-detect-nonexistent-xyz");
        assert!(!d.detects(&tmp));
    }
}
