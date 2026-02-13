//! # Market Module
//!
//! Provides peg and order book health analysis for stablecoin markets.
//! Fetches level-2 order book data from exchange APIs (e.g., Biconomy) and
//! runs configurable health checks: peg deviation, spread, bid/ask balance,
//! and minimum depth thresholds.
//!
//! ## Supported Exchanges
//!
//! - **Binance**: Spot REST depth API (public, no auth)
//! - **Biconomy**: CEX-style REST depth API
//! - **Ethereum DEX**: Synthesized from DexScreener liquidity
//! - **Solana DEX**: Synthesized from DexScreener liquidity
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
//!     let summary = MarketSummary::from_order_book(&book, 1.0, &HealthThresholds::default(), None);
//!     print!("{}", summary.format_text(Some("biconomy")));
//!     Ok(())
//! }
//! ```

mod orderbook;

pub use orderbook::{
    BiconomyClient, BinanceClient, ExecutionEstimate, ExecutionSide, HealthCheck, HealthThresholds,
    MarketSummary, MarketVenue, OrderBook, OrderBookClient, OrderBookLevel,
    order_book_from_analytics,
};
