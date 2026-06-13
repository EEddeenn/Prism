//! `Cargo.toml` manifest parsing into [`CanonicalEvent`]s.
//!
//! Reads `[package]` (name/version), the three dependency tables
//! (`[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`), their
//! `[dependencies.x]` long forms (version/features/optional/default-features/
//! package rename), `[features]`, and `[target.'cfg(...)'.<deps>]` blocks whose
//! `cfg(...)` key is recorded as an [`ConditionKind::Environment`] condition.
//!
//! Parsing is pure and side-effect free: it never resolves workspace
//! inheritance, fetches anything, or executes a build script. Unknown forms are
//! surfaced as [`ParseWarning`]s rather than failing the whole document.

use prism_model::{ConditionKind, DependencyKind};
use prism_plugin_api::{
    CanonicalBatch, CanonicalEvent, ManifestParser, PackageFact, PackageName, ParseError,
    ParseWarning, RawDocument, RequirementFact, VersionFact,
};
use prism_textfmt::{parse_toml, Toml};

/// A placeholder version used for the declaring package when `[package].version`
/// is absent (a virtual/workspace manifest); requirement facts still carry a
/// `src_version_raw`, so we need a stable sentinel.
const UNVERSIONED: &str = "0.0.0";

/// `Cargo.toml` parser.
pub struct CargoManifest;

impl ManifestParser for CargoManifest {
    fn parse_manifest(&self, raw: &RawDocument) -> Result<CanonicalBatch, ParseError> {
        let text = std::str::from_utf8(&raw.bytes)
            .map_err(|_| ParseError::new("not-utf8", "Cargo.toml is not utf-8"))?;
        parse_manifest_str(text)
    }
}

/// Parse a `Cargo.toml` string into a [`CanonicalBatch`]. Exposed for tests and
/// callers that already hold the text.
pub fn parse_manifest_str(text: &str) -> Result<CanonicalBatch, ParseError> {
    let doc = parse_toml(text).map_err(|e| ParseError::new("invalid-toml", e.to_string()))?;
    let mut batch = CanonicalBatch::new();

    // --- [package] -------------------------------------------------------
    let (pkg_name, pkg_version) = package_identity(&doc);
    batch.push(CanonicalEvent::PackageSeen(PackageFact {
        name: PackageName::new(pkg_name.clone()),
        namespace: None,
        repository_url: doc
            .path(["package", "repository"])
            .and_then(Toml::as_str)
            .map(str::to_string),
        homepage_url: doc
            .path(["package", "homepage"])
            .and_then(Toml::as_str)
            .map(str::to_string),
    }));
    batch.push(CanonicalEvent::VersionSeen(VersionFact {
        name: PackageName::new(pkg_name.clone()),
        version_raw: pkg_version.clone(),
        released_at_unix: None,
        license_expr: doc
            .path(["package", "license"])
            .and_then(Toml::as_str)
            .map(str::to_string),
        is_prerelease: pkg_version.contains('-'),
    }));

    let ctx = SrcCtx {
        name: pkg_name,
        version: pkg_version,
    };

    // --- top-level dependency tables ------------------------------------
    for (table, kind) in DEP_TABLES {
        if let Some(t) = doc.get(table).and_then(Toml::as_table) {
            parse_dep_table(&ctx, t, *kind, None, &mut batch);
        }
    }

    // --- [target.'cfg(...)'.<dep-table>] --------------------------------
    if let Some(targets) = doc.get("target").and_then(Toml::as_table) {
        for (cfg_key, cfg_val) in targets {
            for (table, kind) in DEP_TABLES {
                if let Some(t) = cfg_val.get(table).and_then(Toml::as_table) {
                    parse_dep_table(&ctx, t, *kind, Some(cfg_key.as_str()), &mut batch);
                }
            }
        }
    }

    // --- [features] ------------------------------------------------------
    // Recorded as warnings carrying the feature definition; the activation
    // closure is computed downstream from these. We surface each feature and
    // its enabled set so feature unification has the data without inventing a
    // bespoke event kind (the canonical event set has no FeatureFact).
    if let Some(features) = doc.get("features").and_then(Toml::as_table) {
        for (feat, members) in features {
            let enabled: Vec<String> = members
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Toml::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            batch.warn(
                ParseWarning::new("feature", format!("{feat} = {:?}", enabled))
                    .located(format!("features.{feat}")),
            );
        }
    }

    Ok(batch)
}

/// The three Cargo dependency tables and the [`DependencyKind`] each maps to.
const DEP_TABLES: &[(&str, DependencyKind)] = &[
    ("dependencies", DependencyKind::Runtime),
    ("dev-dependencies", DependencyKind::Dev),
    ("build-dependencies", DependencyKind::Build),
];

/// The declaring package's identity, threaded into every requirement fact.
struct SrcCtx {
    name: String,
    version: String,
}

