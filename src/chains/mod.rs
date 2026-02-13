//! # Blockchain Client Module
//!
//! This module provides abstractions and implementations for interacting
//! with various blockchain networks. It defines a common `ChainClient` trait
//! that all chain-specific implementations must satisfy.
//!
//! ## Capabilities
//!
//! All chain clients support:
//! - **Balance queries** with optional USD valuation via DexScreener
//! - **Transaction lookup** by hash/signature with full details
//! - **Transaction history** for addresses with pagination
//! - **Token balances** (ERC-20, SPL, TRC-20) for portfolio tracking
//!
//! ## Supported Chains
//!
//! ### EVM-Compatible Chains
//!
//! - **Ethereum** - Ethereum Mainnet (via Etherscan V2 API)
//! - **Polygon** - Polygon PoS
//! - **Arbitrum** - Arbitrum One
//! - **Optimism** - Optimism Mainnet
//! - **Base** - Base (Coinbase L2)
//! - **BSC** - BNB Smart Chain (Binance)
//!
//! ### Non-EVM Chains
//!
//! - **Solana** - Solana Mainnet (JSON-RPC with `jsonParsed` encoding)
//! - **Tron** - Tron Mainnet (TronGrid API, base58check address validation)
//!
//! ### DEX Data
//!
//! - **DexScreener** - Token prices, volume, liquidity, and trading data across all DEX pairs
//!
//! ## Usage
//!
//! ### Ethereum/EVM Client
//!
//! ```rust,no_run
//! use scope::chains::{ChainClient, EthereumClient};
//! use scope::Config;
//!
//! #[tokio::main]
//! async fn main() -> scope::Result<()> {
//!     let config = Config::load(None)?;
//!     let client = EthereumClient::new(&config.chains)?;
//!     
//!     let balance = client.get_balance("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2").await?;
//!     println!("Balance: {} ETH", balance.formatted);
//!     Ok(())
//! }
//! ```
//!
//! ### Solana Client
//!
//! ```rust,no_run
//! use scope::chains::SolanaClient;
//! use scope::Config;
//!
//! #[tokio::main]
//! async fn main() -> scope::Result<()> {
//!     let config = Config::load(None)?;
//!     let client = SolanaClient::new(&config.chains)?;
//!     
//!     let balance = client.get_balance("DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy").await?;
//!     println!("Balance: {} SOL", balance.formatted);
//!     Ok(())
//! }
//! ```
//!
//! ### Tron Client
//!
//! ```rust,no_run
//! use scope::chains::TronClient;
//! use scope::Config;
//!
//! #[tokio::main]
//! async fn main() -> scope::Result<()> {
//!     let config = Config::load(None)?;
//!     let client = TronClient::new(&config.chains)?;
//!     
//!     let balance = client.get_balance("TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf").await?;
//!     println!("Balance: {} TRX", balance.formatted);
//!     Ok(())
//! }
//! ```

pub mod dex;
pub mod ethereum;
pub mod solana;
pub mod tron;

pub use dex::{DexClient, DexDataSource, DiscoverToken, TokenSearchResult};
pub use ethereum::{ApiType, EthereumClient};
pub use solana::{SolanaClient, validate_solana_address, validate_solana_signature};
pub use tron::{TronClient, validate_tron_address, validate_tron_tx_hash};

use crate::error::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Trait defining common blockchain client operations.
///
/// All chain-specific clients must implement this trait to provide
/// a consistent interface for blockchain interactions.
///
/// ## Core Methods
///
/// Every implementation must provide: `chain_name`, `native_token_symbol`,
/// `get_balance`, `get_transaction`, `get_transactions`, `get_block_number`,
/// `enrich_balance_usd`, and `get_token_balances`.
///
/// ## Token Explorer Methods
///
/// The token-explorer methods (`get_token_info`, `get_token_holders`,
/// `get_token_holder_count`) have default implementations that return
/// "not supported" errors or empty results. Only chains with block-explorer
/// support for these endpoints (currently EVM chains) need to override them.
#[async_trait]
pub trait ChainClient: Send + Sync {
    /// Returns the name of the blockchain network.
    fn chain_name(&self) -> &str;

    /// Returns the native token symbol (e.g., "ETH", "MATIC").
    fn native_token_symbol(&self) -> &str;

    /// Fetches the native token balance for an address.
    ///
    /// # Arguments
    ///
    /// * `address` - The blockchain address to query
    ///
    /// # Returns
    ///
    /// Returns a [`Balance`] containing the balance in multiple formats.
    async fn get_balance(&self, address: &str) -> Result<Balance>;

    /// Enriches a balance with USD valuation via DexScreener.
    ///
    /// # Arguments
    ///
    /// * `balance` - The balance to enrich with a USD value
    async fn enrich_balance_usd(&self, balance: &mut Balance);

    /// Fetches transaction details by hash.
    ///
    /// # Arguments
    ///
    /// * `hash` - The transaction hash to query
    ///
    /// # Returns
    ///
    /// Returns [`Transaction`] details or an error if not found.
    async fn get_transaction(&self, hash: &str) -> Result<Transaction>;

    /// Fetches recent transactions for an address.
    ///
    /// # Arguments
    ///
    /// * `address` - The address to query
    /// * `limit` - Maximum number of transactions to return
    ///
    /// # Returns
    ///
    /// Returns a vector of [`Transaction`] objects.
    async fn get_transactions(&self, address: &str, limit: u32) -> Result<Vec<Transaction>>;

    /// Fetches the current block number.
    async fn get_block_number(&self) -> Result<u64>;

