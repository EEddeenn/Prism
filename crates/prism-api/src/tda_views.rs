//! Display-oriented view structs for the facade's TDA queries.

use prism_model::{FiltrationId, GraphEdgeId, GraphNodeId, PairId, RunId};

use crate::views::GraphNodeView;

/// Per-dimension barcode counts for one persistence diagram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarcodeDim {
    /// Homology dimension (0 = components, 1 = cycles in v0.1).
    pub dim: u8,
    /// Number of finite (born-and-killed) pairs in this dimension.
    pub finite: usize,
    /// Number of essential (never-dying) pairs in this dimension.
    pub essential: usize,
}

impl BarcodeDim {
    /// Total pairs in this dimension.
    pub fn total(&self) -> usize {
        self.finite + self.essential
    }
}

/// A barcode summary of a `.ptda` result: per-dimension birth/death counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BarcodeView {
    /// The run this filtration was computed over.
    pub run: RunId,
    /// The filtration id whose `.ptda` was persisted.
    pub filtration: FiltrationId,
    /// Per-dimension counts, ascending by dimension.
    pub dims: Vec<BarcodeDim>,
}

impl BarcodeView {
    /// The number of essential classes in dimension `dim` (0 if absent).
    pub fn essential(&self, dim: u8) -> usize {
        self.dims.iter().find(|d| d.dim == dim).map_or(0, |d| d.essential)
    }

    /// The number of connected components (essential H0 classes).
    pub fn component_count(&self) -> usize {
        self.essential(0)
    }

    /// The cycle rank (essential H1 classes).
    pub fn cycle_count(&self) -> usize {
        self.essential(1)
    }

    /// True if the barcode has at least one bar in any dimension.
    pub fn is_nonempty(&self) -> bool {
        self.dims.iter().any(|d| d.total() > 0)
    }
}

/// A graph object backing a persistence-pair generator (mapped for display).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SupportObject {
    /// A graph node, resolved to its display view.
    Node(GraphNodeView),
    /// A graph edge (snapshot-local id) with its endpoints.
    Edge {
        /// The dense edge id within the run's snapshot.
        id: GraphEdgeId,
        /// Source node id.
        source: GraphNodeId,
        /// Target node id.
        target: GraphNodeId,
    },
}

/// The generator support of one persistence pair, mapped to graph objects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupportView {
    /// The pair these generators belong to.
    pub pair: PairId,
    /// Homology dimension of the pair.
    pub dim: u8,
    /// The graph objects (nodes/edges) carrying the representative chain.
    pub objects: Vec<SupportObject>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn barcode_helpers() {
        let b = BarcodeView {
            run: RunId::new(1),
            filtration: FiltrationId::new(1),
            dims: vec![
                BarcodeDim {
                    dim: 0,
                    finite: 2,
                    essential: 1,
                },
                BarcodeDim {
                    dim: 1,
                    finite: 0,
                    essential: 2,
                },
            ],
        };
        assert_eq!(b.component_count(), 1);
        assert_eq!(b.cycle_count(), 2);
        assert_eq!(b.essential(2), 0);
        assert!(b.is_nonempty());
        assert_eq!(b.dims[0].total(), 3);
    }
}
