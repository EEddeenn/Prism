//! Request routing: map a parsed [`Request`] to a [`Response`].
//!
//! All state is one [`Prism`] behind a [`Mutex`] (it owns the workspace write
//! lock, so only one may exist; reads still go through the lock). The router is
//! read-only over the facade: it serves graph/TDA queries as JSON, and
//! turns any [`PrismError`] into a JSON `{"error": ...}` body with a mapped
//! status. It never panics on input — unknown routes and bad params are `400`/
//! `404` JSON errors.

use std::sync::Mutex;

use prism_api::Prism;
use prism_model::{FiltrationId, GraphNodeId, PairId, PrismError, RunId};

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
///
/// This is a pure JSON API (the UI is the separate Next.js app): `OPTIONS` is a
/// CORS preflight, `GET /` is a small service descriptor, `GET /api/...` is the
/// data surface, and everything else is a JSON `404`/`405`.
pub fn handle(state: &AppState, req: &Request) -> Response {
    // CORS preflight: answer 204 with the allow-* headers (added by write_to).
    if req.method == "OPTIONS" {
        return Response::no_content();
    }
    // Only GET is otherwise served; everything else is 405.
    if req.method != "GET" {
        return ApiError::status(Status::METHOD_NOT_ALLOWED, "only GET is supported")
            .into_response();
    }

    // Root: a tiny machine-readable descriptor pointing at the API.
    if req.path == "/" {
        return Response::json(Status::OK, api_index_body());
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

/// The `GET /` body: identifies the service and lists the JSON endpoints so a
/// human (or a probe) hitting the bare host learns where the data lives.
fn api_index_body() -> String {
    let mut w = JsonWriter::new();
    w.begin_object();
    w.field_str("name", "prism-serve");
    w.field_str("kind", "json-api");
    w.field_str("ui", "the Prism web app (Next.js) consumes this API");
    w.key("endpoints");
    w.begin_array();
    for ep in [
        "/api/status",
        "/api/runs",
        "/api/run?run=",
        "/api/graph/stats?run=",
        "/api/graph/neighborhood?run=&node=",
        "/api/graph/impact?run=&node=",
        "/api/graph/scc?run=",
        "/api/graph/duplicates?run=",
        "/api/graph/scope?run=",
        "/api/graph/chokepoints?run=",
        "/api/graph/cycles?run=",
        "/api/graph/why?run=&node=",
        "/api/graph/dup-causes?run=",
        "/api/graph/risk?run=",
        "/api/graph/fragmentation?run=",
        "/api/graph/subsystems?run=",
        "/api/graph/diamonds?run=",
        "/api/graph/minimal?run=",
        "/api/graph/shared?run=",
        "/api/graph/distance?base=&head=",
        "/api/graph/shape?run=",
        "/api/graph/h2?run=",
        "/api/regions",
        "/api/tda?run=",
        "/api/tda/pairs?run=",
        "/api/tda/support?run=&pair=",
        "/api/tda/compare?run=",
        "/api/export?run=",
    ] {
        w.str_value(ep);
    }
    w.end_array();
    w.end_object();
    w.finish()
}

/// Dispatch the `/api/...` namespace, returning a JSON body on success.
fn route_api(state: &AppState, rest: &str, q: &Query) -> ApiResult<String> {
    match rest {
        "status" => api_status(state),
        "runs" => api_runs(state),
        "run" => api_run(state, q),
        "graph/stats" => api_graph_stats(state, q),
        "graph/neighborhood" => api_neighborhood(state, q),
        "graph/impact" => api_impact(state, q),
        "graph/scc" => api_scc(state, q),
        "graph/duplicates" => api_duplicates(state, q),
        "graph/scope" => api_scope(state, q),
        "graph/chokepoints" => api_chokepoints(state, q),
        "graph/cycles" => api_classified_cycles(state, q),
        "graph/why" => api_why(state, q),
        "graph/dup-causes" => api_dup_causes(state, q),
        "graph/risk" => api_risk(state, q),
        "graph/fragmentation" => api_fragmentation(state, q),
        "graph/subsystems" => api_subsystems(state, q),
        "graph/diamonds" => api_diamonds(state, q),
        "graph/minimal" => api_minimal(state, q),
        "graph/shared" => api_shared(state, q),
        "graph/distance" => api_distance(state, q),
        "graph/shape" => api_shape(state, q),
        "graph/h2" => api_h2(state, q),
        "regions" => api_regions(state),
        "tda" => api_tda(state, q),
        "tda/pairs" => api_tda_pairs(state, q),
        "tda/support" => api_tda_support(state, q),
        "tda/compare" => api_tda_compare(state, q),
        "package/candidates" => api_package_candidates(state, q),
        "export" => api_export(state, q),
        _ => Err(ApiError::not_found("no such api endpoint")),
    }
}

// Note: this loopback-only, read-only viewer intentionally uses the snapshot-local
// integer `run`/`node` ids as ephemeral request handles (the design doc's opaque
// `<runId>:<nodeId>` string scheme + cursor store is for a future multi-tenant HTTP
// mode). The portable key — the PURL — is always included in every node payload.

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

/// `GET /api/run?run=` — run provenance: ecosystem, resolver, status, and the
/// [`prism_model::GraphFidelity`] (tier + "what was dropped" caveats) so the UI
/// never overclaims correctness (architecture doc: fidelity travels with a run).
fn api_run(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let record = prism.store().get_run(run)?;
        let resolver = prism
            .store()
            .resolve_string(record.resolver_name)
            .unwrap_or_default();
        let mut w = JsonWriter::new();
        w.begin_object();
        w.field_u64("run", run.get());
        w.field_str("ecosystem", ecosystem_name(record.ecosystem));
        w.field_str("resolver", &resolver);
        w.field_str("status", run_status_name(record.status));
        w.field_str("fidelity_tier", fidelity_tier_name(record.fidelity.tier));
        w.key("fidelity_caveats");
        w.begin_array();
        for name in caveat_names(record.fidelity.caveats) {
            w.str_value(name);
        }
        w.end_array();
        w.end_object();
        Ok(w.finish())
    })
}

