//! The single-writer workspace lock.
//!
//! The architecture specifies an OS advisory lock (`flock`) that auto-releases on
//! process death. That requires `libc`/`unsafe`, which this crate forbids, so the
//! v0.1 implementation is a presence lock: an exclusively-created `LOCK` file
//! holding the owner's PID, removed on drop. A crashed writer leaves a stale lock
//! that [`WorkspaceLock::break_lock`] can clear. Replacing this with a true
//! advisory lock is tracked for when a platform crate is allowed.

use prism_model::{PrismError, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// An RAII guard over the workspace `LOCK` file. Dropping it releases the lock.
pub struct WorkspaceLock {
    path: PathBuf,
}

impl WorkspaceLock {
    /// Acquire the lock by exclusively creating `path`. Fails if it already exists.
    pub fn acquire(path: &Path) -> Result<Self> {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut f) => {
                let _ = write!(f, "{}", std::process::id());
                let _ = f.sync_all();
                Ok(Self {
                    path: path.to_path_buf(),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(PrismError::Other(
                format!("workspace is locked ({}); another writer holds it", path.display()),
            )),
            Err(e) => Err(PrismError::Io(e)),
        }
    }

    /// Forcibly remove a stale lock file left by a crashed writer.
    pub fn break_lock(path: &Path) -> Result<()> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PrismError::Io(e)),
        }
    }
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn exclusive_then_released_on_drop() {
        let td = TempDir::new("lock");
        let lp = td.path().join("LOCK");
        let g = WorkspaceLock::acquire(&lp).unwrap();
        assert!(WorkspaceLock::acquire(&lp).is_err()); // held
        drop(g);
        let _g2 = WorkspaceLock::acquire(&lp).unwrap(); // released
    }

    #[test]
    fn break_lock_clears_stale() {
        let td = TempDir::new("lock-break");
        let lp = td.path().join("LOCK");
        std::mem::forget(WorkspaceLock::acquire(&lp).unwrap()); // leak: simulate crash
        assert!(WorkspaceLock::acquire(&lp).is_err());
        WorkspaceLock::break_lock(&lp).unwrap();
        let _g = WorkspaceLock::acquire(&lp).unwrap();
    }
}
