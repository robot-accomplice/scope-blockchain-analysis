//! # DEX Aggregator Client
//!
//! This module provides a client for fetching token price and volume data
//! from DEX aggregator APIs like DexScreener.
//!
//! ## Supported APIs
//!
//! - **DexScreener** (primary): Free API, no key required
//!   - Token data: `GET https://api.dexscreener.com/latest/dex/tokens/{address}`
//!   - Pair data: `GET https://api.dexscreener.com/latest/dex/pairs/{chain}/{pair}`
//!   - Token search: `GET https://api.dexscreener.com/latest/dex/search?q={query}`
//!
//! ## Features
//!
//! - Comprehensive token data aggregation across all DEX pairs
//! - Native token price lookups for USD valuation (ETH, SOL, BNB, MATIC, etc.)
//! - Individual token price lookups by contract address
//! - Token search with chain filtering
//! - Historical price and volume data interpolation
//!
//! ## Usage
//!
//! ```rust,no_run
//! use scope::chains::DexClient;
//!
//! #[tokio::main]
//! async fn main() -> scope::Result<()> {
//!     let client = DexClient::new();
//!     
//!     // Fetch token data by address
//!     let data = client.get_token_data("ethereum", "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").await?;
//!     println!("Price: ${}", data.price_usd);
//!     
//!     // Get native token price for USD valuation
//!     if let Some(eth_price) = client.get_native_token_price("ethereum").await {
//!         println!("ETH: ${:.2}", eth_price);
//!     }
//!     
//!     Ok(())
//! }
//! ```

use crate::chains::{DexPair, PricePoint, VolumePoint};
use crate::error::{Result, ScopeError};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

/// DexScreener API base URL.
const DEXSCREENER_API_BASE: &str = "https://api.dexscreener.com";

/// Trait for DEX data providers (prices, token data, search).
///
/// Abstracts the DexScreener API to enable dependency injection and testing.
#[async_trait]
pub trait DexDataSource: Send + Sync {
    /// Fetches the price for a specific token on a chain.
    async fn get_token_price(&self, chain: &str, address: &str) -> Option<f64>;

    /// Fetches the native token price for a chain (e.g., ETH for ethereum).
    async fn get_native_token_price(&self, chain: &str) -> Option<f64>;

    /// Fetches comprehensive token data including pairs, volume, liquidity.
    async fn get_token_data(&self, chain: &str, address: &str) -> Result<DexTokenData>;

    /// Searches for tokens by query string with optional chain filter.
    async fn search_tokens(
        &self,
        query: &str,
        chain: Option<&str>,
    ) -> Result<Vec<TokenSearchResult>>;
}

/// Client for fetching DEX aggregator data.
#[derive(Debug, Clone)]
pub struct DexClient {
    http: Client,
    base_url: String,
}

/// Response from DexScreener token endpoint.
#[derive(Debug, Deserialize)]
struct DexScreenerTokenResponse {
    pairs: Option<Vec<DexScreenerPair>>,
}

/// A trading pair from DexScreener.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DexScreenerPair {
    chain_id: String,
    dex_id: String,
    pair_address: String,
    base_token: DexScreenerToken,
    quote_token: DexScreenerToken,
    #[serde(default)]
    price_usd: Option<String>,
    #[serde(default)]
    price_change: Option<DexScreenerPriceChange>,
    #[serde(default)]
    volume: Option<DexScreenerVolume>,
    #[serde(default)]
    liquidity: Option<DexScreenerLiquidity>,
    #[serde(default)]
    fdv: Option<f64>,
    #[serde(default)]
    market_cap: Option<f64>,
    /// Direct URL to the pair on DexScreener.
    #[serde(default)]
    url: Option<String>,
    /// Timestamp when the pair was created.
    #[serde(default)]
    pair_created_at: Option<i64>,
    /// Transaction counts for buy/sell analysis.
    #[serde(default)]
    txns: Option<DexScreenerTxns>,
    /// Token metadata including socials and websites.
    #[serde(default)]
    info: Option<DexScreenerInfo>,
}

/// Token info from DexScreener.
#[derive(Debug, Deserialize)]
struct DexScreenerToken {
    address: String,
    name: String,
    symbol: String,
}

/// Price change percentages from DexScreener.
#[derive(Debug, Deserialize)]
struct DexScreenerPriceChange {
    h24: Option<f64>,
    h6: Option<f64>,
    h1: Option<f64>,
    m5: Option<f64>,
}

/// Volume data from DexScreener.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DexScreenerVolume {
    h24: Option<f64>,
    h6: Option<f64>,
    h1: Option<f64>,
    m5: Option<f64>,
}

/// Liquidity data from DexScreener.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DexScreenerLiquidity {
    usd: Option<f64>,
    base: Option<f64>,
    quote: Option<f64>,
}

/// Transaction counts from DexScreener (buy/sell activity).
#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
struct DexScreenerTxns {
    #[serde(default)]
    h24: Option<TxnCounts>,
    #[serde(default)]
    h6: Option<TxnCounts>,
    #[serde(default)]
    h1: Option<TxnCounts>,
    #[serde(default)]
    m5: Option<TxnCounts>,
}

/// Buy/sell transaction counts for a time period.
#[derive(Debug, Deserialize, Clone, Default)]
struct TxnCounts {
    #[serde(default)]
    buys: u64,
    #[serde(default)]
    sells: u64,
}

/// Token metadata from DexScreener info endpoint.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DexScreenerInfo {
    #[serde(default)]
    image_url: Option<String>,
    #[serde(default)]
    websites: Option<Vec<DexScreenerWebsite>>,
    #[serde(default)]
    socials: Option<Vec<DexScreenerSocial>>,
}

/// Website info from DexScreener.
#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
struct DexScreenerWebsite {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

/// Social media info from DexScreener.
#[derive(Debug, Deserialize, Clone)]
struct DexScreenerSocial {
    #[serde(rename = "type", default)]
    platform: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

/// Aggregated token data from DEX sources.
#[derive(Debug, Clone)]
pub struct DexTokenData {
    /// Token contract address.
    pub address: String,

    /// Token symbol.
    pub symbol: String,

    /// Token name.
    pub name: String,

    /// Current price in USD.
    pub price_usd: f64,

    /// 24-hour price change percentage.
    pub price_change_24h: f64,

    /// 6-hour price change percentage.
    pub price_change_6h: f64,

    /// 1-hour price change percentage.
    pub price_change_1h: f64,

    /// 5-minute price change percentage.
    pub price_change_5m: f64,

    /// 24-hour trading volume in USD.
    pub volume_24h: f64,

    /// 6-hour trading volume in USD.
    pub volume_6h: f64,

    /// 1-hour trading volume in USD.
    pub volume_1h: f64,

