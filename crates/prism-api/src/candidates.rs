//! Candidate-version index and conflict-pressure analysis over the store
//! (architecture doc, *Candidate Version Index* and *Conflict Pressure*).
//!
//! [`Prism::candidates_for`] loads a package's known versions, sorts them by the
//! ecosystem `version_sort_key` total order, parses a constraint (via the
//! ecosystem's [`prism_plugin_api::RequirementParser`], falling back to
//! [`prism_semver::VersionReq`]), and computes the ordinal candidate ranges with
//! [`prism_semver::candidate_ranges`]. Yanked versions stay in the list (so
//! ordinals are stable) but are reported as exclusions, never as candidates.
//!
//! [`Prism::candidates`] does the same starting from a stored
//! [`prism_model::Requirement`] id: it resolves the requirement's target package
//! and raw constraint, then defers to `candidates_for`.
//!
//! [`Prism::conflict_pressure`] computes a [`prism_model::CandidateSet`] for every
//! requirement targeting a package, intersects them pairwise (honouring
//! exclusions), and reports the conflicting pairs plus a tightness score.

use std::collections::BTreeSet;
use std::ops::Range;

use prism_model::{
    CandidateReason, EcosystemId, EnvironmentId, ExclusionReason, PackageId, PrismError,
    RequirementId, Result, SnapshotId, VersionFlags, VersionId,
};
use prism_plugin_api::EcosystemPlugin;
use prism_semver::{candidate_ranges, Version, VersionReq};

use crate::candidate_views::{
    CandidateExclusion, CandidateVersion, CandidateView, ConflictPair, ConflictReportView,
};
use crate::ecosystem::Ecosystem;
use crate::Prism;

/// A package's versions in ascending `version_sort_key` order, paired with the
/// parsed [`Version`] and the yanked bit (so exclusions can be reported).
struct SortedVersions {
    /// `(version_id, parsed-version, yanked)` ascending by sort key.
    entries: Vec<(VersionId, Version, bool)>,
}

impl SortedVersions {
    /// The ascending parsed-version list `candidate_ranges` consumes.
    fn versions(&self) -> Vec<Version> {
        self.entries.iter().map(|(_, v, _)| v.clone()).collect()
    }

    /// Total version count (the candidate-set ordinal space).
    fn len(&self) -> u32 {
        self.entries.len() as u32
    }
}

impl Prism {
    /// Compute the candidates for an explicit `package` + `constraint` under
    /// `ecosystem`.
    ///
    /// Loads the package's versions (`versions_by_package` → `get_version`),
    /// sorts them by the stored `version_sort_key`, parses `constraint`, and
    /// returns the satisfying versions (purl + ordinal), the raw ordinal ranges,
    /// and the exclusions (yanked + prerelease-not-requested). Never panics on a
    /// malformed constraint: it surfaces a [`PrismError`].
    pub fn candidates_for(
        &self,
        package: PackageId,
        constraint: &str,
        ecosystem: Ecosystem,
    ) -> Result<CandidateView> {
        let sorted = self.sorted_versions(package)?;
        let req = parse_constraint(ecosystem, constraint)?;
        let view = self.build_candidate_view(package, constraint, &req, &sorted);
        Ok(view)
    }

    /// Compute the candidates for a stored [`prism_model::Requirement`] id.
    ///
    /// Resolves the requirement's target package (`dst_package_id`) and raw
    /// constraint (`raw_requirement`), then defers to [`Prism::candidates_for`].
    /// A requirement without a resolved target is a [`PrismError::Other`] (it is
    /// not indexed by `REQS_BY_DST_PACKAGE`, so there is no candidate space).
    pub fn candidates(&self, requirement: RequirementId) -> Result<CandidateView> {
        let req = self.store.get_requirement(requirement)?;
        let package = req.dst_package_id.ok_or_else(|| {
            PrismError::Other(format!(
                "requirement {} has no resolved target package",
                requirement.get()
            ))
        })?;
        let constraint = self
            .store
            .resolve_string(req.raw_requirement)
            .unwrap_or_default();
        let eco = eco_of(self.store.get_package(package)?.ecosystem);
        self.candidates_for(package, &constraint, eco)
    }

