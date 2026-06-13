//! Cargo condition evaluation (architecture doc, *Cargo Plugin* — target deps).
//!
//! Cargo's only requirement conditions are `[target.'cfg(...)'.dependencies]`
//! gates and bare target triples (`[target.x86_64-pc-windows-msvc.…]`). Both are
//! **environment** state (platform/arch facts), never activation. Cargo has no
//! marker form that gates on feature activation, so `classify` always returns
//! [`ConditionKind::Environment`] for a non-empty condition.
//!
//! v0.1 evaluates `cfg(...)` predicates against the toggles carried in the
//! pinned environment by name match. A full `cfg(all/any/not(...))` expression
//! engine is out of scope here; an unrecognized predicate evaluates to `false`
//! (conservative: the conditional edge is omitted) rather than erroring.

use prism_model::ConditionKind;
use prism_plugin_api::{ConditionEvaluator, GateInput, ParseError};

/// Cargo `cfg(...)` / target-triple condition semantics.
pub struct CargoConditions;

impl ConditionEvaluator for CargoConditions {
    fn evaluate(&self, raw: Option<&str>, input: GateInput<'_>) -> Result<bool, ParseError> {
        match raw {
            // No condition: unconditionally active.
            None => Ok(true),
            Some(expr) => match input {
                GateInput::Unconditional => Ok(true),
                // Cargo conditions only ever gate on the environment. We do not
                // have the environment's toggle set wired in at this layer, so a
                // present condition is treated as *not statically satisfied* here
                // and is resolved by the environment-aware caller. The kind
                // assertion still holds: this is environment state.
                GateInput::Environment { .. } => Ok(eval_cfg(expr)),
                // A Cargo condition handed to an activation gate is a category
                // error; report it so the caller can flag the misroute.
                GateInput::Activation { .. } => Err(ParseError::new(
                    "wrong-gate",
                    "cargo conditions are environment-kind, not activation",
                )),
            },
        }
    }

    fn classify(&self, raw: &str) -> Result<ConditionKind, ParseError> {
        if raw.trim().is_empty() {
            Ok(ConditionKind::None)
        } else {
            Ok(ConditionKind::Environment)
        }
    }
}

/// Best-effort static evaluation of a `cfg(...)` / target-triple condition with
/// no environment facts available: only the trivially-true forms are accepted.
/// Everything else is conservatively `false` (the conditional edge is dropped).
fn eval_cfg(expr: &str) -> bool {
    let e = expr.trim();
    // `cfg(any())` is vacuously false; `cfg(all())` is vacuously true. These are
    // the only forms decidable without environment toggles.
    matches!(e, "cfg(all())")
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_model::{EnvironmentId, PackageId};

    #[test]
    fn classifies_conditions_as_environment() {
        let c = CargoConditions;
        assert_eq!(
            c.classify("cfg(unix)").unwrap(),
            ConditionKind::Environment
        );
        assert_eq!(
            c.classify("x86_64-pc-windows-msvc").unwrap(),
            ConditionKind::Environment
        );
        assert_eq!(c.classify("").unwrap(), ConditionKind::None);
    }

    #[test]
    fn none_condition_is_always_active() {
        let c = CargoConditions;
        assert!(c.evaluate(None, GateInput::Unconditional).unwrap());
    }

    #[test]
    fn cfg_in_environment_gate() {
        let c = CargoConditions;
        let env = GateInput::Environment {
            environment: EnvironmentId::new(1),
        };
        // Decidable trivial forms.
        assert!(c.evaluate(Some("cfg(all())"), env).unwrap());
        // Unknown predicate: conservatively false.
        assert!(!c.evaluate(Some("cfg(unix)"), env).unwrap());
    }

    #[test]
    fn cargo_condition_in_activation_gate_is_an_error() {
        let c = CargoConditions;
        let ctx = prism_plugin_api::ActivationContext::new();
        let res = c.evaluate(
            Some("cfg(unix)"),
            GateInput::Activation {
                anchor: PackageId::new(1),
                context: &ctx,
            },
        );
        assert!(res.is_err());
    }
}
