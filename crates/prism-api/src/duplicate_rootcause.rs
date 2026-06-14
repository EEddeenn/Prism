//! Duplicate-version **root-cause** analysis (architecture doc, *Duplicate
//! Root-Cause*).
//!
//! [`crate::query::Prism::duplicate_clusters`] reports *which* families are
//! fragmented; this answers *why* and *how risky*. For each family present at
//! more than one version it attributes every version to the upstream
//! requirements that pin it, measures the family's external blast radius, and
//! offers a convergence verdict so a maintainer can tell a trivially-dedupable
//! family from one wedged apart by genuinely incompatible constraints.

use std::collections::{BTreeMap, BTreeSet};

use prism_graph::{family_clusters, GraphSnapshot};
use prism_model::{GraphNodeId, PackageId, RequirementId, Result, RunId, VersionId};

use crate::Prism;

/// Sentinel `RequirementId` meaning "this edge carries no requirement" (the CSR
/// packs `Option<RequirementId>::None` as `u64::MAX`).
const NO_REQUIREMENT: u64 = u64::MAX;

/// One upstream requirement that pins a particular version of a family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstraintIntroducer {
    /// The purl of the occurrence that imposes the requirement.
    pub introducer_purl: String,
    /// The raw constraint text (e.g. `^1.2`), or empty when the lockfile records
    /// no constraint on the edge (a fidelity limitation, not an absence of one).
    pub constraint: String,
}

/// One distinct version of a fragmented family and who pins it there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateVersionCause {
    /// The version id.
    pub version: VersionId,
    /// The full version purl.
    pub purl: String,
    /// Occurrences of this version in the graph.
    pub occurrences: u32,
    /// Distinct (introducer, constraint) pairs that introduce this version.
    pub introducers: Vec<ConstraintIntroducer>,
}

/// The root-cause report for one duplicate-version family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateRootCauseView {
    /// The package family.
    pub package: PackageId,
    /// The family's base purl.
    pub package_purl: String,
    /// Distinct coexisting versions (≥ 2), ascending, each with its causes.
    pub versions: Vec<DuplicateVersionCause>,
    /// Total occurrences across all versions.
    pub occurrence_count: u32,
    /// Distinct occurrences outside the family that depend on it (blast radius).
    pub blast_radius: u32,
    /// Number of distinct constraint texts seen across all versions — the count
    /// of competing requirement opinions that fragment the family.
    pub distinct_constraints: usize,
    /// Convergence verdict (see [`convergence_label`]).
    pub convergence: &'static str,
    /// A severity score: version drift × (1 + blast radius). Higher = worse.
    pub risk_score: u32,
}

/// Classify how hard a family is to converge from its version strings:
/// - `compatible`: every version shares the same breaking key (same major, or
///   same `0.minor` for 0.x) — almost always dedupable to the highest.
/// - `major-drift`: versions span breaking boundaries — converging is a real
///   (possibly breaking) upgrade.
/// - `unknown`: version strings could not be parsed for comparison.
fn convergence_label(version_strings: &[String]) -> &'static str {
    let mut keys: BTreeSet<Option<(u64, u64)>> = BTreeSet::new();
    for v in version_strings {
        keys.insert(breaking_key(v));
    }
    if keys.iter().any(|k| k.is_none()) {
        return "unknown";
    }
    if keys.len() <= 1 {
        "compatible"
    } else {
        "major-drift"
    }
}

/// The semver-ish "breaking key" of a version string: `(major, 0)` for `>=1.0`,
/// or `(0, minor)` for a `0.x` version (where minor bumps are breaking).
/// Returns `None` if the leading numeric component cannot be read.
fn breaking_key(version: &str) -> Option<(u64, u64)> {
    let mut parts = version.split(['.', '+', '-']);
    let major: u64 = parts.next()?.parse().ok()?;
    if major == 0 {
        let minor: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        Some((0, minor))
    } else {
        Some((major, 0))
    }
}

/// Extract the version substring of a purl (`pkg:npm/foo@1.2.3` → `1.2.3`).
fn version_of_purl(purl: &str) -> &str {
    purl.rsplit_once('@').map(|(_, v)| v).unwrap_or("")
}

