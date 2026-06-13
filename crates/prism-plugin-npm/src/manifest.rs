//! `package.json` manifest parsing (architecture doc, *npm Plugin*).
//!
//! [`NpmManifest::parse_manifest`] turns a `package.json` into a
//! [`CanonicalBatch`]:
//!
//! - one [`PackageFact`] for the declaring package (name + scope split out,
//!   `repository`/`homepage` URLs captured);
//! - one [`VersionFact`] for the declaring version (when `version` is present);
//! - a [`RequirementFact`] per entry of `dependencies` ([`DependencyKind::Runtime`]),
//!   `devDependencies` ([`DependencyKind::Dev`]), `optionalDependencies`
//!   ([`DependencyKind::Optional`]), and `peerDependencies`
//!   ([`DependencyKind::Peer`]);
//! - package-level `engines`/`os`/`cpu` captured as an **environment** condition
//!   string (see [`crate::cond`]) attached to every runtime requirement;
//! - `workspaces` globs surfaced as [`ParseWarning`]s carrying the pattern (the
//!   writer expands them against the filesystem, which the parser must not do).
//!
//! Fidelity: `peerDependenciesMeta.optional`, `bundleDependencies`, and
//! `overrides` are surfaced as warnings but not modeled as edges in v0.1.

use prism_model::{ConditionKind, DependencyKind};
use prism_plugin_api::{
    CanonicalBatch, CanonicalEvent, PackageFact, ParseError, ParseWarning, RawDocument,
    RequirementFact, VersionFact,
};
use prism_plugin_api::{ManifestParser, NameNormalizer};
use prism_semver::Version;
use prism_textfmt::{parse_json, Json};

use crate::name::NpmNames;

/// Build a [`ParseWarning`] with a location hint (the API's `new` omits it).
fn warn_at(code: &str, detail: impl Into<String>, location: &str) -> ParseWarning {
    ParseWarning {
        code: code.to_string(),
        detail: detail.into(),
        location: Some(location.to_string()),
    }
}

/// npm [`ManifestParser`] for `package.json`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NpmManifest;

/// One dependency-map field and the [`DependencyKind`] its entries carry.
const DEP_FIELDS: &[(&str, DependencyKind)] = &[
    ("dependencies", DependencyKind::Runtime),
    ("devDependencies", DependencyKind::Dev),
    ("optionalDependencies", DependencyKind::Optional),
    ("peerDependencies", DependencyKind::Peer),
];

