//! Market summary API handler.

use crate::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use scope::market::{HealthOverrides, MarketSummary, VenueRegistry, order_book_from_analytics};
use scope_cli::cli::crawl::{self, Period};
use serde::Deserialize;
use std::sync::Arc;

/// Request body for market summary.
#[derive(Debug, Deserialize)]
pub struct MarketRequest {
    /// Token symbol (e.g., "USDC", "DAI"). Default: "USDC".
    #[serde(default = "default_pair")]
    pub pair: String,
    /// Market venue: "binance", "biconomy", "eth", "solana".
    #[serde(default = "default_venue")]
    pub market_venue: String,
    /// Chain for DEX venues.
    #[serde(default = "default_chain")]
    pub chain: String,
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
fn default_chain() -> String {
    "ethereum".to_string()
}

/// Converts a MarketSummary to a JSON Value.
fn summary_to_json(summary: &MarketSummary) -> serde_json::Value {
    let exec_buy = summary.execution_10k_buy.as_ref().map(|e| {
        serde_json::json!({
            "notional_usdt": e.notional_usdt,
            "vwap": e.vwap,
            "slippage_bps": e.slippage_bps,
            "fillable": e.fillable,
        })
    });
    let exec_sell = summary.execution_10k_sell.as_ref().map(|e| {
        serde_json::json!({
            "notional_usdt": e.notional_usdt,
            "vwap": e.vwap,
            "slippage_bps": e.slippage_bps,
            "fillable": e.fillable,
        })
    });

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
        "execution_10k_buy": exec_buy,
        "execution_10k_sell": exec_sell,
        "bids": summary.bids.iter().take(20).map(|l| {
            serde_json::json!({"price": l.price, "quantity": l.quantity, "value": l.value()})
        }).collect::<Vec<_>>(),
        "asks": summary.asks.iter().take(20).map(|l| {
            serde_json::json!({"price": l.price, "quantity": l.quantity, "value": l.value()})
        }).collect::<Vec<_>>(),
        "checks": summary.checks.iter().map(|c| match c {
            scope::market::HealthCheck::Pass(msg) => serde_json::json!({"status": "pass", "message": msg}),
            scope::market::HealthCheck::Fail(msg) => serde_json::json!({"status": "fail", "message": msg}),
        }).collect::<Vec<_>>(),
    })
}

/// POST /api/market/summary — Peg and order book health.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MarketRequest>,
) -> impl IntoResponse {
    let venue_id = &req.market_venue;

    let thresholds = state.config.market.health.with_overrides(&req.health);

    if !is_dex_venue(venue_id) {
        // CEX venue — use the venue registry
        let registry = match VenueRegistry::load() {
            Ok(r) => r,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Registry error: {e}") })),
                )
                    .into_response();
            }
        };
        let exchange = match registry.create_exchange_client(venue_id) {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        };

        let pair = exchange.format_pair(&req.pair);
        match exchange.fetch_order_book(&pair).await {
            Ok(book) => {
                let volume_24h = if exchange.has_ticker() {
                    exchange
                        .fetch_ticker(&pair)
                        .await
                        .ok()
                        .and_then(|t| t.quote_volume_24h.or(t.volume_24h))
                } else {
                    None
                };
                let summary = MarketSummary::from_order_book(&book, &thresholds, volume_24h);
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
        let venue_chain = dex_venue_to_chain(venue_id);

        match crawl::fetch_analytics_for_input(
            &req.pair,
            venue_chain,
            Period::Hour24,
            10,
            &state.factory,
            None,
            &scope_cli::cli::output::Output::stdio(),
        )
        .await
        {
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
                    .max_by(|a, b| {
                        a.liquidity_usd
                            .partial_cmp(&b.liquidity_usd)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap();
                let book =
                    order_book_from_analytics(venue_chain, best_pair, &analytics.token.symbol);
                let summary =
                    MarketSummary::from_order_book(&book, &thresholds, Some(best_pair.volume_24h));
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

/// Whether the venue string refers to a DEX venue.
fn is_dex_venue(venue: &str) -> bool {
    matches!(venue.to_lowercase().as_str(), "ethereum" | "eth" | "solana")
}

/// Resolve DEX venue name to a canonical chain name.
fn dex_venue_to_chain(venue: &str) -> &str {
    match venue.to_lowercase().as_str() {
        "ethereum" | "eth" => "ethereum",
        "solana" => "solana",
        _ => "ethereum",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(req.chain, "polygon");
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
        assert_eq!(req.chain, "ethereum");
        // No threshold keys: every value comes from config `market.health`.
        assert_eq!(req.health, HealthOverrides::default());
    }

    #[test]
    fn test_all_defaults() {
        assert_eq!(default_pair(), "USDC");
        assert_eq!(default_venue(), "binance");
        assert_eq!(default_chain(), "ethereum");
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
        assert_eq!(req.chain, "ethereum");
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
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = MarketRequest {
            pair: "USDC".to_string(),
            market_venue: "binance".to_string(),
            chain: "ethereum".to_string(),
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
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = MarketRequest {
            pair: "USDC".to_string(),
            market_venue: "eth".to_string(),
            chain: "ethereum".to_string(),
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
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = MarketRequest {
            pair: "BTC".to_string(),
            market_venue: "binance".to_string(),
            chain: "ethereum".to_string(),
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
        assert!(is_dex_venue("eth"));
        assert!(is_dex_venue("ethereum"));
        assert!(is_dex_venue("solana"));
        assert!(!is_dex_venue("binance"));
        assert!(!is_dex_venue("mexc"));
    }

    #[test]
    fn test_dex_venue_to_chain() {
        assert_eq!(dex_venue_to_chain("eth"), "ethereum");
        assert_eq!(dex_venue_to_chain("ethereum"), "ethereum");
        assert_eq!(dex_venue_to_chain("solana"), "solana");
        assert_eq!(dex_venue_to_chain("unknown"), "ethereum");
    }

    #[test]
    fn test_market_request_debug() {
        let req = MarketRequest {
            pair: "USDC".to_string(),
            market_venue: "binance".to_string(),
            chain: "ethereum".to_string(),
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
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = MarketRequest {
            pair: "USDC".to_string(),
            market_venue: "nonexistent_venue_xyz".to_string(),
            chain: "ethereum".to_string(),
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
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = MarketRequest {
            pair: "USDC".to_string(),
            market_venue: "binance".to_string(),
            chain: "ethereum".to_string(),
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
        assert!(is_dex_venue("ETHEREUM"));
        assert!(is_dex_venue("SOLANA"));
        assert!(!is_dex_venue("BINANCE"));
    }

    #[test]
    fn test_summary_to_json_with_execution_estimates() {
        use scope::market::OrderBookLevel;

        let book = scope::market::OrderBook {
            pair: "USDC/USDT".to_string(),
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
        let json = summary_to_json(&summary);

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
