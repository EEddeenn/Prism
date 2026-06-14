//! Topology shape signatures, cross-run distance, and CI regression budgets
//! (architecture doc, *Topology Distance* and *Regression Budgets*).
//!
//! A [`ShapeSignature`] is the compact structural fingerprint of a run — the
//! handful of numbers that characterize how sprawling, concentrated, fragmented,
//! and cyclic its dependency graph is. [`Prism::topology_distance`] compares two
//! signatures dimension-by-dimension so an upgrade can be judged "made the graph
//! 30% more fragmented" rather than merely "changed". [`Prism::ci_budget`] turns
//! the same signals into a pass/fail gate for CI, optionally against a baseline
//! run, so duplicate pressure / cycle risk / H₁ complexity regressions can be
//! flagged before merge.

use prism_graph::{classify_cycles, dominators, family_clusters, scope_summary};
use prism_model::{DependencyKind, GraphNodeId, PrismError, Result, RunId};
use prism_textfmt::parse_json;

use crate::Prism;

/// The structural fingerprint of one run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeSignature {
    /// The run fingerprinted.
    pub run: RunId,
    /// Occurrence nodes.
    pub nodes: u32,
    /// Directed edges.
    pub edges: u32,
    /// Maximum dependency depth.
    pub max_depth: u32,
    /// Weakly-connected components.
    pub components: u32,
    /// Total dependency cycles.
    pub cycles: u32,
    /// Runtime-relevant cycles.
    pub runtime_cycles: u32,
    /// Version-fragmented families.
    pub duplicate_families: u32,
    /// Reconvergence (H₁ proxy = Σ max(0, parents−1)).
    pub reconvergence: u32,
    /// Largest chokepoint's dominated count (concentration).
    pub top_chokepoint: u32,
    /// Production (runtime) footprint reachable from the roots.
    pub runtime_footprint: u32,
}

impl ShapeSignature {
    /// The named (metric, value) pairs, in a stable order, for diffing.
    fn metrics(&self) -> [(&'static str, u32); 10] {
        [
            ("nodes", self.nodes),
            ("edges", self.edges),
            ("max_depth", self.max_depth),
            ("components", self.components),
            ("cycles", self.cycles),
            ("runtime_cycles", self.runtime_cycles),
            ("duplicate_families", self.duplicate_families),
            ("reconvergence", self.reconvergence),
            ("top_chokepoint", self.top_chokepoint),
            ("runtime_footprint", self.runtime_footprint),
        ]
    }
}

/// One dimension's movement between two signatures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricDelta {
    /// Metric name.
    pub metric: &'static str,
    /// Base value.
    pub base: u32,
    /// Head value.
    pub head: u32,
    /// Signed relative change in percent (`(head−base)/max(1,base)·100`).
    pub change_pct: i64,
}

/// The shape distance between two runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyDistanceView {
    /// Base run.
    pub base: RunId,
    /// Head run.
    pub head: RunId,
    /// Mean absolute relative change across dimensions (0 = identical shape).
    pub distance_pct: u32,
    /// Per-dimension movements, largest absolute change first.
    pub deltas: Vec<MetricDelta>,
}

/// A single budget violation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetViolation {
    /// The budget key that was exceeded.
    pub rule: String,
    /// The observed value.
    pub observed: i64,
    /// The configured limit.
    pub limit: i64,
}

/// The result of a CI budget check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetReport {
    /// The head run checked.
    pub head: RunId,
    /// The optional baseline run for delta rules.
    pub base: Option<RunId>,
    /// True if no budget was exceeded.
    pub pass: bool,
    /// The violations (empty when `pass`).
    pub violations: Vec<BudgetViolation>,
    /// Number of budget rules evaluated.
    pub rules_checked: usize,
}

impl Prism {
    /// Compute the structural shape signature of `run`.
    pub fn shape_signature(&self, run: RunId) -> Result<ShapeSignature> {
        let g = self.load_graph(run)?;
        let n = g.node_count();

        let mut max_depth = 0u32;
        let mut reconvergence: u64 = 0;
        for i in 0..n as u32 {
            let node = GraphNodeId::new(i);
            max_depth = max_depth.max(g.node_depth(node)?);
            let parents = g.in_neighbors(node)?;
            // distinct parents
            let mut last = u32::MAX;
            let mut distinct = 0u32;
            let mut sorted: Vec<u32> = parents.iter().map(|p| p.get()).collect();
            sorted.sort_unstable();
            for p in sorted {
                if p != last {
                    distinct += 1;
                    last = p;
                }
            }
            if distinct > 1 {
                reconvergence += (distinct - 1) as u64;
            }
        }

        let components = prism_graph::weak_components(&g)?.len() as u32;
        let cycles_all = classify_cycles(&g)?;
        let cycles = cycles_all.len() as u32;
        let runtime_cycles = cycles_all.iter().filter(|c| c.runtime_relevant).count() as u32;
        let duplicate_families = family_clusters(&g, |v| self.package_of(v))?
            .into_iter()
            .filter(|c| c.is_version_fragmented())
            .count() as u32;
        let top_chokepoint = dominators(&g)?
            .chokepoints(&g, 1)?
            .first()
            .map(|c| c.dominated)
            .unwrap_or(0);
        let runtime_footprint = scope_summary(&g)?
            .iter()
            .find(|fp| fp.kind == DependencyKind::Runtime)
            .map(|fp| fp.footprint.len() as u32)
            .unwrap_or(0);

        Ok(ShapeSignature {
            run,
            nodes: n as u32,
            edges: g.edge_count() as u32,
            max_depth,
            components,
            cycles,
            runtime_cycles,
            duplicate_families,
            reconvergence: reconvergence.min(u32::MAX as u64) as u32,
            top_chokepoint,
            runtime_footprint,
        })
    }

