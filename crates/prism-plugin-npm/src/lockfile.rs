//! `package-lock.json` parsing (architecture doc, *npm Plugin*).
//!
//! npm lockfile v2/v3 carry a `packages` map keyed by the **install path** of
//! each package, relative to the project root:
//!
//! ```text
//! ""                                          the root project
//! "node_modules/left-pad"                     a hoisted (top-level) dep
//! "node_modules/a/node_modules/b"             b nested under a (no hoist)
//! "node_modules/@scope/pkg"                   a scoped package
//! ```
//!
//! This module turns that map into a plain [`ResolvedGraph`]: one
//! [`ResolvedNode`] per `packages` entry (its `occurrence_key` is the path, its
//! `version` from the entry), and one [`ResolvedEdge`] per `dependencies` /
//! `optionalDependencies` entry, resolved to a target node by npm's
//! **nearest-ancestor `node_modules`** rule.
//!
//! ## Fidelity
//!
//! The result is [`FidelityTier::LockfileExact`]: every selection is read
//! verbatim, no version choices are made. Two approximations are documented and
//! flagged in the caveats:
//!
//! - **Synthetic version ids.** Parse-time has no store, so each occurrence is
//!   given a deterministic synthetic [`VersionId`] (1-based by `packages`
//!   iteration order). The downstream graph builder re-interns these against the
//!   real store; they exist only to wire edges locally.
//! - **Peer placement.** `peerDependencies` are not edges here — npm records
//!   their *placement* implicitly via hoisting, which we do not re-derive — so
//!   the fidelity carries [`FidelityCaveats::NO_PEER_PLACEMENT`]. A dependency
//!   that cannot be resolved to any ancestor node is dropped with a node count
//!   that still reflects every entry (we never fabricate a target).

use std::collections::HashMap;

use prism_model::{
    DependencyKind, EdgeFlags, FidelityCaveats, FidelityTier, GraphFidelity, NodeFlags, VersionId,
};
use prism_plugin_api::{
    CanonicalBatch, CanonicalEvent, ParseError, RawDocument, RequirementFact, VersionFact,
};
use prism_plugin_api::{LockfileParser, ResolvedEdge, ResolvedGraph};
use prism_textfmt::{parse_json, Json};

/// Lowest supported `lockfileVersion` (the `packages` map exists in v2 and v3).
pub const MIN_LOCKFILE_VERSION: i64 = 2;
/// Highest supported `lockfileVersion`.
pub const MAX_LOCKFILE_VERSION: i64 = 3;

/// npm [`LockfileParser`] for `package-lock.json` / `npm-shrinkwrap.json`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NpmLockfile;

impl LockfileParser for NpmLockfile {
    /// Emit canonical version + requirement facts for every locked package.
    /// (The resolved *graph* is produced by [`parse_package_lock`].)
    fn parse_lockfile(&self, raw: &RawDocument) -> Result<CanonicalBatch, ParseError> {
        let json = parse_root(raw)?;
        // Validate the lockfile version on BOTH entry points so the facts path and
        // the graph path agree on what is supported (was only checked in
        // parse_package_lock).
        let _ = check_lockfile_version(&json)?;
        let packages = packages_map(&json)?;

        let mut batch = CanonicalBatch::new();
        for (path, entry) in packages {
            let Some(name) = name_from_path(path) else {
                // The root entry ("") has no node_modules name; skip as a fact.
                continue;
            };
            let Some(version) = entry.get("version").and_then(Json::as_str) else {
                continue;
            };
            batch.push(CanonicalEvent::VersionSeen(VersionFact {
                name: name.into(),
                version_raw: version.to_string(),
                released_at_unix: None,
                license_expr: entry.get("license").and_then(Json::as_str).map(str::to_string),
                is_prerelease: version.contains('-'),
            }));

            for (field, kind) in [
                ("dependencies", DependencyKind::Runtime),
                ("optionalDependencies", DependencyKind::Optional),
                ("peerDependencies", DependencyKind::Peer),
            ] {
                if let Some(deps) = entry.get(field).and_then(Json::as_object) {
                    for (dep, req) in deps {
                        if let Some(rr) = req.as_str() {
                            batch.push(CanonicalEvent::RequirementSeen(RequirementFact {
                                src_name: name.into(),
                                src_version_raw: version.to_string(),
                                declared_target: dep.clone(),
                                target_alias: None,
                                raw_requirement: rr.to_string(),
                                kind,
                                scope: None,
                                condition_kind: prism_model::ConditionKind::None,
                                raw_condition: None,
                                features_or_extras: Vec::new(),
                            }));
                        }
                    }
                }
            }
        }
        Ok(batch)
    }
}

