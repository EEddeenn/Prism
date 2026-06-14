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
    FiltrationId, GraphNodeId, PairId, PrismError, RequirementId, ResolverRun, Result, RunId,
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
    // Every other command operates on the workspace in the *current* directory, so
    // a non-`.` path here needs a matching `cd` to be reachable.
    if path != "." {
        writeln!(
            out,
            "note: other commands use the workspace in the current directory — run them from {path}"
        )
        .map_err(PrismError::Io)?;
    }
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

/// `prism graph duplicates --run <id>` — list package families present at more
/// than one version, with their versions and depth span.
pub fn graph_duplicates(out: &mut dyn Write, run: u64) -> Result<()> {
    let prism = open_here()?;
    let clusters = prism.duplicate_clusters(RunId::new(run))?;
    if clusters.is_empty() {
        writeln!(out, "no duplicate-version families in run {run}").map_err(PrismError::Io)?;
        return Ok(());
    }
    writeln!(
        out,
        "{} duplicate-version famil{} in run {run}:",
        clusters.len(),
        if clusters.len() == 1 { "y" } else { "ies" }
    )
    .map_err(PrismError::Io)?;
    for c in &clusters {
        writeln!(
            out,
            "  {} — {} versions (depth {}..{}):",
            c.package_purl,
            c.versions.len(),
            c.min_depth,
            c.max_depth
        )
        .map_err(PrismError::Io)?;
        for v in &c.versions {
            writeln!(out, "      {}", v.purl).map_err(PrismError::Io)?;
        }
    }
    Ok(())
}

/// `prism graph scope --run <id>` — per-scope dependency footprint summary.
pub fn graph_scope(out: &mut dyn Write, run: u64) -> Result<()> {
    let prism = open_here()?;
    let rows = prism.scope_summary(RunId::new(run))?;
    if rows.is_empty() {
        writeln!(out, "no scoped dependency edges in run {run}").map_err(PrismError::Io)?;
        return Ok(());
    }
    writeln!(out, "scope footprints for run {run}:").map_err(PrismError::Io)?;
    writeln!(
        out,
        "  {:<10} {:>7} {:>9} {:>9} {:>9}",
        "scope", "edges", "footprint", "exclusive", "max-depth"
    )
    .map_err(PrismError::Io)?;
    for r in &rows {
        writeln!(
            out,
            "  {:<10} {:>7} {:>9} {:>9} {:>9}",
            r.kind, r.direct_edges, r.footprint, r.exclusive, r.max_depth
        )
        .map_err(PrismError::Io)?;
    }
    writeln!(
        out,
        "(footprint = reachable from roots under production+scope; exclusive = added beyond production)"
    )
    .map_err(PrismError::Io)?;
    Ok(())
}

/// `prism graph chokepoints --run <id> [--limit <n>]` — packages that
/// structurally control access to large parts of the graph.
pub fn graph_chokepoints(out: &mut dyn Write, run: u64, limit: u32) -> Result<()> {
    let prism = open_here()?;
    let cps = prism.chokepoints(RunId::new(run), limit as usize)?;
    if cps.is_empty() {
        writeln!(out, "no chokepoints in run {run} (graph is a flat fan-out)")
            .map_err(PrismError::Io)?;
        return Ok(());
    }
    writeln!(out, "top {} chokepoint(s) in run {run}:", cps.len()).map_err(PrismError::Io)?;
    for c in &cps {
        writeln!(
            out,
            "  {:>5} dominated  node {} {} (depth {})",
            c.dominated,
            c.node.id.get(),
            c.node.purl,
            c.node.depth
        )
        .map_err(PrismError::Io)?;
    }
    writeln!(
        out,
        "(dominated = occurrences reachable from the roots only through this package)"
    )
    .map_err(PrismError::Io)?;
    Ok(())
}

