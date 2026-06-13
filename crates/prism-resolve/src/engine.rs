//! The synthetic two-phase fixpoint resolver.
//!
//! Given a [`RequirementSource`] and a [`CandidateSource`], the engine builds a
//! plain [`ResolvedGraph`] (occurrences + edges) by repeated BFS materialization
//! over a growing [`ActivationContext`], to a fixpoint. The construction mirrors
//! the architecture doc's *Resolution and Graph Construction* pseudocode:
//!
//! - **Cycle guard.** The guard keys on the *package family*, not the occurrence
//!   key: if a target family already appears on the current root→node path the
//!   engine emits a [`EdgeFlags::BACK_EDGE`] to the existing occurrence instead
//!   of recursing forever. Termination is therefore independent of how
//!   path/feature-sensitive occurrence identity is.
//! - **Budgets.** [`ResolveBudget::max_depth`] bounds expansion depth and
//!   [`ResolveBudget::max_nodes`] bounds total occurrences. Hitting either sets
//!   [`ResolvedGraph::truncated`], marks the over-budget source node
//!   [`NodeFlags::TRUNCATED`], and adds the [`FidelityCaveats::BUDGET_TRUNCATED`]
//!   caveat to the run fidelity.
//! - **Activation fixpoint.** Taking an edge can activate features/extras on its
//!   target (Cargo `serde/derive`, PyPI `requests[socks]`) and a version can
//!   activate features on itself (a crate's `default`). Those grow the closure;
//!   while [`ActivationContext::activate`] reports something newly activated the
//!   engine re-runs the whole BFS, so newly-active activation-gated requirements
//!   appear in the final graph.

use std::collections::VecDeque;

use prism_model::{
    EdgeFlags, EnvironmentId, FidelityCaveats, FidelityTier, GraphFidelity, NodeFlags, VersionId,
};
use prism_plugin_api::{
    ActivationContext, GateInput, ResolveError, ResolvePolicy, ResolvedEdge, ResolvedGraph,
};

use crate::source::{CandidateSource, DeclaredRequirement, RequirementSource};

/// Evaluates a requirement's opaque condition against the split gate input.
///
/// The engine never parses conditions itself; it delegates to this trait so
/// environment markers and activation gates route to the right state (the same
/// split as [`prism_plugin_api::ConditionEvaluator`], but standalone so the
/// crate is testable without a full plugin).
pub trait ConditionGate {
    /// Evaluate `condition` (already classified by its [`GateInput`] kind).
    /// `GateInput::Unconditional` is always active.
    fn evaluate(
        &self,
        condition: Option<&str>,
        input: GateInput<'_>,
    ) -> Result<bool, ResolveError>;
}

/// A condition gate that treats every requirement as unconditionally active.
///
/// Useful for ecosystems without conditions, and as the default when the caller
/// supplies none. Activation gates still drive the fixpoint via
/// [`DeclaredRequirement::activates`].
#[derive(Clone, Copy, Debug, Default)]
pub struct AlwaysActive;

impl ConditionGate for AlwaysActive {
    fn evaluate(
        &self,
        _condition: Option<&str>,
        _input: GateInput<'_>,
    ) -> Result<bool, ResolveError> {
        Ok(true)
    }
}

/// Expansion budgets. Both bounds are inclusive caps; exceeding either truncates
/// the run rather than erroring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolveBudget {
    /// Maximum occurrence depth to expand. Nodes at this depth are kept but not
    /// expanded; deeper edges are not minted.
    pub max_depth: u32,
    /// Maximum number of occurrence nodes. Once reached, no new node is minted.
    pub max_nodes: u32,
    /// Hard cap on fixpoint iterations, guarding against a pathological gate that
    /// never converges. Reaching it truncates the run.
    pub max_iterations: u32,
}

impl Default for ResolveBudget {
    fn default() -> Self {
        Self {
            max_depth: 64,
            max_nodes: 100_000,
            max_iterations: 64,
        }
    }
}

impl ResolveBudget {
    /// A budget with a specific depth cap and otherwise-generous defaults.
    pub fn with_max_depth(max_depth: u32) -> Self {
        Self {
            max_depth,
            ..Self::default()
        }
    }
}

