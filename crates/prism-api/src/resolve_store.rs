//! Store-backed adapters wiring [`prism_resolve::Engine`] to a live workspace
//! (architecture doc, *Resolution and Graph Construction* — the synthetic path).
//!
//! The engine is store-agnostic: it pulls on two traits. These adapters back
//! them with the store's indexes:
//!
//! - [`StoreRequirementSource`] reads `REQS_BY_SRC_VER` and maps each stored
//!   [`prism_model::Requirement`] to a [`prism_resolve::DeclaredRequirement`].
//! - [`StoreCandidateSource`] selects a [`VersionId`] from `VERS_BY_PACKAGE` under
//!   the run policy (lowest / highest-non-prerelease / locked) using SemVer
//!   matching over the snapshot-pinned version list.
//!
//! [`Prism::resolve_synthetic_from_roots`] runs the engine from a set of root
//! [`VersionId`]s and persists the resulting [`prism_resolve::ResolvedGraph`] via
//! the SAME graph-build + publish path `ingest_lockfile` uses, recording a
//! `ModelResolved` [`prism_model::ResolverRun`].

use std::collections::HashMap;

use prism_model::{
    ConditionKind, EnvironmentId, PackageId, Requirement, VersionFlags, VersionId,
};
use prism_resolve::{
    ActivationContext, CandidateSource, DeclaredRequirement, ResolveError, ResolvePolicy,
    RequirementSource,
};
use prism_semver::{Version, VersionReq};
use prism_store::Store;

/// A [`RequirementSource`] reading declared requirements straight from the store.
pub(crate) struct StoreRequirementSource<'a> {
    store: &'a Store,
}

impl<'a> StoreRequirementSource<'a> {
    /// Wrap a store borrow.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Map a stored [`Requirement`] to a resolver [`DeclaredRequirement`], or
    /// `None` if it has no resolved target package (so no edge can be drawn).
    fn to_declared(&self, req: &Requirement) -> Option<DeclaredRequirement> {
        let target = req.dst_package_id?;
        let matcher = self
            .store
            .resolve_string(req.raw_requirement)
            .unwrap_or_else(|| "*".to_string());
        let condition = req
            .condition_ref
            .and_then(|b| self.store.blobs().get(&b).ok())
            .and_then(|bytes| String::from_utf8(bytes).ok());
        let features = req
            .features_or_extras_ref
            .and_then(|b| self.store.blobs().get(&b).ok())
            .and_then(|bytes| String::from_utf8(bytes).ok());
        let activates = features
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Some(DeclaredRequirement {
            requirement_id: req.id,
            target,
            matcher,
            kind: req.kind,
            condition_kind: req.condition_kind,
            // Only carry the condition string when it actually gates (the engine
            // treats `None` as unconditional).
            condition: if req.condition_kind == ConditionKind::None {
                None
            } else {
                condition
            },
            activates,
        })
    }
}

impl RequirementSource for StoreRequirementSource<'_> {
    fn requirements(&self, version: VersionId) -> Result<Vec<DeclaredRequirement>, ResolveError> {
        let mut out = Vec::new();
        for rid in self.store.requirements_by_source_version(version) {
            let req = self
                .store
                .get_requirement(rid)
                .map_err(|e| ResolveError::Other(format!("load requirement {}: {e}", rid.get())))?;
            if let Some(d) = self.to_declared(&req) {
                out.push(d);
            }
        }
        Ok(out)
    }

    fn package_of(&self, version: VersionId) -> Result<PackageId, ResolveError> {
        self.store
            .get_version(version)
            .map(|v| v.package_id)
            .map_err(|e| ResolveError::Other(format!("load version {}: {e}", version.get())))
    }
}

/// A [`CandidateSource`] selecting a version from the store under a policy.
///
/// Versions of a package come from `VERS_BY_PACKAGE`; each is parsed as SemVer
/// and matched against the requirement. The lock map (for [`ResolvePolicy::Locked`])
/// is seeded by the caller from a prior lockfile run.
pub(crate) struct StoreCandidateSource<'a> {
    store: &'a Store,
    /// Optional pins for the `Locked` policy: package family → chosen version.
    locked: HashMap<PackageId, VersionId>,
}

