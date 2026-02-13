//! # Market Module
//!
//! Provides peg and order book health analysis for stablecoin markets.
//! Fetches level-2 order book data from exchange APIs (e.g., Biconomy) and
//! runs configurable health checks: peg deviation, spread, bid/ask balance,
//! and minimum depth thresholds.
//!
//! ## Supported Exchanges
//!
//! - **Biconomy**: CEX-style REST depth API
//!
//! ## Usage
//!
//! ```rust,no_run
//! use scope::market::{OrderBookClient, MarketSummary, HealthThresholds};
//!
//! #[tokio::main]
//! async fn main() -> scope::Result<()> {
//!     let client = scope::market::BiconomyClient::new("https://api.biconomy.com");
//!     let book = client.fetch_order_book("PUSD_USDT").await?;
//!     let summary = MarketSummary::from_order_book(&book, 1.0, &HealthThresholds::default());
//!     print!("{}", summary.format_text(Some("biconomy")));
//!     Ok(())
//! }
//! ```

mod orderbook;

pub use orderbook::{
    BiconomyClient, HealthCheck, HealthThresholds, MarketSummary, OrderBook, OrderBookClient,
    OrderBookLevel,
};