/// Parse a `package-lock.json` into a plain [`ResolvedGraph`].
///
/// One node per `packages` entry (including the root `""`), edges resolved by
/// nearest-ancestor `node_modules` lookup. See the module docs for fidelity
/// caveats. Returns [`ParseError`] on malformed input; never panics.
pub fn parse_package_lock(raw: &RawDocument) -> Result<ResolvedGraph, ParseError> {
    let json = parse_root(raw)?;
    let _ = check_lockfile_version(&json)?;
    let packages = packages_map(&json)?;

    // Pass 1: a node per entry; index by install dir for ancestor lookups. The
    // install *dir* of a `node_modules/x` entry is `node_modules/x`; a dep `d`
    // of that entry is searched at `<dir>/node_modules/d`, then up the parents.
    let mut graph = ResolvedGraph::new();
    // path -> local_id
    let mut by_path: HashMap<&str, u32> = HashMap::new();

    for (idx, (path, entry)) in packages.iter().enumerate() {
        let depth = node_modules_depth(path);
        let flags = if path.is_empty() {
            NodeFlags::ROOT
        } else {
            NodeFlags::EMPTY
        };
        // Synthetic, deterministic version id (1-based by iteration order).
        let version_id = VersionId::new(idx as u64 + 1);
        let local = graph.add_node(version_id, depth, path.as_str(), flags);
        // A `packages` map with a duplicate path (JSON keeps every duplicate key)
        // would otherwise silently mis-attribute the first entry's edges to the
        // second and orphan the first node; reject it instead.
        if by_path.insert(path.as_str(), local).is_some() {
            return Err(ParseError::new(
                "duplicate-package-path",
                format!("duplicate packages entry {path:?}"),
            ));
        }
        // Surface a missing version as a warning-free no-op: the node still
        // exists (occurrence identity is the path), just without a real version.
        let _ = entry.get("version");
    }

    // Pass 2: edges. For each entry, resolve each declared dependency name to the
    // nearest ancestor node_modules directory that holds it.
    for (path, entry) in packages.iter() {
        let from = by_path[path.as_str()];
        let mut ordinal = 0u32;
        for (field, kind, flag) in [
            ("dependencies", DependencyKind::Runtime, EdgeFlags::RESOLVED),
            (
                "optionalDependencies",
                DependencyKind::Optional,
                EdgeFlags::RESOLVED,
            ),
        ] {
            let Some(deps) = entry.get(field).and_then(Json::as_object) else {
                continue;
            };
            for (dep_name, _req) in deps {
                match resolve_nearest(path, dep_name, &by_path) {
                    Some(to) => {
                        graph.add_edge(ResolvedEdge {
                            from,
                            to,
                            ordinal,
                            requirement_id: None,
                            kind,
                            flags: flag,
                        });
                    }
                    None if kind == DependencyKind::Optional => {
                        // An optional dependency with no installed node was
                        // omitted by npm; record it as a dangling self-edge-free
                        // marker is impossible without a target, so we simply
                        // skip — the absence is itself the OPTIONAL_OMITTED fact.
                    }
                    None => {
                        // A runtime dep with no resolvable node: skip rather than
                        // fabricate a target (fidelity note in module docs).
                    }
                }
                ordinal += 1;
            }
        }
    }

    graph.fidelity = Some(GraphFidelity {
        tier: FidelityTier::LockfileExact,
        caveats: FidelityCaveats::NO_PEER_PLACEMENT,
    });
    Ok(graph)
}

