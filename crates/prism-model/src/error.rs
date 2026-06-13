//! The shared public error type and `Result` alias for Prism.

/// The crate-wide result type.
pub type Result<T> = core::result::Result<T, PrismError>;

/// The single public error type returned across the Prism API surface.
///
/// `#[non_exhaustive]` so new variants (budget, security, plugin) can be added
/// without a breaking change, per the architecture doc's versioning policy.
#[non_exhaustive]
#[derive(Debug)]
pub enum PrismError {
    /// A record addressed by a known ID was not found.
    NotFound,
    /// A stored structure is structurally invalid.
    Corrupt {
        /// Human-readable detail.
        detail: String,
    },
    /// A content/whole-file checksum did not match.
    ChecksumMismatch {
        /// Human-readable detail (e.g. file name).
        detail: String,
    },
    /// An on-disk format version is outside the supported range.
    UnsupportedFormatVersion {
        /// The version found on disk.
        found: u32,
        /// The maximum version this binary understands.
        max: u32,
    },
    /// A bounds/limit check failed while decoding untrusted bytes.
    OutOfBounds {
        /// Human-readable detail.
        detail: String,
    },
    /// An underlying I/O failure.
    Io(std::io::Error),
    /// A fallback for errors that do not yet have a dedicated variant.
    Other(String),
}

impl core::fmt::Display for PrismError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PrismError::NotFound => write!(f, "not found"),
            PrismError::Corrupt { detail } => write!(f, "corrupt store data: {detail}"),
            PrismError::ChecksumMismatch { detail } => write!(f, "checksum mismatch: {detail}"),
            PrismError::UnsupportedFormatVersion { found, max } => {
                write!(f, "unsupported format version {found} (max supported {max})")
            }
            PrismError::OutOfBounds { detail } => write!(f, "decode out of bounds: {detail}"),
            PrismError::Io(e) => write!(f, "io error: {e}"),
            PrismError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for PrismError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PrismError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for PrismError {
    fn from(e: std::io::Error) -> Self {
        PrismError::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages() {
        assert_eq!(PrismError::NotFound.to_string(), "not found");
        assert_eq!(
            PrismError::UnsupportedFormatVersion { found: 5, max: 2 }.to_string(),
            "unsupported format version 5 (max supported 2)"
        );
    }

    #[test]
    fn io_error_converts_and_chains() {
        let io = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "boom");
        let e: PrismError = io.into();
        assert!(matches!(e, PrismError::Io(_)));
        assert!(std::error::Error::source(&e).is_some());
    }
}
