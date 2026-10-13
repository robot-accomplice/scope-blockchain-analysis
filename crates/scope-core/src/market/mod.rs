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
//! use scope::market::{VenueRegistry, HealthThresholds, MarketSummary, Pair};
//!
//! #[tokio::main]
//! async fn main() -> scope::Result<()> {
//!     let registry = VenueRegistry::load()?;
//!     let exchange = registry.create_exchange_client("binance")?;
//!     let pair = exchange.venue_symbol(&Pair::parse("USDC"));
//!     let book = exchange.fetch_order_book(&pair).await?;
//!     let summary = MarketSummary::from_order_book(&book, &HealthThresholds::default(), None);
//!     print!("{}", summary.format_text(Some("binance")));
//!     Ok(())
//! }
//! ```

// Submodules (split from the original monolithic orderbook.rs)
pub mod analytics;
pub mod fetch;
pub mod health;
pub mod pair;
pub mod series;
pub mod traits;
pub mod types;

// Data-driven venue system
pub mod configurable_client;
mod default_venue;
pub mod descriptor;
pub mod exchange;
pub mod registry;
pub mod venue;

/// Test helpers: a built-in venue pointed at a local mock server.
#[cfg(any(test, feature = "test-support"))]
pub mod testing {
    use super::{ExchangeClient, VenueRegistry};

    /// The built-in venue `id`, with its real mapping, sending to
    /// `base_url` (a mockito server) instead of the live API.
    pub fn venue_at(id: &str, base_url: &str) -> ExchangeClient {
        let mut desc = VenueRegistry::load()
            .expect("built-in venues load")
            .get(id)
            .unwrap_or_else(|| panic!("no built-in venue {id}"))
            .clone();
        desc.base_url = base_url.to_string();
        ExchangeClient::from_descriptor(&desc)
    }
}

// Backward-compatible re-export shim (allows `use crate::market::OrderBook` etc.)
mod orderbook;

pub use configurable_client::ConfigurableExchangeClient;
pub use default_venue::DEFAULT_VENUE;
pub use descriptor::VenueDescriptor;
pub use exchange::ExchangeClient;
pub use fetch::{DexAnalyticsSource, fetch_book_and_volume, fetch_summary, fetch_token_summary};
pub use orderbook::{
    BookSide, BookSource, Candle, ExecutionEstimate, ExecutionSide, HealthCheck, HealthOverrides,
    HealthThresholds, MarketSnapshot, MarketSummary, OhlcClient, OrderBook, OrderBookClient,
    OrderBookLevel, Ticker, TickerClient, Trade, TradeHistoryClient, TradeSide,
    order_book_from_analytics,
};
pub use pair::Pair;
pub use registry::VenueRegistry;
pub use venue::{DexVenue, MarketVenue, VENUE_HELP, VenueKind, VenueListing};
