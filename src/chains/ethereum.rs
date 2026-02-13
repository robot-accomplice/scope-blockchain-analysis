//! # Ethereum Client
//!
//! This module provides an Ethereum blockchain client that supports
//! Ethereum mainnet and EVM-compatible chains (Polygon, Arbitrum, etc.).
// Allow nested ifs for readability in API response parsing
#![allow(clippy::collapsible_if)]
//!
//! ## Features
//!
//! - Balance queries via block explorer APIs (with USD valuation via DexScreener)
//! - Transaction details lookup via Etherscan proxy API (`eth_getTransactionByHash`)
//! - Transaction receipt fetching for gas usage and status
//! - Transaction history retrieval
//! - ERC-20 token balance fetching (via `tokentx` + `tokenbalance` endpoints)
//! - Token holder count estimation with pagination
//! - Token information and holder analytics
//! - Support for both block explorer API and JSON-RPC modes
//!
//! ## Usage
//!
//! ```rust,no_run
//! use scope::chains::EthereumClient;
//! use scope::config::ChainsConfig;
//!
//! #[tokio::main]
//! async fn main() -> scope::Result<()> {
//!     let config = ChainsConfig::default();
//!     let client = EthereumClient::new(&config)?;
//!     
//!     let mut balance = client.get_balance("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2").await?;
//!     client.enrich_balance_usd(&mut balance).await;
//!     println!("Balance: {} (${:.2})", balance.formatted, balance.usd_value.unwrap_or(0.0));
//!     Ok(())
//! }
//! ```

use crate::chains::{Balance, ChainClient, Token, TokenBalance, TokenHolder, Transaction};
use crate::config::ChainsConfig;
use crate::error::{Result, ScopeError};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

/// Default Etherscan V2 API base URL.
///
/// All EVM chains (Ethereum, Polygon, Arbitrum, etc.) use this single
/// endpoint with a `chainid` query parameter to select the network.
const ETHERSCAN_V2_API: &str = "https://api.etherscan.io/v2/api";

/// Default JSON-RPC fallback for custom EVM chain.
const DEFAULT_AEGIS_RPC: &str = "http://localhost:8545";

/// API type for the client endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiType {
    /// Block explorer API (Etherscan-compatible).
    BlockExplorer,
    /// Direct JSON-RPC endpoint.
    JsonRpc,
}

/// Ethereum and EVM-compatible chain client.
///
/// Uses block explorer APIs (Etherscan, etc.) or JSON-RPC for data retrieval.
/// Supports multiple networks through configuration.
#[derive(Debug, Clone)]
pub struct EthereumClient {
    /// HTTP client for API requests.
    client: Client,

    /// Base URL for the block explorer API or JSON-RPC endpoint.
    base_url: String,

    /// Chain ID for Etherscan V2 API.
    chain_id: Option<String>,

    /// API key for the block explorer.
    api_key: Option<String>,

    /// Chain name for display purposes.
    chain_name: String,

    /// Native token symbol.
    native_symbol: String,

    /// Native token decimals.
    native_decimals: u8,

    /// Type of API endpoint (block explorer or JSON-RPC).
    api_type: ApiType,
}

/// Response from Etherscan-compatible APIs.
#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    status: String,
    message: String,
    result: T,
}

/// Balance response from API.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
#[allow(dead_code)] // Reserved for future error handling
enum BalanceResult {
    /// Successful balance string.
    Balance(String),
    /// Error message.
    Error(String),
}

/// Transaction list response from API.
#[derive(Debug, Deserialize)]
struct TxListItem {
    hash: String,
    #[serde(rename = "blockNumber")]
    block_number: String,
    #[serde(rename = "timeStamp")]
    timestamp: String,
    from: String,
    to: String,
    value: String,
    gas: String,
    #[serde(rename = "gasUsed")]
    gas_used: String,
    #[serde(rename = "gasPrice")]
    gas_price: String,
    nonce: String,
    input: String,
    #[serde(rename = "isError")]
    is_error: String,
}

/// Proxy API response wrapper (JSON-RPC style from Etherscan proxy endpoints).
#[derive(Debug, Deserialize)]
struct ProxyResponse<T> {
    result: Option<T>,
}

/// Transaction object from eth_getTransactionByHash proxy endpoint.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProxyTransaction {
    #[serde(default)]
    block_number: Option<String>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    gas: Option<String>,
    #[serde(default)]
    gas_price: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    input: Option<String>,
}

/// Transaction receipt from eth_getTransactionReceipt proxy endpoint.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProxyTransactionReceipt {
    #[serde(default)]
    gas_used: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// Token holder list item from API.
#[derive(Debug, Deserialize)]
struct TokenHolderItem {
    #[serde(rename = "TokenHolderAddress")]
    address: String,
    #[serde(rename = "TokenHolderQuantity")]
    quantity: String,
}

/// Token info response from API.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TokenInfoItem {
    #[serde(rename = "contractAddress")]
    contract_address: Option<String>,
    #[serde(rename = "tokenName")]
    token_name: Option<String>,
    #[serde(rename = "symbol")]
    symbol: Option<String>,
    #[serde(rename = "divisor")]
    divisor: Option<String>,
    #[serde(rename = "tokenType")]
    token_type: Option<String>,
    #[serde(rename = "totalSupply")]
    total_supply: Option<String>,
}

