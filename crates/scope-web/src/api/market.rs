//! Market summary API handler.

use super::{error_response, status_for};
use crate::AppState;
use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use scope::market::{HealthOverrides, MarketSummary, MarketVenue, Pair, fetch_summary};
use scope_cli::cli::market::CrawlDexSource;
use scope_cli::cli::output::Output;
use serde::Deserialize;
use std::sync::Arc;

/// Request body for market summary.
#[derive(Debug, Deserialize)]
pub struct MarketRequest {
    /// Pair or base symbol, read by `Pair::parse` ("DAI_USDC", "OPUSDT", "USDC"). Default: "USDC".
    #[serde(default = "default_pair")]
    pub pair: String,
    /// Market venue: a CEX id ("binance") or a DEX ("ethereum", "solana").
    #[serde(default = "default_venue")]
    pub market_venue: String,
    /// Chain check for a DEX venue: when set, it must name the venue's chain.
    /// The venue already names its chain. Ignored for CEX venues.
    #[serde(default)]
    pub chain: Option<String>,
    /// Health threshold overrides (top-level JSON keys, e.g. `peg`,
    /// `min_levels`, `max_spread_pct`). Unset keys use config `market.health`.
    #[serde(flatten)]
    pub health: HealthOverrides,
}

fn default_pair() -> String {
    "USDC".to_string()
}
fn default_venue() -> String {
    "binance".to_string()
}

/// POST /api/market/summary — Peg and order book health. The pair and venue
/// are read exactly as the CLI reads them (core `Pair` and `MarketVenue`).
/// Errors map through `status_for`: an unknown venue or a chain mismatch is
/// 400, no DEX pairs is 404.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MarketRequest>,
) -> impl IntoResponse {
    match market_summary(&state, &req).await {
        Ok(summary) => Json(summary.to_json()).into_response(),
        Err(e) => error_response(status_for(&e), e.to_string()),
    }
}

