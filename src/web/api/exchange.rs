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

#[cfg(test)]
mod tests {
    use super::*;

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
}
