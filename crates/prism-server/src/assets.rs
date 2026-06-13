//! Static UI assets, baked into the binary with `include_str!`.
//!
//! Everything the page needs is served from `web/` — no CDN, no npm, no external
//! script or style. The binary is hermetic: it carries its own UI.

/// `web/index.html` — the single-page app shell.
pub const INDEX_HTML: &str = include_str!("../web/index.html");

/// `web/app.js` — the vanilla-JS graph explorer.
pub const APP_JS: &str = include_str!("../web/app.js");

/// `web/style.css` — the page styling.
pub const STYLE_CSS: &str = include_str!("../web/style.css");

/// Resolve a request path to a `(content_type, body)` static asset, if it is one
/// of the three served files. `/` maps to the HTML shell.
pub fn lookup(path: &str) -> Option<(&'static str, &'static str)> {
    match path {
        "/" | "/index.html" => Some(("text/html; charset=utf-8", INDEX_HTML)),
        "/app.js" => Some(("application/javascript; charset=utf-8", APP_JS)),
        "/style.css" => Some(("text/css; charset=utf-8", STYLE_CSS)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_serves_html() {
        let (ct, body) = lookup("/").unwrap();
        assert!(ct.starts_with("text/html"));
        assert!(body.contains("<html"));
    }

    #[test]
    fn js_and_css_are_served() {
        assert!(lookup("/app.js").unwrap().0.contains("javascript"));
        assert!(lookup("/style.css").unwrap().0.contains("text/css"));
    }

    #[test]
    fn unknown_static_path_is_none() {
        assert!(lookup("/secret").is_none());
        assert!(lookup("/api/status").is_none());
    }

    #[test]
    fn no_external_resource_fetches() {
        // The UI must be fully hermetic: no CDN/script/style/font fetches. The
        // SVG/XML namespace URI (`http://www.w3.org/...`) is an identifier, not a
        // network fetch, so we look only for resource-loading attributes that
        // point off-origin.
        for body in [INDEX_HTML, APP_JS, STYLE_CSS] {
            for needle in [
                "src=\"http",
                "src=\"//",
                "href=\"http",
                "href=\"//",
                "@import",
                "url(http",
                "//cdn",
                "fonts.googleapis",
            ] {
                assert!(
                    !body.contains(needle),
                    "asset must not fetch external resources (found {needle:?})"
                );
            }
        }
    }

    #[test]
    fn local_assets_are_self_referential() {
        // The shell links only the two same-origin assets this server serves.
        assert!(INDEX_HTML.contains("href=\"/style.css\""));
        assert!(INDEX_HTML.contains("src=\"/app.js\""));
    }
}
