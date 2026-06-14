//! prism-wasm: WebAssembly bridge for Prism.
//!
//! This crate is intentionally NON-hermetic: it depends on `wasm-bindgen`
//! and friends so the browser can decode a `.prismbundle` in memory and
//! call into the hermetic view layer (`prism-api`) without any backend.
//!
//! The public surface is three `#[wasm_bindgen]` exports:
//!
//! - [`start`] — installs a panic hook so Rust panics surface as
//!   `console.error` instead of an opaque WebAssembly trap.
//! - [`load_bundle`] — takes the raw bytes of a `.prismbundle`, decodes +
//!   verifies them in linear memory, replaces the singleton workspace,
//!   and returns a JSON summary identical to the one the old
//!   `POST /api/bundles` response produced (modulo the legacy 16-hex
//!   prefix `id`, which is replaced by the full 64-char SHA-256 hex).
//! - [`call_view`] — routes a string view name + a flat-JSON-object args
//!   bag through [`prism_api::dispatch::dispatch`] against the loaded
//!   singleton, returning the same JSON body the old HTTP endpoints did.
//!
//! Storage model: the bundle bytes enter linear memory exactly once and
//! are kept alive behind a single `Arc<Vec<u8>>` for as long as the
//! singleton workspace lives. Loading a new bundle drops the previous
//! workspace (and therefore the previous Arc), freeing the bytes.

use std::cell::RefCell;
use std::sync::Arc;

use wasm_bindgen::prelude::*;

use prism_api::{dispatch, list_run_ids, views_json, Args, Prism};

// The singleton workspace lives in thread-local storage. WebAssembly is
// single-threaded by default, so a single workspace per module instance is
// the right granularity — there is exactly one DOM `Window` per page and
// thus exactly one instance of the WASM module.
thread_local! {
    static WORKSPACE: RefCell<Option<Prism>> = const { RefCell::new(None) };
}

/// View names that depend on reading a server-local file path. These cannot
/// be honoured from the browser (there is no filesystem), so the WASM glue
/// rejects them at the boundary with a typed error before reaching the
/// dispatcher. The CLI still serves them via `prism analyze`.
///
/// This list is the WASM-side mirror of the CLI-only arms in
/// [`prism_api::dispatch`]; keep the two in sync when adding a view that
/// reads a path from the caller. Drop a view from `FILE_READING_VIEWS`
/// once its dispatcher arm stops touching the filesystem.
const FILE_READING_VIEWS: &[&str] = &["graph/overlay", "graph/budget"];

