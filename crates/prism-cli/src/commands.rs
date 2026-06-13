//! Command handlers: each maps a parsed [`Command`] to a `prism-api` call and
//! renders a friendly, line-oriented report to `out`.
//!
//! Handlers borrow a `&mut dyn Write` for normal output so tests can capture it;
//! errors surface as [`PrismError`] and are turned into a message + exit code by
//! the caller in [`crate::run`].

use std::io::Write;
use std::path::Path;

use prism_api::{Ecosystem, Prism};
use prism_model::{
    FiltrationId, GraphNodeId, PairId, PrismError, ResolverRun, Result, RunId,
};
use prism_store::{record_type, records};

use crate::json::JsonWriter;

/// Find the existing workspace for a command that operates on one. The CLI uses
/// the current directory as the workspace root.
fn open_here() -> Result<Prism> {
    Prism::open(workspace_root())
}

/// The workspace root: the current working directory.
fn workspace_root() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf())
}

/// `prism init [path]`.
pub fn init(out: &mut dyn Write, path: &str) -> Result<()> {
    let prism = Prism::init(path)?;
    // Touch the store so the parameter is plainly used and the workspace is open.
    let _ = prism.store();
    writeln!(out, "initialized prism workspace at {path}").map_err(PrismError::Io)?;
    Ok(())
}

/// `prism ingest <path>` — manifest or lockfile, detected by file name.
pub fn ingest(out: &mut dyn Write, path: &str) -> Result<()> {
    let p = Path::new(path);
    let mut prism = open_here()?;
    if Ecosystem::is_lockfile(p) {
        let res = prism.ingest_lockfile_detailed(p)?;
        writeln!(
            out,
            "ingested lockfile ({}): run {} with {} nodes, {} edges",
            res.ecosystem.label(),
            res.run.get(),
            res.nodes,
            res.edges
        )
        .map_err(PrismError::Io)?;
    } else if Ecosystem::is_manifest(p) {
        let res = prism.ingest_manifest(p)?;
        writeln!(
            out,
            "ingested manifest ({}): {} packages, {} versions, {} requirements",
            res.ecosystem.label(),
            res.packages,
            res.versions,
            res.requirements
        )
        .map_err(PrismError::Io)?;
    } else {
        return Err(PrismError::Other(format!(
            "{path:?} is not a recognized manifest or lockfile"
        )));
    }
    Ok(())
}

/// `prism status` — runs, packages, and persisted graphs/TDA in the workspace.
pub fn status(out: &mut dyn Write) -> Result<()> {
    let prism = open_here()?;
    let store = prism.store();

    // Runs and packages come from the record log; graphs/TDA from the manifest's
    // published file set.
    let runs = list_runs(&prism)?;
    let mut packages = 0usize;
    for rec in store.read_all_records()? {
        if rec.record_type == record_type::PACKAGE {
            packages += 1;
        }
    }
    let manifest = store.manifest();
    let graphs = manifest
        .files
        .iter()
        .filter(|f| f.kind == prism_store::FileKind::Graph)
        .count();
    let tda = manifest
        .files
        .iter()
        .filter(|f| f.kind == prism_store::FileKind::Tda)
        .count();

    writeln!(out, "workspace: {}", workspace_root().display()).map_err(PrismError::Io)?;
    writeln!(out, "runs:     {}", runs.len()).map_err(PrismError::Io)?;
    writeln!(out, "packages: {packages}").map_err(PrismError::Io)?;
    writeln!(out, "graphs:   {graphs}").map_err(PrismError::Io)?;
    writeln!(out, "tda:      {tda}").map_err(PrismError::Io)?;
    for run in &runs {
        let label = ecosystem_label(run.ecosystem);
        writeln!(out, "  run {} [{}]", run.id.get(), label).map_err(PrismError::Io)?;
    }
    Ok(())
}

