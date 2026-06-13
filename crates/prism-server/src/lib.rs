//! A hermetic, local-only web UI over a Prism workspace (architecture doc's
//! *local HTTP/IPC service* option).
//!
//! Zero external crates: a minimal HTTP/1.1 server in pure std serves a vanilla-
//! JS single-page graph explorer (baked in via `include_str!`) plus a read-only
//! JSON API over [`prism_api::Prism`]. The facade owns the workspace write lock,
//! so it lives behind a [`std::sync::Mutex`] and the server only ever reads from
//! it. Untrusted requests never panic: a malformed request is answered `400`,
//! and every [`prism_model::PrismError`] maps to a `{"error": ...}` JSON body.
//!
//! Wiring: [`server::Server::start`] binds `127.0.0.1:<port>` and dispatches each
//! request through [`router::handle`] to either a static asset or a JSON
//! endpoint.

pub mod args;
mod assets;
mod http;
mod json;
mod query;
mod router;
mod runs;
mod server;
mod views_json;

#[cfg(test)]
mod test_support;

pub use args::{Args, ArgsError};
pub use server::Server;

use prism_api::Prism;

/// Open the workspace at `workspace` and start serving it on
/// `127.0.0.1:<port>` (port 0 picks an ephemeral port). The returned [`Server`]
/// serves on background threads until dropped.
pub fn serve(workspace: &str, port: u16) -> prism_model::Result<Server> {
    let prism = Prism::open(workspace)?;
    let workers = default_worker_count();
    let server = Server::start(prism, port, workers).map_err(prism_model::PrismError::Io)?;
    Ok(server)
}

/// A small default thread-pool size derived from available parallelism, clamped
/// to a sane band (loopback-only, mostly I/O-light reads).
fn default_worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(2, 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_count_in_band() {
        let n = default_worker_count();
        assert!((2..=8).contains(&n));
    }
}
