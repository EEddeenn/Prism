//! Duplicate-version analysis (architecture doc, *Duplicate Version Clusters*).
//!
//! Two flavors:
//! - [`version_clusters`] groups occurrence nodes by their `version_id` — the
//!   same exact version appearing at multiple occurrences (npm hoisting paths,
//!   etc.). A cluster with more than one occurrence is a duplicated *occurrence*
//!   of one version.
//! - [`family_clusters`] takes a `VersionId -> PackageId` mapping and clusters by
//!   package family, reporting how many distinct versions of one family coexist
//!   (the classic "multiple versions of the same package" report) with depth and
//!   occurrence metrics.
//!
//! Clustering counts distinct ids, not edges, so it is unaffected by multigraph
//! parallel edges.

use std::collections::BTreeMap;

use prism_model::{GraphNodeId, PackageId, Result, VersionId};

use crate::snapshot::GraphSnapshot;

/// Occurrences of a single exact package version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionCluster {
    /// The shared version id.
    pub version_id: VersionId,
    /// Every node that resolves to this version.
    pub occurrences: Vec<GraphNodeId>,
    /// Minimum BFS depth across the occurrences.
    pub min_depth: u32,
    /// Maximum BFS depth across the occurrences.
    pub max_depth: u32,
}

impl VersionCluster {
    /// Number of occurrences of this version.
    pub fn occurrence_count(&self) -> usize {
        self.occurrences.len()
    }

    /// True if this version appears at more than one occurrence.
    pub fn is_duplicated(&self) -> bool {
        self.occurrences.len() > 1
    }
}

/// Group occurrence nodes by exact `version_id`.
///
/// Returns one [`VersionCluster`] per distinct version, ordered by `version_id`.
/// Use [`VersionCluster::is_duplicated`] to filter for the actual duplicates.
pub fn version_clusters(g: &GraphSnapshot) -> Result<Vec<VersionCluster>> {
    // version_id -> (occurrences, min_depth, max_depth)
    let mut groups: BTreeMap<u64, (Vec<GraphNodeId>, u32, u32)> = BTreeMap::new();
    for i in 0..g.node_count() as u32 {
        let node = GraphNodeId::new(i);
        let vid = g.node_version(node)?;
        let depth = g.node_depth(node)?;
        let entry = groups
            .entry(vid.get())
            .or_insert_with(|| (Vec::new(), u32::MAX, 0));
        entry.0.push(node);
        entry.1 = entry.1.min(depth);
        entry.2 = entry.2.max(depth);
    }

    Ok(groups
        .into_iter()
        .map(|(vid, (occurrences, min_depth, max_depth))| VersionCluster {
            version_id: VersionId::new(vid),
            occurrences,
            min_depth,
            max_depth,
        })
        .collect())
}

/// A package family that has more than one coexisting version, or one version at
/// one or more occurrences — the "duplicate-version cluster" view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyCluster {
    /// The package family.
    pub package_id: PackageId,
    /// Distinct versions of this family present in the graph, sorted ascending.
    pub versions: Vec<VersionId>,
    /// Total occurrence nodes across all versions of this family.
    pub occurrence_count: u32,
    /// Minimum BFS depth across all occurrences of the family.
    pub min_depth: u32,
    /// Maximum BFS depth across all occurrences of the family.
    pub max_depth: u32,
}

impl FamilyCluster {
    /// Number of distinct versions of this family in the graph.
    pub fn version_count(&self) -> usize {
        self.versions.len()
    }

    /// True if more than one distinct version of this family coexists — the
    /// condition that makes this a *duplicate-version* situation.
    pub fn is_version_fragmented(&self) -> bool {
        self.versions.len() > 1
    }
}

