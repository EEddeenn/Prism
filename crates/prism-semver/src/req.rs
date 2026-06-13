//! [`VersionReq`]: SemVer constraint parsing and matching covering the npm and
//! Cargo range dialects.
//!
//! Supported syntax:
//! - exact: `1.2.3`
//! - comparators: `>=1.2.3`, `>1.2`, `<2`, `<=1.2.3`, `=1.2.3`
//! - caret: `^1.2.3` (incl. `^0.x` rules)
//! - tilde: `~1.2.3`
//! - wildcards: `1.x`, `1.*`, `1.2.*`, `*`
//! - hyphen ranges: `1.2.3 - 2.3.4`
//! - AND sets: comma- or whitespace-separated comparators (`>=1.2, <2`)
//!
//! Prerelease matching follows the SemVer convention used by npm and Cargo: a
//! prerelease version only satisfies a comparator that itself names a
//! prerelease at the *same* `[major, minor, patch]`.

use std::fmt;

use prism_model::{PrismError, Result};

use crate::version::Version;

/// A comparison operator within a single comparator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// `=` — exact equality of the numeric core (and prerelease, if given).
    Exact,
    /// `>` — strictly greater.
    Greater,
    /// `>=` — greater or equal.
    GreaterEq,
    /// `<` — strictly less.
    Less,
    /// `<=` — less or equal.
    LessEq,
}

/// A single comparator: an operator applied to a (possibly partial) version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comparator {
    /// The comparison operator.
    pub op: Op,
    /// Major component (always present once normalized).
    pub major: u64,
    /// Minor component, or `None` when omitted via a wildcard.
    pub minor: Option<u64>,
    /// Patch component, or `None` when omitted via a wildcard.
    pub patch: Option<u64>,
    /// Prerelease identifiers attached to this comparator.
    pub pre: Vec<crate::version::Identifier>,
}

impl Comparator {
    fn from_version(op: Op, v: &Version) -> Self {
        Comparator {
            op,
            major: v.major,
            minor: Some(v.minor),
            patch: Some(v.patch),
            pre: v.pre.clone(),
        }
    }

    fn has_pre(&self) -> bool {
        !self.pre.is_empty()
    }

    /// Returns the full [`Version`] this comparator's bound refers to, filling
    /// omitted components with zero.
    fn bound(&self) -> Version {
        Version {
            major: self.major,
            minor: self.minor.unwrap_or(0),
            patch: self.patch.unwrap_or(0),
            pre: self.pre.clone(),
            build: Vec::new(),
        }
    }

    /// Evaluates this comparator against `v`, including the SemVer prerelease
    /// gate. Caller-level prerelease gating is handled in [`VersionReq::matches`];
    /// here we apply only the raw operator semantics on the numeric core +
    /// prerelease, with partial-version comparators widened appropriately.
    fn matches_core(&self, v: &Version) -> bool {
        use std::cmp::Ordering;
        let bound = self.bound();
        match self.op {
            Op::Exact => {
                // A partial comparator (e.g. `=1.2`) matches the whole range of
                // versions sharing the given prefix.
                match (self.minor, self.patch) {
                    (None, _) => v.major == self.major,
                    (Some(min), None) => v.major == self.major && v.minor == min,
                    (Some(_), Some(_)) => v.cmp(&bound) == Ordering::Equal,
                }
            }
            Op::Greater => {
                match (self.minor, self.patch) {
                    // `>1` means >= 2.0.0; `>1.2` means >= 1.3.0.
                    (None, _) => v.major > self.major,
                    (Some(min), None) => {
                        v.major > self.major || (v.major == self.major && v.minor > min)
                    }
                    (Some(_), Some(_)) => v.cmp(&bound) == Ordering::Greater,
                }
            }
            Op::GreaterEq => v.cmp(&bound) != Ordering::Less,
            Op::Less => v.cmp(&bound) == Ordering::Less,
            Op::LessEq => {
                match (self.minor, self.patch) {
                    // `<=1` means <= 1.x.x i.e. < 2.0.0; `<=1.2` means < 1.3.0.
                    (None, _) => v.major <= self.major,
                    (Some(min), None) => {
                        v.major < self.major || (v.major == self.major && v.minor <= min)
                    }
                    (Some(_), Some(_)) => v.cmp(&bound) != Ordering::Greater,
                }
            }
        }
    }
}

