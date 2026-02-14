//! Market client traits for fetching exchange data.
//!
//! These traits define the interface for fetching order books, tickers, and
//! recent trades. They are implemented by `ConfigurableExchangeClient` and
//! composed by the `ExchangeClient` facade.

use crate::error::Result;
use async_trait::async_trait;

use super::types::{OrderBook, Ticker, Trade};

/// Trait for clients that can fetch an order book snapshot.
#[async_trait]
pub trait OrderBookClient: Send + Sync {
    /// Fetch the current order book for the given pair symbol.
    async fn fetch_order_book(&self, pair_symbol: &str) -> Result<OrderBook>;
}

/// Trait for clients that can fetch a 24h ticker.
#[async_trait]
pub trait TickerClient: Send + Sync {
    /// Fetch the 24h ticker for the given pair symbol.
    async fn fetch_ticker(&self, pair_symbol: &str) -> Result<Ticker>;
}

/// Trait for clients that can fetch recent trades.
#[async_trait]
pub trait TradeHistoryClient: Send + Sync {
    /// Fetch the most recent trades for the given pair symbol.
    /// `limit` controls the maximum number of trades returned.
    async fn fetch_recent_trades(&self, pair_symbol: &str, limit: u32) -> Result<Vec<Trade>>;
}