    /// Fetches token balances for an address.
    ///
    /// Returns a unified [`TokenBalance`] list regardless of chain
    /// (ERC-20, SPL, TRC-20 all map to the same type).
    async fn get_token_balances(&self, address: &str) -> Result<Vec<TokenBalance>>;

    /// Fetches token information for a contract address.
    ///
    /// Default implementation returns "not supported" error.
    /// Override in chain clients that support token info lookups.
    async fn get_token_info(&self, _address: &str) -> Result<Token> {
        Err(crate::error::ScopeError::Chain(
            "Token info lookup not supported on this chain".to_string(),
        ))
    }

    /// Fetches top token holders for a contract address.
    ///
    /// Default implementation returns an empty vector.
    /// Override in chain clients that support holder lookups.
    async fn get_token_holders(&self, _address: &str, _limit: u32) -> Result<Vec<TokenHolder>> {
        Ok(Vec::new())
    }

    /// Fetches total token holder count for a contract address.
    ///
    /// Default implementation returns 0.
    /// Override in chain clients that support holder count lookups.
    async fn get_token_holder_count(&self, _address: &str) -> Result<u64> {
        Ok(0)
    }

    /// Fetches bytecode at address (EVM: eth_getCode).
    /// Returns "0x" for EOA, non-empty hex for contracts.
    /// Default: not supported.
    async fn get_code(&self, _address: &str) -> Result<String> {
        Err(crate::error::ScopeError::Chain(
            "Code lookup not supported on this chain".to_string(),
        ))
    }
}

/// Factory trait for creating chain clients and DEX data sources.
///
/// Bundles both chain and DEX client creation so CLI functions
/// only need one injected dependency instead of two.
///
/// # Example
///
/// ```rust,no_run
/// use scope::chains::{ChainClientFactory, DefaultClientFactory};
/// use scope::Config;
///
/// let config = Config::default();
/// let factory = DefaultClientFactory { chains_config: config.chains.clone() };
/// let client = factory.create_chain_client("ethereum").unwrap();
/// ```
pub trait ChainClientFactory: Send + Sync {
    /// Creates a chain client for the given blockchain network.
    ///
    /// # Arguments
    ///
    /// * `chain` - The chain name (e.g., "ethereum", "solana", "tron")
    fn create_chain_client(&self, chain: &str) -> Result<Box<dyn ChainClient>>;

    /// Creates a DEX data source client.
    fn create_dex_client(&self) -> Box<dyn DexDataSource>;
}

/// Default factory that creates real chain clients from configuration.
pub struct DefaultClientFactory {
    /// Chain configuration containing API keys and endpoints.
    pub chains_config: crate::config::ChainsConfig,
}

impl ChainClientFactory for DefaultClientFactory {
    fn create_chain_client(&self, chain: &str) -> Result<Box<dyn ChainClient>> {
        match chain.to_lowercase().as_str() {
            "solana" | "sol" => Ok(Box::new(SolanaClient::new(&self.chains_config)?)),
            "tron" | "trx" => Ok(Box::new(TronClient::new(&self.chains_config)?)),
            _ => Ok(Box::new(EthereumClient::for_chain(
                chain,
                &self.chains_config,
            )?)),
        }
    }

    fn create_dex_client(&self) -> Box<dyn DexDataSource> {
        Box::new(DexClient::new())
    }
}

/// Balance representation with multiple formats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Balance {
    /// Raw balance in smallest unit (e.g., wei).
    pub raw: String,

    /// Human-readable formatted balance.
    pub formatted: String,

    /// Number of decimals for the token.
    pub decimals: u8,

    /// Token symbol.
    pub symbol: String,

    /// USD value (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usd_value: Option<f64>,
}

/// Transaction information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    /// Transaction hash.
    pub hash: String,

    /// Block number (None if pending).
    pub block_number: Option<u64>,

    /// Block timestamp (None if pending).
    pub timestamp: Option<u64>,

    /// Sender address.
    pub from: String,

    /// Recipient address (None for contract creation).
    pub to: Option<String>,

    /// Value transferred in native token.
    pub value: String,

    /// Gas limit.
    pub gas_limit: u64,

    /// Gas used (None if pending).
    pub gas_used: Option<u64>,

    /// Gas price in wei.
    pub gas_price: String,

    /// Transaction nonce.
    pub nonce: u64,

    /// Input data.
    pub input: String,

    /// Transaction status (None if pending, Some(true) for success).
    pub status: Option<bool>,
}

/// Token information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    /// Contract address.
    pub contract_address: String,

    /// Token symbol.
    pub symbol: String,

    /// Token name.
    pub name: String,

    /// Decimal places.
    pub decimals: u8,
}

/// Token balance for an address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBalance {
    /// Token information.
    pub token: Token,

    /// Raw balance.
    pub balance: String,

    /// Formatted balance.
    pub formatted_balance: String,

    /// USD value (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usd_value: Option<f64>,
}

// ============================================================================
// Token Analytics Types
// ============================================================================

/// A token holder with their balance and percentage of supply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenHolder {
    /// Holder's address.
    pub address: String,

    /// Raw balance amount.
    pub balance: String,

    /// Formatted balance with proper decimals.
    pub formatted_balance: String,

    /// Percentage of total supply held.
    pub percentage: f64,

    /// Rank among all holders (1 = largest).
    pub rank: u32,
}

/// A price data point for historical charting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricePoint {
    /// Unix timestamp in seconds.
    pub timestamp: i64,

    /// Price in USD.
    pub price: f64,
}

/// A volume data point for historical charting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumePoint {
    /// Unix timestamp in seconds.
    pub timestamp: i64,

    /// Volume in USD.
    pub volume: f64,
}

