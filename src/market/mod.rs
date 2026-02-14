//! # Market Module
//!
//! Provides peg and order book health analysis for stablecoin markets.
//! Uses a **data-driven architecture**: exchange behaviour is described by YAML
//! venue descriptors (see `venues/` directory). A `VenueRegistry` loads all
//! descriptors and creates generic `ExchangeClient` instances at runtime, so
//! new venues can be added without code changes.
//!
//! ## Supported Venues
//!
//! Built-in CEX descriptors: Binance, MEXC, Bitget, Gate.io, Bybit, OKX,
//! Coinbase, Kraken, HTX, Crypto.com, and Biconomy. DEX data (Ethereum, Solana)
//! is synthesized from DexScreener liquidity.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use scope::market::{VenueRegistry, HealthThresholds, MarketSummary};
//!
//! #[tokio::main]
//! async fn main() -> scope::Result<()> {
//!     let registry = VenueRegistry::load()?;
//!     let exchange = registry.create_exchange_client("binance")?;
//!     let pair = exchange.format_pair("USDC");
//!     let book = exchange.fetch_order_book(&pair).await?;
//!     let summary = MarketSummary::from_order_book(&book, 1.0, &HealthThresholds::default(), None);
//!     print!("{}", summary.format_text(Some("binance")));
//!     Ok(())
//! }
//! ```

// Submodules (split from the original monolithic orderbook.rs)
pub mod analytics;
pub mod health;
pub mod traits;
pub mod types;

// Data-driven venue system
pub mod configurable_client;
pub mod descriptor;
pub mod exchange;
pub mod registry;

// Backward-compatible re-export shim (allows `use crate::market::OrderBook` etc.)
mod orderbook;

pub use configurable_client::ConfigurableExchangeClient;
pub use descriptor::VenueDescriptor;
pub use exchange::ExchangeClient;
pub use orderbook::{
    ExecutionEstimate, ExecutionSide, HealthCheck, HealthThresholds, MarketSnapshot, MarketSummary,
    OrderBook, OrderBookClient, OrderBookLevel, Ticker, TickerClient, Trade, TradeHistoryClient,
    TradeSide, order_book_from_analytics,
};
pub use registry::VenueRegistry;
