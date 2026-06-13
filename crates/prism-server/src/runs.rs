//! Enumerating the runs a workspace can serve, using only the `prism-api`
//! facade.
//!
//! `prism-api` doesn't expose a run listing, and this crate intentionally
//! depends on nothing below it. But every ingested run persists a graph file
//! named `graphs/run_<8 digits>.pcsr` (see `prism_store::Layout::graph_file_name`),
//! recorded in the published manifest. We recover the servable run ids by
//! scanning those file names — no extra dependency, no record decoding.

use prism_api::Prism;
use prism_model::RunId;

/// Prefix and suffix of a persisted graph file's manifest `rel_path`.
const GRAPH_PREFIX: &str = "graphs/run_";
const GRAPH_SUFFIX: &str = ".pcsr";

/// Every run id with a persisted graph snapshot, ascending. Derived from the
/// manifest's published file set, so it reflects exactly what the query
/// endpoints can answer.
pub fn list_run_ids(prism: &Prism) -> Vec<RunId> {
    let mut ids: Vec<u64> = prism
        .store()
        .manifest()
        .files
        .iter()
        .filter_map(|f| run_id_from_graph_path(&f.rel_path))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter().map(RunId::new).collect()
}

/// Parse a run id out of a `graphs/run_NNNNNNNN.pcsr` path, or `None` for any
/// other file. Tolerant of a longer numeric field than the canonical 8 digits.
fn run_id_from_graph_path(rel_path: &str) -> Option<u64> {
    let rest = rel_path.strip_prefix(GRAPH_PREFIX)?;
    let digits = rest.strip_suffix(GRAPH_SUFFIX)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_graph_path() {
        assert_eq!(run_id_from_graph_path("graphs/run_00000007.pcsr"), Some(7));
        assert_eq!(run_id_from_graph_path("graphs/run_00000123.pcsr"), Some(123));
    }

    #[test]
    fn rejects_non_graph_paths() {
        assert_eq!(run_id_from_graph_path("indexes/000001.pidx"), None);
        assert_eq!(run_id_from_graph_path("tda/filtration_00000001.ptda"), None);
        assert_eq!(run_id_from_graph_path("segments/000001.pseg"), None);
        assert_eq!(run_id_from_graph_path("graphs/run_.pcsr"), None);
        assert_eq!(run_id_from_graph_path("graphs/run_abc.pcsr"), None);
        assert_eq!(run_id_from_graph_path("graphs/run_7.txt"), None);
    }
}
