//! Ecosystem-aware lockfile parsing into a topology plus per-node identity.
//!
//! The plugins return a [`ResolvedGraph`] whose `version_id` is *synthetic*
//! (`local_id + 1`); the facade must re-intern each occurrence's real
//! `(name, version)` against the store. To do that it needs, for every resolved
//! node, the concrete package name / namespace / version that the synthetic node
//! stands for. This module re-derives that identity column in the **same node
//! order** the plugin used, so `idents[local_id]` lines up with the graph.
//!
//! Parsing is offline and never panics on hostile bytes: malformed input becomes
//! a [`prism_model::PrismError`] (via [`prism_plugin_api::ParseError`]).

use prism_model::{PrismError, Result};
use prism_plugin_api::ResolvedGraph;
use prism_plugin_cargo::parse_lockfile_graph;
use prism_plugin_cargo::parse_lockfile_packages;
use prism_plugin_npm::{name_from_path as npm_name_from_path, parse_package_lock_value, NpmNames};
use prism_plugin_pnpm::{parse_pnpm_graph, parse_pnpm_units};
use prism_plugin_uv::{parse_uv_graph, parse_uv_packages};
use prism_textfmt::{parse_json, Json};

use crate::ecosystem::Ecosystem;

/// The concrete identity a resolved node stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NodeIdent {
    /// Raw package name (Cargo crate name or npm unscoped/scoped name).
    pub name: String,
    /// Optional namespace/scope (npm `@scope`, without the leading `@`).
    pub namespace: Option<String>,
    /// Raw pinned version, or `None` for a node with no version (e.g. the npm
    /// workspace root entry).
    pub version: Option<String>,
}

/// A lockfile parsed into a topology and an aligned identity column.
pub(crate) struct ParsedLockfile {
    /// The plain resolved graph (synthetic version ids, real topology).
    pub graph: ResolvedGraph,
    /// `idents[local_id]` is the identity of node `local_id` in `graph`.
    pub idents: Vec<NodeIdent>,
}

/// Parse `text` (the lockfile bytes as UTF-8) for `eco` into a [`ParsedLockfile`].
pub(crate) fn parse_lockfile(eco: Ecosystem, bytes: &[u8]) -> Result<ParsedLockfile> {
    match eco {
        Ecosystem::Cargo => parse_cargo(bytes),
        Ecosystem::Npm => parse_npm(bytes),
        Ecosystem::Bun => parse_bun(bytes),
        Ecosystem::Pnpm => parse_pnpm(bytes),
        Ecosystem::Uv => parse_uv(bytes),
    }
}

fn parse_cargo(bytes: &[u8]) -> Result<ParsedLockfile> {
    let text = std::str::from_utf8(bytes).map_err(|_| PrismError::Corrupt {
        detail: "Cargo.lock is not utf-8".into(),
    })?;
    // `parse_lockfile_packages` preserves `[[package]]` order, which is exactly
    // the node order `parse_lockfile_graph` assigns (`local_id == index`).
    let packages = parse_lockfile_packages(text)?;
    let graph = parse_lockfile_graph(text)?;
    debug_assert_eq!(packages.len(), graph.node_count());
    let idents = packages
        .into_iter()
        .map(|p| NodeIdent {
            name: p.name,
            namespace: None,
            version: Some(p.version),
        })
        .collect();
    Ok(ParsedLockfile { graph, idents })
}

fn parse_uv(bytes: &[u8]) -> Result<ParsedLockfile> {
    let text = std::str::from_utf8(bytes).map_err(|_| PrismError::Corrupt {
        detail: "uv.lock is not utf-8".into(),
    })?;
    // `parse_uv_packages` preserves `[[package]]` order, which is the node order
    // `parse_uv_graph` assigns (`local_id == index`). PyPI has no namespace.
    let packages = parse_uv_packages(text)?;
    let graph = parse_uv_graph(text)?;
    debug_assert_eq!(packages.len(), graph.node_count());
    let idents = packages
        .into_iter()
        .map(|p| NodeIdent {
            name: p.name,
            namespace: None,
            version: p.version,
        })
        .collect();
    Ok(ParsedLockfile { graph, idents })
}

