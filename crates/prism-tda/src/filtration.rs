//! Filtration scalars and convenience constructors (architecture doc,
//! *Supported Filtrations in Version 0.1*).
//!
//! A filtration supplies one scalar `f(v)` per graph node; the
//! [`crate::complex::Complex`] derives edge values via the lower-star / max rule.
//! Every v0.1 filtration is **sublevel / increasing** (direction `0`).

use prism_graph::GraphSnapshot;
use prism_model::Result;

/// The kind of scalar a filtration filters on. The discriminant is stable and
/// persisted in the `.ptda` header (`filtration_type`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u16)]
pub enum FiltrationType {
    /// `f(node) = node.depth`. Structure near the roots appears first.
    Depth = 0,
    /// `f(node) = age/staleness in days` (caller-supplied scalar).
    Staleness = 1,
    /// A caller-defined scalar with no fixed semantics.
    Custom = 0xFFFF,
}

impl FiltrationType {
    /// Stable persisted discriminant.
    #[inline]
    pub const fn to_u16(self) -> u16 {
        self as u16
    }

    /// Inverse of [`FiltrationType::to_u16`]; unknown codes map to
    /// [`FiltrationType::Custom`] (forward-compatible).
    pub fn from_u16(v: u16) -> Self {
        match v {
            0 => FiltrationType::Depth,
            1 => FiltrationType::Staleness,
            _ => FiltrationType::Custom,
        }
    }
}

/// Filtration direction. v0.1 only emits sublevel (increasing).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Direction {
    /// Sublevel set / increasing filtration value.
    Sublevel = 0,
}

impl Direction {
    /// Stable persisted discriminant.
    #[inline]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Inverse of [`Direction::to_u8`]; rejects unknown codes.
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(Direction::Sublevel),
            other => Err(prism_model::PrismError::Corrupt {
                detail: format!("unknown filtration direction {other}"),
            }),
        }
    }
}

/// A node-scalar filtration over one [`GraphSnapshot`].
///
/// `values[i]` is `f` for node `i`; the length always equals the snapshot's
/// `node_count()`. The complex derives edge (1-cell) values from these.
#[derive(Clone, Debug)]
pub struct Filtration {
    filtration_type: FiltrationType,
    direction: Direction,
    values: Vec<f64>,
}

impl Filtration {
    /// A filtration from explicit per-node scalars (sublevel/increasing).
    pub fn new(filtration_type: FiltrationType, values: Vec<f64>) -> Self {
        Self {
            filtration_type,
            direction: Direction::Sublevel,
            values,
        }
    }

    /// The filtration kind.
    #[inline]
    pub fn filtration_type(&self) -> FiltrationType {
        self.filtration_type
    }

    /// The filtration direction.
    #[inline]
    pub fn direction(&self) -> Direction {
        self.direction
    }

    /// The per-node scalar column.
    #[inline]
    pub fn values(&self) -> &[f64] {
        &self.values
    }
}

/// The depth filtration: `f(node) = node.depth` (architecture doc). Returns the
/// node-depth column lifted to `f64`.
pub fn depth_filtration(graph: &GraphSnapshot) -> Result<Filtration> {
    let n = graph.node_count();
    let mut values = Vec::with_capacity(n);
    for i in 0..n as u32 {
        let d = graph.node_depth(prism_model::GraphNodeId::new(i))?;
        values.push(d as f64);
    }
    Ok(Filtration::new(FiltrationType::Depth, values))
}