    /// Total liquidity across all pairs in USD.
    pub liquidity_usd: f64,

    /// Market capitalization (if available).
    pub market_cap: Option<f64>,

    /// Fully diluted valuation (if available).
    pub fdv: Option<f64>,

    /// All trading pairs for this token.
    pub pairs: Vec<DexPair>,

    /// Historical price points (derived from multiple time frames).
    pub price_history: Vec<PricePoint>,

    /// Historical volume points (derived from multiple time frames).
    pub volume_history: Vec<VolumePoint>,

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

    /// Earliest pair creation timestamp (token age indicator).
    pub earliest_pair_created_at: Option<i64>,

    /// Token image URL.
    pub image_url: Option<String>,

    /// Token website URLs.
    pub websites: Vec<String>,

    /// Token social media links.
    pub socials: Vec<TokenSocial>,

    /// DexScreener URL for the token.
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

/// A token search result from DEX aggregator.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenSearchResult {
    /// Token contract address.
    pub address: String,

    /// Token symbol.
    pub symbol: String,

    /// Token name.
    pub name: String,

    /// Blockchain network.
    pub chain: String,

    /// Current price in USD (if available).
    pub price_usd: Option<f64>,

    /// 24-hour trading volume in USD.
    pub volume_24h: f64,

    /// Total liquidity in USD.
    pub liquidity_usd: f64,

    /// Market cap (if available).
    pub market_cap: Option<f64>,
}

/// A discovered token from DexScreener (profiles, boosts, etc.)
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoverToken {
    pub chain_id: String,
    pub token_address: String,
    pub url: String,
    pub description: Option<String>,
    pub links: Vec<DiscoverLink>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoverLink {
    pub label: Option<String>,
    pub link_type: Option<String>,
    pub url: String,
}

/// Response from DexScreener search endpoint.
#[derive(Debug, Deserialize)]
struct DexScreenerSearchResponse {
    pairs: Option<Vec<DexScreenerPair>>,
}

impl DexClient {
    /// Creates a new DEX client.
    pub fn new() -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            http,
            base_url: DEXSCREENER_API_BASE.to_string(),
        }
    }

    /// Creates a new DEX client with a custom base URL (for testing).
    #[cfg(test)]
    pub(crate) fn with_base_url(base_url: &str) -> Self {
        Self {
            http: Client::new(),
            base_url: base_url.to_string(),
        }
    }

    /// Maps chain names to DexScreener chain IDs.
    fn map_chain_to_dexscreener(chain: &str) -> String {
        match chain.to_lowercase().as_str() {
            "ethereum" | "eth" => "ethereum".to_string(),
            "polygon" | "matic" => "polygon".to_string(),
            "arbitrum" | "arb" => "arbitrum".to_string(),
            "optimism" | "op" => "optimism".to_string(),
            "base" => "base".to_string(),
            "bsc" | "bnb" => "bsc".to_string(),
            "solana" | "sol" => "solana".to_string(),
            "avalanche" | "avax" => "avalanche".to_string(),
            _ => chain.to_lowercase(),
        }
    }

    /// Fetches the USD price of a token by its address.
    ///
    /// Returns `None` if the token is not found or has no price data.
    pub async fn get_token_price(&self, chain: &str, token_address: &str) -> Option<f64> {
        let url = format!("{}/latest/dex/tokens/{}", self.base_url, token_address);

        let response = self.http.get(&url).send().await.ok()?;
        let dex_response: DexScreenerTokenResponse = response.json().await.ok()?;

        let dex_chain = Self::map_chain_to_dexscreener(chain);

        dex_response
            .pairs
            .as_ref()?
            .iter()
            .filter(|p| p.chain_id.to_lowercase() == dex_chain)
            .filter_map(|p| p.price_usd.as_ref()?.parse::<f64>().ok())
            .next()
    }

    /// Fetches the native token price for a chain.
    ///
    /// Uses well-known wrapped token addresses to determine the native token price.
    pub async fn get_native_token_price(&self, chain: &str) -> Option<f64> {
        let (search_chain, token_address) = match chain.to_lowercase().as_str() {
            "ethereum" | "eth" => ("ethereum", "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"), // WETH
            "polygon" | "matic" => ("polygon", "0x0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270"), // WMATIC
            "arbitrum" | "arb" => ("arbitrum", "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1"), // WETH on Arb
            "optimism" | "op" => ("optimism", "0x4200000000000000000000000000000000000006"), // WETH on OP
            "base" => ("base", "0x4200000000000000000000000000000000000006"), // WETH on Base
            "bsc" | "bnb" => ("bsc", "0xbb4CdB9CBd36B01bD1cBaEBF2De08d9173bc095c"), // WBNB
            "solana" | "sol" => ("solana", "So11111111111111111111111111111111111111112"), // Wrapped SOL
            "tron" | "trx" => return None, // Tron wrapped token varies; skip for now
            _ => return None,
        };

        self.get_token_price(search_chain, token_address).await
    }

    /// Fetches token data from DexScreener.
    ///
    /// # Arguments
    ///
    /// * `chain` - The blockchain name (e.g., "ethereum", "bsc")
    /// * `token_address` - The token contract address
    ///
    /// # Returns
    ///
    /// Returns aggregated token data from all DEX pairs.
    pub async fn get_token_data(&self, chain: &str, token_address: &str) -> Result<DexTokenData> {
        let url = format!("{}/latest/dex/tokens/{}", self.base_url, token_address);

        tracing::debug!(url = %url, "Fetching token data from DexScreener");

        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ScopeError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(ScopeError::Api(format!(
                "DexScreener API error: {}",
                response.status()
            )));
        }

        let data: DexScreenerTokenResponse = response
            .json()
            .await
            .map_err(|e| ScopeError::Api(format!("Failed to parse DexScreener response: {}", e)))?;

        let pairs = data.pairs.unwrap_or_default();

        if pairs.is_empty() {
            return Err(ScopeError::NotFound(format!(
                "No DEX pairs found for token {}",
                token_address
            )));
        }

        // Filter pairs by chain
        let chain_id = Self::map_chain_to_dexscreener(chain);
        let chain_pairs: Vec<_> = pairs
            .iter()
            .filter(|p| p.chain_id.to_lowercase() == chain_id)
            .collect();

        // Use all pairs if no chain-specific pairs found
        let relevant_pairs = if chain_pairs.is_empty() {
            pairs.iter().collect()
        } else {
            chain_pairs
        };

        // Get token info from first pair
        let first_pair = &relevant_pairs[0];
        let is_base_token =
            first_pair.base_token.address.to_lowercase() == token_address.to_lowercase();
        let token_info = if is_base_token {
            &first_pair.base_token
        } else {
            &first_pair.quote_token
        };

