//! PEP 508 environment-marker conditions.
//!
//! A uv dependency may carry a marker such as `python_full_version < '3.11'`,
//! `sys_platform == 'win32'`, or `extra == 'cli'`. Platform / interpreter markers
//! gate on the static **environment**; an `extra == '…'` marker gates on the
//! resolution-time **activation** closure (the requested extras). So `classify`
//! returns [`ConditionKind::Activation`] for any marker mentioning `extra` and
//! [`ConditionKind::Environment`] otherwise.
//!
//! v0.1 does not have the environment toggles or activation closure wired in at
//! this layer, so `evaluate` is conservative: a present condition is treated as
//! not-statically-satisfied (the conditional edge is omitted) rather than guessed.

use prism_model::ConditionKind;
use prism_plugin_api::{ConditionEvaluator, GateInput, ParseError};

/// PEP 508 marker condition semantics.
pub struct UvConditions;

/// Classify a raw marker into the gate kind it gates on (shared with the lockfile
/// fact emitter): empty → none, mentions `extra` → activation, else environment.
pub(crate) fn classify_marker(marker: &str) -> ConditionKind {
    if marker.trim().is_empty() {
        ConditionKind::None
    } else if mentions_extra(marker) {
        ConditionKind::Activation
    } else {
        ConditionKind::Environment
    }
}

/// True if `marker` references the `extra` variable (an activation gate).
fn mentions_extra(marker: &str) -> bool {
    // A whole-word `extra` reference; substrings like `extras` don't count.
    let bytes = marker.as_bytes();
    let mut i = 0;
    while let Some(pos) = marker[i..].find("extra") {
        let start = i + pos;
        let end = start + "extra".len();
        let before_ok = start == 0 || !is_ident_byte(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_ident_byte(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        i = end;
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl ConditionEvaluator for UvConditions {
    fn evaluate(&self, raw: Option<&str>, input: GateInput<'_>) -> Result<bool, ParseError> {
        match raw {
            None => Ok(true),
            Some(expr) => match input {
                GateInput::Unconditional => Ok(true),
                // No environment toggles / activation closure are wired in here, so
                // a present marker is conservatively not statically satisfied; the
                // environment/activation-aware caller resolves it.
                GateInput::Environment { .. } => Ok(false),
                GateInput::Activation { .. } => {
                    let _ = expr;
                    Ok(false)
                }
            },
        }
    }

    fn classify(&self, raw: &str) -> Result<ConditionKind, ParseError> {
        Ok(classify_marker(raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_markers() {
        let c = UvConditions;
        assert_eq!(c.classify("").unwrap(), ConditionKind::None);
        assert_eq!(
            c.classify("python_full_version < '3.11'").unwrap(),
            ConditionKind::Environment
        );
        assert_eq!(
            c.classify("sys_platform == 'win32'").unwrap(),
            ConditionKind::Environment
        );
        assert_eq!(
            c.classify("extra == 'cli'").unwrap(),
            ConditionKind::Activation
        );
        // A mixed marker that mentions extra is activation-gated.
        assert_eq!(
            c.classify("python_version >= '3.10' and extra == 'cli'").unwrap(),
            ConditionKind::Activation
        );
        // `extras` (a different identifier) is not the activation variable.
        assert_eq!(
            c.classify("sys_platform == 'extras'").unwrap(),
            ConditionKind::Environment
        );
    }

    #[test]
    fn none_marker_is_active() {
        let c = UvConditions;
        assert!(c.evaluate(None, GateInput::Unconditional).unwrap());
    }
}
