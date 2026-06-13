//! Constructing the [`RawDocument`]s plugin parsers consume.
//!
//! Plugin manifest/lockfile parsers take a [`prism_plugin_api::RawDocument`]: the
//! bytes plus provenance. The facade reads local files, so it stamps a
//! `file://`-keyed document with a content hash and an empty (local, non-cached)
//! provenance. URLs go through [`SafeUrl`] so no credentials can leak.

use prism_codec::sha256;
use prism_plugin_api::{Provenance, RawDocument, SafeUrl};

/// Build a [`RawDocument`] for `bytes` keyed at the redacted `source_key` URL.
pub(crate) fn raw_document(source_key: &str, bytes: Vec<u8>) -> RawDocument {
    let content_hash = sha256(&bytes);
    let url = SafeUrl::new(source_key);
    RawDocument {
        source_key: url.clone(),
        media_type: None,
        content_hash,
        bytes,
        fetched_at_unix: 0,
        etag: None,
        last_modified: None,
        provenance: Provenance {
            source_id: "local".into(),
            retrieved_from: url,
            from_cache: false,
            snapshot: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_redacts() {
        let doc = raw_document("https://user:pw@host/Cargo.toml", b"hello".to_vec());
        assert_eq!(doc.bytes, b"hello");
        assert_eq!(doc.content_hash, sha256(b"hello"));
        // Credentials in the URL authority are stripped by SafeUrl.
        assert!(!doc.source_key.as_str().contains("pw"));
        assert!(doc.source_key.as_str().contains("host"));
    }
}