impl EthereumClient {
    /// Creates a new Ethereum client with the given configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - Chain configuration containing API keys and endpoints
    ///
    /// # Returns
    ///
    /// Returns a configured [`EthereumClient`] instance.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use scope::chains::EthereumClient;
    /// use scope::config::ChainsConfig;
    ///
    /// let config = ChainsConfig::default();
    /// let client = EthereumClient::new(&config).unwrap();
    /// ```
    pub fn new(config: &ChainsConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ScopeError::Chain(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            base_url: ETHERSCAN_V2_API.to_string(),
            chain_id: Some("1".to_string()),
            api_key: config.api_keys.get("etherscan").cloned(),
            chain_name: "ethereum".to_string(),
            native_symbol: "ETH".to_string(),
            native_decimals: 18,
            api_type: ApiType::BlockExplorer,
        })
    }

    /// Creates a client with a custom base URL (for testing or alternative networks).
    ///
    /// # Arguments
    ///
    /// * `base_url` - The base URL for the block explorer API
    pub fn with_base_url(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.to_string(),
            chain_id: None,
            api_key: None,
            chain_name: "ethereum".to_string(),
            native_symbol: "ETH".to_string(),
            native_decimals: 18,
            api_type: ApiType::BlockExplorer,
        }
    }

    /// Creates a client for a specific EVM chain.
    ///
    /// # Arguments
    ///
    /// * `chain` - Chain identifier (ethereum, polygon, arbitrum, optimism, base, bsc)
    /// * `config` - Chain configuration
    ///
    /// # Supported Chains
    ///
    /// - `ethereum` - Ethereum mainnet via Etherscan
    /// - `polygon` - Polygon via PolygonScan
    /// - `arbitrum` - Arbitrum One via Arbiscan
    /// - `optimism` - Optimism via Etherscan
    /// - `base` - Base via Basescan
    /// - `bsc` - BNB Smart Chain (BSC) via BscScan
    ///
    /// # API Version
    ///
    /// Uses Etherscan V2 API format which requires an API key for most endpoints.
    /// Get a free API key at <https://etherscan.io/apis>
    pub fn for_chain(chain: &str, config: &ChainsConfig) -> Result<Self> {
        // Etherscan V2 API uses chainid parameter
        // V2 format: https://api.etherscan.io/v2/api?chainid=X&module=...
        let (base_url, chain_id, api_key_name, symbol) = match chain {
            "ethereum" => (ETHERSCAN_V2_API, "1", "etherscan", "ETH"),
            "polygon" => (ETHERSCAN_V2_API, "137", "polygonscan", "MATIC"),
            "arbitrum" => (ETHERSCAN_V2_API, "42161", "arbiscan", "ETH"),
            "optimism" => (ETHERSCAN_V2_API, "10", "optimism", "ETH"),
            "base" => (ETHERSCAN_V2_API, "8453", "basescan", "ETH"),
            "bsc" => (ETHERSCAN_V2_API, "56", "bscscan", "BNB"),
            "aegis" => {
                // Aegis/Wraith uses direct JSON-RPC, not block explorer API.
                // Fall back to localhost if not configured.
                let rpc_url = config.aegis_rpc.as_deref().unwrap_or(DEFAULT_AEGIS_RPC);
                return Self::for_aegis(rpc_url, config);
            }
            _ => {
                return Err(ScopeError::Chain(format!("Unsupported chain: {}", chain)));
            }
        };

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ScopeError::Chain(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            base_url: base_url.to_string(),
            chain_id: Some(chain_id.to_string()),
            api_key: config.api_keys.get(api_key_name).cloned(),
            chain_name: chain.to_string(),
            native_symbol: symbol.to_string(),
            native_decimals: 18,
            api_type: ApiType::BlockExplorer,
        })
    }

    /// Creates a client for a custom EVM chain using JSON-RPC.
    ///
    /// # Arguments
    ///
    /// * `rpc_url` - The JSON-RPC endpoint URL
    /// * `_config` - Chain configuration (reserved for future use)
    fn for_aegis(rpc_url: &str, _config: &ChainsConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ScopeError::Chain(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            base_url: rpc_url.to_string(),
            chain_id: None,
            api_key: None,
            chain_name: "aegis".to_string(),
            native_symbol: "WRAITH".to_string(),
            native_decimals: 18,
            api_type: ApiType::JsonRpc,
        })
    }

    /// Returns the chain name.
    pub fn chain_name(&self) -> &str {
        &self.chain_name
    }

    /// Returns the native token symbol.
    pub fn native_token_symbol(&self) -> &str {
        &self.native_symbol
    }

    /// Builds an API URL with the chainid parameter for V2 API.
    fn build_api_url(&self, params: &str) -> String {
        let mut url = format!("{}?", self.base_url);

        // Add chainid for V2 API
        if let Some(ref chain_id) = self.chain_id {
            url.push_str(&format!("chainid={}&", chain_id));
        }

        url.push_str(params);

        // Add API key if available
        if let Some(ref key) = self.api_key {
            url.push_str(&format!("&apikey={}", key));
        }

        url
    }

    /// Fetches the native token balance for an address.
    ///
    /// # Arguments
    ///
    /// * `address` - The Ethereum address to query
    ///
    /// # Returns
    ///
    /// Returns a [`Balance`] struct with the balance in multiple formats.
    ///
    /// # Errors
    ///
    /// Returns [`ScopeError::InvalidAddress`] if the address format is invalid.
    /// Returns [`ScopeError::Request`] if the API request fails.
    pub async fn get_balance(&self, address: &str) -> Result<Balance> {
        // Validate address
        validate_eth_address(address)?;

        match self.api_type {
            ApiType::BlockExplorer => self.get_balance_explorer(address).await,
            ApiType::JsonRpc => self.get_balance_rpc(address).await,
        }
    }

    /// Fetches balance using block explorer API (Etherscan-compatible).
    async fn get_balance_explorer(&self, address: &str) -> Result<Balance> {
        let url = self.build_api_url(&format!(
            "module=account&action=balance&address={}&tag=latest",
            address
        ));

        tracing::debug!(url = %url, "Fetching balance via block explorer");

        let response: ApiResponse<String> = self.client.get(&url).send().await?.json().await?;

        if response.status != "1" {
            return Err(ScopeError::Chain(format!(
                "API error: {}",
                response.message
            )));
        }

        self.parse_balance_wei(&response.result)
    }

    /// Fetches balance using JSON-RPC (eth_getBalance).
    async fn get_balance_rpc(&self, address: &str) -> Result<Balance> {
        #[derive(serde::Serialize)]
        struct RpcRequest<'a> {
            jsonrpc: &'a str,
            method: &'a str,
            params: Vec<&'a str>,
            id: u64,
        }

        #[derive(Deserialize)]
        struct RpcResponse {
            result: Option<String>,
            error: Option<RpcError>,
        }

        #[derive(Deserialize)]
        struct RpcError {
            message: String,
        }

        let request = RpcRequest {
            jsonrpc: "2.0",
            method: "eth_getBalance",
            params: vec![address, "latest"],
            id: 1,
        };

        tracing::debug!(url = %self.base_url, address = %address, "Fetching balance via JSON-RPC");

        let response: RpcResponse = self
            .client
            .post(&self.base_url)
            .json(&request)
            .send()
            .await?
            .json()
            .await?;

        if let Some(error) = response.error {
            return Err(ScopeError::Chain(format!("RPC error: {}", error.message)));
        }

        let result = response
            .result
            .ok_or_else(|| ScopeError::Chain("Empty RPC response".to_string()))?;

        // Parse hex balance (e.g., "0x1234")
        let hex_balance = result.trim_start_matches("0x");
        let wei = u128::from_str_radix(hex_balance, 16)
            .map_err(|_| ScopeError::Chain("Invalid balance hex response".to_string()))?;

        self.parse_balance_wei(&wei.to_string())
    }

    /// Parses a wei balance string into a Balance struct.
    fn parse_balance_wei(&self, wei_str: &str) -> Result<Balance> {
        let wei: u128 = wei_str
            .parse()
            .map_err(|_| ScopeError::Chain("Invalid balance response".to_string()))?;

        let eth = wei as f64 / 10_f64.powi(self.native_decimals as i32);

        Ok(Balance {
            raw: wei_str.to_string(),
            formatted: format!("{:.6} {}", eth, self.native_symbol),
            decimals: self.native_decimals,
            symbol: self.native_symbol.clone(),
            usd_value: None, // Populated by caller via enrich_balance_usd
        })
    }

    /// Enriches a balance with a USD value using DexScreener price lookup.
    pub async fn enrich_balance_usd(&self, balance: &mut Balance) {
        let dex = crate::chains::DexClient::new();
        if let Some(price) = dex.get_native_token_price(&self.chain_name).await {
            let amount: f64 =
                balance.raw.parse().unwrap_or(0.0) / 10_f64.powi(self.native_decimals as i32);
            balance.usd_value = Some(amount * price);
        }
    }

    /// Fetches transaction details by hash.
    ///
    /// # Arguments
    ///
    /// * `hash` - The transaction hash
    ///
    /// # Returns
    ///
    /// Returns [`Transaction`] details.
    pub async fn get_transaction(&self, hash: &str) -> Result<Transaction> {
        // Validate hash
        validate_tx_hash(hash)?;

        match self.api_type {
            ApiType::BlockExplorer => self.get_transaction_explorer(hash).await,
            ApiType::JsonRpc => self.get_transaction_rpc(hash).await,
        }
    }

    /// Fetches transaction via Etherscan proxy API (eth_getTransactionByHash).
    async fn get_transaction_explorer(&self, hash: &str) -> Result<Transaction> {
        // Fetch the transaction object
        let tx_url = self.build_api_url(&format!(
            "module=proxy&action=eth_getTransactionByHash&txhash={}",
            hash
        ));

        tracing::debug!(url = %tx_url, "Fetching transaction via block explorer proxy");

        let tx_response: ProxyResponse<ProxyTransaction> =
            self.client.get(&tx_url).send().await?.json().await?;

        let proxy_tx = tx_response
            .result
            .ok_or_else(|| ScopeError::NotFound(format!("Transaction not found: {}", hash)))?;

        // Fetch the receipt for gas_used and status
        let receipt_url = self.build_api_url(&format!(
            "module=proxy&action=eth_getTransactionReceipt&txhash={}",
            hash
        ));

        tracing::debug!(url = %receipt_url, "Fetching transaction receipt");

        let receipt_response: ProxyResponse<ProxyTransactionReceipt> =
            self.client.get(&receipt_url).send().await?.json().await?;

        let receipt = receipt_response.result;

        // Parse block number from hex
        let block_number = proxy_tx
            .block_number
            .as_deref()
            .and_then(|bn| u64::from_str_radix(bn.trim_start_matches("0x"), 16).ok());

        // Parse gas limit from hex
        let gas_limit = proxy_tx
            .gas
            .as_deref()
            .and_then(|g| u64::from_str_radix(g.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);

        // Parse gas price from hex to decimal string
        let gas_price = proxy_tx
            .gas_price
            .as_deref()
            .and_then(|gp| u128::from_str_radix(gp.trim_start_matches("0x"), 16).ok())
            .map(|gp| gp.to_string())
            .unwrap_or_else(|| "0".to_string());

        // Parse nonce from hex
        let nonce = proxy_tx
            .nonce
            .as_deref()
            .and_then(|n| u64::from_str_radix(n.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);

        // Parse value from hex wei to decimal string
        let value = proxy_tx
            .value
            .as_deref()
            .and_then(|v| u128::from_str_radix(v.trim_start_matches("0x"), 16).ok())
            .map(|v| v.to_string())
            .unwrap_or_else(|| "0".to_string());

        // Parse receipt fields
        let gas_used = receipt.as_ref().and_then(|r| {
            r.gas_used
                .as_deref()
                .and_then(|gu| u64::from_str_radix(gu.trim_start_matches("0x"), 16).ok())
        });

        let status = receipt
            .as_ref()
            .and_then(|r| r.status.as_deref().map(|s| s == "0x1"));

        // Get block timestamp if we have a block number
        let timestamp = if let Some(bn) = block_number {
            self.get_block_timestamp(bn).await.ok()
        } else {
            None
        };

        Ok(Transaction {
            hash: hash.to_string(),
            block_number,
            timestamp,
            from: proxy_tx.from.unwrap_or_default(),
            to: proxy_tx.to,
            value,
            gas_limit,
            gas_used,
            gas_price,
            nonce,
            input: proxy_tx.input.unwrap_or_else(|| "0x".to_string()),
            status,
        })
    }

    /// Fetches transaction via JSON-RPC (eth_getTransactionByHash).
    async fn get_transaction_rpc(&self, hash: &str) -> Result<Transaction> {
        #[derive(serde::Serialize)]
        struct RpcRequest<'a> {
            jsonrpc: &'a str,
            method: &'a str,
            params: Vec<&'a str>,
            id: u64,
        }

        let request = RpcRequest {
            jsonrpc: "2.0",
            method: "eth_getTransactionByHash",
            params: vec![hash],
            id: 1,
        };

        let response: ProxyResponse<ProxyTransaction> = self
            .client
            .post(&self.base_url)
            .json(&request)
            .send()
            .await?
            .json()
            .await?;

        let proxy_tx = response
            .result
            .ok_or_else(|| ScopeError::NotFound(format!("Transaction not found: {}", hash)))?;

        // Also fetch receipt
        let receipt_request = RpcRequest {
            jsonrpc: "2.0",
            method: "eth_getTransactionReceipt",
            params: vec![hash],
            id: 2,
        };

        let receipt_response: ProxyResponse<ProxyTransactionReceipt> = self
            .client
            .post(&self.base_url)
            .json(&receipt_request)
            .send()
            .await?
            .json()
            .await?;

        let receipt = receipt_response.result;

        let block_number = proxy_tx
            .block_number
            .as_deref()
            .and_then(|bn| u64::from_str_radix(bn.trim_start_matches("0x"), 16).ok());

        let gas_limit = proxy_tx
            .gas
            .as_deref()
            .and_then(|g| u64::from_str_radix(g.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);

        let gas_price = proxy_tx
            .gas_price
            .as_deref()
            .and_then(|gp| u128::from_str_radix(gp.trim_start_matches("0x"), 16).ok())
            .map(|gp| gp.to_string())
            .unwrap_or_else(|| "0".to_string());

        let nonce = proxy_tx
            .nonce
            .as_deref()
            .and_then(|n| u64::from_str_radix(n.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);

        let value = proxy_tx
            .value
            .as_deref()
            .and_then(|v| u128::from_str_radix(v.trim_start_matches("0x"), 16).ok())
            .map(|v| v.to_string())
            .unwrap_or_else(|| "0".to_string());

        let gas_used = receipt.as_ref().and_then(|r| {
            r.gas_used
                .as_deref()
                .and_then(|gu| u64::from_str_radix(gu.trim_start_matches("0x"), 16).ok())
        });

        let status = receipt
            .as_ref()
            .and_then(|r| r.status.as_deref().map(|s| s == "0x1"));

        Ok(Transaction {
            hash: hash.to_string(),
            block_number,
            timestamp: None, // JSON-RPC doesn't easily give us timestamp without another call
            from: proxy_tx.from.unwrap_or_default(),
            to: proxy_tx.to,
            value,
            gas_limit,
            gas_used,
            gas_price,
            nonce,
            input: proxy_tx.input.unwrap_or_else(|| "0x".to_string()),
            status,
        })
    }

    /// Fetches block timestamp for a given block number.
    async fn get_block_timestamp(&self, block_number: u64) -> Result<u64> {
        let hex_block = format!("0x{:x}", block_number);
        let url = self.build_api_url(&format!(
            "module=proxy&action=eth_getBlockByNumber&tag={}&boolean=false",
            hex_block
        ));

        #[derive(Deserialize)]
        struct BlockResult {
            timestamp: Option<String>,
        }

        let response: ProxyResponse<BlockResult> =
            self.client.get(&url).send().await?.json().await?;

        let block = response
            .result
            .ok_or_else(|| ScopeError::Chain(format!("Block not found: {}", block_number)))?;

        block
            .timestamp
            .as_deref()
            .and_then(|ts| u64::from_str_radix(ts.trim_start_matches("0x"), 16).ok())
            .ok_or_else(|| ScopeError::Chain("Invalid block timestamp".to_string()))
    }

    /// Fetches recent transactions for an address.
    ///
    /// # Arguments
    ///
    /// * `address` - The address to query
    /// * `limit` - Maximum number of transactions
    ///
    /// # Returns
    ///
    /// Returns a vector of [`Transaction`] objects.
    pub async fn get_transactions(&self, address: &str, limit: u32) -> Result<Vec<Transaction>> {
        validate_eth_address(address)?;

        let url = self.build_api_url(&format!(
            "module=account&action=txlist&address={}&startblock=0&endblock=99999999&page=1&offset={}&sort=desc",
            address, limit
        ));

        tracing::debug!(url = %url, "Fetching transactions");

        let response: ApiResponse<Vec<TxListItem>> =
            self.client.get(&url).send().await?.json().await?;

        if response.status != "1" && response.message != "No transactions found" {
            return Err(ScopeError::Chain(format!(
                "API error: {}",
                response.message
            )));
        }

        let transactions = response
            .result
            .into_iter()
            .map(|tx| Transaction {
                hash: tx.hash,
                block_number: tx.block_number.parse().ok(),
                timestamp: tx.timestamp.parse().ok(),
                from: tx.from,
                to: if tx.to.is_empty() { None } else { Some(tx.to) },
                value: tx.value,
                gas_limit: tx.gas.parse().unwrap_or(0),
                gas_used: tx.gas_used.parse().ok(),
                gas_price: tx.gas_price,
                nonce: tx.nonce.parse().unwrap_or(0),
                input: tx.input,
                status: Some(tx.is_error == "0"),
            })
            .collect();

        Ok(transactions)
    }

    /// Fetches the current block number.
    pub async fn get_block_number(&self) -> Result<u64> {
        let url = self.build_api_url("module=proxy&action=eth_blockNumber");

        #[derive(Deserialize)]
        struct BlockResponse {
            result: String,
        }

        let response: BlockResponse = self.client.get(&url).send().await?.json().await?;

        // Parse hex block number
        let block_hex = response.result.trim_start_matches("0x");
        let block_number = u64::from_str_radix(block_hex, 16)
            .map_err(|_| ScopeError::Chain("Invalid block number response".to_string()))?;

        Ok(block_number)
    }

    /// Fetches ERC-20 token balances for an address.
    ///
    /// Uses Etherscan's tokentx endpoint to find unique tokens the address
    /// has interacted with, then fetches current balances for each.
    pub async fn get_erc20_balances(
        &self,
        address: &str,
    ) -> Result<Vec<crate::chains::TokenBalance>> {
        validate_eth_address(address)?;

        // Step 1: Get recent ERC-20 token transfers to find unique tokens
        let url = self.build_api_url(&format!(
            "module=account&action=tokentx&address={}&page=1&offset=100&sort=desc",
            address
        ));

        tracing::debug!(url = %url, "Fetching ERC-20 token transfers");

        let response = self.client.get(&url).send().await?.text().await?;

        #[derive(Deserialize)]
        struct TokenTxItem {
            #[serde(rename = "contractAddress")]
            contract_address: String,
            #[serde(rename = "tokenSymbol")]
            token_symbol: String,
            #[serde(rename = "tokenName")]
            token_name: String,
            #[serde(rename = "tokenDecimal")]
            token_decimal: String,
        }

        let parsed: std::result::Result<ApiResponse<Vec<TokenTxItem>>, _> =
            serde_json::from_str(&response);

        let token_txs = match parsed {
            Ok(api_resp) if api_resp.status == "1" => api_resp.result,
            _ => return Ok(vec![]),
        };

        // Step 2: Deduplicate by contract address
        let mut seen = std::collections::HashSet::new();
        let unique_tokens: Vec<&TokenTxItem> = token_txs
            .iter()
            .filter(|tx| seen.insert(tx.contract_address.to_lowercase()))
            .collect();

        // Step 3: Fetch current balance for each unique token
        let mut balances = Vec::new();
        for token_tx in unique_tokens.iter().take(20) {
            // Cap at 20 to avoid rate limits
            let balance_url = self.build_api_url(&format!(
                "module=account&action=tokenbalance&contractaddress={}&address={}&tag=latest",
                token_tx.contract_address, address
            ));

            if let Ok(resp) = self.client.get(&balance_url).send().await {
                if let Ok(bal_resp) = resp.json::<ApiResponse<String>>().await {
                    if bal_resp.status == "1" {
                        let raw_balance = bal_resp.result;
                        let decimals: u8 = token_tx.token_decimal.parse().unwrap_or(18);

                        // Skip zero balances
                        if raw_balance == "0" {
                            continue;
                        }

                        let formatted = format_token_balance(&raw_balance, decimals);

                        balances.push(crate::chains::TokenBalance {
                            token: Token {
                                contract_address: token_tx.contract_address.clone(),
                                symbol: token_tx.token_symbol.clone(),
                                name: token_tx.token_name.clone(),
                                decimals,
                            },
                            balance: raw_balance,
                            formatted_balance: formatted,
                            usd_value: None,
                        });
                    }
                }
            }
        }

        Ok(balances)
    }

    /// Fetches token information for a contract address.
    ///
    /// # Arguments
    ///
    /// * `token_address` - The token contract address
    ///
    /// # Returns
    ///
    /// Returns [`Token`] information including name, symbol, and decimals.
    pub async fn get_token_info(&self, token_address: &str) -> Result<Token> {
        validate_eth_address(token_address)?;

        // Try the Pro API tokeninfo endpoint first
        let url = self.build_api_url(&format!(
            "module=token&action=tokeninfo&contractaddress={}",
            token_address
        ));

        tracing::debug!(url = %url, "Fetching token info (Pro API)");

        let response = self.client.get(&url).send().await?;
        let response_text = response.text().await?;

        // Try to parse as successful response
        if let Ok(api_response) =
            serde_json::from_str::<ApiResponse<Vec<TokenInfoItem>>>(&response_text)
        {
            if api_response.status == "1" && !api_response.result.is_empty() {
                let info = &api_response.result[0];
                let decimals = info
                    .divisor
                    .as_ref()
                    .and_then(|d| d.parse::<u32>().ok())
                    .map(|d| (d as f64).log10() as u8)
                    .unwrap_or(18);

                return Ok(Token {
                    contract_address: token_address.to_string(),
                    symbol: info.symbol.clone().unwrap_or_else(|| "UNKNOWN".to_string()),
                    name: info
                        .token_name
                        .clone()
                        .unwrap_or_else(|| "Unknown Token".to_string()),
                    decimals,
                });
            }
        }

        // Fall back to tokensupply endpoint (free) to verify it's a valid ERC20
        // and get the total supply
        self.get_token_info_from_supply(token_address).await
    }

    /// Gets basic token info using the tokensupply endpoint.
    async fn get_token_info_from_supply(&self, token_address: &str) -> Result<Token> {
        let url = self.build_api_url(&format!(
            "module=stats&action=tokensupply&contractaddress={}",
            token_address
        ));

        tracing::debug!(url = %url, "Fetching token supply");

        let response = self.client.get(&url).send().await?;
        let response_text = response.text().await?;

        // Check if the token supply call succeeded (indicates valid ERC20)
        if let Ok(api_response) = serde_json::from_str::<ApiResponse<String>>(&response_text) {
            if api_response.status == "1" {
                // Valid ERC20 token, but we don't have name/symbol
                // Try to get them from contract source if verified
                if let Some(contract_info) = self.try_get_contract_name(token_address).await {
                    return Ok(Token {
                        contract_address: token_address.to_string(),
                        symbol: contract_info.0,
                        name: contract_info.1,
                        decimals: 18,
                    });
                }

                // Return with address-based placeholder
                let short_addr = format!(
                    "{}...{}",
                    &token_address[..6],
                    &token_address[token_address.len() - 4..]
                );
                return Ok(Token {
                    contract_address: token_address.to_string(),
                    symbol: short_addr.clone(),
                    name: format!("Token {}", short_addr),
                    decimals: 18,
                });
            }
        }

        // Not a valid ERC20 token
        Ok(Token {
            contract_address: token_address.to_string(),
            symbol: "UNKNOWN".to_string(),
            name: "Unknown Token".to_string(),
            decimals: 18,
        })
    }

    /// Tries to get contract name from verified source code.
    async fn try_get_contract_name(&self, token_address: &str) -> Option<(String, String)> {
        let url = self.build_api_url(&format!(
            "module=contract&action=getsourcecode&address={}",
            token_address
        ));

        let response = self.client.get(&url).send().await.ok()?;
        let text = response.text().await.ok()?;

        // Parse the response to get ContractName
        #[derive(serde::Deserialize)]
        struct SourceCodeResult {
            #[serde(rename = "ContractName")]
            contract_name: Option<String>,
        }

        #[derive(serde::Deserialize)]
        struct SourceCodeResponse {
            status: String,
            result: Vec<SourceCodeResult>,
        }

        if let Ok(api_response) = serde_json::from_str::<SourceCodeResponse>(&text) {
            if api_response.status == "1" && !api_response.result.is_empty() {
                if let Some(name) = &api_response.result[0].contract_name {
                    if !name.is_empty() {
                        // Use contract name as both symbol and name
                        // Try to extract symbol from name (often in format "NameToken" or "NAME")
                        let symbol = if name.len() <= 6 {
                            name.to_uppercase()
                        } else {
                            // Take first letters that are uppercase
                            name.chars()
                                .filter(|c| c.is_uppercase())
                                .take(6)
                                .collect::<String>()
                        };
                        let symbol = if symbol.is_empty() {
                            name[..name.len().min(6)].to_uppercase()
                        } else {
                            symbol
                        };
                        return Some((symbol, name.clone()));
                    }
                }
            }
        }

        None
    }

    /// Fetches the top token holders for a given token.
    ///
    /// # Arguments
    ///
    /// * `token_address` - The token contract address
    /// * `limit` - Maximum number of holders to return (max 1000 for most APIs)
    ///
    /// # Returns
    ///
    /// Returns a vector of [`TokenHolder`] objects sorted by balance.
    ///
    /// # Note
    ///
    /// This requires an Etherscan Pro API key for most networks.
    pub async fn get_token_holders(
        &self,
        token_address: &str,
        limit: u32,
    ) -> Result<Vec<TokenHolder>> {
        validate_eth_address(token_address)?;

        let effective_limit = limit.min(1000); // API max is typically 1000

        let url = self.build_api_url(&format!(
            "module=token&action=tokenholderlist&contractaddress={}&page=1&offset={}",
            token_address, effective_limit
        ));

        tracing::debug!(url = %url, "Fetching token holders");

        let response = self.client.get(&url).send().await?;
        let response_text = response.text().await?;

        // Parse the response
        let api_response: ApiResponse<serde_json::Value> = serde_json::from_str(&response_text)
            .map_err(|e| ScopeError::Api(format!("Failed to parse holder response: {}", e)))?;

        if api_response.status != "1" {
            // Check for common error messages
            if api_response.message.contains("Pro")
                || api_response.message.contains("API")
                || api_response.message.contains("NOTOK")
            {
                tracing::warn!(
                    "Token holder API requires Pro key or is unavailable: {}",
                    api_response.message
                );
                return Ok(Vec::new());
            }
            return Err(ScopeError::Api(format!(
                "API error: {}",
                api_response.message
            )));
        }

        // Parse the holder list
        let holders: Vec<TokenHolderItem> = serde_json::from_value(api_response.result)
            .map_err(|e| ScopeError::Api(format!("Failed to parse holder list: {}", e)))?;

        // Calculate total supply for percentage calculation
        let total_balance: f64 = holders
            .iter()
            .filter_map(|h| h.quantity.parse::<f64>().ok())
            .sum();

        // Convert to TokenHolder structs
        let token_holders: Vec<TokenHolder> = holders
            .into_iter()
            .enumerate()
            .map(|(i, h)| {
                let balance: f64 = h.quantity.parse().unwrap_or(0.0);
                let percentage = if total_balance > 0.0 {
                    (balance / total_balance) * 100.0
                } else {
                    0.0
                };

                TokenHolder {
                    address: h.address,
                    balance: h.quantity.clone(),
                    formatted_balance: format_token_balance(&h.quantity, 18), // Default to 18 decimals
                    percentage,
                    rank: (i + 1) as u32,
                }
            })
            .collect();

        Ok(token_holders)
    }

    /// Gets the total holder count for a token.
    ///
    /// Uses Etherscan token holder list endpoint to estimate the count.
    /// If the API returns a full page at the max limit, the count is approximate.
    pub async fn get_token_holder_count(&self, token_address: &str) -> Result<u64> {
        // First try to get a large page of holders - the page size tells us if there are more
        let max_page_size: u32 = 1000;
        let holders = self.get_token_holders(token_address, max_page_size).await?;

        if holders.is_empty() {
            return Ok(0);
        }

        let count = holders.len() as u64;

        if count < max_page_size as u64 {
            // We got all holders - this is the exact count
            Ok(count)
        } else {
            // The result was capped - there are at least this many holders.
            // Try fetching additional pages to refine the estimate.
            let mut total = count;
            let mut page = 2u32;
            loop {
                let url = self.build_api_url(&format!(
                    "module=token&action=tokenholderlist&contractaddress={}&page={}&offset={}",
                    token_address, page, max_page_size
                ));
                let response: std::result::Result<ApiResponse<Vec<TokenHolderItem>>, _> =
                    self.client.get(&url).send().await?.json().await;

                match response {
                    Ok(api_resp) if api_resp.status == "1" => {
                        let page_count = api_resp.result.len() as u64;
                        total += page_count;
                        if page_count < max_page_size as u64 || page >= 10 {
                            // Got a partial page (end of list) or hit our max pages limit
                            break;
                        }
                        page += 1;
                    }
                    _ => break,
                }
            }
            Ok(total)
        }
    }
}

/// Formats a token balance with proper decimal places.
fn format_token_balance(balance: &str, decimals: u8) -> String {
    let balance_f64: f64 = balance.parse().unwrap_or(0.0);
    let divisor = 10_f64.powi(decimals as i32);
    let formatted = balance_f64 / divisor;

    if formatted >= 1_000_000_000.0 {
        format!("{:.2}B", formatted / 1_000_000_000.0)
    } else if formatted >= 1_000_000.0 {
        format!("{:.2}M", formatted / 1_000_000.0)
    } else if formatted >= 1_000.0 {
        format!("{:.2}K", formatted / 1_000.0)
    } else {
        format!("{:.4}", formatted)
    }
}

impl Default for EthereumClient {
    fn default() -> Self {
        Self {
            client: Client::new(),
            base_url: ETHERSCAN_V2_API.to_string(),
            chain_id: Some("1".to_string()),
            api_key: None,
            chain_name: "ethereum".to_string(),
            native_symbol: "ETH".to_string(),
            native_decimals: 18,
            api_type: ApiType::BlockExplorer,
        }
    }
}

/// Validates an Ethereum address format.
fn validate_eth_address(address: &str) -> Result<()> {
    if !address.starts_with("0x") {
        return Err(ScopeError::InvalidAddress(format!(
            "Address must start with '0x': {}",
            address
        )));
    }
    if address.len() != 42 {
        return Err(ScopeError::InvalidAddress(format!(
            "Address must be 42 characters: {}",
            address
        )));
    }
    if !address[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ScopeError::InvalidAddress(format!(
            "Address contains invalid hex characters: {}",
            address
        )));
    }
    Ok(())
}

/// Validates a transaction hash format.
fn validate_tx_hash(hash: &str) -> Result<()> {
    if !hash.starts_with("0x") {
        return Err(ScopeError::InvalidHash(format!(
            "Hash must start with '0x': {}",
            hash
        )));
    }
    if hash.len() != 66 {
        return Err(ScopeError::InvalidHash(format!(
            "Hash must be 66 characters: {}",
            hash
        )));
    }
    if !hash[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ScopeError::InvalidHash(format!(
            "Hash contains invalid hex characters: {}",
            hash
        )));
    }
    Ok(())
}