/// `prism graph why --run <id> --node <id>` — explain a dependency's presence.
pub fn graph_why(out: &mut dyn Write, run: u64, node: u32) -> Result<()> {
    let prism = open_here()?;
    let ex = prism
        .explain_node(RunId::new(run), GraphNodeId::new(node), 4)
        .map_err(id_not_found)?;
    writeln!(out, "why is node {} {} here?", ex.target.id.get(), ex.target.purl)
        .map_err(PrismError::Io)?;
    let reach = if ex.production_reachable {
        "production-reachable"
    } else if ex.non_production_only {
        "non-production only (dev/build/test)"
    } else {
        "not reachable from any root"
    };
    writeln!(out, "  reachability: {reach}").map_err(PrismError::Io)?;
    match &ex.nearest_owner {
        Some(o) => writeln!(
            out,
            "  nearest first-party owner: {} (distance {})",
            o.purl, ex.owner_distance
        )
        .map_err(PrismError::Io)?,
        None => writeln!(out, "  nearest first-party owner: (none)").map_err(PrismError::Io)?,
    }
    writeln!(out, "  introducers ({}):", ex.introducers.len()).map_err(PrismError::Io)?;
    for i in &ex.introducers {
        writeln!(out, "    {} [{}]", i.node.purl, i.kinds.join(", ")).map_err(PrismError::Io)?;
    }
    writeln!(out, "  paths from a root ({}):", ex.paths.len()).map_err(PrismError::Io)?;
    for p in &ex.paths {
        let chain: Vec<&str> = p.iter().map(|n| n.purl.as_str()).collect();
        writeln!(out, "    {}", chain.join(" → ")).map_err(PrismError::Io)?;
    }
    Ok(())
}

/// `prism graph cycles --run <id>` — classified dependency cycles.
pub fn graph_cycles(out: &mut dyn Write, run: u64) -> Result<()> {
    let prism = open_here()?;
    let cycles = prism.classified_cycles(RunId::new(run))?;
    if cycles.is_empty() {
        writeln!(out, "no dependency cycles in run {run}").map_err(PrismError::Io)?;
        return Ok(());
    }
    writeln!(out, "{} cycle(s) in run {run}:", cycles.len()).map_err(PrismError::Io)?;
    for (i, c) in cycles.iter().enumerate() {
        writeln!(
            out,
            "  [{}] {} members, {} internal edges — {} [{}]",
            i + 1,
            c.members.len(),
            c.internal_edges,
            c.class,
            c.kinds.join(", ")
        )
        .map_err(PrismError::Io)?;
        writeln!(
            out,
            "      runtime-relevant: {}  first-party: {}  external dependents: {}",
            c.runtime_relevant, c.touches_root, c.external_dependents
        )
        .map_err(PrismError::Io)?;
        let names: Vec<&str> = c.members.iter().map(|m| m.purl.as_str()).collect();
        writeln!(out, "      members: {}", names.join(" ⇄ ")).map_err(PrismError::Io)?;
        writeln!(out, "      {}", c.advice).map_err(PrismError::Io)?;
    }
    Ok(())
}

/// `prism graph dup-causes --run <id>` — duplicate-version root-cause analysis:
/// who pins each version, the family's blast radius, and a convergence verdict.
pub fn graph_dup_causes(out: &mut dyn Write, run: u64) -> Result<()> {
    let prism = open_here()?;
    let causes = prism.duplicate_root_causes(RunId::new(run))?;
    if causes.is_empty() {
        writeln!(out, "no duplicate-version families in run {run}").map_err(PrismError::Io)?;
        return Ok(());
    }
    writeln!(
        out,
        "{} duplicate-version famil{} in run {run} (by risk):",
        causes.len(),
        if causes.len() == 1 { "y" } else { "ies" }
    )
    .map_err(PrismError::Io)?;
    for c in &causes {
        writeln!(
            out,
            "  {} — {} versions, {} occurrences, blast {}, {} — risk {}",
            c.package_purl,
            c.versions.len(),
            c.occurrence_count,
            c.blast_radius,
            c.convergence,
            c.risk_score
        )
        .map_err(PrismError::Io)?;
        for v in &c.versions {
            writeln!(
                out,
                "      {} ({}×) ← {}",
                version_tail(&v.purl),
                v.occurrences,
                introducer_brief(&v.introducers)
            )
            .map_err(PrismError::Io)?;
        }
    }
    Ok(())
}

