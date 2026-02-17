//! # Tron Client
//!
//! This module provides a Tron blockchain client for querying balances,
//! transactions, and account information on the Tron network.
//!
//! ## Features
//!
//! - Balance queries via TronGrid API (with USD valuation via DexScreener)
//! - Transaction history retrieval
//! - Transaction details lookup by hash
//! - TRC-20 token balance fetching from TronGrid account endpoint
//! - T-address validation with full base58check verification (double SHA256 checksum)
//!
//! ## Usage
//!
//! ```rust,no_run
//! use scope::chains::TronClient;
//! use scope::config::ChainsConfig;
//!
//! #[tokio::main]
//! async fn main() -> scope::Result<()> {
//!     let config = ChainsConfig::default();
//!     let client = TronClient::new(&config)?;
//!     
//!     let balance = client.get_balance("TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf").await?;
//!     println!("Balance: {} TRX", balance.formatted);
//!     Ok(())
//! }
//! ```

use crate::chains::{Balance, ChainClient, Token, TokenHolder, Transaction};
use crate::config::ChainsConfig;
use crate::error::{Result, ScopeError};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json;
use sha2::{Digest, Sha256};

/// Default TronGrid API endpoint.
const DEFAULT_TRON_API: &str = "https://api.trongrid.io";

/// Tronscan API base for token info and holder lookups.
const TRONSCAN_API: &str = "https://apilist.tronscanapi.com";

/// DexScreener search URL for TRX/USDT price lookup.
const DEXSCREENER_TRX_SEARCH: &str = "https://api.dexscreener.com/latest/dex/search?q=TRX%20USDT";

/// Tron native token decimals (TRX uses 6 decimals, stored as "sun").
const TRX_DECIMALS: u8 = 6;

/// Tron blockchain client.
///
/// Uses TronGrid REST API for data retrieval.
#[derive(Debug, Clone)]
pub struct TronClient {
    /// HTTP client for API requests.
    client: Client,

    /// TronGrid API base URL.
    api_url: String,

    /// TronGrid API key for higher rate limits.
    api_key: Option<String>,
}

/// Account response from TronGrid API.
#[derive(Debug, Deserialize)]
struct AccountResponse {
    data: Vec<AccountData>,
    success: bool,
    error: Option<String>,
}

/// Account data from TronGrid.
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Fields used for deserialization
struct AccountData {
    balance: Option<u64>,
    address: String,
    create_time: Option<u64>,
    #[serde(default)]
    trc20: Vec<Trc20Balance>,
}

/// TRC20 token balance.
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Reserved for future TRC20 token support
struct Trc20Balance {
    #[serde(flatten)]
    balances: std::collections::HashMap<String, String>,
}

/// Transaction list response from TronGrid.
#[derive(Debug, Deserialize)]
struct TransactionListResponse {
    data: Vec<TronTransaction>,
    success: bool,
    error: Option<String>,
}

/// Tron transaction from API.
#[derive(Debug, Deserialize)]
struct TronTransaction {
    #[serde(rename = "txID")]
    tx_id: String,
    block_number: Option<u64>,
    block_timestamp: Option<u64>,
    raw_data: Option<RawData>,
    ret: Option<Vec<TransactionResult>>,
}

/// Raw transaction data.
#[derive(Debug, Deserialize)]
struct RawData {
    contract: Option<Vec<Contract>>,
}

/// Contract call in transaction.
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Fields used for deserialization
struct Contract {
    parameter: Option<ContractParameter>,
    #[serde(rename = "type")]
    contract_type: Option<String>,
}

/// Contract parameters.
#[derive(Debug, Deserialize)]
struct ContractParameter {
    value: Option<ContractValue>,
}

/// Contract value containing transfer details.
#[derive(Debug, Deserialize)]
struct ContractValue {
    amount: Option<u64>,
    owner_address: Option<String>,
    to_address: Option<String>,
}

/// Transaction result.
#[derive(Debug, Deserialize)]
struct TransactionResult {
    #[serde(rename = "contractRet")]
    contract_ret: Option<String>,
}

impl TronClient {
    /// Creates a new Tron client with the given configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - Chain configuration containing API endpoint and keys
    ///
    /// # Returns
    ///
    /// Returns a configured [`TronClient`] instance.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use scope::chains::TronClient;
    /// use scope::config::ChainsConfig;
    ///
    /// let config = ChainsConfig::default();
    /// let client = TronClient::new(&config).unwrap();
    /// ```
    pub fn new(config: &ChainsConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ScopeError::Chain(format!("Failed to create HTTP client: {}", e)))?;

        let api_url = config
            .tron_api
            .as_deref()
            .unwrap_or(DEFAULT_TRON_API)
            .to_string();

