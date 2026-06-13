//! Core domain entities (architecture doc, *Canonical Domain Model*).
//!
//! These are plain data records. Large or rarely-queried payloads (constraint
//! ASTs, condition ASTs, raw metadata) are referenced by [`BlobId`] rather than
//! inlined, keeping the queryable records small and fixed-shape.

use crate::flags::{EdgeFlags, FidelityCaveats, NodeFlags, VersionFlags};
use crate::ids::{
    BlobId, EcosystemId, EnvironmentId, GraphEdgeId, GraphNodeId, HashDigest, PackageId, RequirementId,
    RunId, SnapshotId, StringId, VersionId,
};

/// A package family within an ecosystem (a name without a specific version).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub id: PackageId,
    pub ecosystem: EcosystemId,
    pub namespace: Option<StringId>,
    pub name: StringId,
    /// Sole identity key within an ecosystem; the output of the plugin's `NameNormalizer`.
    pub canonical_name: StringId,
    pub purl_base: StringId,
    pub repository_url: Option<StringId>,
    pub homepage_url: Option<StringId>,
    pub metadata_ref: Option<BlobId>,
}

/// A concrete published version of a package family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageVersion {
    pub id: VersionId,
    pub package_id: PackageId,
    pub version_raw: StringId,
    pub version_norm: StringId,
    /// Ecosystem-specific total-order sort key (see `VersionScheme`).
    pub version_sort_key: Vec<u8>,
    pub purl: StringId,
    pub released_at_unix: Option<i64>,
    pub flags: VersionFlags,
    pub license_expr: Option<StringId>,
    pub artifact_hash: Option<HashDigest>,
    pub metadata_ref: Option<BlobId>,
}

/// The kind of a declared dependency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DependencyKind {
    Runtime,
    Dev,
    Build,
    Peer,
    Optional,
    Test,
    Plugin,
    Workspace,
}

/// Whether a requirement's condition gates on the static environment or on
/// resolution-time feature/extra activation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConditionKind {
    /// No condition; always active.
    None,
    /// Evaluated against [`Environment`] (platform, language version, markers).
    Environment,
    /// Evaluated against the resolved activation closure (Cargo features, PyPI extras).
    Activation,
}

/// A declared dependency constraint from one package version to a target family.
///
/// Distinct from a resolved [`GraphEdge`]: it records *what was declared*, not
/// *what was selected*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requirement {
    pub id: RequirementId,
    pub src_version_id: VersionId,
    /// Raw/uninterpolated target coordinate as written.
    pub declared_target: StringId,
    /// Resolved target family, filled in after effective-POM build / patch / replace.
    pub dst_package_id: Option<PackageId>,
    pub target_alias: Option<StringId>,
    pub raw_requirement: StringId,
    pub constraint_ref: BlobId,
    /// Version supplied by depMgmt/BOM/workspace inheritance, if any.
    pub managed_version_ref: Option<BlobId>,
    pub kind: DependencyKind,
    pub scope: Option<StringId>,
    pub condition_kind: ConditionKind,
    pub condition_ref: Option<BlobId>,
    pub features_or_extras_ref: Option<BlobId>,
    pub metadata_ref: Option<BlobId>,
}

/// Why a candidate set has the shape it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateReason {
    /// Derived directly from the requirement's version constraint.
    Constraint,
    /// Locked to a single version by a lockfile.
    Locked,
    /// Empty because the constraint is unsatisfiable in this snapshot.
    Unsatisfiable,
}

/// Why a specific ordinal was excluded from a candidate set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExclusionReason {
    Yanked,
    RequiresPythonMismatch,
    NoCompatibleWheel,
    DynamicMetadataUnknown,
    Excluded,
    Retracted,
}

/// The set of versions that may satisfy a requirement under a snapshot,
/// environment, and source policy — represented as ordinal ranges over the
/// package's snapshot-pinned sorted version list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateSet {
    pub requirement_id: RequirementId,
    pub environment_id: EnvironmentId,
    pub snapshot_id: SnapshotId,
    pub package_id: PackageId,
    pub ranges: Vec<core::ops::Range<u32>>,
    pub exclusions: Vec<(u32, ExclusionReason)>,
    pub reason: CandidateReason,
}

/// The resolution-relevant target description (content-addressed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Environment {
    pub id: EnvironmentId,
    pub language_version: Option<StringId>,
    pub implementation: Option<StringId>,
    pub platform: Option<StringId>,
    pub markers: Vec<(StringId, StringId)>,
    pub toggles: Vec<StringId>,
}

