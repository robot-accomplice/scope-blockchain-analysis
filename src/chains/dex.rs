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
//!
//! ## Usage
//!
//! ```rust,no_run
//! use bcc::chains::DexClient;
//!
//! #[tokio::main]
//! async fn main() -> bcc::Result<()> {
//!     let client = DexClient::new();
//!     
//!     // Fetch token data by address
//!     let data = client.get_token_data("ethereum", "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").await?;
//!     println!("Price: ${}", data.price_usd);
//!     
//!     Ok(())
//! }
//! ```

use crate::chains::{DexPair, PricePoint, VolumePoint};
use crate::error::{BccError, Result};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

/// DexScreener API base URL.
const DEXSCREENER_API_BASE: &str = "https://api.dexscreener.com";

/// Client for fetching DEX aggregator data.
#[derive(Debug, Clone)]
pub struct DexClient {
    http: Client,
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

        Self { http }
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
        let url = format!(
            "{}/latest/dex/tokens/{}",
            DEXSCREENER_API_BASE, token_address
        );

        tracing::debug!(url = %url, "Fetching token data from DexScreener");

        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| BccError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(BccError::Api(format!(
                "DexScreener API error: {}",
                response.status()
            )));
        }

        let data: DexScreenerTokenResponse = response
            .json()
            .await
            .map_err(|e| BccError::Api(format!("Failed to parse DexScreener response: {}", e)))?;

        let pairs = data.pairs.unwrap_or_default();

        if pairs.is_empty() {
            return Err(BccError::NotFound(format!(
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
            DEXSCREENER_API_BASE,
            urlencoding::encode(query)
        );

        tracing::debug!(url = %url, "Searching tokens on DexScreener");

        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| BccError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(BccError::Api(format!(
                "DexScreener search API error: {}",
                response.status()
            )));
        }

        let data: DexScreenerSearchResponse = response
            .json()
            .await
            .map_err(|e| BccError::Api(format!("Failed to parse search response: {}", e)))?;

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
}