/// `prism graph fidelity --run <id>` — per-metric fidelity notes.
pub fn graph_fidelity(out: &mut dyn Write, run: u64) -> Result<()> {
    let prism = open_here()?;
    let f = prism.fidelity_notes(RunId::new(run))?;
    writeln!(out, "fidelity for run {run} ({}, {}):", f.ecosystem, f.tier).map_err(PrismError::Io)?;
    for note in &f.notes {
        writeln!(out, "  [{}] {}", note.topic, note.note).map_err(PrismError::Io)?;
    }
    Ok(())
}

/// `prism graph subsystems --run <id>` — top-level subsystem partition.
pub fn graph_subsystems(out: &mut dyn Write, run: u64) -> Result<()> {
    let prism = open_here()?;
    let subs = prism.subsystems(RunId::new(run))?;
    if subs.is_empty() {
        writeln!(out, "no top-level subsystems in run {run}").map_err(PrismError::Io)?;
        return Ok(());
    }
    writeln!(out, "{} subsystem(s) in run {run} (by size):", subs.len()).map_err(PrismError::Io)?;
    for s in &subs {
        let scope = if s.runtime { "runtime" } else { "non-runtime" };
        writeln!(
            out,
            "  {:>5} pkgs ({:>4} exclusive) [{scope}]  {}",
            s.size, s.exclusive, s.owner_purl
        )
        .map_err(PrismError::Io)?;
    }
    Ok(())
}

/// `prism regions` — every run as a comparable dependency region.
pub fn regions(out: &mut dyn Write) -> Result<()> {
    let prism = open_here()?;
    let regions = prism.workspace_regions()?;
    if regions.is_empty() {
        writeln!(out, "no runs in this workspace").map_err(PrismError::Io)?;
        return Ok(());
    }
    writeln!(out, "{} dependency region(s):", regions.len()).map_err(PrismError::Io)?;
    writeln!(
        out,
        "  {:>5} {:<7} {:>6} {:>6} {:>8} {:>7} {:>5} {:>5}",
        "run", "eco", "nodes", "edges", "runtime", "cycles", "dups", "risk"
    )
    .map_err(PrismError::Io)?;
    for r in &regions {
        writeln!(
            out,
            "  {:>5} {:<7} {:>6} {:>6} {:>8} {:>7} {:>5} {:>3}/{}",
            r.run.get(),
            r.ecosystem,
            r.nodes,
            r.edges,
            r.runtime_footprint,
            r.cycles,
            r.duplicate_families,
            r.risk_score,
            r.risk_band
        )
        .map_err(PrismError::Io)?;
    }
    Ok(())
}

/// `prism graph risk --run <id>` — structural risk profile.
pub fn graph_risk(out: &mut dyn Write, run: u64) -> Result<()> {
    let prism = open_here()?;
    let r = prism.structural_risk(RunId::new(run))?;
    writeln!(out, "structural risk for run {run}: {} / 100 ({})", r.overall, r.band)
        .map_err(PrismError::Io)?;
    for d in &r.dimensions {
        writeln!(out, "  {:<18} {:>3}  {}", d.name, d.score, d.detail).map_err(PrismError::Io)?;
    }
    Ok(())
}

/// `prism graph fragmentation --run <id>` — connected-component breakdown.
pub fn graph_fragmentation(out: &mut dyn Write, run: u64) -> Result<()> {
    let prism = open_here()?;
    let f = prism.fragmentation(RunId::new(run))?;
    writeln!(out, "fragmentation for run {run}:").map_err(PrismError::Io)?;
    writeln!(out, "  {}", f.note).map_err(PrismError::Io)?;
    if !f.subsystem_islands.is_empty() {
        writeln!(out, "  detached subsystems:").map_err(PrismError::Io)?;
        for island in &f.subsystem_islands {
            let tag = if island.detached_from_roots { "no root" } else { "has root" };
            let sample: Vec<&str> = island.members.iter().map(|s| s.as_str()).collect();
            writeln!(
                out,
                "    {} node(s) [{tag}]: {}",
                island.size,
                sample.join(", ")
            )
            .map_err(PrismError::Io)?;
        }
    }
    Ok(())
}

