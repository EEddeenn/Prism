//! Serializing `prism-api` view structs to JSON via the in-crate [`JsonWriter`].
//!
//! Each function writes one *value* (object or array) into a writer already
//! positioned at a value slot. Field order is fixed for deterministic output;
//! snapshot-local node ids are included but, like the CLI exporter, the purl is
//! the portable key.

use prism_api::{
    BarcodeView, BudgetReport, CandidateView, ChokepointView, CycleView, DiamondReuseView,
    DuplicateClusterView, DuplicateRootCauseView, ExplanationView, FidelityReport,
    FiltrationSummary, FragmentationView, GraphEdgeView, GraphNodeView, GraphStatsView, H2View,
    MinimalView, NeighborhoodView, OverlapView, OverlayReportView, PairView, RiskProfileView,
    RunDiffView, SccView, ScopeSummaryView, ShapeSignature, Subsystem, SupportObject, SupportView,
    TopologyDistanceView, WorkspaceRegion,
};
use prism_model::{CandidateReason, ExclusionReason};

use crate::json::JsonWriter;

/// Write a [`GraphNodeView`] as a JSON object.
pub fn node(w: &mut JsonWriter, n: &GraphNodeView) {
    w.begin_object();
    w.field_u64("id", n.id.get() as u64);
    w.field_str("purl", &n.purl);
    w.field_u64("version_id", n.version.get());
    if let Some(pkg) = n.package {
        w.field_u64("package_id", pkg.get());
    }
    w.field_u64("depth", n.depth as u64);
    w.end_object();
}

/// Write a [`GraphEdgeView`] as a JSON object.
pub fn edge(w: &mut JsonWriter, e: &GraphEdgeView) {
    w.begin_object();
    w.field_u64("source", e.source.get() as u64);
    w.field_u64("target", e.target.get() as u64);
    w.field_u64("kind", e.kind as u64);
    w.field_u64("flags", e.flags as u64);
    w.end_object();
}

/// Write a [`GraphStatsView`] as a JSON object.
pub fn stats(w: &mut JsonWriter, s: &GraphStatsView) {
    w.begin_object();
    w.field_u64("run", s.run.get());
    w.field_u64("nodes", s.nodes as u64);
    w.field_u64("edges", s.edges as u64);
    w.field_u64("scc_count", s.scc_count as u64);
    w.field_bool("has_cycle", s.has_cycle);
    w.field_u64("max_depth", s.max_depth as u64);
    w.field_u64("duplicate_version_count", s.duplicate_version_count as u64);
    w.end_object();
}

/// Write a [`NeighborhoodView`] (center + nodes + edges + truncated) as an object.
pub fn neighborhood(w: &mut JsonWriter, v: &NeighborhoodView) {
    w.begin_object();
    w.key("center");
    node(w, &v.center);
    w.key("nodes");
    w.begin_array();
    for n in &v.nodes {
        node(w, n);
    }
    w.end_array();
    w.key("edges");
    w.begin_array();
    for e in &v.edges {
        edge(w, e);
    }
    w.end_array();
    w.field_bool("truncated", v.truncated);
    w.end_object();
}

/// Write a list of [`SccView`]s as a JSON array of `{members:[...]}` objects.
pub fn scc_list(w: &mut JsonWriter, comps: &[SccView]) {
    w.begin_array();
    for c in comps {
        w.begin_object();
        w.field_bool("is_cycle", c.is_cycle());
        w.key("members");
        w.begin_array();
        for m in &c.members {
            node(w, m);
        }
        w.end_array();
        w.end_object();
    }
    w.end_array();
}

/// Write a list of [`DuplicateClusterView`]s as a JSON array.
pub fn duplicates(w: &mut JsonWriter, clusters: &[DuplicateClusterView]) {
    w.begin_array();
    for c in clusters {
        w.begin_object();
        w.field_u64("package_id", c.package.get());
        w.field_str("package_purl", &c.package_purl);
        w.field_u64("version_count", c.versions.len() as u64);
        w.field_u64("occurrence_count", c.occurrence_count as u64);
        w.field_u64("min_depth", c.min_depth as u64);
        w.field_u64("max_depth", c.max_depth as u64);
        w.key("versions");
        w.begin_array();
        for v in &c.versions {
            w.begin_object();
            w.field_u64("version_id", v.version.get());
            w.field_str("purl", &v.purl);
            w.end_object();
        }
        w.end_array();
        w.end_object();
    }
    w.end_array();
}

