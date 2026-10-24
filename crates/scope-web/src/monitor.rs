//! WebSocket monitor handler for live token data streaming.
//!
//! Streams real-time token price, volume, and transaction data
//! to connected browser clients via WebSocket.

use crate::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use scope::chains::DexDataSource;
use scope::chains::dex::DexTokenData;
use scope::market::{ExchangeClient, Pair, VenueRegistry};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

/// Query parameters for the WebSocket monitor connection.
#[derive(Debug, Deserialize)]
pub struct MonitorQuery {
    /// Token address or symbol to monitor.
    pub token: String,
    /// Chain (default: "ethereum").
    #[serde(default = "crate::api::default_chain")]
    pub chain: String,
    /// Refresh interval in seconds (default: 5).
    #[serde(default = "default_refresh")]
    pub refresh: u64,
    /// Optional exchange venue ID (e.g., "binance") — when set, exchange data
    /// (order book, ticker, recent trades) is included in each update frame.
    #[serde(default)]
    pub venue: Option<String>,
    /// Pair for exchange data, read like the CLI pair (default: the token
    /// symbol with the venue's default quote).
    #[serde(default)]
    pub pair: Option<String>,
}

fn default_refresh() -> u64 {
    5
}

/// The venue's exchange client, or the one unknown-venue error.
fn exchange_client_for(
    venue: Option<&str>,
    http: Arc<dyn scope::http::HttpClient>,
) -> scope::error::Result<Option<ExchangeClient>> {
    let Some(id) = venue else {
        return Ok(None);
    };
    VenueRegistry::load()?
        .create_exchange_client_with_http(id, http)
        .map(Some)
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

    // Optionally create an exchange client for CEX data. An unknown venue is
    // the one unknown-venue error, reported on the socket (same frame shape
    // as a DexTokenData error), not silently dropped.
    let exchange_client: Option<ExchangeClient> =
        match exchange_client_for(params.venue.as_deref(), state.factory.http()) {
            Ok(client) => client,
            Err(e) => {
                let msg = serde_json::json!({"type": "error", "message": e.to_string()});
                let _ = socket.send(Message::Text(msg.to_string())).await;
                return;
            }
        };

    let exchange_pair: Option<String> = exchange_client.as_ref().map(|ec| {
        let pair = match params.pair.as_deref() {
            Some(pair) => Pair::parse(pair),
            None => Pair::base_only(&token_input),
        };
        ec.venue_symbol(&pair)
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
            Some(
                ec.fetch_market_snapshot(pair, scope::market::exchange::DEFAULT_SNAPSHOT_TRADES)
                    .await,
            )
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

/// Levels and trades a monitor frame lists, at most.
const FRAME_ITEMS: usize = 20;

/// Attach exchange snapshot data (order book, ticker, trades) to a JSON
/// frame, in the one JSON form of each (`OrderBook::to_json`, `Ticker` and
/// `Trade` serialization).
fn attach_exchange_data(frame: &mut serde_json::Value, snap: &scope::market::MarketSnapshot) {
    if let Some(book) = &snap.order_book {
        frame["exchange_order_book"] = book.to_json(Some(FRAME_ITEMS));
    }
    if let Some(ticker) = &snap.ticker {
        frame["exchange_ticker"] = serde_json::json!(ticker);
    }
    if let Some(trades) = &snap.recent_trades {
        let recent: Vec<_> = trades.iter().take(FRAME_ITEMS).collect();
        frame["exchange_trades"] = serde_json::json!(recent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope::market::{MarketSnapshot, OrderBook, OrderBookLevel, Ticker, Trade, TradeSide};

    #[test]
    fn test_default_chain() {
        assert_eq!(crate::api::default_chain(), "ethereum");
    }

    #[test]
    fn test_default_refresh() {
        assert_eq!(default_refresh(), 5);
    }

    /// An unknown venue used to be silently dropped (no exchange data, no
    /// word); now the one unknown-venue error reaches the client.
    #[test]
    fn test_exchange_client_for_unknown_venue_is_a_request_fault() {
        let err = exchange_client_for(Some("nonexistent_venue_xyz"), scope::http::test_transport())
            .err()
            .unwrap();
        assert!(
            matches!(err, scope::error::ScopeError::InvalidInput(_)),
            "{err:?}"
        );
        assert!(err.to_string().contains("Unknown venue"), "{err}");
    }

    #[test]
    fn test_exchange_client_for_none_and_known_venue() {
        assert!(
            exchange_client_for(None, scope::http::test_transport())
                .unwrap()
                .is_none()
        );
        assert!(
            exchange_client_for(Some("binance"), scope::http::test_transport())
                .unwrap()
                .is_some()
        );
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
                source: scope::market::BookSource::Exchange,
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
                price_change_pct_24h: None,
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
    fn test_attach_exchange_data_spread_matches_the_market_summary() {
        use scope::market::{
            BookSource, HealthThresholds, MarketSummary, OrderBook, OrderBookLevel,
        };
        let level = |price, quantity| OrderBookLevel { price, quantity };
        // Zero-quantity top levels are not real orders.
        let book = OrderBook {
            pair: "X/USDT".to_string(),
            source: BookSource::Exchange,
            bids: vec![level(1.0, 0.0), level(0.999, 500.0)],
            asks: vec![level(1.0001, 0.0), level(1.001, 500.0)],
        };
        let summary = MarketSummary::from_order_book(&book, &HealthThresholds::default(), None);
        let snapshot = scope::market::MarketSnapshot {
            order_book: Some(book),
            ticker: None,
            recent_trades: None,
        };
        let mut frame = serde_json::json!({"type": "update"});
        attach_exchange_data(&mut frame, &snapshot);
        assert_eq!(
            frame["exchange_order_book"]["spread"].as_f64(),
            summary.spread
        );
        assert_eq!(
            frame["exchange_order_book"]["mid_price"].as_f64(),
            summary.mid_price
        );
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
                source: scope::market::BookSource::Exchange,
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
                price_change_pct_24h: None,
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

    // ========================================================================
    // The endpoint over a real socket: the router on a local port, a mock
    // DEX, and a venue transport that answers from canned replies.
    // ========================================================================

    use scope::chains::ChainClientFactory;
    use scope::chains::mocks::MockClientFactory;
    use scope::http::{HttpClient, Request, Response};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    /// Answers a Binance-shaped venue's book, ticker and trades requests;
    /// anything else is a 404. Nothing leaves the process.
    struct CannedVenue;

    impl HttpClient for CannedVenue {
        fn send<'a, 'b>(
            &'a self,
            req: Request,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = scope::error::Result<Response>> + Send + 'b>,
        >
        where
            'a: 'b,
            Self: 'b,
        {
            let body = if req.url.contains("/api/v3/depth") {
                Some(r#"{"bids":[["0.999","100"]],"asks":[["1.001","200"]]}"#)
            } else if req.url.contains("/api/v3/ticker/24hr") {
                Some(
                    r#"{"lastPrice":"1.0","highPrice":"1.1","lowPrice":"0.9","volume":"10","quoteVolume":"10","priceChangePercent":"0.5","bidPrice":"0.999","askPrice":"1.001"}"#,
                )
            } else if req.url.contains("/api/v3/trades") {
                Some(
                    r#"[{"id":9,"price":"1.0","qty":"3","quoteQty":"3","time":1700000000000,"isBuyerMaker":false}]"#,
                )
            } else {
                None
            };
            Box::pin(async move {
                Ok(Response {
                    status_code: if body.is_some() { 200 } else { 404 },
                    headers: Default::default(),
                    body: body.unwrap_or_default().to_string(),
                })
            })
        }
    }

    /// The mock chain and DEX clients, with the canned venue transport.
    struct TestFactory(MockClientFactory);

    impl ChainClientFactory for TestFactory {
        fn create_chain_client(
            &self,
            chain: &str,
        ) -> scope::error::Result<Box<dyn scope::chains::ChainClient>> {
            self.0.create_chain_client(chain)
        }

        fn create_dex_client(&self) -> Box<dyn DexDataSource> {
            self.0.create_dex_client()
        }

        fn http(&self) -> Arc<dyn HttpClient> {
            Arc::new(CannedVenue)
        }
    }

    /// Serves the router on a free local port.
    async fn serve(mock: MockClientFactory) -> std::net::SocketAddr {
        let state = Arc::new(crate::AppState::new(
            scope::config::Config::default(),
            TestFactory(mock),
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, crate::build_router(state))
                .await
                .unwrap()
        });
        addr
    }

    /// A minimal WebSocket client: the handshake, unfragmented frames.
    struct Client(TcpStream);

    impl Client {
        async fn connect(addr: std::net::SocketAddr, query: &str) -> Self {
            let mut s = TcpStream::connect(addr).await.unwrap();
            let req = format!(
                "GET /ws/monitor?{query} HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\n\
                 Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                 Sec-WebSocket-Version: 13\r\n\r\n"
            );
            s.write_all(req.as_bytes()).await.unwrap();
            // Byte by byte, so no frame byte is read with the headers.
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                head.push(s.read_u8().await.unwrap());
            }
            let head = String::from_utf8(head).unwrap();
            assert!(head.starts_with("HTTP/1.1 101"), "{head}");
            Self(s)
        }

        /// The next text frame as JSON, or `None` when the server closed.
        async fn recv(&mut self) -> Option<serde_json::Value> {
            let read = async {
                let mut h = [0u8; 2];
                if self.0.read_exact(&mut h).await.is_err() {
                    return None;
                }
                let len = match h[1] & 0x7f {
                    126 => u64::from(self.0.read_u16().await.unwrap()),
                    127 => self.0.read_u64().await.unwrap(),
                    n => u64::from(n),
                };
                let mut payload = vec![0u8; len as usize];
                self.0.read_exact(&mut payload).await.unwrap();
                match h[0] & 0x0f {
                    1 => Some(serde_json::from_slice(&payload).unwrap()),
                    8 => None,
                    op => panic!("unexpected opcode {op}"),
                }
            };
            tokio::time::timeout(Duration::from_secs(10), read)
                .await
                .expect("a frame within 10s")
        }

        /// Sends a masked frame, as a client must.
        async fn send(&mut self, opcode: u8, payload: &[u8]) {
            assert!(payload.len() < 126);
            let mask = [0x12, 0x34, 0x56, 0x78];
            let mut frame = vec![0x80 | opcode, 0x80 | payload.len() as u8];
            frame.extend(mask);
            frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
            self.0.write_all(&frame).await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_the_socket_streams_dex_updates_and_answers_a_ping() {
        let addr = serve(MockClientFactory::new()).await;
        let mut ws = Client::connect(addr, "token=0xmocktoken&chain=base&refresh=1").await;
        let hello = ws.recv().await.unwrap();
        assert_eq!(hello["type"], "connected");
        assert_eq!(hello["token"], "0xmocktoken");
        assert_eq!(hello["chain"], "base");
        assert_eq!(hello["refresh_secs"], 1);
        assert!(hello["venue"].is_null() && hello["exchange_pair"].is_null());

        let update = ws.recv().await.unwrap();
        assert_eq!(update["type"], "update", "{update}");
        assert_eq!(update["token"]["symbol"], "MOCK");
        assert_eq!(update["price_usd"], 1.0);
        assert!(update["pairs"].as_array().unwrap().len() <= 5);
        assert!(
            update.get("exchange_order_book").is_none(),
            "no venue asked"
        );

        // A ping is answered at once, before the next refresh.
        ws.send(1, br#"{"type":"ping"}"#).await;
        assert_eq!(ws.recv().await.unwrap()["type"], "pong");
        // Then the stream goes on.
        assert_eq!(ws.recv().await.unwrap()["type"], "update");
        // Another text message is ignored; a close ends the stream.
        ws.send(1, b"not json").await;
        ws.send(8, &[]).await;
        while let Some(frame) = ws.recv().await {
            assert_eq!(frame["type"], "update", "{frame}");
        }
    }

    #[tokio::test]
    async fn test_a_venue_adds_its_book_ticker_and_trades_to_each_update() {
        let addr = serve(MockClientFactory::new()).await;
        // With no pair, the token is the base, with the venue's quote.
        let mut ws = Client::connect(addr, "token=BTC&venue=binance").await;
        let hello = ws.recv().await.unwrap();
        assert_eq!(hello["venue"], "binance");
        assert_eq!(hello["exchange_pair"], "BTCUSDT");
        let update = ws.recv().await.unwrap();
        assert_eq!(update["type"], "update", "{update}");
        assert_eq!(update["exchange_order_book"]["best_bid"], 0.999);
        assert_eq!(update["exchange_ticker"]["last_price"], 1.0);
        assert_eq!(update["exchange_trades"][0]["side"], "buy");
    }

    #[tokio::test]
    async fn test_a_dex_failure_is_an_error_frame_that_keeps_the_exchange_data() {
        let mut mock = MockClientFactory::new();
        mock.mock_dex.token_data = None;
        let addr = serve(mock).await;
        let mut ws = Client::connect(addr, "token=ETH&venue=binance&pair=ETH/USDC").await;
        let hello = ws.recv().await.unwrap();
        assert_eq!(hello["exchange_pair"], "ETHUSDC", "the asked pair");
        let frame = ws.recv().await.unwrap();
        assert_eq!(frame["type"], "error", "{frame}");
        assert!(
            frame["message"].as_str().unwrap().contains("No DEX data"),
            "{frame}"
        );
        assert_eq!(frame["exchange_ticker"]["last_price"], 1.0, "{frame}");
        assert!(frame["exchange_trades"].is_array(), "{frame}");
    }

    #[tokio::test]
    async fn test_an_unknown_venue_is_reported_and_the_socket_closes() {
        let addr = serve(MockClientFactory::new()).await;
        let mut ws = Client::connect(addr, "token=BTC&venue=nonexistent_venue_xyz").await;
        let frame = ws.recv().await.unwrap();
        assert_eq!(frame["type"], "error", "{frame}");
        assert!(
            frame["message"].as_str().unwrap().contains("Unknown venue"),
            "{frame}"
        );
        assert!(ws.recv().await.is_none(), "no stream after the error");
    }
}
