//! # Scope Blockchain Analysis
//!
//! A command-line tool and library for blockchain data analysis,
//! address book management, and transaction investigation.
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
//! - **Token Discovery**: Browse trending and boosted tokens from DexScreener
//!   (`scope discover`) — featured profiles, recent boosts, top boosts. No API key.
//!
//! - **Live Monitoring**: Real-time TUI dashboard with price/volume/candlestick
//!   charts, buy/sell gauges, activity logs, and Unicode-rich visualization.
//!
//! - **Address Book**: Track multiple addresses (wallets, contracts) across chains with
//!   labels, tags, and aggregated balance views including ERC-20, SPL, and
//!   TRC-20 token balances.
//!
//! - **Data Export**: Export transaction history in JSON or CSV with date range
//!   filtering. Chain auto-detection for addresses.
//!
//! - **Market Health**: Peg and order book health for stablecoin markets. Fetches
//!   level-2 depth from Binance, Biconomy, or DEX liquidity (Ethereum/Solana via DexScreener).
//!   Configurable health checks (peg safety, bid/ask ratio, depth thresholds).
//!
//! - **Token Health Suite**: Composite command (`scope token-health`) combining DEX analytics
//!   with optional market/order book summary. Venues: binance, biconomy, eth, solana.
//!
//! - **Unified Insights**: Auto-detect addresses, transaction hashes, or tokens
//!   and run the appropriate analyses (`scope insights`).
//!
//! - **Compliance & Risk**: Risk scoring, transaction pattern detection, taint
//!   analysis, and compliance reporting (`scope compliance`).
//!
//! - **Shell Completion**: Tab-completion for bash, zsh, and fish
//!   (`scope completions <shell>`).
//!
//! - **Progress Indicators**: Spinners and progress bars for all long-running
//!   operations. Non-TTY aware (hidden in pipes).
//!
//! - **Error Remediation**: Actionable hints for common errors (invalid address,
//!   missing config, network failures, auth issues).
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
//! # Auto-detect and analyze any target
//! scope insights 0xabc123...
//!
//! # Token DEX data
//! scope crawl USDC --chain ethereum
//!
//! # Manage address book
//! scope address-book add 0x742d... --label "Main Wallet"
//! scope address-book list
//!
//! # Export data
//! scope export --address 0x742d... --output history.json
//!
//! # Shell tab-completion
//! scope completions zsh > ~/.zfunc/_scope
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
//!   format: table  # table, json, csv, markdown
//!   color: true
//!
//! address_book:
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
//! - [`cli`]: Command-line interface definitions (address, tx, crawl, monitor, market, address-book, export)
//! - [`config`]: Configuration management
//! - [`market`]: Peg and order book health for stablecoin markets
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

/// Shared domain logic used by both CLI and web delivery mechanisms.
///
/// Contains core data types and business logic (e.g. address book
/// storage, label resolution) that are independent of any particular
/// delivery mechanism.
pub mod domain;

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

/// Smart contract analysis module.
///
/// Provides contract source retrieval, ABI decoding, proxy detection,
/// access control mapping, vulnerability scanning, DeFi protocol checks,
/// and external intelligence (GitHub linking, audit reports).
pub mod contract;

/// HTTP transport abstraction.
///
/// Provides a trait-based abstraction over HTTP clients, enabling
/// transparent switching between direct `reqwest` calls and the
/// Ghola sidecar proxy for stealth analysis.
pub mod http;

/// Market module for peg and order book health analysis.
///
/// Fetches level-2 order book data from exchange APIs and runs
/// configurable health checks for stablecoin markets.
pub mod market;

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