    /// Compute the conflict-pressure report for a package family.
    ///
    /// For every requirement targeting `package`
    /// (`requirements_by_dst_package`), compute its [`prism_model::CandidateSet`],
    /// then pairwise same-package intersection (a pair conflicts when the
    /// intersection of `ranges` minus `exclusions` is empty). Reports per-set
    /// sizes, the conflicting pairs, and a tightness score
    /// `1 - |union of candidates| / total`.
    pub fn conflict_pressure(
        &self,
        package: PackageId,
        ecosystem: Ecosystem,
    ) -> Result<ConflictReportView> {
        let sorted = self.sorted_versions(package)?;
        let total = sorted.len();
        let req_ids = self.store.requirements_by_dst_package(package);

        // Build a CandidateSet (as an admissible-ordinal bitset) per requirement.
        let mut sets: Vec<(RequirementId, BTreeSet<u32>)> = Vec::with_capacity(req_ids.len());
        for rid in &req_ids {
            let raw = self
                .store
                .get_requirement(*rid)
                .ok()
                .and_then(|r| self.store.resolve_string(r.raw_requirement));
            let Some(raw) = raw else { continue };
            // A malformed stored constraint should not abort the whole report;
            // treat it as matching nothing (an empty admissible set).
            let admissible = match parse_constraint(ecosystem, &raw) {
                Ok(req) => admissible_ordinals(&req, &sorted),
                Err(_) => BTreeSet::new(),
            };
            sets.push((*rid, admissible));
        }

        let set_sizes: Vec<(RequirementId, u32)> =
            sets.iter().map(|(id, s)| (*id, s.len() as u32)).collect();

        // Pairwise same-package conflict: empty intersection => conflict.
        let mut conflicts = Vec::new();
        for i in 0..sets.len() {
            for j in (i + 1)..sets.len() {
                if sets[i].1.is_disjoint(&sets[j].1) {
                    conflicts.push(ConflictPair {
                        a: sets[i].0,
                        b: sets[j].0,
                    });
                }
            }
        }

        // Tightness over the union of all requirements' admissible ordinals.
        let union: BTreeSet<u32> = sets.iter().flat_map(|(_, s)| s.iter().copied()).collect();
        let tightness = if total == 0 {
            0.0
        } else {
            1.0 - (union.len() as f64) / (total as f64)
        };

        Ok(ConflictReportView {
            package,
            package_purl: self.package_purl(package),
            total_versions: total,
            set_sizes,
            conflicts,
            tightness,
        })
    }

    // --- internals -----------------------------------------------------------

