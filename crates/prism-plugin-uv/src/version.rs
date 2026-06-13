//! PEP 440 version semantics for PyPI.
//!
//! Python versions are NOT SemVer, so this is a self-contained parser + total
//! order rather than a delegation to [`prism_semver`]. A version is
//!
//! ```text
//! [N!]N(.N)*[{a|b|rc}N][.postN][.devN][+local]
//!   epoch  release  pre-release  post   dev   local
//! ```
//!
//! Ordering follows PEP 440 / the reference `packaging` library's comparison key:
//! compare epoch, then the release tuple (trailing zeros are insignificant), then
//! pre/post/dev, then the local segment. The key insight encoded below:
//! `1.0.dev1 < 1.0a1 < 1.0a1.post1 < 1.0 < 1.0.post1 < 1.0+local`.
//!
//! Parsing is permissive about spelling (accepts `alpha`/`beta`/`preview`, the
//! `-_.`-separator variants, and the implicit `-N` post form) and normalizes to
//! the canonical form; an unparseable string is an error, never a panic.

use prism_plugin_api::{ParseError, VersionOrdering, VersionScheme};

/// A parsed PEP 440 version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pep440 {
    epoch: u64,
    release: Vec<u64>,
    /// `(letter-rank, number)` — a=0, b=1, rc=2.
    pre: Option<(u8, u64)>,
    post: Option<u64>,
    dev: Option<u64>,
    local: Vec<LocalSeg>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum LocalSeg {
    // PEP 440: an alphabetic local segment sorts BEFORE a numeric one.
    Alpha(String),
    Num(u64),
}

impl Pep440 {
    /// Parse a raw version string, or `None` if it is not a valid PEP 440 version.
    pub(crate) fn parse(raw: &str) -> Option<Pep440> {
        let s = raw.trim().to_ascii_lowercase();
        let s = s.strip_prefix('v').unwrap_or(&s);
        if s.is_empty() {
            return None;
        }
        // Split off the local segment at the first '+'.
        let (main, local_str) = match s.split_once('+') {
            Some((m, l)) => (m, Some(l)),
            None => (s, None),
        };
        // Split off the epoch at '!'.
        let (epoch, rest) = match main.split_once('!') {
            Some((e, r)) => (parse_u64(e)?, r),
            None => (0, main),
        };

        let bytes = rest.as_bytes();
        let mut i = 0;
        let release = read_release(bytes, &mut i)?;
        let pre = try_pre(bytes, &mut i);
        let post = try_post(bytes, &mut i);
        let dev = try_dev(bytes, &mut i);
        if i != bytes.len() {
            return None; // trailing garbage
        }

        let local = match local_str {
            Some(l) => parse_local(l)?,
            None => Vec::new(),
        };
        Some(Pep440 {
            epoch,
            release,
            pre,
            post,
            dev,
            local,
        })
    }

    /// True if this is a pre-release or dev release (a post-release is NOT).
    pub(crate) fn is_prerelease(&self) -> bool {
        self.pre.is_some() || self.dev.is_some()
    }

    /// For `==X.*`: same epoch and this version's release begins with `prefix`'s
    /// release (missing trailing components count as zero).
    pub(crate) fn release_starts_with(&self, prefix: &Pep440) -> bool {
        self.epoch == prefix.epoch && release_prefix(&self.release, &prefix.release)
    }

    /// PEP 440 `==` equality: when the target declares no local segment, this
    /// version's local segment is ignored (`==1.0` matches `1.0+local`).
    pub(crate) fn equals_for_eq(&self, target: &Pep440) -> bool {
        if target.local.is_empty() {
            let mut me = self.clone();
            me.local.clear();
            me.cmp(target) == std::cmp::Ordering::Equal
        } else {
            self.cmp(target) == std::cmp::Ordering::Equal
        }
    }

