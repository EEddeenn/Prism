//! Request routing: map a parsed [`Request`] to a [`Response`].
//!
//! All state is one [`Prism`] behind a [`Mutex`] (it owns the workspace write
//! lock, so only one may exist; reads still go through the lock). The router is
//! read-only over the facade: it serves graph/TDA queries and static assets, and
//! turns any [`PrismError`] into a JSON `{"error": ...}` body with a mapped
//! status. It never panics on input — unknown routes and bad params are `400`/
//! `404` JSON errors.

use std::sync::Mutex;

use prism_api::Prism;
use prism_model::{GraphNodeId, PrismError, RunId};

use crate::assets;
use crate::http::{Request, Response, Status};
use crate::json::{error_body, JsonWriter};
use crate::query::Query;
use crate::runs;
use crate::views_json;

/// Default node budget for an unbounded forward/impact traversal.
const DEFAULT_LIMIT: u32 = 4096;
/// Default forward-neighborhood depth.
const DEFAULT_DEPTH: u32 = 64;

/// The shared application state served to every connection.
pub struct AppState {
    /// The open workspace facade. The `Mutex` serializes access (the facade is
    /// not `Sync` for concurrent mutation, and reads must not race a `tda_run`).
    prism: Mutex<Prism>,
}

impl AppState {
    /// Wrap an open [`Prism`].
    pub fn new(prism: Prism) -> AppState {
        AppState {
            prism: Mutex::new(prism),
        }
    }

    /// Run `f` with a locked borrow of the facade. A poisoned lock (a prior panic
    /// while holding it) is surfaced as a `500`, not a re-panic.
    fn with_prism<T>(&self, f: impl FnOnce(&Prism) -> ApiResult<T>) -> ApiResult<T> {
        match self.prism.lock() {
            Ok(guard) => f(&guard),
            Err(_) => Err(ApiError::status(
                Status::INTERNAL_ERROR,
                "internal state unavailable",
            )),
        }
    }
}

/// An error carrying the HTTP status it should produce and a message.
struct ApiError {
    status: Status,
    message: String,
}

impl ApiError {
    fn status(status: Status, message: &str) -> ApiError {
        ApiError {
            status,
            message: message.to_string(),
        }
    }

    fn bad_request(message: &str) -> ApiError {
        ApiError::status(Status::BAD_REQUEST, message)
    }

    fn not_found(message: &str) -> ApiError {
        ApiError::status(Status::NOT_FOUND, message)
    }

    /// Map a facade [`PrismError`] to a status: missing data is `404`,
    /// caller-shaped problems (`Other`, bad bounds) are `400`/`404`, and
    /// corruption/IO are `500`.
    fn from_prism(e: PrismError) -> ApiError {
        let (status, message) = match e {
            PrismError::NotFound => (Status::NOT_FOUND, "not found".to_string()),
            PrismError::OutOfBounds { detail } => (Status::NOT_FOUND, detail),
            PrismError::Other(s) => (Status::BAD_REQUEST, s),
            other => (Status::INTERNAL_ERROR, other.to_string()),
        };
        ApiError { status, message }
    }

    fn into_response(self) -> Response {
        Response::json(self.status, error_body(&self.message))
    }
}

impl From<PrismError> for ApiError {
    fn from(e: PrismError) -> Self {
        ApiError::from_prism(e)
    }
}

type ApiResult<T> = Result<T, ApiError>;

/// The top-level entry: dispatch a request to a response. Method, route, and
/// param validation all funnel through here; no path panics.
pub fn handle(state: &AppState, req: &Request) -> Response {
    // Only GET is served; everything else is 405.
    if req.method != "GET" {
        return ApiError::status(Status::METHOD_NOT_ALLOWED, "only GET is supported")
            .into_response();
    }

    // Static assets (and the SPA shell) first.
    if let Some((content_type, body)) = assets::lookup(&req.path) {
        return Response::text(content_type, body);
    }

    // JSON API.
    if let Some(rest) = req.path.strip_prefix("/api/") {
        let query = Query::parse(&req.raw_query);
        return match route_api(state, rest, &query) {
            Ok(body) => Response::json(Status::OK, body),
            Err(e) => e.into_response(),
        };
    }

    ApiError::not_found("no such route").into_response()
}

