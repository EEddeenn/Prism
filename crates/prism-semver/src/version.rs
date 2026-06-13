//! SemVer 2.0 [`Version`]: parsing, total ordering, and sortable byte keys.
//!
//! Ordering follows the [SemVer 2.0 spec](https://semver.org/): the numeric
//! `major.minor.patch` core compares numerically; a version *with* a prerelease
//! is lower than the otherwise-identical version *without* one; prerelease
//! identifiers compare field by field (numeric identifiers numerically and
//! ranking below alphanumeric ones, alphanumeric identifiers lexically); build
//! metadata is ignored entirely in ordering.

use std::cmp::Ordering;
use std::fmt;

use prism_model::{PrismError, Result};

/// A single dot-separated prerelease identifier.
///
/// SemVer distinguishes purely numeric identifiers (compared numerically, and
/// ranked *below* alphanumeric ones) from alphanumeric identifiers (compared
/// by ASCII lexical order).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Identifier {
    /// A purely numeric identifier with no leading zeros (e.g. `1`, `0`).
    Numeric(u64),
    /// An alphanumeric (or hyphen-containing) identifier (e.g. `alpha`, `x-1`).
    AlphaNumeric(String),
}

impl Identifier {
    fn cmp(&self, other: &Identifier) -> Ordering {
        match (self, other) {
            (Identifier::Numeric(a), Identifier::Numeric(b)) => a.cmp(b),
            (Identifier::AlphaNumeric(a), Identifier::AlphaNumeric(b)) => a.cmp(b),
            // Numeric identifiers always have lower precedence than alphanumeric.
            (Identifier::Numeric(_), Identifier::AlphaNumeric(_)) => Ordering::Less,
            (Identifier::AlphaNumeric(_), Identifier::Numeric(_)) => Ordering::Greater,
        }
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Identifier::Numeric(n) => write!(f, "{n}"),
            Identifier::AlphaNumeric(s) => f.write_str(s),
        }
    }
}

/// A parsed SemVer 2.0 version.
///
/// Equality and ordering ignore [`Version::build`] (build metadata), per spec.
#[derive(Clone, Debug)]
pub struct Version {
    /// Major version (breaking-change component).
    pub major: u64,
    /// Minor version (feature component).
    pub minor: u64,
    /// Patch version (fix component).
    pub patch: u64,
    /// Dot-separated prerelease identifiers; empty for a normal release.
    pub pre: Vec<Identifier>,
    /// Dot-separated build-metadata identifiers; ignored in ordering/equality.
    pub build: Vec<String>,
}

