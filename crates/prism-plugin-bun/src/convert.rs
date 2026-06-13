//! Transform a Bun text lockfile (`bun.lock`) into an npm `package-lock.json`
//! v3 [`Json`] value, in memory.
//!
//! Bun and npm install the *same* registry packages with the *same* SemVer
//! semantics, so once the lockfile is reshaped the npm plugin can resolve the
//! topology and emit facts verbatim. The reshape is faithful because Bun's
//! `packages` keys ARE npm's nested-`node_modules` resolution tree:
//!
//! ```text
//! bun key            npm packages path
//! "foo"          ->  "node_modules/foo"                       (hoisted)
//! "foo/bar"      ->  "node_modules/foo/node_modules/bar"      (a 2nd copy of bar)
//! "@scope/pkg"   ->  "node_modules/@scope/pkg"                (scoped, one segment)
//! "a/@s/b"       ->  "node_modules/a/node_modules/@s/b"
//! ```
//!
//! Because npm resolves each edge by the dependency *name* (nearest ancestor),
//! Bun's `catalog:` / `workspace:*` constraint *values* need no resolution — Bun
//! has already pinned the node set. Workspace packages become real nodes; a
//! synthetic root (`""`) depends on every workspace so the graph is rooted.
//!
//! `bun.lock` is JSONC (line comments + trailing commas); [`strip_jsonc`] removes
//! both — string-aware — before the hermetic [`parse_json`].

use std::collections::HashSet;

use prism_plugin_api::ParseError;
use prism_textfmt::{parse_json, Json};

/// Reshape `bytes` (a `bun.lock`) into an npm `package-lock.json` v3 [`Json`].
///
/// Never panics on hostile input: malformed bytes become a [`ParseError`].
pub fn bun_lock_to_npm_value(bytes: &[u8]) -> Result<Json, ParseError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ParseError::new("not-utf8", "bun.lock is not utf-8"))?;
    let doc = parse_json(&strip_jsonc(text))
        .map_err(|e| ParseError::new("invalid-bun-lock", e.to_string()))?;

    // `seen` rejects the (impossible-in-practice) case of two keys mapping to one
    // path, which npm would reject as a duplicate; keeping the first is safe.
    let mut seen: HashSet<String> = HashSet::new();
    let mut npm_pkgs: Vec<(String, Json)> = Vec::new();

    // --- registry (and other non-workspace) packages ------------------------
    if let Some(pkgs) = doc.get("packages").and_then(Json::as_object) {
        for (key, val) in pkgs {
            let Some(arr) = val.as_array() else { continue };
            let Some(descriptor) = arr.first().and_then(Json::as_str) else {
                continue;
            };
            let Some(version) = descriptor_version(descriptor) else {
                continue;
            };
            // Workspace re-exports (`name@workspace:path`) are emitted from the
            // `workspaces` section below, not here.
            if version.starts_with("workspace:") {
                continue;
            }
            let path = bun_key_to_npm_path(key);
            if !seen.insert(path.clone()) {
                continue;
            }
            // Bun stores per-package metadata as the first object element.
            let meta = arr.iter().find(|j| j.as_object().is_some());
            let mut entry: Vec<(String, Json)> =
                vec![("version".into(), Json::Str(version.to_string()))];
            if let Some(m) = meta {
                copy_dep_map(m, "dependencies", &mut entry);
                copy_dep_map(m, "optionalDependencies", &mut entry);
                copy_dep_map(m, "peerDependencies", &mut entry);
            }
            npm_pkgs.push((path, Json::Object(entry)));
        }
    }

    // --- workspace packages + the synthetic root's dependency set -----------
    let mut root_deps: Vec<(String, Json)> = Vec::new();
    let mut root_dep_names: HashSet<String> = HashSet::new();
    let mut root_name = String::from("root");
    let mut ws_names: HashSet<String> = HashSet::new();

    if let Some(ws) = doc.get("workspaces").and_then(Json::as_object) {
        // First pass: the root's declared name + the set of workspace names (so a
        // root that shares a package name with a workspace can be told apart).
        for (wpath, w) in ws {
            if wpath.is_empty() {
                if let Some(n) = w.get("name").and_then(Json::as_str) {
                    root_name = n.to_string();
                }
            } else if let Some(n) = w.get("name").and_then(Json::as_str) {
                ws_names.insert(n.to_string());
            }
        }
        // Second pass: root deps, and a node per non-root workspace.
        for (wpath, w) in ws {
            if wpath.is_empty() {
                merge_dep_names(w, &mut root_deps, &mut root_dep_names);
                continue;
            }
            let Some(name) = w.get("name").and_then(Json::as_str) else {
                continue;
            };
            // Every workspace is reachable from the repo root.
            if root_dep_names.insert(name.to_string()) {
                root_deps.push((name.to_string(), Json::Str("workspace:*".into())));
            }
            let path = format!("node_modules/{name}");
            if !seen.insert(path.clone()) {
                continue;
            }
            let version = w.get("version").and_then(Json::as_str).unwrap_or("0.0.0");
            let mut entry: Vec<(String, Json)> =
                vec![("version".into(), Json::Str(version.to_string()))];
            // A workspace's dev/runtime/optional deps are all installed, so merge
            // them as runtime edges; npm's lockfile parser does not mark dev edges.
            let deps = merged_workspace_deps(w);
            if !deps.is_empty() {
                entry.push(("dependencies".into(), Json::Object(deps)));
            }
            copy_dep_map(w, "peerDependencies", &mut entry);
            npm_pkgs.push((path, Json::Object(entry)));
        }
    }

    // Distinguish the aggregation root if its name also names a workspace package
    // (e.g. the repo root and `packages/foo` are both "foo"), so the root is not a
    // phantom duplicate-version family with that package.
    if ws_names.contains(&root_name) {
        root_name = format!("{root_name}-workspace-root");
    }

    // The root entry first (npm identifies it by the empty path either way).
    let mut packages: Vec<(String, Json)> = Vec::with_capacity(npm_pkgs.len() + 1);
    packages.push((
        String::new(),
        Json::Object(vec![
            ("name".into(), Json::Str(root_name.clone())),
            ("dependencies".into(), Json::Object(root_deps)),
        ]),
    ));
    packages.extend(npm_pkgs);

    Ok(Json::Object(vec![
        ("name".into(), Json::Str(root_name)),
        ("lockfileVersion".into(), Json::Int(3)),
        ("requires".into(), Json::Bool(true)),
        ("packages".into(), Json::Object(packages)),
    ]))
}