/// Write a [`BarcodeView`] as a JSON object with per-dimension counts.
pub fn barcode(w: &mut JsonWriter, b: &BarcodeView) {
    w.begin_object();
    w.field_u64("run", b.run.get());
    w.field_u64("filtration", b.filtration.get());
    w.field_u64("component_count", b.component_count() as u64);
    w.field_u64("cycle_count", b.cycle_count() as u64);
    w.key("dims");
    w.begin_array();
    for d in &b.dims {
        w.begin_object();
        w.field_u64("dim", d.dim as u64);
        w.field_u64("finite", d.finite as u64);
        w.field_u64("essential", d.essential as u64);
        w.field_u64("total", d.total() as u64);
        w.end_object();
    }
    w.end_array();
    w.end_object();
}

/// Write a single [`PairView`] as a JSON object. `death`/`persistence` are
/// `null` for an essential class (one that never dies).
pub fn pair(w: &mut JsonWriter, p: &PairView) {
    w.begin_object();
    w.field_u64("pair", p.pair.get());
    w.field_u64("dim", p.dim as u64);
    w.key("birth");
    w.f64_value(p.birth);
    w.key("death");
    match p.death {
        Some(d) => w.f64_value(d),
        None => w.null_value(),
    }
    w.field_bool("essential", p.is_essential());
    w.key("persistence");
    match p.persistence() {
        Some(d) => w.f64_value(d),
        None => w.null_value(),
    }
    w.end_object();
}

/// Write the list of persistence pairs for a filtration as a JSON object
/// `{run, filtration, pairs:[...]}`.
pub fn pairs(w: &mut JsonWriter, run: prism_model::RunId, filtration: prism_model::FiltrationId, ps: &[PairView]) {
    w.begin_object();
    w.field_u64("run", run.get());
    w.field_u64("filtration", filtration.get());
    w.field_u64("count", ps.len() as u64);
    w.key("pairs");
    w.begin_array();
    for p in ps {
        pair(w, p);
    }
    w.end_array();
    w.end_object();
}

/// Write a [`SupportView`] — a pair's generator chain resolved to graph objects.
///
/// Each object is tagged: a `"node"` carries the full node view; an `"edge"`
/// carries its snapshot-local id and endpoints so the UI can light up the
/// concrete dependency arc that the topological feature rides on.
pub fn support(w: &mut JsonWriter, v: &SupportView) {
    w.begin_object();
    w.field_u64("pair", v.pair.get());
    w.field_u64("dim", v.dim as u64);
    w.field_u64("object_count", v.objects.len() as u64);
    w.key("objects");
    w.begin_array();
    for obj in &v.objects {
        match obj {
            SupportObject::Node(n) => {
                w.begin_object();
                w.field_str("type", "node");
                w.field_u64("id", n.id.get() as u64);
                w.field_str("purl", &n.purl);
                w.field_u64("version_id", n.version.get());
                if let Some(pkg) = n.package {
                    w.field_u64("package_id", pkg.get());
                }
                w.field_u64("depth", n.depth as u64);
                w.end_object();
            }
            SupportObject::Edge { id, source, target } => {
                w.begin_object();
                w.field_str("type", "edge");
                w.field_u64("id", id.get() as u64);
                w.field_u64("source", source.get() as u64);
                w.field_u64("target", target.get() as u64);
                w.end_object();
            }
        }
    }
    w.end_array();
    w.end_object();
}

/// Display name for a [`CandidateReason`].
fn candidate_reason_name(r: CandidateReason) -> &'static str {
    match r {
        CandidateReason::Constraint => "constraint",
        CandidateReason::Locked => "locked",
        CandidateReason::Unsatisfiable => "unsatisfiable",
    }
}

/// Display name for an [`ExclusionReason`].
fn exclusion_reason_name(r: ExclusionReason) -> &'static str {
    match r {
        ExclusionReason::Yanked => "yanked",
        ExclusionReason::RequiresPythonMismatch => "requires-python-mismatch",
        ExclusionReason::NoCompatibleWheel => "no-compatible-wheel",
        ExclusionReason::DynamicMetadataUnknown => "dynamic-metadata-unknown",
        ExclusionReason::Excluded => "excluded",
        ExclusionReason::Retracted => "retracted",
    }
}