/// Cluster occurrence nodes by package family using a `VersionId -> PackageId`
/// mapping, reporting version/occurrence counts and depth span per family.
///
/// `package_of` resolves each node's version to its package family. A version
/// not covered by `package_of` (mapping returns `None`) is skipped — its nodes
/// are simply not attributed to any family. Families are returned sorted by
/// `package_id`. To get only the fragmented families, filter on
/// [`FamilyCluster::is_version_fragmented`].
pub fn family_clusters<F>(g: &GraphSnapshot, package_of: F) -> Result<Vec<FamilyCluster>>
where
    F: Fn(VersionId) -> Option<PackageId>,
{
    // package_id -> (set of versions, occurrence_count, min_depth, max_depth)
    struct Acc {
        versions: std::collections::BTreeSet<u64>,
        occurrences: u32,
        min_depth: u32,
        max_depth: u32,
    }
    let mut groups: BTreeMap<u64, Acc> = BTreeMap::new();

    for i in 0..g.node_count() as u32 {
        let node = GraphNodeId::new(i);
        let vid = g.node_version(node)?;
        let Some(pkg) = package_of(vid) else {
            continue;
        };
        let depth = g.node_depth(node)?;
        let entry = groups.entry(pkg.get()).or_insert_with(|| Acc {
            versions: std::collections::BTreeSet::new(),
            occurrences: 0,
            min_depth: u32::MAX,
            max_depth: 0,
        });
        entry.versions.insert(vid.get());
        entry.occurrences += 1;
        entry.min_depth = entry.min_depth.min(depth);
        entry.max_depth = entry.max_depth.max(depth);
    }

    Ok(groups
        .into_iter()
        .map(|(pkg, acc)| FamilyCluster {
            package_id: PackageId::new(pkg),
            versions: acc.versions.into_iter().map(VersionId::new).collect(),
            occurrence_count: acc.occurrences,
            min_depth: acc.min_depth,
            max_depth: acc.max_depth,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{BuildEdge, BuildNode, GraphBuilder};
    use prism_model::RunId;

    fn nid(i: u32) -> GraphNodeId {
        GraphNodeId::new(i)
    }

    // Graph with 5 occurrences:
    //   node 0: version 10 (depth 0)
    //   node 1: version 20 (depth 1)
    //   node 2: version 20 (depth 2)  <- same version 20 duplicated
    //   node 3: version 21 (depth 2)  <- different version, same family as 20
    //   node 4: version 30 (depth 1)
    fn graph() -> GraphSnapshot {
        let mut b = GraphBuilder::new();
        let specs = [(10u64, 0u32), (20, 1), (20, 2), (21, 2), (30, 1)];
        for (v, d) in specs {
            b.add_node(BuildNode::new(VersionId::new(v), d)).unwrap();
        }
        // Edges just to have a connected-ish graph; clustering ignores them.
        b.add_edge(BuildEdge::new(nid(0), nid(1), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(0), nid(4), 1)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(2), 0)).unwrap();
        b.add_edge(BuildEdge::new(nid(1), nid(3), 1)).unwrap();
        b.finalize(RunId::new(1)).unwrap()
    }

    #[test]
    fn version_clusters_group_occurrences() {
        let g = graph();
        let clusters = version_clusters(&g).unwrap();
        // Distinct versions: 10, 20, 21, 30 -> 4 clusters.
        assert_eq!(clusters.len(), 4);

        let dup = clusters
            .iter()
            .find(|c| c.version_id == VersionId::new(20))
            .unwrap();
        assert!(dup.is_duplicated());
        assert_eq!(dup.occurrence_count(), 2);
        assert_eq!(dup.min_depth, 1);
        assert_eq!(dup.max_depth, 2);
        let occ: Vec<u32> = dup.occurrences.iter().map(|x| x.get()).collect();
        assert_eq!(occ, vec![1, 2]);

        // Version 10 is unique.
        let solo = clusters
            .iter()
            .find(|c| c.version_id == VersionId::new(10))
            .unwrap();
        assert!(!solo.is_duplicated());
        assert_eq!(solo.occurrence_count(), 1);

        // Only one version is occurrence-duplicated.
        assert_eq!(clusters.iter().filter(|c| c.is_duplicated()).count(), 1);
    }

    #[test]
    fn family_clusters_count_distinct_versions() {
        let g = graph();
        // Family mapping: versions 20 and 21 belong to package 100;
        // version 10 -> package 50; version 30 -> package 70.
        let map = |v: VersionId| -> Option<PackageId> {
            match v.get() {
                10 => Some(PackageId::new(50)),
                20 | 21 => Some(PackageId::new(100)),
                30 => Some(PackageId::new(70)),
                _ => None,
            }
        };
        let fams = family_clusters(&g, map).unwrap();
        assert_eq!(fams.len(), 3);

        let frag = fams
            .iter()
            .find(|f| f.package_id == PackageId::new(100))
            .unwrap();
        assert!(frag.is_version_fragmented());
        assert_eq!(frag.version_count(), 2);
        assert_eq!(frag.versions, vec![VersionId::new(20), VersionId::new(21)]);
        // Occurrences: node 1 (v20), node 2 (v20), node 3 (v21) = 3.
        assert_eq!(frag.occurrence_count, 3);
        assert_eq!(frag.min_depth, 1);
        assert_eq!(frag.max_depth, 2);

        // Exactly one family is version-fragmented.
        assert_eq!(
            fams.iter().filter(|f| f.is_version_fragmented()).count(),
            1
        );

        // Non-fragmented families.
        let solo = fams
            .iter()
            .find(|f| f.package_id == PackageId::new(50))
            .unwrap();
        assert!(!solo.is_version_fragmented());
        assert_eq!(solo.version_count(), 1);
    }

    #[test]
    fn unmapped_versions_are_skipped() {
        let g = graph();
        // Only map version 10; everything else returns None.
        let map = |v: VersionId| -> Option<PackageId> {
            (v.get() == 10).then(|| PackageId::new(1))
        };
        let fams = family_clusters(&g, map).unwrap();
        assert_eq!(fams.len(), 1);
        assert_eq!(fams[0].package_id, PackageId::new(1));
        assert_eq!(fams[0].occurrence_count, 1);
    }

    #[test]
    fn empty_graph_has_no_clusters() {
        let g = GraphBuilder::new().finalize(RunId::new(1)).unwrap();
        assert!(version_clusters(&g).unwrap().is_empty());
        assert!(family_clusters(&g, |_| Some(PackageId::new(1)))
            .unwrap()
            .is_empty());
    }
}