/// A holder count data point for historical charting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HolderCountPoint {
    /// Unix timestamp in seconds.
    pub timestamp: i64,

    /// Number of holders.
    pub count: u64,
}

/// DEX trading pair information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexPair {
    /// DEX name (e.g., "Uniswap V3", "SushiSwap").
    pub dex_name: String,

    /// Pair address on the DEX.
    pub pair_address: String,

    /// Base token symbol.
    pub base_token: String,

    /// Quote token symbol.
    pub quote_token: String,

    /// Current price in USD.
    pub price_usd: f64,

    /// 24h trading volume in USD.
    pub volume_24h: f64,

    /// Liquidity in USD.
    pub liquidity_usd: f64,

    /// Price change percentage in 24h.
    pub price_change_24h: f64,

    /// Buy transactions in 24h.
    pub buys_24h: u64,

    /// Sell transactions in 24h.
    pub sells_24h: u64,

    /// Buy transactions in 6h.
    pub buys_6h: u64,

    /// Sell transactions in 6h.
    pub sells_6h: u64,

    /// Buy transactions in 1h.
    pub buys_1h: u64,

    /// Sell transactions in 1h.
    pub sells_1h: u64,

    /// Pair creation timestamp.
    pub pair_created_at: Option<i64>,

    /// Direct URL to this pair on DexScreener.
    pub url: Option<String>,
}

/// Comprehensive token analytics data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenAnalytics {
    /// Token information.
    pub token: Token,

    /// Blockchain network name.
    pub chain: String,

    /// Top token holders.
    pub holders: Vec<TokenHolder>,

    /// Total number of holders.
    pub total_holders: u64,

    /// 24-hour trading volume in USD.
    pub volume_24h: f64,

    /// 7-day trading volume in USD.
    pub volume_7d: f64,

    /// Current price in USD.
    pub price_usd: f64,

    /// 24-hour price change percentage.
    pub price_change_24h: f64,

    /// 7-day price change percentage.
    pub price_change_7d: f64,

    /// Total liquidity across DEXs in USD.
    pub liquidity_usd: f64,

    /// Market capitalization (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market_cap: Option<f64>,

    /// Fully diluted valuation (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fdv: Option<f64>,

    /// Total supply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_supply: Option<String>,

    /// Circulating supply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub circulating_supply: Option<String>,

    /// Historical price data for charting.
    pub price_history: Vec<PricePoint>,

    /// Historical volume data for charting.
    pub volume_history: Vec<VolumePoint>,

    /// Historical holder count data for charting.
    pub holder_history: Vec<HolderCountPoint>,

    /// DEX trading pairs.
    pub dex_pairs: Vec<DexPair>,

    /// Timestamp when this data was fetched.
    pub fetched_at: i64,

    /// Percentage of supply held by top 10 holders.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_10_concentration: Option<f64>,

    /// Percentage of supply held by top 50 holders.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_50_concentration: Option<f64>,

    /// Percentage of supply held by top 100 holders.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_100_concentration: Option<f64>,

    /// 6-hour price change percentage.
    pub price_change_6h: f64,

    /// 1-hour price change percentage.
    pub price_change_1h: f64,

    /// Total buy transactions in 24 hours.
    pub total_buys_24h: u64,

    /// Total sell transactions in 24 hours.
    pub total_sells_24h: u64,

    /// Total buy transactions in 6 hours.
    pub total_buys_6h: u64,

    /// Total sell transactions in 6 hours.
    pub total_sells_6h: u64,

    /// Total buy transactions in 1 hour.
    pub total_buys_1h: u64,

    /// Total sell transactions in 1 hour.
    pub total_sells_1h: u64,

    /// Token age in hours (since earliest pair creation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_age_hours: Option<f64>,

    /// Token image URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,

    /// Token website URLs.
    pub websites: Vec<String>,

    /// Token social media links.
    pub socials: Vec<TokenSocial>,

    /// DexScreener URL for the primary pair.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dexscreener_url: Option<String>,
}

/// Social media link for a token.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenSocial {
    /// Platform name (twitter, telegram, discord, etc.)
    pub platform: String,
    /// URL or handle for the social account.
    pub url: String,
}

// ============================================================================
// Chain Metadata
// ============================================================================

/// Metadata for a blockchain network (symbol, decimals, explorer URLs).
///
/// Used for normalized presentation across all chains.
#[derive(Debug, Clone)]
pub struct ChainMetadata {
    /// Canonical chain identifier.
    pub chain_id: &'static str,
    /// Native token symbol (e.g., ETH, SOL, TRX).
    pub native_symbol: &'static str,
    /// Native token decimals.
    pub native_decimals: u8,
    /// Block explorer base URL for token pages.
    pub explorer_token_base: &'static str,
}

/// Returns chain metadata for display and formatting.
///
/// Returns `None` for unknown chains.
pub fn chain_metadata(chain: &str) -> Option<ChainMetadata> {
    match chain.to_lowercase().as_str() {
        "ethereum" | "eth" => Some(ChainMetadata {
            chain_id: "ethereum",
            native_symbol: "ETH",
            native_decimals: 18,
            explorer_token_base: "https://etherscan.io/token",
        }),
        "polygon" => Some(ChainMetadata {
            chain_id: "polygon",
            native_symbol: "MATIC",
            native_decimals: 18,
            explorer_token_base: "https://polygonscan.com/token",
        }),
        "arbitrum" => Some(ChainMetadata {
            chain_id: "arbitrum",
            native_symbol: "ETH",
            native_decimals: 18,
            explorer_token_base: "https://arbiscan.io/token",
        }),
        "optimism" => Some(ChainMetadata {
            chain_id: "optimism",
            native_symbol: "ETH",
            native_decimals: 18,
            explorer_token_base: "https://optimistic.etherscan.io/token",
        }),
        "base" => Some(ChainMetadata {
            chain_id: "base",
            native_symbol: "ETH",
            native_decimals: 18,
            explorer_token_base: "https://basescan.org/token",
        }),
        "bsc" => Some(ChainMetadata {
            chain_id: "bsc",
            native_symbol: "BNB",
            native_decimals: 18,
            explorer_token_base: "https://bscscan.com/token",
        }),
        "solana" | "sol" => Some(ChainMetadata {
            chain_id: "solana",
            native_symbol: "SOL",
            native_decimals: 9,
            explorer_token_base: "https://solscan.io/token",
        }),
        "tron" | "trx" => Some(ChainMetadata {
            chain_id: "tron",
            native_symbol: "TRX",
            native_decimals: 6,
            explorer_token_base: "https://tronscan.org/#/token20",
        }),
        _ => None,
    }
}