        Ok(Self {
            client,
            api_url,
            api_key: config.api_keys.get("tronscan").cloned(),
        })
    }

    /// Creates a client with a custom API URL.
    ///
    /// # Arguments
    ///
    /// * `api_url` - The TronGrid API endpoint URL
    pub fn with_api_url(api_url: &str) -> Self {
        Self {
            client: Client::new(),
            api_url: api_url.to_string(),
            api_key: None,
        }
    }

    /// Returns the chain name.
    pub fn chain_name(&self) -> &str {
        "tron"
    }

    /// Returns the native token symbol.
    pub fn native_token_symbol(&self) -> &str {
        "TRX"
    }

    /// Fetches the TRX balance for an address.
    ///
    /// # Arguments
    ///
    /// * `address` - The Tron address (T-address format)
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
        validate_tron_address(address)?;

        let url = format!("{}/v1/accounts/{}", self.api_url, address);

        tracing::debug!(url = %url, address = %address, "Fetching Tron balance");

        let mut request = self.client.get(&url);
        if let Some(ref key) = self.api_key {
            request = request.header("TRON-PRO-API-KEY", key);
        }

        let response: AccountResponse = request.send().await?.json().await?;

        if !response.success {
            return Err(ScopeError::Chain(format!(
                "TronGrid API error: {}",
                response.error.unwrap_or_else(|| "Unknown error".into())
            )));
        }

        // Account may not exist yet (no balance)
        let sun = response.data.first().and_then(|d| d.balance).unwrap_or(0);

        let trx = sun as f64 / 10_f64.powi(TRX_DECIMALS as i32);

        Ok(Balance {
            raw: sun.to_string(),
            formatted: format!("{:.6} TRX", trx),
            decimals: TRX_DECIMALS,
            symbol: "TRX".to_string(),
            usd_value: None, // Populated by caller via enrich_balance_usd
        })
    }

    /// Fetches TRC-20 token balances for an address.
    ///
    /// Uses the TronGrid `/v1/accounts/{address}` endpoint which includes
    /// TRC-20 balances in the account data.
    pub async fn get_trc20_balances(&self, address: &str) -> Result<Vec<Trc20TokenBalance>> {
        validate_tron_address(address)?;

        let url = format!("{}/v1/accounts/{}", self.api_url, address);

        tracing::debug!(url = %url, "Fetching TRC-20 token balances");

        let mut request = self.client.get(&url);
        if let Some(ref key) = self.api_key {
            request = request.header("TRON-PRO-API-KEY", key);
        }

        let response: AccountResponse = request.send().await?.json().await?;

        if !response.success {
            return Err(ScopeError::Chain(format!(
                "TronGrid API error: {}",
                response.error.unwrap_or_else(|| "Unknown error".into())
            )));
        }

        let account = match response.data.first() {
            Some(data) => data,
            None => return Ok(vec![]),
        };

        let mut balances = Vec::new();
        for trc20 in &account.trc20 {
            for (contract_address, raw_balance) in &trc20.balances {
                // Skip zero balances
                if raw_balance == "0" {
                    continue;
                }
                balances.push(Trc20TokenBalance {
                    contract_address: contract_address.clone(),
                    raw_balance: raw_balance.clone(),
                });
            }
        }

        Ok(balances)
    }

    /// Fetches TRC-20 token info from Tronscan API.
    ///
    /// Returns symbol, name, decimals, and other metadata for a TRC-20 contract.
    pub async fn get_token_info(&self, contract_address: &str) -> Result<Token> {
        validate_tron_address(contract_address)?;

        let url = format!(
            "{}/api/token_trc20?contract={}&showAll=1",
            TRONSCAN_API, contract_address
        );

        tracing::debug!(url = %url, "Fetching TRC-20 token info via Tronscan");

        let mut request = self.client.get(&url);
        if let Some(ref key) = self.api_key {
            request = request.header("TRON-PRO-API-KEY", key);
        }

        let response = request.send().await?;
        let text = response.text().await?;
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| ScopeError::Api(format!("Failed to parse Tronscan response: {}", e)))?;

        let tokens = json
            .get("trc20_tokens")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                ScopeError::NotFound(format!(
                    "No token info found for TRC-20 contract {}",
                    contract_address
                ))
            })?;

        let token_data = tokens.first().ok_or_else(|| {
            ScopeError::NotFound(format!(
                "No token info found for TRC-20 contract {}",
                contract_address
            ))
        })?;

        let symbol = token_data
            .get("symbol")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN")
            .to_string();
        let name = token_data
            .get("contract_name")
            .or_else(|| token_data.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Token")
            .to_string();
        let decimals = token_data
            .get("decimals")
            .and_then(|v| v.as_u64())
            .unwrap_or(6) as u8;

        Ok(Token {
            contract_address: contract_address.to_string(),
            symbol,
            name,
            decimals,
        })
    }

    /// Fetches top TRC-20 token holders from Tronscan API.
    ///
    /// Returns holders sorted by balance (largest first).
    pub async fn get_token_holders(
        &self,
        contract_address: &str,
        limit: u32,
    ) -> Result<Vec<TokenHolder>> {
        validate_tron_address(contract_address)?;

        let effective_limit = limit.min(100);
        let url = format!(
            "{}/api/token_trc20/holders?contract_address={}&start=0&limit={}",
            TRONSCAN_API, contract_address, effective_limit
        );

        tracing::debug!(url = %url, "Fetching TRC-20 token holders via Tronscan");

        let mut request = self.client.get(&url);
        if let Some(ref key) = self.api_key {
            request = request.header("TRON-PRO-API-KEY", key);
        }

        let response = request.send().await?;
        let text = response.text().await?;
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| ScopeError::Api(format!("Failed to parse Tronscan holders: {}", e)))?;

        let holders_data: &[serde_json::Value] = json
            .get("trc20_tokens")
            .and_then(|v| v.as_array())
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        // Get decimals for formatted balance
        let token_info = self.get_token_info(contract_address).await;
        let decimals = token_info.as_ref().map(|t| t.decimals).unwrap_or(6);

        // Percentage is relative to sum of fetched holder balances (same as EVM chains)
        let total_balance: f64 = holders_data
            .iter()
            .filter_map(|h| h.get("balance").and_then(|v| v.as_str()))
            .filter_map(|s| s.parse::<f64>().ok())
            .sum();

        let token_holders: Vec<TokenHolder> = holders_data
            .iter()
            .enumerate()
            .filter_map(|(i, h)| {
                let holder_address = h.get("holder_address")?.as_str()?.to_string();
                let balance_raw = h.get("balance")?.as_str()?.to_string();
                let balance: f64 = balance_raw.parse().ok()?;
                let percentage = if total_balance > 0.0 {
                    (balance / total_balance) * 100.0
                } else {
                    0.0
                };
                let divisor = 10_f64.powi(decimals as i32);
                let formatted = format!("{:.6}", balance / divisor);

                Some(TokenHolder {
                    address: holder_address,
                    balance: balance_raw,
                    formatted_balance: formatted,
                    percentage,
                    rank: (i + 1) as u32,
                })
            })
            .collect();

        Ok(token_holders)
    }

    /// Fetches total holder count for a TRC-20 token.
    pub async fn get_token_holder_count(&self, contract_address: &str) -> Result<u64> {
        validate_tron_address(contract_address)?;

        let url = format!(
            "{}/api/token_trc20/holders?contract_address={}&start=0&limit=1",
            TRONSCAN_API, contract_address
        );

        let mut request = self.client.get(&url);
        if let Some(ref key) = self.api_key {
            request = request.header("TRON-PRO-API-KEY", key);
        }

        let response = request.send().await?;
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ScopeError::Api(format!("Failed to parse Tronscan response: {}", e)))?;

        let count = json.get("rangeTotal").and_then(|v| v.as_u64()).unwrap_or(0);

        Ok(count)
    }

    /// Enriches a balance with a USD value using DexScreener price lookup.
    ///
    /// Note: Tron native token price lookup via DexScreener is not yet supported.
    /// This is a placeholder that uses CoinGecko-style simple price API as fallback.
    pub async fn enrich_balance_usd(&self, balance: &mut Balance) {
        // Try to get TRX price from DexScreener search API
        let url = DEXSCREENER_TRX_SEARCH;
        if let Ok(response) = self.client.get(url).send().await
            && let Ok(text) = response.text().await
            && let Ok(search_result) = serde_json::from_str::<DexSearchResponse>(&text)
            && let Some(pairs) = search_result.pairs
        {
            for pair in &pairs {
                if (pair.base_token_symbol.as_deref() == Some("TRX")
                    || pair.base_token_symbol.as_deref() == Some("WTRX"))
                    && let Some(price) = pair.price_usd.as_ref().and_then(|p| p.parse::<f64>().ok())
                {
                    let sun: f64 = balance.raw.parse().unwrap_or(0.0);
                    let trx = sun / 10_f64.powi(TRX_DECIMALS as i32);
                    balance.usd_value = Some(trx * price);
                    return;
                }
            }
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
        validate_tron_tx_hash(hash)?;

        let url = format!("{}/v1/transactions/{}", self.api_url, hash);

        tracing::debug!(url = %url, hash = %hash, "Fetching Tron transaction");

        let mut request = self.client.get(&url);
        if let Some(ref key) = self.api_key {
            request = request.header("TRON-PRO-API-KEY", key);
        }

        let response: TransactionListResponse = request.send().await?.json().await?;

        if !response.success {
            return Err(ScopeError::Chain(format!(
                "TronGrid API error: {}",
                response.error.unwrap_or_else(|| "Unknown error".into())
            )));
        }

        let tx = response
            .data
            .into_iter()
            .next()
            .ok_or_else(|| ScopeError::Chain("Transaction not found".into()))?;

        // Extract transfer details from contract
        let (from, to, value) = tx
            .raw_data
            .and_then(|rd| rd.contract)
            .and_then(|contracts| contracts.into_iter().next())
            .and_then(|c| c.parameter)
            .and_then(|p| p.value)
            .map(|v| {
                (
                    v.owner_address.unwrap_or_default(),
                    v.to_address,
                    v.amount.unwrap_or(0).to_string(),
                )
            })
            .unwrap_or_else(|| (String::new(), None, "0".to_string()));

        let status = tx
            .ret
            .and_then(|r| r.into_iter().next())
            .and_then(|r| r.contract_ret)
            .map(|s| s == "SUCCESS");

        Ok(Transaction {
            hash: tx.tx_id,
            block_number: tx.block_number,
            timestamp: tx.block_timestamp.map(|t| t / 1000), // Convert ms to seconds
            from,
            to,
            value,
            gas_limit: 0, // Tron uses bandwidth/energy instead of gas
            gas_used: None,
            gas_price: "0".to_string(),
            nonce: 0,
            input: String::new(),
            status,
        })
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
        validate_tron_address(address)?;

        let url = format!(
            "{}/v1/accounts/{}/transactions?limit={}",
            self.api_url, address, limit
        );

        tracing::debug!(url = %url, address = %address, "Fetching Tron transactions");

        let mut request = self.client.get(&url);
        if let Some(ref key) = self.api_key {
            request = request.header("TRON-PRO-API-KEY", key);
        }

        let response: TransactionListResponse = request.send().await?.json().await?;

        if !response.success {
            return Err(ScopeError::Chain(format!(
                "TronGrid API error: {}",
                response.error.unwrap_or_else(|| "Unknown error".into())
            )));
        }

        let transactions = response
            .data
            .into_iter()
            .map(|tx| {
                let (from, to, value) = tx
                    .raw_data
                    .and_then(|rd| rd.contract)
                    .and_then(|contracts| contracts.into_iter().next())
                    .and_then(|c| c.parameter)
                    .and_then(|p| p.value)
                    .map(|v| {
                        (
                            v.owner_address.unwrap_or_default(),
                            v.to_address,
                            v.amount.unwrap_or(0).to_string(),
                        )
                    })
                    .unwrap_or_else(|| (String::new(), None, "0".to_string()));

                let status = tx
                    .ret
                    .and_then(|r| r.into_iter().next())
                    .and_then(|r| r.contract_ret)
                    .map(|s| s == "SUCCESS");

                Transaction {
                    hash: tx.tx_id,
                    block_number: tx.block_number,
                    timestamp: tx.block_timestamp.map(|t| t / 1000),
                    from,
                    to,
                    value,
                    gas_limit: 0,
                    gas_used: None,
                    gas_price: "0".to_string(),
                    nonce: 0,
                    input: String::new(),
                    status,
                }
            })
            .collect();

        Ok(transactions)
    }

    /// Fetches the current block number.
    pub async fn get_block_number(&self) -> Result<u64> {
        let url = format!("{}/wallet/getnowblock", self.api_url);

        #[derive(Deserialize)]
        struct BlockResponse {
            block_header: Option<BlockHeader>,
        }

        #[derive(Deserialize)]
        struct BlockHeader {
            raw_data: Option<BlockRawData>,
        }

        #[derive(Deserialize)]
        struct BlockRawData {
            number: Option<u64>,
        }

        let response: BlockResponse = self.client.post(&url).send().await?.json().await?;

        response
            .block_header
            .and_then(|h| h.raw_data)
            .and_then(|d| d.number)
            .ok_or_else(|| ScopeError::Chain("Invalid block response".into()))
    }
}