/// Write a [`CandidateView`] — a package's versions sorted into ordinals, with
/// the satisfying set, coalesced ranges, and per-version exclusions. The UI uses
/// this to draw a version "headroom" number-line on the node detail.
pub fn candidates(w: &mut JsonWriter, v: &CandidateView) {
    w.begin_object();
    w.field_u64("package_id", v.package.get());
    w.field_str("package_purl", &v.package_purl);
    w.field_str("constraint", &v.constraint);
    w.field_u64("total_versions", v.total_versions as u64);
    w.field_str("reason", candidate_reason_name(v.reason));
    w.field_u64("satisfying_count", v.candidates.len() as u64);

    w.key("candidates");
    w.begin_array();
    for c in &v.candidates {
        w.begin_object();
        w.field_u64("ordinal", c.ordinal as u64);
        w.field_u64("version_id", c.version.get());
        w.field_str("purl", &c.purl);
        w.end_object();
    }
    w.end_array();

    w.key("ranges");
    w.begin_array();
    for r in &v.ranges {
        w.begin_object();
        w.field_u64("start", r.start as u64);
        w.field_u64("end", r.end as u64);
        w.end_object();
    }
    w.end_array();

    w.key("exclusions");
    w.begin_array();
    for e in &v.exclusions {
        w.begin_object();
        w.field_u64("ordinal", e.ordinal as u64);
        w.field_u64("version_id", e.version.get());
        w.field_str("reason", exclusion_reason_name(e.reason));
        w.end_object();
    }
    w.end_array();
    w.end_object();
}

/// Write the per-scope footprint summary as a JSON array.
pub fn scope_summary(w: &mut JsonWriter, rows: &[ScopeSummaryView]) {
    w.begin_array();
    for r in rows {
        w.begin_object();
        w.field_str("kind", r.kind);
        w.field_u64("direct_edges", r.direct_edges as u64);
        w.field_u64("direct_targets", r.direct_targets as u64);
        w.field_u64("footprint", r.footprint as u64);
        w.field_u64("exclusive", r.exclusive as u64);
        w.field_u64("max_depth", r.max_depth as u64);
        w.end_object();
    }
    w.end_array();
}

/// Write the chokepoint ranking as a JSON array.
pub fn chokepoints(w: &mut JsonWriter, cps: &[ChokepointView]) {
    w.begin_array();
    for c in cps {
        w.begin_object();
        w.field_u64("dominated", c.dominated as u64);
        w.key("node");
        node(w, &c.node);
        w.end_object();
    }
    w.end_array();
}

/// Write the classified-cycle list as a JSON array.
pub fn classified_cycles(w: &mut JsonWriter, cycles: &[CycleView]) {
    w.begin_array();
    for c in cycles {
        w.begin_object();
        w.field_str("class", c.class);
        w.field_str("advice", c.advice);
        w.field_bool("runtime_relevant", c.runtime_relevant);
        w.field_bool("touches_root", c.touches_root);
        w.field_u64("internal_edges", c.internal_edges as u64);
        w.field_u64("external_dependents", c.external_dependents as u64);
        w.key("kinds");
        w.begin_array();
        for k in &c.kinds {
            w.str_value(k);
        }
        w.end_array();
        w.key("members");
        w.begin_array();
        for m in &c.members {
            node(w, m);
        }
        w.end_array();
        w.end_object();
    }
    w.end_array();
}

/// Write the structural risk profile as a JSON object.
pub fn risk(w: &mut JsonWriter, r: &RiskProfileView) {
    w.begin_object();
    w.field_u64("run", r.run.get());
    w.field_u64("overall", r.overall as u64);
    w.field_str("band", r.band);
    w.key("dimensions");
    w.begin_array();
    for d in &r.dimensions {
        w.begin_object();
        w.field_str("name", d.name);
        w.field_u64("score", d.score as u64);
        w.field_str("detail", &d.detail);
        w.end_object();
    }
    w.end_array();
    w.end_object();
}

/// Write the fragmentation decomposition as a JSON object.
pub fn fragmentation(w: &mut JsonWriter, f: &FragmentationView) {
    w.begin_object();
    w.field_u64("run", f.run.get());
    w.field_u64("component_count", f.component_count as u64);
    w.field_u64("largest_component", f.largest_component as u64);
    w.field_u64("artifact_singletons", f.artifact_singletons as u64);
    w.field_str("note", &f.note);
    w.key("subsystem_islands");
    w.begin_array();
    for island in &f.subsystem_islands {
        w.begin_object();
        w.field_u64("size", island.size as u64);
        w.field_bool("detached_from_roots", island.detached_from_roots);
        w.key("members");
        w.begin_array();
        for m in &island.members {
            w.str_value(m);
        }
        w.end_array();
        w.end_object();
    }
    w.end_array();
    w.end_object();
}

