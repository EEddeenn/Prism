//! `Cargo.lock` parsing into a resolved graph (architecture doc, *Cargo Plugin*).
//!
//! Each `[[package]]` table becomes one resolved node (`name`, `version`,
//! `source`); each entry in a package's `dependencies` list becomes one edge,
//! resolving the `"name version"` / `"name"` form to the matching package node.
//! Because a lockfile pins exact selections verbatim, the resulting graph's
//! fidelity is [`FidelityTier::LockfileExact`].
//!
//! Two views are produced:
//! - [`parse_lockfile_graph`] returns the plain [`ResolvedGraph`] used by the
//!   graph builder.
//! - The [`LockfileParser`] trait impl returns a [`CanonicalBatch`] of
//!   package/version/requirement facts for the store writer.
//!
//! Parsing never resolves names against a registry or executes anything.

use std::collections::HashMap;

use prism_model::{
    ConditionKind, DependencyKind, EdgeFlags, FidelityTier, GraphFidelity, NodeFlags, VersionId,
};
use prism_plugin_api::{
    CanonicalBatch, CanonicalEvent, LockfileParser, PackageFact, PackageName, ParseError,
    ParseWarning, RawDocument, RequirementFact, ResolvedEdge, ResolvedGraph, VersionFact,
};
use prism_textfmt::{parse_toml, Toml};

/// Upper bound on `[[package]]` entries to keep adversarial lockfiles bounded.
const MAX_PACKAGES: usize = 1_000_000;

/// `Cargo.lock` parser.
pub struct CargoLockfile;

impl LockfileParser for CargoLockfile {
    fn parse_lockfile(&self, raw: &RawDocument) -> Result<CanonicalBatch, ParseError> {
        let text = std::str::from_utf8(&raw.bytes)
            .map_err(|_| ParseError::new("not-utf8", "Cargo.lock is not utf-8"))?;
        parse_lockfile_batch(text)
    }
}

/// One parsed lockfile package, before graph assembly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockPackage {
    /// Crate name exactly as written.
    pub name: String,
    /// Exact pinned version.
    pub version: String,
    /// Source string (`registry+…`, `git+…`, path), if present.
    pub source: Option<String>,
    /// Raw `dependencies` entries (`"name"` or `"name version"`).
    pub dependencies: Vec<String>,
    /// Optional `checksum` hex string.
    pub checksum: Option<String>,
}

/// Parse a `Cargo.lock` string into its [`LockPackage`] list. Shared by the
/// graph and batch views.
pub fn parse_lockfile_packages(text: &str) -> Result<Vec<LockPackage>, ParseError> {
    let doc = parse_toml(text).map_err(|e| ParseError::new("invalid-toml", e.to_string()))?;
    let arr = match doc.get("package").and_then(Toml::as_array) {
        Some(a) => a,
        // A lockfile with no packages is valid (empty workspace).
        None => return Ok(Vec::new()),
    };
    if arr.len() > MAX_PACKAGES {
        return Err(ParseError::new(
            "too-many-packages",
            format!("lockfile has {} packages (max {MAX_PACKAGES})", arr.len()),
        ));
    }

    let mut out = Vec::with_capacity(arr.len());
    for (i, entry) in arr.iter().enumerate() {
        let name = entry
            .get("name")
            .and_then(Toml::as_str)
            .ok_or_else(|| {
                ParseError::new("missing-name", "[[package]] missing `name`")
                    .at(format!("package[{i}]"))
            })?
            .to_string();
        let version = entry
            .get("version")
            .and_then(Toml::as_str)
            .ok_or_else(|| {
                ParseError::new("missing-version", "[[package]] missing `version`")
                    .at(format!("package[{i}]"))
            })?
            .to_string();
        let source = entry
            .get("source")
            .and_then(Toml::as_str)
            .map(str::to_string);
        let checksum = entry
            .get("checksum")
            .and_then(Toml::as_str)
            .map(str::to_string);
        let dependencies = entry
            .get("dependencies")
            .and_then(Toml::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Toml::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        out.push(LockPackage {
            name,
            version,
            source,
            dependencies,
            checksum,
        });
    }
    Ok(out)
}

/// Parse a `Cargo.lock` dependency reference into `(name, Option<version>)`.
///
/// A reference takes one of three forms:
/// - `name` — bare (the name is unambiguous in the graph),
/// - `name version` — disambiguated by version,
/// - `name version (source)` — additionally qualified by source. This third form
///   is used for **every** dependency in lockfile `version = 1`, so failing to
///   strip the ` (source)` qualifier silently drops essentially all edges.
///
/// The trailing ` (source)` qualifier is stripped; resolution keys on
/// `(name, version)`.
fn parse_dep_ref(dep: &str) -> (&str, Option<&str>) {
    // Drop a trailing " (source)" qualifier (source URLs carry no spaces, and the
    // name/version never contain " (", so this split is unambiguous).
    let core = match dep.split_once(" (") {
        Some((head, _source)) => head,
        None => dep,
    };
    match core.split_once(' ') {
        Some((name, version)) => (name, Some(version)),
        None => (core, None),
    }
}

/// Whether `version` is a SemVer prerelease, decided by the shared engine rather
/// than a naive `contains('-')` (which misfires on build metadata such as
/// `1.0.0+build-meta`). An unparseable version is treated as non-prerelease.
pub(crate) fn is_prerelease(version: &str) -> bool {
    prism_semver::Version::parse(version)
        .map(|v| v.is_prerelease())
        .unwrap_or(false)
}

/// Resolve a lockfile dependency reference to the index of the matching package.
///
/// Cargo writes the version only when a name is ambiguous (multiple versions of
/// the same crate in the graph); otherwise a bare name is unique.
fn resolve_ref<'a>(
    dep: &'a str,
    by_name_version: &HashMap<(&'a str, &'a str), usize>,
    by_name: &HashMap<&'a str, Vec<usize>>,
) -> Option<usize> {
    let (name, version) = parse_dep_ref(dep);
    match version {
        // Resolve on (name, version); fall back to a unique bare name if the
        // exact pair is missing (e.g. a benign source mismatch).
        Some(v) => by_name_version.get(&(name, v)).copied().or_else(|| {
            match by_name.get(name) {
                Some(ids) if ids.len() == 1 => Some(ids[0]),
                _ => None,
            }
        }),
        None => match by_name.get(name) {
            Some(ids) if ids.len() == 1 => Some(ids[0]),
            _ => None,
        },
    }
}