/// The configured synthetic resolver: inputs + policy + budgets.
///
/// `R` supplies declared requirements, `C` selects versions, `G` evaluates
/// conditions. Construct with [`Engine::new`] (uses [`AlwaysActive`]) or
/// [`Engine::with_gate`].
pub struct Engine<R, C, G = AlwaysActive> {
    requirements: R,
    candidates: C,
    gate: G,
    policy: ResolvePolicy,
    budget: ResolveBudget,
    environment: EnvironmentId,
}

/// The outcome of a resolution: the plain graph plus the run's chosen policy and
/// fidelity (so callers can record *how* the graph was produced).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The materialized occurrences and edges.
    pub graph: ResolvedGraph,
    /// The policy actually applied during the run.
    pub policy: ResolvePolicy,
    /// The final activation closure reached at the fixpoint.
    pub activation: ActivationContext,
    /// Number of BFS passes run before activation converged.
    pub iterations: u32,
}

impl<R, C> Engine<R, C, AlwaysActive>
where
    R: RequirementSource,
    C: CandidateSource,
{
    /// A new engine with an unconditional gate (no environment/activation
    /// condition filtering beyond the activation fixpoint itself).
    pub fn new(
        requirements: R,
        candidates: C,
        policy: ResolvePolicy,
        environment: EnvironmentId,
    ) -> Self {
        Self::with_gate(
            requirements,
            candidates,
            AlwaysActive,
            policy,
            environment,
            ResolveBudget::default(),
        )
    }
}

