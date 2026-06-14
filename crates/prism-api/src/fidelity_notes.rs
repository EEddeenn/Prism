//! Per-metric fidelity notes (architecture doc, *Fidelity*).
//!
//! Every analysis Prism reports is only as faithful as the lockfile it ingested.
//! [`Prism::fidelity_notes`] surfaces, for a run, what is exact vs inferred:
//! the resolution tier, the recorded [`prism_model::FidelityCaveats`], and
//! ecosystem-specific caveats derived from what the graph actually contains
//! (e.g. an npm lockfile records no transitive dev edges, so dev-scope footprints
//! undercount). Each note is tagged with the analysis it qualifies.

use prism_graph::scope_summary;
use prism_model::{FidelityCaveats, FidelityTier, Result, RunId};

use crate::ecosystem::Ecosystem;
use crate::Prism;

/// One fidelity caveat, tagged with the analysis topic it qualifies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FidelityNote {
    /// Which analysis/metric this note applies to (`scope`, `duplicates`, …),
    /// or `all` for run-wide caveats.
    pub topic: &'static str,
    /// The caveat in plain language.
    pub note: String,
}

/// The fidelity report for a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FidelityReport {
    /// The run described.
    pub run: RunId,
    /// The resolution tier label.
    pub tier: &'static str,
    /// The ecosystem label.
    pub ecosystem: &'static str,
    /// Per-topic caveats, run-wide first.
    pub notes: Vec<FidelityNote>,
}

fn tier_label(t: FidelityTier) -> &'static str {
    match t {
        FidelityTier::LockfileExact => "lockfile-exact",
        FidelityTier::NativeToolResolved => "native-tool-resolved",
        FidelityTier::ModelResolved => "model-resolved",
        FidelityTier::CandidateSpaceOnly => "candidate-space-only",
    }
}

/// Map a recorded caveat flag to a (topic, sentence) note.
fn caveat_note(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "NO_PEER_PLACEMENT" => (
            "scope",
            "peer dependencies are not placed as graph edges — peer reachability is approximate",
        ),
        "NO_FEATURE_UNIFICATION" => (
            "scope",
            "features were not unified/re-expanded — feature-gated edges may be missing",
        ),
        "NO_PROFILE_EXPANSION" => (
            "scope",
            "build profiles were not expanded — build-only edges may be incomplete",
        ),
        "NO_MEDIATION" => (
            "duplicates",
            "version mediation was not applied — duplicate counts reflect the raw lockfile",
        ),
        "NO_EXTRA_ACTIVATION" => (
            "scope",
            "optional extras were not activated — optional reachability may undercount",
        ),
        "MARKERS_APPROXIMATED" => (
            "scope",
            "environment markers were approximated — platform-specific deps may be over- or under-included",
        ),
        "DYNAMIC_METADATA_SKIPPED" => (
            "all",
            "dynamic package metadata was skipped — some edges may be absent",
        ),
        "BUDGET_TRUNCATED" => (
            "all",
            "graph expansion was truncated by a budget — the tail of the graph is missing",
        ),
        _ => return None,
    })
}

impl Prism {
    /// The fidelity report for `run`: tier, recorded caveats, and
    /// ecosystem-derived analysis caveats.
    pub fn fidelity_notes(&self, run: RunId) -> Result<FidelityReport> {
        let record = self.store.get_run(run)?;
        let eco = crate::ecosystem::ecosystem_of(record.ecosystem);
        let mut notes = Vec::new();

        // Run-wide tier note.
        notes.push(FidelityNote {
            topic: "all",
            note: format!(
                "resolution tier: {} (graph read {})",
                tier_label(record.fidelity.tier),
                match record.fidelity.tier {
                    FidelityTier::LockfileExact => "verbatim from the lockfile",
                    FidelityTier::NativeToolResolved => "from a native resolver",
                    FidelityTier::ModelResolved => "from Prism's synthetic resolver",
                    FidelityTier::CandidateSpaceOnly => "as ranges only",
                }
            ),
        });

        // Recorded caveats.
        let caveats = record.fidelity.caveats;
        for (name, bits) in FidelityCaveats::NAMED {
            if caveats.bits() & bits != 0 {
                if let Some((topic, sentence)) = caveat_note(name) {
                    notes.push(FidelityNote {
                        topic,
                        note: sentence.to_string(),
                    });
                }
            }
        }

        // Ecosystem-specific analysis caveats.
        for (topic, note) in ecosystem_notes(eco) {
            notes.push(FidelityNote {
                topic,
                note: note.to_string(),
            });
        }

        // Scope-coverage note: which dependency kinds are actually represented.
        let g = self.load_graph(run)?;
        let kinds: Vec<&'static str> = scope_summary(&g)?
            .iter()
            .map(|fp| crate::structure_views::kind_label(fp.kind))
            .collect();
        notes.push(FidelityNote {
            topic: "scope",
            note: format!(
                "dependency kinds present in this graph: {}",
                if kinds.is_empty() { "(none)".to_string() } else { kinds.join(", ") }
            ),
        });

        Ok(FidelityReport {
            run,
            tier: tier_label(record.fidelity.tier),
            ecosystem: eco.label(),
            notes,
        })
    }
}

/// Static analysis caveats inherent to each ecosystem's lockfile format.
fn ecosystem_notes(eco: Ecosystem) -> &'static [(&'static str, &'static str)] {
    match eco {
        Ecosystem::Npm | Ecosystem::Bun | Ecosystem::Pnpm => &[
            (
                "scope",
                "npm lockfiles record dev/optional per package, not per edge — transitive dev/build scope footprints undercount",
            ),
            (
                "duplicates",
                "lockfile edges carry resolved versions, not requirement ranges — duplicate root-cause shows introducers without constraint text",
            ),
        ],
        Ecosystem::Uv => &[
            (
                "scope",
                "PEP 508 markers/extras drive optional reachability — platform/marker-gated deps are included as optional",
            ),
        ],
        Ecosystem::Cargo => &[
            (
                "scope",
                "Cargo unifies features at resolve time — the graph reflects the unified feature set, not per-edge feature gates",
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_labels_are_stable() {
        assert_eq!(tier_label(FidelityTier::LockfileExact), "lockfile-exact");
        assert_eq!(tier_label(FidelityTier::CandidateSpaceOnly), "candidate-space-only");
    }

    #[test]
    fn known_caveats_map_to_notes() {
        assert!(caveat_note("NO_PEER_PLACEMENT").is_some());
        assert!(caveat_note("MARKERS_APPROXIMATED").is_some());
        assert!(caveat_note("UNKNOWN_FLAG").is_none());
    }

    #[test]
    fn ecosystem_notes_present_for_each() {
        assert!(!ecosystem_notes(Ecosystem::Npm).is_empty());
        assert!(!ecosystem_notes(Ecosystem::Uv).is_empty());
        assert!(!ecosystem_notes(Ecosystem::Cargo).is_empty());
    }
}