        // Aggregate data from all pairs
        let mut total_volume_24h = 0.0;
        let mut total_volume_6h = 0.0;
        let mut total_volume_1h = 0.0;
        let mut total_liquidity = 0.0;
        let mut weighted_price_sum = 0.0;
        let mut liquidity_weight_sum = 0.0;
        let mut dex_pairs = Vec::new();

        for pair in &relevant_pairs {
            let pair_liquidity = pair.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0);

            let pair_price = pair
                .price_usd
                .as_ref()
                .and_then(|p| p.parse::<f64>().ok())
                .unwrap_or(0.0);

            if let Some(vol) = &pair.volume {
                total_volume_24h += vol.h24.unwrap_or(0.0);
                total_volume_6h += vol.h6.unwrap_or(0.0);
                total_volume_1h += vol.h1.unwrap_or(0.0);
            }

            total_liquidity += pair_liquidity;

            // Weight price by liquidity for more accurate average
            if pair_liquidity > 0.0 && pair_price > 0.0 {
                weighted_price_sum += pair_price * pair_liquidity;
                liquidity_weight_sum += pair_liquidity;
            }

            let price_change = pair
                .price_change
                .as_ref()
                .and_then(|pc| pc.h24)
                .unwrap_or(0.0);

            // Extract transaction counts
            let txn_counts_24h = pair.txns.as_ref().and_then(|t| t.h24.clone());
            let txn_counts_6h = pair.txns.as_ref().and_then(|t| t.h6.clone());
            let txn_counts_1h = pair.txns.as_ref().and_then(|t| t.h1.clone());

            dex_pairs.push(DexPair {
                dex_name: pair.dex_id.clone(),
                pair_address: pair.pair_address.clone(),
                base_token: pair.base_token.symbol.clone(),
                quote_token: pair.quote_token.symbol.clone(),
                price_usd: pair_price,
                volume_24h: pair.volume.as_ref().and_then(|v| v.h24).unwrap_or(0.0),
                liquidity_usd: pair_liquidity,
                price_change_24h: price_change,
                buys_24h: txn_counts_24h.as_ref().map(|t| t.buys).unwrap_or(0),
                sells_24h: txn_counts_24h.as_ref().map(|t| t.sells).unwrap_or(0),
                buys_6h: txn_counts_6h.as_ref().map(|t| t.buys).unwrap_or(0),
                sells_6h: txn_counts_6h.as_ref().map(|t| t.sells).unwrap_or(0),
                buys_1h: txn_counts_1h.as_ref().map(|t| t.buys).unwrap_or(0),
                sells_1h: txn_counts_1h.as_ref().map(|t| t.sells).unwrap_or(0),
                pair_created_at: pair.pair_created_at,
                url: pair.url.clone(),
            });
        }

        // Calculate weighted average price
        let avg_price = if liquidity_weight_sum > 0.0 {
            weighted_price_sum / liquidity_weight_sum
        } else {
            first_pair
                .price_usd
                .as_ref()
                .and_then(|p| p.parse().ok())
                .unwrap_or(0.0)
        };

