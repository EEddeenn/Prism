//! Tiny constructors for the [`PrismError`] variants the bundle reader and
//! writer emit. No new error type is introduced: every failure funnels through
//! [`PrismError`] so the CLI's existing exit-code mapping (4/5/6) works for
//! `.prismbundle` errors unchanged.
//!
//! These helpers exist purely to keep call sites short and to keep the detail
//! strings consistent ("bundle: ..." prefix everywhere).

use prism_model::PrismError;

/// Build a [`PrismError::Corrupt`] with a `"bundle: "`-prefixed detail.
pub fn corrupt(detail: impl Into<String>) -> PrismError {
    PrismError::Corrupt {
        detail: format!("bundle: {}", detail.into()),
    }
}

/// Build a [`PrismError::ChecksumMismatch`] with a `"bundle: "`-prefixed detail.
pub fn checksum_mismatch(detail: impl Into<String>) -> PrismError {
    PrismError::ChecksumMismatch {
        detail: format!("bundle: {}", detail.into()),
    }
}

/// Build a [`PrismError::OutOfBounds`] with a `"bundle: "`-prefixed detail.
pub fn out_of_bounds(detail: impl Into<String>) -> PrismError {
    PrismError::OutOfBounds {
        detail: format!("bundle: {}", detail.into()),
    }
}

/// Build a [`PrismError::UnsupportedFormatVersion`].
///
/// `found` is the version read from disk; `max` is the maximum version this
/// binary understands (typically [`crate::format::BUNDLE_FORMAT_VERSION`]).
pub fn unsupported_format_version(found: u32, max: u32) -> PrismError {
    PrismError::UnsupportedFormatVersion { found, max }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_prefix_detail() {
        match corrupt("bad magic") {
            PrismError::Corrupt { detail } => assert_eq!(detail, "bundle: bad magic"),
            other => panic!("unexpected: {other:?}"),
        }
        match checksum_mismatch("trailer") {
            PrismError::ChecksumMismatch { detail } => assert_eq!(detail, "bundle: trailer"),
            other => panic!("unexpected: {other:?}"),
        }
        match out_of_bounds("toc overflow") {
            PrismError::OutOfBounds { detail } => assert_eq!(detail, "bundle: toc overflow"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn unsupported_version_carries_numbers() {
        match unsupported_format_version(2, 1) {
            PrismError::UnsupportedFormatVersion { found, max } => {
                assert_eq!(found, 2);
                assert_eq!(max, 1);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
