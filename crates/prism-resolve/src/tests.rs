//! End-to-end engine tests over an in-crate mock requirement+candidate source.
//!
//! The mocks model a tiny synthetic ecosystem: each package family maps to its
//! declared requirements, and (for the policy test) a [`VersionCatalog`] holds
//! multiple versions so lowest/highest diverge. Node and edge sets are asserted
//! exactly.

use std::collections::BTreeMap;

use prism_model::{ConditionKind, DependencyKind, EdgeFlags, EnvironmentId, NodeFlags, PackageId, RequirementId, VersionId};
use prism_plugin_api::{ActivationContext, GateInput, ResolvePolicy, ResolvedGraph};

use crate::{
    AlwaysActive, CandidateSource, ConditionGate, DeclaredRequirement, Engine, ResolveBudget,
    RequirementSource, VersionCatalog,
};

// --- mock requirement source ----------------------------------------------

/// A synthetic package set: one entry per *version*, declaring its family and
/// out-requirements. Selection (below) maps a family to a single version, so
/// `VersionId == PackageId.get()` by convention in these fixtures.
#[derive(Default)]
struct MockReqs {
    /// version -> (family, requirements)
    versions: BTreeMap<u64, (PackageId, Vec<DeclaredRequirement>)>,
    /// version -> features it activates on itself unconditionally
    defaults: BTreeMap<u64, Vec<String>>,
}

impl MockReqs {
    fn family(&mut self, version: u64, pkg: u64, reqs: Vec<DeclaredRequirement>) {
        self.versions
            .insert(version, (PackageId::new(pkg), reqs));
    }
}

impl RequirementSource for MockReqs {
    fn requirements(
        &self,
        version: VersionId,
    ) -> Result<Vec<DeclaredRequirement>, prism_plugin_api::ResolveError> {
        Ok(self
            .versions
            .get(&version.get())
            .map(|(_, r)| r.clone())
            .unwrap_or_default())
    }

    fn package_of(
        &self,
        version: VersionId,
    ) -> Result<PackageId, prism_plugin_api::ResolveError> {
        self.versions
            .get(&version.get())
            .map(|(p, _)| *p)
            .ok_or_else(|| prism_plugin_api::ResolveError::Other(format!("unknown version {version}")))
    }

    fn default_activations(
        &self,
        version: VersionId,
    ) -> Result<Vec<String>, prism_plugin_api::ResolveError> {
        Ok(self.defaults.get(&version.get()).cloned().unwrap_or_default())
    }
}

// --- mock candidate source: one version per family ------------------------

/// Maps each package family to exactly one version, ignoring the matcher. Used
/// by the structural tests where version choice is not under test.
struct OneVersion {
    /// family -> the single version of it
    pick: BTreeMap<u64, u64>,
}

impl OneVersion {
    fn of(pairs: &[(u64, u64)]) -> Self {
        Self {
            pick: pairs.iter().copied().collect(),
        }
    }
}

impl CandidateSource for OneVersion {
    fn select(
        &self,
        package: PackageId,
        _matcher: &str,
        _environment: EnvironmentId,
        _activation: &ActivationContext,
        _policy: &ResolvePolicy,
    ) -> Result<Option<VersionId>, prism_plugin_api::ResolveError> {
        Ok(self.pick.get(&package.get()).copied().map(VersionId::new))
    }
}

// --- helpers ---------------------------------------------------------------

fn req(id: u64, target: u64, matcher: &str) -> DeclaredRequirement {
    DeclaredRequirement::runtime(RequirementId::new(id), PackageId::new(target), matcher)
}

/// Sorted (from, to) edge pairs for stable comparison.
fn edge_pairs(g: &ResolvedGraph) -> Vec<(u32, u32)> {
    let mut e: Vec<(u32, u32)> = g.edges.iter().map(|x| (x.from, x.to)).collect();
    e.sort_unstable();
    e
}

/// Sorted node version ids.
fn node_versions(g: &ResolvedGraph) -> Vec<u64> {
    let mut v: Vec<u64> = g.nodes.iter().map(|n| n.version_id.get()).collect();
    v.sort_unstable();
    v
}

const ENV: EnvironmentId = EnvironmentId(7);

// --- (1) diamond resolves the shared target once --------------------------

