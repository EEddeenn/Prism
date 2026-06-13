//! The TCP server: bind to loopback, accept connections, and answer each with a
//! single response (HTTP/1.1 `Connection: close`).
//!
//! A small fixed thread pool drains a shared queue of accepted streams; the
//! listener thread feeds it. Everything is loopback-only and read-only over the
//! facade. A malformed request never crashes a worker — it is answered `400` and
//! the connection is closed.

use std::io::{self, Read};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use prism_api::Prism;

use crate::http::{Request, Response, Status};
use crate::json::error_body;
use crate::router::{self, AppState};

/// Read/write timeout per connection, so a slow or stuck client cannot pin a
/// worker forever. This bounds a single read; [`HEAD_DEADLINE`] bounds the whole
/// head read so a client dripping one byte per timeout window cannot extend it.
const CONN_TIMEOUT: Duration = Duration::from_secs(15);

/// Aggregate wall-clock budget for reading a request head, regardless of per-read
/// progress (defeats slowloris-style byte-dripping).
const HEAD_DEADLINE: Duration = Duration::from_secs(10);

/// Bound on the accept→worker hand-off queue. A flood beyond this drops at accept
/// rather than buffering `TcpStream`s without limit.
const ACCEPT_QUEUE_CAP: usize = 256;

/// A reader that fails once an absolute `deadline` passes, so the *total* time
/// spent reading is bounded even if each individual read makes a little progress
/// (the per-read socket timeout alone resets on every byte).
struct DeadlineReader<R> {
    inner: R,
    deadline: Instant,
}

impl<R: Read> Read for DeadlineReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if Instant::now() >= self.deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "request head read deadline exceeded",
            ));
        }
        self.inner.read(buf)
    }
}

/// A running server. Dropping it (or calling [`Server::shutdown`]) stops the
/// accept loop and joins the workers.
pub struct Server {
    addr: SocketAddr,
    /// Set false to ask the accept loop to stop; it then unblocks via a
    /// self-connect.
    running: Arc<Mutex<bool>>,
    accept_handle: Option<JoinHandle<()>>,
}

impl Server {
    /// Bind to `127.0.0.1:port` (port 0 picks an ephemeral port) and start
    /// serving `prism` with `workers` worker threads.
    ///
    /// Returns once the listener is bound; the bound address is available via
    /// [`Server::addr`]. Serving happens on background threads.
    pub fn start(prism: Prism, port: u16, workers: usize) -> io::Result<Server> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
        let addr = listener.local_addr()?;
        let state = Arc::new(AppState::new(prism));
        let workers = workers.max(1);

