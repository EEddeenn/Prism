//! Pluggable risk overlays (architecture doc, *Risk Overlays*).
//!
//! Prism's core stays hermetic: it never fetches advisories, license data, or
//! registry metadata. Instead the caller supplies an **overlay file** — a JSON
//! list of risk facts keyed by purl — and [`Prism::risk_overlay`] joins it to the
//! resolved graph, weighting each finding by what actually matters: whether the
//! flagged package is **reachable through production scopes** and how large its
//! **blast radius** is. So a critical advisory on a dev-only leaf ranks below a
//! medium one on a production chokepoint.
//!
//! Overlay format (any field but `purl` is optional):
//! ```json
//! [
//!   {"purl":"pkg:npm/es-errors@1.3.0","category":"vuln","severity":"high","note":"CVE-…"},
//!   {"purl":"pkg:npm/left-pad","category":"maintainer","severity":"medium"}
//! ]
//! ```
//! A `purl` with a version (`…@1.3.0`) matches that exact occurrence; a bare base
//! purl (`pkg:npm/left-pad`) matches every version of the family. Recognized
//! categories: `vuln`, `license`, `maintainer`, `freshness`, `native`,
//! `install-script`, `provenance` (others are passed through verbatim).

use std::collections::BTreeMap;

use prism_graph::{reverse_reachable, TraversalOptions};
use prism_model::{GraphNodeId, PrismError, Result, RunId};
use prism_textfmt::parse_json;

use crate::Prism;

/// One joined risk finding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayFinding {
    /// The overlay purl key.
    pub purl: String,
    /// Risk category (`vuln`, `license`, …).
    pub category: String,
    /// Severity label (`low`/`medium`/`high`/`critical`/`unknown`).
    pub severity: String,
    /// Optional free-text note from the overlay.
    pub note: String,
    /// Occurrences in the run matched by the purl key.
    pub matched_occurrences: u32,
    /// True if any matched occurrence is reachable through production scopes.
    pub production_reachable: bool,
    /// Distinct dependents across all matched occurrences (blast radius).
    pub blast_radius: u32,
    /// Priority = severity-weight × (production ? 2 : 1) × (1 + blast radius).
    pub priority: u32,
}

/// The joined overlay report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayReportView {
    /// The run the overlay was joined against.
    pub run: RunId,
    /// Total overlay entries parsed.
    pub entries: usize,
    /// Entries that matched at least one occurrence in the run.
    pub matched: usize,
    /// Matched findings that are production-reachable.
    pub production_findings: usize,
    /// Findings ordered by descending priority.
    pub findings: Vec<OverlayFinding>,
}

/// Severity → weight (0 for unknown so unscored facts sort below scored ones).
fn severity_weight(sev: &str) -> u32 {
    match sev {
        "critical" => 8,
        "high" => 4,
        "medium" => 2,
        "low" => 1,
        _ => 1,
    }
}

impl Prism {
    /// Join `overlay_json` (the contents of an overlay file) to `run` and report
    /// production-weighted findings. Parsing errors surface as
    /// [`PrismError::Other`]; an unmatched entry is reported with zero matches.
    pub fn risk_overlay(&self, run: RunId, overlay_json: &str) -> Result<OverlayReportView> {
        let parsed = parse_json(overlay_json)
            .map_err(|e| PrismError::Other(format!("overlay is not valid JSON: {e}")))?;
        let entries = parsed
            .as_array()
            .ok_or_else(|| PrismError::Other("overlay must be a JSON array".into()))?;

        let g = self.load_graph(run)?;

        // Index occurrences by full version purl and by base purl (version-
        // stripped), each normalized so a `%2F`-encoded scoped purl in the store
        // matches a literal-slash purl in the overlay.
        let mut by_version: BTreeMap<String, Vec<GraphNodeId>> = BTreeMap::new();
        let mut by_base: BTreeMap<String, Vec<GraphNodeId>> = BTreeMap::new();
        for i in 0..g.node_count() as u32 {
            let node = GraphNodeId::new(i);
            let purl = normalize_purl(&self.version_label(g.node_version(node)?));
            let base = base_purl(&purl).to_string();
            by_base.entry(base).or_default().push(node);
            by_version.entry(purl).or_default().push(node);
        }

        let mut findings = Vec::new();
        for entry in entries {
            let Some(purl) = entry.get("purl").and_then(|v| v.as_str()) else {
                continue; // entries without a purl are ignored
            };
            let category = entry
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let severity = entry
                .get("severity")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let note = entry
                .get("note")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Match exact version first, else the whole family (encoding-tolerant).
            let key = normalize_purl(purl);
            let matched: &[GraphNodeId] = by_version
                .get(&key)
                .or_else(|| by_base.get(&key))
                .map(|v| v.as_slice())
                .unwrap_or(&[]);

            let (production_reachable, blast_radius) = self.reach_and_blast(&g, matched)?;
            let priority = severity_weight(&severity)
                .saturating_mul(if production_reachable { 2 } else { 1 })
                .saturating_mul(1u32.saturating_add(blast_radius));

            findings.push(OverlayFinding {
                purl: purl.to_string(),
                category,
                severity,
                note,
                matched_occurrences: matched.len() as u32,
                production_reachable,
                blast_radius,
                priority,
            });
        }

        findings.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.purl.cmp(&b.purl)));
        let matched = findings.iter().filter(|f| f.matched_occurrences > 0).count();
        let production_findings = findings.iter().filter(|f| f.production_reachable).count();

        Ok(OverlayReportView {
            run,
            entries: entries.len(),
            matched,
            production_findings,
            findings,
        })
    }

    /// Production reachability (any matched occurrence) and distinct blast radius
    /// across all matched occurrences.
    fn reach_and_blast(
        &self,
        g: &prism_graph::GraphSnapshot,
        matched: &[GraphNodeId],
    ) -> Result<(bool, u32)> {
        if matched.is_empty() {
            return Ok((false, 0));
        }
        let n = g.node_count();
        let mut production = false;
        let mut dependents = vec![false; n];
        for &node in matched {
            // Reachability uses the shared structural explanation.
            let ex = prism_graph::explain(g, node, 1)?;
            if ex.production_reachable {
                production = true;
            }
            // Union of reverse-reachable ancestors (blast radius).
            let anc = reverse_reachable(g, node, TraversalOptions::default())?;
            for a in anc.nodes {
                dependents[a.get() as usize] = true;
            }
        }
        // Exclude the matched occurrences themselves from the blast count.
        for &node in matched {
            dependents[node.get() as usize] = false;
        }
        let blast = dependents.iter().filter(|&&d| d).count() as u32;
        Ok((production, blast))
    }
}

/// Strip the `@version` from a purl, leaving the base family purl.
fn base_purl(purl: &str) -> &str {
    purl.rsplit_once('@').map(|(b, _)| b).unwrap_or(purl)
}

/// Decode the `%2F` a scoped-npm purl carries so store and overlay keys agree.
fn normalize_purl(purl: &str) -> String {
    purl.replace("%2F", "/").replace("%2f", "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_weights_rank() {
        assert!(severity_weight("critical") > severity_weight("high"));
        assert!(severity_weight("high") > severity_weight("medium"));
        assert_eq!(severity_weight("unknown"), 1);
    }

    #[test]
    fn base_purl_strips_version() {
        assert_eq!(base_purl("pkg:npm/foo@1.2.3"), "pkg:npm/foo");
        assert_eq!(base_purl("pkg:npm/foo"), "pkg:npm/foo");
    }
}