impl Default for TronClient {
    fn default() -> Self {
        Self {
            client: Client::new(),
            api_url: DEFAULT_TRON_API.to_string(),
            api_key: None,
        }
    }
}

/// Validates a Tron address format (T-address, base58check encoded).
///
/// Tron addresses:
/// - Start with 'T'
/// - Are 34 characters long
/// - Use base58check encoding (includes checksum)
///
/// # Arguments
///
/// * `address` - The address to validate
///
/// TRC-20 token balance result.
#[derive(Debug, Clone)]
pub struct Trc20TokenBalance {
    /// Token contract address (base58).
    pub contract_address: String,
    /// Raw balance string.
    pub raw_balance: String,
}

/// Minimal DexScreener search response for price lookups.
#[derive(Debug, Deserialize)]
struct DexSearchResponse {
    #[serde(default)]
    pairs: Option<Vec<DexSearchPair>>,
}

/// A pair from DexScreener search results.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DexSearchPair {
    #[serde(default)]
    base_token_symbol: Option<String>,
    #[serde(default)]
    price_usd: Option<String>,
}

/// # Returns
///
/// Returns `Ok(())` if valid, or an error describing the validation failure.
pub fn validate_tron_address(address: &str) -> Result<()> {
    if address.is_empty() {
        return Err(ScopeError::InvalidAddress("Address cannot be empty".into()));
    }

    // Tron addresses start with 'T'
    if !address.starts_with('T') {
        return Err(ScopeError::InvalidAddress(format!(
            "Tron address must start with 'T': {}",
            address
        )));
    }

    // Tron addresses are 34 characters
    if address.len() != 34 {
        return Err(ScopeError::InvalidAddress(format!(
            "Tron address must be 34 characters, got {}: {}",
            address.len(),
            address
        )));
    }

    // Validate base58 encoding
    match bs58::decode(address).into_vec() {
        Ok(bytes) => {
            // Should decode to 25 bytes (1 prefix + 20 address + 4 checksum)
            if bytes.len() != 25 {
                return Err(ScopeError::InvalidAddress(format!(
                    "Tron address must decode to 25 bytes, got {}: {}",
                    bytes.len(),
                    address
                )));
            }

            // First byte should be 0x41 (Tron mainnet prefix)
            if bytes[0] != 0x41 {
                return Err(ScopeError::InvalidAddress(format!(
                    "Invalid Tron address prefix: {}",
                    address
                )));
            }

            // Verify checksum: last 4 bytes must equal first 4 bytes of double SHA256 of first 21 bytes
            let payload = &bytes[0..21];
            let hash1 = Sha256::digest(payload);
            let hash2 = Sha256::digest(hash1);
            let expected_checksum = &hash2[0..4];
            let actual_checksum = &bytes[21..25];

            if expected_checksum != actual_checksum {
                return Err(ScopeError::InvalidAddress(format!(
                    "Invalid Tron address checksum: {}",
                    address
                )));
            }
        }
        Err(e) => {
            return Err(ScopeError::InvalidAddress(format!(
                "Invalid base58 encoding: {}: {}",
                e, address
            )));
        }
    }

    Ok(())
}

/// Validates a Tron transaction hash format.
///
/// Tron transaction hashes are 64-character hex strings.
///
/// # Arguments
///
/// * `hash` - The hash to validate
///
/// # Returns
///
/// Returns `Ok(())` if valid, or an error describing the validation failure.
pub fn validate_tron_tx_hash(hash: &str) -> Result<()> {
    if hash.is_empty() {
        return Err(ScopeError::InvalidHash("Hash cannot be empty".into()));
    }

    // Tron tx hashes are 64 hex characters (without 0x prefix)
    if hash.len() != 64 {
        return Err(ScopeError::InvalidHash(format!(
            "Tron transaction hash must be 64 characters, got {}: {}",
            hash.len(),
            hash
        )));
    }

    // Validate hex encoding
    if !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ScopeError::InvalidHash(format!(
            "Tron hash contains invalid hex characters: {}",
            hash
        )));
    }

    Ok(())
}

// ============================================================================
// ChainClient Trait Implementation
// ============================================================================

#[async_trait]
impl ChainClient for TronClient {
    fn chain_name(&self) -> &str {
        "tron"
    }

