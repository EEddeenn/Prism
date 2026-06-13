//! Cargo ecosystem plugin (architecture doc, *Cargo Plugin*).
//!
//! An offline, network-free [`prism_plugin_api::EcosystemPlugin`] for Rust's
//! Cargo. It parses `Cargo.toml` manifests and `Cargo.lock` lockfiles entirely
//! from local bytes via [`prism_textfmt::parse_toml`], and delegates version /
//! requirement semantics to the shared [`prism_semver`] engine. Nothing here
//! executes a build script, calls `cargo`, or touches the network.
//!
//! ## Capabilities
//!
//! - [`names::CargoNames`] — crates.io name rules and canonical identity.
//! - [`version::CargoVersions`] — SemVer 2.0 parse/compare/sort-key.
//! - [`req::CargoReqs`] — the Cargo range dialect (`^`, `~`, `=`, `*`, AND sets).
//! - [`condition::CargoConditions`] — `cfg(...)`/target conditions (environment).
//! - [`detect::CargoDetector`] — `Cargo.toml` project detection.
//! - [`manifest::CargoManifest`] — `[dependencies]`, `[dev-/build-dependencies]`,
//!   long-form `[dependencies.x]`, `[features]`, `[target.'cfg(...)'.…]`.
//! - [`lockfile::CargoLockfile`] — `Cargo.lock` → [`prism_plugin_api::ResolvedGraph`]
//!   (`LockfileExact`) and canonical facts.
//! - [`candidates::CargoCandidates`] — offline candidate enumeration.
//!
//! The [`CargoPlugin`] aggregator is object-safe and storable as
//! `Box<dyn EcosystemPlugin>`.

pub mod candidates;
pub mod condition;
pub mod detect;
pub mod lockfile;
pub mod manifest;
pub mod names;
pub mod plugin;
pub mod req;
pub mod version;

pub use candidates::CargoCandidates;
pub use condition::CargoConditions;
pub use detect::CargoDetector;
pub use lockfile::{
    parse_lockfile_batch, parse_lockfile_graph, parse_lockfile_packages, CargoLockfile, LockPackage,
};
pub use manifest::{parse_manifest_str, CargoManifest};
pub use names::CargoNames;
pub use plugin::CargoPlugin;
pub use req::CargoReqs;
pub use version::CargoVersions;

/// Compile-time object-safety assertions. Each only type-checks if the named
/// trait is object-safe for this crate's concrete impls. Never called.
#[allow(dead_code)]
mod object_safety {
    use super::*;
    use prism_plugin_api::EcosystemPlugin;

    fn _boxed_plugin(p: Box<dyn EcosystemPlugin>) -> Box<dyn EcosystemPlugin> {
        p
    }

    fn _is_plugin(_: &dyn EcosystemPlugin) {}

