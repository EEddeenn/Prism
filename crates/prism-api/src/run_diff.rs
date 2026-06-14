//! Cross-run diff that explains *why* two resolutions differ (architecture doc,
//! *Cross-Run Diff*).
//!
//! The web tier already diffs two runs' node lists for an added/removed/changed
//! count; this is the authoritative, interpretive comparison: new vs resolved
//! duplicate families, new vs resolved cycles, per-scope footprint shifts, and
//! the hotspots (structural chokepoints) a change introduces or grows.
//!
//! Because [`prism_model::PackageId`]/[`prism_model::VersionId`] are interned
//! **store-globally**, the same package/version shares an id across runs, so the
//! comparison is exact id-set algebra rather than fuzzy purl matching.

use std::collections::{BTreeMap, BTreeSet};

use prism_graph::{classify_cycles, dominators, family_clusters, scope_summary};
use prism_model::{GraphNodeId, Result, RunId};

use crate::structure_views::kind_label;
use crate::Prism;

/// A cycle as it appears in the diff, keyed by its member package families.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CycleDiffEntry {
    /// Member package purls (version-agnostic), ascending.
    pub members: Vec<String>,
    /// The structural triage class label.
    pub class: &'static str,
    /// Whether the cycle is runtime-relevant.
    pub runtime_relevant: bool,
}

/// A per-scope footprint shift between the two runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeShift {
    /// The dependency-kind label.
    pub kind: &'static str,
    /// Change in footprint occurrence count (head − base).
    pub footprint_delta: i64,
    /// Change in scope-exclusive occurrence count (head − base).
    pub exclusive_delta: i64,
}

/// A chokepoint that a change introduced or materially grew.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotspotShift {
    /// The gateway package purl.
    pub package_purl: String,
    /// Occurrences it dominated in the base run (0 if it was not a chokepoint).
    pub base_dominated: u32,
    /// Occurrences it dominates in the head run.
    pub head_dominated: u32,
}

/// The interpretive diff between a `base` and a `head` run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunDiffView {
    /// The base (older) run.
    pub base: RunId,
    /// The head (newer) run.
    pub head: RunId,
    /// Change in occurrence-node count (head − base).
    pub nodes_delta: i64,
    /// Change in edge count.
    pub edges_delta: i64,
    /// Change in strongly-connected-component count.
    pub scc_delta: i64,
    /// Change in the number of duplicate-version families.
    pub duplicate_families_delta: i64,
    /// Package families present in head but not base.
    pub added_packages: Vec<String>,
    /// Package families present in base but not head.
    pub removed_packages: Vec<String>,
    /// Exact versions present in head but not base.
    pub added_versions: Vec<String>,
    /// Exact versions present in base but not head.
    pub removed_versions: Vec<String>,
    /// Families that became version-fragmented in head.
    pub new_duplicate_families: Vec<String>,
    /// Families that stopped being version-fragmented in head.
    pub resolved_duplicate_families: Vec<String>,
    /// Cycles present in head but not base.
    pub new_cycles: Vec<CycleDiffEntry>,
    /// Cycles present in base but not head.
    pub resolved_cycles: Vec<CycleDiffEntry>,
    /// Per-scope footprint shifts (non-zero only).
    pub scope_shifts: Vec<ScopeShift>,
    /// New or materially grown chokepoints, by descending head dominated count.
    pub new_hotspots: Vec<HotspotShift>,
}

/// Everything about one run needed to diff it against another.
struct RunProfile {
    versions: BTreeSet<u64>,
    packages: BTreeSet<u64>,
    /// Distinct version purls keyed by version id (for display).
    version_purls: BTreeMap<u64, String>,
    package_purls: BTreeMap<u64, String>,
    fragmented: BTreeSet<u64>,
    /// cycle key (sorted member package ids) → display entry.
    cycles: BTreeMap<Vec<u64>, CycleDiffEntry>,
    /// scope label → (footprint, exclusive).
    scopes: BTreeMap<&'static str, (i64, i64)>,
    /// gateway package id → dominated count (max across occurrences).
    chokepoints: BTreeMap<u64, u32>,
    nodes: i64,
    edges: i64,
    scc: i64,
}