/// Display name for an ecosystem id. (npm and Bun share id 2; uv/PyPI is id 3.)
fn ecosystem_name(id: prism_model::EcosystemId) -> &'static str {
    match id.get() {
        1 => "cargo",
        2 => "npm",
        3 => "pypi",
        _ => "unknown",
    }
}

/// Display name for a resolver-run status.
fn run_status_name(s: prism_model::RunStatus) -> &'static str {
    match s {
        prism_model::RunStatus::Pending => "pending",
        prism_model::RunStatus::Complete => "complete",
        prism_model::RunStatus::Truncated => "truncated",
        prism_model::RunStatus::Failed => "failed",
    }
}

/// Display name for a graph fidelity tier.
fn fidelity_tier_name(t: prism_model::FidelityTier) -> &'static str {
    match t {
        prism_model::FidelityTier::LockfileExact => "lockfile-exact",
        prism_model::FidelityTier::NativeToolResolved => "native-tool-resolved",
        prism_model::FidelityTier::ModelResolved => "model-resolved",
        prism_model::FidelityTier::CandidateSpaceOnly => "candidate-space-only",
    }
}

/// The names of the set caveat flags (what the resolution did *not* model).
fn caveat_names(c: prism_model::FidelityCaveats) -> Vec<&'static str> {
    use prism_model::FidelityCaveats as F;
    [
        (F::NO_PEER_PLACEMENT, "no-peer-placement"),
        (F::NO_FEATURE_UNIFICATION, "no-feature-unification"),
        (F::NO_PROFILE_EXPANSION, "no-profile-expansion"),
        (F::NO_MEDIATION, "no-mediation"),
        (F::NO_EXTRA_ACTIVATION, "no-extra-activation"),
        (F::MARKERS_APPROXIMATED, "markers-approximated"),
        (F::DYNAMIC_METADATA_SKIPPED, "dynamic-metadata-skipped"),
        (F::BUDGET_TRUNCATED, "budget-truncated"),
    ]
    .into_iter()
    .filter(|(flag, _)| c.contains(*flag))
    .map(|(_, name)| name)
    .collect()
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

/// `GET /api/graph/scope?run=` — per-scope footprint summary.
fn api_scope(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let rows = prism.scope_summary(run)?;
        let mut w = JsonWriter::new();
        views_json::scope_summary(&mut w, &rows);
        Ok(w.finish())
    })
}

/// `GET /api/graph/chokepoints?run=&limit=` — structural gateways.
fn api_chokepoints(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    let limit = q.u32_or("limit", 25).unwrap_or(25) as usize;
    state.with_prism(|prism| {
        let cps = prism.chokepoints(run, limit)?;
        let mut w = JsonWriter::new();
        views_json::chokepoints(&mut w, &cps);
        Ok(w.finish())
    })
}

/// `GET /api/graph/cycles?run=` — classified dependency cycles.
fn api_classified_cycles(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let cycles = prism.classified_cycles(run)?;
        let mut w = JsonWriter::new();
        views_json::classified_cycles(&mut w, &cycles);
        Ok(w.finish())
    })
}

