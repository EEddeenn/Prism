//! Display-oriented views for candidate-version and conflict-pressure analysis
//! (architecture doc, *Candidate Version Index* and *Conflict Pressure*).
//!
//! Like [the graph views](crate::views), these are *resolved* projections: the
//! facade pairs each satisfying version's snapshot-pinned ordinal with the purl
//! the store interned, and a [`ConflictReportView`] surfaces the requirement
//! pairs whose candidate sets do not overlap. They are plain data; the facade
//! owns the lookups.

use std::ops::Range;

use prism_model::{
    CandidateReason, ExclusionReason, PackageId, RequirementId, VersionId,
};

/// One satisfying candidate version, resolved for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateVersion {
    /// Ordinal into the package's snapshot-pinned, ascending version list.
    pub ordinal: u32,
    /// The version id at that ordinal.
    pub version: VersionId,
    /// The version's purl/label for display.
    pub purl: String,
}

/// One excluded ordinal and why (yanked, prerelease-not-requested, …).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateExclusion {
    /// Ordinal into the package's sorted version list.
    pub ordinal: u32,
    /// The version id at that ordinal.
    pub version: VersionId,
    /// Why this version is not a candidate.
    pub reason: ExclusionReason,
}

/// The candidate analysis for one requirement/constraint against a package.
///
/// Mirrors [`prism_model::CandidateSet`] (ordinal ranges over the snapshot-pinned
/// sorted version list), enriched with resolved purls for display. The raw
/// `ranges`/`exclusions` are kept so callers can do their own intersection math.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateView {
    /// The package family these candidates belong to.
    pub package: PackageId,
    /// The package's base purl (`pkg:<eco>/<name>`), for display.
    pub package_purl: String,
    /// The raw constraint expression that was evaluated.
    pub constraint: String,
    /// The satisfying versions (ordinal + purl), ascending by ordinal.
    pub candidates: Vec<CandidateVersion>,
    /// Coalesced half-open ordinal ranges `[start, end)` that satisfy the
    /// constraint (the raw [`prism_model::CandidateSet::ranges`]).
    pub ranges: Vec<Range<u32>>,
    /// Versions excluded with a per-version reason (yanked, prerelease gate).
    pub exclusions: Vec<CandidateExclusion>,
    /// Why the candidate set has the shape it does.
    pub reason: CandidateReason,
    /// Total number of versions of the package in this snapshot.
    pub total_versions: u32,
}

impl CandidateView {
    /// Number of satisfying candidates.
    pub fn count(&self) -> usize {
        self.candidates.len()
    }

    /// True if no version satisfies the constraint.
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

/// A pair of same-family requirements whose candidate sets do not overlap.
///
/// Only meaningful within one package family (cross-package ordinals are not
/// comparable — see the architecture doc's *Conflict Pressure*).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConflictPair {
    /// First requirement in the conflicting pair.
    pub a: RequirementId,
    /// Second requirement in the conflicting pair.
    pub b: RequirementId,
}

/// The conflict-pressure report for one package family.
#[derive(Clone, Debug, PartialEq)]
pub struct ConflictReportView {
    /// The package family analysed.
    pub package: PackageId,
    /// The package's base purl for display.
    pub package_purl: String,
    /// Total number of versions of the package in this snapshot.
    pub total_versions: u32,
    /// One `(requirement_id, satisfying-count)` per requirement targeting the
    /// package, ascending by requirement id.
    pub set_sizes: Vec<(RequirementId, u32)>,
    /// Pairs of requirements that mutually conflict (empty intersection).
    pub conflicts: Vec<ConflictPair>,
    /// Constraint tightness in `[0, 1]`: `1 - satisfying / total` over the union
    /// of all requirements' candidates (0 = every version is admissible by some
    /// requirement; 1 = none is).
    pub tightness: f64,
}

impl ConflictReportView {
    /// Number of requirements targeting the package.
    pub fn requirement_count(&self) -> usize {
        self.set_sizes.len()
    }

    /// True if any same-family requirement pair conflicts.
    pub fn has_conflict(&self) -> bool {
        !self.conflicts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // an honest one-range candidate set
    fn candidate_view_predicates() {
        let v = CandidateView {
            package: PackageId::new(1),
            package_purl: "pkg:cargo/x".into(),
            constraint: "^1".into(),
            candidates: vec![CandidateVersion {
                ordinal: 0,
                version: VersionId::new(10),
                purl: "pkg:cargo/x@1.0.0".into(),
            }],
            ranges: vec![0..1],
            exclusions: Vec::new(),
            reason: CandidateReason::Constraint,
            total_versions: 1,
        };
        assert_eq!(v.count(), 1);
        assert!(!v.is_empty());
    }

    #[test]
    fn conflict_report_predicates() {
        let mut r = ConflictReportView {
            package: PackageId::new(1),
            package_purl: "pkg:cargo/x".into(),
            total_versions: 3,
            set_sizes: vec![(RequirementId::new(1), 1)],
            conflicts: Vec::new(),
            tightness: 0.0,
        };
        assert!(!r.has_conflict());
        assert_eq!(r.requirement_count(), 1);
        r.conflicts.push(ConflictPair {
            a: RequirementId::new(1),
            b: RequirementId::new(2),
        });
        assert!(r.has_conflict());
    }
}