/// `prism graph stats --run <id>`.
pub fn graph_stats(out: &mut dyn Write, run: u64) -> Result<()> {
    let prism = open_here()?;
    let s = prism.graph_stats(RunId::new(run))?;
    writeln!(out, "run {}", s.run.get()).map_err(PrismError::Io)?;
    writeln!(out, "nodes:             {}", s.nodes).map_err(PrismError::Io)?;
    writeln!(out, "edges:             {}", s.edges).map_err(PrismError::Io)?;
    writeln!(out, "scc count:         {}", s.scc_count).map_err(PrismError::Io)?;
    writeln!(out, "has cycle:         {}", s.has_cycle).map_err(PrismError::Io)?;
    writeln!(out, "max depth:         {}", s.max_depth).map_err(PrismError::Io)?;
    writeln!(out, "duplicate versions: {}", s.duplicate_version_count)
        .map_err(PrismError::Io)?;
    Ok(())
}

/// `prism graph neighborhood --run <id> --node <id> --depth <n>`.
pub fn graph_neighborhood(out: &mut dyn Write, run: u64, node: u32, depth: u32) -> Result<()> {
    let prism = open_here()?;
    let view = prism.neighborhood(RunId::new(run), GraphNodeId::new(node), depth, 1024)?;
    writeln!(
        out,
        "center: node {} {} (depth {})",
        view.center.id.get(),
        view.center.purl,
        view.center.depth
    )
    .map_err(PrismError::Io)?;
    writeln!(out, "reached {} node(s):", view.nodes.len()).map_err(PrismError::Io)?;
    for n in &view.nodes {
        writeln!(out, "  node {} {} (depth {})", n.id.get(), n.purl, n.depth)
            .map_err(PrismError::Io)?;
    }
    writeln!(out, "induced {} edge(s)", view.edges.len()).map_err(PrismError::Io)?;
    if view.truncated {
        writeln!(out, "(truncated: node budget reached)").map_err(PrismError::Io)?;
    }
    Ok(())
}

/// `prism graph impact --run <id> --node <id>` — reverse reachability (all
/// ancestors that depend on the node).
pub fn graph_impact(out: &mut dyn Write, run: u64, node: u32) -> Result<()> {
    let prism = open_here()?;
    // Depth `u32::MAX` => the full ancestor set (architecture: all dependents).
    let view = prism.impact(RunId::new(run), GraphNodeId::new(node), u32::MAX)?;
    writeln!(out, "impact of node {} {}", view.center.id.get(), view.center.purl)
        .map_err(PrismError::Io)?;
    writeln!(out, "{} dependent(s):", view.nodes.len()).map_err(PrismError::Io)?;
    for n in &view.nodes {
        writeln!(out, "  node {} {} (depth {})", n.id.get(), n.purl, n.depth)
            .map_err(PrismError::Io)?;
    }
    Ok(())
}

/// `prism tda run --run <id> --filtration depth`.
pub fn tda_run(out: &mut dyn Write, run: u64, filtration: &str) -> Result<()> {
    let mut prism = open_here()?;
    let barcode = prism.tda_run(RunId::new(run), filtration)?;
    writeln!(
        out,
        "tda run {} filtration {}",
        barcode.run.get(),
        barcode.filtration.get()
    )
    .map_err(PrismError::Io)?;
    writeln!(out, "components (H0 essential): {}", barcode.component_count())
        .map_err(PrismError::Io)?;
    writeln!(out, "cycles (H1 essential):     {}", barcode.cycle_count())
        .map_err(PrismError::Io)?;
    for d in &barcode.dims {
        writeln!(
            out,
            "  dim {}: {} finite, {} essential",
            d.dim, d.finite, d.essential
        )
        .map_err(PrismError::Io)?;
    }
    Ok(())
}