impl Version {
    /// Constructs a release version `major.minor.patch` with no pre/build parts.
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Version {
            major,
            minor,
            patch,
            pre: Vec::new(),
            build: Vec::new(),
        }
    }

    /// Whether this version carries a prerelease tag.
    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }

    /// Parses a `MAJOR.MINOR.PATCH[-prerelease][+build]` string.
    ///
    /// Returns [`PrismError::Other`] for any malformed input; never panics.
    pub fn parse(input: &str) -> Result<Version> {
        let s = input.trim();
        if s.is_empty() {
            return Err(bad("empty version string"));
        }

        // Split off build metadata (first '+') and prerelease (first '-' in the
        // remaining core+pre segment).
        let (core_pre, build_str) = match s.split_once('+') {
            Some((cp, b)) => (cp, Some(b)),
            None => (s, None),
        };
        let (core, pre_str) = match core_pre.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (core_pre, None),
        };

        let mut parts = core.split('.');
        let major = parse_core_num(parts.next(), "major")?;
        let minor = parse_core_num(parts.next(), "minor")?;
        let patch = parse_core_num(parts.next(), "patch")?;
        if parts.next().is_some() {
            return Err(bad("version core has more than three numeric fields"));
        }

        let pre = match pre_str {
            Some(p) => parse_pre(p)?,
            None => Vec::new(),
        };
        let build = match build_str {
            Some(b) => parse_build(b)?,
            None => Vec::new(),
        };

        Ok(Version {
            major,
            minor,
            patch,
            pre,
            build,
        })
    }

    /// Returns a byte string whose lexical (`[u8]`) order matches [`Ord`] exactly.
    ///
    /// This is the value stored as `PackageVersion::version_sort_key`. The
    /// encoding guarantees that, for any two versions `a` and `b`,
    /// `a.sort_key().cmp(&b.sort_key()) == a.cmp(&b)`. Build metadata is omitted
    /// (it has no effect on ordering).
    ///
    /// Layout:
    /// 1. The three core numbers, each length-prefixed (so longer = larger),
    ///    big-endian. This makes them compare numerically under byte order.
    /// 2. A one-byte presence tag: `0x00` if a prerelease is present, `0x01`
    ///    if absent — so a release sorts *after* all of its prereleases.
    /// 3. For each prerelease identifier, a typed, order-preserving encoding,
    ///    separated by `0x00` and terminated by `0x00` (which is < any real
    ///    identifier byte), so a shorter prefix run sorts first.
    pub fn sort_key(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32);
        encode_num(&mut out, self.major);
        encode_num(&mut out, self.minor);
        encode_num(&mut out, self.patch);

        // Presence tag: prerelease present => 0, absent => 1, so release > pre.
        out.push(if self.pre.is_empty() { 1 } else { 0 });

        for id in &self.pre {
            // Separator/terminator 0x00 ranks below any encoded identifier byte
            // (which all start at >= 0x01), so a version that runs out of
            // identifiers first sorts lower (e.g. 1.0.0-alpha < 1.0.0-alpha.1).
            out.push(0x00);
            match id {
                Identifier::Numeric(n) => {
                    // Tag 0x01 < 0x02: numeric ranks below alphanumeric.
                    out.push(0x01);
                    encode_num(&mut out, *n);
                }
                Identifier::AlphaNumeric(text) => {
                    out.push(0x02);
                    // ASCII bytes are all >= 0x01, so lexical byte order matches
                    // String order and never collides with the 0x00 separator.
                    out.extend_from_slice(text.as_bytes());
                }
            }
        }
        // Final terminator so a prefix run sorts before a longer one.
        out.push(0x00);
        out
    }
}

/// Length-prefixed big-endian encoding of `n` whose byte order matches numeric
/// order: a one-byte length precedes the minimal big-endian bytes, so smaller
/// numbers (fewer bytes) sort first and equal-length numbers compare by value.
fn encode_num(out: &mut Vec<u8>, n: u64) {
    let bytes = n.to_be_bytes();
    // Strip leading zero bytes to get the minimal big-endian representation.
    let first = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    let trimmed = &bytes[first..];
    out.push(trimmed.len() as u8);
    out.extend_from_slice(trimmed);
}

fn parse_core_num(part: Option<&str>, field: &str) -> Result<u64> {
    let p = part.ok_or_else(|| bad(format!("missing {field} version component")))?;
    if p.is_empty() {
        return Err(bad(format!("empty {field} version component")));
    }
    if p.len() > 1 && p.starts_with('0') {
        return Err(bad(format!("{field} component has a leading zero: {p}")));
    }
    if !p.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad(format!("non-numeric {field} component: {p}")));
    }
    p.parse::<u64>()
        .map_err(|_| bad(format!("{field} component out of range: {p}")))
}

fn parse_pre(s: &str) -> Result<Vec<Identifier>> {
    let mut ids = Vec::new();
    for part in s.split('.') {
        ids.push(parse_pre_identifier(part)?);
    }
    Ok(ids)
}

/// Parses a dot-separated prerelease string into identifiers.
///
/// Exposed for the constraint parser so comparators like `>=1.2.3-rc.1` reuse
/// the exact same identifier rules as version parsing.
pub(crate) fn parse_pre_public(s: &str) -> Result<Vec<Identifier>> {
    parse_pre(s)
}

fn parse_pre_identifier(part: &str) -> Result<Identifier> {
    if part.is_empty() {
        return Err(bad("empty prerelease identifier"));
    }
    if !part.bytes().all(is_ident_byte) {
        return Err(bad(format!("invalid prerelease identifier: {part}")));
    }
    let all_digits = part.bytes().all(|b| b.is_ascii_digit());
    if all_digits {
        if part.len() > 1 && part.starts_with('0') {
            return Err(bad(format!(
                "numeric prerelease identifier has a leading zero: {part}"
            )));
        }
        match part.parse::<u64>() {
            Ok(n) => Ok(Identifier::Numeric(n)),
            // Too large to fit u64: keep it as an alphanumeric identifier so we
            // never lose information or panic on untrusted input.
            Err(_) => Ok(Identifier::AlphaNumeric(part.to_string())),
        }
    } else {
        Ok(Identifier::AlphaNumeric(part.to_string()))
    }
}