fn parse_npm(bytes: &[u8]) -> Result<ParsedLockfile> {
    let text = std::str::from_utf8(bytes).map_err(|_| PrismError::Corrupt {
        detail: "package-lock.json is not utf-8".into(),
    })?;
    let json = parse_json(text).map_err(|e| PrismError::Corrupt {
        detail: format!("invalid package-lock.json: {e}"),
    })?;
    let graph = parse_package_lock_value(&json)?;
    let idents = idents_from_npm_value(&json)?;
    debug_assert_eq!(idents.len(), graph.node_count());
    Ok(ParsedLockfile { graph, idents })
}

/// Parse a `bun.lock` by reshaping it to an npm `package-lock.json` value and
/// reusing the npm topology + identity derivation (purls stay `pkg:npm/...`).
fn parse_bun(bytes: &[u8]) -> Result<ParsedLockfile> {
    let json = prism_plugin_bun::bun_lock_to_npm_value(bytes)?;
    let graph = parse_package_lock_value(&json)?;
    let idents = idents_from_npm_value(&json)?;
    debug_assert_eq!(idents.len(), graph.node_count());
    Ok(ParsedLockfile { graph, idents })
}

/// Parse a `pnpm-lock.yaml` directly into the resolved graph + identity column.
///
/// pnpm's `snapshots:` section is already a flat resolved graph, so the plugin
/// builds it verbatim (no `node_modules` hoisting). `parse_pnpm_units` yields the
/// same node order `parse_pnpm_graph` assigns; scoped `@scope/name` units split
/// into namespace + bare name the same way npm idents do, so purls stay `pkg:npm/...`.
fn parse_pnpm(bytes: &[u8]) -> Result<ParsedLockfile> {
    let text = std::str::from_utf8(bytes).map_err(|_| PrismError::Corrupt {
        detail: "pnpm-lock.yaml is not utf-8".into(),
    })?;
    let units = parse_pnpm_units(text)?;
    let graph = parse_pnpm_graph(text)?;
    debug_assert_eq!(units.len(), graph.node_count());
    let idents = units
        .into_iter()
        .map(|u| match NpmNames::split_scope(&u.name) {
            Some((scope, name)) => NodeIdent {
                name: name.to_string(),
                namespace: Some(scope.to_string()),
                version: u.version,
            },
            None => NodeIdent {
                name: u.name,
                namespace: None,
                version: u.version,
            },
        })
        .collect();
    Ok(ParsedLockfile { graph, idents })
}

