//! Structural risk scoring and fragmentation analysis (architecture doc,
//! *Structural Risk* and *Fragmentation*).
//!
//! [`Prism::structural_risk`] distills the P0 structural signals into seven
//! comparable 0–100 dimensions plus an overall band, so a run's shape can be
//! summarized at a glance and tracked across runs. [`Prism::fragmentation`]
//! decomposes the graph into weakly-connected components and separates the
//! harmless package-manager artifacts (isolated peer/optional placeholders) from
//! genuinely detached subsystems.
//!
//! Scores are deliberately heuristic and bounded; they are meaningful **relative
//! to other runs of a similar ecosystem**, not as absolute truths. Every
//! dimension also carries the raw numbers it was derived from.

use std::collections::BTreeSet;

use prism_graph::{
    classify_cycles, dominators, family_clusters, weak_components, GraphSnapshot,
};
use prism_model::{GraphNodeId, NodeFlags, Result, RunId};

use crate::Prism;

/// One scored risk dimension (0 = benign, 100 = severe) with its raw basis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiskDimension {
    /// Dimension name (`sprawl`, `concentration`, …).
    pub name: &'static str,
    /// Bounded 0–100 score; higher is worse.
    pub score: u32,
    /// Human-readable basis (the raw numbers behind the score).
    pub detail: String,
}

/// The structural risk profile of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiskProfileView {
    /// The run scored.
    pub run: RunId,
    /// Overall score (mean of the dimensions), 0–100.
    pub overall: u32,
    /// Band label for `overall` (`low`/`moderate`/`elevated`/`high`).
    pub band: &'static str,
    /// The seven scored dimensions.
    pub dimensions: Vec<RiskDimension>,
}

/// Map a 0–100 score to a coarse band.
fn band(score: u32) -> &'static str {
    match score {
        0..=24 => "low",
        25..=49 => "moderate",
        50..=74 => "elevated",
        _ => "high",
    }
}

/// `round(100 * num / den)`, saturating, with a zero-denominator guard.
fn pct(num: u64, den: u64) -> u32 {
    if den == 0 {
        return 0;
    }
    let v = (num.saturating_mul(100) + den / 2) / den;
    v.min(100) as u32
}

/// A detached weakly-connected component (an "island").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IslandView {
    /// Member purls (capped for display).
    pub members: Vec<String>,
    /// Total member count (may exceed `members.len()` if capped).
    pub size: usize,
    /// True if every member is reachable from no root (a true detached subsystem
    /// rather than part of the main graph).
    pub detached_from_roots: bool,
}

/// Fragmentation decomposition of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentationView {
    /// The run analyzed.
    pub run: RunId,
    /// Number of weakly-connected components.
    pub component_count: usize,
    /// Size of the largest (the main graph containing the roots, normally).
    pub largest_component: usize,
    /// Isolated single-node islands — typically npm peer/optional placeholders
    /// the resolver lists but wires no edge to (harmless artifacts).
    pub artifact_singletons: usize,
    /// Multi-node detached subsystems worth attention, largest first.
    pub subsystem_islands: Vec<IslandView>,
    /// One-line interpretation.
    pub note: String,
}