fn parse_build(s: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for part in s.split('.') {
        if part.is_empty() {
            return Err(bad("empty build-metadata identifier"));
        }
        if !part.bytes().all(is_ident_byte) {
            return Err(bad(format!("invalid build-metadata identifier: {part}")));
        }
        out.push(part.to_string());
    }
    Ok(out)
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-'
}

fn bad(msg: impl Into<String>) -> PrismError {
    PrismError::Other(msg.into())
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl std::hash::Hash for Version {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Consistent with Eq/Ord: build metadata is excluded.
        self.major.hash(state);
        self.minor.hash(state);
        self.patch.hash(state);
        self.pre.hash(state);
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
            .then_with(|| cmp_pre(&self.pre, &other.pre))
    }
}

/// Compares prerelease identifier lists per SemVer: an empty list (a release)
/// is *greater* than a non-empty one; otherwise compare field by field, and a
/// shorter run is lower when all shared fields are equal.
fn cmp_pre(a: &[Identifier], b: &[Identifier]) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater, // release > prerelease
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }
    for (x, y) in a.iter().zip(b.iter()) {
        match x.cmp(y) {
            Ordering::Equal => continue,
            non_eq => return non_eq,
        }
    }
    a.len().cmp(&b.len())
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            f.write_str("-")?;
            for (i, id) in self.pre.iter().enumerate() {
                if i > 0 {
                    f.write_str(".")?;
                }
                write!(f, "{id}")?;
            }
        }
        if !self.build.is_empty() {
            f.write_str("+")?;
            for (i, b) in self.build.iter().enumerate() {
                if i > 0 {
                    f.write_str(".")?;
                }
                f.write_str(b)?;
            }
        }
        Ok(())
    }
}

