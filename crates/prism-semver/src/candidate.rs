//! Candidate-range computation over a snapshot-pinned, ascending version list.
//!
//! Given a package's versions sorted ascending by `sort_key` (the
//! `version_sort_key` total order) and a [`VersionReq`], this produces the
//! ordinal ranges that satisfy the requirement, plus the set of ordinals that
//! *would* have matched on numeric-core grounds but were excluded purely
//! because they are prereleases the requirement did not opt into.
//!
//! These ordinals index the `SnapshotId`-pinned list and feed directly into
//! `CandidateSet::ranges` / `exclusions` (see the architecture doc's
//! *Candidate Version Index* and *CandidateSet* sections). The computation is
//! a single linear pass: O(V).

use std::ops::Range;

use crate::req::VersionReq;
use crate::version::Version;

/// The result of evaluating a [`VersionReq`] against a sorted version list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CandidateRanges {
    /// Coalesced half-open ordinal ranges `[start, end)` that satisfy the
    /// requirement. Ranges are disjoint and in ascending order.
    pub ranges: Vec<Range<u32>>,
    /// Ordinals excluded *solely* because the version is a prerelease that the
    /// requirement did not request (SemVer prerelease gate). The UI can surface
    /// these as `ExclusionReason`-style "prerelease not requested" entries.
    pub prerelease_excluded: Vec<u32>,
}

impl CandidateRanges {
    /// Total number of satisfying ordinals across all ranges.
    pub fn count(&self) -> u32 {
        self.ranges.iter().map(|r| r.end - r.start).sum()
    }

    /// Whether `ordinal` falls in some satisfying range.
    pub fn contains(&self, ordinal: u32) -> bool {
        self.ranges.iter().any(|r| r.contains(&ordinal))
    }
}

/// Computes candidate ordinal ranges for `req` over `sorted`, which MUST be the
/// package's version list in ascending `sort_key` order (as guaranteed within a
/// single `SnapshotId`).
///
/// Adjacent matching ordinals are coalesced into ranges. A version that would
/// match numerically but is a non-requested prerelease is reported in
/// `prerelease_excluded` instead of `ranges`.
///
/// Runs in a single O(V) pass and never panics.
pub fn candidate_ranges(sorted: &[Version], req: &VersionReq) -> CandidateRanges {
    let mut ranges: Vec<Range<u32>> = Vec::new();
    let mut prerelease_excluded = Vec::new();
    let mut run_start: Option<u32> = None;

    for (i, v) in sorted.iter().enumerate() {
        let idx = i as u32;
        let matched = req.matches(v);
        if matched {
            // Extend or open the current run.
            if run_start.is_none() {
                run_start = Some(idx);
            }
        } else {
            // Close any open run.
            if let Some(start) = run_start.take() {
                ranges.push(start..idx);
            }
            // Distinguish "excluded only because it's a prerelease" from a true
            // non-match: a version is prerelease-excluded iff dropping its
            // prerelease tag would have satisfied the requirement.
            if v.is_prerelease() && req.matches(&strip_pre(v)) {
                prerelease_excluded.push(idx);
            }
        }
    }
    if let Some(start) = run_start.take() {
        ranges.push(start..sorted.len() as u32);
    }

    CandidateRanges {
        ranges,
        prerelease_excluded,
    }
}