/// Reason a `call_view` invocation could not be dispatched at the WASM
/// boundary. Kept as a plain enum (no wasm-bindgen types) so the native
/// test suite can exercise the boundary logic without hitting
/// wasm-bindgen's "imported functions cannot be called on non-wasm
/// targets" panic.
#[derive(Debug, PartialEq, Eq)]
enum BoundaryError {
    FileReadingView(&'static str),
    NoWorkspace,
    BadArgsJson(String),
    Dispatch(String),
}

impl BoundaryError {
    fn message(&self) -> String {
        match self {
            BoundaryError::FileReadingView(name) => format!(
                "view '{name}' is not available in the browser (requires a server-local file path)"
            ),
            BoundaryError::NoWorkspace => {
                "no workspace loaded; call load_bundle first".to_string()
            }
            // Prefix the two pass-through error sources so the JS side can
            // tell "you sent malformed args" from "the view itself failed".
            BoundaryError::BadArgsJson(m) => format!("bad args: {m}"),
            BoundaryError::Dispatch(m) => format!("dispatch: {m}"),
        }
    }
}

/// Pure, wasm-bindgen-free implementation of `call_view`. The
/// `#[wasm_bindgen]` entry point above is a thin adapter that maps the
/// `BoundaryError` enum to a `JsError`; everything that can fail
/// independent of the ABI lives here so unit tests can exercise it on the
/// host target.
fn call_view_impl(name: &str, args_json: &str) -> Result<String, BoundaryError> {
    if let Some(canonical) = FILE_READING_VIEWS.iter().copied().find(|n| *n == name) {
        // `canonical` is the `&'static str` from the table — safe to embed
        // in BoundaryError (which carries a 'static reference).
        return Err(BoundaryError::FileReadingView(canonical));
    }
    let args = Args::from_flat_json(args_json)
        .map_err(|e| BoundaryError::BadArgsJson(e.to_string()))?;
    WORKSPACE.with(|w| match w.borrow().as_ref() {
        None => Err(BoundaryError::NoWorkspace),
        Some(prism) => dispatch::dispatch(prism, name, &args)
            .map_err(|e| BoundaryError::Dispatch(e.to_string())),
    })
}

/// Install a panic hook so Rust panics surface as `console.error`
/// instead of an opaque `RuntimeError: unreachable executed`.
///
/// Called once at module init by wasm-bindgen's start glue; idempotent.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Decode + verify a `.prismbundle` from raw bytes, replace the singleton
/// workspace, and return a JSON-encoded summary.
///
/// Returns a JSON string of the form:
///
/// ```json
/// {
///   "id": "<64-hex>",
///   "bundle_sha256": "<64-hex>",
///   "entry_count": <u32>,
///   "byte_len": <u64>,
///   "runs": [<u64>, ...]
/// }
/// ```
///
/// `id` and `bundle_sha256` are the same 64-character hex digest; the
/// legacy 16-hex prefix scheme used by the previous HTTP server was a
/// server-session artifact and is gone.
///
/// On failure throws a `JsError` carrying the [`PrismError`] display text
/// — the JS side sees a regular `Error` with a useful message. Possible
/// failures:
///
/// - corrupt bundle envelope / TOC / per-entry SHA mismatch
///   (surfaced from `prism_bundle::decode_toc_offsets` and
///   `prism_bundle::read_summary`);
/// - any [`PrismError`] from [`Prism::open_in_memory`] (manifest decode,
///   index parse, blob verify).
///
/// Side effect: replaces the current singleton workspace, dropping the
/// previous one (and freeing its `Arc<Vec<u8>>` of bundle bytes).
#[wasm_bindgen]
pub fn load_bundle(bytes: &[u8]) -> Result<String, JsError> {
    load_bundle_impl(bytes).map_err(|m| JsError::new(&m))
}

/// Pure, wasm-bindgen-free implementation of `load_bundle`. Returns the
/// already-formatted error message on failure so the WASM adapter is a
/// single `JsError::new` away from the JS surface.
fn load_bundle_impl(bytes: &[u8]) -> Result<String, String> {
    // One copy into linear memory: from the &[u8] slice (which lives in
    // the JS heap on entry) into an Arc<Vec<u8>> the workspace owns.
    let arc: Arc<Vec<u8>> = Arc::new(bytes.to_vec());

    // Open + verify the workspace. This runs the bundle's full envelope
    // + per-entry SHA-256 verification through `decode_toc_offsets`, then
    // load_published_from inside the store performs the published-file-set
    // integrity sweep.
    let prism = Prism::open_in_memory(arc.clone()).map_err(|e| e.to_string())?;

    // Run `read_summary` to recover the canonical `BundleSummary` (entry
    // count, byte_len, bundle SHA hex). This re-walks the bundle but is
    // dominated by hashing, which is cheap relative to the open above.
    let summary = prism_bundle::read_summary(&arc).map_err(|e| e.to_string())?;

    // Derive the run id list from the open workspace (same source the
    // dispatcher uses), so the response matches what the UI will see.
    let runs = list_run_ids(&prism);

    // Install the singleton workspace before producing the response so a
    // caller that pipelines `load_bundle` -> `call_view` sees the new
    // workspace immediately. Dropping the previous Option<Prism> drops the
    // previous Arc<Vec<u8>>; no manual deallocation is needed.
    WORKSPACE.with(|w| *w.borrow_mut() = Some(prism));

    Ok(views_json::render_bundle_summary(&summary, &runs))
}

/// Dispatch a view by route name with a flat-JSON-object args bag.
///
/// `name` is one of the keys recognized by [`prism_api::dispatch::dispatch`]:
///
/// ```text
/// status, runs, run,
/// graph/{stats,neighborhood,impact,scc,duplicates,scope,chokepoints,
///        cycles,why,dup-causes,risk,fragmentation,fidelity,subsystems,
///        diamonds,minimal,shared,distance,diff,shape,h2},
/// regions, tda, tda/pairs, tda/support, tda/compare,
/// package/candidates, export
/// ```
///
/// `args_json` is a flat JSON object (no nesting, no arrays) whose values
/// are either JSON numbers or JSON strings. Examples:
///
/// ```json
/// {}
/// {"run":0}
/// {"run":0,"node":42,"depth":3}
/// {"package":"pkg:cargo/serde","constraint":"^1"}
/// ```
///
/// The two file-reading views, `graph/overlay` and `graph/budget`, are
/// rejected at the boundary — they require a server-local file path that
/// has no analogue in the browser. The CLI still serves them.
///
/// Returns the same JSON string the corresponding HTTP endpoint used to
/// return. Errors surface as a `JsError` carrying the [`PrismError`]
/// display text.
#[wasm_bindgen]
pub fn call_view(name: &str, args_json: &str) -> Result<String, JsError> {
    call_view_impl(name, args_json).map_err(|e| JsError::new(&e.message()))
}

// ---------------------------------------------------------------------------
// Native-target unit tests.
//
// wasm-bindgen-test would require a browser runner and a fixture pipeline
// that is overkill for a single smoke test; instead we exercise the
// underlying logic with the same `prism_bundle::pack_workspace` +
// `Prism::open_in_memory` round-trip the WASM exports do, just without
// going through the wasm-bindgen ABI. The exports themselves are thin
// adapters whose error mapping is exercised here too via the public
// helpers (`Args::from_flat_json`, `dispatch::dispatch`,
// `views_json::render_bundle_summary`). When `cargo check --target
// wasm32-unknown-unknown` is run separately, the wasm-bindgen-generated
// glue compiles in the absence of a test harness, proving the cdylib
// builds.
// ---------------------------------------------------------------------------

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    /// We can't invoke wasm-bindgen-imported functions like `JsError::new`
    /// from a non-wasm target — they trap at runtime. Tests therefore drive
    /// the `*_impl` helpers directly; these carry all the boundary logic.
    /// The thin `#[wasm_bindgen]` adapters are exercised by the WASM
    /// build's link step (it must compile the ABI, which we run via
    /// `cargo check --target wasm32-unknown-unknown` and `wasm-pack
    /// build`).

