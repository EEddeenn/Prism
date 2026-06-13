//! A reusable [`CandidateSource`] that applies the standard selection policies
//! over an in-memory catalog of per-package versions.
//!
//! The engine itself is policy-agnostic — it only calls
//! [`CandidateSource::select`] — but real adapters need somewhere to put the
//! locked / lowest / highest-non-prerelease logic. [`VersionCatalog`] is that
//! place: a sorted (ascending, by [`prism_semver::Version`]) list of
//! `(VersionId, raw-version)` pairs per [`PackageId`], plus optional lock pins,
//! with selection resolved against a [`prism_semver::VersionReq`] matcher.

use std::collections::BTreeMap;

use prism_model::{EnvironmentId, PackageId, VersionId};
use prism_plugin_api::{ActivationContext, ResolveError, ResolvePolicy};
use prism_semver::{Version, VersionReq};

use crate::source::CandidateSource;

/// One published version known to the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Candidate {
    id: VersionId,
    version: Version,
}

/// An in-memory catalog of candidate versions per package, plus lock pins.
///
/// Versions are kept sorted ascending so policy selection is a simple scan.
#[derive(Clone, Debug, Default)]
pub struct VersionCatalog {
    by_package: BTreeMap<PackageId, Vec<Candidate>>,
    locked: BTreeMap<PackageId, VersionId>,
}

impl VersionCatalog {
    /// An empty catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a published `(version_id, raw-version)` for `package`. Raw
    /// versions are parsed with [`prism_semver::Version::parse`]; an unparseable
    /// version is rejected so the catalog never holds garbage.
    pub fn add_version(
        &mut self,
        package: PackageId,
        id: VersionId,
        raw_version: &str,
    ) -> Result<(), ResolveError> {
        let version = Version::parse(raw_version)
            .map_err(|e| ResolveError::Other(format!("bad version {raw_version}: {e}")))?;
        let list = self.by_package.entry(package).or_default();
        list.push(Candidate { id, version });
        list.sort_by(|a, b| a.version.cmp(&b.version));
        Ok(())
    }

    /// Pin `package` to a specific `version` for [`ResolvePolicy::Locked`].
    pub fn lock(&mut self, package: PackageId, version: VersionId) {
        self.locked.insert(package, version);
    }

    /// Versions of `package` satisfying `matcher`, ascending. An empty/`*`
    /// matcher matches all known versions.
    fn satisfying<'a>(
        &'a self,
        package: PackageId,
        matcher: &str,
    ) -> Result<Vec<&'a Candidate>, ResolveError> {
        let req = parse_req(matcher)?;
        let Some(list) = self.by_package.get(&package) else {
            return Ok(Vec::new());
        };
        Ok(list.iter().filter(|c| req.matches(&c.version)).collect())
    }
}

/// Parse a matcher into a [`VersionReq`]. Empty and `*` mean "any".
fn parse_req(matcher: &str) -> Result<VersionReq, ResolveError> {
    let m = matcher.trim();
    if m.is_empty() || m == "*" {
        return Ok(VersionReq::any());
    }
    VersionReq::parse(m).map_err(|e| ResolveError::Other(format!("bad matcher {matcher}: {e}")))
}

impl CandidateSource for VersionCatalog {
    fn select(
        &self,
        package: PackageId,
        matcher: &str,
        _environment: EnvironmentId,
        _activation: &ActivationContext,
        policy: &ResolvePolicy,
    ) -> Result<Option<VersionId>, ResolveError> {
        // Locked policy ignores the matcher: it replays the recorded pin.
        if matches!(policy, ResolvePolicy::Locked) {
            return Ok(self.locked.get(&package).copied());
        }

        let candidates = self.satisfying(package, matcher)?;
        if candidates.is_empty() {
            return Ok(None);
        }

        let chosen = match policy {
            ResolvePolicy::Lowest => candidates.first(),
            ResolvePolicy::Latest => candidates.last(),
            ResolvePolicy::HighestNonPrerelease => candidates
                .iter()
                .rev()
                .find(|c| !c.version.is_prerelease())
                // Fall back to the absolute highest if every candidate is a
                // prerelease, so the requirement still resolves.
                .or_else(|| candidates.last()),
            // Policies without bespoke catalog logic default to highest (the
            // common "latest satisfying" behaviour); a richer source overrides.
            _ => candidates.last(),
        };
        Ok(chosen.map(|c| c.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> VersionCatalog {
        // Releases 1.0.0/1.2.0/2.0.0/2.1.0 plus a prerelease 2.0.0-rc.1.
        let mut c = VersionCatalog::new();
        let p = PackageId::new(1);
        c.add_version(p, VersionId::new(10), "1.0.0").unwrap();
        c.add_version(p, VersionId::new(11), "1.2.0").unwrap();
        c.add_version(p, VersionId::new(13), "2.0.0-rc.1").unwrap();
        c.add_version(p, VersionId::new(12), "2.0.0").unwrap();
        c.add_version(p, VersionId::new(14), "2.1.0").unwrap();
        c
    }

    fn sel(c: &VersionCatalog, m: &str, p: ResolvePolicy) -> Option<VersionId> {
        c.select(
            PackageId::new(1),
            m,
            EnvironmentId::new(0),
            &ActivationContext::new(),
            &p,
        )
        .unwrap()
    }

    #[test]
    fn lowest_picks_smallest_satisfying() {
        assert_eq!(sel(&catalog(), ">=1", ResolvePolicy::Lowest), Some(VersionId::new(10)));
    }

    #[test]
    fn latest_can_pick_a_prerelease_when_it_is_the_top() {
        // A matcher whose core names the prerelease admits it, and nothing
        // higher matches, so the prerelease is the latest candidate.
        let mut c = VersionCatalog::new();
        let p = PackageId::new(1);
        c.add_version(p, VersionId::new(20), "3.0.0-rc.1").unwrap();
        let got = c
            .select(
                p,
                ">=3.0.0-rc.0",
                EnvironmentId::new(0),
                &ActivationContext::new(),
                &ResolvePolicy::Latest,
            )
            .unwrap();
        assert_eq!(got, Some(VersionId::new(20)));
    }

    #[test]
    fn highest_non_prerelease_skips_rc() {
        // `>=2.0.0-rc.0` admits the rc, 2.0.0, and 2.1.0; the policy skips the rc.
        assert_eq!(
            sel(&catalog(), ">=2.0.0-rc.0", ResolvePolicy::HighestNonPrerelease),
            Some(VersionId::new(14))
        );
    }

    #[test]
    fn locked_replays_pin_ignoring_matcher() {
        let mut c = catalog();
        c.lock(PackageId::new(1), VersionId::new(11));
        assert_eq!(sel(&c, "=99", ResolvePolicy::Locked), Some(VersionId::new(11)));
        // No pin for an unknown package → no selection.
        assert_eq!(
            c.select(
                PackageId::new(2),
                "*",
                EnvironmentId::new(0),
                &ActivationContext::new(),
                &ResolvePolicy::Locked
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn unsatisfiable_matcher_yields_none() {
        assert_eq!(sel(&catalog(), ">=9", ResolvePolicy::Lowest), None);
    }
}
