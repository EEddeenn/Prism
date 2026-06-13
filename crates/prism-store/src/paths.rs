//! Workspace path layout (architecture doc, *Workspace Layout*).

use prism_model::BlobId;
use std::path::{Path, PathBuf};

/// Resolves the on-disk paths of a Prism workspace rooted at some directory.
#[derive(Clone, Debug)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    /// A layout rooted at `root` (the directory that contains `.prism/`).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The `.prism/` directory.
    pub fn prism_dir(&self) -> PathBuf {
        self.root.join(".prism")
    }

    /// The `.prism/MANIFEST` file.
    pub fn manifest(&self) -> PathBuf {
        self.prism_dir().join("MANIFEST")
    }

    /// The staging name an atomic manifest swap actually uses. `fsutil::atomic_write`
    /// stages every file as a dotted sibling `.{name}.tmp`, so the manifest stages
    /// as `.prism/.MANIFEST.tmp` — recovery sweeps this exact name.
    pub fn manifest_tmp(&self) -> PathBuf {
        self.prism_dir().join(".MANIFEST.tmp")
    }

    /// The `.prism/LOCK` file.
    pub fn lock(&self) -> PathBuf {
        self.prism_dir().join("LOCK")
    }

    /// The user-editable, non-secret config file.
    pub fn config(&self) -> PathBuf {
        self.prism_dir().join("config.toml")
    }

    /// `.prism/segments/`.
    pub fn segments_dir(&self) -> PathBuf {
        self.prism_dir().join("segments")
    }

    /// `.prism/indexes/`.
    pub fn indexes_dir(&self) -> PathBuf {
        self.prism_dir().join("indexes")
    }

    /// `.prism/blobs/`.
    pub fn blobs_dir(&self) -> PathBuf {
        self.prism_dir().join("blobs")
    }

    /// `.prism/graphs/`.
    pub fn graphs_dir(&self) -> PathBuf {
        self.prism_dir().join("graphs")
    }

    /// `.prism/candidates/`.
    pub fn candidates_dir(&self) -> PathBuf {
        self.prism_dir().join("candidates")
    }

    /// `.prism/tda/`.
    pub fn tda_dir(&self) -> PathBuf {
        self.prism_dir().join("tda")
    }

    /// `.prism/tmp/`, the staging area for atomic publishes.
    pub fn tmp_dir(&self) -> PathBuf {
        self.prism_dir().join("tmp")
    }

    /// Every directory that must exist in an initialized workspace.
    pub fn all_dirs(&self) -> [PathBuf; 7] {
        [
            self.segments_dir(),
            self.indexes_dir(),
            self.blobs_dir(),
            self.graphs_dir(),
            self.candidates_dir(),
            self.tda_dir(),
            self.tmp_dir(),
        ]
    }

    /// The conventional file name for an object segment (`000001.pseg`).
    pub fn segment_file_name(id: u32) -> String {
        format!("{id:06}.pseg")
    }

    /// Absolute path of object segment `id`.
    pub fn segment_file(&self, id: u32) -> PathBuf {
        self.segments_dir().join(Self::segment_file_name(id))
    }

    /// The conventional file name for an index segment (`000001.pidx`).
    pub fn index_file_name(id: u32) -> String {
        format!("{id:06}.pidx")
    }

    /// Absolute path of index segment `id`.
    pub fn index_file(&self, id: u32) -> PathBuf {
        self.indexes_dir().join(Self::index_file_name(id))
    }

    /// The conventional file name for a run's graph snapshot (`run_00000001.pcsr`).
    pub fn graph_file_name(run: u64) -> String {
        format!("run_{run:08}.pcsr")
    }

    /// Absolute path of the graph snapshot for run `run`.
    pub fn graph_file(&self, run: u64) -> PathBuf {
        self.graphs_dir().join(Self::graph_file_name(run))
    }

    /// The conventional file name for a filtration's TDA result
    /// (`filtration_00000001.ptda`).
    pub fn tda_file_name(filtration: u64) -> String {
        format!("filtration_{filtration:08}.ptda")
    }

    /// Absolute path of the TDA result for filtration `filtration`.
    pub fn tda_file(&self, filtration: u64) -> PathBuf {
        self.tda_dir().join(Self::tda_file_name(filtration))
    }

    /// Absolute path of the content-addressed blob `id` (raw, uncompressed for v0.1).
    pub fn blob_path(&self, id: &BlobId) -> PathBuf {
        self.blobs_dir().join(id.shard_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_paths() {
        let l = Layout::new("/ws");
        assert_eq!(l.manifest(), Path::new("/ws/.prism/MANIFEST"));
        assert_eq!(l.segment_file(1), Path::new("/ws/.prism/segments/000001.pseg"));
        assert_eq!(Layout::segment_file_name(42), "000042.pseg");
        assert_eq!(l.all_dirs().len(), 7);
    }
}
