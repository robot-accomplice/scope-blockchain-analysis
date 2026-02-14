//! ExchangeClient facade: composes OrderBook, Ticker, and TradeHistory
//! capabilities behind a unified interface with capability discovery.

use crate::error::{Result, ScopeError};
use crate::market::configurable_client::ConfigurableExchangeClient;
use crate::market::descriptor::VenueDescriptor;
use crate::market::orderbook::{
    MarketSnapshot, OrderBook, OrderBookClient, Ticker, TickerClient, Trade, TradeHistoryClient,
};

/// Unified exchange client that wraps per-capability trait objects.
///
/// Created by [`VenueRegistry::create_exchange_client`]. Provides
/// capability discovery and a convenience `fetch_market_snapshot` that
/// fetches all available data in parallel.
pub struct ExchangeClient {
    venue_id: String,
    venue_name: String,
    descriptor: VenueDescriptor,
    order_book: Option<Box<dyn OrderBookClient>>,
    ticker: Option<Box<dyn TickerClient>>,
    trade_history: Option<Box<dyn TradeHistoryClient>>,
}

impl ExchangeClient {
    /// Build an ExchangeClient from a VenueDescriptor.
    ///
    /// Each capability that exists in the descriptor gets a
    /// `ConfigurableExchangeClient` implementation wired in.
    pub fn from_descriptor(desc: &VenueDescriptor) -> Self {
        let client = ConfigurableExchangeClient::new(desc.clone());

        let order_book: Option<Box<dyn OrderBookClient>> = if desc.has_order_book() {
            Some(Box::new(client.clone()))
        } else {
            None
        };
        let ticker: Option<Box<dyn TickerClient>> = if desc.has_ticker() {
            Some(Box::new(client.clone()))
        } else {
            None
        };
        let trade_history: Option<Box<dyn TradeHistoryClient>> = if desc.has_trades() {
            Some(Box::new(client))
        } else {
            None
        };

        Self {
            venue_id: desc.id.clone(),
            venue_name: desc.name.clone(),
            descriptor: desc.clone(),
            order_book,
            ticker,
            trade_history,
        }
    }

    // =========================================================================
    // Metadata
    // =========================================================================

    /// Venue ID (e.g., "binance").
    pub fn venue_id(&self) -> &str {
        &self.venue_id
    }

    /// Venue display name (e.g., "Binance Spot").
    pub fn venue_name(&self) -> &str {
        &self.venue_name
    }

    /// Format a trading pair for this venue.
    pub fn format_pair(&self, base: &str) -> String {
        self.descriptor.format_pair(base, None)
    }

    /// Format a trading pair with explicit quote currency.
    pub fn format_pair_with_quote(&self, base: &str, quote: &str) -> String {
        self.descriptor.format_pair(base, Some(quote))
    }

    // =========================================================================
    // Capability discovery
    // =========================================================================

    /// Whether this client supports order book fetching.
    pub fn has_order_book(&self) -> bool {
        self.order_book.is_some()
    }

    /// Whether this client supports ticker fetching.
    pub fn has_ticker(&self) -> bool {
        self.ticker.is_some()
    }

    /// Whether this client supports trade history fetching.
    pub fn has_trade_history(&self) -> bool {
        self.trade_history.is_some()
    }

    // =========================================================================
    // Individual capability methods
    // =========================================================================

    /// Fetch order book (if supported).
    pub async fn fetch_order_book(&self, pair: &str) -> Result<OrderBook> {
        self.order_book
            .as_ref()
            .ok_or_else(|| {
                ScopeError::Chain(format!("{} does not support order book", self.venue_name))
            })?
            .fetch_order_book(pair)
            .await
    }

    /// Fetch ticker (if supported).
    pub async fn fetch_ticker(&self, pair: &str) -> Result<Ticker> {
        self.ticker
            .as_ref()
            .ok_or_else(|| {
                ScopeError::Chain(format!("{} does not support ticker", self.venue_name))
            })?
            .fetch_ticker(pair)
            .await
    }

    /// Fetch recent trades (if supported).
    pub async fn fetch_recent_trades(&self, pair: &str, limit: u32) -> Result<Vec<Trade>> {
        self.trade_history
            .as_ref()
            .ok_or_else(|| {
                ScopeError::Chain(format!("{} does not support trades", self.venue_name))
            })?
            .fetch_recent_trades(pair, limit)
            .await
    }

    // =========================================================================
    // Combined fetch
    // =========================================================================

    /// Fetch all available market data for a pair in one call.
    ///
    /// Each capability is fetched independently; failures in one capability
    /// do not prevent others from succeeding (they just produce `None`).
    pub async fn fetch_market_snapshot(&self, pair: &str) -> MarketSnapshot {
        let order_book = if self.has_order_book() {
            self.fetch_order_book(pair).await.ok()
        } else {
            None
        };

        let ticker = if self.has_ticker() {
            self.fetch_ticker(pair).await.ok()
        } else {
            None
        };

        let recent_trades = if self.has_trade_history() {
            self.fetch_recent_trades(pair, 50).await.ok()
        } else {
            None
        };

        MarketSnapshot {
            order_book,
            ticker,
            recent_trades,
        }
    }
}

impl std::fmt::Debug for ExchangeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExchangeClient")
            .field("venue_id", &self.venue_id)
            .field("venue_name", &self.venue_name)
            .field("has_order_book", &self.has_order_book())
            .field("has_ticker", &self.has_ticker())
            .field("has_trade_history", &self.has_trade_history())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::registry::VenueRegistry;

    #[test]
    fn test_exchange_client_from_binance_descriptor() {
        let registry = VenueRegistry::default();
        let desc = registry.get("binance").unwrap();
        let client = ExchangeClient::from_descriptor(desc);

        assert_eq!(client.venue_id(), "binance");
        assert_eq!(client.venue_name(), "Binance Spot");
        assert!(client.has_order_book());
        assert!(client.has_ticker());
        assert!(client.has_trade_history());
    }

    #[test]
    fn test_exchange_client_format_pair() {
        let registry = VenueRegistry::default();
        let desc = registry.get("binance").unwrap();
        let client = ExchangeClient::from_descriptor(desc);
        assert_eq!(client.format_pair("BTC"), "BTCUSDT");
        assert_eq!(client.format_pair_with_quote("ETH", "USD"), "ETHUSD");
    }

    #[test]
    fn test_exchange_client_all_venues() {
        let registry = VenueRegistry::default();
        for venue_id in registry.list() {
            let desc = registry.get(venue_id).unwrap();
            let client = ExchangeClient::from_descriptor(desc);
            assert_eq!(client.venue_id(), venue_id);
            // All built-in venues should have at least order_book
            assert!(
                client.has_order_book(),
                "Venue {} missing order_book capability",
                venue_id
            );
        }
    }

    #[test]
    fn test_exchange_client_debug() {
        let registry = VenueRegistry::default();
        let desc = registry.get("okx").unwrap();
        let client = ExchangeClient::from_descriptor(desc);
        let debug = format!("{:?}", client);
        assert!(debug.contains("okx"));
        assert!(debug.contains("has_order_book: true"));
    }
}