    /// Load `package`'s versions sorted ascending by the stored
    /// `version_sort_key`. A version whose raw string does not parse as SemVer is
    /// skipped (it cannot participate in range math) but logged via its absence.
    fn sorted_versions(&self, package: PackageId) -> Result<SortedVersions> {
        let mut raw: Vec<(Vec<u8>, VersionId, Version, bool)> = Vec::new();
        for vid in self.store.versions_by_package(package) {
            let pv = self.store.get_version(vid)?;
            let Some(raw_str) = self.store.resolve_string(pv.version_raw) else {
                continue;
            };
            let Ok(parsed) = Version::parse(&raw_str) else {
                continue;
            };
            let yanked = pv.flags.contains(VersionFlags::YANKED);
            raw.push((pv.version_sort_key.clone(), vid, parsed, yanked));
        }
        // Sort by the ecosystem sort key (the snapshot-pinned total order). Tie-
        // break on version id so the order is deterministic even if two keys
        // collide (which the total-order contract forbids, but we never panic).
        raw.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.get().cmp(&b.1.get())));
        let entries = raw.into_iter().map(|(_, id, v, y)| (id, v, y)).collect();
        Ok(SortedVersions { entries })
    }

    /// Assemble a [`CandidateView`] from a parsed requirement and the sorted list.
    fn build_candidate_view(
        &self,
        package: PackageId,
        constraint: &str,
        req: &VersionReq,
        sorted: &SortedVersions,
    ) -> CandidateView {
        let cr = candidate_ranges(&sorted.versions(), req);

        // Yanked ordinals are excluded by default even when they match the range.
        let yanked: BTreeSet<u32> = sorted
            .entries
            .iter()
            .enumerate()
            .filter(|(_, (_, _, y))| *y)
            .map(|(i, _)| i as u32)
            .collect();

        let mut candidates = Vec::new();
        let mut ranges: Vec<Range<u32>> = Vec::new();
        for range in &cr.ranges {
            // Split each matching range around the yanked ordinals it contains.
            let mut start = range.start;
            for ord in range.clone() {
                if yanked.contains(&ord) {
                    if ord > start {
                        ranges.push(start..ord);
                    }
                    start = ord + 1;
                    continue;
                }
                let (vid, _, _) = &sorted.entries[ord as usize];
                candidates.push(CandidateVersion {
                    ordinal: ord,
                    version: *vid,
                    purl: self.version_label(*vid),
                });
            }
            if range.end > start {
                ranges.push(start..range.end);
            }
        }

        // Exclusions: prerelease-not-requested (from candidate_ranges) and yanked
        // ordinals that fell within a matching range.
        let mut exclusions = Vec::new();
        for &ord in &cr.prerelease_excluded {
            let (vid, _, _) = &sorted.entries[ord as usize];
            exclusions.push(CandidateExclusion {
                ordinal: ord,
                version: *vid,
                reason: ExclusionReason::Excluded,
            });
        }
        for range in &cr.ranges {
            for ord in range.clone() {
                if yanked.contains(&ord) {
                    let (vid, _, _) = &sorted.entries[ord as usize];
                    exclusions.push(CandidateExclusion {
                        ordinal: ord,
                        version: *vid,
                        reason: ExclusionReason::Yanked,
                    });
                }
            }
        }
        exclusions.sort_by_key(|e| e.ordinal);

        let reason = if candidates.is_empty() {
            CandidateReason::Unsatisfiable
        } else {
            CandidateReason::Constraint
        };

        CandidateView {
            package,
            package_purl: self.package_purl(package),
            constraint: constraint.to_string(),
            candidates,
            ranges,
            exclusions,
            reason,
            total_versions: sorted.len(),
        }
    }

    /// Build a stored [`prism_model::CandidateSet`] for `requirement` over
    /// `package` (ordinal ranges + exclusions). Exposed for callers wanting the
    /// canonical model type rather than the display view.
    pub fn candidate_set(
        &self,
        requirement: RequirementId,
        package: PackageId,
        ecosystem: Ecosystem,
    ) -> Result<prism_model::CandidateSet> {
        let sorted = self.sorted_versions(package)?;
        let raw = self
            .store
            .get_requirement(requirement)
            .ok()
            .and_then(|r| self.store.resolve_string(r.raw_requirement))
            .unwrap_or_default();
        let req = parse_constraint(ecosystem, &raw)?;
        let view = self.build_candidate_view(package, &raw, &req, &sorted);
        Ok(prism_model::CandidateSet {
            requirement_id: requirement,
            environment_id: EnvironmentId::new(0),
            snapshot_id: SnapshotId::new(0),
            package_id: package,
            ranges: view.ranges,
            exclusions: view
                .exclusions
                .iter()
                .map(|e| (e.ordinal, e.reason))
                .collect(),
            reason: view.reason,
        })
    }

    /// Resolve a `--package` purl to a `(PackageId, Ecosystem)`.
    ///
    /// Accepts either a base purl (`pkg:cargo/serde`) or a full version purl
    /// (`pkg:cargo/serde@1.0.0`); the `@version` suffix is stripped to its base.
    /// The base is matched against stored packages' `purl_base`. A purl naming no
    /// known package is a [`PrismError::NotFound`].
    pub fn package_by_purl(&self, purl: &str) -> Result<(PackageId, Ecosystem)> {
        // Strip a trailing `@version` only when the suffix looks like a version
        // (starts with a digit or `v`), so an npm `@scope/` prefix is preserved.
        let base = match purl.rsplit_once('@') {
            Some((b, ver))
                if !b.is_empty()
                    && ver.chars().next().is_some_and(|c| c.is_ascii_digit() || c == 'v') =>
            {
                b
            }
            _ => purl,
        };
        for rec in self.store.read_all_records()? {
            if rec.record_type != prism_store::record_type::PACKAGE {
                continue;
            }
            let Ok(pkg) = prism_store::records::decode_package(&rec.value) else {
                continue;
            };
            if self.store.resolve_string(pkg.purl_base).as_deref() == Some(base) {
                return Ok((pkg.id, eco_of(pkg.ecosystem)));
            }
        }
        Err(PrismError::NotFound)
    }

    /// The base purl (`pkg:<eco>/<name>`) for a package, for display.
    fn package_purl(&self, package: PackageId) -> String {
        if let Ok(p) = self.store.get_package(package) {
            if let Some(s) = self.store.resolve_string(p.purl_base) {
                return s;
            }
        }
        format!("package:{}", package.get())
    }
}

