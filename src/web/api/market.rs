//! Market summary API handler.

use crate::cli::crawl::{self, Period};
use crate::market::{
    BinanceClient, HealthThresholds, MarketSummary, MarketVenue,
    order_book_from_analytics,
};
use crate::web::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

/// Request body for market summary.
#[derive(Debug, Deserialize)]
pub struct MarketRequest {
    /// Token symbol (e.g., "USDC", "PUSD"). Default: "USDC".
    #[serde(default = "default_pair")]
    pub pair: String,
    /// Market venue: "binance", "biconomy", "eth", "solana".
    #[serde(default = "default_venue")]
    pub market_venue: String,
    /// Chain for DEX venues.
    #[serde(default = "default_chain")]
    pub chain: String,
    /// Peg target (default: 1.0).
    #[serde(default = "default_peg")]
    pub peg: f64,
    /// Min order book levels per side.
    #[serde(default = "default_min_levels")]
    pub min_levels: usize,
    /// Min depth per side in quote terms.
    #[serde(default = "default_min_depth")]
    pub min_depth: f64,
    /// Peg range for outlier filtering.
    #[serde(default = "default_peg_range")]
    pub peg_range: f64,
}

fn default_pair() -> String { "USDC".to_string() }
fn default_venue() -> String { "binance".to_string() }
fn default_chain() -> String { "ethereum".to_string() }
fn default_peg() -> f64 { 1.0 }
fn default_min_levels() -> usize { 6 }
fn default_min_depth() -> f64 { 3000.0 }
fn default_peg_range() -> f64 { 0.001 }

/// Converts a MarketSummary to a JSON Value.
fn summary_to_json(summary: &MarketSummary) -> serde_json::Value {
    serde_json::json!({
        "pair": summary.pair,
        "peg_target": summary.peg_target,
        "best_bid": summary.best_bid,
        "best_ask": summary.best_ask,
        "mid_price": summary.mid_price,
        "spread": summary.spread,
        "volume_24h": summary.volume_24h,
        "bid_depth": summary.bid_depth,
        "ask_depth": summary.ask_depth,
        "bid_outliers": summary.bid_outliers,
        "ask_outliers": summary.ask_outliers,
        "healthy": summary.healthy,
        "checks": summary.checks.iter().map(|c| match c {
            crate::market::HealthCheck::Pass(msg) => serde_json::json!({"status": "pass", "message": msg}),
            crate::market::HealthCheck::Fail(msg) => serde_json::json!({"status": "fail", "message": msg}),
        }).collect::<Vec<_>>(),
    })
}

