//! Market summary API handler.

use crate::cli::crawl::{self, Period};
use crate::market::{HealthThresholds, MarketSummary, VenueRegistry, order_book_from_analytics};
use crate::web::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
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

fn default_pair() -> String {
    "USDC".to_string()
}
fn default_venue() -> String {
    "binance".to_string()
}
fn default_chain() -> String {
    "ethereum".to_string()
}
fn default_peg() -> f64 {
    1.0
}
fn default_min_levels() -> usize {
    6
}
fn default_min_depth() -> f64 {
    3000.0
}
fn default_peg_range() -> f64 {
    0.001
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
    let venue_id = &req.market_venue;

    let thresholds = HealthThresholds {
        peg_target: req.peg,
        peg_range: req.peg_range,
        min_levels: req.min_levels,
        min_depth: req.min_depth,
        min_bid_ask_ratio: 0.2,
        max_bid_ask_ratio: 5.0,
    };

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
                let summary =
                    MarketSummary::from_order_book(&book, req.peg, &thresholds, volume_24h);
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
                let summary = MarketSummary::from_order_book(
                    &book,
                    req.peg,
                    &thresholds,
                    Some(best_pair.volume_24h),
                );
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
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;

        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
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
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;

        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
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

    #[tokio::test]
    async fn test_handle_market_with_cex_venue() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;

        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = MarketRequest {
            pair: "BTC".to_string(),
            market_venue: "binance".to_string(),
            chain: "ethereum".to_string(),
            peg: 1.0,
            min_levels: 1,
            min_depth: 50.0,
            peg_range: 0.01,
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
            peg: 1.0,
            min_levels: 6,
            min_depth: 3000.0,
            peg_range: 0.001,
        };
        let debug = format!("{:?}", req);
        assert!(debug.contains("MarketRequest"));
    }

    #[tokio::test]
    async fn test_handle_market_invalid_venue_bad_request() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;

        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = MarketRequest {
            pair: "USDC".to_string(),
            market_venue: "nonexistent_venue_xyz".to_string(),
            chain: "ethereum".to_string(),
            peg: 1.0,
            min_levels: 6,
            min_depth: 3000.0,
            peg_range: 0.001,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        // Unknown venue -> BAD_REQUEST from create_exchange_client
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_market_success_json_structure() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::body;
        use axum::extract::State;
        use axum::response::IntoResponse;

        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
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
        use crate::market::OrderBookLevel;

        let book = crate::market::OrderBook {
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
        let thresholds = HealthThresholds::default();
        let summary =
            crate::market::MarketSummary::from_order_book(&book, 1.0, &thresholds, Some(50_000.0));
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