    /// PEP 440 `~=` compatible-release prefix: same epoch and this release begins
    /// with the target's release minus its last component.
    pub(crate) fn compatible_with(&self, target: &Pep440) -> bool {
        if self.epoch != target.epoch {
            return false;
        }
        let cut = target.release.len().saturating_sub(1);
        release_prefix(&self.release, &target.release[..cut])
    }

    /// The comparison key (lexicographically == PEP 440 order).
    fn cmp_key(&self) -> CmpKey {
        // Release: drop trailing zeros so `1.0` == `1.0.0`.
        let mut rel = self.release.clone();
        while rel.len() > 1 && *rel.last().unwrap() == 0 {
            rel.pop();
        }
        // _pre: dev-only release sorts before any pre; a final/post release after.
        let pre = match self.pre {
            Some((letter, n)) => PreKey::Pre(letter, n),
            None if self.post.is_none() && self.dev.is_some() => PreKey::DevOnly,
            None => PreKey::FinalOrPost,
        };
        let post = match self.post {
            Some(n) => PostKey::Num(n),
            None => PostKey::NegInf,
        };
        let dev = match self.dev {
            Some(n) => DevKey::Num(n),
            None => DevKey::Inf,
        };
        let local = if self.local.is_empty() {
            LocalKey::NegInf
        } else {
            LocalKey::Segs(self.local.clone())
        };
        CmpKey {
            epoch: self.epoch,
            release: rel,
            pre,
            post,
            dev,
            local,
        }
    }

    /// An order-preserving byte key (lexicographic == [`Pep440`] order). Used to
    /// satisfy the [`VersionScheme::sort_key`] contract.
    fn sort_key_bytes(&self) -> Vec<u8> {
        let k = self.cmp_key();
        let mut out = Vec::with_capacity(32);
        out.extend_from_slice(&k.epoch.to_be_bytes());
        // Length-terminated u64 sequence: each element prefixed 0x01, end 0x00, so
        // a shorter prefix sorts before a longer one.
        for n in &k.release {
            out.push(0x01);
            out.extend_from_slice(&n.to_be_bytes());
        }
        out.push(0x00);
        match k.pre {
            PreKey::DevOnly => out.push(0),
            PreKey::Pre(l, n) => {
                out.push(1);
                out.push(l);
                out.extend_from_slice(&n.to_be_bytes());
            }
            PreKey::FinalOrPost => out.push(2),
        }
        match k.post {
            PostKey::NegInf => out.push(0),
            PostKey::Num(n) => {
                out.push(1);
                out.extend_from_slice(&n.to_be_bytes());
            }
        }
        match k.dev {
            DevKey::Num(n) => {
                out.push(0);
                out.extend_from_slice(&n.to_be_bytes());
            }
            DevKey::Inf => out.push(1),
        }
        match &k.local {
            LocalKey::NegInf => out.push(0),
            LocalKey::Segs(segs) => {
                out.push(1);
                for seg in segs {
                    match seg {
                        LocalSeg::Alpha(s) => {
                            out.push(0);
                            out.extend_from_slice(s.as_bytes());
                            // Terminate with a byte BELOW any local-segment byte
                            // (segments are `[a-z0-9]`, all >= 0x30) so a prefix
                            // string sorts before its extension: `abc` < `abcd`.
                            out.push(0x00);
                        }
                        LocalSeg::Num(n) => {
                            out.push(1);
                            out.extend_from_slice(&n.to_be_bytes());
                        }
                    }
                }
            }
        }
        out
    }

    /// Canonical normalized form, e.g. `1!2.0rc1.post3.dev4+local`.
    fn canonical(&self) -> String {
        let mut s = String::new();
        if self.epoch != 0 {
            s.push_str(&format!("{}!", self.epoch));
        }
        let rel: Vec<String> = self.release.iter().map(|n| n.to_string()).collect();
        s.push_str(&rel.join("."));
        if let Some((letter, n)) = self.pre {
            let l = match letter {
                0 => "a",
                1 => "b",
                _ => "rc",
            };
            s.push_str(&format!("{l}{n}"));
        }
        if let Some(n) = self.post {
            s.push_str(&format!(".post{n}"));
        }
        if let Some(n) = self.dev {
            s.push_str(&format!(".dev{n}"));
        }
        if !self.local.is_empty() {
            s.push('+');
            let segs: Vec<String> = self
                .local
                .iter()
                .map(|seg| match seg {
                    LocalSeg::Alpha(a) => a.clone(),
                    LocalSeg::Num(n) => n.to_string(),
                })
                .collect();
            s.push_str(&segs.join("."));
        }
        s
    }
}