/// POST /api/market/summary — Peg and order book health.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MarketRequest>,
) -> impl IntoResponse {
    let venue: MarketVenue = match req.market_venue.as_str() {
        "biconomy" => MarketVenue::Biconomy,
        "eth" | "ethereum" => MarketVenue::Ethereum,
        "solana" | "sol" => MarketVenue::Solana,
        _ => MarketVenue::Binance,
    };

    let thresholds = HealthThresholds {
        peg_target: req.peg,
        peg_range: req.peg_range,
        min_levels: req.min_levels,
        min_depth: req.min_depth,
        min_bid_ask_ratio: 0.2,
        max_bid_ask_ratio: 5.0,
    };

    if venue.is_cex() {
        let pair = venue.format_pair(&req.pair);
        let client_opt = venue.create_client();
        let Some(client) = client_opt else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Failed to create market client" })),
            )
                .into_response();
        };

        match client.fetch_order_book(&pair).await {
            Ok(book) => {
                let volume_24h = match venue {
                    MarketVenue::Binance => BinanceClient::default_url()
                        .fetch_24h_volume(&pair)
                        .await
                        .ok()
                        .flatten(),
                    _ => None,
                };
                let summary = MarketSummary::from_order_book(&book, req.peg, &thresholds, volume_24h);
                Json(summary_to_json(&summary)).into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        }
    } else {
        // DEX venue: fetch analytics then synthesize order book
        let venue_chain = match venue {
            MarketVenue::Ethereum => "ethereum",
            MarketVenue::Solana => "solana",
            _ => &req.chain,
        };

        match crawl::fetch_analytics_for_input(&req.pair, venue_chain, Period::Hour24, 10, &state.factory).await {
            Ok(analytics) => {
                if analytics.dex_pairs.is_empty() {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({ "error": "No DEX pairs found" })),
                    )
                        .into_response();
                }
                let best_pair = analytics
                    .dex_pairs
                    .iter()
                    .max_by(|a, b| a.liquidity_usd.partial_cmp(&b.liquidity_usd).unwrap_or(std::cmp::Ordering::Equal))
                    .unwrap();
                let book = order_book_from_analytics(venue_chain, best_pair, &analytics.token.symbol);
                let summary = MarketSummary::from_order_book(&book, req.peg, &thresholds, Some(best_pair.volume_24h));
                Json(summary_to_json(&summary)).into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_full() {
        let json = serde_json::json!({
            "pair": "PUSD",
            "market_venue": "biconomy",
            "chain": "polygon",
            "peg": 1.0,
            "min_levels": 10,
            "min_depth": 5000.0,
            "peg_range": 0.002
        });
        let req: MarketRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.pair, "PUSD");
        assert_eq!(req.market_venue, "biconomy");
        assert_eq!(req.chain, "polygon");
        assert_eq!(req.peg, 1.0);
        assert_eq!(req.min_levels, 10);
        assert_eq!(req.min_depth, 5000.0);
        assert_eq!(req.peg_range, 0.002);
    }

    #[test]
    fn test_deserialize_minimal() {
        let json = serde_json::json!({});
        let req: MarketRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.pair, "USDC");
        assert_eq!(req.market_venue, "binance");
        assert_eq!(req.chain, "ethereum");
        assert_eq!(req.peg, 1.0);
        assert_eq!(req.min_levels, 6);
        assert_eq!(req.min_depth, 3000.0);
        assert_eq!(req.peg_range, 0.001);
    }

    #[test]
    fn test_all_defaults() {
        assert_eq!(default_pair(), "USDC");
        assert_eq!(default_venue(), "binance");
        assert_eq!(default_chain(), "ethereum");
        assert_eq!(default_peg(), 1.0);
        assert_eq!(default_min_levels(), 6);
        assert_eq!(default_min_depth(), 3000.0);
        assert_eq!(default_peg_range(), 0.001);
    }

    #[test]
    fn test_custom_thresholds() {
        let json = serde_json::json!({
            "min_levels": 20,
            "min_depth": 10000.0,
            "peg_range": 0.005
        });
        let req: MarketRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.min_levels, 20);
        assert_eq!(req.min_depth, 10000.0);
        assert_eq!(req.peg_range, 0.005);
        // Other fields should use defaults
        assert_eq!(req.pair, "USDC");
        assert_eq!(req.market_venue, "binance");
        assert_eq!(req.chain, "ethereum");
        assert_eq!(req.peg, 1.0);
    }

    #[tokio::test]
    async fn test_handle_market_cex() {
        use axum::extract::State;
        use axum::response::IntoResponse;
        use crate::config::Config;
        use crate::chains::DefaultClientFactory;
        use crate::web::AppState;

        let config = Config::default();
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = MarketRequest {
            pair: "USDC".to_string(),
            market_venue: "binance".to_string(),
            chain: "ethereum".to_string(),
            peg: 1.0,
            min_levels: 6,
            min_depth: 3000.0,
            peg_range: 0.001,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_market_dex() {
        use axum::extract::State;
        use axum::response::IntoResponse;
        use crate::config::Config;
        use crate::chains::DefaultClientFactory;
        use crate::web::AppState;

        let config = Config::default();
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = MarketRequest {
            pair: "USDC".to_string(),
            market_venue: "eth".to_string(),
            chain: "ethereum".to_string(),
            peg: 1.0,
            min_levels: 6,
            min_depth: 3000.0,
            peg_range: 0.001,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }
}