impl Prism {
    /// Score the structural risk of `run` across seven dimensions.
    pub fn structural_risk(&self, run: RunId) -> Result<RiskProfileView> {
        let g = self.load_graph(run)?;
        let n = g.node_count();

        // --- Raw structural facts (a few passes over the snapshot). ---
        let mut max_depth = 0u32;
        let mut versions: BTreeSet<u64> = BTreeSet::new();
        let mut packages: BTreeSet<u64> = BTreeSet::new();
        for i in 0..n as u32 {
            let node = GraphNodeId::new(i);
            max_depth = max_depth.max(g.node_depth(node)?);
            let v = g.node_version(node)?;
            versions.insert(v.get());
            if let Some(p) = self.package_of(v) {
                packages.insert(p.get());
            }
        }

        let dt = dominators(&g)?;
        let chokepoints = dt.chokepoints(&g, 1)?;
        let (top_dom, top_purl) = match chokepoints.first() {
            Some(c) => (c.dominated, self.version_label(g.node_version(c.node)?)),
            None => (0, "—".to_string()),
        };

        let comps = weak_components(&g)?;
        let largest = comps.first().map(|c| c.len()).unwrap_or(0);
        let detached = n.saturating_sub(largest);

        let fragmented = family_clusters(&g, |v| self.package_of(v))?
            .into_iter()
            .filter(|c| c.is_version_fragmented())
            .count();

        let cycles = classify_cycles(&g)?;
        let runtime_cycle_blast: u32 = cycles
            .iter()
            .filter(|c| c.runtime_relevant)
            .map(|c| c.external_dependents)
            .sum();
        let runtime_cycle_count = cycles.iter().filter(|c| c.runtime_relevant).count();

        // --- Bounded 0–100 dimension scores (higher = worse). ---
        let sprawl = pct(n as u64, 2000);
        let depth = pct(max_depth as u64, 15);
        let concentration = pct(top_dom as u64, n.max(1) as u64);
        // Reuse: extra occurrences beyond one-per-version (hoisting duplication).
        let dup_occurrences = (n as u64).saturating_sub(versions.len() as u64);
        let reuse = pct(dup_occurrences, n.max(1) as u64);
        let fragmentation = pct(detached as u64, n.max(1) as u64);
        let cycle_risk = pct(runtime_cycle_blast as u64, n.max(1) as u64);
        let duplicate_pressure = pct(fragmented as u64, packages.len().max(1) as u64);

        let dimensions = vec![
            RiskDimension {
                name: "sprawl",
                score: sprawl,
                detail: format!("{n} occurrences, {} packages", packages.len()),
            },
            RiskDimension {
                name: "concentration",
                score: concentration,
                detail: format!("top gateway {top_purl} dominates {top_dom}"),
            },
            RiskDimension {
                name: "depth",
                score: depth,
                detail: format!("max dependency depth {max_depth}"),
            },
            RiskDimension {
                name: "reuse",
                score: reuse,
                detail: format!(
                    "{dup_occurrences} duplicate occurrences over {} distinct versions",
                    versions.len()
                ),
            },
            RiskDimension {
                name: "fragmentation",
                score: fragmentation,
                detail: format!("{} components, {detached} nodes off the main graph", comps.len()),
            },
            RiskDimension {
                name: "cycle-risk",
                score: cycle_risk,
                detail: format!(
                    "{runtime_cycle_count} runtime cycle(s), {runtime_cycle_blast} exposed dependents"
                ),
            },
            RiskDimension {
                name: "duplicate-pressure",
                score: duplicate_pressure,
                detail: format!("{fragmented} version-fragmented families"),
            },
        ];

        let overall = {
            let sum: u32 = dimensions.iter().map(|d| d.score).sum();
            (sum + dimensions.len() as u32 / 2) / dimensions.len() as u32
        };

        Ok(RiskProfileView {
            run,
            overall,
            band: band(overall),
            dimensions,
        })
    }

    /// Decompose `run` into weakly-connected components and classify the islands.
    pub fn fragmentation(&self, run: RunId) -> Result<FragmentationView> {
        let g = self.load_graph(run)?;
        let comps = weak_components(&g)?;
        let largest = comps.first().map(|c| c.len()).unwrap_or(0);

        let mut artifact_singletons = 0usize;
        let mut subsystem_islands = Vec::new();
        // Skip the first (largest) component — that is the main graph.
        for comp in comps.iter().skip(1) {
            if comp.len() == 1 {
                artifact_singletons += 1;
                continue;
            }
            let detached_from_roots = !self.component_touches_root(&g, comp)?;
            let members: Vec<String> = comp
                .iter()
                .take(12)
                .map(|&nd| {
                    self.version_label(g.node_version(nd).unwrap_or_else(|_| {
                        prism_model::VersionId::new(0)
                    }))
                })
                .collect();
            subsystem_islands.push(IslandView {
                members,
                size: comp.len(),
                detached_from_roots,
            });
        }
        subsystem_islands.sort_by(|a, b| b.size.cmp(&a.size));

        let note = format!(
            "{} component(s): 1 main graph of {largest}, {} detached subsystem(s), {artifact_singletons} isolated peer/optional artifact(s)",
            comps.len(),
            subsystem_islands.len()
        );

        Ok(FragmentationView {
            run,
            component_count: comps.len(),
            largest_component: largest,
            artifact_singletons,
            subsystem_islands,
            note,
        })
    }

    /// Whether any member of `comp` is a root occurrence.
    fn component_touches_root(&self, g: &GraphSnapshot, comp: &[GraphNodeId]) -> Result<bool> {
        for &nd in comp {
            if g.node_flags(nd)?.contains(NodeFlags::ROOT) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pct_rounds_and_saturates() {
        assert_eq!(pct(0, 0), 0);
        assert_eq!(pct(1, 4), 25);
        assert_eq!(pct(1, 3), 33);
        assert_eq!(pct(5, 4), 100); // saturates
        assert_eq!(pct(0, 10), 0);
    }

    #[test]
    fn band_thresholds() {
        assert_eq!(band(0), "low");
        assert_eq!(band(24), "low");
        assert_eq!(band(25), "moderate");
        assert_eq!(band(50), "elevated");
        assert_eq!(band(75), "high");
        assert_eq!(band(100), "high");
    }
}