/// Parse `Cargo.lock` into a [`ResolvedGraph`] with [`FidelityTier::LockfileExact`].
///
/// Node `local_id == package index`; the synthetic [`VersionId`] is
/// `index + 1` (real ids are assigned by the store writer downstream). The
/// `occurrence_key` is `"name version"`. Unresolved dependency references are
/// dropped (they cannot point anywhere) and do not abort the parse.
pub fn parse_lockfile_graph(text: &str) -> Result<ResolvedGraph, ParseError> {
    let packages = parse_lockfile_packages(text)?;

    // Build the resolution lookup tables borrowing from `packages`.
    let mut by_name_version: HashMap<(&str, &str), usize> = HashMap::new();
    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, p) in packages.iter().enumerate() {
        by_name_version.insert((p.name.as_str(), p.version.as_str()), i);
        by_name.entry(p.name.as_str()).or_default().push(i);
    }

    let mut graph = ResolvedGraph::new();
    for (i, p) in packages.iter().enumerate() {
        // Root heuristic: a package with no source is a local/workspace member.
        // (Refined below once edges exist — a sourceless package that is itself
        // depended upon is a path dependency, not a top-level root.)
        let flags = if p.source.is_none() {
            NodeFlags::ROOT
        } else {
            NodeFlags::EMPTY
        };
        // Occurrence identity is source+name+version (architecture doc): include
        // the source so the same name+version from two distinct sources
        // (crates.io vs git/path, or two git revisions) stay distinguishable.
        let key = match &p.source {
            Some(src) => format!("{} {} ({src})", p.name, p.version),
            None => format!("{} {}", p.name, p.version),
        };
        let local = graph.add_node(VersionId::new((i as u64) + 1), 0, key, flags);
        debug_assert_eq!(local as usize, i);
    }

    for (i, p) in packages.iter().enumerate() {
        for (ordinal, dep) in p.dependencies.iter().enumerate() {
            if let Some(to) = resolve_ref(dep, &by_name_version, &by_name) {
                graph.add_edge(ResolvedEdge {
                    from: i as u32,
                    to: to as u32,
                    ordinal: ordinal as u32,
                    requirement_id: None,
                    kind: DependencyKind::Runtime,
                    flags: EdgeFlags::RESOLVED,
                });
            }
            // Unresolvable refs are silently skipped: a malformed lockfile must
            // not crash, and a dangling edge has no target node.
        }
    }

    // Refine the ROOT heuristic: a sourceless package that is itself depended upon
    // (has an incoming edge) is a path dependency or a depended-on member, not a
    // top-level root — clearing ROOT keeps the BFS depth honest.
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

    // `Cargo.lock` is a flat resolved set with no install-layout nesting, so the
    // node `depth` (placed at 0 above) must be derived from the edge structure:
    // each node's minimum BFS distance from the nearest workspace-member root.
    graph.assign_bfs_depths_from_roots();

    graph.fidelity = Some(GraphFidelity::exact(FidelityTier::LockfileExact));
    Ok(graph)
}