/// Returns the native token symbol for a chain, or "???" if unknown.
pub fn native_symbol(chain: &str) -> &'static str {
    chain_metadata(chain)
        .map(|m| m.native_symbol)
        .unwrap_or("???")
}

// ============================================================================
// Chain Inference
// ============================================================================

/// Infers the blockchain from an address format.
///
/// Returns `Some(chain_name)` if the address format is unambiguous,
/// or `None` if the format is not recognized.
///
/// # Supported Formats
///
/// - **EVM** (ethereum): `0x` prefix + 40 hex chars (42 total)
/// - **Tron**: Starts with `T` + 34 chars (Base58Check)
/// - **Solana**: Base58, 32-44 chars, decodes to 32 bytes
///
/// # Examples
///
/// ```
/// use scope::chains::infer_chain_from_address;
///
/// assert_eq!(infer_chain_from_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"), Some("ethereum"));
/// assert_eq!(infer_chain_from_address("TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf"), Some("tron"));
/// assert_eq!(infer_chain_from_address("DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy"), Some("solana"));
/// assert_eq!(infer_chain_from_address("invalid"), None);
/// ```
pub fn infer_chain_from_address(address: &str) -> Option<&'static str> {
    // Tron: starts with 'T', 34 chars, valid base58
    if address.starts_with('T') && address.len() == 34 && bs58::decode(address).into_vec().is_ok() {
        return Some("tron");
    }

    // EVM: 0x prefix, 42 chars total (40 hex + "0x")
    if address.starts_with("0x")
        && address.len() == 42
        && address[2..].chars().all(|c| c.is_ascii_hexdigit())
    {
        return Some("ethereum");
    }

    // Solana: base58, 32-44 chars, decodes to 32 bytes
    if address.len() >= 32
        && address.len() <= 44
        && let Ok(decoded) = bs58::decode(address).into_vec()
        && decoded.len() == 32
    {
        return Some("solana");
    }

    None
}

