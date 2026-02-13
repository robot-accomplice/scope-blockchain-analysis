//! WebSocket monitor handler for live token data streaming.
//!
//! Streams real-time token price, volume, and transaction data
//! to connected browser clients via WebSocket.

use crate::chains::dex::DexTokenData;
use crate::chains::{ChainClientFactory, DexDataSource};
use crate::web::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
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

    // Send initial connection message
    let init_msg = serde_json::json!({
        "type": "connected",
        "token": token_input,
        "chain": chain,
        "refresh_secs": params.refresh,
    });
    if socket
        .send(Message::Text(init_msg.to_string()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        // Fetch latest token data
        let data: crate::error::Result<DexTokenData> =
            dex_client.get_token_data(&chain, &token_input).await;

        let msg = match data {
            Ok(token_data) => {
                serde_json::json!({
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
                })
            }
            Err(e) => {
                serde_json::json!({
                    "type": "error",
                    "message": e.to_string(),
                })
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

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn test_deserialize_monitor_query_custom_refresh() {
        let json = serde_json::json!({
            "token": "BTC",
            "refresh": 30
        });
        let query: MonitorQuery = serde_json::from_value(json).unwrap();
        assert_eq!(query.token, "BTC");
        assert_eq!(query.chain, "ethereum");
        assert_eq!(query.refresh, 30);
    }
}