/// `prism graph diff --base <id> --head <id>` — interpretive cross-run diff.
pub fn graph_diff(out: &mut dyn Write, base: u64, head: u64) -> Result<()> {
    let prism = open_here()?;
    let d = prism.run_diff(RunId::new(base), RunId::new(head))?;
    writeln!(out, "diff run {base} → run {head}").map_err(PrismError::Io)?;
    writeln!(
        out,
        "  nodes {:+}  edges {:+}  components {:+}  duplicate-families {:+}",
        d.nodes_delta, d.edges_delta, d.scc_delta, d.duplicate_families_delta
    )
    .map_err(PrismError::Io)?;
    writeln!(
        out,
        "  packages: +{} / -{}    versions: +{} / -{}",
        d.added_packages.len(),
        d.removed_packages.len(),
        d.added_versions.len(),
        d.removed_versions.len()
    )
    .map_err(PrismError::Io)?;

    if !d.scope_shifts.is_empty() {
        writeln!(out, "  scope shifts:").map_err(PrismError::Io)?;
        for s in &d.scope_shifts {
            writeln!(
                out,
                "    {:<10} footprint {:+}  exclusive {:+}",
                s.kind, s.footprint_delta, s.exclusive_delta
            )
            .map_err(PrismError::Io)?;
        }
    }
    diff_section(out, "new duplicate families", &d.new_duplicate_families)?;
    diff_section(out, "resolved duplicate families", &d.resolved_duplicate_families)?;
    if !d.new_cycles.is_empty() {
        writeln!(out, "  new cycles ({}):", d.new_cycles.len()).map_err(PrismError::Io)?;
        for c in &d.new_cycles {
            writeln!(out, "    [{}] {}", c.class, c.members.join(" ⇄ ")).map_err(PrismError::Io)?;
        }
    }
    if !d.resolved_cycles.is_empty() {
        writeln!(out, "  resolved cycles ({}):", d.resolved_cycles.len()).map_err(PrismError::Io)?;
        for c in &d.resolved_cycles {
            writeln!(out, "    [{}] {}", c.class, c.members.join(" ⇄ ")).map_err(PrismError::Io)?;
        }
    }
    if !d.new_hotspots.is_empty() {
        writeln!(out, "  new/grown hotspots ({}):", d.new_hotspots.len()).map_err(PrismError::Io)?;
        for h in &d.new_hotspots {
            writeln!(
                out,
                "    {} dominates {} (was {})",
                h.package_purl, h.head_dominated, h.base_dominated
            )
            .map_err(PrismError::Io)?;
        }
    }
    Ok(())
}

/// Print a labelled list section, capped to keep output readable.
fn diff_section(out: &mut dyn Write, label: &str, items: &[String]) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    writeln!(out, "  {label} ({}):", items.len()).map_err(PrismError::Io)?;
    for item in items.iter().take(20) {
        writeln!(out, "    {item}").map_err(PrismError::Io)?;
    }
    if items.len() > 20 {
        writeln!(out, "    … and {} more", items.len() - 20).map_err(PrismError::Io)?;
    }
    Ok(())
}

/// The version tail of a purl for compact display (`…@1.2.3` → `1.2.3`).
fn version_tail(purl: &str) -> &str {
    purl.rsplit_once('@').map(|(_, v)| v).unwrap_or(purl)
}

/// A short "constraint←who" summary of the first few introducers.
fn introducer_brief(introducers: &[prism_api::ConstraintIntroducer]) -> String {
    if introducers.is_empty() {
        return "(roots / no recorded requirement)".to_string();
    }
    let shown: Vec<String> = introducers
        .iter()
        .take(3)
        .map(|i| {
            let who = i.introducer_purl.rsplit('/').next().unwrap_or(&i.introducer_purl);
            if i.constraint.is_empty() {
                who.to_string()
            } else {
                format!("{who} wants {}", i.constraint)
            }
        })
        .collect();
    let more = introducers.len().saturating_sub(3);
    if more > 0 {
        format!("{} (+{more} more)", shown.join("; "))
    } else {
        shown.join("; ")
    }
}

/// Map an out-of-range node/pair id (a user-supplied id that is not in the run)
/// to `NotFound` (exit 3), so a plain bad-id typo is distinguishable from a
/// corrupt-store decode bounds failure (which keeps `OutOfBounds`, exit 5).
fn id_not_found(e: PrismError) -> PrismError {
    match e {
        PrismError::OutOfBounds { .. } => PrismError::NotFound,
        other => other,
    }
}