/// Dispatch the `/api/...` namespace, returning a JSON body on success.
fn route_api(state: &AppState, rest: &str, q: &Query) -> ApiResult<String> {
    match rest {
        "status" => api_status(state),
        "runs" => api_runs(state),
        "graph/stats" => api_graph_stats(state, q),
        "graph/neighborhood" => api_neighborhood(state, q),
        "graph/impact" => api_impact(state, q),
        "graph/scc" => api_scc(state, q),
        "graph/duplicates" => api_duplicates(state, q),
        "tda" => api_tda(state, q),
        "export" => api_export(state, q),
        _ => Err(ApiError::not_found("no such api endpoint")),
    }
}

/// A required `run` query param as a [`RunId`].
fn run_param(q: &Query) -> ApiResult<RunId> {
    q.u64("run")
        .map(RunId::new)
        .ok_or_else(|| ApiError::bad_request("missing or invalid 'run' parameter"))
}

/// A required `node` query param as a [`GraphNodeId`].
fn node_param(q: &Query) -> ApiResult<GraphNodeId> {
    q.u32("node")
        .map(GraphNodeId::new)
        .ok_or_else(|| ApiError::bad_request("missing or invalid 'node' parameter"))
}

// --- endpoint handlers ------------------------------------------------------

/// `GET /api/status` — workspace summary: servable run count + their ids.
fn api_status(state: &AppState) -> ApiResult<String> {
    state.with_prism(|prism| {
        let ids = runs::list_run_ids(prism);
        let mut w = JsonWriter::new();
        w.begin_object();
        w.field_str("name", "prism-serve");
        w.field_u64("run_count", ids.len() as u64);
        w.key("runs");
        w.begin_array();
        for id in &ids {
            w.u64_value(id.get());
        }
        w.end_array();
        w.end_object();
        Ok(w.finish())
    })
}

/// `GET /api/runs` — each servable run with its node/edge counts.
fn api_runs(state: &AppState) -> ApiResult<String> {
    state.with_prism(|prism| {
        let ids = runs::list_run_ids(prism);
        let mut w = JsonWriter::new();
        w.begin_array();
        for id in ids {
            // A run whose stats fail to load is reported with nulls rather than
            // failing the whole listing.
            w.begin_object();
            w.field_u64("run", id.get());
            match prism.graph_stats(id) {
                Ok(s) => {
                    w.field_u64("nodes", s.nodes as u64);
                    w.field_u64("edges", s.edges as u64);
                    w.field_bool("has_cycle", s.has_cycle);
                }
                Err(_) => {
                    w.field_bool("available", false);
                }
            }
            w.end_object();
        }
        w.end_array();
        Ok(w.finish())
    })
}

/// `GET /api/graph/stats?run=` — whole-graph statistics.
fn api_graph_stats(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let stats = prism.graph_stats(run)?;
        let mut w = JsonWriter::new();
        views_json::stats(&mut w, &stats);
        Ok(w.finish())
    })
}

/// `GET /api/graph/neighborhood?run=&node=&depth=&limit=` — forward reachables.
fn api_neighborhood(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    let node = node_param(q)?;
    let depth = q
        .u32_or("depth", DEFAULT_DEPTH)
        .ok_or_else(|| ApiError::bad_request("invalid 'depth' parameter"))?;
    let limit = q
        .u32_or("limit", DEFAULT_LIMIT)
        .ok_or_else(|| ApiError::bad_request("invalid 'limit' parameter"))?;
    state.with_prism(|prism| {
        let view = prism.neighborhood(run, node, depth, limit)?;
        let mut w = JsonWriter::new();
        views_json::neighborhood(&mut w, &view);
        Ok(w.finish())
    })
}

