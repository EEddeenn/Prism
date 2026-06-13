//! Small, no-dependency test helpers shared by unit and integration tests:
//! a self-cleaning temp directory and a global lock serializing the
//! cwd-sensitive handler tests.
//!
//! Compiled for tests only (and for the integration test target, which links the
//! library). Mirrors the `prism-api` `TempDir` pattern.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temporary directory removed on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Create a uniquely-named temp directory tagged with `label`.
    pub fn new(label: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("prism-cli-test-{label}-{pid}-{n}"));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    /// The directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Acquire the process-wide lock that serializes cwd-mutating tests. Many CLI
/// handlers key off the current directory; changing it concurrently across
/// threads would corrupt unrelated tests, so they take this guard first.
pub fn cwd_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}
