//! The resolved feature/extra activation closure and the condition-gate split.
//!
//! A requirement's condition gates on one of two kinds of state
//! ([`prism_model::ConditionKind`]):
//!
//! - **Environment** — static target facts (platform, language version, markers).
//!   Evaluated against [`prism_model::Environment`] by a
//!   [`crate::ConditionEvaluator`] in `GateInput::Environment` mode.
//! - **Activation** — resolution-time feature/extra activation (Cargo features,
//!   PyPI extras). Evaluated against the [`ActivationContext`] closure, which the
//!   synthetic resolver grows to a fixpoint.
//!
//! This module models the closure and the input a gate is evaluated against, so
//! `cfg(...)`/marker logic and `feature ==`/`extra ==` logic route to the right
//! state without conflating the two.

use std::collections::BTreeSet;

use prism_model::{ConditionKind, EnvironmentId, PackageId};

/// The resolved set of activated features/extras across a resolution.
///
/// Cargo *features* and PyPI *extras* are both **activation** state: enabling
/// one can pull in otherwise-absent dependencies. The set is keyed by the
/// owning [`PackageId`] so the same feature name on two packages is distinct,
/// and stored sorted for deterministic hashing/equality.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActivationContext {
    /// Activated `(package, feature-or-extra-name)` pairs, sorted/deduped.
    activated: BTreeSet<(PackageId, String)>,
}

impl ActivationContext {
    /// An empty activation closure.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Activate `name` on `package`. Returns `true` if newly activated (i.e. the
    /// fixpoint has not yet converged on this pair).
    pub fn activate(&mut self, package: PackageId, name: impl Into<String>) -> bool {
        self.activated.insert((package, name.into()))
    }

    /// True if `name` is activated on `package`.
    pub fn is_active(&self, package: PackageId, name: &str) -> bool {
        // BTreeSet has no borrow-friendly tuple lookup for (PackageId, &str);
        // a bounded range scan over this package's entries is exact and cheap.
        self.activated
            .range((package, String::new())..)
            .take_while(|(p, _)| *p == package)
            .any(|(_, n)| n == name)
    }

    /// Number of activated pairs.
    #[inline]
    pub fn len(&self) -> usize {
        self.activated.len()
    }

    /// True if nothing is activated.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.activated.is_empty()
    }

    /// Iterate the activated pairs in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = (PackageId, &str)> + '_ {
        self.activated.iter().map(|(p, n)| (*p, n.as_str()))
    }

    /// All features/extras activated on a single package, in sorted order.
    pub fn activated_on(&self, package: PackageId) -> impl Iterator<Item = &str> + '_ {
        self.activated
            .range((package, String::new())..)
            .take_while(move |(p, _)| *p == package)
            .map(|(_, n)| n.as_str())
    }
}

/// The state a condition gate is evaluated against, split to match
/// [`prism_model::ConditionKind`]. A [`crate::ConditionEvaluator`] consumes a
/// borrowed `GateInput` so environment and activation logic never mix.
#[derive(Clone, Copy, Debug)]
pub enum GateInput<'a> {
    /// No condition — the requirement is unconditionally active.
    Unconditional,
    /// Evaluate against the static environment.
    Environment {
        /// The pinned environment id.
        environment: EnvironmentId,
    },
    /// Evaluate against the resolved activation closure.
    Activation {
        /// The package the condition is anchored to (for feature/extra scope).
        anchor: PackageId,
        /// The activation closure.
        context: &'a ActivationContext,
    },
}

impl GateInput<'_> {
    /// The [`ConditionKind`] this gate input corresponds to. A
    /// [`crate::ConditionEvaluator`] can assert the requirement's declared
    /// `condition_kind` matches the input it was handed.
    pub fn kind(&self) -> ConditionKind {
        match self {
            GateInput::Unconditional => ConditionKind::None,
            GateInput::Environment { .. } => ConditionKind::Environment,
            GateInput::Activation { .. } => ConditionKind::Activation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_dedup_and_lookup() {
        let mut ctx = ActivationContext::new();
        let serde = PackageId::new(1);
        let other = PackageId::new(2);
        assert!(ctx.activate(serde, "derive"));
        assert!(!ctx.activate(serde, "derive")); // already active → fixpoint
        assert!(ctx.activate(serde, "alloc"));
        assert!(ctx.activate(other, "derive"));

        assert!(ctx.is_active(serde, "derive"));
        assert!(ctx.is_active(serde, "alloc"));
        assert!(!ctx.is_active(serde, "std"));
        // Same feature name on a different package is distinct, but present.
        assert!(ctx.is_active(other, "derive"));
        assert!(!ctx.is_active(other, "alloc"));
        assert_eq!(ctx.len(), 3);
        assert!(!ctx.is_empty());
    }

    #[test]
    fn activated_on_is_package_scoped_and_sorted() {
        let mut ctx = ActivationContext::new();
        let a = PackageId::new(1);
        let b = PackageId::new(2);
        ctx.activate(a, "z");
        ctx.activate(a, "a");
        ctx.activate(b, "m");
        let on_a: Vec<&str> = ctx.activated_on(a).collect();
        assert_eq!(on_a, vec!["a", "z"]); // sorted, package-scoped
        let on_b: Vec<&str> = ctx.activated_on(b).collect();
        assert_eq!(on_b, vec!["m"]);
    }

    #[test]
    fn gate_input_kind_matches_condition_kind() {
        let ctx = ActivationContext::new();
        assert_eq!(GateInput::Unconditional.kind(), ConditionKind::None);
        assert_eq!(
            GateInput::Environment {
                environment: EnvironmentId::new(1)
            }
            .kind(),
            ConditionKind::Environment
        );
        assert_eq!(
            GateInput::Activation {
                anchor: PackageId::new(1),
                context: &ctx,
            }
            .kind(),
            ConditionKind::Activation
        );
    }
}
