//! PEP 440 version-specifier requirements.
//!
//! A PEP 508 requirement is `name[extras] (specifiers) ; marker`. This parser
//! works on the **specifier set** — the comma-separated version constraints — and
//! tests membership with PEP 440 operators: `==`, `!=`, `<=`, `>=`, `<`, `>`,
//! `~=` (compatible release), `===` (arbitrary equality), and the `==X.*` /
//! `!=X.*` prefix forms. A leading project name, `[extras]`, surrounding
//! parentheses, and a trailing `; marker` are stripped before parsing.
//!
//! `canonicalize` returns the specifiers normalized and comma-joined so
//! equivalent requirements collapse to one `RequirementId`.

use prism_plugin_api::{ParseError, RequirementParser};

use crate::version::Pep440;

/// One PEP 440 version specifier.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Specifier {
    op: Op,
    /// The raw version text of the specifier (trailing `.*` stripped into `prefix`).
    version: String,
    /// True for the `==X.*` / `!=X.*` prefix-match forms.
    prefix: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Eq,
    Ne,
    Le,
    Ge,
    Lt,
    Gt,
    Compatible, // ~=
    Arbitrary,  // ===
}

impl Op {
    fn as_str(self) -> &'static str {
        match self {
            Op::Eq => "==",
            Op::Ne => "!=",
            Op::Le => "<=",
            Op::Ge => ">=",
            Op::Lt => "<",
            Op::Gt => ">",
            Op::Compatible => "~=",
            Op::Arbitrary => "===",
        }
    }
}

/// Strip a leading project name + `[extras]`, surrounding parens, and a trailing
/// `; marker`, returning just the specifier-set text.
fn specifier_text(raw: &str) -> &str {
    let mut s = raw.trim();
    // Drop a PEP 508 marker.
    if let Some((head, _marker)) = s.split_once(';') {
        s = head.trim();
    }
    // Drop a leading name + optional [extras]: skip up to the first specifier
    // operator or open paren.
    if let Some(pos) = s.find(['<', '>', '=', '!', '~', '(']) {
        s = s[pos..].trim();
    } else {
        // No operator at all -> a bare name (any version).
        return "";
    }
    // Drop surrounding parens.
    s = s.trim();
    if let Some(inner) = s.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        s = inner.trim();
    }
    s
}