// --- shared helpers ---------------------------------------------------------

fn parse_root(raw: &RawDocument) -> Result<Json, ParseError> {
    let text = std::str::from_utf8(&raw.bytes)
        .map_err(|_| ParseError::new("not-utf8", "package-lock.json is not utf-8"))?;
    let json =
        parse_json(text).map_err(|e| ParseError::new("invalid-json", e.to_string()))?;
    if !matches!(json, Json::Object(_)) {
        return Err(ParseError::new(
            "bad-lockfile",
            "package-lock.json root is not an object",
        ));
    }
    Ok(json)
}

fn check_lockfile_version(json: &Json) -> Result<i64, ParseError> {
    let v = json
        .get("lockfileVersion")
        .and_then(Json::as_i64)
        .ok_or_else(|| ParseError::new("missing-version", "lockfileVersion is missing"))?;
    if !(MIN_LOCKFILE_VERSION..=MAX_LOCKFILE_VERSION).contains(&v) {
        return Err(ParseError::new(
            "unsupported-lockfile-version",
            format!(
                "lockfileVersion {v} is unsupported (need {MIN_LOCKFILE_VERSION}..={MAX_LOCKFILE_VERSION})"
            ),
        ));
    }
    Ok(v)
}

fn packages_map(json: &Json) -> Result<&[(String, Json)], ParseError> {
    json.get("packages").and_then(Json::as_object).ok_or_else(|| {
        ParseError::new(
            "missing-packages",
            "lockfile has no `packages` map (v1 lockfiles are unsupported)",
        )
    })
}

/// The package name for a `packages` path. `""` (root) yields `None`; otherwise
/// the segment(s) after the last `node_modules` path *segment* are returned,
/// joining a scope (`@scope/name`) into a single name.
///
/// Matching on path-segment boundaries (not the `"node_modules/"` substring) means
/// a package legitimately named `my-node_modules`, or a crafted trailing slash,
/// cannot produce a stray-slashed or wrong name.
fn name_from_path(path: &str) -> Option<&str> {
    // Byte offset just after the last segment that is exactly "node_modules".
    let mut start: Option<usize> = None;
    let mut seg_begin = 0usize;
    for (i, ch) in path.char_indices() {
        if ch == '/' {
            if &path[seg_begin..i] == "node_modules" {
                start = Some(i + 1);
            }
            seg_begin = i + 1;
        }
    }
    let tail = &path[start?..];
    // Drop a trailing '/' (a crafted `node_modules/x/`) and reject an empty name.
    let tail = tail.strip_suffix('/').unwrap_or(tail);
    if tail.is_empty() {
        None
    } else {
        Some(tail)
    }
}

/// Install depth = number of `node_modules` path *segments* that contain a child
/// (root = 0). Counts directory boundaries, not substring occurrences, so a
/// package named e.g. `my-node_modules` does not inflate the depth.
fn node_modules_depth(path: &str) -> u32 {
    let segs: Vec<&str> = path.split('/').collect();
    // A "node_modules" segment is an install boundary only if a name follows it,
    // i.e. it is not the final segment.
    segs.iter()
        .take(segs.len().saturating_sub(1))
        .filter(|s| **s == "node_modules")
        .count() as u32
}

/// The directory prefix in which a dependency `dep` of the entry at `from_path`
/// is searched. For an entry at `node_modules/a`, deps live first at
/// `node_modules/a/node_modules/<dep>`, then at the parent's `node_modules`, up
/// to the root's `node_modules/<dep>`.
///
/// Returns the `local_id` of the nearest ancestor node that provides `dep`.
fn resolve_nearest(
    from_path: &str,
    dep: &str,
    by_path: &HashMap<&str, u32>,
) -> Option<u32> {
    // Build the chain of candidate install dirs from deepest to shallowest.
    // Start from the entry's own dir, then strip one `node_modules/<seg>` (or
    // scoped `node_modules/@scope/<seg>`) at a time.
    let mut dir = from_path.to_string();
    loop {
        let candidate = if dir.is_empty() {
            format!("node_modules/{dep}")
        } else {
            format!("{dir}/node_modules/{dep}")
        };
        if let Some(&id) = by_path.get(candidate.as_str()) {
            return Some(id);
        }
        if dir.is_empty() {
            return None;
        }
        dir = parent_install_dir(&dir);
    }
}

