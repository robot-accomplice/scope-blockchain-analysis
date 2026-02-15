//! Exchange snapshot API handler.
//!
//! POST /api/exchange/snapshot — Fetches full market snapshot (order book, ticker,
//! recent trades) for a given venue and pair.

use crate::market::VenueRegistry;
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;

/// Request body for exchange snapshot.
#[derive(Debug, Deserialize)]
pub struct SnapshotRequest {
    /// Venue ID (e.g., "binance", "mexc").
    pub venue: String,
    /// Base token symbol (e.g., "BTC", "USDC").
    #[serde(default = "default_pair")]
    pub pair: String,
    /// Maximum number of recent trades to fetch.
    #[serde(default = "default_trades_limit")]
    pub trades_limit: u32,
}

fn default_pair() -> String {
    "BTC".to_string()
}

fn default_trades_limit() -> u32 {
    50
}

/// POST /api/exchange/snapshot — Full market snapshot.
pub async fn handle(Json(req): Json<SnapshotRequest>) -> impl IntoResponse {
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

    let exchange = match registry.create_exchange_client(&req.venue) {
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
    let snapshot = exchange.fetch_market_snapshot(&pair).await;

    let order_book_json = snapshot.order_book.as_ref().map(|book| {
        serde_json::json!({
            "pair": book.pair,
            "best_bid": book.best_bid(),
            "best_ask": book.best_ask(),
            "mid_price": book.mid_price(),
            "spread": book.spread(),
            "bid_depth": book.bid_depth(),
            "ask_depth": book.ask_depth(),
            "bids": book.bids.iter().map(|l| {
                serde_json::json!({"price": l.price, "quantity": l.quantity, "value": l.value()})
            }).collect::<Vec<_>>(),
            "asks": book.asks.iter().map(|l| {
                serde_json::json!({"price": l.price, "quantity": l.quantity, "value": l.value()})
            }).collect::<Vec<_>>(),
        })
    });

    let ticker_json = snapshot.ticker.as_ref().map(|t| {
        serde_json::json!({
            "pair": t.pair,
            "last_price": t.last_price,
            "high_24h": t.high_24h,
            "low_24h": t.low_24h,
            "volume_24h": t.volume_24h,
            "quote_volume_24h": t.quote_volume_24h,
            "best_bid": t.best_bid,
            "best_ask": t.best_ask,
        })
    });

    let trades_json = snapshot.recent_trades.as_ref().map(|trades| {
        trades
            .iter()
            .map(|t| {
                serde_json::json!({
                    "price": t.price,
                    "quantity": t.quantity,
                    "quote_quantity": t.quote_quantity,
                    "timestamp_ms": t.timestamp_ms,
                    "side": match t.side {
                        crate::market::TradeSide::Buy => "buy",
                        crate::market::TradeSide::Sell => "sell",
                    },
                    "id": t.id,
                })
            })
            .collect::<Vec<_>>()
    });

    let output = serde_json::json!({
        "venue": req.venue,
        "pair": pair,
        "order_book": order_book_json,
        "ticker": ticker_json,
        "recent_trades": trades_json,
    });

    Json(output).into_response()
}

// =============================================================================
// POST /api/exchange/trades
// =============================================================================

/// Request body for exchange trades.
#[derive(Debug, Deserialize)]
pub struct TradesRequest {
    /// Venue ID (e.g., "binance", "mexc").
    pub venue: String,
    /// Base token symbol (e.g., "BTC", "USDC").
    #[serde(default = "default_pair")]
    pub pair: String,
    /// Maximum number of trades to return.
    #[serde(default = "default_trades_limit")]
    pub limit: u32,
}

/// POST /api/exchange/trades — Recent trades for a venue/pair.
pub async fn handle_trades(Json(req): Json<TradesRequest>) -> impl IntoResponse {
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

    let exchange = match registry.create_exchange_client(&req.venue) {
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
    match exchange.fetch_recent_trades(&pair, req.limit).await {
        Ok(trades) => {
            let json_trades: Vec<serde_json::Value> = trades
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "price": t.price,
                        "quantity": t.quantity,
                        "quote_quantity": t.quote_quantity,
                        "timestamp_ms": t.timestamp_ms,
                        "side": match t.side {
                            crate::market::TradeSide::Buy => "buy",
                            crate::market::TradeSide::Sell => "sell",
                        },
                        "id": t.id,
                    })
                })
                .collect();
            Json(serde_json::json!({
                "venue": req.venue,
                "pair": pair,
                "trades": json_trades,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// =============================================================================
// POST /api/exchange/ohlc
// =============================================================================

/// Request body for exchange OHLC.
#[derive(Debug, Deserialize)]
pub struct OhlcRequest {
    /// Venue ID (e.g., "binance", "mexc").
    pub venue: String,
    /// Base token symbol (e.g., "BTC", "USDC").
    #[serde(default = "default_pair")]
    pub pair: String,
    /// Candle interval (e.g., "1m", "1h", "1d").
    #[serde(default = "default_interval")]
    pub interval: String,
    /// Maximum number of candles to return.
    #[serde(default = "default_ohlc_limit")]
    pub limit: u32,
}

fn default_interval() -> String {
    "1h".to_string()
}

fn default_ohlc_limit() -> u32 {
    100
}

/// POST /api/exchange/ohlc — OHLC candlestick data for a venue/pair.
pub async fn handle_ohlc(Json(req): Json<OhlcRequest>) -> impl IntoResponse {
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

    let exchange = match registry.create_exchange_client(&req.venue) {
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
    match exchange.fetch_ohlc(&pair, &req.interval, req.limit).await {
        Ok(candles) => {
            let json_candles: Vec<serde_json::Value> = candles
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "open_time": c.open_time,
                        "open": c.open,
                        "high": c.high,
                        "low": c.low,
                        "close": c.close,
                        "volume": c.volume,
                        "close_time": c.close_time,
                    })
                })
                .collect();
            Json(serde_json::json!({
                "venue": req.venue,
                "pair": pair,
                "interval": req.interval,
                "candles": json_candles,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn test_snapshot_request_empty_pair() {
        let json = serde_json::json!({"venue": "binance", "pair": ""});
        let req: SnapshotRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.pair, "");
    }

    #[test]
    fn test_snapshot_request_large_limit() {
        let json = serde_json::json!({"venue": "x", "trades_limit": 1000});
        let req: SnapshotRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.trades_limit, 1000);
    }

    #[test]
    fn test_deserialize_full() {
        let json = serde_json::json!({
            "venue": "binance",
            "pair": "USDC",
            "trades_limit": 20
        });
        let req: SnapshotRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.venue, "binance");
        assert_eq!(req.pair, "USDC");
        assert_eq!(req.trades_limit, 20);
    }

    #[test]
    fn test_deserialize_minimal() {
        let json = serde_json::json!({
            "venue": "mexc"
        });
        let req: SnapshotRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.venue, "mexc");
        assert_eq!(req.pair, "BTC");
        assert_eq!(req.trades_limit, 50);
    }

    #[test]
    fn test_defaults() {
        assert_eq!(default_pair(), "BTC");
        assert_eq!(default_trades_limit(), 50);
    }

    #[tokio::test]
    async fn test_handle_unknown_venue() {
        let req = SnapshotRequest {
            venue: "nonexistent_venue_xyz".to_string(),
            pair: "BTC".to_string(),
            trades_limit: 50,
        };
        let response = handle(Json(req)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_snapshot_request_deserialization_with_defaults() {
        // Only venue provided, pair and trades_limit should use defaults
        let json = serde_json::json!({"venue": "kraken"});
        let req: SnapshotRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.venue, "kraken");
        assert_eq!(req.pair, "BTC");
        assert_eq!(req.trades_limit, 50);
    }

    #[test]
    fn test_trades_request_deserialization() {
        let json = serde_json::json!({
            "venue": "binance",
            "pair": "USDC",
            "limit": 25
        });
        let req: TradesRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.venue, "binance");
        assert_eq!(req.pair, "USDC");
        assert_eq!(req.limit, 25);
    }

    #[test]
    fn test_trades_request_defaults() {
        let json = serde_json::json!({"venue": "kraken"});
        let req: TradesRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.venue, "kraken");
        assert_eq!(req.pair, "BTC");
        assert_eq!(req.limit, 50);
    }

    #[test]
    fn test_ohlc_request_deserialization() {
        let json = serde_json::json!({
            "venue": "binance",
            "pair": "ETH",
            "interval": "4h",
            "limit": 200
        });
        let req: OhlcRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.venue, "binance");
        assert_eq!(req.pair, "ETH");
        assert_eq!(req.interval, "4h");
        assert_eq!(req.limit, 200);
    }

    #[test]
    fn test_ohlc_request_defaults() {
        let json = serde_json::json!({"venue": "mexc"});
        let req: OhlcRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.venue, "mexc");
        assert_eq!(req.pair, "BTC");
        assert_eq!(req.interval, "1h");
        assert_eq!(req.limit, 100);
    }

    #[test]
    fn test_snapshot_request_debug() {
        let req = SnapshotRequest {
            venue: "test".to_string(),
            pair: "ETH".to_string(),
            trades_limit: 100,
        };
        let debug = format!("{:?}", req);
        assert!(debug.contains("SnapshotRequest"));
    }

    #[tokio::test]
    async fn test_handle_valid_venue_graceful_failure() {
        // Uses a real venue (binance) — VenueRegistry loads built-in descriptors.
        // The actual API call will likely fail in CI (no network / timeouts),
        // but the handler catches errors gracefully and still returns 200 with null fields.
        let req = SnapshotRequest {
            venue: "binance".to_string(),
            pair: "BTC".to_string(),
            trades_limit: 5,
        };
        let response = handle(Json(req)).await.into_response();
        // Should succeed even if exchange API is unreachable
        let status = response.status();
        assert!(
            status == StatusCode::OK || status == StatusCode::INTERNAL_SERVER_ERROR,
            "Expected 200 or 500, got {}",
            status
        );

        if status == StatusCode::OK {
            let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(json["venue"], "binance");
            assert!(json["pair"].is_string());
            // order_book/ticker/trades may be null (API failed) or populated (API succeeded)
        }
    }

    #[tokio::test]
    async fn test_handle_multiple_venues() {
        // Test with several built-in venues to exercise the handler broadly
        for venue in &["mexc", "okx", "bybit", "coinbase"] {
            let req = SnapshotRequest {
                venue: venue.to_string(),
                pair: "ETH".to_string(),
                trades_limit: 5,
            };
            let response = handle(Json(req)).await.into_response();
            let status = response.status();
            assert!(
                status == StatusCode::OK || status == StatusCode::INTERNAL_SERVER_ERROR,
                "Venue {} returned unexpected status {}",
                venue,
                status
            );
        }
    }

    // =================================================================
    // Trades endpoint tests
    // =================================================================

    #[tokio::test]
    async fn test_handle_trades_unknown_venue() {
        let req = TradesRequest {
            venue: "nonexistent_venue_xyz".to_string(),
            pair: "BTC".to_string(),
            limit: 50,
        };
        let response = handle_trades(Json(req)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("Unknown venue"));
    }

    #[tokio::test]
    async fn test_handle_trades_valid_venue() {
        let req = TradesRequest {
            venue: "binance".to_string(),
            pair: "BTC".to_string(),
            limit: 5,
        };
        let response = handle_trades(Json(req)).await.into_response();
        let status = response.status();
        // May succeed (200 with trades) or fail gracefully (500 due to network)
        assert!(
            status == StatusCode::OK || status == StatusCode::INTERNAL_SERVER_ERROR,
            "Expected 200 or 500, got {}",
            status
        );
    }

    // =================================================================
    // OHLC endpoint tests
    // =================================================================

    #[tokio::test]
    async fn test_handle_ohlc_unknown_venue() {
        let req = OhlcRequest {
            venue: "nonexistent_venue_xyz".to_string(),
            pair: "BTC".to_string(),
            interval: "1h".to_string(),
            limit: 100,
        };
        let response = handle_ohlc(Json(req)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("Unknown venue"));
    }

    #[tokio::test]
    async fn test_handle_ohlc_valid_venue() {
        let req = OhlcRequest {
            venue: "binance".to_string(),
            pair: "BTC".to_string(),
            interval: "1h".to_string(),
            limit: 5,
        };
        let response = handle_ohlc(Json(req)).await.into_response();
        let status = response.status();
        assert!(
            status == StatusCode::OK || status == StatusCode::INTERNAL_SERVER_ERROR,
            "Expected 200 or 500, got {}",
            status
        );
    }

    #[test]
    fn test_trades_request_debug() {
        let req = TradesRequest {
            venue: "test".to_string(),
            pair: "ETH".to_string(),
            limit: 10,
        };
        let debug = format!("{:?}", req);
        assert!(debug.contains("TradesRequest"));
    }

    #[test]
    fn test_ohlc_request_debug() {
        let req = OhlcRequest {
            venue: "test".to_string(),
            pair: "ETH".to_string(),
            interval: "4h".to_string(),
            limit: 50,
        };
        let debug = format!("{:?}", req);
        assert!(debug.contains("OhlcRequest"));
    }

    #[tokio::test]
    async fn test_handle_trades_multiple_venues() {
        for venue in &["mexc", "okx", "bybit"] {
            let req = TradesRequest {
                venue: venue.to_string(),
                pair: "BTC".to_string(),
                limit: 3,
            };
            let response = handle_trades(Json(req)).await.into_response();
            let status = response.status();
            assert!(
                status == StatusCode::OK || status == StatusCode::INTERNAL_SERVER_ERROR,
                "Venue {} trades returned unexpected status {}",
                venue,
                status
            );
        }
    }

    #[tokio::test]
    async fn test_handle_ohlc_multiple_venues() {
        for venue in &["mexc", "okx", "bybit"] {
            let req = OhlcRequest {
                venue: venue.to_string(),
                pair: "BTC".to_string(),
                interval: "1h".to_string(),
                limit: 3,
            };
            let response = handle_ohlc(Json(req)).await.into_response();
            let status = response.status();
            assert!(
                status == StatusCode::OK || status == StatusCode::INTERNAL_SERVER_ERROR,
                "Venue {} ohlc returned unexpected status {}",
                venue,
                status
            );
        }
    }
}