/// Extract `(name, version)` from `[package]`, falling back to sentinels for a
/// virtual/workspace manifest with no `[package]` table.
fn package_identity(doc: &Toml) -> (String, String) {
    let name = doc
        .path(["package", "name"])
        .and_then(Toml::as_str)
        .unwrap_or("")
        .to_string();
    let version = doc
        .path(["package", "version"])
        .and_then(Toml::as_str)
        .unwrap_or(UNVERSIONED)
        .to_string();
    (name, version)
}

/// Parse one dependency table (each entry is `name = "<req>"` or
/// `name = { version, features, optional, default-features, package }`).
fn parse_dep_table(
    ctx: &SrcCtx,
    entries: &[(String, Toml)],
    kind: DependencyKind,
    raw_condition: Option<&str>,
    batch: &mut CanonicalBatch,
) {
    let condition_kind = if raw_condition.is_some() {
        ConditionKind::Environment
    } else {
        ConditionKind::None
    };

    for (dep_name, spec) in entries {
        let fact = match spec {
            // Shorthand: `dep = "1.2"`.
            Toml::Str(req) => RequirementFact {
                src_name: PackageName::new(ctx.name.clone()),
                src_version_raw: ctx.version.clone(),
                declared_target: dep_name.clone(),
                target_alias: None,
                raw_requirement: req.clone(),
                kind,
                scope: None,
                condition_kind,
                raw_condition: raw_condition.map(str::to_string),
                features_or_extras: Vec::new(),
            },
            // Long form: an inline/sub table.
            Toml::Table(fields) => {
                build_long_form(ctx, dep_name, fields, kind, raw_condition, condition_kind, batch)
            }
            other => {
                batch.warn(
                    ParseWarning::new(
                        "unexpected-dep-value",
                        format!("dependency `{dep_name}` has unexpected value {other:?}"),
                    )
                    .located(dep_name.clone()),
                );
                continue;
            }
        };
        batch.push(CanonicalEvent::RequirementSeen(fact));
    }
}

/// Build a [`RequirementFact`] from a long-form dependency table, honoring
/// `version`, `features`, `optional`, `default-features`, and `package`
/// (rename). An `optional = true` dependency is recorded as
/// [`DependencyKind::Optional`] regardless of which table it sits in.
#[allow(clippy::too_many_arguments)]
fn build_long_form(
    ctx: &SrcCtx,
    dep_name: &str,
    fields: &[(String, Toml)],
    base_kind: DependencyKind,
    raw_condition: Option<&str>,
    condition_kind: ConditionKind,
    batch: &mut CanonicalBatch,
) -> RequirementFact {
    let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v);

    let version = get("version")
        .and_then(Toml::as_str)
        .unwrap_or("*")
        .to_string();

    let optional = get("optional").and_then(Toml::as_bool).unwrap_or(false);
    let default_features = get("default-features")
        .and_then(Toml::as_bool)
        .unwrap_or(true);

    // `package = "real-name"` renames: declared_target is the real crate, the
    // local table key is the alias.
    let (declared_target, target_alias) = match get("package").and_then(Toml::as_str) {
        Some(real) => (real.to_string(), Some(dep_name.to_string())),
        None => (dep_name.to_string(), None),
    };

    let mut features: Vec<String> = get("features")
        .and_then(Toml::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Toml::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    // Record the explicit opt-out of default features as a synthetic marker so
    // downstream feature unification can distinguish "default off" from silence.
    if !default_features {
        batch.warn(
            ParseWarning::new(
                "default-features-off",
                format!("dependency `{dep_name}` disables default features"),
            )
            .located(dep_name.to_string()),
        );
    } else {
        // A dependency that keeps default features implicitly requests them.
        // We do not synthesize a `default` entry into `features` to avoid
        // double-counting; unification adds it when default-features is on.
        features.retain(|f| f != "default");
    }

    let kind = if optional {
        DependencyKind::Optional
    } else {
        base_kind
    };

    RequirementFact {
        src_name: PackageName::new(ctx.name.clone()),
        src_version_raw: ctx.version.clone(),
        declared_target,
        target_alias,
        raw_requirement: version,
        kind,
        scope: None,
        condition_kind,
        raw_condition: raw_condition.map(str::to_string),
        features_or_extras: features,
    }
}

