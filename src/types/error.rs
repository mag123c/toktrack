//! Error types for toktrack

use thiserror::Error;

/// toktrack error types
#[derive(Error, Debug)]
pub enum ToktrackError {
    /// Failed to parse JSON/JSONL
    #[error("parse error: {0}")]
    Parse(String),

    /// File I/O error
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Cache operation failed
    #[error("cache error: {0}")]
    Cache(String),

    /// Pricing fetch failed
    #[error("pricing error: {0}")]
    Pricing(String),

    /// Configuration error
    #[error("config error: {0}")]
    #[allow(dead_code)]
    Config(String),
}

/// Result type alias for toktrack
pub type Result<T> = std::result::Result<T, ToktrackError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = ToktrackError::Parse("invalid json".into());
        assert_eq!(err.to_string(), "parse error: invalid json");
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: ToktrackError = io_err.into();
        assert!(err.to_string().contains("io error"));
    }
}
