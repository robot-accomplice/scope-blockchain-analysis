//! # Solana Client
//!
//! This module provides a Solana blockchain client for querying balances,
//! transactions, and account information on the Solana network.
//!
//! ## Features
//!
//! - Balance queries via Solana JSON-RPC (with USD valuation via DexScreener)
//! - Transaction details lookup via `getTransaction` RPC (jsonParsed encoding)
//! - Enriched transaction history with slot, timestamp, and status from `getSignaturesForAddress`
//! - SPL token balance fetching via `getTokenAccountsByOwner`
//! - Base58 address and signature validation
//! - Support for both legacy and versioned transactions
//!
//! ## Usage
//!
//! ```rust,no_run
//! use bcc::chains::SolanaClient;
//! use bcc::config::ChainsConfig;
//!
//! #[tokio::main]
//! async fn main() -> bcc::Result<()> {
//!     let config = ChainsConfig::default();
//!     let client = SolanaClient::new(&config)?;
//!     
//!     let mut balance = client.get_balance("DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy").await?;
//!     client.enrich_balance_usd(&mut balance).await;
//!     println!("Balance: {} SOL", balance.formatted);
//!     Ok(())
//! }
//! ```

use crate::chains::{Balance, Transaction};
use crate::config::ChainsConfig;
use crate::error::{BccError, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Default Solana mainnet RPC endpoint.
const DEFAULT_SOLANA_RPC: &str = "https://api.mainnet-beta.solana.com";

/// Solscan API base URL for transaction history.
#[allow(dead_code)] // Reserved for future Solscan integration
const SOLSCAN_API_URL: &str = "https://api.solscan.io";

/// Solana native token decimals.
const SOL_DECIMALS: u8 = 9;

/// Solana blockchain client.
///
/// Supports balance queries via JSON-RPC and optional transaction
/// history via Solscan API.
#[derive(Debug, Clone)]
pub struct SolanaClient {
    /// HTTP client for API requests.
    client: Client,

    /// Solana JSON-RPC endpoint URL.
    rpc_url: String,

    /// Solscan API key for enhanced transaction data.
    #[allow(dead_code)] // Reserved for future Solscan integration
    solscan_api_key: Option<String>,
}

/// JSON-RPC request structure.
#[derive(Debug, Serialize)]
struct RpcRequest<'a, T: Serialize> {
    jsonrpc: &'a str,
    id: u64,
    method: &'a str,
    params: T,
}

/// JSON-RPC response structure.
#[derive(Debug, Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

/// JSON-RPC error structure.
#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

/// Balance response from getBalance RPC call.
#[derive(Debug, Deserialize)]
struct BalanceResponse {
    value: u64,
}

/// Response structure for getTokenAccountsByOwner.
#[derive(Debug, Deserialize)]
struct TokenAccountsResponse {
    value: Vec<TokenAccountInfo>,
}

/// Individual token account info.
#[derive(Debug, Deserialize)]
struct TokenAccountInfo {
    pubkey: String,
    account: TokenAccountData,
}

/// Token account data.
#[derive(Debug, Deserialize)]
struct TokenAccountData {
    data: TokenAccountParsedData,
}

/// Parsed token account data.
#[derive(Debug, Deserialize)]
struct TokenAccountParsedData {
    parsed: TokenAccountParsedInfo,
}

/// Parsed info containing token details.
#[derive(Debug, Deserialize)]
struct TokenAccountParsedInfo {
    info: TokenInfo,
}

/// Token balance and mint information.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenInfo {
    mint: String,
    token_amount: TokenAmount,
}

/// Token amount with UI representation.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // ui_amount_string reserved for future use
struct TokenAmount {
    amount: String,
    decimals: u8,
    ui_amount: Option<f64>,
    ui_amount_string: String,
}

/// SPL Token balance with metadata.
#[derive(Debug, Clone, Serialize)]
pub struct TokenBalance {
    /// Token mint address.
    pub mint: String,
    /// Token account address.
    pub token_account: String,
    /// Raw balance in smallest unit.
    pub raw_amount: String,
    /// Human-readable balance.
    pub ui_amount: f64,
    /// Token decimals.
    pub decimals: u8,
    /// Token symbol (if known).
    pub symbol: Option<String>,
    /// Token name (if known).
    pub name: Option<String>,
}