impl std::str::FromStr for Version {
    type Err = PrismError;
    fn from_str(s: &str) -> Result<Version> {
        Version::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn parse_basic() {
        let x = v("1.2.3");
        assert_eq!((x.major, x.minor, x.patch), (1, 2, 3));
        assert!(x.pre.is_empty());
        assert!(x.build.is_empty());
    }

    #[test]
    fn parse_full() {
        let x = v("1.2.3-alpha.1+build.7");
        assert_eq!(x.pre, vec![id_alpha("alpha"), Identifier::Numeric(1)]);
        assert_eq!(x.build, vec!["build".to_string(), "7".to_string()]);
    }

    #[test]
    fn parse_rejects_garbage() {
        for s in [
            "", "1", "1.2", "1.2.3.4", "1.2.x", "a.b.c", "01.2.3", "1.2.3-", "1.2.3-01",
            "1.2.3-béta", "1.2.3+", "-1.2.3",
        ] {
            assert!(Version::parse(s).is_err(), "should reject {s:?}");
        }
    }

    #[test]
    fn display_roundtrip() {
        for s in [
            "0.0.0",
            "1.2.3",
            "10.20.30",
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-0.3.7",
            "1.0.0-x-y-z.1",
            "1.0.0-alpha+001",
            "1.0.0+20130313144700",
            "1.0.0-beta+exp.sha.5114f85",
        ] {
            let parsed = v(s);
            assert_eq!(parsed.to_string(), s, "roundtrip {s}");
        }
    }

    #[test]
    fn build_ignored_in_eq() {
        assert_eq!(v("1.2.3+a"), v("1.2.3+b"));
        assert_eq!(v("1.2.3"), v("1.2.3+meta"));
    }

    #[test]
    fn ordering_core() {
        assert!(v("1.0.0") < v("2.0.0"));
        assert!(v("2.0.0") < v("2.1.0"));
        assert!(v("2.1.0") < v("2.1.1"));
        assert!(v("1.0.0") < v("1.0.1"));
    }

    #[test]
    fn ordering_prerelease_spec_example() {
        // From semver.org §11.
        assert!(v("1.0.0-alpha") < v("1.0.0-alpha.1"));
        assert!(v("1.0.0-alpha.1") < v("1.0.0-alpha.beta"));
        assert!(v("1.0.0-alpha.beta") < v("1.0.0-beta"));
        assert!(v("1.0.0-beta") < v("1.0.0-beta.2"));
        assert!(v("1.0.0-beta.2") < v("1.0.0-beta.11"));
        assert!(v("1.0.0-beta.11") < v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.1") < v("1.0.0"));
    }

    #[test]
    fn ordering_prompt_example() {
        // Exactly the chain named in the task.
        assert!(v("1.0.0-alpha") < v("1.0.0-alpha.1"));
        assert!(v("1.0.0-alpha.1") < v("1.0.0-beta"));
        assert!(v("1.0.0-beta") < v("1.0.0"));
    }

    #[test]
    fn numeric_pre_below_alpha() {
        // Numeric identifiers always rank below alphanumeric ones.
        assert!(v("1.0.0-1") < v("1.0.0-alpha"));
        assert!(v("1.0.0-99") < v("1.0.0-a"));
    }

    fn id_alpha(s: &str) -> Identifier {
        Identifier::AlphaNumeric(s.to_string())
    }

    // ---- sort_key monotonicity ----

    fn sample_versions() -> Vec<Version> {
        let raw = [
            "0.0.0",
            "0.0.1",
            "0.1.0",
            "0.1.0-alpha",
            "0.1.0-alpha.1",
            "0.1.0-alpha.beta",
            "0.1.0-beta",
            "0.1.0-beta.2",
            "0.1.0-beta.11",
            "0.1.0-rc.1",
            "1.0.0-1",
            "1.0.0-1.2",
            "1.0.0-2",
            "1.0.0-alpha",
            "1.0.0",
            "1.0.1",
            "1.2.3",
            "1.2.3+build",
            "2.0.0",
            "10.0.0",
            "1.10.0",
            "1.9.0",
            "100.200.300",
            "1.0.0-x.7.z.92",
            "1.0.0-x-y-z.1",
        ];
        raw.iter().map(|s| v(s)).collect()
    }

    #[test]
    fn sort_key_matches_ord_pairwise() {
        let list = sample_versions();
        for a in &list {
            for b in &list {
                let ka = a.sort_key();
                let kb = b.sort_key();
                assert_eq!(
                    ka.cmp(&kb),
                    a.cmp(b),
                    "sort_key mismatch for {a} vs {b}: key {ka:?} vs {kb:?}"
                );
            }
        }
    }

    #[test]
    fn sort_key_total_sort_matches_ord_sort() {
        let mut by_ord = sample_versions();
        let mut by_key = sample_versions();
        by_ord.sort();
        by_key.sort_by_key(|a| a.sort_key());
        // Equal versions (build-only diff) may swap, but the byte order must
        // never produce a strictly-different ordering.
        for (a, b) in by_ord.iter().zip(by_key.iter()) {
            assert_eq!(a.cmp(b), Ordering::Equal, "{a} vs {b}");
        }
    }

    #[test]
    fn sort_key_release_after_prerelease() {
        let pre = v("1.0.0-rc.1").sort_key();
        let rel = v("1.0.0").sort_key();
        assert!(pre < rel);
    }

    #[test]
    fn sort_key_pseudo_random_pairs() {
        // Deterministic LCG-driven generation to broaden coverage without deps.
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            state >> 33
        };
        let mut gen = || {
            let major = next() % 5;
            let minor = next() % 5;
            let patch = next() % 5;
            let mut s = format!("{major}.{minor}.{patch}");
            if next() % 2 == 0 {
                let kinds = ["alpha", "beta", "rc", "0", "1", "12"];
                let k = kinds[(next() % kinds.len() as u64) as usize];
                s.push('-');
                s.push_str(k);
                if next() % 2 == 0 {
                    s.push('.');
                    s.push_str(&(next() % 20).to_string());
                }
            }
            v(&s)
        };
        let list: Vec<Version> = (0..400).map(|_| gen()).collect();
        for a in &list {
            for b in &list {
                assert_eq!(a.sort_key().cmp(&b.sort_key()), a.cmp(b), "{a} vs {b}");
            }
        }
    }

    #[test]
    fn large_numeric_pre_falls_back_to_alpha() {
        // 20-digit number exceeds u64; must parse without panic.
        let x = v("1.0.0-99999999999999999999");
        assert!(matches!(x.pre[0], Identifier::AlphaNumeric(_)));
    }
}
