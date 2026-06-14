//! pnpm project detection.
//!
//! pnpm reuses npm's `package.json` manifest, so a pnpm project is identified by
//! its lockfile: `pnpm-lock.yaml`.

use std::path::Path;

use prism_plugin_api::ProjectDetector;

/// Manifest filenames pnpm reads (the same as npm).
const MANIFESTS: &[&str] = &["package.json"];

/// Lockfile filenames pnpm writes.
const LOCKFILES: &[&str] = &["pnpm-lock.yaml"];

/// pnpm [`ProjectDetector`]: a directory is a pnpm project iff it has a `pnpm-lock.yaml`.
#[derive(Debug, Default, Clone, Copy)]
pub struct PnpmDetector;

impl ProjectDetector for PnpmDetector {
    fn detects(&self, root: &Path) -> bool {
        root.join("pnpm-lock.yaml").is_file()
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
        let d = PnpmDetector;
        assert_eq!(d.lockfile_filenames(), &["pnpm-lock.yaml"]);
        assert_eq!(d.manifest_filenames(), &["package.json"]);
    }

    #[test]
    fn detects_requires_pnpm_lock() {
        let d = PnpmDetector;
        let tmp = std::env::temp_dir().join("prism-pnpm-detect-nonexistent-xyz");
        assert!(!d.detects(&tmp));
    }
}
