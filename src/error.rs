//! # Error Handling Module
//!
//! This module defines the error types used throughout the BCC application.
//! It provides a unified error type [`BccError`] that wraps all possible
//! error conditions, along with a convenient [`Result`] type alias.
//!
//! ## Error Hierarchy
//!
//! ```text
//! BccError
//! ├── Config     - Configuration loading/parsing errors
//! ├── Chain      - Blockchain client errors
//! ├── Request    - HTTP/API request failures
//! ├── InvalidAddress - Malformed blockchain address
//! ├── InvalidHash    - Malformed transaction hash
//! ├── Io         - File system operations
//! └── Export     - Data export failures
//! ```
//!
//! ## Usage
//!
//! ```rust
//! use bcc::{BccError, Result};
//!
//! fn validate_address(addr: &str) -> Result<()> {
//!     if !addr.starts_with("0x") || addr.len() != 42 {
//!         return Err(BccError::InvalidAddress(addr.to_string()));
//!     }
//!     Ok(())
//! }
//! ```

use std::path::PathBuf;
use thiserror::Error;

/// The primary error type for the BCC application.
///
/// This enum encompasses all error conditions that can occur during
/// blockchain analysis operations. Each variant provides context-specific
/// information to aid in debugging and user-friendly error messages.
#[derive(Debug, Error)]
pub enum BccError {
    /// Configuration file could not be loaded or parsed.
    ///
    /// This includes missing files, invalid YAML syntax, or schema violations.
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    /// Blockchain client encountered an error.
    ///
    /// This covers RPC failures, unsupported chain operations, or protocol errors.
    #[error("Chain client error: {0}")]
    Chain(String),

    /// HTTP request to an external API failed.
    ///
    /// Wraps reqwest errors for network failures, timeouts, or HTTP error responses.
    #[error("API request failed: {0}")]
    Request(#[from] reqwest::Error),

    /// The provided blockchain address is malformed.
    ///
    /// Addresses must be valid for their respective chain (e.g., 0x-prefixed
    /// 40-character hex string for Ethereum).
    #[error("Invalid address format: {0}")]
    InvalidAddress(String),

    /// The provided transaction hash is malformed.
    ///
    /// Transaction hashes must be valid for their respective chain.
    #[error("Invalid transaction hash: {0}")]
    InvalidHash(String),

    /// File system operation failed.
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// File system operation failed (string variant).
    #[error("I/O error: {0}")]
    Io(String),

    /// Data export operation failed.
    #[error("Export failed: {0}")]
    Export(String),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Network operation failed.
    #[error("Network error: {0}")]
    Network(String),

    /// External API returned an error.
    #[error("API error: {0}")]
    Api(String),

    /// Resource not found.
    #[error("Not found: {0}")]
    NotFound(String),
}

/// Configuration-specific error type.
///
/// Provides detailed context for configuration failures, including
/// the file path and underlying cause.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Configuration file was not found at the expected path.
    #[error("Configuration file not found: {path}")]
    NotFound {
        /// The path where the config file was expected.
        path: PathBuf,
    },

    /// Failed to read the configuration file.
    #[error("Failed to read configuration file '{path}': {source}")]
    Read {
        /// The path to the configuration file.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// Configuration file contains invalid YAML.
    #[error("Failed to parse configuration: {source}")]
    Parse {
        /// The underlying YAML parsing error.
        #[source]
        source: serde_yaml::Error,
    },

    /// Configuration values failed validation.
    #[error("Invalid configuration: {message}")]
    Validation {
        /// Description of the validation failure.
        message: String,
    },
}

/// A specialized [`Result`] type for BCC operations.
///
/// This type alias reduces boilerplate by defaulting the error type
/// to [`BccError`].
///
/// # Examples
///
/// ```rust
/// use bcc::Result;
///
/// fn do_something() -> Result<String> {
///     Ok("success".to_string())
/// }
/// ```
pub type Result<T> = std::result::Result<T, BccError>;

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bca_error_display_invalid_address() {
        let err = BccError::InvalidAddress("not-an-address".into());
        assert_eq!(err.to_string(), "Invalid address format: not-an-address");
    }

    #[test]
    fn test_bca_error_display_invalid_hash() {
        let err = BccError::InvalidHash("bad-hash".into());
        assert_eq!(err.to_string(), "Invalid transaction hash: bad-hash");
    }

    #[test]
    fn test_bca_error_display_chain() {
        let err = BccError::Chain("connection refused".into());
        assert_eq!(err.to_string(), "Chain client error: connection refused");
    }

    #[test]
    fn test_bca_error_display_export() {
        let err = BccError::Export("failed to write CSV".into());
        assert_eq!(err.to_string(), "Export failed: failed to write CSV");
    }

    #[test]
    fn test_config_error_not_found() {
        let err = ConfigError::NotFound {
            path: PathBuf::from("/missing/config.yaml"),
        };
        assert_eq!(
            err.to_string(),
            "Configuration file not found: /missing/config.yaml"
        );
    }

    #[test]
    fn test_config_error_validation() {
        let err = ConfigError::Validation {
            message: "ethereum_rpc must be a valid URL".into(),
        };
        assert_eq!(
            err.to_string(),
            "Invalid configuration: ethereum_rpc must be a valid URL"
        );
    }

    #[test]
    fn test_bca_error_from_config_error() {
        let config_err = ConfigError::NotFound {
            path: PathBuf::from("/test"),
        };
        let bca_err: BccError = config_err.into();
        assert!(matches!(bca_err, BccError::Config(_)));
    }

    #[test]
    fn test_bca_error_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let bca_err: BccError = io_err.into();
        assert!(matches!(bca_err, BccError::IoError(_)));
    }

    #[test]
    fn test_result_type_alias_ok() {
        fn returns_ok() -> Result<i32> {
            Ok(42)
        }
        assert_eq!(returns_ok().unwrap(), 42);
    }

    #[test]
    fn test_result_type_alias_err() {
        fn returns_err() -> Result<i32> {
            Err(BccError::Chain("test error".into()))
        }
        assert!(returns_err().is_err());
    }

    #[test]
    fn test_error_debug_impl() {
        let err = BccError::InvalidAddress("0x123".into());
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("InvalidAddress"));
        assert!(debug_str.contains("0x123"));
    }

    #[test]
    fn test_config_error_source_chain() {
        use std::error::Error;

        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let config_err = ConfigError::Read {
            path: PathBuf::from("/etc/bca/config.yaml"),
            source: io_err,
        };

        // Verify the error chain works
        assert!(config_err.source().is_some());
    }
}