impl PartialOrd for Pep440 {
    fn partial_cmp(&self, other: &Self) -> Option<VersionOrdering> {
        Some(self.cmp(other))
    }
}
impl Ord for Pep440 {
    fn cmp(&self, other: &Self) -> VersionOrdering {
        self.cmp_key().cmp(&other.cmp_key())
    }
}

/// The PEP 440 comparison key; derived `Ord` realizes the spec's ordering.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct CmpKey {
    epoch: u64,
    release: Vec<u64>,
    pre: PreKey,
    post: PostKey,
    dev: DevKey,
    local: LocalKey,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum PreKey {
    /// A dev release with no pre — sorts before any pre-release (NegativeInfinity).
    DevOnly,
    /// An actual pre-release `(letter-rank, number)`.
    Pre(u8, u64),
    /// A final or post release — sorts after every pre-release (Infinity).
    FinalOrPost,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum PostKey {
    /// No post segment (NegativeInfinity) — sorts before any post.
    NegInf,
    Num(u64),
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum DevKey {
    Num(u64),
    /// No dev segment (Infinity) — sorts after any dev.
    Inf,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum LocalKey {
    /// No local segment (NegativeInfinity) — sorts before any local.
    NegInf,
    Segs(Vec<LocalSeg>),
}

// --- parsing helpers --------------------------------------------------------

fn parse_u64(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    s.parse::<u64>().ok()
}

/// True if `rel` (zero-padded) begins with every component of `prefix`.
fn release_prefix(rel: &[u64], prefix: &[u64]) -> bool {
    prefix
        .iter()
        .enumerate()
        .all(|(i, p)| rel.get(i).copied().unwrap_or(0) == *p)
}

/// Read `\d+(\.\d+)*` from `bytes` at `*i`, advancing past it.
fn read_release(bytes: &[u8], i: &mut usize) -> Option<Vec<u64>> {
    let mut out = Vec::new();
    let first = read_number(bytes, i)?;
    out.push(first);
    // Continue while a '.' is followed by another digit group.
    while *i < bytes.len() && bytes[*i] == b'.' && i_plus1_is_digit(bytes, *i) {
        *i += 1; // consume '.'
        out.push(read_number(bytes, i)?);
    }
    Some(out)
}

fn i_plus1_is_digit(bytes: &[u8], i: usize) -> bool {
    bytes.get(i + 1).is_some_and(u8::is_ascii_digit)
}

/// Read a run of ASCII digits as a u64, or `None` if none/overflow.
fn read_number(bytes: &[u8], i: &mut usize) -> Option<u64> {
    let start = *i;
    while *i < bytes.len() && bytes[*i].is_ascii_digit() {
        *i += 1;
    }
    if *i == start {
        return None;
    }
    std::str::from_utf8(&bytes[start..*i]).ok()?.parse().ok()
}

/// Consume one optional `-`/`_`/`.` separator.
fn skip_sep(bytes: &[u8], i: &mut usize) {
    if *i < bytes.len() && matches!(bytes[*i], b'-' | b'_' | b'.') {
        *i += 1;
    }
}

/// Try to read a pre-release segment. On no match, `*i` is left unchanged.
fn try_pre(bytes: &[u8], i: &mut usize) -> Option<(u8, u64)> {
    let mut j = *i;
    skip_sep(bytes, &mut j);
    // Longest label first so `alpha`/`beta`/`preview`/`pre` win over `a`/`b`.
    let labels: &[(&str, u8)] = &[
        ("alpha", 0),
        ("beta", 1),
        ("preview", 2),
        ("pre", 2),
        ("rc", 2),
        ("a", 0),
        ("b", 1),
        ("c", 2),
    ];
    let (rank, len) = match_label(bytes, j, labels)?;
    j += len;
    skip_sep(bytes, &mut j);
    let n = read_number(bytes, &mut j).unwrap_or(0);
    *i = j;
    Some((rank, n))
}

/// Try to read a post-release segment (label form or implicit `-N`).
fn try_post(bytes: &[u8], i: &mut usize) -> Option<u64> {
    // Implicit post: `-` directly followed by digits.
    if bytes.get(*i) == Some(&b'-') && i_plus1_is_digit(bytes, *i) {
        let mut j = *i + 1;
        let n = read_number(bytes, &mut j)?;
        *i = j;
        return Some(n);
    }
    let mut j = *i;
    skip_sep(bytes, &mut j);
    let labels: &[(&str, u8)] = &[("post", 0), ("rev", 0), ("r", 0)];
    let (_r, len) = match_label(bytes, j, labels)?;
    j += len;
    skip_sep(bytes, &mut j);
    let n = read_number(bytes, &mut j).unwrap_or(0);
    *i = j;
    Some(n)
}

/// Try to read a dev segment.
fn try_dev(bytes: &[u8], i: &mut usize) -> Option<u64> {
    let mut j = *i;
    skip_sep(bytes, &mut j);
    let labels: &[(&str, u8)] = &[("dev", 0)];
    let (_r, len) = match_label(bytes, j, labels)?;
    j += len;
    skip_sep(bytes, &mut j);
    let n = read_number(bytes, &mut j).unwrap_or(0);
    *i = j;
    Some(n)
}

/// Match the first label whose text is a prefix of `bytes[at..]`, returning its
/// `(rank, byte-length)`. Labels must be tried longest-first by the caller.
fn match_label(bytes: &[u8], at: usize, labels: &[(&str, u8)]) -> Option<(u8, usize)> {
    for (text, rank) in labels {
        let end = at + text.len();
        if end <= bytes.len() && &bytes[at..end] == text.as_bytes() {
            return Some((*rank, text.len()));
        }
    }
    None
}

/// Parse a local version into its dot/`-`/`_`-separated segments.
fn parse_local(s: &str) -> Option<Vec<LocalSeg>> {
    if s.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for part in s.split(['.', '-', '_']) {
        if part.is_empty() {
            return None;
        }
        // A run of pure digits is numeric; otherwise alphanumeric (kept as-is).
        if part.bytes().all(|b| b.is_ascii_digit()) {
            out.push(LocalSeg::Num(part.parse().ok()?));
        } else {
            out.push(LocalSeg::Alpha(part.to_string()));
        }
    }
    Some(out)
}

/// PEP 440 [`VersionScheme`].
pub struct UvVersions;

impl UvVersions {
    fn parse440(raw: &str) -> Result<Pep440, ParseError> {
        Pep440::parse(raw)
            .ok_or_else(|| ParseError::new("bad-version", format!("not a PEP 440 version: {raw}")))
    }
}

impl VersionScheme for UvVersions {
    fn parse(&self, raw: &str) -> Result<String, ParseError> {
        Ok(Self::parse440(raw)?.canonical())
    }

    fn compare(&self, a: &str, b: &str) -> Result<VersionOrdering, ParseError> {
        Ok(Self::parse440(a)?.cmp(&Self::parse440(b)?))
    }

    fn sort_key(&self, raw: &str) -> Result<Vec<u8>, ParseError> {
        Ok(Self::parse440(raw)?.sort_key_bytes())
    }

    fn is_prerelease(&self, raw: &str) -> Result<bool, ParseError> {
        Ok(Self::parse440(raw)?.is_prerelease())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    fn v(s: &str) -> Pep440 {
        Pep440::parse(s).unwrap_or_else(|| panic!("parse {s}"))
    }

    #[test]
    fn parses_canonical_forms() {
        let vs = UvVersions;
        assert_eq!(vs.parse("1.2.3").unwrap(), "1.2.3");
        assert_eq!(vs.parse("v1.0").unwrap(), "1.0");
        assert_eq!(vs.parse("1.0ALPHA1").unwrap(), "1.0a1");
        assert_eq!(vs.parse("1.0-rc-2").unwrap(), "1.0rc2");
        assert_eq!(vs.parse("1.0.post0").unwrap(), "1.0.post0");
        assert_eq!(vs.parse("1.0-1").unwrap(), "1.0.post1");
        assert_eq!(vs.parse("1!2.0").unwrap(), "1!2.0");
        assert_eq!(vs.parse("1.0+ubuntu.1").unwrap(), "1.0+ubuntu.1");
        assert_eq!(vs.parse("1.0.dev0").unwrap(), "1.0.dev0");
        assert!(vs.parse("not-a-version").is_err());
        assert!(vs.parse("").is_err());
    }

    #[test]
    fn pep440_ordering_examples() {
        // The canonical PEP 440 ordering chain:
        //   1.0.dev1 < 1.0a1.dev1 < 1.0a1 < 1.0b1 < 1.0rc1 < 1.0 < 1.0.post1 < 1.0+local
        assert_eq!(v("1.0.dev1").cmp(&v("1.0a1")), Ordering::Less);
        assert_eq!(v("1.0a1.dev1").cmp(&v("1.0a1")), Ordering::Less);
        assert_eq!(v("1.0a1").cmp(&v("1.0b1")), Ordering::Less);
        assert_eq!(v("1.0b1").cmp(&v("1.0rc1")), Ordering::Less);
        assert_eq!(v("1.0rc1").cmp(&v("1.0")), Ordering::Less);
        assert_eq!(v("1.0").cmp(&v("1.0.post1")), Ordering::Less);
        assert_eq!(v("1.0").cmp(&v("1.0+local")), Ordering::Less);
        // Trailing zeros are insignificant.
        assert_eq!(v("1.0").cmp(&v("1.0.0")), Ordering::Equal);
        // Numeric (not lexical) release ordering.
        assert_eq!(v("1.9").cmp(&v("1.10")), Ordering::Less);
        // Epoch dominates.
        assert_eq!(v("1!1.0").cmp(&v("2.0")), Ordering::Greater);
        // Post beats dev within the same release.
        assert_eq!(v("1.0.post1.dev1").cmp(&v("1.0.post1")), Ordering::Less);
    }

    #[test]
    fn sort_key_agrees_with_compare() {
        let raw = [
            "1.0.dev1", "1.0a1.dev1", "1.0a1", "1.0b1", "1.0rc1", "1.0", "1.0.post1", "1.1",
            "2.0", "1!0.1",
            // Local versions, including alpha-prefix pairs that a bad terminator
            // would mis-order (`abc` must sort before `abcd`).
            "1.0+abc", "1.0+abcd", "1.0+abc.1", "1.0+1", "1.0+2",
        ];
        let mut by_cmp = raw.to_vec();
        by_cmp.sort_by_key(|&x| v(x));
        let mut by_key = raw.to_vec();
        by_key.sort_by_key(|&x| v(x).sort_key_bytes());
        assert_eq!(by_cmp, by_key, "byte key order must match compare order");
    }

    #[test]
    fn prerelease_detection() {
        let vs = UvVersions;
        assert!(vs.is_prerelease("1.0a1").unwrap());
        assert!(vs.is_prerelease("1.0.dev1").unwrap());
        assert!(vs.is_prerelease("1.0rc1.dev2").unwrap());
        assert!(!vs.is_prerelease("1.0").unwrap());
        assert!(!vs.is_prerelease("1.0.post1").unwrap()); // post is NOT a prerelease
    }
}