/// Re-derive the per-node identity column from an npm-shaped lockfile [`Json`],
/// in the same `packages` map order the plugin assigns node ids. Shared by the
/// npm and Bun lockfile paths (Bun's document is npm-shaped after reshaping).
fn idents_from_npm_value(json: &Json) -> Result<Vec<NodeIdent>> {
    let packages = json
        .get("packages")
        .and_then(Json::as_object)
        .ok_or_else(|| PrismError::Corrupt {
            detail: "package-lock.json has no `packages` map".into(),
        })?;

    let mut idents = Vec::with_capacity(packages.len());
    for (path, entry) in packages {
        let version = entry.get("version").and_then(Json::as_str).map(str::to_string);
        match npm_name_from_path(path) {
            Some(full) => {
                // Split a scoped `@scope/name` into namespace + bare name.
                match NpmNames::split_scope(full) {
                    Some((scope, name)) => idents.push(NodeIdent {
                        name: name.to_string(),
                        namespace: Some(scope.to_string()),
                        version,
                    }),
                    None => idents.push(NodeIdent {
                        name: full.to_string(),
                        namespace: None,
                        version,
                    }),
                }
            }
            // The root entry ("") has no path-derived name, but npm records the
            // project's own name in the entry's `name` field — prefer that (it may
            // itself be scoped). Fall back to a plain `"root"` label only when the
            // lockfile omits it, and to the path for any other nameless entry.
            // (The old `<root>` placeholder leaked URL-reserved `<`/`>` into the
            // rendered purl, which read as an unfilled template.)
            None => {
                let root_name = entry
                    .get("name")
                    .and_then(Json::as_str)
                    .filter(|s| !s.is_empty());
                match root_name {
                    Some(full) => match NpmNames::split_scope(full) {
                        Some((scope, name)) => idents.push(NodeIdent {
                            name: name.to_string(),
                            namespace: Some(scope.to_string()),
                            version,
                        }),
                        None => idents.push(NodeIdent {
                            name: full.to_string(),
                            namespace: None,
                            version,
                        }),
                    },
                    None => idents.push(NodeIdent {
                        name: if path.is_empty() { "root".into() } else { path.clone() },
                        namespace: None,
                        version,
                    }),
                }
            }
        }
    }
    Ok(idents)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARGO_LOCK: &str = r#"
        version = 3
        [[package]]
        name = "app"
        version = "0.1.0"
        dependencies = ["serde 1.0.0"]
        [[package]]
        name = "serde"
        version = "1.0.0"
        source = "registry+https://github.com/rust-lang/crates.io-index"
    "#;

    const NPM_LOCK: &str = r#"{
      "lockfileVersion": 3,
      "packages": {
        "": { "version": "1.0.0", "dependencies": { "@scope/left-pad": "^1.3.0", "rxjs": "^7" } },
        "node_modules/@scope/left-pad": { "version": "1.3.0" },
        "node_modules/rxjs": { "version": "7.8.1" }
      }
    }"#;

    #[test]
    fn cargo_idents_align_with_graph() {
        let p = parse_lockfile(Ecosystem::Cargo, CARGO_LOCK.as_bytes()).unwrap();
        assert_eq!(p.graph.node_count(), 2);
        assert_eq!(p.idents.len(), 2);
        assert_eq!(p.idents[0].name, "app");
        assert_eq!(p.idents[0].version.as_deref(), Some("0.1.0"));
        assert_eq!(p.idents[1].name, "serde");
        assert!(p.idents[1].namespace.is_none());
        // The app -> serde edge is present.
        assert_eq!(p.graph.edge_count(), 1);
    }

    #[test]
    fn npm_idents_align_and_split_scope() {
        let p = parse_lockfile(Ecosystem::Npm, NPM_LOCK.as_bytes()).unwrap();
        assert_eq!(p.graph.node_count(), 3);
        assert_eq!(p.idents.len(), 3);
        // Root entry — plain `root` (purl-safe; no URL-reserved characters).
        assert_eq!(p.idents[0].name, "root");
        assert_eq!(p.idents[0].version.as_deref(), Some("1.0.0"));
        // Scoped package split into namespace + name.
        assert_eq!(p.idents[1].namespace.as_deref(), Some("scope"));
        assert_eq!(p.idents[1].name, "left-pad");
        assert_eq!(p.idents[1].version.as_deref(), Some("1.3.0"));
        // Unscoped package.
        assert_eq!(p.idents[2].name, "rxjs");
        assert!(p.idents[2].namespace.is_none());
    }

    #[test]
    fn npm_root_uses_the_entry_name_field() {
        // A realistic package-lock.json records the project name on the root entry.
        let lock = r#"{
          "lockfileVersion": 3,
          "packages": {
            "": { "name": "my-app", "version": "2.1.0", "dependencies": { "rxjs": "^7" } },
            "node_modules/rxjs": { "version": "7.8.1" }
          }
        }"#;
        let p = parse_lockfile(Ecosystem::Npm, lock.as_bytes()).unwrap();
        assert_eq!(p.idents[0].name, "my-app", "root takes its `name` field");
        assert!(p.idents[0].namespace.is_none());
        assert_eq!(p.idents[0].version.as_deref(), Some("2.1.0"));
    }

    #[test]
    fn npm_scoped_root_name_splits() {
        let lock = r#"{
          "lockfileVersion": 3,
          "packages": {
            "": { "name": "@acme/app", "version": "1.0.0" }
          }
        }"#;
        let p = parse_lockfile(Ecosystem::Npm, lock.as_bytes()).unwrap();
        assert_eq!(p.idents[0].namespace.as_deref(), Some("acme"));
        assert_eq!(p.idents[0].name, "app");
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        assert!(parse_lockfile(Ecosystem::Npm, b"{ not json").is_err());
        assert!(parse_lockfile(Ecosystem::Cargo, &[0xff, 0xfe]).is_err());
        assert!(parse_lockfile(Ecosystem::Bun, &[0xff, 0xfe]).is_err());
        assert!(parse_lockfile(Ecosystem::Pnpm, &[0xff, 0xfe]).is_err());
    }

    const PNPM_LOCK: &str = r#"lockfileVersion: '9.0'