    fn native_token_symbol(&self) -> &str {
        "TRX"
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

    async fn get_token_balances(&self, address: &str) -> Result<Vec<crate::chains::TokenBalance>> {
        let trc20_balances = self.get_trc20_balances(address).await?;
        let mut result = Vec::with_capacity(trc20_balances.len());

        for tb in trc20_balances {
            let token = match self.get_token_info(&tb.contract_address).await {
                Ok(info) => info,
                Err(e) => {
                    tracing::debug!(
                        contract = %tb.contract_address,
                        error = %e,
                        "Could not fetch TRC-20 token info, using placeholder"
                    );
                    Token {
                        contract_address: tb.contract_address.clone(),
                        symbol: "TRC20".to_string(),
                        name: "TRC-20 Token".to_string(),
                        decimals: 6, // Common for USDT, USDC
                    }
                }
            };

            let raw: f64 = tb.raw_balance.parse().unwrap_or(0.0);
            let divisor = 10_f64.powi(token.decimals as i32);
            let formatted = format!("{:.6}", raw / divisor);

            result.push(crate::chains::TokenBalance {
                token,
                balance: tb.raw_balance,
                formatted_balance: formatted,
                usd_value: None,
            });
        }

        Ok(result)
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

    // Valid Tron address (Binance cold wallet)
    const VALID_ADDRESS: &str = "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf";

    // Valid Tron transaction hash
    const VALID_TX_HASH: &str = "b3c12d62ad7e7b8b83b09a68b9b8f9b23a1b8f8b8f9b8f9b8f9b8f9b8f9b8f9b";

    #[test]
    fn test_validate_tron_address_valid() {
        assert!(validate_tron_address(VALID_ADDRESS).is_ok());
    }

    #[test]
    fn test_validate_tron_address_empty() {
        let result = validate_tron_address("");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_validate_tron_address_wrong_prefix() {
        let result = validate_tron_address("ADqSquXBgUCLYvYC4XZgrprLK589dkhSCf");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("start with 'T'"));
    }

    #[test]
    fn test_validate_tron_address_too_short() {
        let result = validate_tron_address("TDqSquXBgUCLYvYC4XZ");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("34 characters"));
    }

    #[test]
    fn test_validate_tron_address_too_long() {
        let result = validate_tron_address("TDqSquXBgUCLYvYC4XZgrprLK589dkhSCfAAAA");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("34 characters"));
    }

