//! Exchange snapshot API handler.
//!
//! POST /api/exchange/snapshot — Fetches full market snapshot (order book, ticker,
//! recent trades) for a given venue and pair.

use super::{error_response, status_for};
use crate::AppState;
use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use scope::market::{ExchangeClient, Pair, VenueRegistry};
use serde::Deserialize;
use std::sync::Arc;

/// Request body for exchange snapshot.
#[derive(Debug, Deserialize)]
pub struct SnapshotRequest {
    /// Venue ID (e.g., "binance", "mexc").
    pub venue: String,
    /// Pair or base symbol, read by `Pair::parse` (e.g., "BTC", "ETH_BTC", "OPUSDT").
    #[serde(default = "default_pair")]
    pub pair: String,
    /// Maximum number of recent trades to fetch.
    #[serde(default = "default_trades_limit")]
    pub trades_limit: u32,
}

/// The client for `venue`. An unknown venue is InvalidInput, which the one
/// status map makes a 400.
fn venue_client(state: &AppState, venue: &str) -> scope::Result<ExchangeClient> {
    VenueRegistry::load()
        .and_then(|r| r.create_exchange_client_with_http(venue, state.factory.http()))
}

fn default_pair() -> String {
    "BTC".to_string()
}

fn default_trades_limit() -> u32 {
    scope::market::exchange::DEFAULT_SNAPSHOT_TRADES
}

/// POST /api/exchange/snapshot — Full market snapshot.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SnapshotRequest>,
) -> impl IntoResponse {
    match venue_client(&state, &req.venue) {
        Ok(exchange) => snapshot_response(&exchange, req).await,
        Err(e) => error_response(status_for(&e), e.to_string()),
    }
}

/// The `snapshot` reply from `exchange`.
async fn snapshot_response(exchange: &ExchangeClient, req: SnapshotRequest) -> Response {
    let pair = exchange.venue_symbol(&Pair::parse(&req.pair));
    let snapshot = exchange
        .fetch_market_snapshot(&pair, req.trades_limit)
        .await;

    let order_book_json = snapshot.order_book.as_ref().map(|b| b.to_json(None));

    let ticker_json = snapshot.ticker.as_ref();

    let trades_json = snapshot.recent_trades.as_ref();

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
    /// Pair or base symbol, read by `Pair::parse` (e.g., "BTC", "ETH_BTC", "OPUSDT").
    #[serde(default = "default_pair")]
    pub pair: String,
    /// Maximum number of trades to return.
    #[serde(default = "default_trades_limit")]
    pub limit: u32,
}

/// POST /api/exchange/trades — Recent trades for a venue/pair.
pub async fn handle_trades(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TradesRequest>,
) -> impl IntoResponse {
    match venue_client(&state, &req.venue) {
        Ok(exchange) => trades_response(&exchange, req).await,
        Err(e) => error_response(status_for(&e), e.to_string()),
    }
}