impl<'a> StoreCandidateSource<'a> {
    /// Wrap a store borrow with no lock pins.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self {
            store,
            locked: HashMap::new(),
        }
    }

    /// Pin `package` to `version` for [`ResolvePolicy::Locked`].
    pub(crate) fn lock(&mut self, package: PackageId, version: VersionId) {
        self.locked.insert(package, version);
    }

    /// Seed [`ResolvePolicy::Locked`] pins for every package family the store
    /// knows, pinning each to its highest known non-yanked version. This makes
    /// `locked` mean "use the versions already on hand" when no external lockfile
    /// drove the pins — a deterministic, reproducible choice for a manifest-only
    /// workspace. (`roots` is accepted for symmetry but the scan is whole-store.)
    pub(crate) fn seed_locks_from_store(&mut self, _roots: &[VersionId]) {
        use std::collections::BTreeSet;
        // Every distinct package family that owns at least one stored version.
        let mut families: BTreeSet<u64> = BTreeSet::new();
        if let Ok(records) = self.store.read_all_records() {
            for rec in records {
                if rec.record_type != prism_store::record_type::PACKAGE_VERSION {
                    continue;
                }
                if let Ok(v) = prism_store::records::decode_package_version(&rec.value) {
                    families.insert(v.package_id.get());
                }
            }
        }
        for fam in families {
            let pkg = PackageId::new(fam);
            if let Some((vid, _)) = self.versions(pkg).into_iter().next_back() {
                self.lock(pkg, vid);
            }
        }
    }

    /// `(version_id, parsed-version)` of `package`, ascending, skipping yanked and
    /// unparseable versions.
    fn versions(&self, package: PackageId) -> Vec<(VersionId, Version)> {
        let mut out: Vec<(VersionId, Version)> = self
            .store
            .versions_by_package(package)
            .into_iter()
            .filter_map(|vid| {
                let pv = self.store.get_version(vid).ok()?;
                if pv.flags.contains(VersionFlags::YANKED) {
                    return None;
                }
                let raw = self.store.resolve_string(pv.version_raw)?;
                let v = Version::parse(&raw).ok()?;
                Some((vid, v))
            })
            .collect();
        out.sort_by(|a, b| a.1.cmp(&b.1));
        out
    }

    /// Parse a requirement matcher the way candidate analysis does: route the raw
    /// constraint through the owning ecosystem's parser (so a bare Cargo `"1.0"`
    /// becomes `^1.0`) before handing it to the shared [`VersionReq`] engine. An
    /// empty/`*` matcher means "any". Falls back to the direct parse if the
    /// package's ecosystem cannot be loaded.
    fn parse_matcher(
        &self,
        package: PackageId,
        matcher: &str,
    ) -> Result<VersionReq, ResolveError> {
        let m = matcher.trim();
        if m.is_empty() || m == "*" {
            return Ok(VersionReq::any());
        }
        match self.store.get_package(package) {
            Ok(pkg) => {
                let eco = crate::candidates::eco_of(pkg.ecosystem);
                crate::candidates::parse_constraint(eco, m)
                    .map_err(|e| ResolveError::Other(format!("bad matcher {matcher}: {e}")))
            }
            // No package record (shouldn't happen for a selected target): fall back
            // to the strict engine parse rather than failing the whole resolve.
            Err(_) => VersionReq::parse(m)
                .map_err(|e| ResolveError::Other(format!("bad matcher {matcher}: {e}"))),
        }
    }
}

impl CandidateSource for StoreCandidateSource<'_> {
    fn select(
        &self,
        package: PackageId,
        matcher: &str,
        _environment: EnvironmentId,
        _activation: &ActivationContext,
        policy: &ResolvePolicy,
    ) -> Result<Option<VersionId>, ResolveError> {
        // Locked replays the recorded pin and ignores the matcher.
        if matches!(policy, ResolvePolicy::Locked) {
            return Ok(self.locked.get(&package).copied());
        }

        let req = self.parse_matcher(package, matcher)?;
        let versions = self.versions(package);
        let satisfying: Vec<&(VersionId, Version)> =
            versions.iter().filter(|(_, v)| req.matches(v)).collect();
        if satisfying.is_empty() {
            return Ok(None);
        }

        let chosen = match policy {
            ResolvePolicy::Lowest => satisfying.first(),
            ResolvePolicy::Latest => satisfying.last(),
            ResolvePolicy::HighestNonPrerelease => satisfying
                .iter()
                .rev()
                .find(|(_, v)| !v.is_prerelease())
                .or_else(|| satisfying.last()),
            // Any other policy defaults to "latest satisfying".
            _ => satisfying.last(),
        };
        Ok(chosen.map(|(id, _)| *id))
    }
}

/// Map a CLI/string policy label to a [`ResolvePolicy`]. Unknown labels are an
/// error so a typo never silently picks the wrong policy.
pub(crate) fn policy_from_label(label: &str) -> Result<ResolvePolicy, ResolveError> {
    match label {
        "locked" => Ok(ResolvePolicy::Locked),
        "lowest" => Ok(ResolvePolicy::Lowest),
        "latest" => Ok(ResolvePolicy::Latest),
        "highest" | "highest-non-prerelease" => Ok(ResolvePolicy::HighestNonPrerelease),
        other => Err(ResolveError::Other(format!("unknown policy {other:?}"))),
    }
}