/// `GET /api/graph/why?run=&node=` — why a dependency occurrence is present:
/// its introducers, a few shortest root→node paths, and the nearest owner.
fn api_why(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    let node = node_param(q)?;
    state.with_prism(|prism| {
        let v = prism.explain_node(run, node, 6)?;
        let mut w = JsonWriter::new();
        views_json::explanation(&mut w, &v);
        Ok(w.finish())
    })
}

/// `GET /api/graph/dup-causes?run=` — duplicate-version root cause: who pins
/// each coexisting version of a fragmented family, and the convergence verdict.
fn api_dup_causes(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let views = prism.duplicate_root_causes(run)?;
        let mut w = JsonWriter::new();
        views_json::duplicate_root_causes(&mut w, &views);
        Ok(w.finish())
    })
}

/// `GET /api/graph/subsystems?run=` — the top-level subsystem partition.
fn api_subsystems(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let subs = prism.subsystems(run)?;
        let mut w = JsonWriter::new();
        views_json::subsystems(&mut w, &subs);
        Ok(w.finish())
    })
}

/// `GET /api/graph/diamonds?run=&limit=` — diamond-reuse foundations.
fn api_diamonds(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    let limit = q.u32_or("limit", 15).unwrap_or(15) as usize;
    state.with_prism(|prism| {
        let v = prism.diamond_reuse(run, limit)?;
        let mut w = JsonWriter::new();
        views_json::diamonds(&mut w, &v);
        Ok(w.finish())
    })
}

/// `GET /api/graph/minimal?run=` — minimal (dominator-tree) explanation.
fn api_minimal(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let v = prism.minimal_view(run)?;
        let mut w = JsonWriter::new();
        views_json::minimal(&mut w, &v);
        Ok(w.finish())
    })
}

/// `GET /api/graph/shared?run=&limit=` — higher-order overlap across subsystems.
fn api_shared(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    let limit = q.u32_or("limit", 15).unwrap_or(15) as usize;
    state.with_prism(|prism| {
        let v = prism.higher_order_overlap(run, limit)?;
        let mut w = JsonWriter::new();
        views_json::overlap(&mut w, &v);
        Ok(w.finish())
    })
}

/// `GET /api/graph/distance?base=&head=` — topology shape distance between runs.
fn api_distance(state: &AppState, q: &Query) -> ApiResult<String> {
    let base = q
        .u64("base")
        .map(RunId::new)
        .ok_or_else(|| ApiError::bad_request("missing or invalid 'base' parameter"))?;
    let head = q
        .u64("head")
        .map(RunId::new)
        .ok_or_else(|| ApiError::bad_request("missing or invalid 'head' parameter"))?;
    state.with_prism(|prism| {
        let v = prism.topology_distance(base, head)?;
        let mut w = JsonWriter::new();
        views_json::topology_distance(&mut w, &v);
        Ok(w.finish())
    })
}

/// `GET /api/graph/risk?run=` — structural risk profile.
fn api_risk(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let r = prism.structural_risk(run)?;
        let mut w = JsonWriter::new();
        views_json::risk(&mut w, &r);
        Ok(w.finish())
    })
}

/// `GET /api/graph/fragmentation?run=` — connected-component breakdown.
fn api_fragmentation(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let f = prism.fragmentation(run)?;
        let mut w = JsonWriter::new();
        views_json::fragmentation(&mut w, &f);
        Ok(w.finish())
    })
}

/// `GET /api/graph/shape?run=` — topology shape signature.
fn api_shape(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let s = prism.shape_signature(run)?;
        let mut w = JsonWriter::new();
        views_json::shape(&mut w, &s);
        Ok(w.finish())
    })
}

/// `GET /api/graph/h2?run=` — experimental flag-complex H2 homology.
fn api_h2(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let h = prism.tda_h2(run)?;
        let mut w = JsonWriter::new();
        views_json::h2(&mut w, &h);
        Ok(w.finish())
    })
}

