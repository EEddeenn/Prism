//! Ingestion: lockfile → persisted graph + run, and manifest → records.
//!
//! [`Prism::ingest_lockfile`] is the must-have vertical slice. It:
//!   1. detects the ecosystem from the file name;
//!   2. parses the lockfile into a [`ResolvedGraph`] plus an aligned identity
//!      column (`parse::parse_lockfile`);
//!   3. interns each occurrence's real `(name, version)` into store
//!      `PackageId`/`VersionId`, writing `Package`/`PackageVersion` records;
//!   4. builds a CSR [`GraphSnapshot`] with [`GraphBuilder`] (mapping each
//!      [`ResolvedNode`] → a [`BuildNode`] carrying the *real* `VersionId`, and
//!      each [`ResolvedEdge`] → a [`BuildEdge`]);
//!   5. persists it via [`Store::publish_graph`]; and
//!   6. records a `LockfileExact` [`ResolverRun`].
//!
//! [`Prism::ingest_manifest`] writes `Package`/`PackageVersion`/`Requirement`
//! records (and their indexes) from a manifest's canonical facts.

use std::collections::HashMap;
use std::path::Path;

use prism_codec::sha256;
use prism_graph::{write_pcsr, BuildEdge, BuildNode, GraphBuilder};
use prism_model::{
    ConditionKind, DependencyKind, EcosystemId, EnvironmentId, FidelityTier, GraphFidelity,
    Package, PackageId, PackageVersion, PrismError, Requirement, ResolverRun, Result, RunId,
    RunStatus, SnapshotId, StringId, VersionFlags, VersionId,
};
use prism_plugin_api::{
    CanonicalBatch, CanonicalEvent, EcosystemPlugin, PackageFact, RequirementFact, VersionFact,
};

use crate::ecosystem::Ecosystem;
use crate::parse::{parse_lockfile, NodeIdent};
use crate::Prism;

/// The outcome of ingesting a lockfile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockfileIngest {
    /// The run id whose graph was persisted.
    pub run: RunId,
    /// Ecosystem detected from the file name.
    pub ecosystem: Ecosystem,
    /// Number of occurrence nodes in the persisted graph.
    pub nodes: usize,
    /// Number of edges in the persisted graph.
    pub edges: usize,
}

/// The outcome of ingesting a manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManifestIngest {
    /// Ecosystem detected from the file name.
    pub ecosystem: Ecosystem,
    /// Number of `Package` records written.
    pub packages: usize,
    /// Number of `PackageVersion` records written.
    pub versions: usize,
    /// Number of `Requirement` records written.
    pub requirements: usize,
}

impl Prism {
    /// Ingest a lockfile at `path`, persisting a resolved graph snapshot and a
    /// `LockfileExact` resolver run. Returns the new [`RunId`].
    ///
    /// The ecosystem is detected from the file name; an unrecognized name is a
    /// [`PrismError::Other`]. Parsing never panics on hostile bytes.
    pub fn ingest_lockfile(&mut self, path: impl AsRef<Path>) -> Result<RunId> {
        self.ingest_lockfile_detailed(path).map(|r| r.run)
    }

    /// Like [`Prism::ingest_lockfile`] but returns node/edge counts too.
    pub fn ingest_lockfile_detailed(&mut self, path: impl AsRef<Path>) -> Result<LockfileIngest> {
        let path = path.as_ref();
        let eco = Ecosystem::detect(path).ok_or_else(|| {
            PrismError::Other(format!(
                "cannot detect ecosystem from file name {:?}",
                path.file_name()
            ))
        })?;
        if !Ecosystem::is_lockfile(path) {
            return Err(PrismError::Other(format!(
                "{:?} is not a recognized lockfile",
                path.file_name()
            )));
        }
        let bytes = prism_store::fsutil::read_file(path)?;
        self.ingest_lockfile_bytes(eco, &bytes)
    }