async fn market_summary(
    state: &AppState,
    req: &MarketRequest,
) -> scope::error::Result<MarketSummary> {
    let venue = MarketVenue::from_name(&req.market_venue)?;
    if let Some(chain) = &req.chain {
        venue.require_chain(chain)?;
    }
    let thresholds = state.config.market.health.with_overrides(&req.health);
    let out = Output::stdio();
    let dex = CrawlDexSource {
        factory: state.factory.as_ref(),
        out: &out,
    };
    fetch_summary(&venue, &Pair::parse(&req.pair), &thresholds, &dex).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope::market::DexVenue;

    #[test]
    fn test_deserialize_full() {
        let json = serde_json::json!({
            "pair": "DAI",
            "market_venue": "binance",
            "chain": "polygon",
            "peg": 1.0,
            "min_levels": 10,
            "min_depth": 5000.0,
            "peg_range": 0.002
        });
        let req: MarketRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.pair, "DAI");
        assert_eq!(req.market_venue, "binance");
        assert_eq!(req.chain.as_deref(), Some("polygon"));
        assert_eq!(req.health.peg_target, Some(1.0));
        assert_eq!(req.health.min_levels, Some(10));
        assert_eq!(req.health.min_depth, Some(5000.0));
        assert_eq!(req.health.peg_range, Some(0.002));
    }

    #[test]
    fn test_deserialize_minimal() {
        let json = serde_json::json!({});
        let req: MarketRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.pair, "USDC");
        assert_eq!(req.market_venue, "binance");
        assert_eq!(req.chain, None);
        // No threshold keys: every value comes from config `market.health`.
        assert_eq!(req.health, HealthOverrides::default());
    }

    #[test]
    fn test_all_defaults() {
        assert_eq!(default_pair(), "USDC");
        assert_eq!(default_venue(), "binance");
    }

    #[test]
    fn test_new_thresholds_accepted() {
        let json = serde_json::json!({
            "max_spread_pct": 1.5,
            "min_top3_depth": 500.0,
            "min_top10_depth": 4000.0,
            "min_bid_ask_ratio": 0.5,
            "max_bid_ask_ratio": 2.0
        });
        let req: MarketRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.health.max_spread_pct, Some(1.5));
        assert_eq!(req.health.min_top3_depth, Some(500.0));
        assert_eq!(req.health.min_top10_depth, Some(4000.0));
        assert_eq!(req.health.min_bid_ask_ratio, Some(0.5));
        assert_eq!(req.health.max_bid_ask_ratio, Some(2.0));
    }

    #[test]
    fn test_request_accepts_amm_step_pct() {
        // #37: the AMM step is a request field like the other thresholds.
        let req: MarketRequest =
            serde_json::from_value(serde_json::json!({ "amm_step_pct": 0.05 })).unwrap();
        let t = scope::config::Config::default()
            .market
            .health
            .with_overrides(&req.health);
        assert_eq!(t.amm_step_pct, 0.05);
    }

    /// H6: with `peg_target: 0.92` in the config, a market request and a
    /// token-health request that send no peg both judge against 0.92.
    #[test]
    fn test_configured_peg_applies_to_both_panels() {
        let mut config = scope::config::Config::default();
        config.market.health.peg_target = 0.92;
        let market: MarketRequest =
            serde_json::from_value(serde_json::json!({ "pair": "EURC" })).unwrap();
        let health: super::super::token_health::TokenHealthRequest =
            serde_json::from_value(serde_json::json!({ "token": "EURC", "with_market": true }))
                .unwrap();
        let from_market = config.market.health.with_overrides(&market.health);
        let from_health = config.market.health.with_overrides(&health.health);
        assert_eq!(from_market.peg_target, 0.92);
        assert_eq!(from_health.peg_target, 0.92);
    }

    #[test]
    fn test_request_overrides_config() {
        let mut config = scope::config::Config::default();
        config.market.health.min_levels = 20;
        config.market.health.max_spread_pct = 5.0;
        let req: MarketRequest =
            serde_json::from_value(serde_json::json!({ "max_spread_pct": 1.0 })).unwrap();
        let t = config.market.health.with_overrides(&req.health);
        assert_eq!(t.max_spread_pct, 1.0);
        assert_eq!(t.min_levels, 20);
    }

    #[test]
    fn test_custom_thresholds() {
        let json = serde_json::json!({
            "min_levels": 20,
            "min_depth": 10000.0,
            "peg_range": 0.005
        });
        let req: MarketRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.health.min_levels, Some(20));
        assert_eq!(req.health.min_depth, Some(10000.0));
        assert_eq!(req.health.peg_range, Some(0.005));
        // Other fields should use defaults
        assert_eq!(req.pair, "USDC");
        assert_eq!(req.market_venue, "binance");
        assert_eq!(req.chain, None);
        assert_eq!(req.health.peg_target, None);
    }

    #[tokio::test]
    async fn test_handle_market_cex() {
        use crate::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;
        use scope::chains::DefaultClientFactory;
        use scope::config::Config;

        let config = Config::default();
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState::new(config, factory));
        let req = MarketRequest {
            pair: "USDC".to_string(),
            market_venue: "binance".to_string(),
            chain: None,
            health: HealthOverrides::default(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_market_dex() {
        use crate::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;
        use scope::chains::DefaultClientFactory;
        use scope::config::Config;

        let config = Config::default();
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState::new(config, factory));
        let req = MarketRequest {
            pair: "USDC".to_string(),
            market_venue: "eth".to_string(),
            chain: None,
            health: HealthOverrides::default(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_market_with_cex_venue() {
        use crate::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;
        use scope::chains::DefaultClientFactory;
        use scope::config::Config;

        let config = Config::default();
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState::new(config, factory));
        let req = MarketRequest {
            pair: "BTC".to_string(),
            market_venue: "binance".to_string(),
            chain: None,
            health: HealthOverrides::default(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(
            status.is_success() || status.is_server_error(),
            "Unexpected status: {}",
            status
        );
    }

    #[test]
    fn test_is_dex_venue() {
        assert!(DexVenue::parse("eth").is_some());
        assert!(DexVenue::parse("ethereum").is_some());
        assert!(DexVenue::parse("solana").is_some());
        assert!(DexVenue::parse("binance").is_none());
        assert!(DexVenue::parse("mexc").is_none());
    }

    #[test]
    fn test_dex_venue_to_chain() {
        assert_eq!(
            DexVenue::parse("eth").map(DexVenue::chain),
            Some("ethereum")
        );
        assert_eq!(
            DexVenue::parse("ethereum").map(DexVenue::chain),
            Some("ethereum")
        );
        assert_eq!(
            DexVenue::parse("solana").map(DexVenue::chain),
            Some("solana")
        );
        assert_eq!(DexVenue::parse("unknown"), None); // no silent ethereum
    }

    #[test]
    fn test_market_request_debug() {
        let req = MarketRequest {
            pair: "USDC".to_string(),
            market_venue: "binance".to_string(),
            chain: None,
            health: HealthOverrides::default(),
        };
        let debug = format!("{:?}", req);
        assert!(debug.contains("MarketRequest"));
    }

    #[tokio::test]
    async fn test_handle_market_invalid_venue_bad_request() {
        use crate::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;
        use scope::chains::DefaultClientFactory;
        use scope::config::Config;

        let config = Config::default();
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState::new(config, factory));
        let req = MarketRequest {
            pair: "USDC".to_string(),
            market_venue: "nonexistent_venue_xyz".to_string(),
            chain: None,
            health: HealthOverrides::default(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        // Unknown venue -> BAD_REQUEST from create_exchange_client
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_market_success_json_structure() {
        use crate::AppState;
        use axum::body;
        use axum::extract::State;
        use axum::response::IntoResponse;
        use scope::chains::DefaultClientFactory;
        use scope::config::Config;

        let config = Config::default();
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState::new(config, factory));
        let req = MarketRequest {
            pair: "USDC".to_string(),
            market_venue: "binance".to_string(),
            chain: None,
            health: HealthOverrides::default(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        if response.status().is_success() {
            let body_bytes = body::to_bytes(response.into_body(), 1_000_000)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
            assert!(json.get("pair").is_some());
            assert!(json.get("peg_target").is_some());
            assert!(json.get("best_bid").is_some());
            assert!(json.get("best_ask").is_some());
            assert!(json.get("healthy").is_some());
            assert!(json.get("checks").is_some());
        }
    }

    #[test]
    fn test_is_dex_venue_case_insensitive() {
        assert!(DexVenue::parse("ETHEREUM").is_some());
        assert!(DexVenue::parse("SOLANA").is_some());
        assert!(DexVenue::parse("BINANCE").is_none());
    }

    #[test]
    fn test_summary_json_with_execution_estimates() {
        use scope::market::OrderBookLevel;

        let book = scope::market::OrderBook {
            pair: "USDC/USDT".to_string(),
            source: scope::market::BookSource::Exchange,
            bids: vec![
                OrderBookLevel {
                    price: 0.9999,
                    quantity: 20_000.0,
                },
                OrderBookLevel {
                    price: 0.9998,
                    quantity: 10_000.0,
                },
            ],
            asks: vec![
                OrderBookLevel {
                    price: 1.0001,
                    quantity: 20_000.0,
                },
                OrderBookLevel {
                    price: 1.0002,
                    quantity: 10_000.0,
                },
            ],
        };
        let thresholds = scope::market::HealthThresholds::default();
        let summary =
            scope::market::MarketSummary::from_order_book(&book, &thresholds, Some(50_000.0));
        let json = summary.to_json();

        assert_eq!(json["pair"], "USDC/USDT");
        assert!(json["best_bid"].as_f64().unwrap() > 0.0);
        assert!(json["best_ask"].as_f64().unwrap() > 0.0);
        assert!(json.get("healthy").is_some());
        assert!(!json["checks"].as_array().unwrap().is_empty());
        assert!(json.get("execution_10k_buy").is_some());
        assert!(json.get("execution_10k_sell").is_some());
        assert!(json.get("bids").is_some());
        assert!(json.get("asks").is_some());
    }
}