/// `prism tda support --pair <id>` — locate the persisted `.ptda` owning the
/// pair and render its generator support. Pair ids are store-global, so the
/// owning `(run, filtration)` is recovered by scanning persisted filtrations.
pub fn tda_support(out: &mut dyn Write, pair: u64) -> Result<()> {
    let prism = open_here()?;
    let pair = PairId::new(pair);
    let (run, filtration) = locate_pair(&prism, pair)?.ok_or(PrismError::NotFound)?;
    let view = prism.tda_support(run, filtration, pair)?;
    writeln!(
        out,
        "pair {} (dim {}) in run {} filtration {}",
        view.pair.get(),
        view.dim,
        run.get(),
        filtration.get()
    )
    .map_err(PrismError::Io)?;
    writeln!(out, "{} support object(s):", view.objects.len()).map_err(PrismError::Io)?;
    for obj in &view.objects {
        match obj {
            prism_api::SupportObject::Node(n) => {
                writeln!(out, "  node {} {}", n.id.get(), n.purl).map_err(PrismError::Io)?;
            }
            prism_api::SupportObject::Edge { id, source, target } => {
                writeln!(
                    out,
                    "  edge {} ({} -> {})",
                    id.get(),
                    source.get(),
                    target.get()
                )
                .map_err(PrismError::Io)?;
            }
        }
    }
    Ok(())
}

/// `prism export json --run <id>` — emit the run's graph as JSON to `out`.
///
/// PURL-keyed for portability (architecture doc: exports use package/version
/// strings as the stable key; the snapshot-local integer node ids are included
/// but documented as non-portable).
pub fn export_json(out: &mut dyn Write, run: u64) -> Result<()> {
    let prism = open_here()?;
    let run = RunId::new(run);
    let stats = prism.graph_stats(run)?;

    let mut w = JsonWriter::new();
    w.begin_object();
    w.field_u64("run", run.get());
    w.field_u64("nodes", stats.nodes as u64);
    w.field_u64("edges", stats.edges as u64);

    // Nodes, resolved for display (id is snapshot-local; purl is the stable key).
    w.key("graph_nodes");
    w.begin_array();
    for i in 0..stats.nodes as u32 {
        let node = prism.node(run, GraphNodeId::new(i))?;
        w.begin_object();
        w.field_u64("id", node.id.get() as u64);
        w.field_str("purl", &node.purl);
        w.field_u64("version_id", node.version.get());
        w.field_u64("depth", node.depth as u64);
        w.end_object();
    }
    w.end_array();

    // Edges: induced over the whole graph via the full forward neighborhood of
    // every node is wasteful; instead use the neighborhood of each node at depth
    // 1 to enumerate direct out-edges. We do this through the public facade by
    // walking each node's one-hop neighborhood.
    w.key("graph_edges");
    w.begin_array();
    for i in 0..stats.nodes as u32 {
        let nb = prism.neighborhood(run, GraphNodeId::new(i), 1, u32::MAX)?;
        for e in &nb.edges {
            // Keep only edges sourced at `i` so each directed edge is emitted once.
            if e.source.get() != i {
                continue;
            }
            w.begin_object();
            w.field_u64("source", e.source.get() as u64);
            w.field_u64("target", e.target.get() as u64);
            w.field_u64("kind", e.kind as u64);
            w.field_u64("flags", e.flags as u64);
            w.end_object();
        }
    }
    w.end_array();
    w.end_object();

    writeln!(out, "{}", w.finish()).map_err(PrismError::Io)?;
    Ok(())
}

// --- helpers ----------------------------------------------------------------

/// Every [`ResolverRun`] in the workspace, ascending by run id.
fn list_runs(prism: &Prism) -> Result<Vec<ResolverRun>> {
    let mut runs = Vec::new();
    for rec in prism.store().read_all_records()? {
        if rec.record_type == record_type::RESOLVER_RUN {
            if let Ok(run) = records::decode_resolver_run(&rec.value) {
                runs.push(run);
            }
        }
    }
    runs.sort_by_key(|r| r.id.get());
    runs.dedup_by_key(|r| r.id.get());
    Ok(runs)
}

/// Find the `(run, filtration)` whose persisted `.ptda` contains `pair`.
fn locate_pair(prism: &Prism, pair: PairId) -> Result<Option<(RunId, FiltrationId)>> {
    for run in list_runs(prism)? {
        for filtration in prism.tda_filtrations(run.id) {
            // `tda_support` returns NotFound if the pair is absent; treat that as
            // "keep looking" and surface other errors.
            match prism.tda_support(run.id, filtration, pair) {
                Ok(_) => return Ok(Some((run.id, filtration))),
                Err(PrismError::NotFound) => continue,
                Err(e) => return Err(e),
            }
        }
    }
    Ok(None)
}