impl ManifestParser for NpmManifest {
    fn parse_manifest(&self, raw: &RawDocument) -> Result<CanonicalBatch, ParseError> {
        let text = std::str::from_utf8(&raw.bytes)
            .map_err(|_| ParseError::new("not-utf8", "package.json is not utf-8"))?;
        let json = parse_json(text)
            .map_err(|e| ParseError::new("invalid-json", e.to_string()))?;
        let obj = match &json {
            Json::Object(_) => &json,
            _ => {
                return Err(ParseError::new(
                    "bad-manifest",
                    "package.json root is not an object",
                ))
            }
        };

        let mut batch = CanonicalBatch::new();

        // --- declaring package + version ---
        let pkg_name = obj.get("name").and_then(Json::as_str).map(str::to_string);
        let pkg_version = obj.get("version").and_then(Json::as_str).map(str::to_string);

        // The src name used on every requirement; default to "" for a private
        // root package with no name (npm allows this for app projects).
        let src_name = pkg_name.clone().unwrap_or_default();

        if let Some(name) = &pkg_name {
            // Validate the name; a malformed name is fatal for the manifest.
            let _ = NpmNames.normalize(name)?;
            let namespace = NpmNames::split_scope(name).map(|(s, _)| format!("@{s}"));
            batch.push(CanonicalEvent::PackageSeen(PackageFact {
                name: name.as_str().into(),
                namespace,
                repository_url: extract_repo_url(obj),
                homepage_url: obj.get("homepage").and_then(Json::as_str).map(str::to_string),
            }));
        }
        if let (Some(name), Some(version)) = (&pkg_name, &pkg_version) {
            let is_pre = Version::parse(version.trim_start_matches(['v', 'V']))
                .map(|v| v.is_prerelease())
                .unwrap_or(false);
            batch.push(CanonicalEvent::VersionSeen(VersionFact {
                name: name.as_str().into(),
                version_raw: version.clone(),
                released_at_unix: None,
                license_expr: obj.get("license").and_then(Json::as_str).map(str::to_string),
                is_prerelease: is_pre,
            }));
        }

        // --- environment condition from engines/os/cpu (runtime deps only) ---
        let env_condition = build_environment_condition(obj, &mut batch);

        // --- dependency maps ---
        for (field, kind) in DEP_FIELDS {
            let Some(map) = obj.get(field).and_then(Json::as_object) else {
                continue;
            };
            // Runtime deps inherit the package-level environment condition;
            // dev/optional/peer are gated by their category, not the platform.
            let condition = if *kind == DependencyKind::Runtime {
                env_condition.clone()
            } else {
                None
            };
            for (target, req) in map {
                let raw_requirement = match req.as_str() {
                    Some(s) => s.to_string(),
                    None => {
                        batch.warn(warn_at(
                            "non-string-requirement",
                            format!("requirement for `{target}` in `{field}` is not a string"),
                            field,
                        ));
                        continue;
                    }
                };
                if target.trim().is_empty() {
                    batch.warn(ParseWarning::new(
                        "empty-target",
                        format!("empty dependency name in `{field}`"),
                    ));
                    continue;
                }
                let (declared_target, target_alias) =
                    resolve_alias(target, &raw_requirement);
                batch.push(CanonicalEvent::RequirementSeen(RequirementFact {
                    src_name: src_name.as_str().into(),
                    src_version_raw: pkg_version.clone().unwrap_or_default(),
                    declared_target,
                    target_alias,
                    raw_requirement,
                    kind: *kind,
                    scope: None,
                    condition_kind: if condition.is_some() {
                        ConditionKind::Environment
                    } else {
                        ConditionKind::None
                    },
                    raw_condition: condition.clone(),
                    features_or_extras: Vec::new(),
                }));
            }
        }

        // --- peerDependenciesMeta (optional flag) — surfaced, not modeled ---
        if let Some(meta) = obj.get("peerDependenciesMeta").and_then(Json::as_object) {
            for (name, v) in meta {
                let optional = v.get("optional").and_then(Json::as_bool).unwrap_or(false);
                if optional {
                    batch.warn(warn_at(
                        "peer-optional",
                        format!("peer dependency `{name}` is marked optional"),
                        "peerDependenciesMeta",
                    ));
                }
            }
        }

        // --- bundleDependencies / overrides — surfaced, not modeled in v0.1 ---
        if obj.get("bundleDependencies").is_some() || obj.get("bundledDependencies").is_some() {
            batch.warn(ParseWarning::new(
                "bundle-deps-unmodeled",
                "bundleDependencies are recorded but not expanded in v0.1",
            ));
        }
        if obj.get("overrides").is_some() {
            batch.warn(ParseWarning::new(
                "overrides-unmodeled",
                "overrides are recorded but not applied in v0.1",
            ));
        }

        // --- workspaces ---
        emit_workspaces(obj, &mut batch);

        Ok(batch)
    }
}

/// Build the `os:…/cpu:…/node:…` environment condition string from package-level
/// `engines`, `os`, and `cpu`. Returns `None` when no platform constraints exist.
fn build_environment_condition(obj: &Json, batch: &mut CanonicalBatch) -> Option<String> {
    let mut clauses: Vec<String> = Vec::new();

    // `os` and `cpu` are arrays of names; a `!`-prefixed name is a blocklist.
    for (field, key) in [("os", "os"), ("cpu", "cpu")] {
        if let Some(arr) = obj.get(field).and_then(Json::as_array) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    let s = s.trim();
                    if !s.is_empty() {
                        clauses.push(format!("{key}:{s}"));
                    }
                }
            }
        } else if obj.get(field).is_some() {
            batch.warn(ParseWarning::new(
                "bad-platform-field",
                format!("`{field}` is expected to be an array of strings"),
            ));
        }
    }

    // `engines` is an object of `{ node: range, npm: range, … }`.
    if let Some(engines) = obj.get("engines").and_then(Json::as_object) {
        for (engine, range) in engines {
            if let Some(r) = range.as_str() {
                let key = match engine.as_str() {
                    "node" => "node",
                    "npm" => "npm",
                    _ => "engine",
                };
                clauses.push(format!("{key}:{}", r.trim()));
            }
        }
    }

    if clauses.is_empty() {
        None
    } else {
        Some(clauses.join(" && "))
    }
}

/// Emit `workspaces` patterns as warnings carrying each glob. Supports both the
/// array form and the `{ "packages": [...] }` object form.
fn emit_workspaces(obj: &Json, batch: &mut CanonicalBatch) {
    let patterns = match obj.get("workspaces") {
        Some(Json::Array(a)) => Some(a.as_slice()),
        Some(Json::Object(_)) => obj
            .pointer(["workspaces", "packages"])
            .and_then(Json::as_array),
        _ => None,
    };
    if let Some(pats) = patterns {
        for p in pats {
            if let Some(glob) = p.as_str() {
                batch.warn(warn_at("workspace-pattern", glob.to_string(), "workspaces"));
            }
        }
    }
}