/// The admissible (non-yanked, satisfying) ordinals for `req` over `sorted`.
fn admissible_ordinals(req: &VersionReq, sorted: &SortedVersions) -> BTreeSet<u32> {
    let cr = candidate_ranges(&sorted.versions(), req);
    let mut out = BTreeSet::new();
    for range in &cr.ranges {
        for ord in range.clone() {
            // Yanked versions are not admissible candidates.
            if !sorted.entries[ord as usize].2 {
                out.insert(ord);
            }
        }
    }
    out
}

/// Parse a raw constraint for `ecosystem`. We first validate via the ecosystem's
/// [`prism_plugin_api::RequirementParser`] (so e.g. cargo/npm dialect quirks are
/// honoured), then build the matcher with the shared [`VersionReq`] engine the
/// plugins are themselves built on. An empty/`*` constraint means "any".
///
/// Shared with the synthetic resolver ([`crate::resolve_store`]) so the engine
/// interprets a bare `"1.0"` exactly as candidate analysis does.
pub(crate) fn parse_constraint(ecosystem: Ecosystem, constraint: &str) -> Result<VersionReq> {
    let trimmed = constraint.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return Ok(VersionReq::any());
    }
    // Route through the ecosystem parser for validation/canonicalization; map a
    // parse error to a clear PrismError rather than panicking.
    let canonical = match ecosystem {
        Ecosystem::Cargo => Ecosystem::cargo_plugin()
            .requirement_parser()
            .canonicalize(trimmed),
        Ecosystem::Npm => Ecosystem::npm_plugin()
            .requirement_parser()
            .canonicalize(trimmed),
    }
    .map_err(|e| PrismError::Other(format!("bad constraint {constraint:?}: {e}")))?;
    // The canonical form is itself a parseable requirement; fall back to the raw
    // text if the canonical string ever fails to round-trip.
    VersionReq::parse(&canonical)
        .or_else(|_| VersionReq::parse(trimmed))
        .map_err(|e| PrismError::Other(format!("bad constraint {constraint:?}: {e}")))
}

/// The facade ecosystem owning a stored id (only two are registered).
pub(crate) fn eco_of(id: EcosystemId) -> Ecosystem {
    if id == Ecosystem::Npm.id() {
        Ecosystem::Npm
    } else {
        Ecosystem::Cargo
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_constraint_any() {
        assert!(parse_constraint(Ecosystem::Cargo, "*").unwrap().matches(
            &Version::parse("9.9.9").unwrap()
        ));
        assert!(parse_constraint(Ecosystem::Cargo, "").unwrap().matches(
            &Version::parse("0.0.1").unwrap()
        ));
    }

    #[test]
    fn parse_constraint_caret() {
        let req = parse_constraint(Ecosystem::Cargo, "^1.2.3").unwrap();
        assert!(req.matches(&Version::parse("1.5.0").unwrap()));
        assert!(!req.matches(&Version::parse("2.0.0").unwrap()));
    }

    #[test]
    fn parse_constraint_bad_is_error_not_panic() {
        assert!(parse_constraint(Ecosystem::Cargo, ">=>=bogus").is_err());
    }
}