/// Transaction signature info from getSignaturesForAddress.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // Fields used for deserialization
struct SignatureInfo {
    signature: String,
    slot: u64,
    block_time: Option<i64>,
    err: Option<serde_json::Value>,
}

/// Solana transaction result from getTransaction RPC.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SolanaTransactionResult {
    #[serde(default)]
    slot: Option<u64>,
    #[serde(default)]
    block_time: Option<i64>,
    #[serde(default)]
    transaction: Option<SolanaTransactionData>,
    #[serde(default)]
    meta: Option<SolanaTransactionMeta>,
}

/// Transaction data from Solana RPC.
#[derive(Debug, Deserialize)]
struct SolanaTransactionData {
    #[serde(default)]
    message: Option<SolanaTransactionMessage>,
}

/// Transaction message from Solana RPC.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SolanaTransactionMessage {
    #[serde(default)]
    account_keys: Option<Vec<AccountKeyEntry>>,
}

/// Account key can be a string or an object with pubkey + signer fields.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AccountKeyEntry {
    String(String),
    Object {
        pubkey: String,
        #[serde(default)]
        #[allow(dead_code)]
        signer: bool,
    },
}

/// Transaction metadata from Solana RPC.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SolanaTransactionMeta {
    #[serde(default)]
    fee: Option<u64>,
    #[serde(default)]
    pre_balances: Option<Vec<u64>>,
    #[serde(default)]
    post_balances: Option<Vec<u64>>,
    #[serde(default)]
    err: Option<serde_json::Value>,
}

/// Solscan account info response.
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Reserved for future Solscan integration
struct SolscanAccountInfo {
    lamports: u64,
    #[serde(rename = "type")]
    account_type: Option<String>,
}

impl SolanaClient {
    /// Creates a new Solana client with the given configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - Chain configuration containing RPC endpoint and API keys
    ///
    /// # Returns
    ///
    /// Returns a configured [`SolanaClient`] instance.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use bcc::chains::SolanaClient;
    /// use bcc::config::ChainsConfig;
    ///
    /// let config = ChainsConfig::default();
    /// let client = SolanaClient::new(&config).unwrap();
    /// ```
    pub fn new(config: &ChainsConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| BccError::Chain(format!("Failed to create HTTP client: {}", e)))?;

        let rpc_url = config
            .solana_rpc
            .as_deref()
            .unwrap_or(DEFAULT_SOLANA_RPC)
            .to_string();