/// Resolve npm's `npm:alias@range` aliasing. `dep: "npm:real-name@^1"` declares
/// a dependency under the local key `dep` that actually resolves `real-name`.
/// Returns `(declared_target, target_alias)`: the real target name and, when
/// aliased, the local alias key.
fn resolve_alias(local_key: &str, requirement: &str) -> (String, Option<String>) {
    if let Some(rest) = requirement.trim().strip_prefix("npm:") {
        // Split the *last* '@' so a scoped name like `@scope/x@^1` works.
        let real = match rest.rfind('@') {
            // An '@' at index 0 belongs to a scope, not a version separator.
            Some(at) if at > 0 => &rest[..at],
            _ => rest,
        };
        if !real.is_empty() && real != local_key {
            return (real.to_string(), Some(local_key.to_string()));
        }
    }
    (local_key.to_string(), None)
}

/// Pull a repository URL out of the `repository` field, which may be a bare
/// string or `{ "type": "git", "url": "…" }`.
fn extract_repo_url(obj: &Json) -> Option<String> {
    match obj.get("repository") {
        Some(Json::Str(s)) => Some(s.clone()),
        Some(repo @ Json::Object(_)) => repo.get("url").and_then(Json::as_str).map(str::to_string),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_model::HashDigest;
    use prism_plugin_api::{Provenance, SafeUrl};

    fn doc(bytes: &str) -> RawDocument {
        RawDocument {
            source_key: SafeUrl::new("file:///package.json"),
            media_type: Some("application/json".into()),
            content_hash: HashDigest::default(),
            bytes: bytes.as_bytes().to_vec(),
            fetched_at_unix: 0,
            etag: None,
            last_modified: None,
            provenance: Provenance {
                source_id: "local".into(),
                retrieved_from: SafeUrl::new("file:///package.json"),
                from_cache: false,
                snapshot: None,
            },
        }
    }

    fn reqs(batch: &CanonicalBatch) -> Vec<&RequirementFact> {
        batch
            .iter()
            .filter_map(|e| match e {
                CanonicalEvent::RequirementSeen(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    const SAMPLE: &str = r#"{
      "name": "@acme/app",
      "version": "1.2.0",
      "license": "MIT",
      "repository": { "type": "git", "url": "git+https://github.com/acme/app.git" },
      "homepage": "https://acme.example",
      "engines": { "node": ">=18", "npm": ">=9" },
      "os": ["linux", "!win32"],
      "cpu": ["x64"],
      "dependencies": {
        "left-pad": "^1.3.0",
        "react": "18.2.0"
      },
      "devDependencies": {
        "jest": "~29.0.0"
      },
      "optionalDependencies": {
        "fsevents": "^2.3.0"
      },
      "peerDependencies": {
        "react-dom": ">=17"
      },
      "peerDependenciesMeta": {
        "react-dom": { "optional": true }
      },
      "workspaces": ["packages/*", "tools/*"]
    }"#;

    #[test]
    fn parses_all_dependency_kinds() {
        let batch = NpmManifest.parse_manifest(&doc(SAMPLE)).unwrap();
        let rs = reqs(&batch);
        // 2 runtime + 1 dev + 1 optional + 1 peer = 5 requirements.
        assert_eq!(rs.len(), 5);

        let by_target = |t: &str| rs.iter().find(|r| r.declared_target == t).copied();

        let left = by_target("left-pad").unwrap();
        assert_eq!(left.kind, DependencyKind::Runtime);
        assert_eq!(left.raw_requirement, "^1.3.0");
        assert_eq!(left.src_name.as_str(), "@acme/app");
        assert_eq!(left.src_version_raw, "1.2.0");

        assert_eq!(by_target("jest").unwrap().kind, DependencyKind::Dev);
        assert_eq!(
            by_target("fsevents").unwrap().kind,
            DependencyKind::Optional
        );

        // The peer dependency carries DependencyKind::Peer.
        let peer = by_target("react-dom").unwrap();
        assert_eq!(peer.kind, DependencyKind::Peer);
        assert_eq!(peer.raw_requirement, ">=17");
    }

    #[test]
    fn runtime_deps_carry_environment_condition() {
        let batch = NpmManifest.parse_manifest(&doc(SAMPLE)).unwrap();
        let rs = reqs(&batch);
        let left = rs.iter().find(|r| r.declared_target == "left-pad").unwrap();
        assert_eq!(left.condition_kind, ConditionKind::Environment);
        let cond = left.raw_condition.as_deref().unwrap();
        assert!(cond.contains("os:linux"), "{cond}");
        assert!(cond.contains("os:!win32"), "{cond}");
        assert!(cond.contains("cpu:x64"), "{cond}");
        assert!(cond.contains("node:>=18"), "{cond}");
        assert!(cond.contains("npm:>=9"), "{cond}");

        // Dev/optional/peer deps are not platform-conditioned.
        let jest = rs.iter().find(|r| r.declared_target == "jest").unwrap();
        assert_eq!(jest.condition_kind, ConditionKind::None);
        assert!(jest.raw_condition.is_none());
    }

    #[test]
    fn captures_package_and_version_facts() {
        let batch = NpmManifest.parse_manifest(&doc(SAMPLE)).unwrap();
        let pkg = batch
            .iter()
            .find_map(|e| match e {
                CanonicalEvent::PackageSeen(p) => Some(p),
                _ => None,
            })
            .unwrap();
        assert_eq!(pkg.name.as_str(), "@acme/app");
        assert_eq!(pkg.namespace.as_deref(), Some("@acme"));
        assert_eq!(
            pkg.repository_url.as_deref(),
            Some("git+https://github.com/acme/app.git")
        );
        assert_eq!(pkg.homepage_url.as_deref(), Some("https://acme.example"));

        let ver = batch
            .iter()
            .find_map(|e| match e {
                CanonicalEvent::VersionSeen(v) => Some(v),
                _ => None,
            })
            .unwrap();
        assert_eq!(ver.version_raw, "1.2.0");
        assert_eq!(ver.license_expr.as_deref(), Some("MIT"));
        assert!(!ver.is_prerelease);
    }

    #[test]
    fn workspaces_are_surfaced_as_warnings() {
        let batch = NpmManifest.parse_manifest(&doc(SAMPLE)).unwrap();
        let globs: Vec<&str> = batch
            .iter()
            .filter_map(|e| match e {
                CanonicalEvent::Warning(w) if w.code == "workspace-pattern" => Some(w.detail.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(globs, vec!["packages/*", "tools/*"]);
    }

    #[test]
    fn workspaces_object_form() {
        let src = r#"{ "name": "root", "version": "1.0.0",
            "workspaces": { "packages": ["a/*"] } }"#;
        let batch = NpmManifest.parse_manifest(&doc(src)).unwrap();
        let has = batch.iter().any(|e| matches!(e,
            CanonicalEvent::Warning(w) if w.code == "workspace-pattern" && w.detail == "a/*"));
        assert!(has);
    }

    #[test]
    fn aliased_dependency_records_real_target_and_alias() {
        let src = r#"{
          "name": "app", "version": "1.0.0",
          "dependencies": { "lp": "npm:left-pad@^1.3.0", "scoped": "npm:@scope/x@^2" }
        }"#;
        let batch = NpmManifest.parse_manifest(&doc(src)).unwrap();
        let rs = reqs(&batch);
        let lp = rs.iter().find(|r| r.target_alias.as_deref() == Some("lp")).unwrap();
        assert_eq!(lp.declared_target, "left-pad");
        let sc = rs.iter().find(|r| r.target_alias.as_deref() == Some("scoped")).unwrap();
        assert_eq!(sc.declared_target, "@scope/x");
    }

    #[test]
    fn peer_optional_meta_warns() {
        let batch = NpmManifest.parse_manifest(&doc(SAMPLE)).unwrap();
        assert!(batch.iter().any(|e| matches!(e,
            CanonicalEvent::Warning(w) if w.code == "peer-optional")));
    }

    #[test]
    fn nameless_root_is_allowed() {
        let src = r#"{ "private": true, "dependencies": { "x": "^1.0.0" } }"#;
        let batch = NpmManifest.parse_manifest(&doc(src)).unwrap();
        let rs = reqs(&batch);
        assert_eq!(rs.len(), 1);
        assert_eq!(rs[0].src_name.as_str(), "");
        assert_eq!(rs[0].declared_target, "x");
    }

    #[test]
    fn rejects_non_object_root_and_bad_json() {
        assert!(NpmManifest.parse_manifest(&doc("[]")).is_err());
        assert!(NpmManifest.parse_manifest(&doc("not json")).is_err());
        // Bytes that are not UTF-8.
        let mut bad = doc("{}");
        bad.bytes = vec![0xff, 0xfe];
        assert!(NpmManifest.parse_manifest(&bad).is_err());
    }

    #[test]
    fn non_string_requirement_warns_and_skips() {
        let src = r#"{ "name": "a", "version": "1.0.0",
            "dependencies": { "x": 123, "y": "^1.0.0" } }"#;
        let batch = NpmManifest.parse_manifest(&doc(src)).unwrap();
        let rs = reqs(&batch);
        assert_eq!(rs.len(), 1);
        assert_eq!(rs[0].declared_target, "y");
        assert!(batch.iter().any(|e| matches!(e,
            CanonicalEvent::Warning(w) if w.code == "non-string-requirement")));
    }
}