/// Parse `Cargo.lock` into a [`CanonicalBatch`] of package/version/requirement
/// facts. Each dependency reference becomes a [`RequirementFact`] pinned to the
/// exact version when the reference carries one.
pub fn parse_lockfile_batch(text: &str) -> Result<CanonicalBatch, ParseError> {
    let packages = parse_lockfile_packages(text)?;
    let mut batch = CanonicalBatch::new();

    for p in &packages {
        batch.push(CanonicalEvent::PackageSeen(PackageFact {
            name: PackageName::new(p.name.clone()),
            namespace: None,
            repository_url: None,
            homepage_url: None,
        }));
        batch.push(CanonicalEvent::VersionSeen(VersionFact {
            name: PackageName::new(p.name.clone()),
            version_raw: p.version.clone(),
            released_at_unix: None,
            license_expr: None,
            is_prerelease: is_prerelease(&p.version),
        }));
        if let Some(cs) = &p.checksum {
            batch.warn(ParseWarning::new(
                "checksum",
                format!("{} {} = {cs}", p.name, p.version),
            ));
        }
        for dep in &p.dependencies {
            // Use the shared reference parser so the `name version (source)` form
            // (lockfile v1) yields a bare, parseable version — not the malformed
            // `1.0.0 (registry+...)` the naive split_once produced.
            let (name, version) = parse_dep_ref(dep);
            let target = name.to_string();
            // Bare name: the exact version is implied by the unique node.
            let exact = version.unwrap_or("*").to_string();
            batch.push(CanonicalEvent::RequirementSeen(RequirementFact {
                src_name: PackageName::new(p.name.clone()),
                src_version_raw: p.version.clone(),
                declared_target: target,
                target_alias: None,
                raw_requirement: exact,
                kind: DependencyKind::Runtime,
                scope: None,
                condition_kind: ConditionKind::None,
                raw_condition: None,
                features_or_extras: Vec::new(),
            }));
        }
    }
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = r#"
        # This file is automatically @generated by Cargo.
        version = 3

        [[package]]
        name = "my-app"
        version = "0.1.0"
        dependencies = [
         "serde",
         "rand 0.8.5",
        ]

        [[package]]
        name = "serde"
        version = "1.0.197"
        source = "registry+https://github.com/rust-lang/crates.io-index"
        checksum = "3fb1c873e1b9b056a4dc4c0c198b9c0a5b0a3a1b0c9d8e7f6a5b4c3d2e1f0a9b"
        dependencies = [
         "serde_derive",
        ]

        [[package]]
        name = "serde_derive"
        version = "1.0.197"
        source = "registry+https://github.com/rust-lang/crates.io-index"

        [[package]]
        name = "rand"
        version = "0.8.5"
        source = "registry+https://github.com/rust-lang/crates.io-index"
    "#;

    #[test]
    fn parses_package_set() {
        let pkgs = parse_lockfile_packages(LOCK).unwrap();
        assert_eq!(pkgs.len(), 4);
        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["my-app", "serde", "serde_derive", "rand"]);
        // my-app has no source -> local; serde has one.
        assert!(pkgs[0].source.is_none());
        assert!(pkgs[1].source.is_some());
        assert_eq!(
            pkgs[1].checksum.as_deref(),
            Some("3fb1c873e1b9b056a4dc4c0c198b9c0a5b0a3a1b0c9d8e7f6a5b4c3d2e1f0a9b")
        );
    }

    #[test]
    fn builds_resolved_graph_with_edges() {
        let g = parse_lockfile_graph(LOCK).unwrap();
        assert_eq!(g.node_count(), 4);
        // occurrence keys identify each node (source-qualified when present).
        assert_eq!(g.nodes[0].occurrence_key, "my-app 0.1.0"); // root: no source
        assert!(g.nodes[1].occurrence_key.starts_with("serde 1.0.197 (registry+"));
        // my-app -> serde (bare), my-app -> rand 0.8.5 (versioned), serde -> serde_derive.
        assert_eq!(g.edge_count(), 3);
        let edge_targets: Vec<&str> = g
            .edges
            .iter()
            .map(|e| g.nodes[e.to as usize].occurrence_key.as_str())
            .collect();
        assert!(edge_targets.iter().any(|t| t.starts_with("serde 1.0.197 (")));
        assert!(edge_targets.iter().any(|t| t.starts_with("rand 0.8.5 (")));
        assert!(edge_targets.iter().any(|t| t.starts_with("serde_derive 1.0.197 (")));
        // The root member is flagged.
        assert!(g.nodes[0].flags.contains(NodeFlags::ROOT));
        assert!(!g.nodes[1].flags.contains(NodeFlags::ROOT));
    }

    #[test]
    fn depths_are_bfs_distance_from_root() {
        // my-app(0) -> serde(1) -> serde_derive(2); my-app(0) -> rand(1).
        let g = parse_lockfile_graph(LOCK).unwrap();
        assert_eq!(g.nodes[0].depth, 0, "root my-app");
        assert_eq!(g.nodes[1].depth, 1, "serde");
        assert_eq!(g.nodes[2].depth, 2, "serde_derive");
        assert_eq!(g.nodes[3].depth, 1, "rand");
    }

    #[test]
    fn fidelity_is_lockfile_exact() {
        let g = parse_lockfile_graph(LOCK).unwrap();
        assert_eq!(g.fidelity.unwrap().tier, FidelityTier::LockfileExact);
        assert!(!g.truncated);
    }

    #[test]
    fn edges_resolve_to_correct_versions() {
        // Two versions of the same crate -> bare name is ambiguous, versioned
        // references must disambiguate.
        let lock = r#"
            [[package]]
            name = "root"
            version = "0.1.0"
            dependencies = ["dep 1.0.0", "dep 2.0.0"]

            [[package]]
            name = "dep"
            version = "1.0.0"
            source = "registry+x"

            [[package]]
            name = "dep"
            version = "2.0.0"
            source = "registry+x"
        "#;
        let g = parse_lockfile_graph(lock).unwrap();
        assert_eq!(g.edge_count(), 2);
        let mut targets: Vec<&str> = g
            .edges
            .iter()
            .map(|e| g.nodes[e.to as usize].occurrence_key.as_str())
            .collect();
        targets.sort();
        assert_eq!(targets, ["dep 1.0.0 (registry+x)", "dep 2.0.0 (registry+x)"]);
    }

    #[test]
    fn v1_lockfile_source_qualified_refs_resolve() {
        // Lockfile version=1 writes EVERY dependency as `name version (source)`;
        // the ` (source)` qualifier must be stripped so the edge resolves.
        let lock = r#"
            [[package]]
            name = "app"
            version = "0.1.0"
            dependencies = ["serde 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)"]

            [[package]]
            name = "serde"
            version = "1.0.0"
            source = "registry+https://github.com/rust-lang/crates.io-index"
        "#;
        let g = parse_lockfile_graph(lock).unwrap();
        assert_eq!(g.edge_count(), 1, "the source-qualified edge must resolve");
        assert_eq!(g.edges[0].from, 0);
        assert_eq!(g.edges[0].to, 1);

        // And the facts path stores a bare, parseable raw_requirement.
        let batch = parse_lockfile_batch(lock).unwrap();
        let has_bare_req = batch.iter().any(|e| matches!(
            e,
            CanonicalEvent::RequirementSeen(r) if r.raw_requirement == "1.0.0"
        ));
        assert!(has_bare_req, "raw_requirement must be the bare version, not `1.0.0 (source)`");
    }

    #[test]
    fn build_metadata_hyphen_is_not_prerelease() {
        // '-' living in build metadata must NOT flag a prerelease.
        assert!(!is_prerelease("1.0.0+build-meta"));
        assert!(is_prerelease("1.0.0-rc.1"));
        assert!(!is_prerelease("1.2.3"));
    }

    #[test]
    fn unresolvable_reference_is_dropped() {
        let lock = r#"
            [[package]]
            name = "root"
            version = "0.1.0"
            dependencies = ["ghost"]
        "#;
        let g = parse_lockfile_graph(lock).unwrap();
        assert_eq!(g.node_count(), 1);
        assert_eq!(g.edge_count(), 0); // dangling ref skipped, no crash
    }

    #[test]
    fn missing_required_field_errors() {
        let lock = r#"
            [[package]]
            version = "0.1.0"
        "#;
        assert!(parse_lockfile_packages(lock).is_err());
    }

    #[test]
    fn batch_view_emits_facts() {
        let b = parse_lockfile_batch(LOCK).unwrap();
        let pkg_count = b
            .iter()
            .filter(|e| matches!(e, CanonicalEvent::PackageSeen(_)))
            .count();
        assert_eq!(pkg_count, 4);
        let req_count = b
            .iter()
            .filter(|e| matches!(e, CanonicalEvent::RequirementSeen(_)))
            .count();
        // my-app: 2 deps, serde: 1 dep = 3 requirement facts.
        assert_eq!(req_count, 3);
    }

    #[test]
    fn empty_lockfile_is_ok() {
        let g = parse_lockfile_graph("version = 3\n").unwrap();
        assert_eq!(g.node_count(), 0);
    }
}