impl<R, C, G> Engine<R, C, G>
where
    R: RequirementSource,
    C: CandidateSource,
    G: ConditionGate,
{
    /// A new engine with an explicit condition gate and budget.
    pub fn with_gate(
        requirements: R,
        candidates: C,
        gate: G,
        policy: ResolvePolicy,
        environment: EnvironmentId,
        budget: ResolveBudget,
    ) -> Self {
        Self {
            requirements,
            candidates,
            gate,
            policy,
            budget,
            environment,
        }
    }

    /// Override the expansion budget (builder style).
    pub fn with_budget(mut self, budget: ResolveBudget) -> Self {
        self.budget = budget;
        self
    }

    /// Resolve from `roots` (concrete root [`VersionId`]s) to a fixpoint.
    ///
    /// Runs repeated BFS passes; each pass materializes a fresh graph against the
    /// current activation closure and may grow it. When the closure stops growing
    /// (or a budget caps the loop) the last pass's graph is returned.
    pub fn resolve(&self, roots: &[VersionId]) -> Result<Resolution, ResolveError> {
        let mut activation = ActivationContext::new();
        let mut last = self.materialize(roots, &activation)?;
        let mut iterations = 1u32;

        loop {
            // Grow the activation closure from the pass we just ran.
            let grew = self.grow_activation(&last.next_activation, &mut activation);
            let cap_hit = iterations >= self.budget.max_iterations;
            if !grew {
                break;
            }
            if cap_hit {
                last.graph.truncated = true;
                add_caveat(&mut last.graph, FidelityCaveats::BUDGET_TRUNCATED);
                break;
            }
            last = self.materialize(roots, &activation)?;
            iterations += 1;
        }

        Ok(Resolution {
            graph: last.graph,
            policy: self.policy.clone(),
            activation,
            iterations,
        })
    }

    /// Fold the activations discovered in a pass into the persistent closure,
    /// reporting whether anything was newly activated (fixpoint not yet reached).
    fn grow_activation(
        &self,
        discovered: &ActivationContext,
        into: &mut ActivationContext,
    ) -> bool {
        let mut grew = false;
        for (pkg, name) in discovered.iter() {
            if into.activate(pkg, name) {
                grew = true;
            }
        }
        grew
    }

    /// One BFS materialization pass against a fixed activation closure.
    fn materialize(
        &self,
        roots: &[VersionId],
        activation: &ActivationContext,
    ) -> Result<Pass, ResolveError> {
        let mut g = ResolvedGraph::new();
        g.fidelity = Some(GraphFidelity::exact(FidelityTier::ModelResolved));
        // Activations discovered during this pass (fed back into the fixpoint).
        let mut next_activation = ActivationContext::new();
        // Per-node owning package family (parallel to g.nodes).
        let mut node_pkg: Vec<prism_model::PackageId> = Vec::new();
        // Queue carries (node local_id, root->node family path).
        let mut queue: VecDeque<(u32, Vec<prism_model::PackageId>)> = VecDeque::new();

        // Seed roots.
        for &root in roots {
            if g.node_count() as u32 >= self.budget.max_nodes {
                self.mark_truncated(&mut g);
                break;
            }
            let pkg = self.requirements.package_of(root)?;
            let local = g.add_node(root, 0, "", NodeFlags::ROOT);
            node_pkg.push(pkg);
            self.record_default_activations(root, pkg, &mut next_activation)?;
            queue.push_back((local, vec![pkg]));
        }

        while let Some((src, path)) = queue.pop_front() {
            let src_depth = g.nodes[src as usize].depth;
            let src_version = g.nodes[src as usize].version_id;

            // Depth bound: keep the node, do not expand past it.
            if src_depth >= self.budget.max_depth {
                self.mark_truncated(&mut g);
                g.nodes[src as usize].flags.insert(NodeFlags::TRUNCATED);
                continue;
            }

            let reqs = self.requirements.requirements(src_version)?;
            for (ordinal, req) in reqs.into_iter().enumerate() {
                self.expand_requirement(
                    &mut g,
                    &mut node_pkg,
                    &mut queue,
                    &mut next_activation,
                    activation,
                    src,
                    src_depth,
                    &path,
                    ordinal as u32,
                    &req,
                )?;
            }
        }

        Ok(Pass {
            graph: g,
            next_activation,
        })
    }

    /// Gate, select, and (if taken) emit one edge for `req` out of node `src`.
    #[allow(clippy::too_many_arguments)]
    fn expand_requirement(
        &self,
        g: &mut ResolvedGraph,
        node_pkg: &mut Vec<prism_model::PackageId>,
        queue: &mut VecDeque<(u32, Vec<prism_model::PackageId>)>,
        next_activation: &mut ActivationContext,
        activation: &ActivationContext,
        src: u32,
        src_depth: u32,
        path: &[prism_model::PackageId],
        ordinal: u32,
        req: &DeclaredRequirement,
    ) -> Result<(), ResolveError> {
        let src_pkg = node_pkg[src as usize];

        // Split-gate the condition: environment vs activation vs unconditional.
        if !self.gate_active(req, src_pkg, activation)? {
            return Ok(());
        }

        // Phase-2 selection under the run policy.
        let selected = self.candidates.select(
            req.target,
            &req.matcher,
            self.environment,
            activation,
            &self.policy,
        )?;
        let Some(selected) = selected else {
            // Source deliberately wants no candidate (gated off downstream).
            return Ok(());
        };

        // Edge is taken: record the activations it triggers on its target.
        for name in &req.activates {
            next_activation.activate(req.target, name.clone());
        }

        // Cycle guard on the *package family*: if the target family is already on
        // the current root->src path, close a BACK_EDGE to that occurrence.
        if path.contains(&req.target) {
            if let Some(existing) = find_on_path(g, node_pkg, path, req.target) {
                g.add_edge(ResolvedEdge {
                    from: src,
                    to: existing,
                    ordinal,
                    requirement_id: Some(req.requirement_id),
                    kind: req.kind,
                    flags: EdgeFlags::RESOLVED | EdgeFlags::BACK_EDGE,
                });
            }
            return Ok(());
        }

        // Reuse an existing sibling occurrence of the same family if present
        // (shared diamond target → one node), else mint a new one.
        let dst = match find_existing(g, node_pkg, req.target, selected) {
            Some(existing) => {
                // Keep the minimum BFS depth.
                let d = src_depth + 1;
                if d < g.nodes[existing as usize].depth {
                    g.nodes[existing as usize].depth = d;
                }
                existing
            }
            None => {
                if g.node_count() as u32 >= self.budget.max_nodes {
                    self.mark_truncated(g);
                    g.nodes[src as usize].flags.insert(NodeFlags::TRUNCATED);
                    return Ok(());
                }
                let key = occurrence_key(req.target, selected);
                let local = g.add_node(selected, src_depth + 1, key, NodeFlags::EMPTY);
                node_pkg.push(req.target);
                self.record_default_activations(selected, req.target, next_activation)?;
                let mut child_path = path.to_vec();
                child_path.push(req.target);
                queue.push_back((local, child_path));
                local
            }
        };

        g.add_edge(ResolvedEdge {
            from: src,
            to: dst,
            ordinal,
            requirement_id: Some(req.requirement_id),
            kind: req.kind,
            flags: EdgeFlags::RESOLVED,
        });
        Ok(())
    }

    /// Evaluate a requirement's split gate against the right state.
    fn gate_active(
        &self,
        req: &DeclaredRequirement,
        src_pkg: prism_model::PackageId,
        activation: &ActivationContext,
    ) -> Result<bool, ResolveError> {
        let cond = req.condition.as_deref();
        let input = match req.condition_kind {
            prism_model::ConditionKind::None => GateInput::Unconditional,
            prism_model::ConditionKind::Environment => GateInput::Environment {
                environment: self.environment,
            },
            prism_model::ConditionKind::Activation => GateInput::Activation {
                anchor: src_pkg,
                context: activation,
            },
        };
        self.gate.evaluate(cond, input)
    }

    /// Record a version's unconditional self-activations into the closure.
    fn record_default_activations(
        &self,
        version: VersionId,
        pkg: prism_model::PackageId,
        into: &mut ActivationContext,
    ) -> Result<(), ResolveError> {
        for name in self.requirements.default_activations(version)? {
            into.activate(pkg, name);
        }
        Ok(())
    }

    /// Flag a budget truncation on the graph and its fidelity.
    fn mark_truncated(&self, g: &mut ResolvedGraph) {
        g.truncated = true;
        add_caveat(g, FidelityCaveats::BUDGET_TRUNCATED);
    }
}

