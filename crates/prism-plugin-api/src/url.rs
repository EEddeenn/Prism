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

/// Remove the `user[:password]@` segment of an `scheme://userinfo@host/...` URL.
///
/// Only the authority component (the part after `://` and before the first `/`,
/// `?`, or `#`) is inspected, so an `@` inside a path or query is preserved.
fn strip_userinfo(url: &str) -> String {
    // Locate the authority: everything after the scheme separator "://".
    let Some(sep) = url.find("://") else {
        return url.to_string();
    };
    let auth_start = sep + 3;
    let rest = &url[auth_start..];

    // The authority ends at the first '/', '?', or '#'.
    let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..auth_end];

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
}
