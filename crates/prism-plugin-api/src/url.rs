//! [`SafeUrl`]: the only URL form permitted on the fetch path.
//!
//! Per the architecture doc (*Credential handling*, Rule 8), credentials must
//! never enter any store file, provenance record, log, or UI response. `SafeUrl`
//! enforces this structurally: it strips `userinfo` (the `user:password@` part)
//! at construction, and its `Display`/`Debug` impls can only ever emit the
//! redacted form. It is the one URL type allowed in [`crate::RawDocument`],
//! [`crate::Provenance`], and [`crate::RegistrySource`].
//!
//! This is a *lexical* sanitizer, not a network policy: SSRF defense (DNS
//! resolution, address allowlisting, redirect pinning) lives in the fetcher
//! implementation, not in this pure type.

/// A URL string with any `userinfo` (credentials) stripped at construction.
///
/// The inner string is never exposed in a form that could leak credentials,
/// because credentials are removed before storage.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SafeUrl(String);

impl SafeUrl {
    /// Build a `SafeUrl`, stripping any `user[:password]@` authority prefix.
    ///
    /// The scheme and host are preserved; only the credential segment between
    /// `scheme://` and the host is removed. Inputs without an authority (e.g.
    /// `mailto:` or relative fragments) pass through unchanged except for the
    /// credential strip, which is a no-op when no `@` precedes the path.
    pub fn new(raw: impl AsRef<str>) -> Self {
        Self(strip_userinfo(raw.as_ref()))
    }

    /// The redacted URL string.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume into the redacted `String`.
    #[inline]
    pub fn into_string(self) -> String {
        self.0
    }
}

/// Remove any `user[:password]@` userinfo from a URL's authority.
///
/// This is a *redaction* guard, so it errs toward over-stripping: it removes
/// userinfo from every shape an `@`-before-the-path can take — `scheme://u:p@h`,
/// protocol-relative `//u:p@h`, and bare `u:p@h/path` — and treats both `/` and
/// `\` as authority terminators (WHATWG normalizes `\` to `/` for special
/// schemes, so a `host\path@evil` confusion must not survive). An `@` inside a
/// path/query/fragment (after the first delimiter) is preserved.
fn strip_userinfo(url: &str) -> String {
    // Where does the authority begin?
    //  - after "://" for a normal absolute URL,
    //  - after a leading "//" for a protocol-relative URL,
    //  - at the start for a bare `userinfo@host/...` (no scheme).
    let auth_start = if let Some(sep) = url.find("://") {
        sep + 3
    } else if url.starts_with("//") {
        2
    } else {
        0
    };
    let rest = &url[auth_start..];

    // The authority ends at the first '/', '\', '?', or '#'.
    let auth_end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..auth_end];

    // Strip up to and including the LAST '@' in the authority (so the redacted
    // host can never be a userinfo-confused decoy). For the bare/no-scheme case
    // this may also drop a `scheme:` prefix — acceptable: never leaking a
    // credential matters more than preserving an unusual schemed form.
    match authority.rfind('@') {
        Some(at) => {
            let mut out = String::with_capacity(url.len());
            out.push_str(&url[..auth_start]);
            out.push_str(&authority[at + 1..]);
            out.push_str(&rest[auth_end..]);
            out
        }
        None => url.to_string(),
    }
}

impl core::fmt::Display for SafeUrl {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::fmt::Debug for SafeUrl {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "SafeUrl({:?})", self.0)
    }
}

impl From<&str> for SafeUrl {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for SafeUrl {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_user_and_password() {
        let u = SafeUrl::new("https://user:token@registry.example/pkg/foo");
        assert_eq!(u.as_str(), "https://registry.example/pkg/foo");
        assert!(!u.as_str().contains("user"));
        assert!(!u.as_str().contains("token"));
    }

    #[test]
    fn strips_user_only() {
        let u = SafeUrl::new("https://alice@host/path");
        assert_eq!(u.as_str(), "https://host/path");
    }

    #[test]
    fn preserves_url_without_credentials() {
        let u = SafeUrl::new("https://registry.example/a/b?x=1#frag");
        assert_eq!(u.as_str(), "https://registry.example/a/b?x=1#frag");
    }

    #[test]
    fn at_sign_in_path_is_not_userinfo() {
        // npm scoped tarballs can contain '@' in the path; it must survive.
        let u = SafeUrl::new("https://registry.npmjs.org/@scope/pkg/-/pkg-1.0.0.tgz");
        assert_eq!(
            u.as_str(),
            "https://registry.npmjs.org/@scope/pkg/-/pkg-1.0.0.tgz"
        );
    }

    #[test]
    fn at_sign_in_query_is_not_userinfo() {
        let u = SafeUrl::new("https://host/path?email=a@b.com");
        assert_eq!(u.as_str(), "https://host/path?email=a@b.com");
    }

    #[test]
    fn debug_and_display_never_leak() {
        let u = SafeUrl::new("https://user:secret@host/x");
        let d = format!("{u:?}");
        let s = format!("{u}");
        assert!(!d.contains("secret"));
        assert!(!s.contains("secret"));
        assert!(!d.contains("user:"));
    }

    #[test]
    fn non_authority_url_passes_through() {
        let u = SafeUrl::new("file:relative/path");
        assert_eq!(u.as_str(), "file:relative/path");
    }

    #[test]
    fn strips_userinfo_from_schemeless_and_protocol_relative_forms() {
        // Protocol-relative: the credential before '@' must be removed, '//' kept.
        assert_eq!(SafeUrl::new("//user:tok@host/path").as_str(), "//host/path");
        // Bare schemeless authority.
        assert_eq!(SafeUrl::new("user:tok@host/path").as_str(), "host/path");
        // None of these leak the token.
        for s in ["//u:tok@h/p", "u:tok@h/p", "https:u:tok@h/p"] {
            assert!(!SafeUrl::new(s).as_str().contains("tok"), "leaked: {s}");
        }
    }

    #[test]
    fn backslash_is_an_authority_terminator() {
        // WHATWG treats '\' like '/', so the authority is `good.com` and the rest
        // (`\path@evil.com`) is path data, not userinfo. The old code (which did
        // not stop at '\') would have mis-stripped to host `evil.com`; with '\' as
        // a terminator the authority stays `good.com` and the decoy `@evil.com`
        // never becomes the host.
        let u = SafeUrl::new("https://good.com\\path@evil.com/");
        assert!(
            u.as_str().starts_with("https://good.com\\"),
            "authority must remain good.com: {}",
            u.as_str()
        );
        // A genuine userinfo BEFORE the '\' is still stripped.
        let v = SafeUrl::new("https://user:tok@good.com\\path");
        assert_eq!(v.as_str(), "https://good.com\\path");
    }
}