/// The version part of a Bun descriptor: `@scope/name@1.2.3` → `1.2.3`,
/// `7zip-bin@5.2.0` → `5.2.0`. The version follows the last `@` that is not the
/// leading scope marker. Returns `None` for a bare name (no version).
fn descriptor_version(descriptor: &str) -> Option<&str> {
    match descriptor.rfind('@') {
        Some(at) if at > 0 => Some(&descriptor[at + 1..]),
        _ => None,
    }
}

/// Re-encode a Bun `packages` key as an npm nested-`node_modules` path. Splits on
/// `/` into package-name *segments* (a leading `@scope` joins with the next
/// token), then joins with `/node_modules/`.
fn bun_key_to_npm_path(key: &str) -> String {
    let toks: Vec<&str> = key.split('/').collect();
    let mut segs: Vec<String> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        if toks[i].starts_with('@') && i + 1 < toks.len() {
            segs.push(format!("{}/{}", toks[i], toks[i + 1]));
            i += 2;
        } else {
            segs.push(toks[i].to_string());
            i += 1;
        }
    }
    format!("node_modules/{}", segs.join("/node_modules/"))
}

/// Clone `src[field]` into `entry` when it is a non-empty object (a dependency
/// map). The constraint *values* are copied verbatim — npm keys edges by name.
fn copy_dep_map(src: &Json, field: &str, entry: &mut Vec<(String, Json)>) {
    if let Some(v) = src.get(field) {
        if v.as_object().is_some_and(|o| !o.is_empty()) {
            entry.push((field.to_string(), v.clone()));
        }
    }
}

/// Merge a workspace's `dependencies` + `devDependencies` + `optionalDependencies`
/// into one name→constraint object (first wins on a name collision).
fn merged_workspace_deps(w: &Json) -> Vec<(String, Json)> {
    let mut out: Vec<(String, Json)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for field in ["dependencies", "devDependencies", "optionalDependencies"] {
        if let Some(m) = w.get(field).and_then(Json::as_object) {
            for (k, v) in m {
                if seen.insert(k.clone()) {
                    out.push((k.clone(), v.clone()));
                }
            }
        }
    }
    out
}

/// Like [`merged_workspace_deps`] but appends into the caller's accumulator
/// (used for the root's own declared dependencies).
fn merge_dep_names(w: &Json, out: &mut Vec<(String, Json)>, seen: &mut HashSet<String>) {
    for field in ["dependencies", "devDependencies", "optionalDependencies"] {
        if let Some(m) = w.get(field).and_then(Json::as_object) {
            for (k, v) in m {
                if seen.insert(k.clone()) {
                    out.push((k.clone(), v.clone()));
                }
            }
        }
    }
}