        Ok(Self {
            client,
            rpc_url,
            solscan_api_key: config.api_keys.get("solscan").cloned(),
        })
    }

    /// Creates a client with a custom RPC URL.
    ///
    /// # Arguments
    ///
    /// * `rpc_url` - The Solana JSON-RPC endpoint URL
    pub fn with_rpc_url(rpc_url: &str) -> Self {
        Self {
            client: Client::new(),
            rpc_url: rpc_url.to_string(),
            solscan_api_key: None,
        }
    }

    /// Returns the chain name.
    pub fn chain_name(&self) -> &str {
        "solana"
    }

    /// Returns the native token symbol.
    pub fn native_token_symbol(&self) -> &str {
        "SOL"
    }

    /// Fetches the SOL balance for an address.
    ///
    /// # Arguments
    ///
    /// * `address` - The Solana address (base58 encoded)
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
        validate_solana_address(address)?;

        let request = RpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "getBalance",
            params: vec![address],
        };

        tracing::debug!(url = %self.rpc_url, address = %address, "Fetching Solana balance");

        let response: RpcResponse<BalanceResponse> = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await?
            .json()
            .await?;

        if let Some(error) = response.error {
            return Err(BccError::Chain(format!(
                "Solana RPC error ({}): {}",
                error.code, error.message
            )));
        }

        let balance = response
            .result
            .ok_or_else(|| BccError::Chain("Empty RPC response".to_string()))?;

        let lamports = balance.value;
        let sol = lamports as f64 / 10_f64.powi(SOL_DECIMALS as i32);

        Ok(Balance {
            raw: lamports.to_string(),
            formatted: format!("{:.9} SOL", sol),
            decimals: SOL_DECIMALS,
            symbol: "SOL".to_string(),
            usd_value: None, // Populated by caller via enrich_balance_usd
        })
    }

    /// Enriches a balance with a USD value using DexScreener price lookup.
    pub async fn enrich_balance_usd(&self, balance: &mut Balance) {
        let dex = crate::chains::DexClient::new();
        if let Some(price) = dex.get_native_token_price("solana").await {
            let lamports: f64 = balance.raw.parse().unwrap_or(0.0);
            let sol = lamports / 10_f64.powi(SOL_DECIMALS as i32);
            balance.usd_value = Some(sol * price);
        }
    }

    /// Fetches all SPL token balances for an address.
    ///
    /// # Arguments
    ///
    /// * `address` - The Solana wallet address to query
    ///
    /// # Returns
    ///
    /// Returns a vector of [`TokenBalance`] containing all SPL tokens held by the address.
    ///
    /// # Errors
    ///
    /// Returns [`BccError::InvalidAddress`] if the address format is invalid.
    /// Returns [`BccError::Request`] if the API request fails.
    pub async fn get_token_balances(&self, address: &str) -> Result<Vec<TokenBalance>> {
        validate_solana_address(address)?;

        // Use getTokenAccountsByOwner to get all token accounts
        // The TOKEN_PROGRAM_ID is the standard SPL Token program
        const TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTokenAccountsByOwner",
            "params": [
                address,
                { "programId": TOKEN_PROGRAM_ID },
                { "encoding": "jsonParsed" }
            ]
        });

        tracing::debug!(url = %self.rpc_url, address = %address, "Fetching SPL token balances");

        let response: RpcResponse<TokenAccountsResponse> = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await?
            .json()
            .await?;

        if let Some(error) = response.error {
            return Err(BccError::Chain(format!(
                "Solana RPC error ({}): {}",
                error.code, error.message
            )));
        }

        let accounts = response
            .result
            .ok_or_else(|| BccError::Chain("Empty RPC response".to_string()))?;

        let token_balances: Vec<TokenBalance> = accounts
            .value
            .into_iter()
            .filter_map(|account| {
                let info = &account.account.data.parsed.info;
                let ui_amount = info.token_amount.ui_amount.unwrap_or(0.0);

                // Skip zero balances
                if ui_amount == 0.0 {
                    return None;
                }

                Some(TokenBalance {
                    mint: info.mint.clone(),
                    token_account: account.pubkey,
                    raw_amount: info.token_amount.amount.clone(),
                    ui_amount,
                    decimals: info.token_amount.decimals,
                    symbol: None, // Would need token metadata to get this
                    name: None,
                })
            })
            .collect();

        Ok(token_balances)
    }

    /// Fetches recent transaction signatures for an address.
    ///
    /// # Arguments
    ///
    /// * `address` - The Solana address to query
    /// * `limit` - Maximum number of signatures to return
    ///
    /// # Returns
    ///
    /// Returns a vector of transaction signatures.
    pub async fn get_signatures(&self, address: &str, limit: u32) -> Result<Vec<String>> {
        let infos = self.get_signature_infos(address, limit).await?;
        Ok(infos.into_iter().map(|s| s.signature).collect())
    }

    /// Fetches recent transaction signature info (with metadata) for an address.
    async fn get_signature_infos(&self, address: &str, limit: u32) -> Result<Vec<SignatureInfo>> {
        validate_solana_address(address)?;

        #[derive(Serialize)]
        struct GetSignaturesParams<'a> {
            limit: u32,
            #[serde(skip_serializing_if = "Option::is_none")]
            before: Option<&'a str>,
        }

        let request = RpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "getSignaturesForAddress",
            params: (
                address,
                GetSignaturesParams {
                    limit,
                    before: None,
                },
            ),
        };

        tracing::debug!(
            url = %self.rpc_url,
            address = %address,
            limit = %limit,
            "Fetching Solana transaction signatures"
        );

        let response: RpcResponse<Vec<SignatureInfo>> = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await?
            .json()
            .await?;

        if let Some(error) = response.error {
            return Err(BccError::Chain(format!(
                "Solana RPC error ({}): {}",
                error.code, error.message
            )));
        }

        response
            .result
            .ok_or_else(|| BccError::Chain("Empty RPC response".to_string()))
    }

    /// Fetches transaction details by signature.
    ///
    /// # Arguments
    ///
    /// * `signature` - The transaction signature (base58 encoded)
    ///
    /// # Returns
    ///
    /// Returns [`Transaction`] details.
    pub async fn get_transaction(&self, signature: &str) -> Result<Transaction> {
        // Validate signature format
        validate_solana_signature(signature)?;

        let request = RpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "getTransaction",
            params: serde_json::json!([
                signature,
                {
                    "encoding": "jsonParsed",
                    "maxSupportedTransactionVersion": 0
                }
            ]),
        };

        tracing::debug!(
            url = %self.rpc_url,
            signature = %signature,
            "Fetching Solana transaction"
        );

        let response: RpcResponse<SolanaTransactionResult> = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await?
            .json()
            .await?;

        if let Some(error) = response.error {
            return Err(BccError::Chain(format!(
                "Solana RPC error ({}): {}",
                error.code, error.message
            )));
        }

        let tx_result = response
            .result
            .ok_or_else(|| BccError::NotFound(format!("Transaction not found: {}", signature)))?;

        // Extract the first signer (fee payer) as "from"
        let from = tx_result
            .transaction
            .as_ref()
            .and_then(|tx| tx.message.as_ref())
            .and_then(|msg| msg.account_keys.as_ref())
            .and_then(|keys| keys.first())
            .map(|key| match key {
                AccountKeyEntry::String(s) => s.clone(),
                AccountKeyEntry::Object { pubkey, .. } => pubkey.clone(),
            })
            .unwrap_or_default();

        // Try to find the SOL transfer amount from the transaction
        let value = tx_result
            .meta
            .as_ref()
            .and_then(|meta| {
                let pre = meta.pre_balances.as_ref()?;
                let post = meta.post_balances.as_ref()?;
                if pre.len() >= 2 && post.len() >= 2 {
                    // Amount sent = pre[0] - post[0] - fee (fee payer's balance change minus fee)
                    let fee = meta.fee.unwrap_or(0);
                    let sent = pre[0].saturating_sub(post[0]).saturating_sub(fee);
                    if sent > 0 {
                        let sol = sent as f64 / 10_f64.powi(SOL_DECIMALS as i32);
                        return Some(format!("{:.9}", sol));
                    }
                }
                None
            })
            .unwrap_or_else(|| "0".to_string());

        // Extract "to" address (second account key, typically the recipient)
        let to = tx_result
            .transaction
            .as_ref()
            .and_then(|tx| tx.message.as_ref())
            .and_then(|msg| msg.account_keys.as_ref())
            .and_then(|keys| {
                if keys.len() >= 2 {
                    Some(match &keys[1] {
                        AccountKeyEntry::String(s) => s.clone(),
                        AccountKeyEntry::Object { pubkey, .. } => pubkey.clone(),
                    })
                } else {
                    None
                }
            });

        let fee = tx_result
            .meta
            .as_ref()
            .and_then(|meta| meta.fee)
            .unwrap_or(0);

        let status = tx_result.meta.as_ref().map(|meta| meta.err.is_none());

        Ok(Transaction {
            hash: signature.to_string(),
            block_number: tx_result.slot,
            timestamp: tx_result.block_time.map(|t| t as u64),
            from,
            to,
            value,
            gas_limit: 0, // Solana uses compute units, not gas
            gas_used: None,
            gas_price: fee.to_string(), // Use fee as gas_price equivalent
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
        validate_solana_address(address)?;

        // Get signature infos (includes slot, blockTime, err)
        let sig_infos = self.get_signature_infos(address, limit).await?;

        let transactions: Vec<Transaction> = sig_infos
            .into_iter()
            .map(|info| Transaction {
                hash: info.signature,
                block_number: Some(info.slot),
                timestamp: info.block_time.map(|t| t as u64),
                from: address.to_string(),
                to: None,
                value: "0".to_string(),
                gas_limit: 0,
                gas_used: None,
                gas_price: "0".to_string(),
                nonce: 0,
                input: String::new(),
                status: Some(info.err.is_none()),
            })
            .collect();

        Ok(transactions)
    }

    /// Fetches the current slot number (equivalent to block number).
    pub async fn get_slot(&self) -> Result<u64> {
        let request = RpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "getSlot",
            params: (),
        };

        let response: RpcResponse<u64> = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await?
            .json()
            .await?;

        if let Some(error) = response.error {
            return Err(BccError::Chain(format!(
                "Solana RPC error ({}): {}",
                error.code, error.message
            )));
        }

        response
            .result
            .ok_or_else(|| BccError::Chain("Empty RPC response".to_string()))
    }
}