    fn clear_workspace() {
        WORKSPACE.with(|w| *w.borrow_mut() = None);
    }

    #[test]
    fn file_reading_views_are_rejected_at_the_boundary() {
        // The rejection happens before the WORKSPACE borrow, so no fixture
        // is required.
        let err = call_view_impl("graph/overlay", "{}").err().expect("rejected");
        assert_eq!(err, BoundaryError::FileReadingView("graph/overlay"));
        assert!(err.message().contains("graph/overlay"), "{}", err.message());

        let err = call_view_impl("graph/budget", "{}").err().expect("rejected");
        assert_eq!(err, BoundaryError::FileReadingView("graph/budget"));
        assert!(err.message().contains("graph/budget"), "{}", err.message());
    }

    #[test]
    fn call_view_without_workspace_errors() {
        clear_workspace();
        let err = call_view_impl("status", "{}").err().expect("no workspace");
        assert_eq!(err, BoundaryError::NoWorkspace);
        assert!(err.message().contains("no workspace loaded"));
    }

    #[test]
    fn malformed_args_json_errors() {
        clear_workspace();
        let err = call_view_impl("graph/stats", "not-json")
            .err()
            .expect("bad json");
        assert!(matches!(err, BoundaryError::BadArgsJson(_)), "{err:?}");
    }