/// Strip JSONC (`//` line comments + trailing commas) to plain JSON, string-aware
/// so a `//` or `,` inside a string value is preserved. A trailing comma is
/// blanked to a space (O(1)) rather than removed, keeping the pass linear.
pub fn strip_jsonc(s: &str) -> String {
    let b = s.as_bytes();
    let n = b.len();
    let mut out: Vec<u8> = Vec::with_capacity(n);
    let mut i = 0;
    let mut in_str = false;
    // Byte offset in `out` of a comma awaiting its following token (only commas
    // outside strings; whitespace does not clear it, a real token does).
    let mut pending_comma: Option<usize> = None;

    while i < n {
        let c = b[i];
        if in_str {
            out.push(c);
            if c == b'\\' && i + 1 < n {
                out.push(b[i + 1]);
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                pending_comma = None; // a string token follows the comma — it's real
                out.push(c);
                in_str = true;
                i += 1;
            }
            b'/' if i + 1 < n && b[i + 1] == b'/' => {
                // Line comment: skip to (but keep) the newline.
                i += 2;
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
            }
            b',' => {
                pending_comma = Some(out.len());
                out.push(c);
                i += 1;
            }
            b'}' | b']' => {
                if let Some(p) = pending_comma.take() {
                    out[p] = b' '; // trailing comma → whitespace
                }
                out.push(c);
                i += 1;
            }
            b' ' | b'\t' | b'\r' | b'\n' => {
                out.push(c);
                i += 1;
            }
            _ => {
                pending_comma = None; // any other token makes the comma a real one
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_comments_and_trailing_commas() {
        let src = r#"{
            "a": 1, // a comment
            "b": [1, 2, 3,],
            "c": { "x": "y//z", "k": "a,b", },
        }"#;
        let out = strip_jsonc(src);
        let j = parse_json(&out).expect("valid JSON after strip");
        assert_eq!(j.get("a").and_then(Json::as_i64), Some(1));
        assert_eq!(j.get("b").and_then(Json::as_array).map(<[_]>::len), Some(3));
        // The `//` and `,` inside string values survive untouched.
        assert_eq!(j.pointer(["c", "x"]).and_then(Json::as_str), Some("y//z"));
        assert_eq!(j.pointer(["c", "k"]).and_then(Json::as_str), Some("a,b"));
    }

    #[test]
    fn key_to_path_handles_scope_and_nesting() {
        assert_eq!(bun_key_to_npm_path("foo"), "node_modules/foo");
        assert_eq!(bun_key_to_npm_path("@s/p"), "node_modules/@s/p");
        assert_eq!(
            bun_key_to_npm_path("foo/bar"),
            "node_modules/foo/node_modules/bar"
        );
        assert_eq!(
            bun_key_to_npm_path("a/@s/b"),
            "node_modules/a/node_modules/@s/b"
        );
        assert_eq!(
            bun_key_to_npm_path("@a/b/@c/d"),
            "node_modules/@a/b/node_modules/@c/d"
        );
    }

    #[test]
    fn descriptor_version_skips_scope_at() {
        assert_eq!(descriptor_version("7zip-bin@5.2.0"), Some("5.2.0"));
        assert_eq!(descriptor_version("@actions/core@1.11.1"), Some("1.11.1"));
        assert_eq!(descriptor_version("noversion"), None);
    }

    #[test]
    fn converts_a_small_bun_lock() {
        let src = r#"{
          "lockfileVersion": 1,
          "workspaces": {
            "": { "name": "app", "dependencies": { "left-pad": "1.3.0" } },
            "packages/lib": { "name": "@app/lib", "version": "0.1.0",
              "dependencies": { "left-pad": "1.3.0" } },
          },
          "packages": {
            "left-pad": ["left-pad@1.3.0", "", {}, "sha512-x"],
            "@app/lib": ["@app/lib@workspace:packages/lib", { "bin": {} }],
          },
        }"#;
        let v = bun_lock_to_npm_value(src.as_bytes()).expect("convert ok");
        let pkgs = v.get("packages").and_then(Json::as_object).unwrap();
        // root "", node_modules/left-pad, node_modules/@app/lib
        assert_eq!(pkgs.len(), 3);
        // The workspace re-export in `packages` was skipped (workspace:…).
        assert!(pkgs.iter().any(|(p, _)| p == "node_modules/left-pad"));
        assert!(pkgs.iter().any(|(p, _)| p == "node_modules/@app/lib"));
        // Root depends on the workspace + its own dep.
        let root = &pkgs.iter().find(|(p, _)| p.is_empty()).unwrap().1;
        let rdeps = root.get("dependencies").and_then(Json::as_object).unwrap();
        assert!(rdeps.iter().any(|(k, _)| k == "@app/lib"));
        assert!(rdeps.iter().any(|(k, _)| k == "left-pad"));
        // It reuses npm's resolver end to end.
        let graph = prism_plugin_npm::parse_package_lock_value(&v).unwrap();
        assert_eq!(graph.node_count(), 3);
    }
}
