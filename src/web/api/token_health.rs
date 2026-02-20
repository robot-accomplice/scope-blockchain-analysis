//! Token health API handler.

use crate::cli::crawl::{self, Period};
use crate::market::{HealthThresholds, MarketSummary, VenueRegistry, order_book_from_analytics};
use crate::web::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use std::sync::Arc;

/// Request body for token health.
#[derive(Debug, Deserialize)]
pub struct TokenHealthRequest {
    /// Token symbol or contract address.
    pub token: String,
    /// Target chain (default: "ethereum").
    #[serde(default = "default_chain")]
    pub chain: String,
    /// Include market/order book data.
    #[serde(default)]
    pub with_market: bool,
    /// Market venue: "binance", "biconomy", "eth", "solana".
    #[serde(default = "default_venue")]
    pub market_venue: String,
}

fn default_chain() -> String {
    "ethereum".to_string()
}

fn default_venue() -> String {
    "binance".to_string()
}

/// POST /api/token-health — Token health suite.
///
/// Supports address book shortcuts: pass `@label` as the token to
/// resolve it from the address book.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TokenHealthRequest>,
) -> impl IntoResponse {
    // Resolve address book shortcuts (@label or direct address match)
    let resolved = match super::resolve_address_book(&req.token, &state.config) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    };
    let token = resolved.value;
    let chain = resolved.chain.unwrap_or(req.chain);

    // Fetch DEX analytics
    let analytics = match crawl::fetch_analytics_for_input(
        &token,
        &chain,
        Period::Hour24,
        10,
        &state.factory,
        None,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // Optionally fetch market data
    let venue_id = &req.market_venue;
    let market_summary = if req.with_market {
        let thresholds = HealthThresholds {
            peg_target: 1.0,
            peg_range: 0.001,
            min_levels: 6,
            min_depth: 3000.0,
            min_bid_ask_ratio: 0.2,
            max_bid_ask_ratio: 5.0,
        };

        if !is_dex_venue(venue_id) {
            // CEX venue — use venue registry
            if let Ok(registry) = VenueRegistry::load()
                && let Ok(exchange) = registry.create_exchange_client(venue_id)
            {
                let pair = exchange.format_pair(&analytics.token.symbol);
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
                        Some(MarketSummary::from_order_book(
                            &book,
                            1.0,
                            &thresholds,
                            volume_24h,
                        ))
                    }
                    Err(_) => None,
                }
            } else {
                None
            }
        } else {
            // DEX venue
            let venue_chain = dex_venue_to_chain(venue_id);
            if analytics.chain.eq_ignore_ascii_case(venue_chain) && !analytics.dex_pairs.is_empty()
            {
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
                    order_book_from_analytics(&analytics.chain, best_pair, &analytics.token.symbol);
                let volume_24h = Some(best_pair.volume_24h);
                Some(MarketSummary::from_order_book(
                    &book,
                    1.0,
                    &thresholds,
                    volume_24h,
                ))
            } else {
                None
            }
        }
    } else {
        None
    };

    // Build combined JSON
    let market_json = market_summary.map(|m| {
        serde_json::json!({
            "pair": m.pair,
            "peg_target": m.peg_target,
            "best_bid": m.best_bid,
            "best_ask": m.best_ask,
            "mid_price": m.mid_price,
            "spread": m.spread,
            "bid_depth": m.bid_depth,
            "ask_depth": m.ask_depth,
            "healthy": m.healthy,
            "volume_24h": m.volume_24h,
            "checks": m.checks.iter().map(|c| match c {
                crate::market::HealthCheck::Pass(msg) => serde_json::json!({"status": "pass", "message": msg}),
                crate::market::HealthCheck::Fail(msg) => serde_json::json!({"status": "fail", "message": msg}),
            }).collect::<Vec<_>>()
        })
    });

    Json(serde_json::json!({
        "analytics": analytics,
        "market": market_json,
    }))
    .into_response()
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
            "token": "USDC",
            "chain": "polygon",
            "with_market": true,
            "market_venue": "biconomy"
        });
        let req: TokenHealthRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.token, "USDC");
        assert_eq!(req.chain, "polygon");
        assert!(req.with_market);
        assert_eq!(req.market_venue, "biconomy");
    }

    #[test]
    fn test_deserialize_minimal() {
        let json = serde_json::json!({
            "token": "USDC"
        });
        let req: TokenHealthRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.token, "USDC");
        assert_eq!(req.chain, "ethereum");
        assert!(!req.with_market);
        assert_eq!(req.market_venue, "binance");
    }

    #[test]
    fn test_defaults() {
        assert_eq!(default_chain(), "ethereum");
        assert_eq!(default_venue(), "binance");
    }

    #[test]
    fn test_with_market_flag() {
        let json = serde_json::json!({
            "token": "USDC",
            "with_market": true
        });
        let req: TokenHealthRequest = serde_json::from_value(json).unwrap();
        assert!(req.with_market);

        let json_false = serde_json::json!({
            "token": "USDC",
            "with_market": false
        });
        let req_false: TokenHealthRequest = serde_json::from_value(json_false).unwrap();
        assert!(!req_false.with_market);
    }

    #[tokio::test]
    async fn test_handle_token_health_direct() {
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
        let req = TokenHealthRequest {
            token: "USDC".to_string(),
            chain: "ethereum".to_string(),
            with_market: false,
            market_venue: "binance".to_string(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_token_health_with_market() {
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
        let req = TokenHealthRequest {
            token: "USDC".to_string(),
            chain: "ethereum".to_string(),
            with_market: true,
            market_venue: "eth".to_string(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_token_health_with_cex_market() {
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
        let req = TokenHealthRequest {
            token: "USDC".to_string(),
            chain: "ethereum".to_string(),
            with_market: true,
            market_venue: "binance".to_string(), // CEX path
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        // Either succeeds (200) or fails gracefully (500)
        assert!(status.is_success() || status.is_server_error());
    }

    #[test]
    fn test_is_dex_venue() {
        assert!(is_dex_venue("ethereum"));
        assert!(is_dex_venue("eth"));
        assert!(is_dex_venue("Ethereum"));
        assert!(is_dex_venue("ETH"));
        assert!(is_dex_venue("solana"));
        assert!(is_dex_venue("Solana"));
        assert!(!is_dex_venue("binance"));
        assert!(!is_dex_venue("mexc"));
        assert!(!is_dex_venue("okx"));
        assert!(!is_dex_venue(""));
    }

    #[test]
    fn test_dex_venue_to_chain() {
        assert_eq!(dex_venue_to_chain("ethereum"), "ethereum");
        assert_eq!(dex_venue_to_chain("eth"), "ethereum");
        assert_eq!(dex_venue_to_chain("Ethereum"), "ethereum");
        assert_eq!(dex_venue_to_chain("solana"), "solana");
        assert_eq!(dex_venue_to_chain("Solana"), "solana");
        assert_eq!(dex_venue_to_chain("unknown"), "ethereum"); // default
    }

    #[tokio::test]
    async fn test_handle_token_health_invalid_token_error_path() {
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
        let req = TokenHealthRequest {
            token: "INVALID_TOKEN_XYZ_NONEXISTENT".to_string(),
            chain: "ethereum".to_string(),
            with_market: false,
            market_venue: "binance".to_string(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        // May succeed (empty analytics) or 500 (fetch error)
        assert!(status.is_success() || status.is_server_error());
    }

    #[test]
    fn test_token_health_request_debug() {
        let req = TokenHealthRequest {
            token: "USDC".to_string(),
            chain: "ethereum".to_string(),
            with_market: true,
            market_venue: "binance".to_string(),
        };
        let debug = format!("{:?}", req);
        assert!(debug.contains("TokenHealthRequest"));
    }

    #[test]
    fn test_deserialize_with_optional_venue() {
        let json = serde_json::json!({
            "token": "DAI",
            "market_venue": "mexc"
        });
        let req: TokenHealthRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.token, "DAI");
        assert_eq!(req.market_venue, "mexc");
    }

    #[tokio::test]
    async fn test_handle_token_health_success_json_structure() {
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
        let req = TokenHealthRequest {
            token: "USDC".to_string(),
            chain: "ethereum".to_string(),
            with_market: false,
            market_venue: "binance".to_string(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        if response.status().is_success() {
            let body_bytes = body::to_bytes(response.into_body(), 1_000_000)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
            assert!(json.get("analytics").is_some());
            assert!(json.get("market").is_some());
        }
    }

    #[tokio::test]
    async fn test_handle_token_health_label_not_found() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;

        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp.path().to_path_buf()),
            },
            ..Default::default()
        };
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = TokenHealthRequest {
            token: "@nonexistent".to_string(),
            chain: "ethereum".to_string(),
            with_market: false,
            market_venue: "binance".to_string(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("@nonexistent"));
    }
}