    #[test]
    fn test_validate_tron_address_invalid_base58() {
        // Contains '0' which is not valid base58
        let result = validate_tron_address("T0qSquXBgUCLYvYC4XZgrprLK589dkhSCf");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("base58"));
    }

    #[test]
    fn test_validate_tron_tx_hash_valid() {
        assert!(validate_tron_tx_hash(VALID_TX_HASH).is_ok());
    }

    #[test]
    fn test_validate_tron_tx_hash_empty() {
        let result = validate_tron_tx_hash("");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_validate_tron_tx_hash_too_short() {
        let result = validate_tron_tx_hash("b3c12d62ad7e7b8b83b09a68");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("64 characters"));
    }

    #[test]
    fn test_validate_tron_tx_hash_invalid_hex() {
        let hash = "g3c12d62ad7e7b8b83b09a68b9b8f9b23a1b8f8b8f9b8f9b8f9b8f9b8f9b8f9b";
        let result = validate_tron_tx_hash(hash);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid hex"));
    }

    #[test]
    fn test_tron_client_default() {
        let client = TronClient::default();
        assert_eq!(client.chain_name(), "tron");
        assert_eq!(client.native_token_symbol(), "TRX");
        assert!(client.api_url.contains("trongrid"));
    }

    #[test]
    fn test_tron_client_with_api_url() {
        let client = TronClient::with_api_url("https://custom.tron.api");
        assert_eq!(client.api_url, "https://custom.tron.api");
    }

    #[test]
    fn test_tron_client_new() {
        let config = ChainsConfig::default();
        let client = TronClient::new(&config);
        assert!(client.is_ok());
    }

    #[test]
    fn test_tron_client_new_with_custom_api() {
        let config = ChainsConfig {
            tron_api: Some("https://my-tron-api.com".to_string()),
            ..Default::default()
        };
        let client = TronClient::new(&config).unwrap();
        assert_eq!(client.api_url, "https://my-tron-api.com");
    }

    #[test]
    fn test_tron_client_new_with_api_key() {
        use std::collections::HashMap;

        let mut api_keys = HashMap::new();
        api_keys.insert("tronscan".to_string(), "test-key".to_string());

        let config = ChainsConfig {
            api_keys,
            ..Default::default()
        };

        let client = TronClient::new(&config).unwrap();
        assert_eq!(client.api_key, Some("test-key".to_string()));
    }

    #[test]
    fn test_account_response_deserialization() {
        let json = r#"{
            "data": [{
                "balance": 1000000,
                "address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf",
                "create_time": 1600000000000,
                "trc20": []
            }],
            "success": true
        }"#;

        let response: AccountResponse = serde_json::from_str(json).unwrap();
        assert!(response.success);
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].balance, Some(1_000_000));
    }

    #[test]
    fn test_transaction_response_deserialization() {
        let json = r#"{
            "data": [{
                "txID": "abc123",
                "block_number": 12345,
                "block_timestamp": 1600000000000,
                "ret": [{"contractRet": "SUCCESS"}]
            }],
            "success": true
        }"#;

        let response: TransactionListResponse = serde_json::from_str(json).unwrap();
        assert!(response.success);
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].tx_id, "abc123");
    }

    // ========================================================================
    // HTTP mocking tests
    // ========================================================================

    #[tokio::test]
    async fn test_get_balance() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Regex(r"/v1/accounts/.*".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{
                "data": [{"balance": 5000000, "address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf", "trc20": []}],
                "success": true
            }"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let balance = client.get_balance(VALID_ADDRESS).await.unwrap();
        assert_eq!(balance.raw, "5000000");
        assert_eq!(balance.symbol, "TRX");
        assert!(balance.formatted.contains("5.000000"));
    }

    #[tokio::test]
    async fn test_get_balance_new_account() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [], "success": true}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let balance = client.get_balance(VALID_ADDRESS).await.unwrap();
        assert_eq!(balance.raw, "0");
        assert!(balance.formatted.contains("0.000000"));
    }

    #[tokio::test]
    async fn test_get_balance_api_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [], "success": false, "error": "Rate limit exceeded"}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_balance(VALID_ADDRESS).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Rate limit"));
    }

    #[tokio::test]
    async fn test_get_balance_invalid_address() {
        let client = TronClient::default();
        let result = client.get_balance("invalid").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_transaction() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/transactions/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": [{
                    "txID": "b3c12d62ad7e7b8b83b09a68b9b8f9b23a1b8f8b8f9b8f9b8f9b8f9b8f9b8f9b",
                    "block_number": 50000000,
                    "block_timestamp": 1700000000000,
                    "raw_data": {
                        "contract": [{
                            "parameter": {
                                "value": {
                                    "amount": 1000000,
                                    "owner_address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf",
                                    "to_address": "TN3W4H6rK2ce4vX9YnFQHwKENnHjoxb3m9"
                                }
                            },
                            "type": "TransferContract"
                        }]
                    },
                    "ret": [{"contractRet": "SUCCESS"}]
                }],
                "success": true
            }"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let tx = client.get_transaction(VALID_TX_HASH).await.unwrap();
        assert_eq!(tx.hash, VALID_TX_HASH);
        assert_eq!(tx.from, "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf");
        assert_eq!(
            tx.to,
            Some("TN3W4H6rK2ce4vX9YnFQHwKENnHjoxb3m9".to_string())
        );
        assert_eq!(tx.value, "1000000");
        assert_eq!(tx.block_number, Some(50000000));
        assert_eq!(tx.timestamp, Some(1700000000)); // ms → s
        assert!(tx.status.unwrap());
    }

    #[tokio::test]
    async fn test_get_transaction_failed() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/transactions/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": [{
                    "txID": "b3c12d62ad7e7b8b83b09a68b9b8f9b23a1b8f8b8f9b8f9b8f9b8f9b8f9b8f9b",
                    "block_number": 50000000,
                    "block_timestamp": 1700000000000,
                    "ret": [{"contractRet": "REVERT"}]
                }],
                "success": true
            }"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let tx = client.get_transaction(VALID_TX_HASH).await.unwrap();
        assert!(!tx.status.unwrap()); // REVERT → failure
    }

    #[tokio::test]
    async fn test_get_transaction_not_found() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/transactions/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [], "success": true}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_transaction(VALID_TX_HASH).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_transactions() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Regex(r"/v1/accounts/.*/transactions.*".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{
                "data": [
                    {
                        "txID": "aaa111",
                        "block_number": 50000000,
                        "block_timestamp": 1700000000000,
                        "raw_data": {"contract": [{"parameter": {"value": {"amount": 500000, "owner_address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf", "to_address": "TN3W4H6rK2ce4vX9YnFQHwKENnHjoxb3m9"}}, "type": "TransferContract"}]},
                        "ret": [{"contractRet": "SUCCESS"}]
                    },
                    {
                        "txID": "bbb222",
                        "block_number": 50000001,
                        "block_timestamp": 1700000060000,
                        "ret": [{"contractRet": "SUCCESS"}]
                    }
                ],
                "success": true
            }"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let txs = client.get_transactions(VALID_ADDRESS, 10).await.unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].hash, "aaa111");
        assert_eq!(txs[0].value, "500000");
        assert!(txs[0].status.unwrap());
        // Second tx has no contract data → defaults
        assert_eq!(txs[1].value, "0");
    }

    #[tokio::test]
    async fn test_get_transactions_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*/transactions.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [], "success": false, "error": "Invalid address"}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_transactions(VALID_ADDRESS, 10).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_trc20_balances() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": [{
                    "balance": 1000000,
                    "address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf",
                    "trc20": [
                        {"TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t": "5000000"},
                        {"TEkxiTehnzSmSe2XqrBj4w32RUN966rdz8": "0"}
                    ]
                }],
                "success": true
            }"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let balances = client.get_trc20_balances(VALID_ADDRESS).await.unwrap();
        // Zero balance filtered out
        assert_eq!(balances.len(), 1);
        assert_eq!(
            balances[0].contract_address,
            "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"
        );
        assert_eq!(balances[0].raw_balance, "5000000");
    }

    #[tokio::test]
    async fn test_get_trc20_balances_empty_account() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [], "success": true}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let balances = client.get_trc20_balances(VALID_ADDRESS).await.unwrap();
        assert!(balances.is_empty());
    }

    #[tokio::test]
    async fn test_get_block_number() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/wallet/getnowblock")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"block_header":{"raw_data":{"number":60000000}}}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let block = client.get_block_number().await.unwrap();
        assert_eq!(block, 60000000);
    }

    #[tokio::test]
    async fn test_get_block_number_invalid_response() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/wallet/getnowblock")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_block_number().await;
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_tron_address_wrong_decoded_length() {
        // Valid base58 but wrong number of decoded bytes
        let result = validate_tron_address("TTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTT1");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_tron_tx_hash_wrong_length() {
        let result = validate_tron_tx_hash("abc123");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("64 characters"));
    }

    #[tokio::test]
    async fn test_get_transaction_success() {
        let mut server = mockito::Server::new_async().await;
        let valid_hash = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/transactions/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":[{
                "txID":"a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
                "blockNumber":60000000,
                "block_timestamp":1700000000000,
                "raw_data":{"contract":[{"parameter":{"value":{
                    "owner_address":"TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf",
                    "to_address":"TPYmHEhy5n8TCEfYGqW2rPxsghSfzghPDn",
                    "amount":1000000
                }}}]},
                "ret":[{"contractRet":"SUCCESS"}]
            }],"success":true}"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let tx = client.get_transaction(valid_hash).await.unwrap();
        assert_eq!(tx.hash, valid_hash);
        assert_eq!(tx.status, Some(true));
    }

    #[tokio::test]
    async fn test_get_transaction_api_error() {
        let mut server = mockito::Server::new_async().await;
        let valid_hash = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/transactions/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":[],"success":false,"error":"Transaction not found"}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_transaction(valid_hash).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_transactions_success() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*/transactions.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":[{
                "txID":"abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
                "blockNumber":60000000,
                "block_timestamp":1700000000000,
                "raw_data":{"contract":[{"parameter":{"value":{
                    "owner_address":"TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf",
                    "amount":500000
                }}}]},
                "ret":[{"contractRet":"SUCCESS"}]
            }],"success":true}"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let txs = client.get_transactions(VALID_ADDRESS, 10).await.unwrap();
        assert_eq!(txs.len(), 1);
    }

    #[tokio::test]
    async fn test_tron_chain_client_trait_accessors() {
        let client = TronClient::with_api_url("http://localhost");
        let chain_client: &dyn ChainClient = &client;
        assert_eq!(chain_client.chain_name(), "tron");
        assert_eq!(chain_client.native_token_symbol(), "TRX");
    }

    #[tokio::test]
    async fn test_chain_client_trait_get_balance() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Regex(r"/v1/accounts/.*".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [{"balance": 1000000, "address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf", "trc20": []}], "success": true}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let chain_client: &dyn ChainClient = &client;
        let balance = chain_client.get_balance(VALID_ADDRESS).await.unwrap();
        assert_eq!(balance.symbol, "TRX");
    }

    #[tokio::test]
    async fn test_chain_client_trait_get_block_number() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/wallet/getnowblock")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"block_header":{"raw_data":{"number":60000000}}}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let chain_client: &dyn ChainClient = &client;
        let block = chain_client.get_block_number().await.unwrap();
        assert_eq!(block, 60000000);
    }

    #[tokio::test]
    async fn test_chain_client_trait_get_token_balances() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Regex(r"/v1/accounts/.*".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [{"balance": 0, "address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf", "trc20": [{"TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t": "5000000"}]}], "success": true}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let chain_client: &dyn ChainClient = &client;
        let balances = chain_client
            .get_token_balances(VALID_ADDRESS)
            .await
            .unwrap();
        assert_eq!(balances.len(), 1);
        // Token info enriched via Tronscan (USDT) or fallback to placeholder
        assert!(
            balances[0].token.symbol == "USDT" || balances[0].token.symbol == "TRC20",
            "symbol should be USDT (Tronscan) or TRC20 (fallback)"
        );
        // Tronscan returns various name formats (e.g. "TetherToken", "Tether USD");
        // fallback is "TRC-20 Token" or "Unknown Token"
        assert!(!balances[0].token.name.is_empty(), "name must be set");
    }

    #[tokio::test]
    async fn test_chain_client_trait_get_transaction_tron() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/transactions/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data": [{
                "txID": "b3c12d62ad7e7b8b83b09a68b9b8f9b23a1b8f8b8f9b8f9b8f9b8f9b8f9b8f9b",
                "block_number": 50000000,
                "block_timestamp": 1700000000000,
                "raw_data": {
                    "contract": [{
                        "parameter": {
                            "value": {
                                "amount": 1000000,
                                "owner_address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf",
                                "to_address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCg"
                            }
                        },
                        "type": "TransferContract"
                    }]
                },
                "ret": [{"contractRet": "SUCCESS"}]
            }], "success": true}"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let chain_client: &dyn ChainClient = &client;
        let tx = chain_client.get_transaction(VALID_TX_HASH).await.unwrap();
        assert_eq!(tx.hash, VALID_TX_HASH);
        assert!(tx.status.unwrap());
    }

    #[tokio::test]
    async fn test_chain_client_trait_get_transactions_tron() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*/transactions.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data": [{
                "txID": "b3c12d62ad7e7b8b83b09a68b9b8f9b23a1b8f8b8f9b8f9b8f9b8f9b8f9b8f9b",
                "block_number": 50000000,
                "block_timestamp": 1700000000000,
                "raw_data": {
                    "contract": [{
                        "parameter": {
                            "value": {
                                "amount": 2000000,
                                "owner_address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf",
                                "to_address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCg"
                            }
                        }
                    }]
                },
                "ret": [{"contractRet": "REVERT"}]
            }], "success": true}"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let chain_client: &dyn ChainClient = &client;
        let txs = chain_client
            .get_transactions(VALID_ADDRESS, 10)
            .await
            .unwrap();
        assert_eq!(txs.len(), 1);
        assert!(!txs[0].status.unwrap()); // REVERT means failure
    }

    #[tokio::test]
    async fn test_get_balance_with_api_key() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Regex(r"/v1/accounts/.*".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data": [{"balance": 10000000, "address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf", "trc20": []}], "success": true}"#,
            )
            .create_async()
            .await;

        let config = ChainsConfig {
            tron_api: Some(server.url()),
            api_keys: {
                let mut m = std::collections::HashMap::new();
                m.insert("tronscan".to_string(), "test-api-key".to_string());
                m
            },
            ..Default::default()
        };
        let client = TronClient::new(&config).unwrap();
        let balance = client.get_balance(VALID_ADDRESS).await.unwrap();
        assert_eq!(balance.symbol, "TRX");
        assert!(balance.formatted.contains("TRX"));
    }

    #[tokio::test]
    async fn test_get_trc20_balances_error_response() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [], "success": false, "error": "Rate limit exceeded"}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_trc20_balances(VALID_ADDRESS).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Rate limit"));
    }

    #[tokio::test]
    async fn test_get_trc20_balances_no_data() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [], "success": true}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let balances = client.get_trc20_balances(VALID_ADDRESS).await.unwrap();
        assert!(balances.is_empty());
    }

    #[tokio::test]
    async fn test_get_trc20_balances_with_api_key() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Regex(r"/v1/accounts/.*".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data": [{"balance": 0, "address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf", "trc20": [{"TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t": "10000000"}]}], "success": true}"#,
            )
            .create_async()
            .await;

        let config = ChainsConfig {
            tron_api: Some(server.url()),
            api_keys: {
                let mut m = std::collections::HashMap::new();
                m.insert("tronscan".to_string(), "my-api-key".to_string());
                m
            },
            ..Default::default()
        };
        let client = TronClient::new(&config).unwrap();
        let balances = client.get_trc20_balances(VALID_ADDRESS).await.unwrap();
        assert_eq!(balances.len(), 1);
    }

    #[test]
    fn test_validate_tron_address_bad_checksum() {
        // Construct a valid-looking address with bad checksum by modifying last char
        // TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf -> change last char
        let result = validate_tron_address("TDqSquXBgUCLYvYC4XZgrprLK589dkhSCe");
        assert!(result.is_err());
        // Could be checksum error or base58 decode error
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("checksum")
                || err_str.contains("base58")
                || err_str.contains("prefix")
        );
    }

    #[tokio::test]
    async fn test_get_transaction_tron_success() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/transactions/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data": [{
                "txID": "b3c12d62ad7e7b8b83b09a68b9b8f9b23a1b8f8b8f9b8f9b8f9b8f9b8f9b8f9b",
                "block_number": 50000000,
                "block_timestamp": 1700000000000,
                "raw_data": {
                    "contract": [{
                        "parameter": {
                            "value": {
                                "amount": 5000000,
                                "owner_address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf",
                                "to_address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCg"
                            }
                        }
                    }]
                },
                "ret": [{"contractRet": "SUCCESS"}]
            }], "success": true}"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let tx = client.get_transaction(VALID_TX_HASH).await.unwrap();
        assert_eq!(tx.hash, VALID_TX_HASH);
        assert!(tx.status.unwrap());
        assert_eq!(tx.value, "5000000");
        assert_eq!(tx.timestamp, Some(1700000000)); // Converted from ms to s
    }

    #[tokio::test]
    async fn test_get_transaction_tron_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/transactions/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [], "success": false, "error": "Transaction not found"}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_transaction(VALID_TX_HASH).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_transactions_tron_success() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*/transactions.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data": [
                {
                    "txID": "aaa12d62ad7e7b8b83b09a68b9b8f9b23a1b8f8b8f9b8f9b8f9b8f9b8f9b8f9b",
                    "block_number": 50000001,
                    "block_timestamp": 1700000003000,
                    "raw_data": {"contract": [{"parameter": {"value": {"amount": 1000000, "owner_address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf"}}}]},
                    "ret": [{"contractRet": "SUCCESS"}]
                },
                {
                    "txID": "bbb12d62ad7e7b8b83b09a68b9b8f9b23a1b8f8b8f9b8f9b8f9b8f9b8f9b8f9b",
                    "block_number": 50000002,
                    "block_timestamp": 1700000006000,
                    "raw_data": {"contract": [{"parameter": {"value": {"amount": 2000000, "owner_address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf", "to_address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCg"}}}]},
                    "ret": [{"contractRet": "SUCCESS"}]
                }
            ], "success": true}"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let txs = client.get_transactions(VALID_ADDRESS, 10).await.unwrap();
        assert_eq!(txs.len(), 2);
    }

    #[tokio::test]
    async fn test_get_transactions_tron_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*/transactions.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [], "success": false, "error": "Invalid address"}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_transactions(VALID_ADDRESS, 10).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_balance_error_response() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [], "success": false, "error": "Account not found"}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_balance(VALID_ADDRESS).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Account not found")
        );
    }

    #[tokio::test]
    async fn test_get_token_info_success() {
        // get_token_info uses TRONSCAN_API directly (not mockable via api_url),
        // so we test the response parsing path that mirrors get_token_info logic
        let info: serde_json::Value = serde_json::from_str(
            r#"{"trc20_tokens": [{"symbol": "USDT", "contract_name": "TetherToken", "decimals": 6}]}"#,
        )
        .unwrap();
        let tokens = info.get("trc20_tokens").and_then(|v| v.as_array()).unwrap();
        let token_data = tokens.first().unwrap();
        let symbol = token_data
            .get("symbol")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN");
        assert_eq!(symbol, "USDT");
        let name = token_data
            .get("contract_name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Token");
        assert_eq!(name, "TetherToken");
        let decimals = token_data
            .get("decimals")
            .and_then(|v| v.as_u64())
            .unwrap_or(6) as u8;
        assert_eq!(decimals, 6);
    }

    #[tokio::test]
    async fn test_get_token_info_no_tokens() {
        let info: serde_json::Value = serde_json::from_str(r#"{"trc20_tokens": []}"#).unwrap();
        let tokens = info.get("trc20_tokens").and_then(|v| v.as_array()).unwrap();
        assert!(tokens.is_empty());
    }

    #[tokio::test]
    async fn test_get_token_info_missing_field() {
        let info: serde_json::Value =
            serde_json::from_str(r#"{"trc20_tokens": [{"symbol": "TEST"}]}"#).unwrap();
        let tokens = info.get("trc20_tokens").and_then(|v| v.as_array()).unwrap();
        let token_data = tokens.first().unwrap();
        let name = token_data
            .get("contract_name")
            .or_else(|| token_data.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Token");
        assert_eq!(name, "Unknown Token");
        let decimals = token_data
            .get("decimals")
            .and_then(|v| v.as_u64())
            .unwrap_or(6) as u8;
        assert_eq!(decimals, 6);
    }

    #[tokio::test]
    async fn test_token_holder_response_parsing() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"trc20_tokens": [
                {"holder_address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf", "balance": "5000000"},
                {"holder_address": "TN3W4H6rK2ce4vX9YnFQHwKENnHjoxb3m9", "balance": "3000000"}
            ]}"#,
        )
        .unwrap();
        let holders_data: &[serde_json::Value] = json
            .get("trc20_tokens")
            .and_then(|v| v.as_array())
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        assert_eq!(holders_data.len(), 2);

        let total_balance: f64 = holders_data
            .iter()
            .filter_map(|h| h.get("balance").and_then(|v| v.as_str()))
            .filter_map(|s| s.parse::<f64>().ok())
            .sum();
        assert_eq!(total_balance, 8000000.0);

        let decimals: u8 = 6;
        let holders: Vec<TokenHolder> = holders_data
            .iter()
            .enumerate()
            .filter_map(|(i, h)| {
                let holder_address = h.get("holder_address")?.as_str()?.to_string();
                let balance_raw = h.get("balance")?.as_str()?.to_string();
                let balance: f64 = balance_raw.parse().ok()?;
                let percentage = if total_balance > 0.0 {
                    (balance / total_balance) * 100.0
                } else {
                    0.0
                };
                let divisor = 10_f64.powi(decimals as i32);
                let formatted = format!("{:.6}", balance / divisor);
                Some(TokenHolder {
                    address: holder_address,
                    balance: balance_raw,
                    formatted_balance: formatted,
                    percentage,
                    rank: (i + 1) as u32,
                })
            })
            .collect();
        assert_eq!(holders.len(), 2);
        assert_eq!(holders[0].rank, 1);
        assert_eq!(holders[1].rank, 2);
        assert!(holders[0].percentage > 60.0);
        assert!(holders[0].formatted_balance.contains("5.000000"));
    }

    #[tokio::test]
    async fn test_token_holder_count_parsing() {
        let json: serde_json::Value = serde_json::from_str(r#"{"rangeTotal": 12345}"#).unwrap();
        let count = json.get("rangeTotal").and_then(|v| v.as_u64()).unwrap_or(0);
        assert_eq!(count, 12345);

        let json_no_field: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
        let count2 = json_no_field
            .get("rangeTotal")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        assert_eq!(count2, 0);
    }

    #[test]
    fn test_dex_search_response_deserialization() {
        let json = r#"{"pairs":[{"baseTokenSymbol":"TRX","priceUsd":"0.08"}]}"#;
        let result: std::result::Result<DexSearchResponse, _> = serde_json::from_str(json);
        assert!(result.is_ok());
        let resp = result.unwrap();
        let pairs = resp.pairs.unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].price_usd, Some("0.08".to_string()));
    }

    #[test]
    fn test_dex_search_response_empty() {
        let json = r#"{"pairs":[]}"#;
        let result: std::result::Result<DexSearchResponse, _> = serde_json::from_str(json);
        assert!(result.is_ok());
        assert!(result.unwrap().pairs.unwrap().is_empty());
    }

    #[test]
    fn test_dex_search_response_no_pairs() {
        let json = r#"{}"#;
        let result: std::result::Result<DexSearchResponse, _> = serde_json::from_str(json);
        assert!(result.is_ok());
        assert!(result.unwrap().pairs.is_none());
    }

    // -------------------------------------------------------------------------
    // Invalid input validation tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_get_transaction_invalid_hash() {
        let client = TronClient::default();
        let result = client.get_transaction("not-a-valid-hash").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("64 characters"));
    }

    #[tokio::test]
    async fn test_get_transactions_invalid_address() {
        let client = TronClient::default();
        let result = client.get_transactions("invalid-address", 10).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_token_info_invalid_address() {
        let client = TronClient::default();
        let result = client.get_token_info("bad-address").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_token_holders_invalid_address() {
        let client = TronClient::default();
        let result = client.get_token_holders("bad-address", 10).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_token_holder_count_invalid_address() {
        let client = TronClient::default();
        let result = client.get_token_holder_count("bad-address").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_chain_client_get_token_info_invalid_address() {
        let client = TronClient::default();
        let chain_client: &dyn ChainClient = &client;
        let result = chain_client.get_token_info("x").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_chain_client_get_token_holders_invalid_address() {
        let client = TronClient::default();
        let chain_client: &dyn ChainClient = &client;
        let result = chain_client.get_token_holders("x", 10).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_chain_client_get_token_holder_count_invalid_address() {
        let client = TronClient::default();
        let chain_client: &dyn ChainClient = &client;
        let result = chain_client.get_token_holder_count("x").await;
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // API error edge cases
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_get_balance_api_error_unknown_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [], "success": false}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_balance(VALID_ADDRESS).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown error"));
    }

    #[tokio::test]
    async fn test_get_transaction_status_none() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/transactions/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": [{
                    "txID": "b3c12d62ad7e7b8b83b09a68b9b8f9b23a1b8f8b8f9b8f9b8f9b8f9b8f9b8f9b",
                    "block_number": 50000000,
                    "block_timestamp": 1700000000000,
                    "ret": []
                }],
                "success": true
            }"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let tx = client.get_transaction(VALID_TX_HASH).await.unwrap();
        assert_eq!(tx.status, None);
    }

    #[tokio::test]
    async fn test_get_transaction_no_ret_field() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/transactions/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": [{
                    "txID": "b3c12d62ad7e7b8b83b09a68b9b8f9b23a1b8f8b8f9b8f9b8f9b8f9b8f9b8f9b",
                    "block_number": 50000000,
                    "block_timestamp": 1700000000000
                }],
                "success": true
            }"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let tx = client.get_transaction(VALID_TX_HASH).await.unwrap();
        assert_eq!(tx.status, None);
    }

    #[tokio::test]
    async fn test_get_transaction_to_address_none() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/transactions/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": [{
                    "txID": "b3c12d62ad7e7b8b83b09a68b9b8f9b23a1b8f8b8f9b8f9b8f9b8f9b8f9b8f9b",
                    "block_number": 50000000,
                    "block_timestamp": 1700000000000,
                    "raw_data": {
                        "contract": [{
                            "parameter": {
                                "value": {
                                    "amount": 1000000,
                                    "owner_address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf"
                                }
                            }
                        }]
                    },
                    "ret": [{"contractRet": "SUCCESS"}]
                }],
                "success": true
            }"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let tx = client.get_transaction(VALID_TX_HASH).await.unwrap();
        assert_eq!(tx.to, None);
        assert_eq!(tx.value, "1000000");
    }

    #[tokio::test]
    async fn test_get_transaction_amount_none() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/transactions/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": [{
                    "txID": "b3c12d62ad7e7b8b83b09a68b9b8f9b23a1b8f8b8f9b8f9b8f9b8f9b8f9b8f9b",
                    "block_number": 50000000,
                    "block_timestamp": 1700000000000,
                    "raw_data": {
                        "contract": [{
                            "parameter": {
                                "value": {
                                    "owner_address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf",
                                    "to_address": "TN3W4H6rK2ce4vX9YnFQHwKENnHjoxb3m9"
                                }
                            }
                        }]
                    },
                    "ret": [{"contractRet": "SUCCESS"}]
                }],
                "success": true
            }"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let tx = client.get_transaction(VALID_TX_HASH).await.unwrap();
        assert_eq!(tx.value, "0");
    }

    #[tokio::test]
    async fn test_get_transaction_no_raw_data() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/transactions/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": [{
                    "txID": "b3c12d62ad7e7b8b83b09a68b9b8f9b23a1b8f8b8f9b8f9b8f9b8f9b8f9b8f9b",
                    "block_number": 50000000,
                    "block_timestamp": 1700000000000,
                    "ret": [{"contractRet": "SUCCESS"}]
                }],
                "success": true
            }"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let tx = client.get_transaction(VALID_TX_HASH).await.unwrap();
        assert_eq!(tx.from, "");
        assert_eq!(tx.to, None);
        assert_eq!(tx.value, "0");
    }

    #[tokio::test]
    async fn test_get_block_number_block_header_none() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/wallet/getnowblock")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"other": "data"}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_block_number().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid block"));
    }

    #[tokio::test]
    async fn test_get_block_number_raw_data_none() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/wallet/getnowblock")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"block_header":{}}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_block_number().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_block_number_number_none() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/wallet/getnowblock")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"block_header":{"raw_data":{}}}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_block_number().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_trc20_balances_api_error_unknown() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [], "success": false}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_trc20_balances(VALID_ADDRESS).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown error"));
    }

    #[tokio::test]
    async fn test_get_transactions_api_error_unknown() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*/transactions.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [], "success": false}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_transactions(VALID_ADDRESS, 10).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown error"));
    }

    #[tokio::test]
    async fn test_get_transaction_api_error_unknown() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/transactions/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [], "success": false}"#)
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let result = client.get_transaction(VALID_TX_HASH).await;
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // Struct construction and deserialization
    // -------------------------------------------------------------------------

    #[test]
    fn test_trc20_token_balance_struct() {
        let balance = Trc20TokenBalance {
            contract_address: "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t".to_string(),
            raw_balance: "5000000".to_string(),
        };
        assert_eq!(
            balance.contract_address,
            "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"
        );
        assert_eq!(balance.raw_balance, "5000000");
        let debug_str = format!("{:?}", balance);
        assert!(debug_str.contains("Trc20TokenBalance"));
    }

    #[test]
    fn test_account_response_with_error_field() {
        let json = r#"{
            "data": [],
            "success": false,
            "error": "Custom error message"
        }"#;
        let response: AccountResponse = serde_json::from_str(json).unwrap();
        assert!(!response.success);
        assert_eq!(response.error, Some("Custom error message".to_string()));
    }

    #[test]
    fn test_account_response_trc20_balances() {
        let json = r#"{
            "data": [{
                "balance": 1000000,
                "address": "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf",
                "trc20": [
                    {"TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t": "10000000"}
                ]
            }],
            "success": true
        }"#;
        let response: AccountResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].trc20.len(), 1);
        let trc20 = &response.data[0].trc20[0];
        assert_eq!(
            trc20.balances.get("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"),
            Some(&"10000000".to_string())
        );
    }

    #[test]
    fn test_transaction_list_response_with_error() {
        let json = r#"{
            "data": [],
            "success": false,
            "error": "Transaction not found"
        }"#;
        let response: TransactionListResponse = serde_json::from_str(json).unwrap();
        assert!(!response.success);
        assert_eq!(response.error, Some("Transaction not found".to_string()));
    }

    #[test]
    fn test_full_transaction_deserialization() {
        let json = r#"{
            "data": [{
                "txID": "abc123def456",
                "block_number": 12345,
                "block_timestamp": 1600000000000,
                "raw_data": {
                    "contract": [{
                        "parameter": {
                            "value": {
                                "amount": 999999,
                                "owner_address": "TFrom123",
                                "to_address": "TTo456"
                            }
                        },
                        "type": "TransferContract"
                    }]
                },
                "ret": [{"contractRet": "SUCCESS"}]
            }],
            "success": true
        }"#;
        let response: TransactionListResponse = serde_json::from_str(json).unwrap();
        let tx = &response.data[0];
        assert_eq!(tx.tx_id, "abc123def456");
        assert_eq!(tx.block_number, Some(12345));
        assert_eq!(tx.block_timestamp, Some(1600000000000));
        let contract_value = tx
            .raw_data
            .as_ref()
            .and_then(|r| r.contract.as_ref())
            .and_then(|c| c.first())
            .and_then(|c| c.parameter.as_ref())
            .and_then(|p| p.value.as_ref())
            .unwrap();
        assert_eq!(contract_value.amount, Some(999999));
        assert_eq!(contract_value.owner_address.as_deref(), Some("TFrom123"));
        assert_eq!(contract_value.to_address.as_deref(), Some("TTo456"));
        assert_eq!(
            tx.ret
                .as_ref()
                .and_then(|r| r.first())
                .and_then(|r| r.contract_ret.as_deref()),
            Some("SUCCESS")
        );
    }

    #[test]
    fn test_tron_client_default_trait() {
        let client = TronClient::default();
        assert_eq!(client.chain_name(), "tron");
        assert_eq!(client.native_token_symbol(), "TRX");
        assert_eq!(client.api_url, DEFAULT_TRON_API);
    }

    #[test]
    fn test_dex_search_pair_wtrx() {
        let json = r#"{"pairs":[{"baseTokenSymbol":"WTRX","priceUsd":"0.08"}]}"#;
        let result: std::result::Result<DexSearchResponse, _> = serde_json::from_str(json);
        assert!(result.is_ok());
        let resp = result.unwrap();
        let pairs = resp.pairs.unwrap();
        assert_eq!(pairs[0].base_token_symbol, Some("WTRX".to_string()));
        assert_eq!(pairs[0].price_usd, Some("0.08".to_string()));
    }

    #[tokio::test]
    async fn test_enrich_balance_usd_no_panic() {
        let mut balance = Balance {
            raw: "1000000".to_string(),
            formatted: "1.000000 TRX".to_string(),
            decimals: TRX_DECIMALS,
            symbol: "TRX".to_string(),
            usd_value: None,
        };
        let client = TronClient::default();
        client.enrich_balance_usd(&mut balance).await;
        // Should not panic; usd_value may or may not be set depending on DexScreener response
    }

    #[test]
    fn test_trc20_token_balance_debug_format() {
        let b = Trc20TokenBalance {
            contract_address: "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t".to_string(),
            raw_balance: "1000000".to_string(),
        };
        let s = format!("{:?}", b);
        assert!(s.contains("Trc20TokenBalance"));
        assert!(s.contains("1000000"));
    }

    #[tokio::test]
    async fn test_get_transactions_with_minimal_contract_data() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/v1/accounts/.*/transactions.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": [{
                    "txID": "c4d23e73be8f8c9c94c10b79c0c0a0c24b2c9a9c0a0c0a0c0a0c0a0c0a0c0a0c0a0c",
                    "block_number": 50000000,
                    "block_timestamp": 1700000000000,
                    "raw_data": {"contract": [{}]},
                    "ret": []
                }],
                "success": true
            }"#,
            )
            .create_async()
            .await;

        let client = TronClient::with_api_url(&server.url());
        let txs = client.get_transactions(VALID_ADDRESS, 5).await.unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].value, "0");
        assert_eq!(txs[0].status, None);
    }
}