/// Returns a copy of `v` with its prerelease tag removed, used to test whether a
/// prerelease was excluded only by the prerelease gate.
fn strip_pre(v: &Version) -> Version {
    Version {
        major: v.major,
        minor: v.minor,
        patch: v.patch,
        pre: Vec::new(),
        build: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::Version;

    fn versions(list: &[&str]) -> Vec<Version> {
        let mut v: Vec<Version> = list.iter().map(|s| Version::parse(s).unwrap()).collect();
        v.sort_by_key(|a| a.sort_key());
        v
    }

    #[test]
    fn design_doc_example() {
        // Mirrors the doc: B >=2.0,<3.0 over a six-version list => [2,5).
        let list = versions(&["1.0.0", "1.5.0", "2.0.0", "2.1.0", "2.5.0", "3.0.0"]);
        let req = VersionReq::parse(">=2.0.0, <3.0.0").unwrap();
        let out = candidate_ranges(&list, &req);
        assert_eq!(out.ranges, vec![2..5]);
        assert_eq!(out.count(), 3);
        assert!(out.prerelease_excluded.is_empty());
    }

    #[test]
    fn range_with_gap_coalesces_into_two() {
        // ^1.0.0 matches the 1.x band; a 2.x in the middle breaks the run, then
        // there is no second 1.x band so we expect a single contiguous range.
        let list = versions(&["0.9.0", "1.0.0", "1.2.0", "1.9.0", "2.0.0", "2.1.0"]);
        let req = VersionReq::parse("^1.0.0").unwrap();
        let out = candidate_ranges(&list, &req);
        assert_eq!(out.ranges, vec![1..4]);
    }

    #[test]
    fn contiguous_match_band() {
        let list = versions(&["1.0.0", "1.5.0", "2.0.0", "1.2.0"]);
        // sorted => 1.0.0(0), 1.2.0(1), 1.5.0(2), 2.0.0(3)
        let req = VersionReq::parse(">=1.0.0, <1.3.0").unwrap();
        let out = candidate_ranges(&list, &req);
        // 1.0.0(0), 1.2.0(1) match; 1.5.0(2), 2.0.0(3) do not.
        assert_eq!(out.ranges, vec![0..2]);
    }

    #[test]
    fn two_disjoint_ranges_via_prerelease_gap() {
        // An unrequested prerelease sitting between two matching releases breaks
        // the run, producing two disjoint candidate ranges.
        let list = versions(&["1.0.0", "1.5.0", "1.2.0-rc.1", "1.2.5"]);
        // sorted => 1.0.0(0), 1.2.0-rc.1(1), 1.2.5(2), 1.5.0(3)
        let req = VersionReq::parse("^1.0.0").unwrap();
        let out = candidate_ranges(&list, &req);
        // 1.0.0(0) matches; 1.2.0-rc.1(1) gate-excluded; 1.2.5(2), 1.5.0(3) match.
        assert_eq!(out.ranges, vec![0..1, 2..4]);
        assert_eq!(out.prerelease_excluded, vec![1]);
    }

    #[test]
    fn prerelease_at_lower_bound_excluded() {
        let list = versions(&["1.0.0", "2.0.0", "1.0.0-rc.1", "1.9.0"]);
        // sorted => 1.0.0-rc.1(0), 1.0.0(1), 1.9.0(2), 2.0.0(3)
        let req = VersionReq::parse("^1.0.0").unwrap();
        let out = candidate_ranges(&list, &req);
        // 1.0.0-rc.1 is prerelease-excluded; 1.0.0 and 1.9.0 match; 2.0.0 not.
        assert_eq!(out.ranges, vec![1..3]);
        assert_eq!(out.prerelease_excluded, vec![0]);
    }

    #[test]
    fn prerelease_excluded_reported() {
        let list = versions(&["1.2.3-rc.1", "1.2.3", "1.3.0", "1.3.0-rc.1"]);
        // sorted => 1.2.3-rc.1(0), 1.2.3(1), 1.3.0-rc.1(2), 1.3.0(3)
        let req = VersionReq::parse("^1.2.3").unwrap();
        let out = candidate_ranges(&list, &req);
        // Releases 1.2.3 and 1.3.0 match; both prereleases are gate-excluded.
        assert_eq!(out.ranges, vec![1..2, 3..4]);
        assert_eq!(out.prerelease_excluded, vec![0, 2]);
    }

    #[test]
    fn empty_inputs() {
        let out = candidate_ranges(&[], &VersionReq::parse("*").unwrap());
        assert!(out.ranges.is_empty());
        assert_eq!(out.count(), 0);
    }

    #[test]
    fn star_matches_all_releases() {
        let list = versions(&["1.0.0", "2.0.0", "3.0.0"]);
        let out = candidate_ranges(&list, &VersionReq::parse("*").unwrap());
        assert_eq!(out.ranges, vec![0..3]);
    }

    #[test]
    fn contains_helper() {
        let list = versions(&["1.0.0", "1.5.0", "2.0.0", "2.5.0"]);
        let out = candidate_ranges(&list, &VersionReq::parse(">=1.5.0, <2.5.0").unwrap());
        assert!(out.contains(1));
        assert!(out.contains(2));
        assert!(!out.contains(0));
        assert!(!out.contains(3));
    }
}
