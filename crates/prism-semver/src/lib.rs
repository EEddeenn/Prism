//! SemVer 2.0 version and constraint engine for Prism.
//!
//! This crate provides ecosystem-agnostic SemVer machinery shared by the npm
//! and Cargo plugins:
//!
//! - [`Version`] — a parsed SemVer 2.0 version with a spec-correct total
//!   [`Ord`] and a [`Version::sort_key`] byte string whose lexical order matches
//!   that ordering exactly (this becomes a `PackageVersion::version_sort_key`).
//! - [`VersionReq`] — a constraint parser/matcher covering the npm and Cargo
//!   range dialects (exact, comparators, caret, tilde, wildcards, hyphen ranges,
//!   and comma/space-separated AND sets).
//! - [`candidate_ranges`] — O(V) computation of ordinal candidate ranges over a
//!   snapshot-pinned, ascending version list, feeding `CandidateSet` ranges and
//!   prerelease exclusions.
//!
//! All parsing returns [`prism_model::Result`] and never panics on untrusted
//! input. See the architecture doc's *PackageVersion*, *Candidate Version
//! Index*, and *CandidateSet* sections.

mod candidate;
mod req;
mod version;

pub use candidate::{candidate_ranges, CandidateRanges};
pub use req::{Comparator, Op, VersionReq};
pub use version::{Identifier, Version};

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn end_to_end_caret_over_sorted_list() {
        let mut versions: Vec<Version> = ["1.1.0", "1.2.3", "1.9.0", "2.0.0", "1.2.4-rc.1"]
            .iter()
            .map(|s| Version::parse(s).unwrap())
            .collect();
        versions.sort_by_key(|a| a.sort_key());
        // sorted => 1.1.0, 1.2.3, 1.2.4-rc.1, 1.9.0, 2.0.0
        let req = VersionReq::parse("^1.2.3").unwrap();
        let out = candidate_ranges(&versions, &req);
        // 1.2.3(1), 1.9.0(3) match; 1.2.4-rc.1(2) gate-excluded; 1.1.0/2.0.0 no.
        assert_eq!(out.ranges, vec![1..2, 3..4]);
        assert_eq!(out.prerelease_excluded, vec![2]);
    }

    #[test]
    fn sort_key_feeds_version_sort_key_field() {
        // The sort_key is exactly what would populate a model PackageVersion.
        let v = Version::parse("1.2.3-rc.1+build").unwrap();
        let key = v.sort_key();
        assert!(!key.is_empty());
        // Build metadata must not affect the key.
        let v2 = Version::parse("1.2.3-rc.1+other").unwrap();
        assert_eq!(key, v2.sort_key());
    }
}