/// `GET /api/regions` — every run as a comparable dependency region.
fn api_regions(state: &AppState) -> ApiResult<String> {
    state.with_prism(|prism| {
        let regions = prism.workspace_regions()?;
        let mut w = JsonWriter::new();
        views_json::regions(&mut w, &regions);
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

/// Resolve the filtration to serve for `run`: the explicit `filtration` query
/// param if given, else the run's first persisted `.ptda`. A `404` if neither.
fn resolve_filtration(prism: &Prism, run: RunId, q: &Query) -> ApiResult<FiltrationId> {
    if let Some(f) = q.u64("filtration") {
        return Ok(FiltrationId::new(f));
    }
    prism
        .tda_filtrations(run)
        .first()
        .copied()
        .ok_or_else(|| ApiError::not_found("no persisted TDA for this run"))
}

/// `GET /api/tda/pairs?run=&filtration=` — the individual persistence bars.
///
/// Where `/api/tda` returns per-dimension counts, this lists each pair (id,
/// dimension, birth/death, persistence) so the UI can offer feature selection
/// for the 3D TDA view. `filtration` defaults to the run's first.
fn api_tda_pairs(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let filtration = resolve_filtration(prism, run, q)?;
        let ps = prism.tda_pairs(run, filtration)?;
        let mut w = JsonWriter::new();
        views_json::pairs(&mut w, run, filtration, &ps);
        Ok(w.finish())
    })
}

/// `GET /api/tda/support?run=&pair=&filtration=` — one pair's generator chain,
/// resolved to the concrete graph nodes/edges that carry the feature.
fn api_tda_support(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    let pair = q
        .u64("pair")
        .map(PairId::new)
        .ok_or_else(|| ApiError::bad_request("missing or invalid 'pair' parameter"))?;
    state.with_prism(|prism| {
        let filtration = resolve_filtration(prism, run, q)?;
        let view = prism.tda_support(run, filtration, pair)?;
        let mut w = JsonWriter::new();
        views_json::support(&mut w, &view);
        Ok(w.finish())
    })
}

/// `GET /api/tda/compare?run=` — the run's topology under multiple filtration
/// lenses (depth/centrality/scope): one summary row per lens.
fn api_tda_compare(state: &AppState, q: &Query) -> ApiResult<String> {
    let run = run_param(q)?;
    state.with_prism(|prism| {
        let rows = prism.tda_compare(run)?;
        let mut w = JsonWriter::new();
        views_json::filtration_compare(&mut w, &rows);
        Ok(w.finish())
    })
}

/// `GET /api/package/candidates?package=<base-purl>&constraint=<expr>` — the
/// version "headroom" for a package: its versions sorted into ordinals, the
/// satisfying set for `constraint` (default `*` = all), coalesced ranges, and
/// per-version exclusions. Versions are store-global, so no `run` is required.
fn api_package_candidates(state: &AppState, q: &Query) -> ApiResult<String> {
    let purl = q
        .get("package")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::bad_request("missing 'package' parameter"))?;
    let constraint = q.get("constraint").filter(|s| !s.is_empty()).unwrap_or("*");
    state.with_prism(|prism| {
        let (pkg, eco) = prism.package_by_purl(purl)?;
        let view = prism.candidates_for(pkg, constraint, eco)?;
        let mut w = JsonWriter::new();
        views_json::candidates(&mut w, &view);
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
        // Single snapshot load (was ~2N reads + SHA-256 verifies + CSR parses).
        let (nodes, edges) = prism.graph_export(run)?;
        let mut w = JsonWriter::new();
        w.begin_object();
        w.field_u64("run", run.get());
        w.field_u64("nodes", nodes.len() as u64);
        w.field_u64("edges", edges.len() as u64);

        w.key("graph_nodes");
        w.begin_array();
        for n in &nodes {
            views_json::node(&mut w, n);
        }
        w.end_array();

        w.key("graph_edges");
        w.begin_array();
        for e in &edges {
            views_json::edge(&mut w, e);
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
    fn run_metadata_name_mappings() {
        use prism_model::{EcosystemId, FidelityCaveats, FidelityTier, RunStatus};
        assert_eq!(ecosystem_name(EcosystemId::new(1)), "cargo");
        assert_eq!(ecosystem_name(EcosystemId::new(2)), "npm");
        assert_eq!(ecosystem_name(EcosystemId::new(99)), "unknown");
        assert_eq!(run_status_name(RunStatus::Complete), "complete");
        assert_eq!(
            fidelity_tier_name(FidelityTier::LockfileExact),
            "lockfile-exact"
        );
        // No caveats => empty; a couple set => their names, in flag order.
        assert!(caveat_names(FidelityCaveats::EMPTY).is_empty());
        let some = FidelityCaveats::NO_MEDIATION | FidelityCaveats::BUDGET_TRUNCATED;
        assert_eq!(caveat_names(some), vec!["no-mediation", "budget-truncated"]);
    }

    #[test]
    fn error_into_response_is_json() {
        let resp = ApiError::not_found("nope").into_response();
        assert_eq!(resp.status, Status::NOT_FOUND);
        let body = String::from_utf8(resp.body).unwrap();
        assert_eq!(body, r#"{"error":"nope"}"#);
    }
}