impl fmt::Display for Comparator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let op = match self.op {
            Op::Exact => "=",
            Op::Greater => ">",
            Op::GreaterEq => ">=",
            Op::Less => "<",
            Op::LessEq => "<=",
        };
        write!(f, "{op}{}", self.major)?;
        if let Some(min) = self.minor {
            write!(f, ".{min}")?;
            if let Some(p) = self.patch {
                write!(f, ".{p}")?;
            }
        }
        if !self.pre.is_empty() {
            f.write_str("-")?;
            for (i, id) in self.pre.iter().enumerate() {
                if i > 0 {
                    f.write_str(".")?;
                }
                write!(f, "{id}")?;
            }
        }
        Ok(())
    }
}

/// A version requirement: an AND of one or more [`Comparator`]s.
///
/// A `Version` satisfies the requirement iff it satisfies every comparator.
/// The wildcard `*` (or empty string) yields an empty comparator set that
/// matches any non-prerelease version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionReq {
    /// The conjoined comparators; empty means "any" (`*`).
    pub comparators: Vec<Comparator>,
}

impl VersionReq {
    /// A requirement that matches any (non-prerelease) version.
    pub fn any() -> Self {
        VersionReq {
            comparators: Vec::new(),
        }
    }

    /// Parses a requirement string in the npm/Cargo dialect.
    ///
    /// Returns [`PrismError::Other`] for malformed input; never panics.
    pub fn parse(input: &str) -> Result<VersionReq> {
        let s = input.trim();
        if s.is_empty() || s == "*" {
            return Ok(VersionReq::any());
        }

        // Hyphen range: "A - B". The hyphen must be surrounded by whitespace to
        // distinguish it from prerelease hyphens (e.g. `1.2.3-alpha`).
        if let Some(comparators) = parse_hyphen_range(s)? {
            return Ok(VersionReq { comparators });
        }

        // Otherwise split into AND'd comparator tokens on commas/whitespace.
        let mut comparators = Vec::new();
        for token in split_and(s) {
            if token == "*" {
                // A bare wildcard inside an AND-set imposes no constraint.
                continue;
            }
            comparators.extend(parse_token(token)?);
        }
        Ok(VersionReq { comparators })
    }

    /// Returns `true` iff `v` satisfies every comparator.
    ///
    /// Prerelease gate: a prerelease `v` is only accepted if at least one
    /// comparator names a prerelease at the same `[major, minor, patch]` as
    /// `v`. This mirrors npm/Cargo behaviour where `^1.2.3` does *not* pull in
    /// `2.0.0-rc.1` but `^1.2.3-rc.1` does allow `1.2.3-rc.2`.
    pub fn matches(&self, v: &Version) -> bool {
        if self.comparators.is_empty() {
            // `*` matches any normal release, but not a prerelease.
            return !v.is_prerelease();
        }

        if v.is_prerelease() && !self.allows_prerelease(v) {
            return false;
        }

        self.comparators.iter().all(|c| c.matches_core(v))
    }