/// `prism graph neighborhood --run <id> --node <id> --depth <n>`.
pub fn graph_neighborhood(out: &mut dyn Write, run: u64, node: u32, depth: u32) -> Result<()> {
    let prism = open_here()?;
    let view = prism
        .neighborhood(RunId::new(run), GraphNodeId::new(node), depth, 1024)
        .map_err(id_not_found)?;
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
    let view = prism
        .impact(RunId::new(run), GraphNodeId::new(node), u32::MAX)
        .map_err(id_not_found)?;
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

/// `prism candidates --requirement <id>` or
/// `--package <purl> --constraint <expr>` — show candidate ranges + exclusions.
pub fn candidates(
    out: &mut dyn Write,
    requirement: Option<u64>,
    package: Option<&str>,
    constraint: Option<&str>,
) -> Result<()> {
    let prism = open_here()?;
    let view = match (requirement, package, constraint) {
        (Some(id), _, _) => prism.candidates(RequirementId::new(id))?,
        (None, Some(purl), Some(expr)) => {
            let (pkg, eco) = prism.package_by_purl(purl)?;
            prism.candidates_for(pkg, expr, eco)?
        }
        // The arg parser guarantees one of the two forms; be defensive anyway.
        _ => {
            return Err(PrismError::Other(
                "candidates needs --requirement or --package + --constraint".into(),
            ))
        }
    };

    writeln!(out, "package {}", view.package_purl).map_err(PrismError::Io)?;
    writeln!(out, "constraint: {}", view.constraint).map_err(PrismError::Io)?;
    writeln!(
        out,
        "{} of {} version(s) satisfy",
        view.count(),
        view.total_versions
    )
    .map_err(PrismError::Io)?;
    for c in &view.candidates {
        writeln!(out, "  [{}] {}", c.ordinal, c.purl).map_err(PrismError::Io)?;
    }
    if !view.ranges.is_empty() {
        let ranges: Vec<String> = view
            .ranges
            .iter()
            .map(|r| format!("[{}, {})", r.start, r.end))
            .collect();
        writeln!(out, "ranges: {}", ranges.join(" ")).map_err(PrismError::Io)?;
    }
    for e in &view.exclusions {
        writeln!(
            out,
            "  excluded [{}] version {} ({:?})",
            e.ordinal,
            e.version.get(),
            e.reason
        )
        .map_err(PrismError::Io)?;
    }
    Ok(())
}

/// `prism conflicts --package <purl>` — same-package conflict pressure report.
pub fn conflicts(out: &mut dyn Write, package: &str) -> Result<()> {
    let prism = open_here()?;
    let (pkg, eco) = prism.package_by_purl(package)?;
    let report = prism.conflict_pressure(pkg, eco)?;

    writeln!(out, "package {}", report.package_purl).map_err(PrismError::Io)?;
    writeln!(
        out,
        "{} requirement(s) over {} version(s)",
        report.requirement_count(),
        report.total_versions
    )
    .map_err(PrismError::Io)?;
    writeln!(out, "tightness: {:.3}", report.tightness).map_err(PrismError::Io)?;
    for (rid, size) in &report.set_sizes {
        writeln!(out, "  requirement {} -> {} candidate(s)", rid.get(), size)
            .map_err(PrismError::Io)?;
    }
    if report.has_conflict() {
        writeln!(out, "{} conflicting pair(s):", report.conflicts.len())
            .map_err(PrismError::Io)?;
        for pair in &report.conflicts {
            writeln!(out, "  {} <-> {}", pair.a.get(), pair.b.get()).map_err(PrismError::Io)?;
        }
    } else {
        writeln!(out, "no conflicts").map_err(PrismError::Io)?;
    }
    Ok(())
}

/// `prism resolve <manifest-path> --policy <lowest|highest|locked>` — ingest the
/// manifest and run the synthetic resolver, printing the run id + node/edge counts.
pub fn resolve(out: &mut dyn Write, manifest: &str, policy: &str) -> Result<()> {
    let mut prism = open_here()?;
    let res = prism.resolve_manifest(manifest, policy)?;
    writeln!(
        out,
        "resolved ({policy}): run {} with {} nodes, {} edges",
        res.run.get(),
        res.nodes,
        res.edges
    )
    .map_err(PrismError::Io)?;
    if res.truncated {
        writeln!(out, "(truncated: a resolution budget was reached)").map_err(PrismError::Io)?;
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
    // One snapshot load for the whole export (was O(N x file size)).
    let (nodes, edges) = prism.graph_export(run)?;

    let mut w = JsonWriter::new();
    w.begin_object();
    w.field_u64("run", run.get());
    w.field_u64("nodes", nodes.len() as u64);
    w.field_u64("edges", edges.len() as u64);

    // Nodes, resolved for display (id is snapshot-local; purl is the stable key).
    w.key("graph_nodes");
    w.begin_array();
    for node in &nodes {
        w.begin_object();
        w.field_u64("id", node.id.get() as u64);
        w.field_str("purl", &node.purl);
        w.field_u64("version_id", node.version.get());
        w.field_u64("depth", node.depth as u64);
        w.end_object();
    }
    w.end_array();

    w.key("graph_edges");
    w.begin_array();
    for e in &edges {
        w.begin_object();
        w.field_u64("source", e.source.get() as u64);
        w.field_u64("target", e.target.get() as u64);
        w.field_u64("kind", e.kind as u64);
        w.field_u64("flags", e.flags as u64);
        w.end_object();
    }
    w.end_array();
    w.end_object();

    writeln!(out, "{}", w.finish()).map_err(PrismError::Io)?;
    Ok(())
}

/// `prism export graphml --run <id>` — emit the run's graph as GraphML to `out`.
///
/// GraphML is a standard XML graph-interchange format. Nodes carry their PURL (the
/// portable key; the GraphML element id `n<i>` is the snapshot-local node id and is
/// *not* portable across workspaces). The run's [`prism_model::GraphFidelity`]
/// (tier + caveats) is carried as graph-level `<data>` so an export never
/// overclaims correctness (architecture doc: every export header includes the
/// fidelity).
pub fn export_graphml(out: &mut dyn Write, run: u64) -> Result<()> {
    let prism = open_here()?;
    let run_id = RunId::new(run);
    let fidelity = prism.store().get_run(run_id)?.fidelity;
    let (tier, caveats) = fidelity_strings(&fidelity);

    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\">\n");
    // Typed key definitions (GraphML requires keys declared before use).
    for (id, dom, ty) in [
        ("run", "graph", "long"),
        ("fidelity_tier", "graph", "string"),
        ("fidelity_caveats", "graph", "string"),
        ("purl", "node", "string"),
        ("version_id", "node", "long"),
        ("depth", "node", "int"),
        ("kind", "edge", "int"),
        ("flags", "edge", "int"),
    ] {
        s.push_str(&format!(
            "  <key id=\"{id}\" for=\"{dom}\" attr.name=\"{id}\" attr.type=\"{ty}\"/>\n"
        ));
    }
    s.push_str(&format!("  <graph id=\"run_{run}\" edgedefault=\"directed\">\n"));
    s.push_str(&format!("    <data key=\"run\">{run}</data>\n"));
    s.push_str(&format!(
        "    <data key=\"fidelity_tier\">{}</data>\n",
        xml_escape(&tier)
    ));
    s.push_str(&format!(
        "    <data key=\"fidelity_caveats\">{}</data>\n",
        xml_escape(&caveats)
    ));

    // One snapshot load for nodes + edges.
    let (nodes, edges) = prism.graph_export(run_id)?;

    // Nodes (PURL is the stable key; the element id is snapshot-local).
    for node in &nodes {
        s.push_str(&format!("    <node id=\"n{}\">\n", node.id.get()));
        s.push_str(&format!(
            "      <data key=\"purl\">{}</data>\n",
            xml_escape(&node.purl)
        ));
        s.push_str(&format!(
            "      <data key=\"version_id\">{}</data>\n",
            node.version.get()
        ));
        s.push_str(&format!("      <data key=\"depth\">{}</data>\n", node.depth));
        s.push_str("    </node>\n");
    }

    // Edges: each directed edge once, sourced at its node (mirrors export_json).
    for (eid, e) in edges.iter().enumerate() {
        s.push_str(&format!(
            "    <edge id=\"e{eid}\" source=\"n{}\" target=\"n{}\">\n",
            e.source.get(),
            e.target.get()
        ));
        s.push_str(&format!("      <data key=\"kind\">{}</data>\n", e.kind as u64));
        s.push_str(&format!(
            "      <data key=\"flags\">{}</data>\n",
            e.flags as u64
        ));
        s.push_str("    </edge>\n");
    }
    s.push_str("  </graph>\n");
    s.push_str("</graphml>\n");

    write!(out, "{s}").map_err(PrismError::Io)?;
    Ok(())
}

/// Escape XML text/attribute content so the output is always well-formed XML 1.0.
///
/// Beyond the five predefined entities, this drops C0 control characters that are
/// illegal in XML 1.0 even as numeric refs (everything below U+0020 except tab,
/// LF, CR). A crafted package name with an embedded control char (reachable via a
/// hand-edited lockfile) would otherwise produce malformed GraphML.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c), // the only XML-legal C0 chars
            c if (c as u32) < 0x20 => out.push('\u{FFFD}'), // illegal C0: replace
            c => out.push(c),
        }
    }
    out
}