/// Write a topology shape signature as a JSON object.
pub fn shape(w: &mut JsonWriter, s: &ShapeSignature) {
    w.begin_object();
    w.field_u64("run", s.run.get());
    w.field_u64("nodes", s.nodes as u64);
    w.field_u64("edges", s.edges as u64);
    w.field_u64("max_depth", s.max_depth as u64);
    w.field_u64("components", s.components as u64);
    w.field_u64("cycles", s.cycles as u64);
    w.field_u64("runtime_cycles", s.runtime_cycles as u64);
    w.field_u64("duplicate_families", s.duplicate_families as u64);
    w.field_u64("reconvergence", s.reconvergence as u64);
    w.field_u64("top_chokepoint", s.top_chokepoint as u64);
    w.field_u64("runtime_footprint", s.runtime_footprint as u64);
    w.end_object();
}

/// Write the workspace dependency regions as a JSON array.
pub fn regions(w: &mut JsonWriter, regions: &[WorkspaceRegion]) {
    w.begin_array();
    for r in regions {
        w.begin_object();
        w.field_u64("run", r.run.get());
        w.field_str("ecosystem", r.ecosystem);
        w.field_u64("nodes", r.nodes as u64);
        w.field_u64("edges", r.edges as u64);
        w.field_u64("runtime_footprint", r.runtime_footprint as u64);
        w.field_u64("cycles", r.cycles as u64);
        w.field_u64("duplicate_families", r.duplicate_families as u64);
        w.field_u64("risk_score", r.risk_score as u64);
        w.field_str("risk_band", r.risk_band);
        w.end_object();
    }
    w.end_array();
}

/// Write the experimental H2 (flag-complex) homology as a JSON object.
pub fn h2(w: &mut JsonWriter, h: &H2View) {
    w.begin_object();
    w.field_u64("run", h.run.get());
    w.field_u64("betti0", h.betti0 as u64);
    w.field_u64("betti1", h.betti1 as u64);
    w.field_u64("betti2", h.betti2 as u64);
    w.field_u64("triangles", h.triangles as u64);
    w.key("voids");
    w.begin_array();
    for v in &h.voids {
        w.begin_array();
        for m in v {
            node(w, m);
        }
        w.end_array();
    }
    w.end_array();
    w.end_object();
}

/// Write a "why is this here?" [`ExplanationView`] as a JSON object: the target
/// occurrence, its direct introducers (with the kinds they pull it in by), a few
/// shortest root→target paths, and the nearest first-party owner.
pub fn explanation(w: &mut JsonWriter, v: &ExplanationView) {
    w.begin_object();
    w.key("target");
    node(w, &v.target);
    w.key("introducers");
    w.begin_array();
    for i in &v.introducers {
        w.begin_object();
        w.key("node");
        node(w, &i.node);
        w.key("kinds");
        w.begin_array();
        for k in &i.kinds {
            w.str_value(k);
        }
        w.end_array();
        w.end_object();
    }
    w.end_array();
    w.key("paths");
    w.begin_array();
    for path in &v.paths {
        w.begin_array();
        for n in path {
            node(w, n);
        }
        w.end_array();
    }
    w.end_array();
    w.key("nearest_owner");
    match &v.nearest_owner {
        Some(n) => node(w, n),
        None => w.null_value(),
    }
    w.field_u64("owner_distance", v.owner_distance as u64);
    w.field_bool("production_reachable", v.production_reachable);
    w.field_bool("non_production_only", v.non_production_only);
    w.end_object();
}

/// Write the duplicate-version root-cause report as a JSON array, one object per
/// fragmented family with its versions, the (introducer, constraint) pairs that
/// pin each, and the convergence verdict + severity.
pub fn duplicate_root_causes(w: &mut JsonWriter, views: &[DuplicateRootCauseView]) {
    w.begin_array();
    for v in views {
        w.begin_object();
        w.field_u64("package_id", v.package.get());
        w.field_str("package_purl", &v.package_purl);
        w.key("versions");
        w.begin_array();
        for ver in &v.versions {
            w.begin_object();
            w.field_u64("version_id", ver.version.get());
            w.field_str("purl", &ver.purl);
            w.field_u64("occurrences", ver.occurrences as u64);
            w.key("introducers");
            w.begin_array();
            for ci in &ver.introducers {
                w.begin_object();
                w.field_str("introducer_purl", &ci.introducer_purl);
                w.field_str("constraint", &ci.constraint);
                w.end_object();
            }
            w.end_array();
            w.end_object();
        }
        w.end_array();
        w.field_u64("occurrence_count", v.occurrence_count as u64);
        w.field_u64("blast_radius", v.blast_radius as u64);
        w.field_u64("distinct_constraints", v.distinct_constraints as u64);
        w.field_str("convergence", v.convergence);
        w.field_u64("risk_score", v.risk_score as u64);
        w.end_object();
    }
    w.end_array();
}