#[test]
fn diamond_shares_target_and_terminates() {
    // A->B, A->C, B->D, C->D. Families/versions: A=1,B=2,C=3,D=4.
    let mut reqs = MockReqs::default();
    reqs.family(1, 1, vec![req(10, 2, "*"), req(11, 3, "*")]); // A
    reqs.family(2, 2, vec![req(12, 4, "*")]); // B
    reqs.family(3, 3, vec![req(13, 4, "*")]); // C
    reqs.family(4, 4, vec![]); // D (leaf)

    let cands = OneVersion::of(&[(1, 1), (2, 2), (3, 3), (4, 4)]);
    let engine = Engine::new(reqs, cands, ResolvePolicy::Latest, ENV);
    let out = engine.resolve(&[VersionId::new(1)]).unwrap();
    let g = &out.graph;

    // Exactly four occurrences: A,B,C,D — D appears once (shared).
    assert_eq!(node_versions(g), vec![1, 2, 3, 4]);
    assert_eq!(g.node_count(), 4);

    // Locate locals by version.
    let id = |v: u64| g.nodes.iter().find(|n| n.version_id.get() == v).unwrap().local_id;
    let (a, b, c, d) = (id(1), id(2), id(3), id(4));
    let mut expected = vec![(a, b), (a, c), (b, d), (c, d)];
    expected.sort_unstable();
    assert_eq!(edge_pairs(g), expected);

    // All forward-resolved, none back-edges, not truncated.
    assert!(g.edges.iter().all(|e| e.flags.contains(EdgeFlags::RESOLVED)));
    assert!(g.edges.iter().all(|e| !e.flags.contains(EdgeFlags::BACK_EDGE)));
    assert!(!g.truncated);
    // D's depth is the min over the two paths (2).
    assert_eq!(g.nodes[d as usize].depth, 2);
    assert!(g.nodes[a as usize].flags.contains(NodeFlags::ROOT));
}

// --- (2) cycle terminates via a back-edge ---------------------------------

#[test]
fn cycle_closes_with_back_edge() {
    // A->B->A. Families/versions A=1,B=2.
    let mut reqs = MockReqs::default();
    reqs.family(1, 1, vec![req(10, 2, "*")]); // A -> B
    reqs.family(2, 2, vec![req(11, 1, "*")]); // B -> A (cycle)

    let cands = OneVersion::of(&[(1, 1), (2, 2)]);
    let engine = Engine::new(reqs, cands, ResolvePolicy::Latest, ENV);
    let out = engine.resolve(&[VersionId::new(1)]).unwrap();
    let g = &out.graph;

    // Exactly two occurrences (no infinite minting).
    assert_eq!(node_versions(g), vec![1, 2]);
    let id = |v: u64| g.nodes.iter().find(|n| n.version_id.get() == v).unwrap().local_id;
    let (a, b) = (id(1), id(2));

    // Edges: A->B (resolved forward) and B->A (back-edge to the root A).
    assert_eq!(edge_pairs(g), {
        let mut e = vec![(a, b), (b, a)];
        e.sort_unstable();
        e
    });
    let back = g.edges.iter().find(|e| e.from == b && e.to == a).unwrap();
    assert!(back.flags.contains(EdgeFlags::BACK_EDGE));
    let fwd = g.edges.iter().find(|e| e.from == a && e.to == b).unwrap();
    assert!(!fwd.flags.contains(EdgeFlags::BACK_EDGE));
    assert!(!g.truncated);
}

// --- (3) depth budget truncates and flags ---------------------------------