importers:

  .:
    dependencies:
      '@scope/left-pad':
        specifier: 1.3.0
        version: 1.3.0
      rxjs:
        specifier: ^7
        version: 7.8.1

snapshots:

  '@scope/left-pad@1.3.0': {}

  rxjs@7.8.1:
    dependencies:
      tslib: 2.6.0

  tslib@2.6.0: {}
"#;

    #[test]
    fn pnpm_idents_align_and_split_scope() {
        let p = parse_lockfile(Ecosystem::Pnpm, PNPM_LOCK.as_bytes()).unwrap();
        // root + @scope/left-pad + rxjs + tslib
        assert_eq!(p.graph.node_count(), 4);
        assert_eq!(p.idents.len(), p.graph.node_count());
        // Synthetic root first — plain `root`, purl-safe (no `<`/`>`).
        assert_eq!(p.idents[0].name, "root");
        assert!(p.idents[0].version.is_none());
        // Scoped package split into namespace + bare name (purls stay pkg:npm/...).
        let lp = p.idents.iter().find(|i| i.name == "left-pad").unwrap();
        assert_eq!(lp.namespace.as_deref(), Some("scope"));
        assert_eq!(lp.version.as_deref(), Some("1.3.0"));
        // The transitive edge rxjs -> tslib resolved from the flat snapshot graph.
        let tslib = p.idents.iter().find(|i| i.name == "tslib").unwrap();
        assert!(tslib.namespace.is_none());
        // root -> {@scope/left-pad, rxjs}, rxjs -> tslib = 3 edges.
        assert_eq!(p.graph.edge_count(), 3);
    }

    const BUN_LOCK: &str = r#"{
      "lockfileVersion": 1,
      "workspaces": {
        "": { "name": "app", "dependencies": { "@scope/left-pad": "1.3.0" } },
      },
      "packages": {
        "@scope/left-pad": ["@scope/left-pad@1.3.0", "", {}, "sha512-x"],
        "rxjs": ["rxjs@7.8.1", "", {}, "sha512-y"],
      },
    }"#;

    #[test]
    fn idents_use_segment_based_naming_not_substring() {
        // A package path whose parent name *contains* "node_modules" as a
        // substring. The identity column must use the npm plugin's segment-based
        // rule (-> "a-node_modules/x"), the same rule the graph/facts use, not a
        // naive rfind("node_modules/") (which would wrongly yield "x"). This is
        // the regression the bun review surfaced.
        let lock = r#"{
          "lockfileVersion": 3,
          "packages": {
            "": { "name": "app" },
            "node_modules/a-node_modules/x": { "version": "9.9.9" }
          }
        }"#;
        let p = parse_lockfile(Ecosystem::Npm, lock.as_bytes()).unwrap();
        let tricky = p.idents.iter().find(|i| i.version.as_deref() == Some("9.9.9")).unwrap();
        assert_eq!(tricky.name, "a-node_modules/x");
        assert!(tricky.namespace.is_none());
    }

    #[test]
    fn bun_idents_align_and_split_scope() {
        // bun.lock parses (JSONC + trailing commas) into the same identity column
        // npm would produce for the equivalent package-lock.json.
        let p = parse_lockfile(Ecosystem::Bun, BUN_LOCK.as_bytes()).unwrap();
        // root + @scope/left-pad (+ a root->workspace... no workspace here) + rxjs
        assert_eq!(p.graph.node_count(), p.idents.len());
        // The scoped package is split into namespace + bare name.
        let lp = p
            .idents
            .iter()
            .find(|i| i.name == "left-pad")
            .expect("left-pad present");
        assert_eq!(lp.namespace.as_deref(), Some("scope"));
        assert_eq!(lp.version.as_deref(), Some("1.3.0"));
        // The root edge to @scope/left-pad resolves (root depends on it).
        assert!(p.graph.edge_count() >= 1);
    }
}