/// Parse a specifier-set string into specifiers. Empty / `*` → no constraints.
fn parse_specifiers(raw: &str) -> Result<Vec<Specifier>, ParseError> {
    let text = specifier_text(raw);
    if text.is_empty() || text == "*" {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for part in text.split(',') {
        let t = part.trim();
        if t.is_empty() {
            continue;
        }
        out.push(parse_one(t)?);
    }
    Ok(out)
}

fn parse_one(t: &str) -> Result<Specifier, ParseError> {
    // Longest operators first so `===`/`==`/`<=` win over `<`/`=`.
    let (op, rest) = if let Some(r) = t.strip_prefix("===") {
        (Op::Arbitrary, r)
    } else if let Some(r) = t.strip_prefix("==") {
        (Op::Eq, r)
    } else if let Some(r) = t.strip_prefix("!=") {
        (Op::Ne, r)
    } else if let Some(r) = t.strip_prefix("<=") {
        (Op::Le, r)
    } else if let Some(r) = t.strip_prefix(">=") {
        (Op::Ge, r)
    } else if let Some(r) = t.strip_prefix("~=") {
        (Op::Compatible, r)
    } else if let Some(r) = t.strip_prefix('<') {
        (Op::Lt, r)
    } else if let Some(r) = t.strip_prefix('>') {
        (Op::Gt, r)
    } else {
        return Err(ParseError::new("bad-specifier", format!("no operator in {t:?}")));
    };
    let ver = rest.trim();
    let (version, prefix) = match ver.strip_suffix(".*") {
        Some(base) => (base.to_string(), true),
        None => (ver.to_string(), false),
    };
    if version.is_empty() {
        return Err(ParseError::new("bad-specifier", format!("empty version in {t:?}")));
    }
    Ok(Specifier { op, version, prefix })
}

/// Test whether `version` satisfies a single specifier.
fn satisfies(spec: &Specifier, version: &Pep440, version_raw: &str) -> bool {
    // `===` is a raw string comparison, no PEP 440 parsing.
    if spec.op == Op::Arbitrary {
        return version_raw.trim() == spec.version;
    }
    let Some(target) = Pep440::parse(&spec.version) else {
        return false; // an unparseable specifier version matches nothing
    };
    if spec.prefix {
        // `==X.*` / `!=X.*`: prefix-match on the release tuple.
        let is_prefix = version.release_starts_with(&target);
        return match spec.op {
            Op::Eq => is_prefix,
            Op::Ne => !is_prefix,
            // A prefix form with another operator is not standard; treat as exact.
            _ => false,
        };
    }
    let ord = version.cmp(&target);
    match spec.op {
        Op::Eq => version.equals_for_eq(&target),
        Op::Ne => !version.equals_for_eq(&target),
        Op::Lt => ord == std::cmp::Ordering::Less,
        Op::Le => ord != std::cmp::Ordering::Greater,
        Op::Gt => ord == std::cmp::Ordering::Greater,
        Op::Ge => ord != std::cmp::Ordering::Less,
        Op::Compatible => {
            // `~=` is `>= target` AND same release prefix (target with its last
            // release component dropped).
            ord != std::cmp::Ordering::Less && version.compatible_with(&target)
        }
        Op::Arbitrary => unreachable!("handled above"),
    }
}

/// PEP 440 [`RequirementParser`].
pub struct UvReqs;

impl RequirementParser for UvReqs {
    fn canonicalize(&self, raw: &str) -> Result<String, ParseError> {
        let mut specs = parse_specifiers(raw)?;
        if specs.is_empty() {
            return Ok("*".to_string());
        }
        // Stable canonical order so equivalent sets collapse.
        specs.sort_by(|a, b| {
            (a.op.as_str(), &a.version, a.prefix).cmp(&(b.op.as_str(), &b.version, b.prefix))
        });
        let parts: Vec<String> = specs
            .iter()
            .map(|s| format!("{}{}{}", s.op.as_str(), s.version, if s.prefix { ".*" } else { "" }))
            .collect();
        Ok(parts.join(","))
    }

    fn matches(&self, raw: &str, version: &str) -> Result<bool, ParseError> {
        let specs = parse_specifiers(raw)?;
        let Some(v) = Pep440::parse(version) else {
            return Ok(false);
        };
        Ok(specs.iter().all(|s| satisfies(s, &v, version)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(req: &str, ver: &str) -> bool {
        UvReqs.matches(req, ver).unwrap()
    }

    #[test]
    fn comparison_operators() {
        assert!(m(">=4.0", "4.13.0"));
        assert!(!m(">=4.0", "3.9"));
        assert!(m(">=1.0,<2.0", "1.5"));
        assert!(!m(">=1.0,<2.0", "2.0"));
        assert!(m("<=1.0", "1.0"));
        assert!(m(">1.0", "1.0.1"));
        assert!(!m(">1.0", "1.0"));
        assert!(m("!=1.5", "1.6"));
        assert!(!m("!=1.5", "1.5"));
    }

    #[test]
    fn prefix_and_compatible() {
        assert!(m("==1.4.*", "1.4.9"));
        assert!(!m("==1.4.*", "1.5.0"));
        assert!(m("!=1.4.*", "1.5.0"));
        // ~=1.2.3 means >=1.2.3, ==1.2.*
        assert!(m("~=1.2.3", "1.2.9"));
        assert!(!m("~=1.2.3", "1.3.0"));
        assert!(!m("~=1.2.3", "1.2.2"));
        // ~=1.2 means >=1.2, ==1.*
        assert!(m("~=1.2", "1.9"));
        assert!(!m("~=1.2", "2.0"));
    }

    #[test]
    fn strips_name_marker_and_parens() {
        // A full PEP 508 requirement reduces to its specifier set.
        assert!(m("anyio>=4.0; python_version < '3.11'", "4.13.0"));
        assert!(m("anyio (>=4.0)", "4.13.0"));
        // A bare name (no specifier) matches anything.
        assert!(m("anyio", "4.13.0"));
        assert!(m("*", "9.9.9"));
    }

    #[test]
    fn canonicalize_collapses_and_sorts() {
        let r = UvReqs;
        assert_eq!(r.canonicalize("anyio").unwrap(), "*");
        // The two orderings of the same set collapse to one canonical string.
        let a = r.canonicalize("<2.0,>=1.0").unwrap();
        let b = r.canonicalize(">=1.0, <2.0").unwrap();
        assert_eq!(a, b);
        assert_eq!(a, "<2.0,>=1.0");
    }

    #[test]
    fn arbitrary_equality_is_raw() {
        assert!(m("===1.0.0", "1.0.0"));
        assert!(!m("===1.0.0", "1.0"));
    }
}