/// Registry metadata frozen at a point in time, making deterministic runs reproducible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrySnapshot {
    pub id: SnapshotId,
    pub source: StringId,
    pub captured_at_unix: i64,
    pub digest: HashDigest,
    pub bindings_ref: BlobId,
}

/// Status of a resolver run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Pending,
    Complete,
    Truncated,
    Failed,
}

/// Coarse fidelity tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FidelityTier {
    /// Read verbatim from a lockfile.
    LockfileExact,
    /// Produced by a native resolver tool.
    NativeToolResolved,
    /// Prism synthetic resolver, full fixpoint.
    ModelResolved,
    /// Ranges only, no concrete selection.
    CandidateSpaceOnly,
}

/// The single fidelity type: a base tier plus a set of "what was dropped" caveats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphFidelity {
    pub tier: FidelityTier,
    pub caveats: FidelityCaveats,
}

impl GraphFidelity {
    /// A clean tier with no caveats.
    pub const fn exact(tier: FidelityTier) -> Self {
        Self {
            tier,
            caveats: FidelityCaveats::EMPTY,
        }
    }
}

/// A specific analysis context: one attempt to resolve a dependency graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolverRun {
    pub id: RunId,
    pub ecosystem: EcosystemId,
    pub root_set_hash: HashDigest,
    pub resolver_name: StringId,
    pub resolver_version: Option<StringId>,
    pub environment_id: EnvironmentId,
    pub snapshot_id: SnapshotId,
    pub input_manifest_ref: Option<BlobId>,
    pub input_lockfile_ref: Option<BlobId>,
    pub status: RunStatus,
    pub fidelity: GraphFidelity,
    /// Reference timestamp pinned for reproducible staleness filtrations.
    pub reference_unix: i64,
    pub created_at_unix: i64,
    pub metadata_ref: Option<BlobId>,
}

/// Structured occurrence context, canonicalized into `GraphNode::occurrence_key`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OccurrenceContext {
    pub path: Option<StringId>,
    pub feature_set_ref: Option<BlobId>,
    pub extras_ref: Option<BlobId>,
    pub platform: Option<StringId>,
    pub mediation_ref: Option<BlobId>,
}

impl OccurrenceContext {
    /// The root occurrence context (empty).
    pub fn root() -> Self {
        Self::default()
    }

    /// True if every field is unset (the root/default context).
    pub fn is_root(&self) -> bool {
        self == &Self::default()
    }
}

/// A node in a resolved graph snapshot: an *occurrence* of a package version,
/// not merely the version itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphNode {
    pub node_id: GraphNodeId,
    pub version_id: VersionId,
    pub occurrence_key: StringId,
    /// Minimum depth over discovery paths (BFS).
    pub depth: u32,
    /// Resolved/unified feature+extra closure (Cargo/PyPI).
    pub enabled_features_ref: Option<BlobId>,
    pub flags: NodeFlags,
    pub attributes_ref: Option<BlobId>,
}

/// A resolved dependency edge from one occurrence to another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphEdge {
    pub edge_id: GraphEdgeId,
    pub from: GraphNodeId,
    pub to: GraphNodeId,
    /// Position within the source node's dependency list (stable ordering).
    pub ordinal: u32,
    pub requirement_id: Option<RequirementId>,
    pub kind: DependencyKind,
    pub flags: EdgeFlags,
    pub attributes_ref: Option<BlobId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occurrence_root_is_default() {
        let r = OccurrenceContext::root();
        assert!(r.is_root());
        let mut c = OccurrenceContext::root();
        c.path = Some(StringId::new(1));
        assert!(!c.is_root());
    }

    #[test]
    fn fidelity_exact_has_no_caveats() {
        let f = GraphFidelity::exact(FidelityTier::LockfileExact);
        assert!(f.caveats.is_empty());
        assert_eq!(f.tier, FidelityTier::LockfileExact);
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // a one-element Vec<Range> is intentional here
    fn candidate_set_holds_ranges_and_reasons() {
        let cs = CandidateSet {
            requirement_id: RequirementId::new(1),
            environment_id: EnvironmentId::new(2),
            snapshot_id: SnapshotId::new(3),
            package_id: PackageId::new(4),
            ranges: vec![2..5],
            exclusions: vec![(3, ExclusionReason::Yanked)],
            reason: CandidateReason::Constraint,
        };
        assert_eq!(cs.ranges[0], 2..5);
        assert_eq!(cs.exclusions[0].1, ExclusionReason::Yanked);
    }
}