impl Prism {
    /// Compute the interpretive diff `base → head`.
    pub fn run_diff(&self, base: RunId, head: RunId) -> Result<RunDiffView> {
        let pa = self.run_profile(base)?;
        let pb = self.run_profile(head)?;

        let added_packages = purls_for(&difference(&pb.packages, &pa.packages), &pb.package_purls);
        let removed_packages =
            purls_for(&difference(&pa.packages, &pb.packages), &pa.package_purls);
        let added_versions = purls_for(&difference(&pb.versions, &pa.versions), &pb.version_purls);
        let removed_versions =
            purls_for(&difference(&pa.versions, &pb.versions), &pa.version_purls);

        let new_duplicate_families =
            purls_for(&difference(&pb.fragmented, &pa.fragmented), &pb.package_purls);
        let resolved_duplicate_families =
            purls_for(&difference(&pa.fragmented, &pb.fragmented), &pa.package_purls);

        let new_cycles: Vec<CycleDiffEntry> = pb
            .cycles
            .iter()
            .filter(|(k, _)| !pa.cycles.contains_key(*k))
            .map(|(_, v)| v.clone())
            .collect();
        let resolved_cycles: Vec<CycleDiffEntry> = pa
            .cycles
            .iter()
            .filter(|(k, _)| !pb.cycles.contains_key(*k))
            .map(|(_, v)| v.clone())
            .collect();

        // Scope shifts: union of scope labels in either run.
        let mut scope_labels: BTreeSet<&'static str> = BTreeSet::new();
        scope_labels.extend(pa.scopes.keys().copied());
        scope_labels.extend(pb.scopes.keys().copied());
        let mut scope_shifts = Vec::new();
        for label in scope_labels {
            let (af, ae) = pa.scopes.get(label).copied().unwrap_or((0, 0));
            let (bf, be) = pb.scopes.get(label).copied().unwrap_or((0, 0));
            let footprint_delta = bf - af;
            let exclusive_delta = be - ae;
            if footprint_delta != 0 || exclusive_delta != 0 {
                scope_shifts.push(ScopeShift {
                    kind: label,
                    footprint_delta,
                    exclusive_delta,
                });
            }
        }

        // Hotspots: chokepoints new or grown by ≥ 5 dominated, surfaced if they
        // dominate ≥ 5 in head.
        let mut new_hotspots = Vec::new();
        for (&pkg, &head_dom) in &pb.chokepoints {
            if head_dom < 5 {
                continue;
            }
            let base_dom = pa.chokepoints.get(&pkg).copied().unwrap_or(0);
            if head_dom >= base_dom.saturating_add(5) {
                new_hotspots.push(HotspotShift {
                    package_purl: pb
                        .package_purls
                        .get(&pkg)
                        .cloned()
                        .unwrap_or_else(|| format!("package:{pkg}")),
                    base_dominated: base_dom,
                    head_dominated: head_dom,
                });
            }
        }
        new_hotspots.sort_by(|a, b| {
            b.head_dominated
                .cmp(&a.head_dominated)
                .then(a.package_purl.cmp(&b.package_purl))
        });

        Ok(RunDiffView {
            base,
            head,
            nodes_delta: pb.nodes - pa.nodes,
            edges_delta: pb.edges - pa.edges,
            scc_delta: pb.scc - pa.scc,
            duplicate_families_delta: pb.fragmented.len() as i64 - pa.fragmented.len() as i64,
            added_packages,
            removed_packages,
            added_versions,
            removed_versions,
            new_duplicate_families,
            resolved_duplicate_families,
            new_cycles,
            resolved_cycles,
            scope_shifts,
            new_hotspots,
        })
    }

