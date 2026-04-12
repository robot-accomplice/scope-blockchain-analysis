//! WebSocket monitor handler for live token data streaming.
//!
//! Streams real-time token price, volume, and transaction data
//! to connected browser clients via WebSocket.

use crate::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use scope::chains::dex::DexTokenData;
use scope::chains::{ChainClientFactory, DexDataSource};
use scope::market::{ExchangeClient, TradeSide, VenueRegistry};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

/// Query parameters for the WebSocket monitor connection.
#[derive(Debug, Deserialize)]
pub struct MonitorQuery {
    /// Token address or symbol to monitor.
    pub token: String,
    /// Chain (default: "ethereum").
    #[serde(default = "default_chain")]
    pub chain: String,
    /// Refresh interval in seconds (default: 5).
    #[serde(default = "default_refresh")]
    pub refresh: u64,
    /// Optional exchange venue ID (e.g., "binance") — when set, exchange data
    /// (order book, ticker, recent trades) is included in each update frame.
    #[serde(default)]
    pub venue: Option<String>,
    /// Base pair for exchange data (default: same as token).
    #[serde(default)]
    pub pair: Option<String>,
}

fn default_chain() -> String {
    "ethereum".to_string()
}

fn default_refresh() -> u64 {
    5
}

/// WS /ws/monitor — WebSocket endpoint for live token monitoring.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Query(params): Query<MonitorQuery>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, params))
}