    /// The shape distance between two runs.
    pub fn topology_distance(&self, base: RunId, head: RunId) -> Result<TopologyDistanceView> {
        let a = self.shape_signature(base)?;
        let b = self.shape_signature(head)?;
        let am = a.metrics();
        let bm = b.metrics();

        let mut deltas = Vec::with_capacity(am.len());
        let mut abs_sum: i64 = 0;
        for ((name, av), (_, bv)) in am.iter().zip(bm.iter()) {
            let change_pct =
                ((*bv as i64 - *av as i64) * 100) / (*av).max(1) as i64;
            abs_sum += change_pct.abs();
            deltas.push(MetricDelta {
                metric: name,
                base: *av,
                head: *bv,
                change_pct,
            });
        }
        deltas.sort_by_key(|d| std::cmp::Reverse(d.change_pct.abs()));
        let distance_pct = (abs_sum / am.len() as i64).max(0) as u32;

        Ok(TopologyDistanceView {
            base,
            head,
            distance_pct,
            deltas,
        })
    }

    /// Evaluate a CI topology budget for `head`, optionally relative to `base`.
    ///
    /// `budget_json` is a JSON object of rules. Absolute caps on the head run:
    /// `max_nodes`, `max_duplicate_families`, `max_cycles`, `max_runtime_cycles`,
    /// `max_reconvergence` (H₁), `max_top_chokepoint`. Delta rules (require
    /// `base`): `max_new_cycles`, `max_duplicate_families_increase`,
    /// `max_reconvergence_increase`. Returns a [`BudgetReport`]; the CLI maps a
    /// failing report to a non-zero exit.
    pub fn ci_budget(
        &self,
        head: RunId,
        base: Option<RunId>,
        budget_json: &str,
    ) -> Result<BudgetReport> {
        let parsed = parse_json(budget_json)
            .map_err(|e| PrismError::Other(format!("budget is not valid JSON: {e}")))?;
        let obj = parsed
            .as_object()
            .ok_or_else(|| PrismError::Other("budget must be a JSON object".into()))?;

        let h = self.shape_signature(head)?;
        let b = match base {
            Some(r) => Some(self.shape_signature(r)?),
            None => None,
        };

        let mut violations = Vec::new();
        let mut rules_checked = 0usize;

        // Absolute caps on the head run.
        let abs_rules: [(&str, i64); 6] = [
            ("max_nodes", h.nodes as i64),
            ("max_duplicate_families", h.duplicate_families as i64),
            ("max_cycles", h.cycles as i64),
            ("max_runtime_cycles", h.runtime_cycles as i64),
            ("max_reconvergence", h.reconvergence as i64),
            ("max_top_chokepoint", h.top_chokepoint as i64),
        ];
        for (rule, observed) in abs_rules {
            if let Some(limit) = obj_i64(obj, rule) {
                rules_checked += 1;
                if observed > limit {
                    violations.push(BudgetViolation {
                        rule: rule.into(),
                        observed,
                        limit,
                    });
                }
            }
        }

        // Delta rules against the baseline.
        if let Some(b) = &b {
            let new_cycles = self.run_diff(b.run, head)?.new_cycles.len() as i64;
            let delta_rules: [(&str, i64); 3] = [
                ("max_new_cycles", new_cycles),
                (
                    "max_duplicate_families_increase",
                    h.duplicate_families as i64 - b.duplicate_families as i64,
                ),
                (
                    "max_reconvergence_increase",
                    h.reconvergence as i64 - b.reconvergence as i64,
                ),
            ];
            for (rule, observed) in delta_rules {
                if let Some(limit) = obj_i64(obj, rule) {
                    rules_checked += 1;
                    if observed > limit {
                        violations.push(BudgetViolation {
                            rule: rule.into(),
                            observed,
                            limit,
                        });
                    }
                }
            }
        }

        Ok(BudgetReport {
            head,
            base,
            pass: violations.is_empty(),
            violations,
            rules_checked,
        })
    }
}

/// Read an integer budget value (accepts JSON ints and integral floats).
fn obj_i64(obj: &[(String, prism_textfmt::Json)], key: &str) -> Option<i64> {
    obj.iter()
        .rev()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_textfmt::parse_json;

    #[test]
    fn obj_i64_reads_ints_and_floats() {
        let j = parse_json(r#"{"a": 5, "b": 3.0, "c": "x"}"#).unwrap();
        let o = j.as_object().unwrap();
        assert_eq!(obj_i64(o, "a"), Some(5));
        assert_eq!(obj_i64(o, "b"), Some(3));
        assert_eq!(obj_i64(o, "c"), None);
        assert_eq!(obj_i64(o, "missing"), None);
    }
}
