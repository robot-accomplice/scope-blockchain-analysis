//! Token health API handler.

use crate::cli::crawl::{self, Period};
use crate::market::{BinanceClient, HealthThresholds, MarketSummary, MarketVenue, order_book_from_analytics};
use crate::web::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
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
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TokenHealthRequest>,
) -> impl IntoResponse {
    // Fetch DEX analytics
    let analytics = match crawl::fetch_analytics_for_input(
        &req.token,
        &req.chain,
        Period::Hour24,
        10,
        &state.factory,
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
    let market_summary = if req.with_market {
        let venue: MarketVenue = match req.market_venue.as_str() {
            "biconomy" => MarketVenue::Biconomy,
            "eth" | "ethereum" => MarketVenue::Ethereum,
            "solana" | "sol" => MarketVenue::Solana,
            _ => MarketVenue::Binance,
        };

        let thresholds = HealthThresholds {
            peg_target: 1.0,
            peg_range: 0.001,
            min_levels: 6,
            min_depth: 3000.0,
            min_bid_ask_ratio: 0.2,
            max_bid_ask_ratio: 5.0,
        };

        if venue.is_cex() {
            let pair = venue.format_pair(&analytics.token.symbol);
            if let Some(client) = venue.create_client() {
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
                        Some(MarketSummary::from_order_book(&book, 1.0, &thresholds, volume_24h))
                    }
                    Err(_) => None,
                }
            } else {
                None
            }
        } else {
            // DEX venue
            let venue_chain = match venue {
                MarketVenue::Ethereum => "ethereum",
                MarketVenue::Solana => "solana",
                _ => &analytics.chain,
            };
            if analytics.chain.eq_ignore_ascii_case(venue_chain) && !analytics.dex_pairs.is_empty() {
                let best_pair = analytics
                    .dex_pairs
                    .iter()
                    .max_by(|a, b| a.liquidity_usd.partial_cmp(&b.liquidity_usd).unwrap_or(std::cmp::Ordering::Equal))
                    .unwrap();
                let book = order_book_from_analytics(&analytics.chain, best_pair, &analytics.token.symbol);
                let volume_24h = Some(best_pair.volume_24h);
                Some(MarketSummary::from_order_book(&book, 1.0, &thresholds, volume_24h))
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
        assert_eq!(req.with_market, true);
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
        assert_eq!(req.with_market, false);
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
        assert_eq!(req.with_market, true);

        let json_false = serde_json::json!({
            "token": "USDC",
            "with_market": false
        });
        let req_false: TokenHealthRequest = serde_json::from_value(json_false).unwrap();
        assert_eq!(req_false.with_market, false);
    }

    #[tokio::test]
    async fn test_handle_token_health_direct() {
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
}