/// Handles an individual WebSocket connection.
///
/// Polls DexScreener at the configured interval and sends JSON updates
/// containing price, volume, and activity data.
async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>, params: MonitorQuery) {
    let dex_client: Box<dyn DexDataSource> = state.factory.create_dex_client();
    let refresh = Duration::from_secs(params.refresh.max(1));

    // Resolve token to address if needed
    let token_input = params.token.clone();
    let chain = params.chain.clone();

    // Optionally create an exchange client for CEX data
    let exchange_client: Option<ExchangeClient> = params.venue.as_ref().and_then(|venue_id| {
        VenueRegistry::load()
            .ok()
            .and_then(|r| r.create_exchange_client(venue_id).ok())
    });

    let exchange_pair: Option<String> = exchange_client.as_ref().map(|ec| {
        let base = params.pair.as_deref().unwrap_or(&token_input);
        ec.format_pair(base)
    });

    // Send initial connection message
    let init_msg = serde_json::json!({
        "type": "connected",
        "token": token_input,
        "chain": chain,
        "refresh_secs": params.refresh,
        "venue": params.venue,
        "exchange_pair": exchange_pair,
    });
    if socket
        .send(Message::Text(init_msg.to_string()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        // Fetch latest DEX token data
        let data: scope::error::Result<DexTokenData> =
            dex_client.get_token_data(&chain, &token_input).await;

        // Optionally fetch exchange snapshot in parallel
        let exchange_snapshot = if let (Some(ec), Some(pair)) = (&exchange_client, &exchange_pair) {
            Some(ec.fetch_market_snapshot(pair).await)
        } else {
            None
        };

        let msg = match data {
            Ok(token_data) => {
                let mut frame = serde_json::json!({
                    "type": "update",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "token": {
                        "symbol": token_data.symbol,
                        "name": token_data.name,
                        "address": token_data.address,
                    },
                    "price_usd": token_data.price_usd,
                    "price_change_24h": token_data.price_change_24h,
                    "price_change_6h": token_data.price_change_6h,
                    "price_change_1h": token_data.price_change_1h,
                    "volume_24h": token_data.volume_24h,
                    "volume_6h": token_data.volume_6h,
                    "volume_1h": token_data.volume_1h,
                    "liquidity_usd": token_data.liquidity_usd,
                    "market_cap": token_data.market_cap,
                    "buys_24h": token_data.total_buys_24h,
                    "sells_24h": token_data.total_sells_24h,
                    "buys_1h": token_data.total_buys_1h,
                    "sells_1h": token_data.total_sells_1h,
                    "pairs": token_data.pairs.iter().take(5).map(|p| {
                        serde_json::json!({
                            "dex": p.dex_name,
                            "base": p.base_token,
                            "quote": p.quote_token,
                            "price_usd": p.price_usd,
                            "volume_24h": p.volume_24h,
                            "liquidity_usd": p.liquidity_usd,
                        })
                    }).collect::<Vec<_>>(),
                });

                // Attach exchange data if available
                if let Some(snap) = &exchange_snapshot {
                    attach_exchange_data(&mut frame, snap);
                }

                frame
            }
            Err(e) => {
                let mut frame = serde_json::json!({
                    "type": "error",
                    "message": e.to_string(),
                });
                // Still attach exchange data even on DEX error
                if let Some(snap) = &exchange_snapshot {
                    attach_exchange_data(&mut frame, snap);
                }
                frame
            }
        };

        if socket.send(Message::Text(msg.to_string())).await.is_err() {
            // Client disconnected
            break;
        }

        // Wait for next refresh or client message
        tokio::select! {
            _ = tokio::time::sleep(refresh) => {},
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Text(text))) => {
                        // Handle client commands (e.g., change token)
                        if let Ok(cmd) = serde_json::from_str::<serde_json::Value>(&text)
                            && cmd.get("type").and_then(|t| t.as_str()) == Some("ping")
                        {
                            let pong = serde_json::json!({"type": "pong"});
                            let _ = socket.send(Message::Text(pong.to_string())).await;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Attach exchange snapshot data (order book, ticker, trades) to a JSON frame.
fn attach_exchange_data(frame: &mut serde_json::Value, snap: &scope::market::MarketSnapshot) {
    if let Some(book) = &snap.order_book {
        frame["exchange_order_book"] = serde_json::json!({
            "pair": book.pair,
            "best_bid": book.best_bid(),
            "best_ask": book.best_ask(),
            "mid_price": book.mid_price(),
            "spread": book.spread(),
            "bid_depth": book.bid_depth(),
            "ask_depth": book.ask_depth(),
            "bids": book.bids.iter().take(20).map(|l| {
                serde_json::json!({"price": l.price, "quantity": l.quantity})
            }).collect::<Vec<_>>(),
            "asks": book.asks.iter().take(20).map(|l| {
                serde_json::json!({"price": l.price, "quantity": l.quantity})
            }).collect::<Vec<_>>(),
        });
    }

    if let Some(ticker) = &snap.ticker {
        frame["exchange_ticker"] = serde_json::json!({
            "pair": ticker.pair,
            "last_price": ticker.last_price,
            "high_24h": ticker.high_24h,
            "low_24h": ticker.low_24h,
            "volume_24h": ticker.volume_24h,
            "quote_volume_24h": ticker.quote_volume_24h,
            "best_bid": ticker.best_bid,
            "best_ask": ticker.best_ask,
        });
    }

    if let Some(trades) = &snap.recent_trades {
        frame["exchange_trades"] = serde_json::json!(
            trades
                .iter()
                .take(20)
                .map(|t| {
                    serde_json::json!({
                        "price": t.price,
                        "quantity": t.quantity,
                        "timestamp_ms": t.timestamp_ms,
                        "side": match t.side {
                            TradeSide::Buy => "buy",
                            TradeSide::Sell => "sell",
                        },
                    })
                })
                .collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope::market::{MarketSnapshot, OrderBook, OrderBookLevel, Ticker, Trade};

    #[test]
    fn test_default_chain() {
        assert_eq!(default_chain(), "ethereum");
    }

    #[test]
    fn test_default_refresh() {
        assert_eq!(default_refresh(), 5);
    }

    #[test]
    fn test_deserialize_monitor_query_full() {
        let json = serde_json::json!({
            "token": "USDC",
            "chain": "solana",
            "refresh": 10
        });
        let query: MonitorQuery = serde_json::from_value(json).unwrap();
        assert_eq!(query.token, "USDC");
        assert_eq!(query.chain, "solana");
        assert_eq!(query.refresh, 10);
        assert!(query.venue.is_none());
    }

    #[test]
    fn test_deserialize_monitor_query_minimal() {
        let json = serde_json::json!({
            "token": "ETH"
        });
        let query: MonitorQuery = serde_json::from_value(json).unwrap();
        assert_eq!(query.token, "ETH");
        assert_eq!(query.chain, "ethereum");
        assert_eq!(query.refresh, 5);
        assert!(query.venue.is_none());
        assert!(query.pair.is_none());
    }

    #[test]
    fn test_deserialize_monitor_query_with_venue() {
        let json = serde_json::json!({
            "token": "BTC",
            "venue": "binance",
            "pair": "USDC",
            "refresh": 10
        });
        let query: MonitorQuery = serde_json::from_value(json).unwrap();
        assert_eq!(query.token, "BTC");
        assert_eq!(query.venue.as_deref(), Some("binance"));
        assert_eq!(query.pair.as_deref(), Some("USDC"));
    }

    #[test]
    fn test_attach_exchange_data_full() {
        let snapshot = MarketSnapshot {
            order_book: Some(OrderBook {
                pair: "BTC/USDT".to_string(),
                bids: vec![OrderBookLevel {
                    price: 50000.0,
                    quantity: 1.5,
                }],
                asks: vec![OrderBookLevel {
                    price: 50010.0,
                    quantity: 2.0,
                }],
            }),
            ticker: Some(Ticker {
                pair: "BTC/USDT".to_string(),
                last_price: Some(50005.0),
                high_24h: Some(51000.0),
                low_24h: Some(49000.0),
                volume_24h: Some(1_000_000.0),
                quote_volume_24h: Some(50_000_000.0),
                best_bid: Some(50000.0),
                best_ask: Some(50010.0),
            }),
            recent_trades: Some(vec![Trade {
                price: 50005.0,
                quantity: 0.5,
                quote_quantity: Some(25002.5),
                timestamp_ms: 1700000000000,
                side: TradeSide::Buy,
                id: None,
            }]),
        };

        let mut frame = serde_json::json!({"type": "update"});
        attach_exchange_data(&mut frame, &snapshot);

        assert!(frame.get("exchange_order_book").is_some());
        assert!(frame.get("exchange_ticker").is_some());
        assert!(frame.get("exchange_trades").is_some());
        assert_eq!(
            frame["exchange_ticker"]["last_price"].as_f64().unwrap(),
            50005.0
        );
    }

    #[test]
    fn test_attach_exchange_data_empty() {
        let snapshot = MarketSnapshot {
            order_book: None,
            ticker: None,
            recent_trades: None,
        };

        let mut frame = serde_json::json!({"type": "update"});
        attach_exchange_data(&mut frame, &snapshot);

        assert!(frame.get("exchange_order_book").is_none());
        assert!(frame.get("exchange_ticker").is_none());
        assert!(frame.get("exchange_trades").is_none());
    }

    #[test]
    fn test_attach_exchange_data_with_sell_trade() {
        let snapshot = MarketSnapshot {
            order_book: None,
            ticker: None,
            recent_trades: Some(vec![Trade {
                price: 50005.0,
                quantity: 0.5,
                quote_quantity: Some(25002.5),
                timestamp_ms: 1700000000000,
                side: TradeSide::Sell,
                id: None,
            }]),
        };

        let mut frame = serde_json::json!({"type": "update"});
        attach_exchange_data(&mut frame, &snapshot);

        assert!(frame.get("exchange_trades").is_some());
        let trades = frame["exchange_trades"].as_array().unwrap();
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0]["side"], "sell");
    }

    #[test]
    fn test_attach_exchange_data_order_book_only() {
        let snapshot = MarketSnapshot {
            order_book: Some(OrderBook {
                pair: "ETH/USDT".to_string(),
                bids: vec![OrderBookLevel {
                    price: 2000.0,
                    quantity: 2.0,
                }],
                asks: vec![OrderBookLevel {
                    price: 2005.0,
                    quantity: 1.5,
                }],
            }),
            ticker: None,
            recent_trades: None,
        };

        let mut frame = serde_json::json!({"type": "update"});
        attach_exchange_data(&mut frame, &snapshot);

        assert!(frame.get("exchange_order_book").is_some());
        assert_eq!(frame["exchange_order_book"]["pair"], "ETH/USDT");
        assert_eq!(frame["exchange_order_book"]["best_bid"], 2000.0);
        assert_eq!(frame["exchange_order_book"]["best_ask"], 2005.0);
        assert!(frame.get("exchange_ticker").is_none());
        assert!(frame.get("exchange_trades").is_none());
    }

    #[test]
    fn test_attach_exchange_data_ticker_only() {
        let snapshot = MarketSnapshot {
            order_book: None,
            ticker: Some(Ticker {
                pair: "SOL/USDT".to_string(),
                last_price: Some(100.5),
                high_24h: Some(102.0),
                low_24h: Some(99.0),
                volume_24h: Some(50000.0),
                quote_volume_24h: Some(5_000_000.0),
                best_bid: Some(100.0),
                best_ask: Some(101.0),
            }),
            recent_trades: None,
        };

        let mut frame = serde_json::json!({"type": "update"});
        attach_exchange_data(&mut frame, &snapshot);

        assert!(frame.get("exchange_ticker").is_some());
        assert_eq!(frame["exchange_ticker"]["pair"], "SOL/USDT");
        assert_eq!(frame["exchange_ticker"]["last_price"], 100.5);
        assert!(frame.get("exchange_order_book").is_none());
    }
}
