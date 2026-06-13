//! Durable filesystem helpers implementing the write protocol's fsync discipline
//! (architecture doc, *Crash Safety*).
//!
//! On POSIX a rename's durability requires fsync-ing the *parent directory*, not
//! just the file — so every publish path here pairs a file fsync with a directory
//! fsync.

use prism_model::Result;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

/// fsync a file's contents and metadata.
pub fn fsync_file(path: &Path) -> Result<()> {
    let f = File::open(path)?;
    f.sync_all()?;
    Ok(())
}

/// fsync a directory so that prior renames/creations within it are durable.
pub fn fsync_dir(path: &Path) -> Result<()> {
    // Opening a directory read-only and calling sync_all is the POSIX idiom.
    let f = File::open(path)?;
    f.sync_all()?;
    Ok(())
}

/// Write `bytes` to `path` durably and atomically: stage in a sibling `.tmp`
/// file, fsync it, rename into place, then fsync the parent directory.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| prism_model::PrismError::Other("path has no parent".into()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| prism_model::PrismError::Other("path has no file name".into()))?
        .to_string_lossy();
    let tmp = parent.join(format!(".{file_name}.tmp"));

    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }

    fs::rename(&tmp, path)?;
    fsync_dir(parent)?;
    Ok(())
}

/// Read a whole file into memory.
pub fn read_file(path: &Path) -> Result<Vec<u8>> {
    Ok(fs::read(path)?)
}

/// Ensure `dir` exists (creating parents), and fsync it.
pub fn ensure_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    fsync_dir(dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn atomic_write_then_read() {
        let td = TempDir::new("fsutil");
        let p = td.path().join("file.bin");
        atomic_write(&p, b"hello world").unwrap();
        assert_eq!(read_file(&p).unwrap(), b"hello world");
        // No leftover staging file.
        let staged = td.path().join(".file.bin.tmp");
        assert!(!staged.exists());
    }

    #[test]
    fn atomic_write_overwrites() {
        let td = TempDir::new("fsutil2");
        let p = td.path().join("f");
        atomic_write(&p, b"v1").unwrap();
        atomic_write(&p, b"v2-longer").unwrap();
        assert_eq!(read_file(&p).unwrap(), b"v2-longer");
    }
}