/// Write the top-level subsystem partition as a JSON array.
pub fn subsystems(w: &mut JsonWriter, subs: &[Subsystem]) {
    w.begin_array();
    for s in subs {
        w.begin_object();
        w.field_str("owner_purl", &s.owner_purl);
        w.field_u64("size", s.size as u64);
        w.field_u64("exclusive", s.exclusive as u64);
        w.field_bool("runtime", s.runtime);
        w.end_object();
    }
    w.end_array();
}

/// Write the diamond-reuse summary as a JSON object: total reconvergence and the
/// top shared foundations that close the most independent loops.
pub fn diamonds(w: &mut JsonWriter, v: &DiamondReuseView) {
    w.begin_object();
    w.field_u64("run", v.run.get());
    w.field_u64("total_reconvergence", v.total_reconvergence as u64);
    w.field_u64("top_share_pct", v.top_share_pct as u64);
    w.key("foundations");
    w.begin_array();
    for d in &v.foundations {
        w.begin_object();
        w.field_str("purl", &d.purl);
        w.field_u64("parents", d.parents as u64);
        w.field_u64("reconvergence", d.reconvergence as u64);
        w.key("sample_parents");
        w.begin_array();
        for p in &d.sample_parents {
            w.str_value(p);
        }
        w.end_array();
        w.end_object();
    }
    w.end_array();
    w.end_object();
}

/// Write the minimal (dominator-tree) explanation view as a JSON object.
pub fn minimal(w: &mut JsonWriter, v: &MinimalView) {
    w.begin_object();
    w.field_u64("run", v.run.get());
    w.field_u64("original_edges", v.original_edges as u64);
    w.field_u64("reduced_edges", v.reduced_edges as u64);
    w.key("edges");
    w.begin_array();
    for e in &v.edges {
        w.begin_object();
        w.field_str("gateway", &e.gateway);
        w.field_str("node", &e.node);
        w.end_object();
    }
    w.end_array();
    w.end_object();
}

/// Write the topology shape distance between two runs as a JSON object. The
/// signed `change_pct` rides the `f64` writer (it is always integral).
pub fn topology_distance(w: &mut JsonWriter, v: &TopologyDistanceView) {
    w.begin_object();
    w.field_u64("base", v.base.get());
    w.field_u64("head", v.head.get());
    w.field_u64("distance_pct", v.distance_pct as u64);
    w.key("deltas");
    w.begin_array();
    for d in &v.deltas {
        w.begin_object();
        w.field_str("metric", d.metric);
        w.field_u64("base", d.base as u64);
        w.field_u64("head", d.head as u64);
        w.key("change_pct");
        w.f64_value(d.change_pct as f64);
        w.end_object();
    }
    w.end_array();
    w.end_object();
}

/// Write the higher-order overlap report as a JSON object: packages shared by
/// multiple subsystems, and the subsystem pairs with the largest shared surface.
pub fn overlap(w: &mut JsonWriter, v: &OverlapView) {
    w.begin_object();
    w.field_u64("run", v.run.get());
    w.field_u64("subsystems", v.subsystems as u64);
    w.key("shared_surfaces");
    w.begin_array();
    for s in &v.shared_surfaces {
        w.begin_object();
        w.field_str("purl", &s.purl);
        w.field_u64("spread", s.spread as u64);
        w.key("subsystems");
        w.begin_array();
        for owner in &s.subsystems {
            w.str_value(owner);
        }
        w.end_array();
        w.end_object();
    }
    w.end_array();
    w.key("top_pairs");
    w.begin_array();
    for p in &v.top_pairs {
        w.begin_object();
        w.field_str("a", &p.a);
        w.field_str("b", &p.b);
        w.field_u64("shared", p.shared as u64);
        w.end_object();
    }
    w.end_array();
    w.end_object();
}

/// Write per-metric fidelity caveats for a run.
pub fn fidelity(w: &mut JsonWriter, r: &FidelityReport) {
    w.begin_object();
    w.field_u64("run", r.run.get());
    w.field_str("tier", r.tier);
    w.field_str("ecosystem", r.ecosystem);
    w.key("notes");
    w.begin_array();
    for n in &r.notes {
        w.begin_object();
        w.field_str("topic", n.topic);
        w.field_str("note", &n.note);
        w.end_object();
    }
    w.end_array();
    w.end_object();
}

