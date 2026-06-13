//! `uv.lock` parsing into a resolved graph (and canonical facts).
//!
//! `uv.lock` is TOML. Each `[[package]]` is one resolved node (`name`,
//! `version`, `source`). Edges come from three places, all lists of inline
//! tables `{ name, version?, marker?, extra? }`:
//!
//! - `dependencies`            → runtime edges
//! - `[package.optional-dependencies]` (a table of extra → deps) → optional edges
//! - `[package.dev-dependencies]`      (a table of group → deps) → dev edges
//!
//! A dependency reference carries a `version` only when the name is ambiguous
//! (a resolution *fork* with multiple versions of one package), exactly like
//! Cargo's `name version` form — so resolution keys on `(name, version)` and
//! falls back to a unique bare name. A package whose `source` is `editable`,
//! `virtual`, or `directory` is a local/workspace member (a graph root).
//!
//! The lockfile pins exact selections verbatim, so the graph is
//! [`FidelityTier::LockfileExact`]. Parsing never resolves against an index or
//! executes anything, and never panics on hostile bytes.

use std::collections::HashMap;

use prism_model::{
    DependencyKind, EdgeFlags, FidelityTier, GraphFidelity, NodeFlags, VersionId,
};
use prism_plugin_api::{
    CanonicalBatch, CanonicalEvent, LockfileParser, PackageFact, PackageName, ParseError,
    RawDocument, RequirementFact, ResolvedEdge, ResolvedGraph, VersionFact,
};
use prism_textfmt::{parse_toml, Toml};

use crate::condition::classify_marker;
use crate::version::Pep440;

/// Upper bound on `[[package]]` entries to keep adversarial lockfiles bounded.
const MAX_PACKAGES: usize = 1_000_000;

/// `uv.lock` parser.
pub struct UvLockfile;

impl LockfileParser for UvLockfile {
    fn parse_lockfile(&self, raw: &RawDocument) -> Result<CanonicalBatch, ParseError> {
        let text = std::str::from_utf8(&raw.bytes)
            .map_err(|_| ParseError::new("not-utf8", "uv.lock is not utf-8"))?;
        parse_uv_batch(text)
    }
}

/// One resolved dependency reference within a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UvDep {
    /// Target project name (PEP 503 form, as uv writes it).
    pub name: String,
    /// Pinned version, present only to disambiguate a fork.
    pub version: Option<String>,
    /// Dependency category (runtime / optional / dev).
    pub kind: DependencyKind,
    /// PEP 508 environment/activation marker, if any.
    pub marker: Option<String>,
    /// Extras requested on the target.
    pub extras: Vec<String>,
}

/// One parsed `[[package]]`, before graph assembly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UvPackage {
    /// Project name exactly as written.
    pub name: String,
    /// Exact pinned version. Absent for an editable/virtual root project, which
    /// uv writes without a `version` field.
    pub version: Option<String>,
    /// True if the source is local (`editable`/`virtual`/`directory`) — a root.
    pub local: bool,
    /// All resolved dependency references (runtime + optional + dev).
    pub deps: Vec<UvDep>,
}

/// Parse a `uv.lock` string into its [`UvPackage`] list.
pub fn parse_uv_packages(text: &str) -> Result<Vec<UvPackage>, ParseError> {
    let doc = parse_toml(text).map_err(|e| ParseError::new("invalid-toml", e.to_string()))?;
    let arr = match doc.get("package").and_then(Toml::as_array) {
        Some(a) => a,
        None => return Ok(Vec::new()),
    };
    if arr.len() > MAX_PACKAGES {
        return Err(ParseError::new(
            "too-many-packages",
            format!("uv.lock has {} packages (max {MAX_PACKAGES})", arr.len()),
        ));
    }

    let mut out = Vec::with_capacity(arr.len());
    for (i, entry) in arr.iter().enumerate() {
        let name = entry.get("name").and_then(Toml::as_str).ok_or_else(|| {
            ParseError::new("missing-name", "[[package]] missing `name`").at(format!("package[{i}]"))
        })?;
        // uv omits `version` for an editable/virtual root project; that is valid.
        let version = entry.get("version").and_then(Toml::as_str).map(str::to_string);
        let local = entry
            .get("source")
            .and_then(Toml::as_table)
            .map(|t| {
                t.iter()
                    .any(|(k, _)| matches!(k.as_str(), "editable" | "virtual" | "directory"))
            })
            .unwrap_or(false);

        let mut deps = Vec::new();
        // Runtime dependencies: a plain array of inline tables.
        if let Some(list) = entry.get("dependencies").and_then(Toml::as_array) {
            collect_deps(list, DependencyKind::Runtime, &mut deps);
        }
        // optional-dependencies / dev-dependencies: a table of group -> dep array.
        collect_grouped(entry, "optional-dependencies", DependencyKind::Optional, &mut deps);
        collect_grouped(entry, "dev-dependencies", DependencyKind::Dev, &mut deps);

        out.push(UvPackage {
            name: name.to_string(),
            version,
            local,
            deps,
        });
    }
    Ok(out)
}

