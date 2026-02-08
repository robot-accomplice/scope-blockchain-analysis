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
//! use bcc::chains::TronClient;
//! use bcc::config::ChainsConfig;
//!
//! #[tokio::main]
//! async fn main() -> bcc::Result<()> {
//!     let config = ChainsConfig::default();
//!     let client = TronClient::new(&config)?;
//!     
//!     let balance = client.get_balance("TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf").await?;
//!     println!("Balance: {} TRX", balance.formatted);
//!     Ok(())
//! }
//! ```

use crate::chains::{Balance, Transaction};
use crate::config::ChainsConfig;
use crate::error::{BccError, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json;
use sha2::{Digest, Sha256};

/// Default TronGrid API endpoint.
const DEFAULT_TRON_API: &str = "https://api.trongrid.io";

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
    /// use bcc::chains::TronClient;
    /// use bcc::config::ChainsConfig;
    ///
    /// let config = ChainsConfig::default();
    /// let client = TronClient::new(&config).unwrap();
    /// ```
    pub fn new(config: &ChainsConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| BccError::Chain(format!("Failed to create HTTP client: {}", e)))?;

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
    /// Returns [`BccError::InvalidAddress`] if the address format is invalid.
    /// Returns [`BccError::Request`] if the API request fails.
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
            return Err(BccError::Chain(format!(
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
            return Err(BccError::Chain(format!(
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

    /// Enriches a balance with a USD value using DexScreener price lookup.
    ///
    /// Note: Tron native token price lookup via DexScreener is not yet supported.
    /// This is a placeholder that uses CoinGecko-style simple price API as fallback.
    pub async fn enrich_balance_usd(&self, balance: &mut Balance) {
        // Try to get TRX price from a simple API
        let url = "https://api.dexscreener.com/latest/dex/search?q=TRX%20USDT";
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
            return Err(BccError::Chain(format!(
                "TronGrid API error: {}",
                response.error.unwrap_or_else(|| "Unknown error".into())
            )));
        }

        let tx = response
            .data
            .into_iter()
            .next()
            .ok_or_else(|| BccError::Chain("Transaction not found".into()))?;

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
            return Err(BccError::Chain(format!(
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
            .ok_or_else(|| BccError::Chain("Invalid block response".into()))
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
        return Err(BccError::InvalidAddress("Address cannot be empty".into()));
    }

    // Tron addresses start with 'T'
    if !address.starts_with('T') {
        return Err(BccError::InvalidAddress(format!(
            "Tron address must start with 'T': {}",
            address
        )));
    }

    // Tron addresses are 34 characters
    if address.len() != 34 {
        return Err(BccError::InvalidAddress(format!(
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
                return Err(BccError::InvalidAddress(format!(
                    "Tron address must decode to 25 bytes, got {}: {}",
                    bytes.len(),
                    address
                )));
            }

            // First byte should be 0x41 (Tron mainnet prefix)
            if bytes[0] != 0x41 {
                return Err(BccError::InvalidAddress(format!(
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
                return Err(BccError::InvalidAddress(format!(
                    "Invalid Tron address checksum: {}",
                    address
                )));
            }
        }
        Err(e) => {
            return Err(BccError::InvalidAddress(format!(
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
        return Err(BccError::InvalidHash("Hash cannot be empty".into()));
    }

    // Tron tx hashes are 64 hex characters (without 0x prefix)
    if hash.len() != 64 {
        return Err(BccError::InvalidHash(format!(
            "Tron transaction hash must be 64 characters, got {}: {}",
            hash.len(),
            hash
        )));
    }

    // Validate hex encoding
    if !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(BccError::InvalidHash(format!(
            "Tron hash contains invalid hex characters: {}",
            hash
        )));
    }

    Ok(())
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
}