    /// Whether some comparator explicitly opts into prereleases at `v`'s core.
    fn allows_prerelease(&self, v: &Version) -> bool {
        self.comparators.iter().any(|c| {
            c.has_pre()
                && c.major == v.major
                && c.minor == Some(v.minor)
                && c.patch == Some(v.patch)
        })
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.comparators.is_empty() {
            return f.write_str("*");
        }
        for (i, c) in self.comparators.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for VersionReq {
    type Err = PrismError;
    fn from_str(s: &str) -> Result<VersionReq> {
        VersionReq::parse(s)
    }
}

// ---- parsing internals ----

/// A possibly-partial version: `(major, minor?, patch?, prerelease)`.
type Partial = (u64, Option<u64>, Option<u64>, Vec<crate::version::Identifier>);

/// Splits an AND-set on commas and runs of whitespace, while keeping operators
/// glued to their operand (`>= 1.2.3` and `>=1.2.3` both yield `>=1.2.3`).
fn split_and(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    let n = bytes.len();
    while i < n {
        // Skip separators (commas and whitespace).
        while i < n && (bytes[i] == b',' || bytes[i].is_ascii_whitespace()) {
            i += 1;
        }
        if i >= n {
            break;
        }
        let start = i;
        // Consume an operator prefix.
        while i < n && matches!(bytes[i], b'>' | b'<' | b'=') {
            i += 1;
        }
        // Allow optional whitespace between operator and version.
        while i < n && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        // Consume the operand (anything up to the next comma/whitespace).
        while i < n && bytes[i] != b',' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        // Re-slice from start to i but strip internal whitespace between op and
        // operand by reconstructing without spaces.
        let raw = &s[start..i];
        tokens.push(raw);
    }
    // Strip any embedded whitespace (from "op  ver" forms) by trimming each.
    tokens.into_iter().map(|t| t.trim()).collect()
}

/// Parses `"A - B"` if present, returning the two-comparator AND-set.
fn parse_hyphen_range(s: &str) -> Result<Option<Vec<Comparator>>> {
    // Find a hyphen surrounded by whitespace.
    let bytes = s.as_bytes();
    let mut split_at = None;
    for i in 0..bytes.len() {
        if bytes[i] == b'-'
            && i > 0
            && i + 1 < bytes.len()
            && bytes[i - 1].is_ascii_whitespace()
            && bytes[i + 1].is_ascii_whitespace()
        {
            split_at = Some(i);
            break;
        }
    }
    let Some(idx) = split_at else {
        return Ok(None);
    };
    let lower = s[..idx].trim();
    let upper = s[idx + 1..].trim();
    if lower.is_empty() || upper.is_empty() {
        return Err(PrismError::Other("malformed hyphen range".to_string()));
    }

    let (lmaj, lmin, lpat, lpre) = parse_partial(lower)?;
    let low = Comparator {
        op: Op::GreaterEq,
        major: lmaj,
        minor: lmin,
        patch: lpat,
        pre: lpre,
    };

    let (umaj, umin, upat, upre) = parse_partial(upper)?;
    // Upper bound: if patch omitted, it's inclusive of the whole minor/major.
    let high = match (umin, upat) {
        (Some(min), Some(pat)) => Comparator {
            op: Op::LessEq,
            major: umaj,
            minor: Some(min),
            patch: Some(pat),
            pre: upre,
        },
        (Some(min), None) => Comparator {
            // `... - 2.3` => < 2.4.0
            op: Op::Less,
            major: umaj,
            minor: Some(bump(min)?),
            patch: Some(0),
            pre: Vec::new(),
        },
        (None, _) => Comparator {
            // `... - 2` => < 3.0.0
            op: Op::Less,
            major: bump(umaj)?,
            minor: Some(0),
            patch: Some(0),
            pre: Vec::new(),
        },
    };
    Ok(Some(vec![low, high]))
}

/// Parses a single comparator token (operator + sugar) into comparators.
fn parse_token(token: &str) -> Result<Vec<Comparator>> {
    let t = token.trim();
    if t.is_empty() {
        return Ok(Vec::new());
    }
    let first = t.as_bytes()[0];
    match first {
        b'^' => parse_caret(&t[1..]),
        b'~' => parse_tilde(&t[1..]),
        b'>' | b'<' | b'=' => parse_comparator(t),
        _ => parse_bare(t),
    }
}

/// Parses an explicit operator comparator like `>=1.2.3`.
fn parse_comparator(t: &str) -> Result<Vec<Comparator>> {
    let (op, rest) = if let Some(r) = t.strip_prefix(">=") {
        (Op::GreaterEq, r)
    } else if let Some(r) = t.strip_prefix("<=") {
        (Op::LessEq, r)
    } else if let Some(r) = t.strip_prefix('>') {
        (Op::Greater, r)
    } else if let Some(r) = t.strip_prefix('<') {
        (Op::Less, r)
    } else if let Some(r) = t.strip_prefix('=') {
        (Op::Exact, r)
    } else {
        return Err(PrismError::Other(format!("unknown comparator: {t}")));
    };
    let rest = rest.trim();
    let (major, minor, patch, pre) = parse_partial(rest)?;
    Ok(vec![Comparator {
        op,
        major,
        minor,
        patch,
        pre,
    }])
}

/// Parses a bare version / wildcard with no operator (`1.2.3`, `1.x`, `1`).
fn parse_bare(t: &str) -> Result<Vec<Comparator>> {
    // Wildcard forms expand to a half-open range.
    if let Some(cs) = parse_wildcard(t)? {
        return Ok(cs);
    }
    // A fully specified bare version is exact.
    let v = Version::parse(t)?;
    Ok(vec![Comparator::from_version(Op::Exact, &v)])
}

/// Detects and expands wildcard syntax (`*`, `1.x`, `1.*`, `1.2.x`).
///
/// Returns `Ok(None)` if `t` is not a wildcard form.
fn parse_wildcard(t: &str) -> Result<Option<Vec<Comparator>>> {
    let parts: Vec<&str> = t.split('.').collect();
    let is_wild = |p: &str| matches!(p, "*" | "x" | "X");

    // `*` alone => any.
    if parts.len() == 1 && is_wild(parts[0]) {
        return Ok(Some(Vec::new()));
    }

    // Find the first wildcard component, if any.
    let wild_pos = parts.iter().position(|p| is_wild(p));
    let Some(pos) = wild_pos else {
        return Ok(None);
    };
    // Everything before the wildcard must be a plain number.
    let nums: Result<Vec<u64>> = parts[..pos]
        .iter()
        .map(|p| {
            p.parse::<u64>()
                .map_err(|_| PrismError::Other(format!("invalid version component: {p}")))
        })
        .collect();
    let nums = nums?;
    // Anything after the first wildcard must also be a wildcard (or absent).
    if parts[pos + 1..].iter().any(|p| !is_wild(p)) {
        return Err(PrismError::Other(format!("invalid wildcard pattern: {t}")));
    }

    match nums.len() {
        // `*.x` style with no leading number is just "any".
        0 => Ok(Some(Vec::new())),
        // `1.*` / `1.x` => >=1.0.0, <2.0.0
        1 => Ok(Some(vec![
            Comparator {
                op: Op::GreaterEq,
                major: nums[0],
                minor: Some(0),
                patch: Some(0),
                pre: Vec::new(),
            },
            Comparator {
                op: Op::Less,
                major: bump(nums[0])?,
                minor: Some(0),
                patch: Some(0),
                pre: Vec::new(),
            },
        ])),
        // `1.2.*` / `1.2.x` => >=1.2.0, <1.3.0
        2 => Ok(Some(vec![
            Comparator {
                op: Op::GreaterEq,
                major: nums[0],
                minor: Some(nums[1]),
                patch: Some(0),
                pre: Vec::new(),
            },
            Comparator {
                op: Op::Less,
                major: nums[0],
                minor: Some(bump(nums[1])?),
                patch: Some(0),
                pre: Vec::new(),
            },
        ])),
        _ => Err(PrismError::Other(format!(
            "too many components in wildcard: {t}"
        ))),
    }
}

/// Expands a caret requirement `^…` into a `>=lower, <upper` pair.
///
/// Caret allows changes that do not modify the left-most non-zero component:
/// - `^1.2.3` => `>=1.2.3, <2.0.0`
/// - `^0.2.3` => `>=0.2.3, <0.3.0`
/// - `^0.0.3` => `>=0.0.3, <0.0.4`
/// - `^1.2`   => `>=1.2.0, <2.0.0`
/// - `^0.0`   => `>=0.0.0, <0.1.0`
/// - `^0`     => `>=0.0.0, <1.0.0`
fn parse_caret(rest: &str) -> Result<Vec<Comparator>> {
    let (major, minor, patch, pre) = parse_partial(rest)?;
    let lower = Comparator {
        op: Op::GreaterEq,
        major,
        minor: Some(minor.unwrap_or(0)),
        patch: Some(patch.unwrap_or(0)),
        pre,
    };

    let upper = if major > 0 {
        ver_lt(bump(major)?, 0, 0)
    } else {
        // major == 0
        match minor {
            None => ver_lt(1, 0, 0), // ^0 => <1.0.0
            Some(min) if min > 0 => ver_lt(0, bump(min)?, 0),
            Some(_) => {
                // minor == 0
                match patch {
                    None => ver_lt(0, 1, 0), // ^0.0 => <0.1.0
                    Some(p) => ver_lt(0, 0, bump(p)?),
                }
            }
        }
    };
    Ok(vec![lower, upper])
}

/// Expands a tilde requirement `~…` into a `>=lower, <upper` pair.
///
/// Tilde allows patch-level changes when a minor is given, otherwise
/// minor-level changes:
/// - `~1.2.3` => `>=1.2.3, <1.3.0`
/// - `~1.2`   => `>=1.2.0, <1.3.0`
/// - `~1`     => `>=1.0.0, <2.0.0`
fn parse_tilde(rest: &str) -> Result<Vec<Comparator>> {
    let (major, minor, patch, pre) = parse_partial(rest)?;
    let lower = Comparator {
        op: Op::GreaterEq,
        major,
        minor: Some(minor.unwrap_or(0)),
        patch: Some(patch.unwrap_or(0)),
        pre,
    };
    let upper = match minor {
        Some(min) => ver_lt(major, bump(min)?, 0),
        None => ver_lt(bump(major)?, 0, 0),
    };
    Ok(vec![lower, upper])
}

fn ver_lt(major: u64, minor: u64, patch: u64) -> Comparator {
    Comparator {
        op: Op::Less,
        major,
        minor: Some(minor),
        patch: Some(patch),
        pre: Vec::new(),
    }
}

/// `x + 1` with overflow rejected: version components come verbatim from an
/// untrusted requirement string (`num` accepts the whole `u64` range), so a
/// component at `u64::MAX` must not wrap an upper bound — it would panic in debug
/// and silently produce a nonsensical (empty-matching) comparator in release.
fn bump(x: u64) -> Result<u64> {
    x.checked_add(1).ok_or_else(|| {
        PrismError::Other("version component too large to form an upper bound".into())
    })
}

/// Parses a possibly-partial version string `MAJOR[.MINOR[.PATCH]][-pre]`,
/// returning `(major, minor?, patch?, pre)`. Wildcard placeholders (`x`/`*`)
/// in trailing positions are treated as omission.
fn parse_partial(s: &str) -> Result<Partial> {
    let s = s.trim();
    if s.is_empty() {
        return Err(PrismError::Other("empty version in comparator".to_string()));
    }
    // Separate an optional prerelease tag. Build metadata is dropped.
    let core_pre = s.split('+').next().unwrap_or(s);
    let (core, pre_str) = match core_pre.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (core_pre, None),
    };

    let mut parts = core.split('.');
    let is_wild = |p: &str| matches!(p, "*" | "x" | "X");

    let major_s = parts
        .next()
        .ok_or_else(|| PrismError::Other("missing major in comparator".to_string()))?;
    if is_wild(major_s) {
        // `x.y.z` with wildcard major is handled by parse_wildcard; reaching
        // here means an operator was applied to a wildcard, which we reject.
        return Err(PrismError::Other(format!(
            "wildcard major not allowed here: {s}"
        )));
    }
    let major = num(major_s)?;

    let minor_s = parts.next();
    let patch_s = parts.next();
    if parts.next().is_some() {
        return Err(PrismError::Other(format!(
            "too many version components: {s}"
        )));
    }

    let minor = match minor_s {
        None => None,
        Some(p) if is_wild(p) => None,
        Some(p) => Some(num(p)?),
    };
    let patch = match patch_s {
        None => None,
        Some(p) if is_wild(p) => None,
        Some(p) => {
            if minor.is_none() {
                // e.g. `1.x.3` is nonsensical.
                return Err(PrismError::Other(format!(
                    "patch specified after wildcard minor: {s}"
                )));
            }
            Some(num(p)?)
        }
    };

    // Prerelease is only meaningful with a full core.
    let pre = match pre_str {
        Some(p) if patch.is_some() => crate::version::parse_pre_public(p)?,
        Some(_) => {
            return Err(PrismError::Other(format!(
                "prerelease requires full version: {s}"
            )));
        }
        None => Vec::new(),
    };

    Ok((major, minor, patch, pre))
}

fn num(p: &str) -> Result<u64> {
    if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
        return Err(PrismError::Other(format!("invalid numeric component: {p}")));
    }
    if p.len() > 1 && p.starts_with('0') {
        return Err(PrismError::Other(format!("leading zero in component: {p}")));
    }
    p.parse::<u64>()
        .map_err(|_| PrismError::Other(format!("component out of range: {p}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }
    fn req(s: &str) -> VersionReq {
        VersionReq::parse(s).unwrap()
    }

    #[test]
    fn exact() {
        let r = req("1.2.3");
        assert!(r.matches(&v("1.2.3")));
        assert!(!r.matches(&v("1.2.4")));
        assert!(!r.matches(&v("1.2.2")));
    }

    #[test]
    fn comparators() {
        assert!(req(">=1.2.3").matches(&v("1.2.3")));
        assert!(req(">=1.2.3").matches(&v("9.0.0")));
        assert!(!req(">=1.2.3").matches(&v("1.2.2")));
        assert!(req(">1.2.3").matches(&v("1.2.4")));
        assert!(!req(">1.2.3").matches(&v("1.2.3")));
        assert!(req("<2.0.0").matches(&v("1.9.9")));
        assert!(!req("<2.0.0").matches(&v("2.0.0")));
        assert!(req("<=2.0.0").matches(&v("2.0.0")));
        assert!(req("=1.2.3").matches(&v("1.2.3")));
    }

    #[test]
    fn and_set_comma_and_space() {
        let r = req(">=1.2.0, <1.5.0");
        assert!(r.matches(&v("1.3.0")));
        assert!(!r.matches(&v("1.5.0")));
        let r2 = req(">=1.2.0 <1.5.0");
        assert!(r2.matches(&v("1.4.9")));
        assert!(!r2.matches(&v("1.1.0")));
    }

    #[test]
    fn caret_major_nonzero() {
        let r = req("^1.2.3");
        assert!(r.matches(&v("1.2.3")));
        assert!(r.matches(&v("1.9.0"))); // task: matches 1.9.0
        assert!(r.matches(&v("1.2.4")));
        assert!(!r.matches(&v("2.0.0"))); // task: not 2.0.0
        assert!(!r.matches(&v("1.2.2")));
    }

    #[test]
    fn caret_zero_minor() {
        let r = req("^0.2.3");
        assert!(r.matches(&v("0.2.3")));
        assert!(r.matches(&v("0.2.9"))); // task: matches 0.2.9
        assert!(!r.matches(&v("0.3.0"))); // task: not 0.3.0
        assert!(!r.matches(&v("0.2.2")));
    }

    #[test]
    fn caret_zero_zero() {
        let r = req("^0.0.3");
        assert!(r.matches(&v("0.0.3")));
        assert!(!r.matches(&v("0.0.4")));
        assert!(!r.matches(&v("0.1.0")));
    }

    #[test]
    fn caret_partial() {
        assert!(req("^1.2").matches(&v("1.9.9")));
        assert!(!req("^1.2").matches(&v("2.0.0")));
        assert!(req("^0").matches(&v("0.9.9")));
        assert!(!req("^0").matches(&v("1.0.0")));
        assert!(req("^0.0").matches(&v("0.0.9")));
        assert!(!req("^0.0").matches(&v("0.1.0")));
    }

    #[test]
    fn tilde() {
        let r = req("~1.2.3");
        assert!(r.matches(&v("1.2.3")));
        assert!(r.matches(&v("1.2.9"))); // task: matches 1.2.9
        assert!(!r.matches(&v("1.3.0"))); // task: not 1.3.0
        assert!(!r.matches(&v("1.2.2")));
        assert!(req("~1.2").matches(&v("1.2.5")));
        assert!(!req("~1.2").matches(&v("1.3.0")));
        assert!(req("~1").matches(&v("1.9.9")));
        assert!(!req("~1").matches(&v("2.0.0")));
    }

    #[test]
    fn wildcards() {
        assert!(req("*").matches(&v("1.2.3")));
        assert!(req("*").matches(&v("0.0.1")));
        let r = req("1.x");
        assert!(r.matches(&v("1.0.0")));
        assert!(r.matches(&v("1.9.9")));
        assert!(!r.matches(&v("2.0.0")));
        assert!(!r.matches(&v("0.9.0")));
        let r2 = req("1.2.*");
        assert!(r2.matches(&v("1.2.0")));
        assert!(r2.matches(&v("1.2.99")));
        assert!(!r2.matches(&v("1.3.0")));
        assert!(req("1.*").matches(&v("1.5.0")));
    }

    #[test]
    fn hyphen_range() {
        let r = req("1.2.3 - 2.3.4");
        assert!(r.matches(&v("1.2.3")));
        assert!(r.matches(&v("2.0.0")));
        assert!(r.matches(&v("2.3.4")));
        assert!(!r.matches(&v("2.3.5")));
        assert!(!r.matches(&v("1.2.2")));
        // Partial upper bound.
        let r2 = req("1.2.3 - 2.3");
        assert!(r2.matches(&v("2.3.9")));
        assert!(!r2.matches(&v("2.4.0")));
        let r3 = req("1.2 - 2");
        assert!(r3.matches(&v("2.9.9")));
        assert!(!r3.matches(&v("3.0.0")));
        assert!(r3.matches(&v("1.2.0")));
    }

    #[test]
    fn extreme_components_error_not_panic_or_wrap() {
        // u64::MAX in an upper-bound-forming position must be a typed error, never
        // a panic (debug) or a wrapped nonsensical comparator (release).
        let m = "18446744073709551615";
        for s in [
            format!("^{m}"),                 // caret major
            format!("^0.{m}"),               // caret minor
            format!("^0.0.{m}"),             // caret patch
            format!("~{m}"),                 // tilde major
            format!("~1.{m}"),               // tilde minor
            format!("{m}.x"),                // wildcard major
            format!("1.{m}.x"),              // wildcard minor
            format!("1.0.0 - {m}"),          // hyphen upper major
            format!("1.0.0 - 1.{m}"),        // hyphen upper minor
        ] {
            let r = VersionReq::parse(&s);
            assert!(r.is_err(), "expected typed error for {s:?}, got {r:?}");
        }
        // One below the max still forms a valid bound.
        assert!(VersionReq::parse("^18446744073709551614").is_ok());
    }

    #[test]
    fn prerelease_gate() {
        // ^1.2.3 must NOT match a prerelease of a different patch/minor.
        let r = req("^1.2.3");
        assert!(!r.matches(&v("1.3.0-rc.1")));
        assert!(!r.matches(&v("2.0.0-rc.1")));
        // A comparator naming the same core+pre admits prereleases there.
        let r2 = req(">=1.2.3-rc.1, <2.0.0");
        assert!(r2.matches(&v("1.2.3-rc.1")));
        assert!(r2.matches(&v("1.2.3-rc.2")));
        assert!(r2.matches(&v("1.2.3")));
        // But not a prerelease at a different core.
        assert!(!r2.matches(&v("1.5.0-rc.1")));
    }

    #[test]
    fn star_excludes_prerelease() {
        assert!(!req("*").matches(&v("1.0.0-rc.1")));
        assert!(req("*").matches(&v("1.0.0")));
    }

    #[test]
    fn display_roundtrip_parse() {
        for s in [">=1.2.3", "<=2.0.0", ">1.0.0", "<3.0.0", "=1.2.3"] {
            let r = req(s);
            // Re-parsing the Display output yields an equal requirement.
            assert_eq!(VersionReq::parse(&r.to_string()).unwrap(), r);
        }
    }

    #[test]
    fn rejects_garbage() {
        for s in ["1.2.3.4", ">=a.b.c", "^x.y", "~", ">=", "1 - ", " - 2"] {
            assert!(VersionReq::parse(s).is_err(), "should reject {s:?}");
        }
    }

    #[test]
    fn whitespace_between_op_and_version() {
        let r = req(">= 1.2.0");
        assert!(r.matches(&v("1.2.0")));
        assert!(!r.matches(&v("1.1.9")));
    }
}