/// Push every inline-table dep in `list` (with the given `kind`) into `out`.
fn collect_deps(list: &[Toml], kind: DependencyKind, out: &mut Vec<UvDep>) {
    for item in list {
        let Some(name) = item.get("name").and_then(Toml::as_str) else {
            continue; // a dep with no name is unusable; skip rather than fail
        };
        let version = item.get("version").and_then(Toml::as_str).map(str::to_string);
        let marker = item.get("marker").and_then(Toml::as_str).map(str::to_string);
        let extras = item
            .get("extra")
            .and_then(Toml::as_array)
            .map(|a| a.iter().filter_map(Toml::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        out.push(UvDep {
            name: name.to_string(),
            version,
            kind,
            marker,
            extras,
        });
    }
}

/// Collect deps from a `field` that is a table of `group -> [deps]`.
fn collect_grouped(entry: &Toml, field: &str, kind: DependencyKind, out: &mut Vec<UvDep>) {
    let Some(groups) = entry.get(field).and_then(Toml::as_table) else {
        return;
    };
    for (_group, list) in groups {
        if let Some(arr) = list.as_array() {
            collect_deps(arr, kind, out);
        }
    }
}

/// Whether `version` is a PEP 440 pre-release/dev release.
fn is_prerelease(version: &str) -> bool {
    Pep440::parse(version).is_some_and(|v| v.is_prerelease())
}

/// Parse `uv.lock` into a [`ResolvedGraph`] at [`FidelityTier::LockfileExact`].
///
/// Node `local_id == package index`; the synthetic [`VersionId`] is `index + 1`.
/// The `occurrence_key` is `"name version"`. Unresolved references are dropped.
pub fn parse_uv_graph(text: &str) -> Result<ResolvedGraph, ParseError> {
    let packages = parse_uv_packages(text)?;

    // Resolution tables borrowing from `packages`.
    let mut by_name_version: HashMap<(&str, &str), usize> = HashMap::new();
    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, p) in packages.iter().enumerate() {
        if let Some(v) = &p.version {
            by_name_version.insert((p.name.as_str(), v.as_str()), i);
        }
        by_name.entry(p.name.as_str()).or_default().push(i);
    }

    let mut graph = ResolvedGraph::new();
    for (i, p) in packages.iter().enumerate() {
        let flags = if p.local {
            NodeFlags::ROOT
        } else {
            NodeFlags::EMPTY
        };
        // Occurrence identity is `name version`; a version-less root is just `name`.
        let key = match &p.version {
            Some(v) => format!("{} {}", p.name, v),
            None => p.name.clone(),
        };
        let local = graph.add_node(VersionId::new((i as u64) + 1), 0, key, flags);
        debug_assert_eq!(local as usize, i);
    }

    for (i, p) in packages.iter().enumerate() {
        for (ordinal, dep) in p.deps.iter().enumerate() {
            if let Some(to) = resolve(dep, &by_name_version, &by_name) {
                // Skip a self-edge: a package commonly dev-depends on itself with
                // an extra (`foo` → `foo[standard]`), which is not a topology edge
                // and would both demote `foo` from ROOT and fabricate a self-loop.
                if to == i {
                    continue;
                }
                graph.add_edge(ResolvedEdge {
                    from: i as u32,
                    to: to as u32,
                    ordinal: ordinal as u32,
                    requirement_id: None,
                    kind: dep.kind,
                    flags: EdgeFlags::RESOLVED,
                });
            }
        }
    }

    // Refine ROOT: a local member that is itself depended upon (path dependency)
    // is not a top-level root — clearing ROOT keeps the BFS depth honest.
    let mut has_incoming = vec![false; packages.len()];
    for e in &graph.edges {
        if (e.to as usize) < has_incoming.len() {
            has_incoming[e.to as usize] = true;
        }
    }
    for (i, node) in graph.nodes.iter_mut().enumerate() {
        if has_incoming[i] {
            node.flags.remove(NodeFlags::ROOT);
        }
    }

    graph.assign_bfs_depths_from_roots();
    graph.fidelity = Some(GraphFidelity::exact(FidelityTier::LockfileExact));
    Ok(graph)
}

/// Resolve a dependency reference to the index of the matching package, by
/// `(name, version)` when the fork-disambiguating version is present, else by a
/// unique bare name (ambiguous bare names are dropped, like Cargo).
fn resolve(
    dep: &UvDep,
    by_name_version: &HashMap<(&str, &str), usize>,
    by_name: &HashMap<&str, Vec<usize>>,
) -> Option<usize> {
    let name = dep.name.as_str();
    match dep.version.as_deref() {
        Some(v) => by_name_version
            .get(&(name, v))
            .copied()
            .or_else(|| unique(by_name, name)),
        None => unique(by_name, name),
    }
}

fn unique(by_name: &HashMap<&str, Vec<usize>>, name: &str) -> Option<usize> {
    match by_name.get(name) {
        Some(ids) if ids.len() == 1 => Some(ids[0]),
        _ => None,
    }
}

/// Parse `uv.lock` into a [`CanonicalBatch`] of package/version/requirement facts.
pub fn parse_uv_batch(text: &str) -> Result<CanonicalBatch, ParseError> {
    let packages = parse_uv_packages(text)?;
    let mut batch = CanonicalBatch::new();

    for p in &packages {
        // A version-less editable root has no concrete version to record; default
        // its purl-facing version the way the facade does (`0.0.0`).
        let version_raw = p.version.as_deref().unwrap_or("0.0.0");
        batch.push(CanonicalEvent::PackageSeen(PackageFact {
            name: PackageName::new(p.name.clone()),
            namespace: None,
            repository_url: None,
            homepage_url: None,
        }));
        batch.push(CanonicalEvent::VersionSeen(VersionFact {
            name: PackageName::new(p.name.clone()),
            version_raw: version_raw.to_string(),
            released_at_unix: None,
            license_expr: None,
            is_prerelease: is_prerelease(version_raw),
        }));
        for dep in &p.deps {
            // The lockfile records a resolved selection, so the requirement is the
            // exact pinned version when a fork disambiguator is present, else `*`.
            let raw_requirement = dep.version.clone().unwrap_or_else(|| "*".to_string());
            let condition_kind = match &dep.marker {
                Some(m) => classify_marker(m),
                None => prism_model::ConditionKind::None,
            };
            batch.push(CanonicalEvent::RequirementSeen(RequirementFact {
                src_name: PackageName::new(p.name.clone()),
                src_version_raw: version_raw.to_string(),
                declared_target: dep.name.clone(),
                target_alias: None,
                raw_requirement,
                kind: dep.kind,
                scope: None,
                condition_kind,
                raw_condition: dep.marker.clone(),
                features_or_extras: dep.extras.clone(),
            }));
        }
    }
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = r#"
version = 1
requires-python = ">=3.10"

[[package]]
name = "my-app"
version = "0.1.0"
source = { editable = "." }
dependencies = [
    { name = "anyio" },
    { name = "click", marker = "python_full_version < '3.11'" },
]

[package.optional-dependencies]
cli = [
    { name = "rich" },
]

[package.dev-dependencies]
dev = [
    { name = "pytest" },
]

[[package]]
name = "anyio"
version = "4.13.0"
source = { registry = "https://pypi.org/simple" }
dependencies = [
    { name = "idna" },
]

[[package]]
name = "click"
version = "8.1.7"
source = { registry = "https://pypi.org/simple" }

[[package]]
name = "rich"
version = "13.7.0"
source = { registry = "https://pypi.org/simple" }

[[package]]
name = "pytest"
version = "8.0.0"
source = { registry = "https://pypi.org/simple" }

[[package]]
name = "idna"
version = "3.6"
source = { registry = "https://pypi.org/simple" }
"#;

    #[test]
    fn parses_packages_with_all_dep_kinds() {
        let pkgs = parse_uv_packages(LOCK).unwrap();
        assert_eq!(pkgs.len(), 6);
        let app = &pkgs[0];
        assert_eq!(app.name, "my-app");
        assert!(app.local, "editable source is local");
        // 2 runtime + 1 optional + 1 dev.
        assert_eq!(app.deps.len(), 4);
        let kinds: Vec<DependencyKind> = app.deps.iter().map(|d| d.kind).collect();
        assert!(kinds.contains(&DependencyKind::Runtime));
        assert!(kinds.contains(&DependencyKind::Optional));
        assert!(kinds.contains(&DependencyKind::Dev));
        // The marker is captured.
        let click = app.deps.iter().find(|d| d.name == "click").unwrap();
        assert_eq!(click.marker.as_deref(), Some("python_full_version < '3.11'"));
    }

    #[test]
    fn builds_graph_with_kinded_edges_and_depths() {
        let g = parse_uv_graph(LOCK).unwrap();
        assert_eq!(g.node_count(), 6);
        // my-app -> {anyio, click, rich, pytest}; anyio -> idna  = 5 edges.
        assert_eq!(g.edge_count(), 5);
        // The root member is flagged; a depended-on package is not.
        assert!(g.nodes[0].flags.contains(NodeFlags::ROOT));
        assert!(!g.nodes[1].flags.contains(NodeFlags::ROOT)); // anyio
        // BFS depth: my-app=0, anyio=1, idna=2.
        assert_eq!(g.nodes[0].depth, 0);
        assert_eq!(g.nodes[1].depth, 1); // anyio
        assert_eq!(g.nodes[5].depth, 2); // idna
        assert_eq!(g.fidelity.unwrap().tier, FidelityTier::LockfileExact);
    }

    #[test]
    fn fork_disambiguation_by_version() {
        let lock = r#"
[[package]]
name = "root"
version = "0.1.0"
source = { virtual = "." }
dependencies = [
    { name = "dep", version = "1.0.0" },
    { name = "dep", version = "2.0.0" },
]

[[package]]
name = "dep"
version = "1.0.0"
source = { registry = "https://pypi.org/simple" }

[[package]]
name = "dep"
version = "2.0.0"
source = { registry = "https://pypi.org/simple" }
"#;
        let g = parse_uv_graph(lock).unwrap();
        assert_eq!(g.edge_count(), 2);
        let mut targets: Vec<&str> = g
            .edges
            .iter()
            .map(|e| g.nodes[e.to as usize].occurrence_key.as_str())
            .collect();
        targets.sort();
        assert_eq!(targets, ["dep 1.0.0", "dep 2.0.0"]);
    }

    #[test]
    fn facts_capture_kinds_markers_and_extras() {
        let b = parse_uv_batch(LOCK).unwrap();
        let reqs: Vec<&RequirementFact> = b
            .iter()
            .filter_map(|e| match e {
                CanonicalEvent::RequirementSeen(r) => Some(r),
                _ => None,
            })
            .collect();
        // 4 from my-app + 1 from anyio.
        assert_eq!(reqs.len(), 5);
        let click = reqs.iter().find(|r| r.declared_target == "click").unwrap();
        assert_eq!(click.condition_kind, prism_model::ConditionKind::Environment);
        let rich = reqs.iter().find(|r| r.declared_target == "rich").unwrap();
        assert_eq!(rich.kind, DependencyKind::Optional);
    }

    #[test]
    fn self_dependency_does_not_demote_root_or_loop() {
        // A package dev-depending on itself with an extra is a uv idiom; it must
        // not create a self-loop nor clear the root's ROOT flag.
        let lock = r#"
[[package]]
name = "app"
source = { editable = "." }
dependencies = [
    { name = "anyio" },
]

[package.dev-dependencies]
dev = [
    { name = "app", extra = ["test"] },
]

[[package]]
name = "anyio"
version = "4.13.0"
source = { registry = "https://pypi.org/simple" }
"#;
        let g = parse_uv_graph(lock).unwrap();
        // Only app -> anyio; the app -> app self-edge is dropped.
        assert_eq!(g.edge_count(), 1);
        assert!(g.nodes[0].flags.contains(NodeFlags::ROOT), "app stays a root");
        assert_eq!(g.nodes[0].depth, 0);
        assert_eq!(g.nodes[1].depth, 1, "anyio is depth 1 from the root");
    }

    #[test]
    fn empty_and_versionless_root() {
        assert_eq!(parse_uv_graph("version = 1\n").unwrap().node_count(), 0);
        // A version-less editable root is valid (uv omits its version).
        let root = "[[package]]\nname = \"app\"\nsource = { editable = \".\" }\n";
        let pkgs = parse_uv_packages(root).unwrap();
        assert_eq!(pkgs.len(), 1);
        assert!(pkgs[0].version.is_none());
        assert!(pkgs[0].local);
        assert_eq!(parse_uv_graph(root).unwrap().nodes[0].occurrence_key, "app");
        // A package missing its NAME is still rejected.
        assert!(parse_uv_packages("[[package]]\nversion = \"1.0\"\n").is_err());
    }
}
