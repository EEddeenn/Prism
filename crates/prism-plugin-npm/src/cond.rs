//! npm condition evaluation (architecture doc, *npm Plugin*).
//!
//! npm has no per-dependency environment markers the way PyPI does; instead a
//! package declares package-level `engines` (node/npm version ranges), `os`, and
//! `cpu` constraints. The [`crate::manifest`] parser captures these as
//! **environment** conditions on the package's requirements using a small,
//! self-describing syntax:
//!
//! ```text
//! os:linux            os:!win32          (a `!`-prefix is a negation/blocklist)
//! cpu:x64             cpu:!ia32
//! node:>=18           npm:>=9
//! ```
//!
//! Multiple clauses are joined with ` && `. All npm conditions are
//! [`ConditionKind::Environment`] — npm has no feature/extra activation gate.
//! Offline, when the environment carries no platform facts, an environment gate
//! is treated as *active* (we do not prune edges we cannot evaluate).

use prism_plugin_api::activation::GateInput;
use prism_plugin_api::{ConditionEvaluator, ParseError};
use prism_model::ConditionKind;

/// npm [`ConditionEvaluator`]. All npm conditions are environment-gated.
#[derive(Debug, Default, Clone, Copy)]
pub struct NpmConditions;

impl ConditionEvaluator for NpmConditions {
    fn evaluate(&self, raw: Option<&str>, input: GateInput<'_>) -> Result<bool, ParseError> {
        let expr = match raw {
            None => return Ok(true),
            Some(e) if e.trim().is_empty() => return Ok(true),
            Some(e) => e.trim(),
        };
        // npm conditions never gate on activation; reject a mismatched input.
        if let GateInput::Activation { .. } = input {
            return Err(ParseError::new(
                "wrong-gate",
                "npm conditions are environment-gated, not activation-gated",
            ));
        }
        // Validate the clause syntax so a malformed manifest condition is caught,
        // even though offline we cannot fully evaluate against real platform data.
        for clause in expr.split("&&") {
            validate_clause(clause.trim())?;
        }
        // Offline / environment-less: an environment gate we cannot disprove is
        // considered active (conservative — never silently drops edges).
        Ok(true)
    }

    fn classify(&self, raw: &str) -> Result<ConditionKind, ParseError> {
        // Every npm condition is environment-gated.
        for clause in raw.split("&&") {
            validate_clause(clause.trim())?;
        }
        Ok(ConditionKind::Environment)
    }
}

/// Accept the `os:`/`cpu:`/`node:`/`npm:` clause shapes the manifest emits.
fn validate_clause(clause: &str) -> Result<(), ParseError> {
    if clause.is_empty() {
        return Err(ParseError::new("empty-condition", "empty condition clause"));
    }
    let (key, value) = clause
        .split_once(':')
        .ok_or_else(|| ParseError::new("bad-condition", format!("missing `:` in `{clause}`")))?;
    let value = value.strip_prefix('!').unwrap_or(value);
    if value.is_empty() {
        return Err(ParseError::new(
            "bad-condition",
            format!("empty value in `{clause}`"),
        ));
    }
    match key {
        "os" | "cpu" | "node" | "npm" | "engine" => Ok(()),
        other => Err(ParseError::new(
            "unknown-condition",
            format!("unknown npm condition key `{other}`"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_model::EnvironmentId;

    #[test]
    fn no_condition_is_active() {
        let c = NpmConditions;
        assert!(c.evaluate(None, GateInput::Unconditional).unwrap());
        assert!(c.evaluate(Some("  "), GateInput::Unconditional).unwrap());
    }

    #[test]
    fn classifies_everything_as_environment() {
        let c = NpmConditions;
        assert_eq!(c.classify("os:linux").unwrap(), ConditionKind::Environment);
        assert_eq!(
            c.classify("cpu:x64 && os:!win32").unwrap(),
            ConditionKind::Environment
        );
        assert_eq!(c.classify("node:>=18").unwrap(), ConditionKind::Environment);
    }

    #[test]
    fn environment_gate_is_conservatively_active() {
        let c = NpmConditions;
        let env = GateInput::Environment {
            environment: EnvironmentId::new(1),
        };
        assert!(c.evaluate(Some("os:linux"), env).unwrap());
        assert!(c.evaluate(Some("cpu:!ia32 && os:darwin"), env).unwrap());
    }

    #[test]
    fn rejects_activation_gate() {
        let c = NpmConditions;
        let ctx = prism_plugin_api::activation::ActivationContext::new();
        let act = GateInput::Activation {
            anchor: prism_model::PackageId::new(1),
            context: &ctx,
        };
        assert!(c.evaluate(Some("os:linux"), act).is_err());
    }

    #[test]
    fn rejects_malformed_conditions() {
        let c = NpmConditions;
        assert!(c.classify("nonsense").is_err());
        assert!(c.classify("weird:x64").is_err());
        assert!(c.classify("os:").is_err());
        assert!(c.classify("os:!").is_err());
    }
}