// ============================================================================
// ChainClient Trait Implementation
// ============================================================================

#[async_trait]
impl ChainClient for EthereumClient {
    fn chain_name(&self) -> &str {
        &self.chain_name
    }

    fn native_token_symbol(&self) -> &str {
        &self.native_symbol
    }

    async fn get_balance(&self, address: &str) -> Result<Balance> {
        self.get_balance(address).await
    }

    async fn enrich_balance_usd(&self, balance: &mut Balance) {
        self.enrich_balance_usd(balance).await
    }

    async fn get_transaction(&self, hash: &str) -> Result<Transaction> {
        self.get_transaction(hash).await
    }

    async fn get_transactions(&self, address: &str, limit: u32) -> Result<Vec<Transaction>> {
        self.get_transactions(address, limit).await
    }

    async fn get_block_number(&self) -> Result<u64> {
        self.get_block_number().await
    }

    async fn get_token_balances(&self, address: &str) -> Result<Vec<TokenBalance>> {
        self.get_erc20_balances(address).await
    }

    async fn get_token_info(&self, address: &str) -> Result<Token> {
        self.get_token_info(address).await
    }

    async fn get_token_holders(&self, address: &str, limit: u32) -> Result<Vec<TokenHolder>> {
        self.get_token_holders(address, limit).await
    }