    /// Ingest pre-read lockfile `bytes` for a known ecosystem. Exposed for tests
    /// and callers that already hold the bytes.
    pub fn ingest_lockfile_bytes(
        &mut self,
        eco: Ecosystem,
        bytes: &[u8],
    ) -> Result<LockfileIngest> {
        let parsed = parse_lockfile(eco, bytes)?;
        let graph = &parsed.graph;

        // Intern every occurrence's real (name, version) -> VersionId, writing
        // Package/PackageVersion records. `real_version[local_id]` is the dense
        // node's true VersionId for the CSR builder.
        let mut real_version: Vec<VersionId> = Vec::with_capacity(parsed.idents.len());
        for ident in &parsed.idents {
            let vid = self.intern_occurrence(eco.id(), ident)?;
            real_version.push(vid);
        }

        // Allocate the run id before finalizing (the snapshot is keyed by it).
        let run = self.store.alloc_run_id();

        // Build the CSR snapshot: BuildNode per ResolvedNode (real VersionId +
        // depth + flags), BuildEdge per ResolvedEdge.
        let mut builder = GraphBuilder::new();
        for (i, node) in graph.nodes.iter().enumerate() {
            let bn = BuildNode {
                version_id: real_version[i],
                depth: node.depth,
                flags: node.flags,
                attr: None,
            };
            builder.add_node(bn)?;
        }
        for edge in &graph.edges {
            builder.add_edge(BuildEdge {
                from: prism_model::GraphNodeId::new(edge.from),
                to: prism_model::GraphNodeId::new(edge.to),
                ordinal: edge.ordinal,
                requirement_id: edge.requirement_id,
                kind: edge.kind,
                scope: StringId::new(0),
                flags: edge.flags,
            })?;
        }
        let snapshot = builder.finalize(run)?;
        let nodes = snapshot.node_count();
        let edges = snapshot.edge_count();

        // Persist the .pcsr image.
        let image = write_pcsr(&snapshot);
        self.store.publish_graph(run, &image)?;

        // Record the LockfileExact resolver run.
        let fidelity = graph
            .fidelity
            .unwrap_or_else(|| GraphFidelity::exact(FidelityTier::LockfileExact));
        self.write_run(run, eco.id(), bytes, fidelity)?;

        Ok(LockfileIngest {
            run,
            ecosystem: eco,
            nodes,
            edges,
        })
    }

    /// Ingest a manifest at `path`, writing `Package`/`PackageVersion`/
    /// `Requirement` records and their indexes.
    pub fn ingest_manifest(&mut self, path: impl AsRef<Path>) -> Result<ManifestIngest> {
        let path = path.as_ref();
        let eco = Ecosystem::detect(path).ok_or_else(|| {
            PrismError::Other(format!(
                "cannot detect ecosystem from file name {:?}",
                path.file_name()
            ))
        })?;
        if !Ecosystem::is_manifest(path) {
            return Err(PrismError::Other(format!(
                "{:?} is not a recognized manifest",
                path.file_name()
            )));
        }
        let bytes = prism_store::fsutil::read_file(path)?;
        self.ingest_manifest_bytes(eco, &bytes)
    }

    /// Ingest pre-read manifest `bytes` for a known ecosystem.
    pub fn ingest_manifest_bytes(
        &mut self,
        eco: Ecosystem,
        bytes: &[u8],
    ) -> Result<ManifestIngest> {
        let raw = crate::raw::raw_document("file:///manifest", bytes.to_vec());
        let batch = parse_manifest(eco, &raw)?;
        self.write_manifest_batch(eco.id(), &batch)
    }

    // --- internals -----------------------------------------------------------

    /// Find-or-create the `Package`/`PackageVersion` for one occurrence's
    /// identity, returning the concrete [`VersionId`]. Records are written only
    /// when newly created (interning is idempotent across calls and sessions).
    fn intern_occurrence(&mut self, eco: EcosystemId, ident: &NodeIdent) -> Result<VersionId> {
        let package_id = self.intern_package(eco, &ident.name, ident.namespace.as_deref())?;
        let version_raw = ident.version.as_deref().unwrap_or("0.0.0");
        self.intern_version(eco, package_id, &ident.name, ident.namespace.as_deref(), version_raw)
    }

