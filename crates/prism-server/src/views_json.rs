//! Serializing `prism-api` view structs to JSON via the in-crate [`JsonWriter`].
//!
//! Each function writes one *value* (object or array) into a writer already
//! positioned at a value slot. Field order is fixed for deterministic output;
//! snapshot-local node ids are included but, like the CLI exporter, the purl is
//! the portable key.

use prism_api::{
    BarcodeView, DuplicateClusterView, GraphEdgeView, GraphNodeView, GraphStatsView,
    NeighborhoodView, SccView,
};

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