#[test]
fn depth_budget_truncates_and_flags() {
    // A chain A->B->C->D->E; cap depth at 2 so D/E are never expanded/minted.
    let mut reqs = MockReqs::default();
    reqs.family(1, 1, vec![req(10, 2, "*")]);
    reqs.family(2, 2, vec![req(11, 3, "*")]);
    reqs.family(3, 3, vec![req(12, 4, "*")]);
    reqs.family(4, 4, vec![req(13, 5, "*")]);
    reqs.family(5, 5, vec![]);

    let cands = OneVersion::of(&[(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]);
    let engine = Engine::with_gate(
        reqs,
        cands,
        AlwaysActive,
        ResolvePolicy::Latest,
        ENV,
        ResolveBudget::with_max_depth(2),
    );
    let out = engine.resolve(&[VersionId::new(1)]).unwrap();
    let g = &out.graph;

    // Depth 0:A, 1:B, 2:C. C is at the cap → kept but not expanded; D,E absent.
    assert_eq!(node_versions(g), vec![1, 2, 3]);
    assert!(g.truncated);
    let fidelity = g.fidelity.unwrap();
    assert!(fidelity
        .caveats
        .contains(prism_model::FidelityCaveats::BUDGET_TRUNCATED));
    // The depth-capped node carries the TRUNCATED flag.
    let c = g.nodes.iter().find(|n| n.version_id.get() == 3).unwrap();
    assert!(c.flags.contains(NodeFlags::TRUNCATED));
    // Only A->B, B->C edges materialized.
    assert_eq!(edge_pairs(g).len(), 2);
}

#[test]
fn leaf_at_exactly_max_depth_is_not_flagged_truncated() {
    // A(d0) -> B(d1). B is a genuine leaf (no requirements) landing exactly at
    // the depth cap: nothing is cut, so the graph must NOT report truncated and
    // B must NOT carry the TRUNCATED flag.
    let mut reqs = MockReqs::default();
    reqs.family(1, 1, vec![req(10, 2, "*")]);
    reqs.family(2, 2, vec![]); // leaf
    let cands = OneVersion::of(&[(1, 1), (2, 2)]);
    let engine = Engine::with_gate(
        reqs,
        cands,
        AlwaysActive,
        ResolvePolicy::Latest,
        ENV,
        ResolveBudget::with_max_depth(1),
    );
    let out = engine.resolve(&[VersionId::new(1)]).unwrap();
    let g = &out.graph;
    assert_eq!(node_versions(g), vec![1, 2]);
    assert!(!g.truncated, "a leaf at the cap cut nothing");
    let b = g.nodes.iter().find(|n| n.version_id.get() == 2).unwrap();
    assert!(!b.flags.contains(NodeFlags::TRUNCATED));
    assert!(!g
        .fidelity
        .unwrap()
        .caveats
        .contains(prism_model::FidelityCaveats::BUDGET_TRUNCATED));
}

#[test]
fn unsatisfiable_required_dependency_errors_but_optional_is_skipped() {
    use prism_plugin_api::ResolveError;
    // A requires family 99, which the candidate source cannot satisfy (returns
    // Ok(None)). A required dep must surface as Unsatisfiable, not vanish.
    let mut reqs = MockReqs::default();
    reqs.family(1, 1, vec![req(10, 99, "*")]);
    let cands = OneVersion::of(&[(1, 1)]); // no version for family 99
    let engine = Engine::new(reqs, cands, ResolvePolicy::Latest, ENV);
    let err = engine.resolve(&[VersionId::new(1)]).unwrap_err();
    assert!(matches!(err, ResolveError::Unsatisfiable { .. }), "{err:?}");

    // The same dependency declared OPTIONAL is an acceptable "not installed".
    let mut reqs2 = MockReqs::default();
    let optional = DeclaredRequirement {
        kind: DependencyKind::Optional,
        ..req(10, 99, "*")
    };
    reqs2.family(1, 1, vec![optional]);
    let cands2 = OneVersion::of(&[(1, 1)]);
    let engine2 = Engine::new(reqs2, cands2, ResolvePolicy::Latest, ENV);
    let out = engine2.resolve(&[VersionId::new(1)]).unwrap();
    assert_eq!(node_versions(&out.graph), vec![1], "only the root, no edge");
    assert!(out.graph.edges.is_empty());
}

// --- (4) feature gate activates an extra edge only when requested ----------

/// A gate that reads `feature:NAME` activation conditions against the closure
/// and treats environment conditions as always-on.
struct FeatureGate;
impl ConditionGate for FeatureGate {
    fn evaluate(
        &self,
        condition: Option<&str>,
        input: GateInput<'_>,
    ) -> Result<bool, prism_plugin_api::ResolveError> {
        match (condition, input) {
            (None, _) => Ok(true),
            (Some(_), GateInput::Unconditional) => Ok(true),
            (Some(_), GateInput::Environment { .. }) => Ok(true),
            (Some(expr), GateInput::Activation { anchor, context }) => {
                let name = expr.strip_prefix("feature:").unwrap_or(expr);
                Ok(context.is_active(anchor, name))
            }
        }
    }
}

fn feature_fixture() -> MockReqs {
    // A (pkg 1) depends on B (pkg 2). B has an optional dep on E (pkg 3) gated on
    // B's own `derive` feature. A *may* activate B's `derive` feature.
    let mut reqs = MockReqs::default();

    // B -> E, gated on activation `feature:derive` anchored at B (pkg 2).
    let gated = DeclaredRequirement {
        requirement_id: RequirementId::new(21),
        target: PackageId::new(3),
        matcher: "*".into(),
        kind: DependencyKind::Optional,
        condition_kind: ConditionKind::Activation,
        condition: Some("feature:derive".into()),
        activates: vec![],
    };
    reqs.family(2, 2, vec![gated]); // B
    reqs.family(3, 3, vec![]); // E (leaf)
    reqs
}

#[test]
fn feature_gate_off_omits_extra_edge() {
    let mut reqs = feature_fixture();
    // A -> B with no activation effect.
    reqs.family(1, 1, vec![req(10, 2, "*")]);

    let cands = OneVersion::of(&[(1, 1), (2, 2), (3, 3)]);
    let engine = Engine::with_gate(
        reqs,
        cands,
        FeatureGate,
        ResolvePolicy::Latest,
        ENV,
        ResolveBudget::default(),
    );
    let out = engine.resolve(&[VersionId::new(1)]).unwrap();
    let g = &out.graph;

    // E is NOT pulled in: derive was never activated on B.
    assert_eq!(node_versions(g), vec![1, 2]);
    assert!(g.edges.iter().all(|e| e.to != g.nodes.iter().find(|n| n.version_id.get() == 3).map(|n| n.local_id).unwrap_or(u32::MAX)));
    assert!(out.activation.is_empty());
}

#[test]
fn feature_gate_on_activates_extra_edge_via_fixpoint() {
    let mut reqs = feature_fixture();
    // A -> B AND activates B's `derive` feature (Cargo `b = { features=["derive"] }`).
    let mut a_to_b = req(10, 2, "*");
    a_to_b.activates = vec!["derive".into()];
    reqs.family(1, 1, vec![a_to_b]);

    let cands = OneVersion::of(&[(1, 1), (2, 2), (3, 3)]);
    let engine = Engine::with_gate(
        reqs,
        cands,
        FeatureGate,
        ResolvePolicy::Latest,
        ENV,
        ResolveBudget::default(),
    );
    let out = engine.resolve(&[VersionId::new(1)]).unwrap();
    let g = &out.graph;

    // After activating derive on B, the fixpoint re-resolves and B->E appears.
    assert_eq!(node_versions(g), vec![1, 2, 3]);
    let id = |v: u64| g.nodes.iter().find(|n| n.version_id.get() == v).unwrap().local_id;
    let (a, b, e) = (id(1), id(2), id(3));
    let mut expected = vec![(a, b), (b, e)];
    expected.sort_unstable();
    assert_eq!(edge_pairs(g), expected);
    // The closure recorded derive on B, and it took >1 pass to converge.
    assert!(out.activation.is_active(PackageId::new(2), "derive"));
    assert!(out.iterations >= 2);
    // The optional edge keeps its declared kind.
    let opt = g.edges.iter().find(|x| x.from == b && x.to == e).unwrap();
    assert_eq!(opt.kind, DependencyKind::Optional);
}

// --- (5) lowest vs highest policy pick different versions ------------------

#[test]
fn lowest_and_highest_policies_pick_different_versions() {
    // Root R (pkg 9, fixed version 100) depends on family P (pkg 1), which has
    // versions 1.0.0 (id 10) and 2.0.0 (id 12). A real catalog drives selection.
    let mut catalog = VersionCatalog::new();
    catalog.add_version(PackageId::new(1), VersionId::new(10), "1.0.0").unwrap();
    catalog.add_version(PackageId::new(1), VersionId::new(12), "2.0.0").unwrap();
    catalog.add_version(PackageId::new(1), VersionId::new(11), "1.5.0-rc.1").unwrap();

    // Requirement source: root R -> P with matcher ">=1".  Both P versions are
    // leaves; we register all candidate version ids so package_of resolves them.
    let make_reqs = || {
        let mut reqs = MockReqs::default();
        reqs.family(100, 9, vec![req(10, 1, ">=1")]); // R -> P
        reqs.family(10, 1, vec![]); // P 1.0.0
        reqs.family(11, 1, vec![]); // P 1.5.0-rc.1
        reqs.family(12, 1, vec![]); // P 2.0.0
        reqs
    };

    let lowest = Engine::new(make_reqs(), catalog.clone(), ResolvePolicy::Lowest, ENV)
        .resolve(&[VersionId::new(100)])
        .unwrap();
    let highest = Engine::new(
        make_reqs(),
        catalog.clone(),
        ResolvePolicy::HighestNonPrerelease,
        ENV,
    )
    .resolve(&[VersionId::new(100)])
    .unwrap();

    // Lowest picks P 1.0.0 (id 10); highest-non-prerelease picks 2.0.0 (id 12),
    // skipping the 1.5.0-rc.1 prerelease.
    assert_eq!(node_versions(&lowest.graph), vec![10, 100]);
    assert_eq!(node_versions(&highest.graph), vec![12, 100]);
    assert_eq!(lowest.policy, ResolvePolicy::Lowest);
    assert_eq!(highest.policy, ResolvePolicy::HighestNonPrerelease);
}