    /// Find-or-create a package family, returning its [`PackageId`].
    fn intern_package(
        &mut self,
        eco: EcosystemId,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<PackageId> {
        let canonical = canonical_name(name, namespace);
        let canon_id = self.store.intern_string(&canonical)?;
        let ns_id = match namespace {
            Some(ns) => Some(self.store.intern_string(ns)?),
            None => None,
        };
        if let Some(existing) = self.store.find_package(eco, ns_id, canon_id) {
            return Ok(existing);
        }
        let name_id = self.store.intern_string(name)?;
        let purl_base = self.store.intern_string(&purl_base(eco, name, namespace))?;
        let pkg = Package {
            id: self.store.alloc_package_id(),
            ecosystem: eco,
            namespace: ns_id,
            name: name_id,
            canonical_name: canon_id,
            purl_base,
            repository_url: None,
            homepage_url: None,
            metadata_ref: None,
        };
        let id = pkg.id;
        let mut batch = self.store.batch();
        self.store.put_package(&mut batch, &pkg);
        self.store.commit_batch(batch)?;
        Ok(id)
    }

    /// Find-or-create a concrete version, returning its [`VersionId`].
    fn intern_version(
        &mut self,
        eco: EcosystemId,
        package_id: PackageId,
        name: &str,
        namespace: Option<&str>,
        version_raw: &str,
    ) -> Result<VersionId> {
        let purl = purl(eco, name, namespace, version_raw);
        let purl_id = self.store.intern_string(&purl)?;
        if let Some(existing) = self.store.find_version_by_purl(purl_id) {
            return Ok(existing);
        }
        let raw_id = self.store.intern_string(version_raw)?;
        let flags = if version_raw.contains('-') {
            VersionFlags::PRERELEASE
        } else {
            VersionFlags::EMPTY
        };
        let ver = PackageVersion {
            id: self.store.alloc_version_id(),
            package_id,
            version_raw: raw_id,
            version_norm: raw_id,
            version_sort_key: version_sort_key(version_raw),
            purl: purl_id,
            released_at_unix: None,
            flags,
            license_expr: None,
            artifact_hash: None,
            metadata_ref: None,
        };
        let id = ver.id;
        let mut batch = self.store.batch();
        self.store.put_version(&mut batch, &ver);
        self.store.commit_batch(batch)?;
        Ok(id)
    }

    /// Write the `ResolverRun` record for a lockfile ingest.
    fn write_run(
        &mut self,
        run: RunId,
        eco: EcosystemId,
        lockfile_bytes: &[u8],
        fidelity: GraphFidelity,
    ) -> Result<()> {
        let resolver_name = self.store.intern_string("lockfile")?;
        let lockfile_ref = self.store.blobs().put(lockfile_bytes)?;
        let record = ResolverRun {
            id: run,
            ecosystem: eco,
            root_set_hash: sha256(lockfile_bytes),
            resolver_name,
            resolver_version: None,
            // No content-addressed environment/snapshot in offline ingest; the
            // sentinels keep the record well-formed without fabricating data.
            environment_id: EnvironmentId::new(0),
            snapshot_id: SnapshotId::new(0),
            input_manifest_ref: None,
            input_lockfile_ref: Some(lockfile_ref),
            status: RunStatus::Complete,
            fidelity,
            reference_unix: 0,
            created_at_unix: 0,
            metadata_ref: None,
        };
        let mut batch = self.store.batch();
        self.store.put_run(&mut batch, &record);
        self.store.commit_batch(batch)?;
        Ok(())
    }

    /// Write a manifest's package/version/requirement facts into the store.
    fn write_manifest_batch(
        &mut self,
        eco: EcosystemId,
        batch: &CanonicalBatch,
    ) -> Result<ManifestIngest> {
        let mut packages = 0usize;
        let mut versions = 0usize;
        let mut requirements = 0usize;
        // Cache the (src_name@version) -> VersionId for requirement linkage.
        let mut version_of: HashMap<(String, String), VersionId> = HashMap::new();

        for event in batch.iter() {
            match event {
                CanonicalEvent::PackageSeen(fact) => {
                    packages += usize::from(self.write_package_fact(eco, fact)?);
                }
                CanonicalEvent::VersionSeen(fact) => {
                    let (vid, created) = self.write_version_fact(eco, fact)?;
                    if created {
                        versions += 1;
                    }
                    version_of.insert(
                        (fact.name.as_str().to_string(), fact.version_raw.clone()),
                        vid,
                    );
                }
                CanonicalEvent::RequirementSeen(fact) => {
                    let src = (fact.src_name.as_str().to_string(), fact.src_version_raw.clone());
                    let src_vid = match version_of.get(&src).copied() {
                        Some(v) => v,
                        None => {
                            // The declaring version wasn't emitted as a fact;
                            // intern it so the requirement has a source.
                            let src_pkg = self.intern_package(eco, fact.src_name.as_str(), None)?;
                            let vid = self.intern_version(
                                eco,
                                src_pkg,
                                fact.src_name.as_str(),
                                None,
                                &fact.src_version_raw,
                            )?;
                            version_of.insert(src, vid);
                            vid
                        }
                    };
                    self.write_requirement_fact(eco, src_vid, fact)?;
                    requirements += 1;
                }
                _ => {}
            }
        }

        Ok(ManifestIngest {
            ecosystem: eco_of(eco),
            packages,
            versions,
            requirements,
        })
    }

    /// Intern a package fact; returns `true` if it was newly created.
    fn write_package_fact(&mut self, eco: EcosystemId, fact: &PackageFact) -> Result<bool> {
        let canon = canonical_name(fact.name.as_str(), fact.namespace.as_deref());
        let canon_id = self.store.intern_string(&canon)?;
        let ns_id = match &fact.namespace {
            Some(ns) => Some(self.store.intern_string(ns)?),
            None => None,
        };
        if self.store.find_package(eco, ns_id, canon_id).is_some() {
            return Ok(false);
        }
        self.intern_package(eco, fact.name.as_str(), fact.namespace.as_deref())?;
        Ok(true)
    }

    /// Intern a version fact; returns its id and whether it was newly created.
    fn write_version_fact(
        &mut self,
        eco: EcosystemId,
        fact: &VersionFact,
    ) -> Result<(VersionId, bool)> {
        let pkg = self.intern_package(eco, fact.name.as_str(), None)?;
        let purl_id = self
            .store
            .intern_string(&purl(eco, fact.name.as_str(), None, &fact.version_raw))?;
        if let Some(existing) = self.store.find_version_by_purl(purl_id) {
            return Ok((existing, false));
        }
        let vid = self.intern_version(eco, pkg, fact.name.as_str(), None, &fact.version_raw)?;
        Ok((vid, true))
    }

    /// Write a `Requirement` record linking `src_vid` to its declared target.
    fn write_requirement_fact(
        &mut self,
        eco: EcosystemId,
        src_vid: VersionId,
        fact: &RequirementFact,
    ) -> Result<()> {
        let declared_target = self.store.intern_string(&fact.declared_target)?;
        let raw_requirement = self.store.intern_string(&fact.raw_requirement)?;
        let target_alias = match &fact.target_alias {
            Some(a) => Some(self.store.intern_string(a)?),
            None => None,
        };
        let scope = match &fact.scope {
            Some(s) => Some(self.store.intern_string(s)?),
            None => None,
        };
        // The declared target's resolved family (so REQS_BY_DST_PACKAGE indexes).
        let dst = self.intern_package(eco, &fact.declared_target, None).ok();
        let constraint_ref = self.store.blobs().put(fact.raw_requirement.as_bytes())?;
        let condition_ref = match &fact.raw_condition {
            Some(c) => Some(self.store.blobs().put(c.as_bytes())?),
            None => None,
        };
        let features_or_extras_ref = if fact.features_or_extras.is_empty() {
            None
        } else {
            Some(self.store.blobs().put(fact.features_or_extras.join(",").as_bytes())?)
        };
        let req = Requirement {
            id: self.store.alloc_requirement_id(),
            src_version_id: src_vid,
            declared_target,
            dst_package_id: dst,
            target_alias,
            raw_requirement,
            constraint_ref,
            managed_version_ref: None,
            kind: map_kind(fact.kind),
            scope,
            condition_kind: map_condition(fact.condition_kind),
            condition_ref,
            features_or_extras_ref,
            metadata_ref: None,
        };
        let mut batch = self.store.batch();
        self.store.put_requirement(&mut batch, &req);
        self.store.commit_batch(batch)?;
        Ok(())
    }
}

/// Parse a manifest [`RawDocument`] for `eco` via its plugin's [`ManifestParser`].
fn parse_manifest(
    eco: Ecosystem,
    raw: &prism_plugin_api::RawDocument,
) -> Result<CanonicalBatch> {
    let batch = match eco {
        Ecosystem::Cargo => {
            let plugin = Ecosystem::cargo_plugin();
            let parser = plugin
                .manifest_parser()
                .ok_or_else(|| PrismError::Other("cargo has no manifest parser".into()))?;
            parser.parse_manifest(raw)?
        }
        Ecosystem::Npm => {
            let plugin = Ecosystem::npm_plugin();
            let parser = plugin
                .manifest_parser()
                .ok_or_else(|| PrismError::Other("npm has no manifest parser".into()))?;
            parser.parse_manifest(raw)?
        }
    };
    Ok(batch)
}

/// The ecosystem owning a known id (the facade only registers two).
fn eco_of(id: EcosystemId) -> Ecosystem {
    if id == Ecosystem::Npm.id() {
        Ecosystem::Npm
    } else {
        Ecosystem::Cargo
    }
}

/// The canonical identity key for a `(name, namespace)`. We fold ASCII case so
/// the key is stable for offline ingest without invoking the plugin normalizer
/// (which can reject names); a scoped name keeps the `@scope/` prefix.
fn canonical_name(name: &str, namespace: Option<&str>) -> String {
    match namespace {
        Some(ns) => format!("@{}/{}", ns.to_ascii_lowercase(), name.to_ascii_lowercase()),
        None => name.to_ascii_lowercase(),
    }
}

/// The purl *base* (`pkg:<eco>/<name>`), without a version.
fn purl_base(eco: EcosystemId, name: &str, namespace: Option<&str>) -> String {
    let scheme = if eco == Ecosystem::Npm.id() { "npm" } else { "cargo" };
    match namespace {
        Some(ns) => format!("pkg:{scheme}/@{ns}%2F{name}"),
        None => format!("pkg:{scheme}/{name}"),
    }
}

/// The full purl (`pkg:<eco>/<name>@<version>`).
fn purl(eco: EcosystemId, name: &str, namespace: Option<&str>, version: &str) -> String {
    format!("{}@{}", purl_base(eco, name, namespace), version)
}

/// A simple, total-order byte key for a dotted version string. Each
/// dot/hyphen-separated component is length-prefixed and numeric components are
/// zero-padded to sort numerically; this is a display/sort convenience, not the
/// ecosystem's authoritative scheme.
fn version_sort_key(version: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(version.len() + 8);
    for part in version.split(['.', '-', '+']) {
        if let Ok(n) = part.parse::<u64>() {
            key.push(0x00); // numeric components sort before alpha at equal pos
            key.extend_from_slice(&n.to_be_bytes());
        } else {
            key.push(0x01);
            key.extend_from_slice(part.as_bytes());
        }
        key.push(0xFF); // component separator
    }
    key
}

/// Map a plugin [`DependencyKind`] through (identity; the model owns the enum).
fn map_kind(kind: DependencyKind) -> DependencyKind {
    kind
}

/// Map a plugin [`ConditionKind`] through (identity).
fn map_condition(kind: ConditionKind) -> ConditionKind {
    kind
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_name_folds_case_and_scope() {
        assert_eq!(canonical_name("Serde", None), "serde");
        assert_eq!(canonical_name("Left-Pad", Some("Scope")), "@scope/left-pad");
    }

    #[test]
    fn purl_shapes() {
        assert_eq!(purl_base(CARGO, "serde", None), "pkg:cargo/serde");
        assert_eq!(purl(CARGO, "serde", None, "1.0.0"), "pkg:cargo/serde@1.0.0");
        assert_eq!(
            purl(NPM, "left-pad", Some("scope"), "1.3.0"),
            "pkg:npm/@scope%2Fleft-pad@1.3.0"
        );
    }

    #[test]
    fn version_sort_key_orders_numerically() {
        let a = version_sort_key("0.9.0");
        let b = version_sort_key("0.27.0");
        let c = version_sort_key("1.0.0");
        assert!(a < b, "0.9.0 < 0.27.0 numerically");
        assert!(b < c);
    }

    const CARGO: EcosystemId = EcosystemId::new(1);
    const NPM: EcosystemId = EcosystemId::new(2);
}