/// One BFS pass result: the graph plus activations discovered in it.
struct Pass {
    graph: ResolvedGraph,
    next_activation: ActivationContext,
}

/// Add a fidelity caveat, downgrading the run's reported fidelity.
fn add_caveat(g: &mut ResolvedGraph, caveat: FidelityCaveats) {
    let base = g
        .fidelity
        .unwrap_or_else(|| GraphFidelity::exact(FidelityTier::ModelResolved));
    let mut caveats = base.caveats;
    caveats.insert(caveat);
    g.fidelity = Some(GraphFidelity {
        tier: base.tier,
        caveats,
    });
}

/// The occurrence key for a freshly minted node. Family + version is sufficient
/// for the synthetic engine's identity (one node per family+version); richer
/// ecosystem occurrence identity is layered on in `prism-api`.
fn occurrence_key(pkg: prism_model::PackageId, version: VersionId) -> String {
    format!("{}@{}", pkg.get(), version.get())
}

/// Find an existing occurrence of `(family, version)` to share (diamonds).
fn find_existing(
    g: &ResolvedGraph,
    node_pkg: &[prism_model::PackageId],
    family: prism_model::PackageId,
    version: VersionId,
) -> Option<u32> {
    g.nodes.iter().enumerate().find_map(|(i, n)| {
        if node_pkg[i] == family && n.version_id == version {
            Some(n.local_id)
        } else {
            None
        }
    })
}

/// Find the occurrence of `family` lying on the current root→src path (for the
/// cycle guard's back-edge target).
fn find_on_path(
    g: &ResolvedGraph,
    node_pkg: &[prism_model::PackageId],
    path: &[prism_model::PackageId],
    family: prism_model::PackageId,
) -> Option<u32> {
    if !path.contains(&family) {
        return None;
    }
    // Prefer the closest ancestor occurrence of the family.
    g.nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| node_pkg[*i] == family)
        .map(|(_, n)| n.local_id)
        .next_back()
}