    fn run_profile(&self, run: RunId) -> Result<RunProfile> {
        let g = self.load_graph(run)?;

        let mut versions = BTreeSet::new();
        let mut packages = BTreeSet::new();
        let mut version_purls = BTreeMap::new();
        let mut package_purls = BTreeMap::new();
        for i in 0..g.node_count() as u32 {
            let vid = g.node_version(GraphNodeId::new(i))?;
            if versions.insert(vid.get()) {
                version_purls.insert(vid.get(), self.version_label(vid));
            }
            if let Some(pkg) = self.package_of(vid) {
                if packages.insert(pkg.get()) {
                    package_purls.insert(pkg.get(), self.package_purl(pkg));
                }
            }
        }

        let mut fragmented = BTreeSet::new();
        for c in family_clusters(&g, |v| self.package_of(v))? {
            if c.is_version_fragmented() {
                fragmented.insert(c.package_id.get());
                package_purls
                    .entry(c.package_id.get())
                    .or_insert_with(|| self.package_purl(c.package_id));
            }
        }

        let mut cycles = BTreeMap::new();
        for c in classify_cycles(&g)? {
            let mut key: Vec<u64> = Vec::with_capacity(c.members.len());
            let mut members = Vec::with_capacity(c.members.len());
            for &m in &c.members {
                if let Some(pkg) = self.package_of(g.node_version(m)?) {
                    key.push(pkg.get());
                    members.push(self.package_purl(pkg));
                } else {
                    members.push(self.version_label(g.node_version(m)?));
                }
            }
            key.sort_unstable();
            key.dedup();
            let (class, _) = crate::structure_views::cycle_class_label(c.class);
            members.sort();
            cycles.insert(
                key,
                CycleDiffEntry {
                    members,
                    class,
                    runtime_relevant: c.runtime_relevant,
                },
            );
        }

        let mut scopes = BTreeMap::new();
        for fp in scope_summary(&g)? {
            scopes.insert(
                kind_label(fp.kind),
                (fp.footprint.len() as i64, fp.exclusive.len() as i64),
            );
        }

        let mut chokepoints = BTreeMap::new();
        let dt = dominators(&g)?;
        for c in dt.chokepoints(&g, 1)? {
            if let Some(pkg) = self.package_of(g.node_version(c.node)?) {
                let e = chokepoints.entry(pkg.get()).or_insert(0u32);
                *e = (*e).max(c.dominated);
                package_purls
                    .entry(pkg.get())
                    .or_insert_with(|| self.package_purl(pkg));
            }
        }

        let scc = prism_graph::tarjan_scc(&g)?.len() as i64;
        Ok(RunProfile {
            versions,
            packages,
            version_purls,
            package_purls,
            fragmented,
            cycles,
            scopes,
            chokepoints,
            nodes: g.node_count() as i64,
            edges: g.edge_count() as i64,
            scc,
        })
    }
}

/// Set difference `a \ b` over interned id sets.
fn difference(a: &BTreeSet<u64>, b: &BTreeSet<u64>) -> Vec<u64> {
    a.difference(b).copied().collect()
}

/// Resolve a list of ids to purls (sorted for stable output).
fn purls_for(ids: &[u64], map: &BTreeMap<u64, String>) -> Vec<String> {
    let mut out: Vec<String> = ids
        .iter()
        .map(|id| map.get(id).cloned().unwrap_or_else(|| format!("id:{id}")))
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difference_is_set_minus() {
        let a: BTreeSet<u64> = [1, 2, 3].into_iter().collect();
        let b: BTreeSet<u64> = [2, 3, 4].into_iter().collect();
        assert_eq!(difference(&a, &b), vec![1]);
        assert_eq!(difference(&b, &a), vec![4]);
    }

    #[test]
    fn purls_for_resolves_and_sorts() {
        let map: BTreeMap<u64, String> =
            [(1, "pkg:npm/b".into()), (2, "pkg:npm/a".into())].into_iter().collect();
        assert_eq!(purls_for(&[1, 2], &map), vec!["pkg:npm/a", "pkg:npm/b"]);
        assert_eq!(purls_for(&[9], &map), vec!["id:9"]);
    }
}