/// Strip the trailing `node_modules/<name>` (handling a scoped `@scope/name`)
/// from an install dir, yielding its parent install dir (possibly `""`).
fn parent_install_dir(dir: &str) -> String {
    match dir.rfind("node_modules/") {
        Some(i) => dir[..i].trim_end_matches('/').to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_model::HashDigest;
    use prism_plugin_api::{Provenance, SafeUrl};

    fn doc(bytes: &str) -> RawDocument {
        RawDocument {
            source_key: SafeUrl::new("file:///package-lock.json"),
            media_type: Some("application/json".into()),
            content_hash: HashDigest::default(),
            bytes: bytes.as_bytes().to_vec(),
            fetched_at_unix: 0,
            etag: None,
            last_modified: None,
            provenance: Provenance {
                source_id: "local".into(),
                retrieved_from: SafeUrl::new("file:///package-lock.json"),
                from_cache: false,
                snapshot: None,
            },
        }
    }

    const LOCK_V3: &str = r#"{
      "name": "app",
      "lockfileVersion": 3,
      "requires": true,
      "packages": {
        "": {
          "name": "app",
          "version": "1.0.0",
          "dependencies": { "a": "^1.0.0", "left-pad": "^1.3.0" }
        },
        "node_modules/a": {
          "version": "1.0.0",
          "dependencies": { "b": "^2.0.0" }
        },
        "node_modules/left-pad": {
          "version": "1.3.0"
        },
        "node_modules/b": {
          "version": "2.0.0"
        },
        "node_modules/a/node_modules/b": {
          "version": "2.5.0"
        }
      }
    }"#;

    fn name_of(g: &ResolvedGraph, local: u32) -> &str {
        name_from_path(&g.nodes[local as usize].occurrence_key).unwrap_or("<root>")
    }

    #[test]
    fn builds_one_node_per_package_entry() {
        let g = parse_package_lock(&doc(LOCK_V3)).unwrap();
        // 5 entries: root, a, left-pad, b (top-level), a/node_modules/b.
        assert_eq!(g.node_count(), 5);
        assert_eq!(g.fidelity.unwrap().tier, FidelityTier::LockfileExact);
        // Root carries ROOT flag and depth 0.
        let root = g.nodes.iter().find(|n| n.occurrence_key.is_empty()).unwrap();
        assert!(root.flags.contains(NodeFlags::ROOT));
        assert_eq!(root.depth, 0);
        // Nested b is at depth 2.
        let nested = g
            .nodes
            .iter()
            .find(|n| n.occurrence_key == "node_modules/a/node_modules/b")
            .unwrap();
        assert_eq!(nested.depth, 2);
    }

    #[test]
    fn edges_use_nearest_ancestor_lookup() {
        let g = parse_package_lock(&doc(LOCK_V3)).unwrap();

        // Root -> a and root -> left-pad (both top-level).
        let root = g.nodes.iter().find(|n| n.occurrence_key.is_empty()).unwrap();
        let root_targets: Vec<&str> = g
            .edges
            .iter()
            .filter(|e| e.from == root.local_id)
            .map(|e| name_of(&g, e.to))
            .collect();
        assert!(root_targets.contains(&"a"));
        assert!(root_targets.contains(&"left-pad"));

        // a depends on b: must resolve to the NESTED b (a/node_modules/b),
        // not the top-level b, by nearest-ancestor.
        let a = g
            .nodes
            .iter()
            .find(|n| n.occurrence_key == "node_modules/a")
            .unwrap();
        let a_edge = g.edges.iter().find(|e| e.from == a.local_id).unwrap();
        assert_eq!(
            g.nodes[a_edge.to as usize].occurrence_key,
            "node_modules/a/node_modules/b"
        );
    }

    #[test]
    fn lockfile_parser_emits_facts() {
        let batch = NpmLockfile.parse_lockfile(&doc(LOCK_V3)).unwrap();
        let versions: Vec<&VersionFact> = batch
            .iter()
            .filter_map(|e| match e {
                CanonicalEvent::VersionSeen(v) => Some(v),
                _ => None,
            })
            .collect();
        // a, left-pad, b, and nested b => 4 version facts (root "" skipped).
        assert_eq!(versions.len(), 4);
        assert!(versions.iter().any(|v| v.name.as_str() == "left-pad"
            && v.version_raw == "1.3.0"));
    }

    #[test]
    fn scoped_package_path_resolves() {
        let src = r#"{
          "name": "app", "lockfileVersion": 3,
          "packages": {
            "": { "version": "1.0.0", "dependencies": { "@scope/x": "^1.0.0" } },
            "node_modules/@scope/x": { "version": "1.2.3" }
          }
        }"#;
        let g = parse_package_lock(&doc(src)).unwrap();
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 1);
        assert_eq!(
            g.nodes[g.edges[0].to as usize].occurrence_key,
            "node_modules/@scope/x"
        );
        assert_eq!(
            name_from_path("node_modules/@scope/x"),
            Some("@scope/x")
        );
    }

    #[test]
    fn node_modules_boundary_not_substring() {
        // A package literally named `my-node_modules` must not inflate depth, and
        // its name must extract cleanly (not be split on the inner substring).
        assert_eq!(node_modules_depth("node_modules/my-node_modules"), 1);
        assert_eq!(
            node_modules_depth("node_modules/my-node_modules/node_modules/dep"),
            2
        );
        assert_eq!(
            name_from_path("node_modules/my-node_modules"),
            Some("my-node_modules")
        );
        // Crafted trailing slash and non-boundary substring are handled.
        assert_eq!(name_from_path("node_modules/x/"), Some("x"));
        assert_eq!(name_from_path("prefixnode_modules/x"), None);
        assert_eq!(name_from_path("node_modules/@scope/"), Some("@scope"));
    }

    #[test]
    fn duplicate_package_path_is_rejected() {
        // JSON keeps duplicate keys; a duplicate `packages` path must be rejected,
        // not silently mis-attribute edges.
        let src = r#"{
          "name": "app", "lockfileVersion": 3,
          "packages": {
            "": { "version": "1.0.0" },
            "node_modules/a": { "version": "1.0.0" },
            "node_modules/a": { "version": "2.0.0" }
          }
        }"#;
        assert!(parse_package_lock(&doc(src)).is_err());
    }

    #[test]
    fn unresolved_dependency_is_skipped_not_fabricated() {
        let src = r#"{
          "name": "app", "lockfileVersion": 3,
          "packages": {
            "": { "version": "1.0.0", "dependencies": { "ghost": "^1.0.0" } }
          }
        }"#;
        let g = parse_package_lock(&doc(src)).unwrap();
        assert_eq!(g.node_count(), 1);
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn rejects_unsupported_lockfile_version() {
        let src = r#"{ "lockfileVersion": 1, "packages": {} }"#;
        assert!(parse_package_lock(&doc(src)).is_err());
        let v4 = r#"{ "lockfileVersion": 4, "packages": {} }"#;
        assert!(parse_package_lock(&doc(v4)).is_err());
    }

    #[test]
    fn rejects_missing_packages_map() {
        let src = r#"{ "lockfileVersion": 3 }"#;
        assert!(parse_package_lock(&doc(src)).is_err());
    }

    #[test]
    fn nearest_ancestor_helpers() {
        assert_eq!(node_modules_depth(""), 0);
        assert_eq!(node_modules_depth("node_modules/a"), 1);
        assert_eq!(node_modules_depth("node_modules/a/node_modules/b"), 2);
        assert_eq!(parent_install_dir("node_modules/a"), "");
        assert_eq!(
            parent_install_dir("node_modules/a/node_modules/b"),
            "node_modules/a"
        );
        assert_eq!(name_from_path(""), None);
        assert_eq!(name_from_path("node_modules/left-pad"), Some("left-pad"));
    }
}