/// Write the cross-run diff as a JSON object: scalar deltas, package/version
/// add/remove lists, duplicate-family churn, cycle churn, per-scope shifts,
/// and the top hotspot shifts. Signed integers ride the `f64` writer (counts
/// never approach 2^53).
pub fn run_diff(w: &mut JsonWriter, d: &RunDiffView) {
    w.begin_object();
    w.field_u64("base", d.base.get());
    w.field_u64("head", d.head.get());
    w.key("nodes_delta");
    w.f64_value(d.nodes_delta as f64);
    w.key("edges_delta");
    w.f64_value(d.edges_delta as f64);
    w.key("scc_delta");
    w.f64_value(d.scc_delta as f64);
    w.key("duplicate_families_delta");
    w.f64_value(d.duplicate_families_delta as f64);
    write_str_array(w, "added_packages", &d.added_packages);
    write_str_array(w, "removed_packages", &d.removed_packages);
    write_str_array(w, "added_versions", &d.added_versions);
    write_str_array(w, "removed_versions", &d.removed_versions);
    write_str_array(w, "new_duplicate_families", &d.new_duplicate_families);
    write_str_array(w, "resolved_duplicate_families", &d.resolved_duplicate_families);
    w.key("new_cycles");
    w.begin_array();
    for c in &d.new_cycles {
        write_cycle_diff_entry(w, c);
    }
    w.end_array();
    w.key("resolved_cycles");
    w.begin_array();
    for c in &d.resolved_cycles {
        write_cycle_diff_entry(w, c);
    }
    w.end_array();
    w.key("scope_shifts");
    w.begin_array();
    for s in &d.scope_shifts {
        w.begin_object();
        w.field_str("kind", s.kind);
        w.key("footprint_delta");
        w.f64_value(s.footprint_delta as f64);
        w.key("exclusive_delta");
        w.f64_value(s.exclusive_delta as f64);
        w.end_object();
    }
    w.end_array();
    w.key("new_hotspots");
    w.begin_array();
    for h in &d.new_hotspots {
        w.begin_object();
        w.field_str("package_purl", &h.package_purl);
        w.field_u64("base_dominated", h.base_dominated as u64);
        w.field_u64("head_dominated", h.head_dominated as u64);
        w.end_object();
    }
    w.end_array();
    w.end_object();
}

fn write_str_array(w: &mut JsonWriter, key: &str, items: &[String]) {
    w.key(key);
    w.begin_array();
    for s in items {
        w.str_value(s);
    }
    w.end_array();
}

fn write_cycle_diff_entry(w: &mut JsonWriter, c: &prism_api::CycleDiffEntry) {
    w.begin_object();
    w.field_u64("size", c.members.len() as u64);
    w.field_str("class", c.class);
    w.field_bool("runtime_relevant", c.runtime_relevant);
    w.key("members");
    w.begin_array();
    for m in &c.members {
        w.str_value(m);
    }
    w.end_array();
    w.end_object();
}

/// Write a risk-overlay join report: per-finding priority, severity, blast
/// radius, and reachability.
pub fn overlay_report(w: &mut JsonWriter, r: &OverlayReportView) {
    w.begin_object();
    w.field_u64("run", r.run.get());
    w.field_u64("entries", r.entries as u64);
    w.field_u64("matched", r.matched as u64);
    w.field_u64("production_findings", r.production_findings as u64);
    w.key("findings");
    w.begin_array();
    for f in &r.findings {
        w.begin_object();
        w.field_str("purl", &f.purl);
        w.field_str("category", &f.category);
        w.field_str("severity", &f.severity);
        w.field_str("note", &f.note);
        w.field_u64("matched_occurrences", f.matched_occurrences as u64);
        w.field_bool("production_reachable", f.production_reachable);
        w.field_u64("blast_radius", f.blast_radius as u64);
        w.field_u64("priority", f.priority as u64);
        w.end_object();
    }
    w.end_array();
    w.end_object();
}