/// A short ecosystem label for display, recovered from the stored id.
fn ecosystem_label(eco: prism_model::EcosystemId) -> &'static str {
    if eco == Ecosystem::Npm.id() {
        "npm"
    } else if eco == Ecosystem::Cargo.id() {
        "cargo"
    } else {
        "?"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsupport::TempDir;

    const CARGO_LOCK: &str = r#"
# @generated by Cargo
version = 3

[[package]]
name = "app"
version = "0.1.0"
dependencies = [
 "serde 1.0.197",
]

[[package]]
name = "serde"
version = "1.0.197"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

    /// Run a closure with the process cwd set to `dir`, restoring it to a
    /// guaranteed-stable directory afterward. The CLI handlers key off the cwd,
    /// so tests must pin it; the caller holds [`crate::testsupport::cwd_lock`] so
    /// only one such test runs at a time. We restore to the OS temp root (always
    /// present) rather than the prior cwd, which may have been a now-removed
    /// `TempDir` from an earlier test.
    fn with_cwd<R>(dir: &Path, f: impl FnOnce() -> R) -> R {
        std::env::set_current_dir(dir).unwrap();
        let r = f();
        std::env::set_current_dir(std::env::temp_dir()).unwrap();
        r
    }

    #[test]
    fn init_status_ingest_roundtrip() {
        // cwd mutation is process-global; serialize these handler tests.
        let _guard = crate::testsupport::cwd_lock();
        let td = TempDir::new("cmd");
        let mut buf = Vec::new();
        init(&mut buf, td.path().to_str().unwrap()).unwrap();
        assert!(String::from_utf8(buf).unwrap().contains("initialized"));

        std::fs::write(td.path().join("Cargo.lock"), CARGO_LOCK).unwrap();

        with_cwd(td.path(), || {
            let mut out = Vec::new();
            ingest(&mut out, "Cargo.lock").unwrap();
            let s = String::from_utf8(out).unwrap();
            assert!(s.contains("ingested lockfile"), "{s}");
            assert!(s.contains("2 nodes"), "{s}");

            let mut st = Vec::new();
            status(&mut st).unwrap();
            let s = String::from_utf8(st).unwrap();
            assert!(s.contains("runs:     1"), "{s}");
            assert!(s.contains("packages: 2"), "{s}");
            assert!(s.contains("graphs:   1"), "{s}");
        });
    }

    #[test]
    fn export_json_emits_nodes_and_edges() {
        let _guard = crate::testsupport::cwd_lock();
        let td = TempDir::new("cmd-export");
        std::fs::write(td.path().join("Cargo.lock"), CARGO_LOCK).unwrap();
        let run = {
            let mut prism = Prism::init(td.path()).unwrap();
            prism.ingest_lockfile(td.path().join("Cargo.lock")).unwrap()
        };
        with_cwd(td.path(), || {
            let mut out = Vec::new();
            export_json(&mut out, run.get()).unwrap();
            let json = String::from_utf8(out).unwrap();
            assert!(json.contains(r#""nodes":2"#), "{json}");
            assert!(json.contains(r#""edges":1"#), "{json}");
            assert!(json.contains("pkg:cargo/app@0.1.0"), "{json}");
            assert!(json.contains(r#""graph_edges":[{"source":0"#), "{json}");
        });
    }

    #[test]
    fn missing_run_is_not_found() {
        let _guard = crate::testsupport::cwd_lock();
        let td = TempDir::new("cmd-missing");
        // Create then release the workspace so the command can re-open it.
        drop(Prism::init(td.path()).unwrap());
        with_cwd(td.path(), || {
            let mut out = Vec::new();
            let err = graph_stats(&mut out, 999).unwrap_err();
            // No graph published for run 999 -> the store reports NotFound.
            assert!(matches!(err, PrismError::NotFound), "{err:?}");
        });
    }
}