/// Small helper to attach a location to a [`ParseWarning`].
trait Located {
    fn located(self, loc: impl Into<String>) -> Self;
}
impl Located for ParseWarning {
    fn located(mut self, loc: impl Into<String>) -> Self {
        self.location = Some(loc.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
        [package]
        name = "my-app"
        version = "0.3.1"
        license = "MIT"

        [dependencies]
        serde = { version = "1.0", features = ["derive"], optional = true }
        log = "0.4"
        rename_me = { version = "2", package = "real-crate", default-features = false }

        [dev-dependencies]
        criterion = "0.5"

        [build-dependencies]
        cc = "1.0"

        [features]
        default = ["std"]
        std = []
        extra = ["serde"]

        [target.'cfg(unix)'.dependencies]
        libc = "0.2"

        [target."cfg(windows)".dependencies]
        winapi = { version = "0.3", features = ["winuser"] }
    "#;

    fn reqs(batch: &CanonicalBatch) -> Vec<&RequirementFact> {
        batch
            .iter()
            .filter_map(|e| match e {
                CanonicalEvent::RequirementSeen(f) => Some(f),
                _ => None,
            })
            .collect()
    }

    fn find<'a>(rs: &'a [&'a RequirementFact], target: &str) -> &'a RequirementFact {
        rs.iter()
            .find(|f| f.declared_target == target)
            .unwrap_or_else(|| panic!("requirement for {target} not found"))
    }

    #[test]
    fn package_facts_emitted() {
        let b = parse_manifest_str(FIXTURE).unwrap();
        let pkg = b
            .iter()
            .find_map(|e| match e {
                CanonicalEvent::PackageSeen(p) => Some(p),
                _ => None,
            })
            .unwrap();
        assert_eq!(pkg.name.as_str(), "my-app");
        let ver = b
            .iter()
            .find_map(|e| match e {
                CanonicalEvent::VersionSeen(v) => Some(v),
                _ => None,
            })
            .unwrap();
        assert_eq!(ver.version_raw, "0.3.1");
        assert_eq!(ver.license_expr.as_deref(), Some("MIT"));
    }

    #[test]
    fn dependency_kinds_classified() {
        let b = parse_manifest_str(FIXTURE).unwrap();
        let rs = reqs(&b);
        // serde is optional -> Optional, overriding the Runtime table.
        assert_eq!(find(&rs, "serde").kind, DependencyKind::Optional);
        assert_eq!(find(&rs, "log").kind, DependencyKind::Runtime);
        assert_eq!(find(&rs, "criterion").kind, DependencyKind::Dev);
        assert_eq!(find(&rs, "cc").kind, DependencyKind::Build);
        assert_eq!(find(&rs, "libc").kind, DependencyKind::Runtime);
    }

    #[test]
    fn features_recorded_on_requirement() {
        let b = parse_manifest_str(FIXTURE).unwrap();
        let rs = reqs(&b);
        assert_eq!(find(&rs, "serde").features_or_extras, vec!["derive"]);
        assert_eq!(find(&rs, "real-crate").features_or_extras, Vec::<String>::new());
        assert_eq!(find(&rs, "winapi").features_or_extras, vec!["winuser"]);
        assert_eq!(find(&rs, "serde").raw_requirement, "1.0");
    }

    #[test]
    fn renamed_dependency_uses_package_field() {
        let b = parse_manifest_str(FIXTURE).unwrap();
        let rs = reqs(&b);
        let r = find(&rs, "real-crate");
        assert_eq!(r.declared_target, "real-crate");
        assert_eq!(r.target_alias.as_deref(), Some("rename_me"));
        assert_eq!(r.raw_requirement, "2");
    }

    #[test]
    fn target_conditions_recorded_as_environment() {
        let b = parse_manifest_str(FIXTURE).unwrap();
        let rs = reqs(&b);
        let libc = find(&rs, "libc");
        assert_eq!(libc.condition_kind, ConditionKind::Environment);
        assert_eq!(libc.raw_condition.as_deref(), Some("cfg(unix)"));
        let winapi = find(&rs, "winapi");
        assert_eq!(winapi.condition_kind, ConditionKind::Environment);
        assert_eq!(winapi.raw_condition.as_deref(), Some("cfg(windows)"));
        // A non-target dep has no condition.
        assert_eq!(find(&rs, "log").condition_kind, ConditionKind::None);
        assert!(find(&rs, "log").raw_condition.is_none());
    }

    #[test]
    fn features_table_surfaced_as_warnings() {
        let b = parse_manifest_str(FIXTURE).unwrap();
        let feats: Vec<&str> = b
            .iter()
            .filter_map(|e| match e {
                CanonicalEvent::Warning(w) if w.code == "feature" => w.location.as_deref(),
                _ => None,
            })
            .collect();
        assert!(feats.contains(&"features.default"));
        assert!(feats.contains(&"features.extra"));
    }

    #[test]
    fn virtual_manifest_has_no_package() {
        let src = r#"
            [workspace]
            members = ["a", "b"]
        "#;
        let b = parse_manifest_str(src).unwrap();
        // Still emits a (sentinel-named) package fact and no requirements.
        assert!(reqs(&b).is_empty());
    }

    #[test]
    fn rejects_non_utf8_via_trait() {
        let mp = CargoManifest;
        let raw = mk_raw(vec![0xff, 0xfe]);
        assert!(mp.parse_manifest(&raw).is_err());
    }

    fn mk_raw(bytes: Vec<u8>) -> RawDocument {
        use prism_plugin_api::{Provenance, SafeUrl};
        RawDocument {
            source_key: SafeUrl::new("file:///Cargo.toml"),
            media_type: None,
            content_hash: prism_model::HashDigest::default(),
            bytes,
            fetched_at_unix: 0,
            etag: None,
            last_modified: None,
            provenance: Provenance {
                source_id: "local".into(),
                retrieved_from: SafeUrl::new("file:///Cargo.toml"),
                from_cache: false,
                snapshot: None,
            },
        }
    }
}