/// Write a CI budget report: pass/fail, rules checked, and each violation.
pub fn budget(w: &mut JsonWriter, r: &BudgetReport) {
    w.begin_object();
    w.field_u64("head", r.head.get());
    w.key("base");
    match r.base {
        Some(b) => w.u64_value(b.get()),
        None => w.null_value(),
    }
    w.field_bool("pass", r.pass);
    w.field_u64("rules_checked", r.rules_checked as u64);
    w.key("violations");
    w.begin_array();
    for v in &r.violations {
        w.begin_object();
        w.field_str("rule", &v.rule);
        w.key("observed");
        w.f64_value(v.observed as f64);
        w.key("limit");
        w.f64_value(v.limit as f64);
        w.end_object();
    }
    w.end_array();
    w.end_object();
}

/// Write the multi-filtration comparison as a JSON array, one row per lens.
pub fn filtration_compare(w: &mut JsonWriter, rows: &[FiltrationSummary]) {
    w.begin_array();
    for r in rows {
        w.begin_object();
        w.field_str("filtration", r.filtration);
        w.field_u64("h0_components", r.h0_components as u64);
        w.field_u64("h1_loops", r.h1_loops as u64);
        w.field_u64("finite_bars", r.finite_bars as u64);
        w.key("total_persistence");
        w.f64_value(r.total_persistence);
        w.key("max_persistence");
        w.f64_value(r.max_persistence);
        w.end_object();
    }
    w.end_array();
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_api::BarcodeDim;
    use prism_model::{FiltrationId, GraphNodeId, PackageId, RunId, VersionId};

    fn sample_node() -> GraphNodeView {
        GraphNodeView {
            id: GraphNodeId::new(2),
            version: VersionId::new(40),
            package: Some(PackageId::new(9)),
            purl: "pkg:cargo/serde@1.0.0".into(),
            depth: 1,
        }
    }

    #[test]
    fn node_object_has_expected_fields() {
        let mut w = JsonWriter::new();
        node(&mut w, &sample_node());
        let out = w.finish();
        assert!(out.contains(r#""id":2"#));
        assert!(out.contains(r#""purl":"pkg:cargo/serde@1.0.0""#));
        assert!(out.contains(r#""version_id":40"#));
        assert!(out.contains(r#""package_id":9"#));
        assert!(out.contains(r#""depth":1"#));
    }

    #[test]
    fn node_omits_absent_package() {
        let mut n = sample_node();
        n.package = None;
        let mut w = JsonWriter::new();
        node(&mut w, &n);
        assert!(!w.finish().contains("package_id"));
    }

    #[test]
    fn stats_object_roundtrips_counts() {
        let s = GraphStatsView {
            run: RunId::new(3),
            nodes: 4,
            edges: 3,
            scc_count: 4,
            has_cycle: false,
            max_depth: 1,
            duplicate_version_count: 0,
        };
        let mut w = JsonWriter::new();
        stats(&mut w, &s);
        let out = w.finish();
        assert!(out.contains(r#""nodes":4"#));
        assert!(out.contains(r#""edges":3"#));
        assert!(out.contains(r#""has_cycle":false"#));
    }

    #[test]
    fn pair_object_finite_and_essential() {
        use prism_model::PairId;
        // Finite pair: death + persistence present.
        let finite = PairView {
            pair: PairId::new(7),
            dim: 1,
            birth: 2.0,
            death: Some(5.0),
        };
        let mut w = JsonWriter::new();
        pair(&mut w, &finite);
        let out = w.finish();
        assert!(out.contains(r#""pair":7"#), "{out}");
        assert!(out.contains(r#""dim":1"#));
        assert!(out.contains(r#""birth":2"#));
        assert!(out.contains(r#""death":5"#));
        assert!(out.contains(r#""essential":false"#));
        assert!(out.contains(r#""persistence":3"#));

        // Essential pair: death + persistence are null.
        let essential = PairView {
            pair: PairId::new(8),
            dim: 0,
            birth: 0.0,
            death: None,
        };
        let mut w = JsonWriter::new();
        pair(&mut w, &essential);
        let out = w.finish();
        assert!(out.contains(r#""death":null"#), "{out}");
        assert!(out.contains(r#""essential":true"#));
        assert!(out.contains(r#""persistence":null"#));
    }

    #[test]
    fn support_object_tags_nodes_and_edges() {
        use prism_model::{GraphEdgeId, GraphNodeId, PairId};
        let v = SupportView {
            pair: PairId::new(3),
            dim: 1,
            objects: vec![
                SupportObject::Node(sample_node()),
                SupportObject::Edge {
                    id: GraphEdgeId::new(4),
                    source: GraphNodeId::new(2),
                    target: GraphNodeId::new(5),
                },
            ],
        };
        let mut w = JsonWriter::new();
        support(&mut w, &v);
        let out = w.finish();
        assert!(out.contains(r#""object_count":2"#), "{out}");
        assert!(out.contains(r#""type":"node""#));
        assert!(out.contains(r#""type":"edge""#));
        assert!(out.contains(r#""source":2"#));
        assert!(out.contains(r#""target":5"#));
    }

    #[test]
    fn explanation_object_has_target_and_paths() {
        use prism_api::{ExplanationView, IntroducerView};
        let v = ExplanationView {
            target: sample_node(),
            introducers: vec![IntroducerView {
                node: sample_node(),
                kinds: vec!["runtime", "build"],
            }],
            paths: vec![vec![sample_node(), sample_node()]],
            nearest_owner: None,
            owner_distance: 2,
            production_reachable: true,
            non_production_only: false,
        };
        let mut w = JsonWriter::new();
        explanation(&mut w, &v);
        let out = w.finish();
        assert!(out.contains(r#""kinds":["runtime","build"]"#), "{out}");
        assert!(out.contains(r#""nearest_owner":null"#), "{out}");
        assert!(out.contains(r#""owner_distance":2"#));
        assert!(out.contains(r#""production_reachable":true"#));
    }

    #[test]
    fn distance_object_serializes_negative_change() {
        use prism_api::{MetricDelta, TopologyDistanceView};
        let v = TopologyDistanceView {
            base: RunId::new(1),
            head: RunId::new(2),
            distance_pct: 30,
            deltas: vec![MetricDelta {
                metric: "edges",
                base: 100,
                head: 50,
                change_pct: -50,
            }],
        };
        let mut w = JsonWriter::new();
        topology_distance(&mut w, &v);
        let out = w.finish();
        assert!(out.contains(r#""distance_pct":30"#), "{out}");
        // Signed change must round-trip as a negative integer, not null.
        assert!(out.contains(r#""change_pct":-50"#), "{out}");
    }

    #[test]
    fn compare_array_carries_persistence() {
        use prism_api::FiltrationSummary;
        let rows = vec![FiltrationSummary {
            filtration: "depth",
            h0_components: 1,
            h1_loops: 4,
            finite_bars: 9,
            total_persistence: 12.5,
            max_persistence: 3.0,
        }];
        let mut w = JsonWriter::new();
        filtration_compare(&mut w, &rows);
        let out = w.finish();
        assert!(out.contains(r#""filtration":"depth""#), "{out}");
        assert!(out.contains(r#""h1_loops":4"#));
        assert!(out.contains(r#""total_persistence":12.5"#), "{out}");
        assert!(out.contains(r#""max_persistence":3"#), "{out}");
    }

    #[test]
    fn dup_causes_array_nests_introducers() {
        use prism_api::{ConstraintIntroducer, DuplicateRootCauseView, DuplicateVersionCause};
        use prism_model::{PackageId, VersionId};
        let views = vec![DuplicateRootCauseView {
            package: PackageId::new(3),
            package_purl: "pkg:npm/lodash".into(),
            versions: vec![DuplicateVersionCause {
                version: VersionId::new(7),
                purl: "pkg:npm/lodash@4.0.0".into(),
                occurrences: 2,
                introducers: vec![ConstraintIntroducer {
                    introducer_purl: "pkg:npm/a@1.0.0".into(),
                    constraint: "^4".into(),
                }],
            }],
            occurrence_count: 5,
            blast_radius: 8,
            distinct_constraints: 2,
            convergence: "diverged",
            risk_score: 24,
        }];
        let mut w = JsonWriter::new();
        duplicate_root_causes(&mut w, &views);
        let out = w.finish();
        assert!(out.contains(r#""convergence":"diverged""#), "{out}");
        assert!(out.contains(r#""constraint":"^4""#), "{out}");
        assert!(out.contains(r#""distinct_constraints":2"#));
        assert!(out.contains(r#""blast_radius":8"#));
    }

    #[test]
    fn barcode_object_has_dims() {
        let b = BarcodeView {
            run: RunId::new(1),
            filtration: FiltrationId::new(5),
            dims: vec![BarcodeDim {
                dim: 0,
                finite: 2,
                essential: 1,
            }],
        };
        let mut w = JsonWriter::new();
        barcode(&mut w, &b);
        let out = w.finish();
        assert!(out.contains(r#""filtration":5"#));
        assert!(out.contains(r#""component_count":1"#));
        assert!(out.contains(r#""dim":0"#));
        assert!(out.contains(r#""total":3"#));
    }
}