    #[test]
    fn load_bundle_rejects_garbage_bytes() {
        // A four-byte non-bundle blob must surface as a typed error string,
        // not a panic. (Underlying check lives in prism_bundle.)
        let err = load_bundle_impl(b"junk").err().expect("rejected");
        assert!(!err.is_empty(), "{err}");
    }

    #[test]
    fn load_bundle_then_call_view_round_trip() {
        // Build a tiny on-disk workspace, pack it to a .prismbundle byte
        // image, hand the bytes to the WASM entry point, and call a couple
        // of views through the dispatcher.
        let (tmp, bytes) = build_fixture_bundle();

        clear_workspace();
        let summary = load_bundle_impl(&bytes).expect("load_bundle");

        // The response JSON must include the expected top-level keys.
        for needle in &["\"id\"", "\"bundle_sha256\"", "\"entry_count\"", "\"runs\""] {
            assert!(summary.contains(needle), "missing {needle}: {summary}");
        }

        // After load_bundle, the singleton is installed and `status`
        // dispatches without args.
        let status = call_view_impl("status", "{}").expect("status");
        assert!(status.contains("\"name\":\"prism\""), "{status}");

        // `runs` dispatches and returns a JSON array.
        let runs = call_view_impl("runs", "{}").expect("runs");
        assert!(runs.starts_with('['), "{runs}");
        assert!(runs.ends_with(']'), "{runs}");

        // Tear down: drop the workspace (and its Arc<Vec<u8>>) so the
        // singleton does not leak into other tests in this process.
        clear_workspace();
        drop(tmp);
    }

    /// Build the smallest meaningful workspace and return its packed
    /// bundle bytes plus the owning TempDir (kept alive until the test
    /// has consumed the bytes so the workspace files outlive the pack).
    /// Uses the hand-rolled `TempDir` pattern from `prism-api::test_util`
    /// so we don't grow an external dev-dep.
    fn build_fixture_bundle() -> (HandRolledTempDir, Vec<u8>) {
        let dir = HandRolledTempDir::new("prism-wasm-bundle");
        // Ingest a 4-crate Cargo.lock so there is one run with a persisted
        // graph file (mirrors prism-api's end-to-end fixture).
        let lock = b"\
# @generated by Cargo
version = 3

[[package]]
name = \"app\"
version = \"0.1.0\"
dependencies = [
 \"serde 1.0.197\",
 \"rand 0.8.5\",
]

[[package]]
name = \"serde\"
version = \"1.0.197\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"
dependencies = [
 \"serde_derive 1.0.197\",
]

[[package]]
name = \"serde_derive\"
version = \"1.0.197\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"

[[package]]
name = \"rand\"
version = \"0.8.5\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"
";
        let lock_path = dir.path().join("Cargo.lock");
        std::fs::write(&lock_path, lock).expect("write lock");

        let mut prism = prism_api::Prism::init(dir.path()).expect("init");
        prism.ingest_lockfile(&lock_path).expect("ingest");
        // Drop the prism (and its write lock) before pack so packing sees
        // a quiesced workspace.
        drop(prism);

        let layout = prism_store::Layout::new(dir.path());
        let bytes = prism_bundle::pack_workspace(&layout).expect("pack");
        (dir, bytes)
    }

    /// Self-cleaning tempdir mirroring `prism-api::test_util::TempDir`.
    /// Kept inline here so prism-wasm gains no new external dep just to
    /// run a smoke test.
    struct HandRolledTempDir {
        path: std::path::PathBuf,
    }

    impl HandRolledTempDir {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let path = std::env::temp_dir().join(format!("prism-wasm-{label}-{pid}-{n}"));
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for HandRolledTempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