impl Prism {
    /// Root-cause analysis of every duplicate-version family in `run`, ordered by
    /// descending risk score.
    pub fn duplicate_root_causes(&self, run: RunId) -> Result<Vec<DuplicateRootCauseView>> {
        let g = self.load_graph(run)?;

        // Fragmented families and their package ids.
        let fragmented: Vec<_> = family_clusters(&g, |v| self.package_of(v))?
            .into_iter()
            .filter(|c| c.is_version_fragmented())
            .collect();
        if fragmented.is_empty() {
            return Ok(Vec::new());
        }
        let frag_pkgs: BTreeSet<u64> = fragmented.iter().map(|c| c.package_id.get()).collect();

        // One pass over nodes: group occurrence nodes by (package, version).
        let mut occ: BTreeMap<(u64, u64), Vec<GraphNodeId>> = BTreeMap::new();
        // And mark every node belonging to a fragmented family, for blast radius.
        let mut family_members: BTreeMap<u64, Vec<GraphNodeId>> = BTreeMap::new();
        for i in 0..g.node_count() as u32 {
            let node = GraphNodeId::new(i);
            let vid = g.node_version(node)?;
            if let Some(pkg) = self.package_of(vid) {
                if frag_pkgs.contains(&pkg.get()) {
                    occ.entry((pkg.get(), vid.get())).or_default().push(node);
                    family_members.entry(pkg.get()).or_default().push(node);
                }
            }
        }

        let mut out = Vec::with_capacity(fragmented.len());
        for cluster in &fragmented {
            let pkg = cluster.package_id;
            let members = family_members.get(&pkg.get()).cloned().unwrap_or_default();
            let blast_radius = self.external_dependents(&g, &members)?;

            let mut versions = Vec::with_capacity(cluster.versions.len());
            let mut constraints_seen: BTreeSet<String> = BTreeSet::new();
            let mut version_strings: Vec<String> = Vec::new();
            for &vid in &cluster.versions {
                let nodes = occ.get(&(pkg.get(), vid.get())).cloned().unwrap_or_default();
                let purl = self.version_label(vid);
                version_strings.push(version_of_purl(&purl).to_string());
                let introducers = self.constraint_introducers(&g, &nodes, &mut constraints_seen)?;
                versions.push(DuplicateVersionCause {
                    version: vid,
                    purl,
                    occurrences: nodes.len() as u32,
                    introducers,
                });
            }

            let convergence = convergence_label(&version_strings);
            let drift = cluster.version_count() as u32;
            let risk_score = drift.saturating_mul(1u32.saturating_add(blast_radius));

            out.push(DuplicateRootCauseView {
                package: pkg,
                package_purl: self.package_purl(pkg),
                versions,
                occurrence_count: cluster.occurrence_count,
                blast_radius,
                distinct_constraints: constraints_seen.len(),
                convergence,
                risk_score,
            });
        }

        out.sort_by(|a, b| {
            b.risk_score
                .cmp(&a.risk_score)
                .then(a.package_purl.cmp(&b.package_purl))
        });
        Ok(out)
    }

    /// The distinct (introducer purl, constraint) pairs across `nodes`'
    /// incoming edges, recording every constraint text into `seen`.
    fn constraint_introducers(
        &self,
        g: &GraphSnapshot,
        nodes: &[GraphNodeId],
        seen: &mut BTreeSet<String>,
    ) -> Result<Vec<ConstraintIntroducer>> {
        let mut pairs: BTreeSet<(String, String)> = BTreeSet::new();
        for &node in nodes {
            let preds = g.in_neighbors(node)?;
            let eids = g.in_edge_ids(node)?;
            for (k, &p) in preds.iter().enumerate() {
                let rid = g.edge_requirement_id(eids[k])?;
                let constraint = if rid.get() == NO_REQUIREMENT {
                    String::new()
                } else {
                    self.constraint_text(rid)
                };
                if !constraint.is_empty() {
                    seen.insert(constraint.clone());
                }
                let introducer_purl = self.version_label(g.node_version(p)?);
                pairs.insert((introducer_purl, constraint));
            }
        }
        Ok(pairs
            .into_iter()
            .map(|(introducer_purl, constraint)| ConstraintIntroducer {
                introducer_purl,
                constraint,
            })
            .collect())
    }

    /// The raw constraint text of a stored requirement, or empty if unresolvable.
    fn constraint_text(&self, rid: RequirementId) -> String {
        self.store
            .get_requirement(rid)
            .ok()
            .and_then(|r| self.store.resolve_string(r.raw_requirement))
            .unwrap_or_default()
    }

    /// Count distinct occurrences outside `members` that can reach any member
    /// (multi-source reverse BFS).
    fn external_dependents(&self, g: &GraphSnapshot, members: &[GraphNodeId]) -> Result<u32> {
        let n = g.node_count();
        let mut in_family = vec![false; n];
        for &m in members {
            in_family[m.get() as usize] = true;
        }
        let mut seen = vec![false; n];
        let mut queue: std::collections::VecDeque<GraphNodeId> = std::collections::VecDeque::new();
        for &m in members {
            if !seen[m.get() as usize] {
                seen[m.get() as usize] = true;
                queue.push_back(m);
            }
        }
        let mut count: u32 = 0;
        while let Some(u) = queue.pop_front() {
            for &p in g.in_neighbors(u)? {
                let pi = p.get() as usize;
                if !seen[pi] {
                    seen[pi] = true;
                    if !in_family[pi] {
                        count = count.saturating_add(1);
                    }
                    queue.push_back(p);
                }
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaking_key_distinguishes_major_and_zerox() {
        assert_eq!(breaking_key("1.2.3"), Some((1, 0)));
        assert_eq!(breaking_key("2.0.0"), Some((2, 0)));
        assert_eq!(breaking_key("0.5.1"), Some((0, 5)));
        assert_eq!(breaking_key("0.6.0"), Some((0, 6)));
        assert_eq!(breaking_key("abc"), None);
    }

    #[test]
    fn convergence_classifies_drift() {
        assert_eq!(
            convergence_label(&["1.2.3".into(), "1.9.0".into()]),
            "compatible"
        );
        assert_eq!(
            convergence_label(&["1.0.0".into(), "2.0.0".into()]),
            "major-drift"
        );
        assert_eq!(
            convergence_label(&["0.5.0".into(), "0.6.0".into()]),
            "major-drift"
        );
        assert_eq!(convergence_label(&["x".into(), "1.0".into()]), "unknown");
    }

    #[test]
    fn version_of_purl_extracts_tail() {
        assert_eq!(version_of_purl("pkg:npm/foo@1.2.3"), "1.2.3");
        assert_eq!(version_of_purl("pkg:cargo/a%2Fb@0.1.0"), "0.1.0");
        assert_eq!(version_of_purl("no-version"), "");
    }
}