/// Render a [`prism_model::GraphFidelity`] as `(tier, caveats)` display strings.
/// `caveats` is a comma-separated list of set caveat names, or `"none"`.
fn fidelity_strings(f: &prism_model::GraphFidelity) -> (String, String) {
    use prism_model::{FidelityCaveats, FidelityTier};
    let tier = match f.tier {
        FidelityTier::LockfileExact => "lockfile-exact",
        FidelityTier::NativeToolResolved => "native-tool-resolved",
        FidelityTier::ModelResolved => "model-resolved",
        FidelityTier::CandidateSpaceOnly => "candidate-space-only",
    }
    .to_string();
    let names: [(FidelityCaveats, &str); 8] = [
        (FidelityCaveats::NO_PEER_PLACEMENT, "no-peer-placement"),
        (FidelityCaveats::NO_FEATURE_UNIFICATION, "no-feature-unification"),
        (FidelityCaveats::NO_PROFILE_EXPANSION, "no-profile-expansion"),
        (FidelityCaveats::NO_MEDIATION, "no-mediation"),
        (FidelityCaveats::NO_EXTRA_ACTIVATION, "no-extra-activation"),
        (FidelityCaveats::MARKERS_APPROXIMATED, "markers-approximated"),
        (FidelityCaveats::DYNAMIC_METADATA_SKIPPED, "dynamic-metadata-skipped"),
        (FidelityCaveats::BUDGET_TRUNCATED, "budget-truncated"),
    ];
    let set: Vec<&str> = names
        .iter()
        .filter(|(flag, _)| f.caveats.contains(*flag))
        .map(|(_, n)| *n)
        .collect();
    let caveats = if set.is_empty() {
        "none".to_string()
    } else {
        set.join(",")
    };
    (tier, caveats)
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

/// A short ecosystem label for display, recovered from the stored id. (npm and
/// Bun share one id; uv is the PyPI id, shown as `pypi`.)
fn ecosystem_label(eco: prism_model::EcosystemId) -> &'static str {
    if eco == Ecosystem::Npm.id() {
        "npm"
    } else if eco == Ecosystem::Cargo.id() {
        "cargo"
    } else if eco == Ecosystem::Uv.id() {
        "pypi"
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
    fn export_graphml_emits_wellformed_xml() {
        let _guard = crate::testsupport::cwd_lock();
        let td = TempDir::new("cmd-export-graphml");
        std::fs::write(td.path().join("Cargo.lock"), CARGO_LOCK).unwrap();
        let run = {
            let mut prism = Prism::init(td.path()).unwrap();
            prism.ingest_lockfile(td.path().join("Cargo.lock")).unwrap()
        };
        with_cwd(td.path(), || {
            let mut out = Vec::new();
            export_graphml(&mut out, run.get()).unwrap();
            let xml = String::from_utf8(out).unwrap();
            // Structural well-formedness: header, single graph, balanced root.
            assert!(xml.starts_with("<?xml version=\"1.0\""), "{xml}");
            assert!(xml.contains("<graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\">"));
            assert!(xml.contains("edgedefault=\"directed\""));
            assert_eq!(xml.matches("</graphml>").count(), 1);
            // Fidelity header is present (a lockfile run is LockfileExact).
            assert!(xml.contains("<data key=\"fidelity_tier\">lockfile-exact</data>"), "{xml}");
            // Two nodes, one edge, PURL-keyed.
            assert_eq!(xml.matches("<node ").count(), 2, "{xml}");
            assert_eq!(xml.matches("<edge ").count(), 1, "{xml}");
            assert!(xml.contains("pkg:cargo/app@0.1.0"), "{xml}");
            // Every opened node/edge tag is closed.
            assert_eq!(xml.matches("<node ").count(), xml.matches("</node>").count());
            assert_eq!(xml.matches("<edge ").count(), xml.matches("</edge>").count());
        });
    }

    #[test]
    fn xml_escape_handles_specials() {
        assert_eq!(xml_escape("a&b<c>d\"e'f"), "a&amp;b&lt;c&gt;d&quot;e&apos;f");
        assert_eq!(xml_escape("plain"), "plain");
    }

    #[test]
    fn xml_escape_drops_illegal_c0_controls() {
        // U+0001 is illegal in XML 1.0 even as a numeric ref; it must be replaced,
        // never passed through (which would make GraphML non-well-formed).
        let escaped = xml_escape("a\u{1}b");
        assert!(!escaped.contains('\u{1}'));
        assert_eq!(escaped, "a\u{FFFD}b");
        // Tab/newline/CR are the only legal C0 chars and pass through.
        assert_eq!(xml_escape("a\tb\nc\rd"), "a\tb\nc\rd");
    }

    /// A Cargo.lock with two serde versions so the candidate/conflict path has a
    /// non-trivial version space.
    const TWO_SERDE_LOCK: &str = r#"
# @generated by Cargo
version = 3

[[package]]
name = "app"
version = "0.1.0"
dependencies = [
 "serde 1.0.0",
]

[[package]]
name = "serde"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "serde"
version = "2.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

    #[test]
    fn candidates_conflicts_and_resolve_handlers() {
        let _guard = crate::testsupport::cwd_lock();
        let td = TempDir::new("cmd-candidates");
        // Seed two serde versions via a lockfile.
        std::fs::write(td.path().join("Cargo.lock"), TWO_SERDE_LOCK).unwrap();
        // A manifest declaring serde ^2 (drives requirements + resolution).
        std::fs::write(
            td.path().join("Cargo.toml"),
            "[package]\nname = \"root\"\nversion = \"1.0.0\"\n\n[dependencies]\nserde = \"^2\"\n",
        )
        .unwrap();
        {
            let mut prism = Prism::init(td.path()).unwrap();
            prism.ingest_lockfile(td.path().join("Cargo.lock")).unwrap();
        }

        with_cwd(td.path(), || {
            // candidates by package + constraint (no requirement needed).
            let mut out = Vec::new();
            candidates(&mut out, None, Some("pkg:cargo/serde"), Some("^2")).unwrap();
            let s = String::from_utf8(out).unwrap();
            assert!(s.contains("1 of 2 version(s) satisfy"), "{s}");
            assert!(s.contains("pkg:cargo/serde@2.0.0"), "{s}");

            // resolve the manifest under the highest policy (also ingests the
            // manifest's requirements exactly once).
            let mut out = Vec::new();
            resolve(&mut out, "Cargo.toml", "highest").unwrap();
            let s = String::from_utf8(out).unwrap();
            assert!(s.contains("resolved (highest): run"), "{s}");
            assert!(s.contains("2 nodes"), "{s}");
            assert!(s.contains("1 edges"), "{s}");

            // conflicts: the lone serde ^2 requirement -> no conflict.
            let mut out = Vec::new();
            conflicts(&mut out, "pkg:cargo/serde").unwrap();
            let s = String::from_utf8(out).unwrap();
            assert!(s.contains("requirement(s) over 2 version(s)"), "{s}");
            assert!(s.contains("no conflicts"), "{s}");
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
