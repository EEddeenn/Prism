//! Stable process exit codes and the [`PrismError`] → code mapping.
//!
//! Codes are part of the CLI contract (architecture doc, *CLI Design* — each
//! [`PrismError`] variant maps to a stable code). Scripts depend on them, so they
//! are fixed here and exercised by tests.

use prism_model::PrismError;

/// Success.
pub const OK: i32 = 0;
/// A usage error: bad arguments, unknown subcommand, missing flag.
pub const USAGE: i32 = 2;
/// A requested record/run/node/pair was not found.
pub const NOT_FOUND: i32 = 3;
/// Stored data was corrupt or failed a checksum/format check.
pub const CORRUPT: i32 = 4;
/// A decode bounds check failed on untrusted bytes.
pub const OUT_OF_BOUNDS: i32 = 5;
/// An underlying I/O failure.
pub const IO: i32 = 6;
/// Any other operational failure (e.g. an unsupported ecosystem/filtration).
pub const OTHER: i32 = 1;

/// Map a [`PrismError`] to its stable CLI exit code.
pub fn code_for(err: &PrismError) -> i32 {
    match err {
        PrismError::NotFound => NOT_FOUND,
        PrismError::Corrupt { .. }
        | PrismError::ChecksumMismatch { .. }
        | PrismError::UnsupportedFormatVersion { .. } => CORRUPT,
        PrismError::OutOfBounds { .. } => OUT_OF_BOUNDS,
        PrismError::Io(_) => IO,
        PrismError::Other(_) => OTHER,
        // `PrismError` is `#[non_exhaustive]`: an unforeseen variant is a generic
        // operational failure.
        _ => OTHER,
    }
}

/// A friendly one-line message for `stderr`, prefixed with `prism: `.
pub fn message_for(err: &PrismError) -> String {
    match err {
        PrismError::NotFound => "not found (no such run, node, or pair)".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_map_to_stable_codes() {
        assert_eq!(code_for(&PrismError::NotFound), NOT_FOUND);
        assert_eq!(
            code_for(&PrismError::Corrupt {
                detail: "x".into()
            }),
            CORRUPT
        );
        assert_eq!(
            code_for(&PrismError::ChecksumMismatch {
                detail: "x".into()
            }),
            CORRUPT
        );
        assert_eq!(
            code_for(&PrismError::UnsupportedFormatVersion { found: 9, max: 1 }),
            CORRUPT
        );
        assert_eq!(
            code_for(&PrismError::OutOfBounds {
                detail: "x".into()
            }),
            OUT_OF_BOUNDS
        );
        let io = PrismError::Io(std::io::Error::other("boom"));
        assert_eq!(code_for(&io), IO);
        assert_eq!(code_for(&PrismError::Other("nope".into())), OTHER);
    }

    #[test]
    fn message_is_friendly_for_not_found() {
        assert!(message_for(&PrismError::NotFound).contains("no such run"));
        assert_eq!(message_for(&PrismError::Other("hi".into())), "hi");
    }
}