/// `GET /api/graph/impact?run=&node=&depth=` — reverse reachables.
fn api_impact(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    let node = node_param(q)?;
    let depth = q
        .u32_or("depth", u32::MAX)
        .ok_or_else(|| ApiError::bad_request("invalid 'depth' parameter"))?;
    state.with_prism(|prism| {
        let view = prism.impact(run, node, depth)?;
        let mut w = JsonWriter::new();
        views_json::neighborhood(&mut w, &view);
        Ok(w.finish())
    })
}

/// `GET /api/graph/scc?run=` — strongly-connected components.
fn api_scc(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let comps = prism.scc(run)?;
        let mut w = JsonWriter::new();
        views_json::scc_list(&mut w, &comps);
        Ok(w.finish())
    })
}

/// `GET /api/graph/duplicates?run=` — duplicate-version clusters.
fn api_duplicates(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let clusters = prism.duplicate_clusters(run)?;
        let mut w = JsonWriter::new();
        views_json::duplicates(&mut w, &clusters);
        Ok(w.finish())
    })
}

/// `GET /api/tda?run=` — the barcode of the run's first persisted `.ptda`.
///
/// If none is persisted yet, returns `404` (the UI prompts to compute one via
/// the CLI). We never trigger a `tda_run` here — this is a read-only service.
fn api_tda(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let filtrations = prism.tda_filtrations(run);
        let first = filtrations
            .first()
            .copied()
            .ok_or_else(|| ApiError::not_found("no persisted TDA for this run"))?;
        let barcode = prism.tda_barcode(run, first)?;
        let mut w = JsonWriter::new();
        views_json::barcode(&mut w, &barcode);
        Ok(w.finish())
    })
}

/// `GET /api/export?run=` — the whole graph (nodes + edges) for the UI to draw.
///
/// PURL-keyed like the CLI exporter; the integer node ids are included but are
/// snapshot-local. Each directed edge is emitted once (sourced at its `i`).
fn api_export(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let stats = prism.graph_stats(run)?;
        let mut w = JsonWriter::new();
        w.begin_object();
        w.field_u64("run", run.get());
        w.field_u64("nodes", stats.nodes as u64);
        w.field_u64("edges", stats.edges as u64);

        w.key("graph_nodes");
        w.begin_array();
        for i in 0..stats.nodes as u32 {
            let n = prism.node(run, GraphNodeId::new(i))?;
            views_json::node(&mut w, &n);
        }
        w.end_array();

        w.key("graph_edges");
        w.begin_array();
        for i in 0..stats.nodes as u32 {
            let nb = prism.neighborhood(run, GraphNodeId::new(i), 1, u32::MAX)?;
            for e in &nb.edges {
                if e.source.get() != i {
                    continue; // emit each directed edge once, from its source
                }
                views_json::edge(&mut w, e);
            }
        }
        w.end_array();
        w.end_object();
        Ok(w.finish())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prism_error_status_mapping() {
        assert_eq!(
            ApiError::from_prism(PrismError::NotFound).status,
            Status::NOT_FOUND
        );
        assert_eq!(
            ApiError::from_prism(PrismError::OutOfBounds {
                detail: "x".into()
            })
            .status,
            Status::NOT_FOUND
        );
        assert_eq!(
            ApiError::from_prism(PrismError::Other("bad".into())).status,
            Status::BAD_REQUEST
        );
        assert_eq!(
            ApiError::from_prism(PrismError::Corrupt {
                detail: "x".into()
            })
            .status,
            Status::INTERNAL_ERROR
        );
    }

    #[test]
    fn run_param_requires_numeric() {
        assert!(run_param(&Query::parse("run=5")).is_ok());
        assert_eq!(
            run_param(&Query::parse("run=abc")).err().unwrap().status,
            Status::BAD_REQUEST
        );
        assert_eq!(
            run_param(&Query::parse("")).err().unwrap().status,
            Status::BAD_REQUEST
        );
    }

    #[test]
    fn error_into_response_is_json() {
        let resp = ApiError::not_found("nope").into_response();
        assert_eq!(resp.status, Status::NOT_FOUND);
        let body = String::from_utf8(resp.body).unwrap();
        assert_eq!(body, r#"{"error":"nope"}"#);
    }
}