/// Infers the blockchain from a transaction hash format.
///
/// Returns `Some(chain_name)` if the hash format is unambiguous,
/// or `None` if the format is not recognized.
///
/// # Supported Formats
///
/// - **EVM** (ethereum): `0x` prefix + 64 hex chars (66 total)
/// - **Tron**: 64 hex chars (no prefix)
/// - **Solana**: Base58, 80-90 chars, decodes to 64 bytes
///
/// # Examples
///
/// ```
/// use scope::chains::infer_chain_from_hash;
///
/// // EVM hash
/// let evm_hash = "0xabc123def456789012345678901234567890123456789012345678901234abcd";
/// assert_eq!(infer_chain_from_hash(evm_hash), Some("ethereum"));
///
/// // Tron hash (64 hex chars, no 0x prefix)
/// let tron_hash = "abc123def456789012345678901234567890123456789012345678901234abcd";
/// assert_eq!(infer_chain_from_hash(tron_hash), Some("tron"));
/// ```
pub fn infer_chain_from_hash(hash: &str) -> Option<&'static str> {
    // EVM: 0x prefix, 66 chars total (64 hex + "0x")
    if hash.starts_with("0x")
        && hash.len() == 66
        && hash[2..].chars().all(|c| c.is_ascii_hexdigit())
    {
        return Some("ethereum");
    }

    // Tron: 64 hex chars, no prefix
    if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some("tron");
    }

    // Solana: base58, 80-90 chars, decodes to 64 bytes
    if hash.len() >= 80
        && hash.len() <= 90
        && let Ok(decoded) = bs58::decode(hash).into_vec()
        && decoded.len() == 64
    {
        return Some("solana");
    }

    None
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_balance_serialization() {
        let balance = Balance {
            raw: "1000000000000000000".to_string(),
            formatted: "1.0".to_string(),
            decimals: 18,
            symbol: "ETH".to_string(),
            usd_value: Some(3500.0),
        };

        let json = serde_json::to_string(&balance).unwrap();
        assert!(json.contains("1000000000000000000"));
        assert!(json.contains("1.0"));
        assert!(json.contains("ETH"));
        assert!(json.contains("3500"));

        let deserialized: Balance = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.raw, balance.raw);
        assert_eq!(deserialized.decimals, 18);
    }

    #[test]
    fn test_balance_without_usd() {
        let balance = Balance {
            raw: "1000000000000000000".to_string(),
            formatted: "1.0".to_string(),
            decimals: 18,
            symbol: "ETH".to_string(),
            usd_value: None,
        };

        let json = serde_json::to_string(&balance).unwrap();
        assert!(!json.contains("usd_value"));
    }

    #[test]
    fn test_transaction_serialization() {
        let tx = Transaction {
            hash: "0xabc123".to_string(),
            block_number: Some(12345678),
            timestamp: Some(1700000000),
            from: "0xfrom".to_string(),
            to: Some("0xto".to_string()),
            value: "1.0".to_string(),
            gas_limit: 21000,
            gas_used: Some(21000),
            gas_price: "20000000000".to_string(),
            nonce: 42,
            input: "0x".to_string(),
            status: Some(true),
        };

        let json = serde_json::to_string(&tx).unwrap();
        assert!(json.contains("0xabc123"));
        assert!(json.contains("12345678"));
        assert!(json.contains("0xfrom"));
        assert!(json.contains("0xto"));

        let deserialized: Transaction = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.hash, tx.hash);
        assert_eq!(deserialized.nonce, 42);
    }

    #[test]
    fn test_pending_transaction_serialization() {
        let tx = Transaction {
            hash: "0xpending".to_string(),
            block_number: None,
            timestamp: None,
            from: "0xfrom".to_string(),
            to: Some("0xto".to_string()),
            value: "1.0".to_string(),
            gas_limit: 21000,
            gas_used: None,
            gas_price: "20000000000".to_string(),
            nonce: 0,
            input: "0x".to_string(),
            status: None,
        };

        let json = serde_json::to_string(&tx).unwrap();
        assert!(json.contains("0xpending"));
        assert!(json.contains("null")); // None values serialize as null

        let deserialized: Transaction = serde_json::from_str(&json).unwrap();
        assert!(deserialized.block_number.is_none());
        assert!(deserialized.status.is_none());
    }

    #[test]
    fn test_contract_creation_transaction() {
        let tx = Transaction {
            hash: "0xcreate".to_string(),
            block_number: Some(100),
            timestamp: Some(1700000000),
            from: "0xdeployer".to_string(),
            to: None, // Contract creation
            value: "0".to_string(),
            gas_limit: 1000000,
            gas_used: Some(500000),
            gas_price: "20000000000".to_string(),
            nonce: 0,
            input: "0x608060...".to_string(),
            status: Some(true),
        };

        let json = serde_json::to_string(&tx).unwrap();
        assert!(json.contains("\"to\":null"));
    }

    #[test]
    fn test_token_serialization() {
        let token = Token {
            contract_address: "0xtoken".to_string(),
            symbol: "USDC".to_string(),
            name: "USD Coin".to_string(),
            decimals: 6,
        };

        let json = serde_json::to_string(&token).unwrap();
        assert!(json.contains("USDC"));
        assert!(json.contains("USD Coin"));
        assert!(json.contains("\"decimals\":6"));

        let deserialized: Token = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.decimals, 6);
    }

    #[test]
    fn test_token_balance_serialization() {
        let token_balance = TokenBalance {
            token: Token {
                contract_address: "0xtoken".to_string(),
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                decimals: 6,
            },
            balance: "1000000".to_string(),
            formatted_balance: "1.0".to_string(),
            usd_value: Some(1.0),
        };

        let json = serde_json::to_string(&token_balance).unwrap();
        assert!(json.contains("USDC"));
        assert!(json.contains("1000000"));
        assert!(json.contains("1.0"));
    }

    #[test]
    fn test_balance_debug() {
        let balance = Balance {
            raw: "1000".to_string(),
            formatted: "0.001".to_string(),
            decimals: 18,
            symbol: "ETH".to_string(),
            usd_value: None,
        };

        let debug_str = format!("{:?}", balance);
        assert!(debug_str.contains("Balance"));
        assert!(debug_str.contains("1000"));
    }

    #[test]
    fn test_transaction_debug() {
        let tx = Transaction {
            hash: "0xtest".to_string(),
            block_number: Some(1),
            timestamp: Some(0),
            from: "0x1".to_string(),
            to: Some("0x2".to_string()),
            value: "0".to_string(),
            gas_limit: 21000,
            gas_used: Some(21000),
            gas_price: "0".to_string(),
            nonce: 0,
            input: "0x".to_string(),
            status: Some(true),
        };

        let debug_str = format!("{:?}", tx);
        assert!(debug_str.contains("Transaction"));
        assert!(debug_str.contains("0xtest"));
    }

    // ============================================================================
    // Chain Inference Tests
    // ============================================================================

    #[test]
    fn test_infer_chain_from_address_evm() {
        // Valid EVM addresses (0x + 40 hex chars)
        assert_eq!(
            super::infer_chain_from_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"),
            Some("ethereum")
        );
        assert_eq!(
            super::infer_chain_from_address("0x0000000000000000000000000000000000000000"),
            Some("ethereum")
        );
        assert_eq!(
            super::infer_chain_from_address("0xABCDEF1234567890abcdef1234567890ABCDEF12"),
            Some("ethereum")
        );
    }

    #[test]
    fn test_infer_chain_from_address_tron() {
        // Valid Tron addresses (T + 33 chars = 34 total, base58)
        assert_eq!(
            super::infer_chain_from_address("TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf"),
            Some("tron")
        );
        assert_eq!(
            super::infer_chain_from_address("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"),
            Some("tron")
        );
    }

    #[test]
    fn test_infer_chain_from_address_solana() {
        // Valid Solana addresses (base58, 32-44 chars, decodes to 32 bytes)
        assert_eq!(
            super::infer_chain_from_address("DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy"),
            Some("solana")
        );
        // System program address
        assert_eq!(
            super::infer_chain_from_address("11111111111111111111111111111111"),
            Some("solana")
        );
    }

    #[test]
    fn test_infer_chain_from_address_invalid() {
        // Too short
        assert_eq!(super::infer_chain_from_address("0x123"), None);
        // Invalid characters
        assert_eq!(super::infer_chain_from_address("not_an_address"), None);
        // Empty
        assert_eq!(super::infer_chain_from_address(""), None);
        // EVM-like but wrong length
        assert_eq!(super::infer_chain_from_address("0x123456"), None);
        // Tron-like but not starting with T
        assert_eq!(
            super::infer_chain_from_address("ADqSquXBgUCLYvYC4XZgrprLK589dkhSCf"),
            None
        );
    }

    #[test]
    fn test_infer_chain_from_hash_evm() {
        // Valid EVM transaction hash (0x + 64 hex chars)
        assert_eq!(
            super::infer_chain_from_hash(
                "0xabc123def456789012345678901234567890123456789012345678901234abcd"
            ),
            Some("ethereum")
        );
        assert_eq!(
            super::infer_chain_from_hash(
                "0x0000000000000000000000000000000000000000000000000000000000000000"
            ),
            Some("ethereum")
        );
    }

    #[test]
    fn test_infer_chain_from_hash_tron() {
        // Valid Tron transaction hash (64 hex chars, no 0x prefix)
        assert_eq!(
            super::infer_chain_from_hash(
                "abc123def456789012345678901234567890123456789012345678901234abcd"
            ),
            Some("tron")
        );
        assert_eq!(
            super::infer_chain_from_hash(
                "0000000000000000000000000000000000000000000000000000000000000000"
            ),
            Some("tron")
        );
    }

    #[test]
    fn test_infer_chain_from_hash_solana() {
        // Valid Solana signature (base58, 80-90 chars, decodes to 64 bytes)
        // This is a made-up example that fits the pattern
        let solana_sig = "5VERv8NMvzbJMEkV8xnrLkEaWRtSz9CosKDYjCJjBRnbJLgp8uirBgmQpjKhoR4tjF3ZpRzrFmBV6UjKdiSZkQUW";
        assert_eq!(super::infer_chain_from_hash(solana_sig), Some("solana"));
    }

    #[test]
    fn test_infer_chain_from_hash_invalid() {
        // Too short
        assert_eq!(super::infer_chain_from_hash("0x123"), None);
        // Invalid
        assert_eq!(super::infer_chain_from_hash("not_a_hash"), None);
        // Empty
        assert_eq!(super::infer_chain_from_hash(""), None);
        // 64 chars but with invalid hex (contains 'g')
        assert_eq!(
            super::infer_chain_from_hash(
                "abc123gef456789012345678901234567890123456789012345678901234abcd"
            ),
            None
        );
    }

    // ============================================================================
    // DefaultClientFactory Tests
    // ============================================================================

    #[test]
    fn test_default_client_factory_create_dex_client() {
        let config = crate::config::ChainsConfig::default();
        let factory = DefaultClientFactory {
            chains_config: config,
        };
        let dex = factory.create_dex_client();
        // Just verify it returns without panicking - the client is a Box<dyn DexDataSource>
        let _ = format!("{:?}", std::mem::size_of_val(&dex));
    }

    #[test]
    fn test_default_client_factory_create_ethereum_client() {
        let config = crate::config::ChainsConfig::default();
        let factory = DefaultClientFactory {
            chains_config: config,
        };
        // ethereum, polygon, etc use EthereumClient::for_chain
        let client = factory.create_chain_client("ethereum");
        assert!(client.is_ok());
        assert_eq!(client.unwrap().chain_name(), "ethereum");
    }

    #[test]
    fn test_default_client_factory_create_polygon_client() {
        let config = crate::config::ChainsConfig::default();
        let factory = DefaultClientFactory {
            chains_config: config,
        };
        let client = factory.create_chain_client("polygon");
        assert!(client.is_ok());
        assert_eq!(client.unwrap().chain_name(), "polygon");
    }

    #[test]
    fn test_default_client_factory_create_solana_client() {
        let config = crate::config::ChainsConfig::default();
        let factory = DefaultClientFactory {
            chains_config: config,
        };
        let client = factory.create_chain_client("solana");
        assert!(client.is_ok());
        assert_eq!(client.unwrap().chain_name(), "solana");
    }

    #[test]
    fn test_default_client_factory_create_sol_alias() {
        let config = crate::config::ChainsConfig::default();
        let factory = DefaultClientFactory {
            chains_config: config,
        };
        let client = factory.create_chain_client("sol");
        assert!(client.is_ok());
        assert_eq!(client.unwrap().chain_name(), "solana");
    }

    #[test]
    fn test_default_client_factory_create_tron_client() {
        let config = crate::config::ChainsConfig::default();
        let factory = DefaultClientFactory {
            chains_config: config,
        };
        let client = factory.create_chain_client("tron");
        assert!(client.is_ok());
        assert_eq!(client.unwrap().chain_name(), "tron");
    }

    #[test]
    fn test_default_client_factory_create_trx_alias() {
        let config = crate::config::ChainsConfig::default();
        let factory = DefaultClientFactory {
            chains_config: config,
        };
        let client = factory.create_chain_client("trx");
        assert!(client.is_ok());
        assert_eq!(client.unwrap().chain_name(), "tron");
    }

    // ============================================================================
    // ChainClient trait default method tests
    // ============================================================================

    #[tokio::test]
    async fn test_chain_client_default_get_token_info() {
        use super::mocks::MockChainClient;
        // Create a client without token_info set (None)
        let client = MockChainClient::new("ethereum", "ETH");
        let result = client.get_token_info("0xsometoken").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_chain_client_default_get_token_holders() {
        use super::mocks::MockChainClient;
        let client = MockChainClient::new("ethereum", "ETH");
        let holders = client.get_token_holders("0xsometoken", 10).await.unwrap();
        assert!(holders.is_empty());
    }

    #[tokio::test]
    async fn test_chain_client_default_get_token_holder_count() {
        use super::mocks::MockChainClient;
        let client = MockChainClient::new("ethereum", "ETH");
        let count = client.get_token_holder_count("0xsometoken").await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_mock_client_factory_creates_chain_client() {
        use super::mocks::MockClientFactory;
        let factory = MockClientFactory::new();
        let client = factory.create_chain_client("anything").unwrap();
        assert_eq!(client.chain_name(), "ethereum"); // defaults to ethereum mock
    }

    #[tokio::test]
    async fn test_mock_client_factory_creates_dex_client() {
        use super::mocks::MockClientFactory;
        let factory = MockClientFactory::new();
        let dex = factory.create_dex_client();
        let price = dex.get_token_price("ethereum", "0xtest").await;
        assert_eq!(price, Some(1.0));
    }

    #[tokio::test]
    async fn test_mock_chain_client_balance() {
        use super::mocks::MockChainClient;
        let client = MockChainClient::new("ethereum", "ETH");
        let balance = client.get_balance("0xtest").await.unwrap();
        assert_eq!(balance.formatted, "1.0");
        assert_eq!(balance.symbol, "ETH");
        assert_eq!(balance.usd_value, Some(2500.0));
    }

    #[tokio::test]
    async fn test_mock_chain_client_transaction() {
        use super::mocks::MockChainClient;
        let client = MockChainClient::new("ethereum", "ETH");
        let tx = client.get_transaction("0xanyhash").await.unwrap();
        assert_eq!(tx.hash, "0xmocktx");
        assert_eq!(tx.nonce, 42);
    }

    #[tokio::test]
    async fn test_mock_chain_client_block_number() {
        use super::mocks::MockChainClient;
        let client = MockChainClient::new("ethereum", "ETH");
        let block = client.get_block_number().await.unwrap();
        assert_eq!(block, 12345678);
    }

    #[tokio::test]
    async fn test_mock_dex_source_data() {
        use super::mocks::MockDexSource;
        let dex = MockDexSource::new();
        let data = dex.get_token_data("ethereum", "0xtest").await.unwrap();
        assert_eq!(data.symbol, "MOCK");
        assert_eq!(data.price_usd, 1.0);
    }

    #[tokio::test]
    async fn test_mock_dex_source_search() {
        use super::mocks::MockDexSource;
        let dex = MockDexSource::new();
        let results = dex.search_tokens("test", None).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_mock_dex_source_native_price() {
        use super::mocks::MockDexSource;
        let dex = MockDexSource::new();
        let price = dex.get_native_token_price("ethereum").await;
        assert_eq!(price, Some(2500.0));
    }

    // ========================================================================
    // Default ChainClient trait method tests
    // ========================================================================

    /// Minimal ChainClient impl that uses all default methods.
    struct MinimalChainClient;

    #[async_trait::async_trait]
    impl ChainClient for MinimalChainClient {
        fn chain_name(&self) -> &str {
            "test"
        }

        fn native_token_symbol(&self) -> &str {
            "TEST"
        }

        async fn get_balance(&self, _address: &str) -> Result<Balance> {
            Ok(Balance {
                raw: "0".to_string(),
                formatted: "0".to_string(),
                decimals: 18,
                symbol: "TEST".to_string(),
                usd_value: None,
            })
        }

        async fn get_transaction(&self, _hash: &str) -> Result<Transaction> {
            unimplemented!()
        }

        async fn get_transactions(&self, _address: &str, _limit: u32) -> Result<Vec<Transaction>> {
            Ok(Vec::new())
        }

        async fn get_block_number(&self) -> Result<u64> {
            Ok(0)
        }

        async fn get_token_balances(&self, _address: &str) -> Result<Vec<TokenBalance>> {
            Ok(Vec::new())
        }

        async fn enrich_balance_usd(&self, _balance: &mut Balance) {}
    }

    #[tokio::test]
    async fn test_default_get_token_info() {
        let client = MinimalChainClient;
        let result = client.get_token_info("0xtest").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not supported"));
    }

    #[tokio::test]
    async fn test_default_get_token_holders() {
        let client = MinimalChainClient;
        let holders = client.get_token_holders("0xtest", 10).await.unwrap();
        assert!(holders.is_empty());
    }

    #[tokio::test]
    async fn test_default_get_token_holder_count() {
        let client = MinimalChainClient;
        let count = client.get_token_holder_count("0xtest").await.unwrap();
        assert_eq!(count, 0);
    }
}

// ============================================================================
// Mock Test Utilities
// ============================================================================

/// Test helper module providing mock implementations of chain client traits.
///
/// These mocks are available across all test modules in the crate for
/// end-to-end testing of CLI `run()` functions without network calls.
#[cfg(test)]
pub mod mocks {
    use super::*;
    use crate::chains::dex::{DexDataSource, DexTokenData, TokenSearchResult};
    use async_trait::async_trait;

    /// Mock chain client with configurable responses.
    #[derive(Debug, Clone)]
    pub struct MockChainClient {
        pub chain: String,
        pub symbol: String,
        pub balance: Balance,
        pub transaction: Transaction,
        pub transactions: Vec<Transaction>,
        pub token_balances: Vec<TokenBalance>,
        pub block_number: u64,
        pub token_info: Option<Token>,
        pub token_holders: Vec<TokenHolder>,
        pub token_holder_count: u64,
    }

    impl MockChainClient {
        /// Creates a mock client with sensible default test data.
        pub fn new(chain: &str, symbol: &str) -> Self {
            Self {
                chain: chain.to_string(),
                symbol: symbol.to_string(),
                balance: Balance {
                    raw: "1000000000000000000".to_string(),
                    formatted: "1.0".to_string(),
                    decimals: 18,
                    symbol: symbol.to_string(),
                    usd_value: Some(2500.0),
                },
                transaction: Transaction {
                    hash: "0xmocktx".to_string(),
                    block_number: Some(12345678),
                    timestamp: Some(1700000000),
                    from: "0xfrom".to_string(),
                    to: Some("0xto".to_string()),
                    value: "1.0".to_string(),
                    gas_limit: 21000,
                    gas_used: Some(21000),
                    gas_price: "20000000000".to_string(),
                    nonce: 42,
                    input: "0x".to_string(),
                    status: Some(true),
                },
                transactions: vec![],
                token_balances: vec![],
                block_number: 12345678,
                token_info: None,
                token_holders: vec![],
                token_holder_count: 0,
            }
        }
    }

    #[async_trait]
    impl ChainClient for MockChainClient {
        fn chain_name(&self) -> &str {
            &self.chain
        }

        fn native_token_symbol(&self) -> &str {
            &self.symbol
        }

        async fn get_balance(&self, _address: &str) -> Result<Balance> {
            Ok(self.balance.clone())
        }

        async fn enrich_balance_usd(&self, _balance: &mut Balance) {
            // Mock: no-op, balance already has usd_value set
        }

        async fn get_transaction(&self, _hash: &str) -> Result<Transaction> {
            Ok(self.transaction.clone())
        }

        async fn get_transactions(&self, _address: &str, _limit: u32) -> Result<Vec<Transaction>> {
            Ok(self.transactions.clone())
        }

        async fn get_block_number(&self) -> Result<u64> {
            Ok(self.block_number)
        }

        async fn get_token_balances(&self, _address: &str) -> Result<Vec<TokenBalance>> {
            Ok(self.token_balances.clone())
        }

        async fn get_token_info(&self, _address: &str) -> Result<Token> {
            match &self.token_info {
                Some(t) => Ok(t.clone()),
                None => Err(crate::error::ScopeError::Chain(
                    "Token info not available".to_string(),
                )),
            }
        }

        async fn get_token_holders(&self, _address: &str, _limit: u32) -> Result<Vec<TokenHolder>> {
            Ok(self.token_holders.clone())
        }

        async fn get_token_holder_count(&self, _address: &str) -> Result<u64> {
            Ok(self.token_holder_count)
        }
    }

    /// Mock DEX data source with configurable responses.
    #[derive(Debug, Clone)]
    pub struct MockDexSource {
        pub token_price: Option<f64>,
        pub native_price: Option<f64>,
        pub token_data: Option<DexTokenData>,
        pub search_results: Vec<TokenSearchResult>,
    }

    impl Default for MockDexSource {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockDexSource {
        /// Creates a mock DEX source with default test data.
        pub fn new() -> Self {
            Self {
                token_price: Some(1.0),
                native_price: Some(2500.0),
                token_data: Some(DexTokenData {
                    address: "0xmocktoken".to_string(),
                    symbol: "MOCK".to_string(),
                    name: "Mock Token".to_string(),
                    price_usd: 1.0,
                    price_change_24h: 5.0,
                    price_change_6h: 2.0,
                    price_change_1h: 0.5,
                    price_change_5m: 0.1,
                    volume_24h: 1_000_000.0,
                    volume_6h: 250_000.0,
                    volume_1h: 50_000.0,
                    liquidity_usd: 5_000_000.0,
                    market_cap: Some(100_000_000.0),
                    fdv: Some(200_000_000.0),
                    pairs: vec![],
                    price_history: vec![],
                    volume_history: vec![],
                    total_buys_24h: 500,
                    total_sells_24h: 450,
                    total_buys_6h: 120,
                    total_sells_6h: 110,
                    total_buys_1h: 20,
                    total_sells_1h: 18,
                    earliest_pair_created_at: Some(1690000000),
                    image_url: None,
                    websites: vec![],
                    socials: vec![],
                    dexscreener_url: None,
                }),
                search_results: vec![],
            }
        }
    }

    #[async_trait]
    impl DexDataSource for MockDexSource {
        async fn get_token_price(&self, _chain: &str, _address: &str) -> Option<f64> {
            self.token_price
        }

        async fn get_native_token_price(&self, _chain: &str) -> Option<f64> {
            self.native_price
        }

        async fn get_token_data(&self, _chain: &str, _address: &str) -> Result<DexTokenData> {
            match &self.token_data {
                Some(data) => Ok(data.clone()),
                None => Err(crate::error::ScopeError::NotFound(
                    "No DEX data found".to_string(),
                )),
            }
        }

        async fn search_tokens(
            &self,
            _query: &str,
            _chain: Option<&str>,
        ) -> Result<Vec<TokenSearchResult>> {
            Ok(self.search_results.clone())
        }
    }

    /// Mock client factory that returns pre-configured mock clients.
    pub struct MockClientFactory {
        pub mock_client: MockChainClient,
        pub mock_dex: MockDexSource,
    }

    impl Default for MockClientFactory {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockClientFactory {
        /// Creates a factory with default mock data for Ethereum.
        pub fn new() -> Self {
            Self {
                mock_client: MockChainClient::new("ethereum", "ETH"),
                mock_dex: MockDexSource::new(),
            }
        }
    }

    impl ChainClientFactory for MockClientFactory {
        fn create_chain_client(&self, _chain: &str) -> Result<Box<dyn ChainClient>> {
            Ok(Box::new(self.mock_client.clone()))
        }

        fn create_dex_client(&self) -> Box<dyn DexDataSource> {
            Box::new(self.mock_dex.clone())
        }
    }
}
