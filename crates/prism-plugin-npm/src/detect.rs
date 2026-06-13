//! npm project detection (architecture doc, *npm Plugin*).

use std::path::Path;

use prism_plugin_api::ProjectDetector;

/// Manifest filenames npm reads, in preference order.
const MANIFESTS: &[&str] = &["package.json"];

/// Lockfile filenames npm reads, in preference order. A shrinkwrap pins the same
/// shape as a `package-lock.json` and takes precedence when published.
const LOCKFILES: &[&str] = &["npm-shrinkwrap.json", "package-lock.json"];

/// npm [`ProjectDetector`]: a directory is an npm project iff it has a
/// `package.json`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NpmDetector;

impl ProjectDetector for NpmDetector {
    fn detects(&self, root: &Path) -> bool {
        root.join("package.json").is_file()
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
    fn lists_manifest_and_lockfile_names() {
        let d = NpmDetector;
        assert_eq!(d.manifest_filenames(), &["package.json"]);
        assert!(d.lockfile_filenames().contains(&"package-lock.json"));
        assert!(d.lockfile_filenames().contains(&"npm-shrinkwrap.json"));
    }

    #[test]
    fn detects_requires_package_json() {
        let d = NpmDetector;
        // A path that certainly does not contain package.json.
        let tmp = std::env::temp_dir().join("prism-npm-detect-nonexistent-xyz");
        assert!(!d.detects(&tmp));
    }
}
