//! # Scope Blockchain Analysis
//!
//! A command-line tool and library for blockchain data analysis,
//! portfolio tracking, and transaction investigation.
//!
//! ## Features
//!
//! - **Address Analysis**: Query balances (with USD valuation), transaction history,
//!   and token holdings (ERC-20, SPL, TRC-20) for blockchain addresses across
//!   multiple chains. Chain auto-detection from address format.
//!
//! - **Transaction Analysis**: Look up and decode blockchain transactions across
//!   EVM chains (via Etherscan proxy API), Solana (via `getTransaction` RPC),
//!   and Tron (via TronGrid). Includes receipt data, gas usage, and status.
//!
//! - **Token Crawling**: Crawl DEX data for any token with price, volume,
//!   liquidity, holder analysis, and risk scoring. Markdown report generation.
//!
//! - **Live Monitoring**: Real-time TUI dashboard with price/volume/candlestick
//!   charts, buy/sell gauges, activity logs, and Unicode-rich visualization.
//!
//! - **Portfolio Management**: Track multiple addresses across chains with
//!   labels, tags, and aggregated balance views including ERC-20, SPL, and
//!   TRC-20 token balances.
//!
//! - **Data Export**: Export transaction history in JSON or CSV with date range
//!   filtering. Chain auto-detection for addresses.
//!
//! - **USD Valuation**: Native token prices via DexScreener for all supported
//!   chains (ETH, SOL, BNB, MATIC, etc.).
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
//! scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2
//!
//! # Analyze a transaction
//! scope tx 0xabc123...
//!
//! # Manage portfolio
//! scope portfolio add 0x742d... --label "Main Wallet"
//! scope portfolio list
//!
//! # Export data
//! scope export --address 0x742d... --output history.json
//! ```
//!
//! ## Library Usage
//!
//! The Scope library can be used programmatically in your Rust applications:
//!
//! ```rust,no_run
//! use scope::{Config, chains::EthereumClient};
//!
//! #[tokio::main]
//! async fn main() -> scope::Result<()> {
//!     // Load configuration
//!     let config = Config::load(None)?;
//!     
//!     // Create a chain client
//!     let client = EthereumClient::new(&config.chains)?;
//!     
//!     // Query an address balance (with USD valuation)
//!     let mut balance = client.get_balance("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2").await?;
//!     client.enrich_balance_usd(&mut balance).await;
//!     println!("Balance: {} (${:.2})", balance.formatted, balance.usd_value.unwrap_or(0.0));
//!     
//!     // Look up a transaction
//!     let tx = client.get_transaction("0xabc123...").await?;
//!     println!("From: {}, Status: {:?}", tx.from, tx.status);
//!     
//!     // Fetch ERC-20 token balances
//!     let tokens = client.get_erc20_balances("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2").await?;
//!     for token in &tokens {
//!         println!("{}: {}", token.token.symbol, token.formatted_balance);
//!     }
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## Configuration
//!
//! Scope reads configuration from `~/.config/scope/config.yaml`:
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
//!   data_dir: "~/.local/share/scope"
//! ```
//!
//! ## Error Handling
//!
//! All fallible operations return [`Result<T>`], which uses [`ScopeError`]
//! as the error type. This provides detailed error context for debugging
//! and user-friendly error messages.
//!
//! ```rust
//! use scope::{ScopeError, Result};
//!
//! fn validate_address(addr: &str) -> Result<()> {
//!     if !addr.starts_with("0x") || addr.len() != 42 {
//!         return Err(ScopeError::InvalidAddress(addr.to_string()));
//!     }
//!     Ok(())
//! }
//! ```
//!
//! ## Modules
//!
//! - [`chains`]: Blockchain client implementations (Ethereum/EVM, Solana, Tron, DexScreener)
//! - [`cli`]: Command-line interface definitions (address, tx, crawl, monitor, portfolio, export)
//! - [`config`]: Configuration management
//! - [`display`]: Terminal output utilities and markdown report generation
//! - [`error`]: Error types and result aliases
//! - [`tokens`]: Token alias storage for friendly name lookups

// Re-export commonly used types at crate root
pub use config::Config;
pub use error::{ConfigError, Result, ScopeError};

/// Blockchain client implementations.
///
/// Provides abstractions and concrete clients for interacting with
/// blockchain networks. See `ChainClient` for the common
/// interface and `EthereumClient` for EVM chain support.
pub mod chains;

/// Command-line interface definitions.
///
/// Contains argument structures and command handlers for the Scope CLI.
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
/// Defines [`ScopeError`] for all error conditions and provides a
/// convenient [`Result`] type alias.
pub mod error;

/// Token alias storage for saving token lookups.
///
/// Allows users to reference tokens by friendly names instead
/// of full contract addresses.
pub mod tokens;

/// Compliance and risk analysis module.
///
/// Provides risk scoring, transaction taint analysis, pattern detection,
/// and compliance reporting for blockchain addresses.
pub mod compliance;

/// Library version string.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Returns the library version.
///
/// # Examples
///
/// ```rust
/// let version = scope::version();
/// println!("Scope version: {}", version);
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
        let err = ScopeError::InvalidAddress("test".to_string());
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
