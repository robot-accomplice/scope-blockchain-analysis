//! # BCC - Blockchain Crawler CLI
//!
//! A command-line tool and library for blockchain data analysis,
//! portfolio tracking, and transaction investigation.
//!
//! ## Features
//!
//! - **Address Analysis**: Query balances, transaction history, and token holdings
//!   for blockchain addresses across multiple EVM-compatible chains.
//!
//! - **Transaction Analysis**: Decode and trace blockchain transactions,
//!   including internal calls and contract interactions.
//!
//! - **Portfolio Management**: Track multiple addresses across chains with
//!   labels, tags, and aggregated balance views.
//!
//! - **Data Export**: Export analysis results in JSON, CSV, or formatted
//!   table output for further processing.
//!
//! ## Supported Chains
//!
//! ### EVM-Compatible
//!
//! - Ethereum Mainnet
//! - Polygon
//! - Arbitrum
//! - Optimism
//! - Base
//! - BSC (BNB Smart Chain)
//! - Aegis (Wraith)
//!
//! ### Non-EVM
//!
//! - Solana
//! - Tron
//!
//! ## Quick Start (CLI)
//!
//! ```bash
//! # Analyze an address
//! bca address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2
//!
//! # Analyze a transaction
//! bca tx 0xabc123...
//!
//! # Manage portfolio
//! bca portfolio add 0x742d... --label "Main Wallet"
//! bca portfolio list
//!
//! # Export data
//! bca export --address 0x742d... --output history.json
//! ```
//!
//! ## Library Usage
//!
//! The BCC library can be used programmatically in your Rust applications:
//!
//! ```rust,no_run
//! use bcc::{Config, chains::EthereumClient};
//!
//! #[tokio::main]
//! async fn main() -> bcc::Result<()> {
//!     // Load configuration
//!     let config = Config::load(None)?;
//!     
//!     // Create a chain client
//!     let client = EthereumClient::new(&config.chains)?;
//!     
//!     // Query an address balance
//!     let balance = client.get_balance("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2").await?;
//!     println!("Balance: {}", balance.formatted);
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## Configuration
//!
//! BCC reads configuration from `~/.config/bcc/config.yaml`:
//!
//! ```yaml
//! chains:
//!   # EVM chains
//!   ethereum_rpc: "https://mainnet.infura.io/v3/YOUR_KEY"
//!   bsc_rpc: "https://bsc-dataseed.binance.org"
//!   aegis_rpc: "http://localhost:8545"
//!
//!   # Non-EVM chains
//!   solana_rpc: "https://api.mainnet-beta.solana.com"
//!   tron_api: "https://api.trongrid.io"
//!
//!   api_keys:
//!     etherscan: "YOUR_ETHERSCAN_KEY"
//!     polygonscan: "YOUR_POLYGONSCAN_KEY"
//!     bscscan: "YOUR_BSCSCAN_KEY"
//!     solscan: "YOUR_SOLSCAN_KEY"
//!     tronscan: "YOUR_TRONSCAN_KEY"
//!
//! output:
//!   format: table  # table, json, csv
//!   color: true
//!
//! portfolio:
//!   data_dir: "~/.local/share/bcc"
//! ```
//!
//! ## Error Handling
//!
//! All fallible operations return [`Result<T>`], which uses [`BccError`]
//! as the error type. This provides detailed error context for debugging
//! and user-friendly error messages.
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
//!
//! ## Modules
//!
//! - [`chains`]: Blockchain client implementations
//! - [`cli`]: Command-line interface definitions
//! - [`config`]: Configuration management
//! - [`error`]: Error types and result aliases

// Re-export commonly used types at crate root
pub use config::Config;
pub use error::{BccError, ConfigError, Result};

/// Blockchain client implementations.
///
/// Provides abstractions and concrete clients for interacting with
/// blockchain networks. See `ChainClient` for the common
/// interface and `EthereumClient` for EVM chain support.
pub mod chains;

/// Command-line interface definitions.
///
/// Contains argument structures and command handlers for the BCC CLI.
/// This module is primarily used by the binary crate but is exposed
/// for programmatic CLI invocation. It provides the main `Cli` struct
/// and `Commands` enum that define all available commands.
pub mod cli;

/// Configuration management.
///
/// Handles loading, merging, and validation of configuration from
/// multiple sources (CLI args, environment variables, config files).
pub mod config;

/// Display utilities for terminal output and reports.
///
/// Provides ASCII chart rendering and markdown report generation
/// for token analytics data.
pub mod display;

/// Error types and result aliases.
///
/// Defines [`BccError`] for all error conditions and provides a
/// convenient [`Result`] type alias.
pub mod error;

/// Token alias storage for saving token lookups.
///
/// Allows users to reference tokens by friendly names instead
/// of full contract addresses.
pub mod tokens;

/// Library version string.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Returns the library version.
///
/// # Examples
///
/// ```rust
/// let version = bcc::version();
/// println!("BCC version: {}", version);
/// ```
pub fn version() -> &'static str {
    VERSION
}

// ============================================================================
// Integration Tests for Library API
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_not_empty() {
        assert!(!version().is_empty());
    }

    #[test]
    fn test_version_format() {
        // Version should be semver-like (X.Y.Z)
        let v = version();
        let parts: Vec<&str> = v.split('.').collect();
        assert!(parts.len() >= 2, "Version should have at least major.minor");
    }

    #[test]
    fn test_config_reexport() {
        // Verify Config is accessible from crate root
        let config = Config::default();
        assert!(config.chains.api_keys.is_empty());
    }

    #[test]
    fn test_error_reexport() {
        // Verify error types are accessible from crate root
        let err = BccError::InvalidAddress("test".to_string());
        assert!(err.to_string().contains("test"));
    }

    #[test]
    fn test_result_type_alias() {
        fn test_fn() -> Result<i32> {
            Ok(42)
        }
        assert_eq!(test_fn().unwrap(), 42);
    }

    #[test]
    fn test_config_error_reexport() {
        use std::path::PathBuf;
        let err = ConfigError::NotFound {
            path: PathBuf::from("/test"),
        };
        assert!(err.to_string().contains("/test"));
    }
}
