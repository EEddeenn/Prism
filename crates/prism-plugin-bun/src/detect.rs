//! Bun project detection.
//!
//! Bun reuses npm's `package.json` manifest, so a Bun project is identified by
//! its lockfile: the text `bun.lock`. (The legacy binary `bun.lockb` is not a
//! text format and is out of scope.)

use std::path::Path;

use prism_plugin_api::ProjectDetector;

/// Manifest filenames Bun reads (the same as npm).
const MANIFESTS: &[&str] = &["package.json"];

/// Lockfile filenames Bun reads. Only the text lockfile is supported.
const LOCKFILES: &[&str] = &["bun.lock"];

/// Bun [`ProjectDetector`]: a directory is a Bun project iff it has a `bun.lock`.
#[derive(Debug, Default, Clone, Copy)]
pub struct BunDetector;

impl ProjectDetector for BunDetector {
    fn detects(&self, root: &Path) -> bool {
        root.join("bun.lock").is_file()
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
    fn lists_lockfile_name() {
        let d = BunDetector;
        assert_eq!(d.lockfile_filenames(), &["bun.lock"]);
        assert_eq!(d.manifest_filenames(), &["package.json"]);
    }

    #[test]
    fn detects_requires_bun_lock() {
        let d = BunDetector;
        let tmp = std::env::temp_dir().join("prism-bun-detect-nonexistent-xyz");
        assert!(!d.detects(&tmp));
    }
}