    fn _assert() {
        let p = CargoPlugin::new(prism_model::EcosystemId::new(2));
        _is_plugin(&p);
        let _boxed: Box<dyn EcosystemPlugin> = Box::new(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_model::{ConditionKind, DependencyKind, EcosystemId, FidelityTier};
    use prism_plugin_api::{CanonicalEvent, EcosystemPlugin, RequirementFact, VersionScheme};

    const CARGO_ECO: EcosystemId = EcosystemId::new(2);

    const MANIFEST: &str = r#"
        [package]
        name = "demo"
        version = "1.2.3"

        [dependencies]
        serde = { version = "1.0", features = ["derive"], optional = true }
        anyhow = "1"

        [dev-dependencies]
        proptest = "1.4"

        [build-dependencies]
        cc = "1"

        [features]
        default = ["serde"]

        [target.'cfg(target_os = "linux")'.dependencies]
        nix = { version = "0.27", features = ["fs"] }
    "#;

    const LOCK: &str = r#"
        version = 3

        [[package]]
        name = "demo"
        version = "1.2.3"
        dependencies = ["anyhow", "serde 1.0.197"]

        [[package]]
        name = "anyhow"
        version = "1.0.80"
        source = "registry+https://github.com/rust-lang/crates.io-index"

        [[package]]
        name = "serde"
        version = "1.0.197"
        source = "registry+https://github.com/rust-lang/crates.io-index"
    "#;

    fn requirement_facts(text: &str) -> Vec<RequirementFact> {
        parse_manifest_str(text)
            .unwrap()
            .events
            .into_iter()
            .filter_map(|e| match e {
                CanonicalEvent::RequirementSeen(f) => Some(f),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn manifest_requirement_facts_have_kinds_conditions_features() {
        let facts = requirement_facts(MANIFEST);
        let by = |t: &str| -> RequirementFact {
            facts
                .iter()
                .find(|f| f.declared_target == t)
                .cloned()
                .unwrap_or_else(|| panic!("missing {t}"))
        };

        // Optional dep keeps its requested features and is reclassified Optional.
        let serde = by("serde");
        assert_eq!(serde.kind, DependencyKind::Optional);
        assert_eq!(serde.features_or_extras, vec!["derive"]);
        assert_eq!(serde.condition_kind, ConditionKind::None);

        // Plain runtime / dev / build classification.
        assert_eq!(by("anyhow").kind, DependencyKind::Runtime);
        assert_eq!(by("proptest").kind, DependencyKind::Dev);
        assert_eq!(by("cc").kind, DependencyKind::Build);

        // Target-conditioned dep records the cfg as an environment condition.
        let nix = by("nix");
        assert_eq!(nix.condition_kind, ConditionKind::Environment);
        assert_eq!(
            nix.raw_condition.as_deref(),
            Some("cfg(target_os = \"linux\")")
        );
        assert_eq!(nix.features_or_extras, vec!["fs"]);
    }

    #[test]
    fn lockfile_resolved_node_set_and_edges() {
        let g = parse_lockfile_graph(LOCK).unwrap();
        assert_eq!(g.node_count(), 3);
        assert_eq!(g.fidelity.unwrap().tier, FidelityTier::LockfileExact);

        // Node set by occurrence key.
        let mut keys: Vec<&str> = g.nodes.iter().map(|n| n.occurrence_key.as_str()).collect();
        keys.sort();
        assert_eq!(keys, ["anyhow 1.0.80", "demo 1.2.3", "serde 1.0.197"]);

        // demo -> anyhow (bare), demo -> serde 1.0.197 (versioned).
        assert_eq!(g.edge_count(), 2);
        let targets: Vec<&str> = g
            .edges
            .iter()
            .map(|e| g.nodes[e.to as usize].occurrence_key.as_str())
            .collect();
        assert!(targets.contains(&"anyhow 1.0.80"));
        assert!(targets.contains(&"serde 1.0.197"));
        // All edges originate from the root member `demo`.
        for e in &g.edges {
            assert_eq!(g.nodes[e.from as usize].occurrence_key, "demo 1.2.3");
        }
    }

    #[test]
    fn semver_ordering_via_the_scheme() {
        let vs = CargoVersions;
        let mut versions = ["0.27.1", "0.27.0-rc.1", "1.0.0", "0.9.0", "0.27.0"];
        versions.sort_by(|a, b| vs.compare(a, b).unwrap());
        assert_eq!(
            versions,
            ["0.9.0", "0.27.0-rc.1", "0.27.0", "0.27.1", "1.0.0"]
        );
        // Through the boxed plugin's dyn accessor too.
        let p: Box<dyn EcosystemPlugin> = Box::new(CargoPlugin::new(CARGO_ECO));
        assert_eq!(
            p.version_scheme().compare("1.10.0", "1.2.0").unwrap(),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn object_safety_and_plugin_wiring() {
        // Storable as a trait object, capabilities self-consistent.
        let p: Box<dyn EcosystemPlugin> = Box::new(CargoPlugin::new(CARGO_ECO));
        assert!(p.self_consistent());
        assert_eq!(p.id(), CARGO_ECO);

        // End-to-end through dyn: parse the manifest via the trait object.
        let mp = p.manifest_parser().expect("manifest capability");
        let raw = mk_raw(MANIFEST.as_bytes().to_vec());
        let batch = mp.parse_manifest(&raw).unwrap();
        assert!(batch.fact_count() >= 5);

        // Name normalization via dyn.
        assert_eq!(
            p.name_normalizer().normalize("Demo-Crate").unwrap(),
            "demo_crate"
        );
    }

    fn mk_raw(bytes: Vec<u8>) -> prism_plugin_api::RawDocument {
        use prism_plugin_api::{Provenance, RawDocument, SafeUrl};
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