    async fn get_token_holder_count(&self, address: &str) -> Result<u64> {
        self.get_token_holder_count(address).await
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_ADDRESS: &str = "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2";
    const VALID_TX_HASH: &str =
        "0xabc123def456789012345678901234567890123456789012345678901234abcd";

    #[test]
    fn test_validate_eth_address_valid() {
        assert!(validate_eth_address(VALID_ADDRESS).is_ok());
    }

    #[test]
    fn test_validate_eth_address_lowercase() {
        let addr = "0x742d35cc6634c0532925a3b844bc9e7595f1b3c2";
        assert!(validate_eth_address(addr).is_ok());
    }

    #[test]
    fn test_validate_eth_address_missing_prefix() {
        let addr = "742d35Cc6634C0532925a3b844Bc9e7595f1b3c2";
        let result = validate_eth_address(addr);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("0x"));
    }

    #[test]
    fn test_validate_eth_address_too_short() {
        let addr = "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3";
        let result = validate_eth_address(addr);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("42 characters"));
    }

    #[test]
    fn test_validate_eth_address_invalid_hex() {
        let addr = "0x742d35Cc6634C0532925a3b844Bc9e7595f1bXYZ";
        let result = validate_eth_address(addr);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid hex"));
    }

    #[test]
    fn test_validate_tx_hash_valid() {
        assert!(validate_tx_hash(VALID_TX_HASH).is_ok());
    }

    #[test]
    fn test_validate_tx_hash_missing_prefix() {
        let hash = "abc123def456789012345678901234567890123456789012345678901234abcd";
        let result = validate_tx_hash(hash);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_tx_hash_too_short() {
        let hash = "0xabc123";
        let result = validate_tx_hash(hash);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("66 characters"));
    }

    #[test]
    fn test_ethereum_client_default() {
        let client = EthereumClient::default();
        assert_eq!(client.chain_name(), "ethereum");
        assert_eq!(client.native_token_symbol(), "ETH");
    }

    #[test]
    fn test_ethereum_client_with_base_url() {
        let client = EthereumClient::with_base_url("https://custom.api.com");
        assert_eq!(client.base_url, "https://custom.api.com");
    }

    #[test]
    fn test_ethereum_client_for_chain_ethereum() {
        let config = ChainsConfig::default();
        let client = EthereumClient::for_chain("ethereum", &config).unwrap();
        assert_eq!(client.chain_name(), "ethereum");
        assert_eq!(client.native_token_symbol(), "ETH");
    }

    #[test]
    fn test_ethereum_client_for_chain_polygon() {
        let config = ChainsConfig::default();
        let client = EthereumClient::for_chain("polygon", &config).unwrap();
        assert_eq!(client.chain_name(), "polygon");
        assert_eq!(client.native_token_symbol(), "MATIC");
        // V2 API uses unified URL with chainid parameter
        assert!(client.base_url.contains("etherscan.io/v2"));
        assert_eq!(client.chain_id, Some("137".to_string()));
    }

    #[test]
    fn test_ethereum_client_for_chain_arbitrum() {
        let config = ChainsConfig::default();
        let client = EthereumClient::for_chain("arbitrum", &config).unwrap();
        assert_eq!(client.chain_name(), "arbitrum");
        // V2 API uses unified URL with chainid parameter
        assert!(client.base_url.contains("etherscan.io/v2"));
        assert_eq!(client.chain_id, Some("42161".to_string()));
    }

    #[test]
    fn test_ethereum_client_for_chain_bsc() {
        let config = ChainsConfig::default();
        let client = EthereumClient::for_chain("bsc", &config).unwrap();
        assert_eq!(client.chain_name(), "bsc");
        assert_eq!(client.native_token_symbol(), "BNB");
        // V2 API uses unified URL with chainid parameter
        assert!(client.base_url.contains("etherscan.io/v2"));
        assert_eq!(client.chain_id, Some("56".to_string()));
        assert_eq!(client.api_type, ApiType::BlockExplorer);
    }

    #[test]
    fn test_ethereum_client_for_chain_aegis() {
        let config = ChainsConfig::default();
        let client = EthereumClient::for_chain("aegis", &config).unwrap();
        assert_eq!(client.chain_name(), "aegis");
        assert_eq!(client.native_token_symbol(), "WRAITH");
        assert_eq!(client.api_type, ApiType::JsonRpc);
        // Default URL when not configured
        assert!(client.base_url.contains("localhost:8545"));
    }

    #[test]
    fn test_ethereum_client_for_chain_aegis_with_config() {
        let config = ChainsConfig {
            aegis_rpc: Some("https://aegis.example.com:8545".to_string()),
            ..Default::default()
        };
        let client = EthereumClient::for_chain("aegis", &config).unwrap();
        assert_eq!(client.base_url, "https://aegis.example.com:8545");
    }

    #[test]
    fn test_ethereum_client_for_chain_unsupported() {
        let config = ChainsConfig::default();
        let result = EthereumClient::for_chain("bitcoin", &config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unsupported chain")
        );
    }

    #[test]
    fn test_ethereum_client_new() {
        let config = ChainsConfig::default();
        let client = EthereumClient::new(&config);
        assert!(client.is_ok());
    }

    #[test]
    fn test_ethereum_client_with_api_key() {
        use std::collections::HashMap;

        let mut api_keys = HashMap::new();
        api_keys.insert("etherscan".to_string(), "test-key".to_string());

        let config = ChainsConfig {
            api_keys,
            ..Default::default()
        };

        let client = EthereumClient::new(&config).unwrap();
        assert_eq!(client.api_key, Some("test-key".to_string()));
    }

    #[test]
    fn test_api_response_deserialization() {
        let json = r#"{"status":"1","message":"OK","result":"1000000000000000000"}"#;
        let response: ApiResponse<String> = serde_json::from_str(json).unwrap();
        assert_eq!(response.status, "1");
        assert_eq!(response.message, "OK");
        assert_eq!(response.result, "1000000000000000000");
    }

    #[test]
    fn test_tx_list_item_deserialization() {
        let json = r#"{
            "hash": "0xabc",
            "blockNumber": "12345",
            "timeStamp": "1700000000",
            "from": "0xfrom",
            "to": "0xto",
            "value": "1000000000000000000",
            "gas": "21000",
            "gasUsed": "21000",
            "gasPrice": "20000000000",
            "nonce": "42",
            "input": "0x",
            "isError": "0"
        }"#;

        let item: TxListItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.hash, "0xabc");
        assert_eq!(item.block_number, "12345");
        assert_eq!(item.nonce, "42");
        assert_eq!(item.is_error, "0");
    }

    // ========================================================================
    // Pure function tests
    // ========================================================================

    #[test]
    fn test_parse_balance_wei_valid() {
        let client = EthereumClient::default();
        let balance = client.parse_balance_wei("1000000000000000000").unwrap();
        assert_eq!(balance.symbol, "ETH");
        assert_eq!(balance.raw, "1000000000000000000");
        assert!(balance.formatted.contains("1.000000"));
        assert!(balance.usd_value.is_none());
    }

    #[test]
    fn test_parse_balance_wei_zero() {
        let client = EthereumClient::default();
        let balance = client.parse_balance_wei("0").unwrap();
        assert!(balance.formatted.contains("0.000000"));
    }

    #[test]
    fn test_parse_balance_wei_invalid() {
        let client = EthereumClient::default();
        let result = client.parse_balance_wei("not_a_number");
        assert!(result.is_err());
    }

    #[test]
    fn test_format_token_balance_large() {
        assert!(format_token_balance("1000000000000000000000000000", 18).contains("B"));
    }

    #[test]
    fn test_format_token_balance_millions() {
        assert!(format_token_balance("5000000000000000000000000", 18).contains("M"));
    }

    #[test]
    fn test_format_token_balance_thousands() {
        assert!(format_token_balance("5000000000000000000000", 18).contains("K"));
    }

    #[test]
    fn test_format_token_balance_small() {
        let formatted = format_token_balance("500000000000000000", 18);
        assert!(formatted.contains("0.5"));
    }

    #[test]
    fn test_format_token_balance_zero() {
        let formatted = format_token_balance("0", 18);
        assert!(formatted.contains("0.0000"));
    }

    #[test]
    fn test_build_api_url_with_chain_id_and_key() {
        use std::collections::HashMap;
        let mut keys = HashMap::new();
        keys.insert("etherscan".to_string(), "MYKEY".to_string());
        let config = ChainsConfig {
            api_keys: keys,
            ..Default::default()
        };
        let client = EthereumClient::new(&config).unwrap();
        let url = client.build_api_url("module=account&action=balance&address=0x123");
        assert!(url.contains("chainid=1"));
        assert!(url.contains("module=account"));
        assert!(url.contains("apikey=MYKEY"));
    }

    #[test]
    fn test_build_api_url_no_chain_id_no_key() {
        let client = EthereumClient::with_base_url("https://example.com/api");
        let url = client.build_api_url("module=account&action=balance");
        assert_eq!(url, "https://example.com/api?module=account&action=balance");
        assert!(!url.contains("chainid"));
        assert!(!url.contains("apikey"));
    }

    // ========================================================================
    // HTTP mocking tests - Block Explorer API
    // ========================================================================

    #[tokio::test]
    async fn test_get_balance_explorer() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"1","message":"OK","result":"2500000000000000000"}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let balance = client.get_balance(VALID_ADDRESS).await.unwrap();
        assert_eq!(balance.raw, "2500000000000000000");
        assert_eq!(balance.symbol, "ETH");
        assert!(balance.formatted.contains("2.5"));
    }

    #[tokio::test]
    async fn test_get_balance_explorer_api_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"0","message":"NOTOK","result":"Max rate limit reached"}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let result = client.get_balance(VALID_ADDRESS).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("API error"));
    }

    #[tokio::test]
    async fn test_get_balance_invalid_address() {
        let client = EthereumClient::default();
        let result = client.get_balance("invalid").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_balance_rpc() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":"0xDE0B6B3A7640000"}"#)
            .create_async()
            .await;

        let client = EthereumClient {
            client: Client::new(),
            base_url: server.url(),
            chain_id: None,
            api_key: None,
            chain_name: "aegis".to_string(),
            native_symbol: "WRAITH".to_string(),
            native_decimals: 18,
            api_type: ApiType::JsonRpc,
        };
        let balance = client.get_balance(VALID_ADDRESS).await.unwrap();
        assert_eq!(balance.symbol, "WRAITH");
        assert!(balance.formatted.contains("1.000000"));
    }

    #[tokio::test]
    async fn test_get_balance_rpc_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"execution reverted"}}"#)
            .create_async()
            .await;

        let client = EthereumClient {
            client: Client::new(),
            base_url: server.url(),
            chain_id: None,
            api_key: None,
            chain_name: "aegis".to_string(),
            native_symbol: "WRAITH".to_string(),
            native_decimals: 18,
            api_type: ApiType::JsonRpc,
        };
        let result = client.get_balance(VALID_ADDRESS).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("RPC error"));
    }

    #[tokio::test]
    async fn test_get_balance_rpc_empty_result() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1}"#)
            .create_async()
            .await;

        let client = EthereumClient {
            client: Client::new(),
            base_url: server.url(),
            chain_id: None,
            api_key: None,
            chain_name: "aegis".to_string(),
            native_symbol: "WRAITH".to_string(),
            native_decimals: 18,
            api_type: ApiType::JsonRpc,
        };
        let result = client.get_balance(VALID_ADDRESS).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Empty RPC response")
        );
    }

    #[tokio::test]
    async fn test_get_transaction_explorer() {
        let mut server = mockito::Server::new_async().await;
        // The explorer makes 3 sequential requests: tx, receipt, block
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"jsonrpc":"2.0","id":1,"result":{
                "hash":"0xabc123def456789012345678901234567890123456789012345678901234abcd",
                "from":"0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
                "to":"0x1111111111111111111111111111111111111111",
                "blockNumber":"0x10",
                "gas":"0x5208",
                "gasPrice":"0x4A817C800",
                "nonce":"0x2A",
                "value":"0xDE0B6B3A7640000",
                "input":"0x"
            }}"#,
            )
            .expect_at_most(1)
            .create_async()
            .await;

        // Receipt response
        let _receipt_mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"jsonrpc":"2.0","id":1,"result":{
                "gasUsed":"0x5208",
                "status":"0x1"
            }}"#,
            )
            .expect_at_most(1)
            .create_async()
            .await;

        // Block timestamp response
        let _block_mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"jsonrpc":"2.0","id":1,"result":{
                "timestamp":"0x65A8C580"
            }}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let tx = client.get_transaction(VALID_TX_HASH).await.unwrap();
        assert_eq!(tx.hash, VALID_TX_HASH);
        assert_eq!(tx.from, "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2");
        assert_eq!(
            tx.to,
            Some("0x1111111111111111111111111111111111111111".to_string())
        );
        assert!(tx.gas_limit > 0);
        assert!(tx.nonce > 0);
    }

    #[tokio::test]
    async fn test_get_transaction_explorer_not_found() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":null}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let result = client.get_transaction(VALID_TX_HASH).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_get_transaction_rpc() {
        let mut server = mockito::Server::new_async().await;
        // Transaction response
        let _tx_mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"jsonrpc":"2.0","id":1,"result":{
                "hash":"0xabc123def456789012345678901234567890123456789012345678901234abcd",
                "from":"0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
                "to":"0x1111111111111111111111111111111111111111",
                "blockNumber":"0x10",
                "gas":"0x5208",
                "gasPrice":"0x4A817C800",
                "nonce":"0x2A",
                "value":"0xDE0B6B3A7640000",
                "input":"0x"
            }}"#,
            )
            .expect_at_most(1)
            .create_async()
            .await;

        // Receipt response
        let _receipt_mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"jsonrpc":"2.0","id":2,"result":{
                "gasUsed":"0x5208",
                "status":"0x1"
            }}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient {
            client: Client::new(),
            base_url: server.url(),
            chain_id: None,
            api_key: None,
            chain_name: "aegis".to_string(),
            native_symbol: "WRAITH".to_string(),
            native_decimals: 18,
            api_type: ApiType::JsonRpc,
        };
        let tx = client.get_transaction(VALID_TX_HASH).await.unwrap();
        assert_eq!(tx.from, "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2");
        assert!(tx.status.unwrap());
        assert!(tx.timestamp.is_none()); // JSON-RPC doesn't fetch timestamp
    }

    #[tokio::test]
    async fn test_get_transaction_invalid_hash() {
        let client = EthereumClient::default();
        let result = client.get_transaction("0xbad").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_transactions() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"1","message":"OK","result":[
                {
                    "hash":"0xabc",
                    "blockNumber":"12345",
                    "timeStamp":"1700000000",
                    "from":"0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
                    "to":"0x1111111111111111111111111111111111111111",
                    "value":"1000000000000000000",
                    "gas":"21000",
                    "gasUsed":"21000",
                    "gasPrice":"20000000000",
                    "nonce":"1",
                    "input":"0x",
                    "isError":"0"
                },
                {
                    "hash":"0xdef",
                    "blockNumber":"12346",
                    "timeStamp":"1700000060",
                    "from":"0x1111111111111111111111111111111111111111",
                    "to":"0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
                    "value":"500000000000000000",
                    "gas":"21000",
                    "gasUsed":"21000",
                    "gasPrice":"20000000000",
                    "nonce":"5",
                    "input":"0x",
                    "isError":"1"
                }
            ]}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let txs = client.get_transactions(VALID_ADDRESS, 10).await.unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].hash, "0xabc");
        assert!(txs[0].status.unwrap()); // isError == "0" → success
        assert!(!txs[1].status.unwrap()); // isError == "1" → failure
        assert_eq!(txs[1].nonce, 5);
    }

    #[tokio::test]
    async fn test_get_transactions_no_transactions() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"0","message":"No transactions found","result":[]}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let txs = client.get_transactions(VALID_ADDRESS, 10).await.unwrap();
        assert!(txs.is_empty());
    }

    #[tokio::test]
    async fn test_get_transactions_empty_to_field() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"1","message":"OK","result":[{
                "hash":"0xabc",
                "blockNumber":"12345",
                "timeStamp":"1700000000",
                "from":"0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
                "to":"",
                "value":"0",
                "gas":"200000",
                "gasUsed":"150000",
                "gasPrice":"20000000000",
                "nonce":"1",
                "input":"0x60806040",
                "isError":"0"
            }]}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let txs = client.get_transactions(VALID_ADDRESS, 10).await.unwrap();
        assert_eq!(txs.len(), 1);
        assert!(txs[0].to.is_none()); // Empty "to" → contract creation
    }

    #[tokio::test]
    async fn test_get_block_number() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"result":"0x1234AB"}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let block = client.get_block_number().await.unwrap();
        assert_eq!(block, 0x1234AB);
    }

    #[tokio::test]
    async fn test_get_erc20_balances() {
        let mut server = mockito::Server::new_async().await;
        // First request: token transfers
        let _tokentx_mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"1","message":"OK","result":[
                {
                    "contractAddress":"0xdac17f958d2ee523a2206206994597c13d831ec7",
                    "tokenSymbol":"USDT",
                    "tokenName":"Tether USD",
                    "tokenDecimal":"6"
                },
                {
                    "contractAddress":"0xdac17f958d2ee523a2206206994597c13d831ec7",
                    "tokenSymbol":"USDT",
                    "tokenName":"Tether USD",
                    "tokenDecimal":"6"
                },
                {
                    "contractAddress":"0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                    "tokenSymbol":"USDC",
                    "tokenName":"USD Coin",
                    "tokenDecimal":"6"
                }
            ]}"#,
            )
            .expect_at_most(1)
            .create_async()
            .await;

        // Second+ requests: token balances (returns for first unique token)
        let _balance_mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"1","message":"OK","result":"5000000000"}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let balances = client.get_erc20_balances(VALID_ADDRESS).await.unwrap();
        // Should have 2 unique tokens (USDT deduplicated)
        assert!(balances.len() <= 2);
        if !balances.is_empty() {
            assert!(!balances[0].balance.is_empty());
        }
    }

    #[tokio::test]
    async fn test_get_erc20_balances_empty() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"0","message":"No transactions found","result":[]}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let balances = client.get_erc20_balances(VALID_ADDRESS).await.unwrap();
        assert!(balances.is_empty());
    }

    #[tokio::test]
    async fn test_get_token_info_success() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"1","message":"OK","result":[{
                "tokenName":"Tether USD",
                "symbol":"USDT",
                "divisor":"1000000",
                "tokenType":"ERC20",
                "totalSupply":"1000000000000"
            }]}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let token = client.get_token_info(VALID_ADDRESS).await.unwrap();
        assert_eq!(token.symbol, "USDT");
        assert_eq!(token.name, "Tether USD");
        assert_eq!(token.decimals, 6); // log10(1000000) = 6
    }

    #[tokio::test]
    async fn test_get_token_info_fallback_to_supply() {
        let mut server = mockito::Server::new_async().await;
        // First request: tokeninfo fails
        let _info_mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"0","message":"NOTOK","result":"Error"}"#)
            .expect_at_most(1)
            .create_async()
            .await;

        // Second request: tokensupply succeeds
        let _supply_mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"1","message":"OK","result":"1000000000000"}"#)
            .expect_at_most(1)
            .create_async()
            .await;

        // Third request: getsourcecode returns contract name
        let _source_mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"1","message":"OK","result":[{"ContractName":"TetherToken"}]}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let token = client.get_token_info(VALID_ADDRESS).await.unwrap();
        // Should get some token info via fallback
        assert!(!token.symbol.is_empty());
    }

    #[tokio::test]
    async fn test_get_token_info_unknown() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"0","message":"NOTOK","result":"Error"}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let token = client.get_token_info(VALID_ADDRESS).await.unwrap();
        // Fallback returns UNKNOWN or address-based placeholder
        assert!(!token.symbol.is_empty());
    }

    #[tokio::test]
    async fn test_try_get_contract_name() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"1","message":"OK","result":[{"ContractName":"USDT"}]}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let result = client.try_get_contract_name(VALID_ADDRESS).await;
        assert!(result.is_some());
        let (symbol, name) = result.unwrap();
        assert_eq!(name, "USDT");
        assert_eq!(symbol, "USDT"); // Short name → uppercased as-is
    }

    #[tokio::test]
    async fn test_try_get_contract_name_long_name() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"1","message":"OK","result":[{"ContractName":"TetherUSDToken"}]}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let result = client.try_get_contract_name(VALID_ADDRESS).await;
        assert!(result.is_some());
        let (symbol, name) = result.unwrap();
        assert_eq!(name, "TetherUSDToken");
        assert_eq!(symbol, "TUSDT"); // Uppercase chars extracted
    }

    #[tokio::test]
    async fn test_try_get_contract_name_not_verified() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"1","message":"OK","result":[{"ContractName":""}]}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let result = client.try_get_contract_name(VALID_ADDRESS).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_get_token_holders() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"1","message":"OK","result":[
                {"TokenHolderAddress":"0x1111111111111111111111111111111111111111","TokenHolderQuantity":"5000000000000000000000"},
                {"TokenHolderAddress":"0x2222222222222222222222222222222222222222","TokenHolderQuantity":"3000000000000000000000"},
                {"TokenHolderAddress":"0x3333333333333333333333333333333333333333","TokenHolderQuantity":"2000000000000000000000"}
            ]}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let holders = client.get_token_holders(VALID_ADDRESS, 10).await.unwrap();
        assert_eq!(holders.len(), 3);
        assert_eq!(
            holders[0].address,
            "0x1111111111111111111111111111111111111111"
        );
        assert_eq!(holders[0].rank, 1);
        assert_eq!(holders[2].rank, 3);
        // Percentage should be 50%, 30%, 20%
        assert!((holders[0].percentage - 50.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_get_token_holders_pro_required() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"0","message":"This endpoint requires a Pro API key","result":[]}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let holders = client.get_token_holders(VALID_ADDRESS, 10).await.unwrap();
        assert!(holders.is_empty()); // Graceful fallback
    }

    #[tokio::test]
    async fn test_get_token_holder_count_small() {
        let mut server = mockito::Server::new_async().await;
        // Return fewer than 1000 holders → exact count
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"1","message":"OK","result":[
                {"TokenHolderAddress":"0x1111111111111111111111111111111111111111","TokenHolderQuantity":"1000"},
                {"TokenHolderAddress":"0x2222222222222222222222222222222222222222","TokenHolderQuantity":"500"}
            ]}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let count = client.get_token_holder_count(VALID_ADDRESS).await.unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_get_token_holder_count_empty() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"0","message":"NOTOK - Missing or invalid API Pro key","result":[]}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let count = client.get_token_holder_count(VALID_ADDRESS).await.unwrap();
        // Pro required → empty holders → 0
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_get_block_timestamp() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{"timestamp":"0x65A8C580"}}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let ts = client.get_block_timestamp(16).await.unwrap();
        assert_eq!(ts, 0x65A8C580);
    }

    #[tokio::test]
    async fn test_get_block_timestamp_not_found() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":null}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let result = client.get_block_timestamp(99999999).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_chain_name_and_symbol_accessors() {
        let client = EthereumClient::with_base_url("http://test");
        assert_eq!(client.chain_name(), "ethereum");
        assert_eq!(client.native_token_symbol(), "ETH");
    }

    #[test]
    fn test_validate_tx_hash_invalid_hex() {
        let hash = "0xZZZZ23def456789012345678901234567890123456789012345678901234abcd";
        let result = validate_tx_hash(hash);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid hex"));
    }

    #[tokio::test]
    async fn test_get_transactions_api_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"0","message":"NOTOK","result":"Max rate limit reached"}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let result = client.get_transactions(VALID_ADDRESS, 10).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_token_holders_pro_key_required() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"0","message":"Pro API key required","result":[]}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let holders = client
            .get_token_holders("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", 10)
            .await
            .unwrap();
        assert!(holders.is_empty());
    }

    #[tokio::test]
    async fn test_get_token_holders_api_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"0","message":"Some other error","result":[]}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let result = client
            .get_token_holders("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", 10)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_token_holders_success() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"1","message":"OK","result":[
                {"TokenHolderAddress":"0xHolder1","TokenHolderQuantity":"1000000000000000000"},
                {"TokenHolderAddress":"0xHolder2","TokenHolderQuantity":"500000000000000000"}
            ]}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let holders = client
            .get_token_holders("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", 10)
            .await
            .unwrap();
        assert_eq!(holders.len(), 2);
        assert_eq!(holders[0].rank, 1);
        assert_eq!(holders[1].rank, 2);
        assert!(holders[0].percentage > 0.0);
    }

    #[tokio::test]
    async fn test_get_token_info_unknown_token() {
        let mut server = mockito::Server::new_async().await;
        // First call for tokeninfo - return empty/error
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"0","message":"No data found","result":[]}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let token = client
            .get_token_info("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")
            .await
            .unwrap();
        assert_eq!(token.symbol, "UNKNOWN");
    }

    #[tokio::test]
    async fn test_get_transaction_with_null_block_number() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"jsonrpc":"2.0","id":1,"result":{
                "hash":"0xabc123def456789012345678901234567890123456789012345678901234abcd",
                "from":"0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
                "to":"0xB0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
                "value":"0xde0b6b3a7640000",
                "gas":"0x5208",
                "gasPrice":"0x3b9aca00",
                "nonce":"0x5",
                "input":"0x",
                "blockNumber":null
            }}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let tx = client.get_transaction(VALID_TX_HASH).await.unwrap();
        // blockNumber is null, so timestamp should also be None
        assert!(tx.timestamp.is_none());
    }

    #[tokio::test]
    async fn test_chain_client_trait_balance() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"1","message":"OK","result":"1000000000000000000"}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let chain_client: &dyn ChainClient = &client;
        let balance = chain_client.get_balance(VALID_ADDRESS).await.unwrap();
        assert_eq!(balance.symbol, "ETH");
    }

    #[tokio::test]
    async fn test_chain_client_trait_get_transaction() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"jsonrpc":"2.0","id":1,"result":{
                "hash":"0xabc123def456789012345678901234567890123456789012345678901234abcd",
                "from":"0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
                "to":"0xB0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
                "value":"0xde0b6b3a7640000",
                "gas":"0x5208",
                "gasPrice":"0x3b9aca00",
                "nonce":"0x5",
                "input":"0x",
                "blockNumber":"0xf4240"
            }}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let chain_client: &dyn ChainClient = &client;
        let tx = chain_client.get_transaction(VALID_TX_HASH).await.unwrap();
        assert!(!tx.hash.is_empty());
    }

    #[tokio::test]
    async fn test_chain_client_trait_get_block_number() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":"0xf4240"}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let chain_client: &dyn ChainClient = &client;
        let block = chain_client.get_block_number().await.unwrap();
        assert_eq!(block, 1000000);
    }

    #[tokio::test]
    async fn test_chain_client_trait_get_transactions() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"1","message":"OK","result":[{
                "hash":"0xabc",
                "blockNumber":"12345",
                "timeStamp":"1700000000",
                "from":"0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
                "to":"0x1111111111111111111111111111111111111111",
                "value":"1000000000000000000",
                "gas":"21000","gasUsed":"21000","gasPrice":"20000000000",
                "nonce":"1","input":"0x","isError":"0"
            }]}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let chain_client: &dyn ChainClient = &client;
        let txs = chain_client
            .get_transactions(VALID_ADDRESS, 10)
            .await
            .unwrap();
        assert_eq!(txs.len(), 1);
    }

    #[tokio::test]
    async fn test_chain_client_trait_get_token_balances() {
        let mut server = mockito::Server::new_async().await;
        // First call: tokentx returns tokens
        let _tokentx = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"1","message":"OK","result":[{
                "contractAddress":"0xdac17f958d2ee523a2206206994597c13d831ec7",
                "tokenSymbol":"USDT","tokenName":"Tether USD","tokenDecimal":"6"
            }]}"#,
            )
            .expect_at_most(1)
            .create_async()
            .await;

        // Second call: token balance
        let _balance = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"1","message":"OK","result":"5000000000"}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let chain_client: &dyn ChainClient = &client;
        let balances = chain_client
            .get_token_balances(VALID_ADDRESS)
            .await
            .unwrap();
        assert!(!balances.is_empty());
    }

    #[tokio::test]
    async fn test_chain_client_trait_get_token_info() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"1","message":"OK","result":[{
                "tokenName":"Tether USD","symbol":"USDT",
                "divisor":"1000000","tokenType":"ERC20","totalSupply":"1000000000000"
            }]}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let chain_client: &dyn ChainClient = &client;
        let token = chain_client.get_token_info(VALID_ADDRESS).await.unwrap();
        assert_eq!(token.symbol, "USDT");
    }

    #[tokio::test]
    async fn test_chain_client_trait_get_token_holders() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"1","message":"OK","result":[
                {"TokenHolderAddress":"0x1111111111111111111111111111111111111111","TokenHolderQuantity":"5000"}
            ]}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let chain_client: &dyn ChainClient = &client;
        let holders = chain_client
            .get_token_holders(VALID_ADDRESS, 10)
            .await
            .unwrap();
        assert_eq!(holders.len(), 1);
    }

    #[tokio::test]
    async fn test_chain_client_trait_get_token_holder_count() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"1","message":"OK","result":[
                {"TokenHolderAddress":"0x1111111111111111111111111111111111111111","TokenHolderQuantity":"5000"},
                {"TokenHolderAddress":"0x2222222222222222222222222222222222222222","TokenHolderQuantity":"3000"}
            ]}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let chain_client: &dyn ChainClient = &client;
        let count = chain_client
            .get_token_holder_count(VALID_ADDRESS)
            .await
            .unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_chain_client_trait_native_token_symbol() {
        let client = EthereumClient::with_base_url("http://test");
        let chain_client: &dyn ChainClient = &client;
        assert_eq!(chain_client.native_token_symbol(), "ETH");
        assert_eq!(chain_client.chain_name(), "ethereum");
    }

    #[tokio::test]
    async fn test_get_token_info_supply_fallback_no_contract() {
        let mut server = mockito::Server::new_async().await;
        // First request: tokeninfo fails
        let _info_mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"0","message":"NOTOK","result":"Error"}"#)
            .expect_at_most(1)
            .create_async()
            .await;

        // Second request: tokensupply succeeds (valid ERC20)
        let _supply_mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"1","message":"OK","result":"1000000000000"}"#)
            .expect_at_most(1)
            .create_async()
            .await;

        // Third request: getsourcecode returns empty contract name
        let _source_mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"1","message":"OK","result":[{"ContractName":""}]}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let token = client.get_token_info(VALID_ADDRESS).await.unwrap();
        // Should return address-based placeholder since contract name is empty
        assert!(token.symbol.contains("...") || !token.symbol.is_empty());
    }

    #[tokio::test]
    async fn test_try_get_contract_name_short_lowercase() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"1","message":"OK","result":[{"ContractName":"token"}]}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let result = client.try_get_contract_name(VALID_ADDRESS).await;
        assert!(result.is_some());
        let (symbol, name) = result.unwrap();
        assert_eq!(name, "token");
        // Name <= 6 chars → uppercased directly
        assert_eq!(symbol, "TOKEN");
    }

    #[tokio::test]
    async fn test_try_get_contract_name_long_lowercase() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"1","message":"OK","result":[{"ContractName":"mytokencontract"}]}"#,
            )
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let result = client.try_get_contract_name(VALID_ADDRESS).await;
        assert!(result.is_some());
        let (symbol, name) = result.unwrap();
        assert_eq!(name, "mytokencontract");
        // Long name with no uppercase → extract uppercase chars gets empty string
        // Falls back to first 6 chars uppercased
        assert_eq!(symbol, "MYTOKE");
    }

    #[tokio::test]
    async fn test_get_erc20_balances_zero_balance_skipped() {
        let mut server = mockito::Server::new_async().await;
        // First request: token transfers
        let _tokentx = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"1","message":"OK","result":[{
                "contractAddress":"0xdac17f958d2ee523a2206206994597c13d831ec7",
                "tokenSymbol":"USDT","tokenName":"Tether USD","tokenDecimal":"6"
            }]}"#,
            )
            .expect_at_most(1)
            .create_async()
            .await;

        // Second request: token balance returns zero
        let _balance = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"1","message":"OK","result":"0"}"#)
            .create_async()
            .await;

        let client = EthereumClient::with_base_url(&server.url());
        let balances = client.get_erc20_balances(VALID_ADDRESS).await.unwrap();
        // Zero balance should be skipped
        assert!(balances.is_empty());
    }
}