        // Worker pool: a shared, mutex-guarded receiver of accepted streams. The
        // queue is BOUNDED (sync_channel) so a connection flood blocks/drops at the
        // accept loop instead of buffering TcpStreams without limit.
        let (tx, rx): (SyncSender<TcpStream>, Receiver<TcpStream>) =
            mpsc::sync_channel(ACCEPT_QUEUE_CAP);
        let rx = Arc::new(Mutex::new(rx));
        let mut worker_handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let rx = Arc::clone(&rx);
            let state = Arc::clone(&state);
            worker_handles.push(thread::spawn(move || worker_loop(&rx, &state)));
        }

        let running = Arc::new(Mutex::new(true));
        let accept_running = Arc::clone(&running);
        let accept_handle = thread::spawn(move || {
            accept_loop(listener, &tx, &accept_running);
            // Dropping `tx` closes the channel; workers see `Err` and exit.
            drop(tx);
            for h in worker_handles {
                let _ = h.join();
            }
        });

        Ok(Server {
            addr,
            running,
            accept_handle: Some(accept_handle),
        })
    }

    /// The bound local address (host + chosen port).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The URL clients should open.
    pub fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }

    /// Stop accepting, drain workers, and join the background threads.
    pub fn shutdown(&mut self) {
        if let Ok(mut flag) = self.running.lock() {
            if !*flag {
                return; // already shutting down
            }
            *flag = false;
        }
        // Unblock the blocking `accept()` with a throwaway self-connection.
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.accept_handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Accept connections until `running` is cleared, handing each stream to the
/// worker pool.
fn accept_loop(listener: TcpListener, tx: &SyncSender<TcpStream>, running: &Arc<Mutex<bool>>) {
    for stream in listener.incoming() {
        if !*running.lock().unwrap_or_else(|e| e.into_inner()) {
            break;
        }
        match stream {
            Ok(s) => {
                // Hand off without blocking the accept loop. If the bounded queue
                // is full (a flood faster than the pool drains), DROP this
                // connection rather than stall accepting; a disconnected receiver
                // means the server is shutting down.
                match tx.try_send(s) {
                    Ok(()) => {}
                    Err(mpsc::TrySendError::Full(_)) => continue,
                    Err(mpsc::TrySendError::Disconnected(_)) => break,
                }
            }
            // Transient accept errors are ignored; keep serving.
            Err(_) => continue,
        }
    }
}

/// A worker: pull streams off the shared queue and serve one request each.
fn worker_loop(rx: &Arc<Mutex<Receiver<TcpStream>>>, state: &Arc<AppState>) {
    loop {
        // Hold the lock only long enough to receive; serving happens unlocked.
        let stream = {
            let guard = match rx.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            guard.recv()
        };
        match stream {
            Ok(stream) => serve_connection(stream, state),
            Err(_) => break, // channel closed -> shut down
        }
    }
}

/// Serve exactly one request on `stream`, then close it. All failure modes
/// (malformed request, I/O) are contained here; a worker never unwinds.
fn serve_connection(stream: TcpStream, state: &AppState) {
    let _ = stream.set_read_timeout(Some(CONN_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CONN_TIMEOUT));

    // Bound the whole head read by an absolute deadline (not just per-read), so a
    // byte-dripping client cannot hold this worker far beyond CONN_TIMEOUT.
    let reader = DeadlineReader {
        inner: &stream,
        deadline: Instant::now() + HEAD_DEADLINE,
    };
    let response = build_response(reader, state);
    // std implements `Write for &TcpStream`, so a shared borrow writes the reply.
    let mut writer = &stream;
    let _ = response.write_to(&mut writer);
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

/// Parse the request off `reader` and route it, mapping a parse failure to a
/// `400` JSON error.
fn build_response<R: Read>(reader: R, state: &AppState) -> Response {
    match Request::read_from(reader) {
        Ok(Ok(req)) => router::handle(state, &req),
        Ok(Err(parse_err)) => {
            Response::json(Status::BAD_REQUEST, error_body(&parse_err.to_string()))
        }
        // An I/O error reading the head (incl. an over-long line) -> 400.
        Err(_) => Response::json(
            Status::BAD_REQUEST,
            error_body("could not read request"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ingested_workspace;
    // `Read` is already in scope via `super::*`; only `Write` needs importing for
    // `write_all` on the stream.
    use std::io::Write as _;

    /// Send one raw request to `addr` and return the full response text.
    fn roundtrip(addr: SocketAddr, raw: &str) -> String {
        let mut stream = TcpStream::connect(addr).expect("connect");
        stream.write_all(raw.as_bytes()).expect("write");
        let mut resp = String::new();
        stream.read_to_string(&mut resp).expect("read");
        resp
    }

    #[test]
    fn serves_status_and_stats_then_shuts_down() {
        let (_tmp, prism, run) = ingested_workspace();
        let mut server = Server::start(prism, 0, 2).expect("start");
        let addr = server.addr();
        assert_eq!(addr.ip().to_string(), "127.0.0.1");

        // The SPA shell.
        let html = roundtrip(addr, "GET / HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(html.starts_with("HTTP/1.1 200 OK"), "{html}");
        assert!(html.contains("<html"), "{html}");

        // Graph stats for the ingested run: 4 nodes / 3 edges.
        let req = format!("GET /api/graph/stats?run={} HTTP/1.1\r\nHost: x\r\n\r\n", run.get());
        let stats = roundtrip(addr, &req);
        assert!(stats.starts_with("HTTP/1.1 200 OK"), "{stats}");
        assert!(stats.contains(r#""nodes":4"#), "{stats}");
        assert!(stats.contains(r#""edges":3"#), "{stats}");

        server.shutdown();
    }

    #[test]
    fn malformed_request_is_400() {
        let (_tmp, prism, _run) = ingested_workspace();
        let mut server = Server::start(prism, 0, 1).expect("start");
        let addr = server.addr();

        let resp = roundtrip(addr, "GET\r\n\r\n");
        assert!(resp.starts_with("HTTP/1.1 400"), "{resp}");
        assert!(resp.contains("error"), "{resp}");

        server.shutdown();
    }

    #[test]
    fn unknown_api_is_404() {
        let (_tmp, prism, _run) = ingested_workspace();
        let mut server = Server::start(prism, 0, 1).expect("start");
        let addr = server.addr();

        let resp = roundtrip(addr, "GET /api/nope HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(resp.starts_with("HTTP/1.1 404"), "{resp}");
        server.shutdown();
    }

    #[test]
    fn missing_run_param_is_400() {
        let (_tmp, prism, _run) = ingested_workspace();
        let mut server = Server::start(prism, 0, 1).expect("start");
        let addr = server.addr();

        let resp = roundtrip(addr, "GET /api/graph/stats HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(resp.starts_with("HTTP/1.1 400"), "{resp}");
        server.shutdown();
    }

    #[test]
    fn post_is_405() {
        let (_tmp, prism, _run) = ingested_workspace();
        let mut server = Server::start(prism, 0, 1).expect("start");
        let addr = server.addr();

        let resp = roundtrip(addr, "POST /api/status HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(resp.starts_with("HTTP/1.1 405"), "{resp}");
        server.shutdown();
    }
}