        // Get price change and market data from the highest liquidity pair
        let best_pair = relevant_pairs
            .iter()
            .max_by(|a, b| {
                let liq_a = a.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0);
                let liq_b = b.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0);
                liq_a
                    .partial_cmp(&liq_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();

        let price_change_24h = best_pair
            .price_change
            .as_ref()
            .and_then(|pc| pc.h24)
            .unwrap_or(0.0);

        let price_change_6h = best_pair
            .price_change
            .as_ref()
            .and_then(|pc| pc.h6)
            .unwrap_or(0.0);

        let price_change_1h = best_pair
            .price_change
            .as_ref()
            .and_then(|pc| pc.h1)
            .unwrap_or(0.0);

        let price_change_5m = best_pair
            .price_change
            .as_ref()
            .and_then(|pc| pc.m5)
            .unwrap_or(0.0);

        // Aggregate transaction counts across all pairs
        let total_buys_24h: u64 = dex_pairs.iter().map(|p| p.buys_24h).sum();
        let total_sells_24h: u64 = dex_pairs.iter().map(|p| p.sells_24h).sum();
        let total_buys_6h: u64 = dex_pairs.iter().map(|p| p.buys_6h).sum();
        let total_sells_6h: u64 = dex_pairs.iter().map(|p| p.sells_6h).sum();
        let total_buys_1h: u64 = dex_pairs.iter().map(|p| p.buys_1h).sum();
        let total_sells_1h: u64 = dex_pairs.iter().map(|p| p.sells_1h).sum();

        // Find earliest pair creation timestamp
        let earliest_pair_created_at = dex_pairs.iter().filter_map(|p| p.pair_created_at).min();

        // Extract token metadata from best pair
        let image_url = best_pair.info.as_ref().and_then(|i| i.image_url.clone());
        let websites: Vec<String> = best_pair
            .info
            .as_ref()
            .and_then(|i| i.websites.as_ref())
            .map(|ws| ws.iter().filter_map(|w| w.url.clone()).collect())
            .unwrap_or_default();
        let socials: Vec<TokenSocial> = best_pair
            .info
            .as_ref()
            .and_then(|i| i.socials.as_ref())
            .map(|ss| {
                ss.iter()
                    .filter_map(|s| {
                        Some(TokenSocial {
                            platform: s.platform.clone()?,
                            url: s.url.clone()?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let dexscreener_url = best_pair.url.clone();

        // Generate synthetic price history from change percentages
        let now = chrono::Utc::now().timestamp();
        let price_history = Self::generate_price_history(avg_price, best_pair, now);

        // Generate synthetic volume history
        let volume_history =
            Self::generate_volume_history(total_volume_24h, total_volume_6h, total_volume_1h, now);

        Ok(DexTokenData {
            address: token_address.to_string(),
            symbol: token_info.symbol.clone(),
            name: token_info.name.clone(),
            price_usd: avg_price,
            price_change_24h,
            price_change_6h,
            price_change_1h,
            price_change_5m,
            volume_24h: total_volume_24h,
            volume_6h: total_volume_6h,
            volume_1h: total_volume_1h,
            liquidity_usd: total_liquidity,
            market_cap: best_pair.market_cap,
            fdv: best_pair.fdv,
            pairs: dex_pairs,
            price_history,
            volume_history,
            total_buys_24h,
            total_sells_24h,
            total_buys_6h,
            total_sells_6h,
            total_buys_1h,
            total_sells_1h,
            earliest_pair_created_at,
            image_url,
            websites,
            socials,
            dexscreener_url,
        })
    }

    /// Searches for tokens by name or symbol.
    ///
    /// # Arguments
    ///
    /// * `query` - The search query (token name or symbol)
    /// * `chain` - Optional chain filter (e.g., "ethereum", "bsc")
    ///
    /// # Returns
    ///
    /// Returns a vector of matching tokens sorted by liquidity.
    pub async fn search_tokens(
        &self,
        query: &str,
        chain: Option<&str>,
    ) -> Result<Vec<TokenSearchResult>> {
        let url = format!(
            "{}/latest/dex/search?q={}",
            self.base_url,
            urlencoding::encode(query)
        );

        tracing::debug!(url = %url, "Searching tokens on DexScreener");

        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ScopeError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(ScopeError::Api(format!(
                "DexScreener search API error: {}",
                response.status()
            )));
        }

        let data: DexScreenerSearchResponse = response
            .json()
            .await
            .map_err(|e| ScopeError::Api(format!("Failed to parse search response: {}", e)))?;

        let pairs = data.pairs.unwrap_or_default();

        if pairs.is_empty() {
            return Ok(Vec::new());
        }

        // Filter by chain if specified
        let chain_id = chain.map(Self::map_chain_to_dexscreener);
        let filtered_pairs: Vec<_> = if let Some(ref cid) = chain_id {
            pairs
                .iter()
                .filter(|p| p.chain_id.to_lowercase() == *cid)
                .collect()
        } else {
            pairs.iter().collect()
        };

        // Deduplicate tokens by address and aggregate data
        let mut token_map: std::collections::HashMap<String, TokenSearchResult> =
            std::collections::HashMap::new();

        for pair in filtered_pairs {
            // Check if the query matches base or quote token
            let base_matches = pair
                .base_token
                .symbol
                .to_lowercase()
                .contains(&query.to_lowercase())
                || pair
                    .base_token
                    .name
                    .to_lowercase()
                    .contains(&query.to_lowercase());
            let quote_matches = pair
                .quote_token
                .symbol
                .to_lowercase()
                .contains(&query.to_lowercase())
                || pair
                    .quote_token
                    .name
                    .to_lowercase()
                    .contains(&query.to_lowercase());

            let token_info = if base_matches {
                &pair.base_token
            } else if quote_matches {
                &pair.quote_token
            } else {
                // Use base token by default
                &pair.base_token
            };

            let key = format!("{}:{}", pair.chain_id, token_info.address.to_lowercase());

            let pair_liquidity = pair.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0);

            let pair_volume = pair.volume.as_ref().and_then(|v| v.h24).unwrap_or(0.0);

            let pair_price = pair.price_usd.as_ref().and_then(|p| p.parse::<f64>().ok());

            let entry = token_map.entry(key).or_insert_with(|| TokenSearchResult {
                address: token_info.address.clone(),
                symbol: token_info.symbol.clone(),
                name: token_info.name.clone(),
                chain: pair.chain_id.clone(),
                price_usd: pair_price,
                volume_24h: 0.0,
                liquidity_usd: 0.0,
                market_cap: pair.market_cap,
            });

            // Aggregate volume and liquidity
            entry.volume_24h += pair_volume;
            entry.liquidity_usd += pair_liquidity;

            // Update price if better data available
            if entry.price_usd.is_none() && pair_price.is_some() {
                entry.price_usd = pair_price;
            }

            // Update market cap if available
            if entry.market_cap.is_none() && pair.market_cap.is_some() {
                entry.market_cap = pair.market_cap;
            }
        }

        // Convert to vector and sort by liquidity (descending)
        let mut results: Vec<TokenSearchResult> = token_map.into_values().collect();
        results.sort_by(|a, b| {
            b.liquidity_usd
                .partial_cmp(&a.liquidity_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Limit results
        results.truncate(20);

        Ok(results)
    }

    /// Fetches latest token profiles (featured tokens) from DexScreener.
    pub async fn get_token_profiles(&self) -> Result<Vec<DiscoverToken>> {
        let url = format!("{}/token-profiles/latest/v1", self.base_url);
        self.fetch_discover_tokens(&url).await
    }

    /// Fetches latest boosted tokens from DexScreener.
    pub async fn get_token_boosts(&self) -> Result<Vec<DiscoverToken>> {
        let url = format!("{}/token-boosts/latest/v1", self.base_url);
        self.fetch_discover_tokens(&url).await
    }

    /// Fetches top boosted tokens (most active boosts) from DexScreener.
    pub async fn get_token_boosts_top(&self) -> Result<Vec<DiscoverToken>> {
        let url = format!("{}/token-boosts/top/v1", self.base_url);
        self.fetch_discover_tokens(&url).await
    }

    async fn fetch_discover_tokens(&self, url: &str) -> Result<Vec<DiscoverToken>> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| ScopeError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(ScopeError::Api(format!(
                "DexScreener API error: {}",
                response.status()
            )));
        }

        #[derive(Deserialize)]
        struct TokenProfileRaw {
            url: Option<String>,
            #[serde(rename = "chainId")]
            chain_id: Option<String>,
            #[serde(rename = "tokenAddress")]
            token_address: Option<String>,
            description: Option<String>,
            links: Option<Vec<LinkRaw>>,
        }

        #[derive(Deserialize)]
        struct LinkRaw {
            label: Option<String>,
            #[serde(rename = "type")]
            link_type: Option<String>,
            url: Option<String>,
        }

        let raw: Vec<TokenProfileRaw> = response
            .json()
            .await
            .map_err(|e| ScopeError::Api(format!("Failed to parse response: {}", e)))?;

        let tokens: Vec<DiscoverToken> = raw
            .into_iter()
            .filter_map(|r| {
                let token_address = r.token_address?;
                let chain_id = r.chain_id.clone().unwrap_or_else(|| "unknown".to_string());
                let url = r.url.clone().unwrap_or_else(|| {
                    format!("https://dexscreener.com/{}/{}", chain_id, token_address)
                });
                let links: Vec<DiscoverLink> = r
                    .links
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|l| {
                        let url = l.url?;
                        Some(DiscoverLink {
                            label: l.label,
                            link_type: l.link_type,
                            url,
                        })
                    })
                    .collect();

                Some(DiscoverToken {
                    chain_id,
                    token_address,
                    url,
                    description: r.description,
                    links,
                })
            })
            .collect();

        Ok(tokens)
    }

    /// Generates synthetic price history from change percentages.
    fn generate_price_history(
        current_price: f64,
        pair: &DexScreenerPair,
        now: i64,
    ) -> Vec<PricePoint> {
        let mut history = Vec::new();

        // Get price changes at different intervals
        let changes = pair.price_change.as_ref();
        let change_24h = changes.and_then(|c| c.h24).unwrap_or(0.0) / 100.0;
        let change_6h = changes.and_then(|c| c.h6).unwrap_or(0.0) / 100.0;
        let change_1h = changes.and_then(|c| c.h1).unwrap_or(0.0) / 100.0;
        let change_5m = changes.and_then(|c| c.m5).unwrap_or(0.0) / 100.0;

        // Calculate historical prices (working backwards)
        let price_24h_ago = current_price / (1.0 + change_24h);
        let price_6h_ago = current_price / (1.0 + change_6h);
        let price_1h_ago = current_price / (1.0 + change_1h);
        let price_5m_ago = current_price / (1.0 + change_5m);

        // Add points at known intervals
        history.push(PricePoint {
            timestamp: now - 86400, // 24h ago
            price: price_24h_ago,
        });
        history.push(PricePoint {
            timestamp: now - 21600, // 6h ago
            price: price_6h_ago,
        });
        history.push(PricePoint {
            timestamp: now - 3600, // 1h ago
            price: price_1h_ago,
        });
        history.push(PricePoint {
            timestamp: now - 300, // 5m ago
            price: price_5m_ago,
        });
        history.push(PricePoint {
            timestamp: now,
            price: current_price,
        });

        // Interpolate additional points for smoother charts
        Self::interpolate_points(&mut history, 24);

        history.sort_by_key(|p| p.timestamp);
        history
    }

    /// Generates synthetic volume history from known data points.
    fn generate_volume_history(
        volume_24h: f64,
        volume_6h: f64,
        volume_1h: f64,
        now: i64,
    ) -> Vec<VolumePoint> {
        let mut history = Vec::new();

        // Create hourly buckets for the last 24 hours
        let hourly_avg = volume_24h / 24.0;

        for i in 0..24 {
            let timestamp = now - (23 - i) * 3600;
            let hours_ago = 24 - i;

            // Adjust volume based on known data points
            let volume = if hours_ago <= 1 {
                volume_1h
            } else if hours_ago <= 6 {
                volume_6h / 6.0
            } else {
                // Use average with some variation
                hourly_avg * (0.8 + (i as f64 / 24.0) * 0.4)
            };

            history.push(VolumePoint { timestamp, volume });
        }

        history
    }

    /// Interpolates additional price points for smoother charts.
    fn interpolate_points(history: &mut Vec<PricePoint>, target_count: usize) {
        if history.len() >= target_count {
            return;
        }

        history.sort_by_key(|p| p.timestamp);

        let mut interpolated = Vec::new();
        for window in history.windows(2) {
            let p1 = &window[0];
            let p2 = &window[1];

            interpolated.push(p1.clone());

            // Add midpoint
            let mid_timestamp = (p1.timestamp + p2.timestamp) / 2;
            let mid_price = (p1.price + p2.price) / 2.0;
            interpolated.push(PricePoint {
                timestamp: mid_timestamp,
                price: mid_price,
            });
        }

        if let Some(last) = history.last() {
            interpolated.push(last.clone());
        }

        *history = interpolated;
    }

    /// Gets the 7-day volume by extrapolating from 24h data.
    ///
    /// Note: DexScreener doesn't provide 7d volume directly,
    /// so we estimate based on 24h volume.
    pub fn estimate_7d_volume(volume_24h: f64) -> f64 {
        // Simple estimation: assume consistent daily volume
        volume_24h * 7.0
    }
}

impl Default for DexClient {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// DexDataSource Trait Implementation
// ============================================================================

#[async_trait]
impl DexDataSource for DexClient {
    async fn get_token_price(&self, chain: &str, address: &str) -> Option<f64> {
        self.get_token_price(chain, address).await
    }

    async fn get_native_token_price(&self, chain: &str) -> Option<f64> {
        self.get_native_token_price(chain).await
    }

    async fn get_token_data(&self, chain: &str, address: &str) -> Result<DexTokenData> {
        self.get_token_data(chain, address).await
    }

    async fn search_tokens(
        &self,
        query: &str,
        chain: Option<&str>,
    ) -> Result<Vec<TokenSearchResult>> {
        self.search_tokens(query, chain).await
    }
}

/// Builds a full DexScreener token response JSON string for testing.
#[cfg(test)]
fn build_test_pair_json(chain_id: &str, base_symbol: &str, base_addr: &str, price: &str) -> String {
    format!(
        r#"{{
        "chainId":"{}","dexId":"uniswap","pairAddress":"0xpair",
        "baseToken":{{"address":"{}","name":"{}","symbol":"{}"}},
        "quoteToken":{{"address":"0xquote","name":"USDC","symbol":"USDC"}},
        "priceUsd":"{}",
        "priceChange":{{"h24":5.2,"h6":2.1,"h1":0.5,"m5":0.1}},
        "volume":{{"h24":1000000,"h6":250000,"h1":50000,"m5":5000}},
        "liquidity":{{"usd":500000,"base":100,"quote":500000}},
        "fdv":10000000,"marketCap":8000000,
        "txns":{{"h24":{{"buys":100,"sells":80}},"h6":{{"buys":20,"sells":15}},"h1":{{"buys":5,"sells":3}}}},
        "pairCreatedAt":1690000000000,
        "url":"https://dexscreener.com/ethereum/0xpair"
    }}"#,
        chain_id, base_addr, base_symbol, base_symbol, price
    )
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_mapping() {
        assert_eq!(
            DexClient::map_chain_to_dexscreener("ethereum"),
            "ethereum".to_string()
        );
        assert_eq!(
            DexClient::map_chain_to_dexscreener("ETH"),
            "ethereum".to_string()
        );
        assert_eq!(
            DexClient::map_chain_to_dexscreener("bsc"),
            "bsc".to_string()
        );
        assert_eq!(
            DexClient::map_chain_to_dexscreener("BNB"),
            "bsc".to_string()
        );
        assert_eq!(
            DexClient::map_chain_to_dexscreener("polygon"),
            "polygon".to_string()
        );
        assert_eq!(
            DexClient::map_chain_to_dexscreener("solana"),
            "solana".to_string()
        );
    }

    #[test]
    fn test_estimate_7d_volume() {
        assert_eq!(DexClient::estimate_7d_volume(1_000_000.0), 7_000_000.0);
        assert_eq!(DexClient::estimate_7d_volume(0.0), 0.0);
    }

    #[test]
    fn test_generate_volume_history() {
        let now = 1700000000;
        let history = DexClient::generate_volume_history(24000.0, 6000.0, 1000.0, now);

        assert_eq!(history.len(), 24);
        assert!(history.iter().all(|v| v.volume >= 0.0));
        assert!(history.iter().all(|v| v.timestamp <= now));
    }

    #[test]
    fn test_dex_client_default() {
        let _client = DexClient::default();
        // Just verify it doesn't panic
    }

    #[test]
    fn test_interpolate_points() {
        let mut history = vec![
            PricePoint {
                timestamp: 0,
                price: 1.0,
            },
            PricePoint {
                timestamp: 100,
                price: 2.0,
            },
        ];

        DexClient::interpolate_points(&mut history, 10);

        assert!(history.len() > 2);
        // Check midpoint was added
        assert!(history.iter().any(|p| p.timestamp == 50));
    }

    // ========================================================================
    // HTTP mocking tests
    // ========================================================================

    #[tokio::test]
    async fn test_get_token_data_success() {
        let mut server = mockito::Server::new_async().await;
        let pair = build_test_pair_json("ethereum", "WETH", "0xtoken", "2500.50");
        let body = format!(r#"{{"pairs":[{}]}}"#, pair);
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/latest/dex/tokens/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&body)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let data = client.get_token_data("ethereum", "0xtoken").await.unwrap();
        assert_eq!(data.symbol, "WETH");
        assert!((data.price_usd - 2500.50).abs() < 0.01);
        assert!(data.volume_24h > 0.0);
        assert!(data.liquidity_usd > 0.0);
        assert_eq!(data.pairs.len(), 1);
        assert!(data.total_buys_24h > 0);
        assert!(data.total_sells_24h > 0);
        assert!(!data.price_history.is_empty());
        assert!(!data.volume_history.is_empty());
    }

    #[tokio::test]
    async fn test_get_token_data_no_pairs() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/latest/dex/tokens/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"pairs":[]}"#)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let result = client.get_token_data("ethereum", "0xunknown").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No DEX pairs"));
    }

    #[tokio::test]
    async fn test_get_token_data_api_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/latest/dex/tokens/.*".to_string()),
            )
            .with_status(500)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let result = client.get_token_data("ethereum", "0xtoken").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_token_data_fallback_to_all_pairs() {
        // When no chain-specific pairs found, should use all pairs
        let mut server = mockito::Server::new_async().await;
        let pair = build_test_pair_json("bsc", "TOKEN", "0xtoken", "1.00");
        let body = format!(r#"{{"pairs":[{}]}}"#, pair);
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/latest/dex/tokens/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&body)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        // Request for ethereum but pair is on bsc → should still get data
        let data = client.get_token_data("ethereum", "0xtoken").await.unwrap();
        assert_eq!(data.symbol, "TOKEN");
    }

    #[tokio::test]
    async fn test_get_token_data_multiple_pairs() {
        let mut server = mockito::Server::new_async().await;
        let pair1 = build_test_pair_json("ethereum", "WETH", "0xtoken", "2500.00");
        let pair2 = build_test_pair_json("ethereum", "WETH", "0xtoken", "2501.00");
        let body = format!(r#"{{"pairs":[{},{}]}}"#, pair1, pair2);
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/latest/dex/tokens/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&body)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let data = client.get_token_data("ethereum", "0xtoken").await.unwrap();
        assert_eq!(data.pairs.len(), 2);
        // Price should be liquidity-weighted average
        assert!(data.price_usd > 2499.0 && data.price_usd < 2502.0);
    }

    #[tokio::test]
    async fn test_get_token_price() {
        let mut server = mockito::Server::new_async().await;
        let pair = build_test_pair_json("ethereum", "WETH", "0xtoken", "2500.50");
        let body = format!(r#"{{"pairs":[{}]}}"#, pair);
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/latest/dex/tokens/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&body)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let price = client.get_token_price("ethereum", "0xtoken").await;
        assert!(price.is_some());
        assert!((price.unwrap() - 2500.50).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_get_token_price_not_found() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/latest/dex/tokens/.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"pairs":null}"#)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let price = client.get_token_price("ethereum", "0xunknown").await;
        assert!(price.is_none());
    }

    #[tokio::test]
    async fn test_search_tokens_success() {
        let mut server = mockito::Server::new_async().await;
        let pair = build_test_pair_json("ethereum", "USDC", "0xusdc", "1.00");
        let body = format!(r#"{{"pairs":[{}]}}"#, pair);
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/latest/dex/search.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&body)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let results = client.search_tokens("USDC", None).await.unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].symbol, "USDC");
    }

    #[tokio::test]
    async fn test_search_tokens_with_chain_filter() {
        let mut server = mockito::Server::new_async().await;
        let pair_eth = build_test_pair_json("ethereum", "USDC", "0xusdc_eth", "1.00");
        let pair_bsc = build_test_pair_json("bsc", "USDC", "0xusdc_bsc", "1.00");
        let body = format!(r#"{{"pairs":[{},{}]}}"#, pair_eth, pair_bsc);
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/latest/dex/search.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&body)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let results = client
            .search_tokens("USDC", Some("ethereum"))
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chain, "ethereum");
    }

    #[tokio::test]
    async fn test_search_tokens_empty() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/latest/dex/search.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"pairs":[]}"#)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let results = client.search_tokens("XYZNONEXIST", None).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_search_tokens_api_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/latest/dex/search.*".to_string()),
            )
            .with_status(429)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let result = client.search_tokens("USDC", None).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_generate_price_history() {
        let pair_json = r#"{
            "chainId":"ethereum","dexId":"uniswap","pairAddress":"0xpair",
            "baseToken":{"address":"0xtoken","name":"Token","symbol":"TKN"},
            "quoteToken":{"address":"0xquote","name":"USDC","symbol":"USDC"},
            "priceUsd":"100.0",
            "priceChange":{"h24":10.0,"h6":5.0,"h1":1.0,"m5":0.5}
        }"#;
        let pair: DexScreenerPair = serde_json::from_str(pair_json).unwrap();
        let history = DexClient::generate_price_history(100.0, &pair, 1700000000);
        assert!(!history.is_empty());
        // Last point should be current price
        assert!(history.iter().any(|p| (p.price - 100.0).abs() < 0.001));
    }

    #[test]
    fn test_chain_mapping_all_variants() {
        // Test all known chains
        assert_eq!(DexClient::map_chain_to_dexscreener("eth"), "ethereum");
        assert_eq!(DexClient::map_chain_to_dexscreener("matic"), "polygon");
        assert_eq!(DexClient::map_chain_to_dexscreener("arb"), "arbitrum");
        assert_eq!(DexClient::map_chain_to_dexscreener("op"), "optimism");
        assert_eq!(DexClient::map_chain_to_dexscreener("base"), "base");
        assert_eq!(DexClient::map_chain_to_dexscreener("bnb"), "bsc");
        assert_eq!(DexClient::map_chain_to_dexscreener("sol"), "solana");
        assert_eq!(DexClient::map_chain_to_dexscreener("avax"), "avalanche");
        assert_eq!(DexClient::map_chain_to_dexscreener("unknown"), "unknown");
    }

    #[tokio::test]
    async fn test_get_native_token_price_ethereum() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"pairs":[{
                "chainId":"ethereum",
                "dexId":"uniswap",
                "pairAddress":"0xpair",
                "baseToken":{"address":"0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2","name":"WETH","symbol":"WETH"},
                "quoteToken":{"address":"0xusdt","name":"USDT","symbol":"USDT"},
                "priceUsd":"3500.00"
            }]}"#)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let price = client.get_native_token_price("ethereum").await;
        assert!(price.is_some());
        assert!((price.unwrap() - 3500.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_get_native_token_price_tron_returns_none() {
        let client = DexClient::with_base_url("http://localhost:1");
        let price = client.get_native_token_price("tron").await;
        assert!(price.is_none());
    }

    #[tokio::test]
    async fn test_get_native_token_price_unknown_chain() {
        let client = DexClient::with_base_url("http://localhost:1");
        let price = client.get_native_token_price("unknownchain").await;
        assert!(price.is_none());
    }

    #[tokio::test]
    async fn test_search_tokens_chain_filter_ethereum_only() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"pairs":[
                {
                    "chainId":"ethereum",
                    "dexId":"uniswap",
                    "pairAddress":"0xpair1",
                    "baseToken":{"address":"0xtoken1","name":"USD Coin","symbol":"USDC"},
                    "quoteToken":{"address":"0xweth","name":"WETH","symbol":"WETH"},
                    "priceUsd":"1.00",
                    "liquidity":{"usd":5000000.0},
                    "volume":{"h24":1000000.0}
                },
                {
                    "chainId":"bsc",
                    "dexId":"pancakeswap",
                    "pairAddress":"0xpair2",
                    "baseToken":{"address":"0xtoken2","name":"Binance USD","symbol":"BUSD"},
                    "quoteToken":{"address":"0xbnb","name":"BNB","symbol":"BNB"},
                    "priceUsd":"1.00",
                    "liquidity":{"usd":2000000.0},
                    "volume":{"h24":500000.0}
                }
            ]}"#,
            )
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        // Filter to ethereum only
        let results = client.search_tokens("USD", Some("ethereum")).await.unwrap();
        assert!(!results.is_empty());
        // All results should be on ethereum
        for r in &results {
            assert_eq!(r.chain.to_lowercase(), "ethereum");
        }
    }

    #[tokio::test]
    async fn test_search_tokens_aggregates_volume_and_liquidity() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"pairs":[
                {
                    "chainId":"ethereum",
                    "dexId":"uniswap",
                    "pairAddress":"0xpair1",
                    "baseToken":{"address":"0xSameToken","name":"Test Token","symbol":"TEST"},
                    "quoteToken":{"address":"0xweth","name":"WETH","symbol":"WETH"},
                    "priceUsd":"10.00",
                    "liquidity":{"usd":1000000.0},
                    "volume":{"h24":100000.0}
                },
                {
                    "chainId":"ethereum",
                    "dexId":"sushiswap",
                    "pairAddress":"0xpair2",
                    "baseToken":{"address":"0xSameToken","name":"Test Token","symbol":"TEST"},
                    "quoteToken":{"address":"0xusdc","name":"USDC","symbol":"USDC"},
                    "priceUsd":"10.05",
                    "liquidity":{"usd":500000.0},
                    "volume":{"h24":50000.0}
                }
            ]}"#,
            )
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let results = client.search_tokens("TEST", None).await.unwrap();
        assert_eq!(results.len(), 1); // Same token aggregated
        // Volume and liquidity should be summed
        assert!(results[0].volume_24h > 100000.0);
        assert!(results[0].liquidity_usd > 1000000.0);
    }

    #[tokio::test]
    async fn test_dex_data_source_trait_methods() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"pairs":[{
                "chainId":"ethereum",
                "dexId":"uniswap",
                "pairAddress":"0xpair",
                "baseToken":{"address":"0xtoken","name":"Token","symbol":"TKN"},
                "quoteToken":{"address":"0xquote","name":"USDC","symbol":"USDC"},
                "priceUsd":"50.0",
                "liquidity":{"usd":1000000.0},
                "volume":{"h24":100000.0}
            }]}"#,
            )
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        // Test through DexDataSource trait
        let trait_client: &dyn DexDataSource = &client;
        let price = trait_client.get_token_price("ethereum", "0xtoken").await;
        assert!(price.is_some());
    }

    #[tokio::test]
    async fn test_dex_data_source_trait_get_native_token_price() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"pairs":[{
                "chainId":"ethereum",
                "dexId":"uniswap",
                "pairAddress":"0xpair",
                "baseToken":{"address":"0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2","name":"WETH","symbol":"WETH"},
                "quoteToken":{"address":"0xquote","name":"USDC","symbol":"USDC"},
                "priceUsd":"3500.0",
                "liquidity":{"usd":10000000.0},
                "volume":{"h24":5000000.0}
            }]}"#,
            )
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let trait_client: &dyn DexDataSource = &client;
        let price = trait_client.get_native_token_price("ethereum").await;
        assert!(price.is_some());
    }

    #[tokio::test]
    async fn test_dex_data_source_trait_get_token_data() {
        let mut server = mockito::Server::new_async().await;
        let pair_json = build_test_pair_json("ethereum", "TKN", "0xtoken", "50.0");
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(r#"{{"pairs":[{}]}}"#, pair_json))
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let trait_client: &dyn DexDataSource = &client;
        let data = trait_client.get_token_data("ethereum", "0xtoken").await;
        assert!(data.is_ok());
    }

    #[tokio::test]
    async fn test_dex_data_source_trait_search_tokens() {
        let mut server = mockito::Server::new_async().await;
        let pair_json = build_test_pair_json("ethereum", "TKN", "0xtoken", "50.0");
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(r#"{{"pairs":[{}]}}"#, pair_json))
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let trait_client: &dyn DexDataSource = &client;
        let results = trait_client.search_tokens("TKN", None).await;
        assert!(results.is_ok());
    }

    #[tokio::test]
    async fn test_get_token_data_quote_token() {
        let mut server = mockito::Server::new_async().await;
        // Token is the quote token, not the base
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"pairs":[{
                "chainId":"ethereum","dexId":"uniswap","pairAddress":"0xpair",
                "baseToken":{"address":"0xother","name":"Other","symbol":"OTH"},
                "quoteToken":{"address":"0xmytoken","name":"MyToken","symbol":"MTK"},
                "priceUsd":"25.0",
                "priceChange":{"h24":1.0,"h6":0.5,"h1":0.2,"m5":0.05},
                "volume":{"h24":500000,"h6":100000,"h1":20000,"m5":2000},
                "liquidity":{"usd":0,"base":0,"quote":0},
                "txns":{"h24":{"buys":50,"sells":40},"h6":{"buys":10,"sells":8},"h1":{"buys":2,"sells":1}},
                "pairCreatedAt":1690000000000,
                "url":"https://dexscreener.com/ethereum/0xpair"
            }]}"#,
            )
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let data = client
            .get_token_data("ethereum", "0xmytoken")
            .await
            .unwrap();
        // Should identify the quote token
        assert_eq!(data.symbol, "MTK");
        assert_eq!(data.name, "MyToken");
        // Zero liquidity fallback for price: should use priceUsd from first pair
        assert!(data.price_usd > 0.0);
    }

    #[tokio::test]
    async fn test_get_token_data_with_socials() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"pairs":[{
                "chainId":"ethereum","dexId":"uniswap","pairAddress":"0xpair",
                "baseToken":{"address":"0xtoken","name":"Token","symbol":"TKN"},
                "quoteToken":{"address":"0xquote","name":"USDC","symbol":"USDC"},
                "priceUsd":"50.0",
                "priceChange":{"h24":5.0,"h6":2.0,"h1":1.0,"m5":0.1},
                "volume":{"h24":1000000,"h6":250000,"h1":50000,"m5":5000},
                "liquidity":{"usd":1000000,"base":100,"quote":1000000},
                "txns":{"h24":{"buys":100,"sells":80},"h6":{"buys":20,"sells":15},"h1":{"buys":5,"sells":3}},
                "pairCreatedAt":1690000000000,
                "url":"https://dexscreener.com/ethereum/0xpair",
                "info":{
                    "imageUrl":"https://example.com/logo.png",
                    "websites":[{"url":"https://example.com"}],
                    "socials":[
                        {"type":"twitter","url":"https://twitter.com/token"},
                        {"type":"telegram","url":"https://t.me/token"}
                    ]
                }
            }]}"#,
            )
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let data = client.get_token_data("ethereum", "0xtoken").await.unwrap();
        assert_eq!(data.symbol, "TKN");
        assert!(data.image_url.is_some());
        assert!(!data.websites.is_empty());
        assert!(!data.socials.is_empty());
        assert_eq!(data.socials[0].platform, "twitter");
    }

    #[tokio::test]
    async fn test_search_tokens_quote_match_and_updates() {
        let mut server = mockito::Server::new_async().await;
        // Token matches as quote, not base
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"pairs":[
                {
                    "chainId":"ethereum","dexId":"uniswap","pairAddress":"0xpair1",
                    "baseToken":{"address":"0xother","name":"Other","symbol":"OTH"},
                    "quoteToken":{"address":"0xmytk","name":"MySearch","symbol":"MSR"},
                    "liquidity":{"usd":500000.0},
                    "volume":{"h24":100000.0},
                    "marketCap":5000000
                },
                {
                    "chainId":"ethereum","dexId":"sushi","pairAddress":"0xpair2",
                    "baseToken":{"address":"0xmytk","name":"MySearch","symbol":"MSR"},
                    "quoteToken":{"address":"0xweth","name":"WETH","symbol":"WETH"},
                    "priceUsd":"10.5",
                    "liquidity":{"usd":800000.0},
                    "volume":{"h24":200000.0}
                }
            ]}"#,
            )
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let results = client.search_tokens("MySearch", None).await.unwrap();
        assert_eq!(results.len(), 1); // Same token aggregated
        assert_eq!(results[0].symbol, "MSR");
        // Volume should be aggregated
        assert!(results[0].volume_24h >= 300000.0);
        // Liquidity should be aggregated
        assert!(results[0].liquidity_usd >= 1300000.0);
        // Price should be set from the second pair
        assert!(results[0].price_usd.is_some());
        // Market cap should be carried from first pair
        assert!(results[0].market_cap.is_some());
    }

    #[test]
    fn test_interpolate_points_midpoint() {
        let mut history = vec![
            PricePoint {
                timestamp: 1000,
                price: 10.0,
            },
            PricePoint {
                timestamp: 2000,
                price: 20.0,
            },
        ];
        // Should not interpolate if already enough points
        DexClient::interpolate_points(&mut history, 2);
        assert_eq!(history.len(), 2);

        // Should add midpoints
        DexClient::interpolate_points(&mut history, 5);
        assert!(history.len() > 2);
        // Check that a midpoint was added
        let midpoints: Vec<_> = history.iter().filter(|p| p.timestamp == 1500).collect();
        assert!(!midpoints.is_empty());
        assert!((midpoints[0].price - 15.0).abs() < 0.01);
    }

    fn discover_token_json() -> &'static str {
        r#"[
            {"chainId":"ethereum","tokenAddress":"0xabc","url":"https://dexscreener.com/ethereum/0xabc","description":"Test token","links":[{"label":"Twitter","type":"twitter","url":"https://twitter.com/test"}]},
            {"chainId":"solana","tokenAddress":"So11111111111111111111111111111111111111112","url":"https://dexscreener.com/solana/So11","links":[]}
        ]"#
    }

    #[tokio::test]
    async fn test_get_token_profiles() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/token-profiles/latest/v1")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(discover_token_json())
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let tokens = client.get_token_profiles().await.unwrap();
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].chain_id, "ethereum");
        assert_eq!(tokens[0].token_address, "0xabc");
        assert_eq!(tokens[0].description.as_deref(), Some("Test token"));
        assert_eq!(tokens[0].links.len(), 1);
        assert_eq!(tokens[1].chain_id, "solana");
    }

    #[tokio::test]
    async fn test_get_token_boosts() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/token-boosts/latest/v1")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(discover_token_json())
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let tokens = client.get_token_boosts().await.unwrap();
        assert_eq!(tokens.len(), 2);
    }

    #[tokio::test]
    async fn test_get_token_boosts_top() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/token-boosts/top/v1")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(discover_token_json())
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let tokens = client.get_token_boosts_top().await.unwrap();
        assert_eq!(tokens.len(), 2);
    }

    #[tokio::test]
    async fn test_fetch_discover_tokens_api_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(500)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let result = client.get_token_profiles().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_discover_tokens_empty_array() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/token-profiles/latest/v1")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("[]")
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let tokens = client.get_token_profiles().await.unwrap();
        assert!(tokens.is_empty());
    }

    #[tokio::test]
    async fn test_fetch_discover_tokens_filters_invalid_entries() {
        // Entries without tokenAddress are filtered out
        let body = r#"[{"chainId":"ethereum","url":"https://example.com"},{"chainId":"solana","tokenAddress":"0xvalid","url":"https://dexscreener.com/solana/0xvalid"}]"#;
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/token-profiles/latest/v1")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let client = DexClient::with_base_url(&server.url());
        let tokens = client.get_token_profiles().await.unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].token_address, "0xvalid");
    }
}