impl Default for SolanaClient {
    fn default() -> Self {
        Self {
            client: Client::new(),
            rpc_url: DEFAULT_SOLANA_RPC.to_string(),
            solscan_api_key: None,
        }
    }
}

/// Validates a Solana address format (base58 encoded, 32-44 characters).
///
/// # Arguments
///
/// * `address` - The address to validate
///
/// # Returns
///
/// Returns `Ok(())` if valid, or an error describing the validation failure.
pub fn validate_solana_address(address: &str) -> Result<()> {
    // Solana addresses are base58 encoded ed25519 public keys
    // They are typically 32-44 characters long

    if address.is_empty() {
        return Err(BccError::InvalidAddress("Address cannot be empty".into()));
    }

    // Check length (base58 encoded 32-byte keys are 32-44 chars)
    if address.len() < 32 || address.len() > 44 {
        return Err(BccError::InvalidAddress(format!(
            "Solana address must be 32-44 characters, got {}: {}",
            address.len(),
            address
        )));
    }

    // Validate base58 encoding
    match bs58::decode(address).into_vec() {
        Ok(bytes) => {
            // Should decode to 32 bytes (ed25519 public key)
            if bytes.len() != 32 {
                return Err(BccError::InvalidAddress(format!(
                    "Solana address must decode to 32 bytes, got {}: {}",
                    bytes.len(),
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

/// Validates a Solana transaction signature format (base58 encoded).
///
/// # Arguments
///
/// * `signature` - The signature to validate
///
/// # Returns
///
/// Returns `Ok(())` if valid, or an error describing the validation failure.
pub fn validate_solana_signature(signature: &str) -> Result<()> {
    // Solana signatures are base58 encoded 64-byte signatures
    // They are typically 87-88 characters long

    if signature.is_empty() {
        return Err(BccError::InvalidHash("Signature cannot be empty".into()));
    }

    // Check length (base58 encoded 64-byte signatures are ~87-88 chars)
    if signature.len() < 80 || signature.len() > 90 {
        return Err(BccError::InvalidHash(format!(
            "Solana signature must be 80-90 characters, got {}: {}",
            signature.len(),
            signature
        )));
    }

    // Validate base58 encoding
    match bs58::decode(signature).into_vec() {
        Ok(bytes) => {
            // Should decode to 64 bytes (ed25519 signature)
            if bytes.len() != 64 {
                return Err(BccError::InvalidHash(format!(
                    "Solana signature must decode to 64 bytes, got {}: {}",
                    bytes.len(),
                    signature
                )));
            }
        }
        Err(e) => {
            return Err(BccError::InvalidHash(format!(
                "Invalid base58 encoding: {}: {}",
                e, signature
            )));
        }
    }

    Ok(())
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Valid Solana address (Phantom treasury)
    const VALID_ADDRESS: &str = "DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy";

    // Valid Solana transaction signature
    const VALID_SIGNATURE: &str =
        "5VERv8NMvzbJMEkV8xnrLkEaWRtSz9CosKDYjCJjBRnbJLgp8uirBgmQpjKhoR4tjF3ZpRzrFmBV6UjKdiSZkQUW";

    #[test]
    fn test_validate_solana_address_valid() {
        assert!(validate_solana_address(VALID_ADDRESS).is_ok());
    }

    #[test]
    fn test_validate_solana_address_empty() {
        let result = validate_solana_address("");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_validate_solana_address_too_short() {
        let result = validate_solana_address("DRpbCBMxVnDK7maPM5t");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("32-44"));
    }

    #[test]
    fn test_validate_solana_address_too_long() {
        let long_addr = "DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hyAAAAAAAAAAAA";
        let result = validate_solana_address(long_addr);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_solana_address_invalid_base58() {
        // Contains '0' which is not valid base58
        let result = validate_solana_address("0RpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("base58"));
    }

    #[test]
    fn test_validate_solana_address_wrong_decoded_length() {
        // Valid base58 but decodes to wrong byte length (not 32 bytes)
        // "abc" is valid base58 but too short when decoded
        let result = validate_solana_address("abcdefghijabcdefghijabcdefghijab");
        assert!(result.is_err());
        // Should fail due to decoded length being wrong
    }

    #[test]
    fn test_validate_solana_signature_valid() {
        assert!(validate_solana_signature(VALID_SIGNATURE).is_ok());
    }

    #[test]
    fn test_validate_solana_signature_empty() {
        let result = validate_solana_signature("");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_validate_solana_signature_too_short() {
        let result = validate_solana_signature("5VERv8NMvzbJMEkV8xnrLkEaWRtSz9CosK");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("80-90"));
    }

    #[test]
    fn test_solana_client_default() {
        let client = SolanaClient::default();
        assert_eq!(client.chain_name(), "solana");
        assert_eq!(client.native_token_symbol(), "SOL");
        assert!(client.rpc_url.contains("mainnet-beta"));
    }

    #[test]
    fn test_solana_client_with_rpc_url() {
        let client = SolanaClient::with_rpc_url("https://custom.rpc.com");
        assert_eq!(client.rpc_url, "https://custom.rpc.com");
    }

    #[test]
    fn test_solana_client_new() {
        let config = ChainsConfig::default();
        let client = SolanaClient::new(&config);
        assert!(client.is_ok());
    }

    #[test]
    fn test_solana_client_new_with_custom_rpc() {
        let config = ChainsConfig {
            solana_rpc: Some("https://my-solana-rpc.com".to_string()),
            ..Default::default()
        };
        let client = SolanaClient::new(&config).unwrap();
        assert_eq!(client.rpc_url, "https://my-solana-rpc.com");
    }

    #[test]
    fn test_solana_client_new_with_api_key() {
        use std::collections::HashMap;

        let mut api_keys = HashMap::new();
        api_keys.insert("solscan".to_string(), "test-key".to_string());

        let config = ChainsConfig {
            api_keys,
            ..Default::default()
        };

        let client = SolanaClient::new(&config).unwrap();
        assert_eq!(client.solscan_api_key, Some("test-key".to_string()));
    }

    #[test]
    fn test_rpc_request_serialization() {
        let request = RpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "getBalance",
            params: vec!["test"],
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("jsonrpc"));
        assert!(json.contains("getBalance"));
    }

    #[test]
    fn test_rpc_response_deserialization() {
        let json = r#"{"jsonrpc":"2.0","result":{"value":1000000000},"id":1}"#;
        let response: RpcResponse<BalanceResponse> = serde_json::from_str(json).unwrap();
        assert!(response.result.is_some());
        assert_eq!(response.result.unwrap().value, 1_000_000_000);
    }

    #[test]
    fn test_rpc_error_deserialization() {
        let json =
            r#"{"jsonrpc":"2.0","error":{"code":-32600,"message":"Invalid request"},"id":1}"#;
        let response: RpcResponse<BalanceResponse> = serde_json::from_str(json).unwrap();
        assert!(response.error.is_some());
        let error = response.error.unwrap();
        assert_eq!(error.code, -32600);
        assert_eq!(error.message, "Invalid request");
    }
}
