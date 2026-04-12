//! Order book fetching and peg/health analysis.
//!
//! This module re-exports types from the split submodules for backward
//! compatibility. The actual implementations live in:
//! - `types` — Core data types (OrderBook, Trade, Ticker, etc.)
//! - `traits` — Client traits (OrderBookClient, TickerClient, TradeHistoryClient)
//! - `health` — Health thresholds and MarketSummary
//! - `analytics` — DEX synthetic order book utilities

// Re-export everything from submodules so existing `use crate::market::X` paths still work.
pub use super::analytics::order_book_from_analytics;
pub use super::health::{HealthThresholds, MarketSummary};
pub use super::traits::{OhlcClient, OrderBookClient, TickerClient, TradeHistoryClient};
pub use super::types::{
    Candle, ExecutionEstimate, ExecutionSide, HealthCheck, MarketSnapshot, OrderBook,
    OrderBookLevel, Ticker, Trade, TradeSide,
};