/// The `trades` reply from `exchange`.
async fn trades_response(exchange: &ExchangeClient, req: TradesRequest) -> Response {
    let pair = exchange.venue_symbol(&Pair::parse(&req.pair));
    match exchange.fetch_recent_trades(&pair, req.limit).await {
        Ok(trades) => Json(serde_json::json!({
            "venue": req.venue,
            "pair": pair,
            "trades": trades,
        }))
        .into_response(),
        Err(e) => error_response(status_for(&e), e.to_string()),
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
    /// Pair or base symbol, read by `Pair::parse` (e.g., "BTC", "ETH_BTC", "OPUSDT").
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
pub async fn handle_ohlc(
    State(state): State<Arc<AppState>>,
    Json(req): Json<OhlcRequest>,
) -> impl IntoResponse {
    match venue_client(&state, &req.venue) {
        Ok(exchange) => ohlc_response(&exchange, req).await,
        Err(e) => error_response(status_for(&e), e.to_string()),
    }
}

/// The `ohlc` reply from `exchange`.
async fn ohlc_response(exchange: &ExchangeClient, req: OhlcRequest) -> Response {
    let pair = exchange.venue_symbol(&Pair::parse(&req.pair));
    match exchange.fetch_ohlc(&pair, &req.interval, req.limit).await {
        Ok(candles) => Json(serde_json::json!({
            "venue": req.venue,
            "pair": pair,
            "interval": req.interval,
            "candles": candles,
        }))
        .into_response(),
        Err(e) => error_response(status_for(&e), e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    /// State for the handlers: venues are looked up for real, with a
    /// native transport.
    fn test_state() -> State<Arc<AppState>> {
        State(Arc::new(AppState::new(
            scope::Config::default(),
            scope::chains::mocks::MockClientFactory::new(),
        )))
    }
    use scope::market::{BookSource, HealthThresholds, MarketSummary, OrderBook, OrderBookLevel};

    #[test]
    fn test_order_book_json_spread_matches_the_market_summary() {
        // The top level of each side has zero quantity: not a real order.
        let level = |price, quantity| OrderBookLevel { price, quantity };
        let book = OrderBook {
            pair: "X/USDT".to_string(),
            source: BookSource::Exchange,
            bids: vec![level(1.0, 0.0), level(0.999, 500.0)],
            asks: vec![level(1.0001, 0.0), level(1.001, 500.0)],
        };
        let summary = MarketSummary::from_order_book(&book, &HealthThresholds::default(), None);
        let json = book.to_json(None);
        assert_eq!(json["spread"].as_f64(), summary.spread);
        assert_eq!(json["best_bid"].as_f64(), summary.best_bid);
        assert_eq!(json["mid_price"].as_f64(), summary.mid_price);
        assert!(json["spread"].as_f64().unwrap() > 0.0019);
    }

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
        let response = handle(test_state(), Json(req)).await.into_response();
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
    async fn test_handle_trades_unknown_venue() {
        let req = TradesRequest {
            venue: "nonexistent_venue_xyz".to_string(),
            pair: "BTC".to_string(),
            limit: 50,
        };
        let response = handle_trades(test_state(), Json(req)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("Unknown venue"));
    }

    #[tokio::test]
    async fn test_handle_ohlc_unknown_venue() {
        let req = OhlcRequest {
            venue: "nonexistent_venue_xyz".to_string(),
            pair: "BTC".to_string(),
            interval: "1h".to_string(),
            limit: 100,
        };
        let response = handle_ohlc(test_state(), Json(req)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("Unknown venue"));
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

    // Each venue's own reply shape, served by a mock: the venue lookup
    // reads the test home, which other tests repoint while they run, and
    // the live APIs answer as they please.
    const VENUE_TRADES: [(&str, &str, &str); 3] = [
        (
            "mexc",
            "/api/v3/trades",
            r#"[{"id":1,"price":"1.5","qty":"2","quoteQty":"3","time":1700000000000,"isBuyerMaker":false}]"#,
        ),
        (
            "okx",
            "/api/v5/market/trades",
            r#"{"data":[{"tradeId":"1","px":"1.5","sz":"2","ts":"1700000000000","side":"buy"}]}"#,
        ),
        (
            "bybit",
            "/v5/market/recent-trade",
            r#"{"result":{"list":[{"execId":"1","price":"1.5","size":"2","time":"1700000000000","side":"Buy"}]}}"#,
        ),
    ];

    const VENUE_OHLC: [(&str, &str, &str); 3] = [
        (
            "mexc",
            "/api/v3/klines",
            r#"[[1700000000000,"1.0","1.5","0.5","1.25","1000.5",1700003599999]]"#,
        ),
        (
            "okx",
            "/api/v5/market/candles",
            r#"{"data":[["1700000000000","1.0","1.5","0.5","1.25","1000.5"]]}"#,
        ),
        (
            "bybit",
            "/v5/market/kline",
            r#"{"result":{"list":[["1700000000000","1.0","1.5","0.5","1.25","1000.5"]]}}"#,
        ),
    ];

    #[tokio::test]
    async fn test_handle_trades_multiple_venues() {
        for (venue, path, body) in VENUE_TRADES {
            let mut server = mockito::Server::new_async().await;
            server
                .mock("GET", path)
                .match_query(mockito::Matcher::Any)
                .with_body(body)
                .create_async()
                .await;
            let exchange = scope::market::testing::venue_at(venue, &server.url());
            let req = TradesRequest {
                venue: venue.to_string(),
                pair: "BTC".to_string(),
                limit: 3,
            };
            let response = trades_response(&exchange, req).await;
            assert_eq!(response.status(), StatusCode::OK, "{venue} trades");
            let json = body_json(response).await;
            assert_eq!(json["trades"][0]["price"], 1.5, "{venue}: {json}");
            assert_eq!(json["trades"][0]["side"], "buy", "{venue}: {json}");
        }
    }

    #[tokio::test]
    async fn test_handle_ohlc_multiple_venues() {
        for (venue, path, body) in VENUE_OHLC {
            let mut server = mockito::Server::new_async().await;
            server
                .mock("GET", path)
                .match_query(mockito::Matcher::Any)
                .with_body(body)
                .create_async()
                .await;
            let exchange = scope::market::testing::venue_at(venue, &server.url());
            let req = OhlcRequest {
                venue: venue.to_string(),
                pair: "BTC".to_string(),
                interval: "1h".to_string(),
                limit: 3,
            };
            let response = ohlc_response(&exchange, req).await;
            assert_eq!(response.status(), StatusCode::OK, "{venue} ohlc");
            let json = body_json(response).await;
            assert_eq!(json["candles"][0]["close"], 1.25, "{venue}: {json}");
        }
    }

    // A venue that fails is a mapped error status, on every venue.
    #[tokio::test]
    async fn test_handle_multiple_venues_report_a_venue_failure() {
        for ((venue, trades_path, _), (_, ohlc_path, _)) in VENUE_TRADES.into_iter().zip(VENUE_OHLC)
        {
            let mut server = mockito::Server::new_async().await;
            for path in [trades_path, ohlc_path] {
                server
                    .mock("GET", path)
                    .match_query(mockito::Matcher::Any)
                    .with_status(500)
                    .create_async()
                    .await;
            }
            let exchange = scope::market::testing::venue_at(venue, &server.url());
            let trades = TradesRequest {
                venue: venue.to_string(),
                pair: "BTC".to_string(),
                limit: 3,
            };
            let status = trades_response(&exchange, trades).await.status();
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{venue} trades");
            let ohlc = OhlcRequest {
                venue: venue.to_string(),
                pair: "BTC".to_string(),
                interval: "1h".to_string(),
                limit: 3,
            };
            let status = ohlc_response(&exchange, ohlc).await.status();
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{venue} ohlc");
        }
    }

    #[tokio::test]
    async fn test_handle_trades_error_response_structure() {
        let req = TradesRequest {
            venue: "nonexistent_venue_xyz".to_string(),
            pair: "BTC".to_string(),
            limit: 50,
        };
        let response = handle_trades(test_state(), Json(req)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("error").is_some());
    }

    #[tokio::test]
    async fn test_handle_ohlc_error_response_structure() {
        let req = OhlcRequest {
            venue: "nonexistent_venue_xyz".to_string(),
            pair: "BTC".to_string(),
            interval: "1h".to_string(),
            limit: 100,
        };
        let response = handle_ohlc(test_state(), Json(req)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("error").is_some());
    }

    // =================================================================
    // Replies from a mock venue (#39: no live API in tests)
    // =================================================================

    async fn body_json(response: Response) -> serde_json::Value {
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn symbol_btcusdt() -> mockito::Matcher {
        mockito::Matcher::UrlEncoded("symbol".into(), "BTCUSDT".into())
    }

    #[tokio::test]
    async fn test_snapshot_reply_has_book_ticker_and_the_asked_trades() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/v3/depth")
            .match_query(symbol_btcusdt())
            .with_body(r#"{"bids":[["0.999","100"]],"asks":[["1.001","200"]]}"#)
            .create_async()
            .await;
        server
            .mock("GET", "/api/v3/ticker/24hr")
            .match_query(symbol_btcusdt())
            .with_body(r#"{"lastPrice":"1.0","highPrice":"1.1","lowPrice":"0.9","volume":"10","quoteVolume":"10","priceChangePercent":"0.5","bidPrice":"0.999","askPrice":"1.001"}"#)
            .create_async()
            .await;
        // The request's trades_limit reaches the venue (it was ignored).
        let trades = server
            .mock("GET", "/api/v3/trades")
            .match_query(mockito::Matcher::AllOf(vec![
                symbol_btcusdt(),
                mockito::Matcher::UrlEncoded("limit".into(), "7".into()),
            ]))
            .with_body(r#"[{"id":9,"price":"1.0","qty":"3","quoteQty":"3","time":1700000000000,"isBuyerMaker":false}]"#)
            .create_async()
            .await;
        let exchange = scope::market::testing::venue_at("binance", &server.url());
        let req = SnapshotRequest {
            venue: "binance".to_string(),
            pair: "BTC".to_string(),
            trades_limit: 7,
        };
        let json = body_json(snapshot_response(&exchange, req).await).await;
        trades.assert_async().await;
        assert_eq!(json["venue"], "binance");
        assert_eq!(json["pair"], "BTCUSDT");
        assert!(json["order_book"].is_object(), "{json}");
        assert_eq!(json["ticker"]["last_price"], 1.0);
        assert_eq!(json["recent_trades"][0]["side"], "buy");
        assert_eq!(json["recent_trades"][0]["id"], "9");
    }

    #[tokio::test]
    async fn test_trades_reply_lists_the_trades() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/v3/trades")
            .match_query(mockito::Matcher::Any)
            .with_body(r#"[{"id":1,"price":"1.5","qty":"2","quoteQty":"3","time":1700000000000,"isBuyerMaker":true}]"#)
            .create_async()
            .await;
        let exchange = scope::market::testing::venue_at("binance", &server.url());
        let req = TradesRequest {
            venue: "binance".to_string(),
            pair: "BTC".to_string(),
            limit: 5,
        };
        let response = trades_response(&exchange, req).await;
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert_eq!(json["trades"][0]["side"], "sell");
        assert_eq!(json["trades"][0]["price"], 1.5);
        assert_eq!(json["trades"][0]["quote_quantity"], 3.0);
    }

    #[tokio::test]
    async fn test_ohlc_reply_lists_the_candles() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/v3/klines")
            .match_query(mockito::Matcher::Any)
            .with_body(r#"[[1700000000000,"1.0","1.5","0.5","1.25","1000.5",1700003599999]]"#)
            .create_async()
            .await;
        let exchange = scope::market::testing::venue_at("binance", &server.url());
        let req = OhlcRequest {
            venue: "binance".to_string(),
            pair: "BTC".to_string(),
            interval: "1h".to_string(),
            limit: 1,
        };
        let json = body_json(ohlc_response(&exchange, req).await).await;
        assert_eq!(json["interval"], "1h");
        assert_eq!(json["candles"][0]["close"], 1.25);
        assert_eq!(json["candles"][0]["close_time"], 1_700_003_599_999u64);
    }

    // A venue failure is the venue's error status, not a 200 with nulls.
    #[tokio::test]
    async fn test_trades_reply_reports_a_venue_failure() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/v3/trades")
            .match_query(mockito::Matcher::Any)
            .with_status(500)
            .create_async()
            .await;
        let exchange = scope::market::testing::venue_at("binance", &server.url());
        let req = TradesRequest {
            venue: "binance".to_string(),
            pair: "BTC".to_string(),
            limit: 5,
        };
        let response = trades_response(&exchange, req).await;
        assert_ne!(response.status(), StatusCode::OK);
        assert!(
            body_json(response).await["error"]
                .as_str()
                .unwrap()
                .contains("binance")
        );
    }
}
