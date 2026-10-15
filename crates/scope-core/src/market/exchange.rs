//! ExchangeClient facade: composes OrderBook, Ticker, and TradeHistory
//! capabilities behind a unified interface with capability discovery.

use crate::error::Result;
use crate::market::configurable_client::{ConfigurableExchangeClient, venue_error};
use crate::market::descriptor::VenueDescriptor;
use crate::market::orderbook::{
    Candle, MarketSnapshot, OhlcClient, OrderBook, OrderBookClient, Ticker, TickerClient, Trade,
    TradeHistoryClient,
};

/// Recent trades in a market snapshot when the caller names no limit.
pub const DEFAULT_SNAPSHOT_TRADES: u32 = 50;

/// Unified exchange client that wraps per-capability trait objects.
///
/// Created by [`super::VenueRegistry::create_exchange_client`]. Provides
/// capability discovery and a convenience `fetch_market_snapshot` that
/// fetches all available data in parallel.
pub struct ExchangeClient {
    venue_id: String,
    venue_name: String,
    descriptor: VenueDescriptor,
    order_book: Option<Box<dyn OrderBookClient>>,
    ticker: Option<Box<dyn TickerClient>>,
    trade_history: Option<Box<dyn TradeHistoryClient>>,
    ohlc: Option<Box<dyn OhlcClient>>,
    assets: Option<ConfigurableExchangeClient>,
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
            Some(Box::new(client.clone()))
        } else {
            None
        };
        let assets = desc.capabilities.assets.is_some().then(|| client.clone());
        let ohlc: Option<Box<dyn OhlcClient>> = if desc.has_ohlc() {
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
            ohlc,
            assets,
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

    /// The venue's symbol for a pair; a pair with no quote uses the venue's
    /// default quote.
    pub fn venue_symbol(&self, pair: &super::pair::Pair) -> String {
        self.descriptor
            .format_pair(&pair.base, pair.quote.as_deref())
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

    /// Whether this client supports OHLC / kline data.
    pub fn has_ohlc(&self) -> bool {
        self.ohlc.is_some()
    }

    // =========================================================================
    // Individual capability methods
    // =========================================================================

    /// Fetch order book (if supported).
    pub async fn fetch_order_book(&self, pair: &str) -> Result<OrderBook> {
        self.order_book
            .as_ref()
            .ok_or_else(|| venue_error(&self.venue_id, "does not support order book"))?
            .fetch_order_book(pair)
            .await
    }

    /// Fetch ticker (if supported).
    pub async fn fetch_ticker(&self, pair: &str) -> Result<Ticker> {
        self.ticker
            .as_ref()
            .ok_or_else(|| venue_error(&self.venue_id, "does not support ticker"))?
            .fetch_ticker(pair)
            .await
    }

    /// Fetch recent trades (if supported).
    pub async fn fetch_recent_trades(&self, pair: &str, limit: u32) -> Result<Vec<Trade>> {
        self.trade_history
            .as_ref()
            .ok_or_else(|| venue_error(&self.venue_id, "does not support trades"))?
            .fetch_recent_trades(pair, limit)
            .await
    }

    /// The venue's coin list, or `None` when the venue has no `assets`
    /// capability.
    pub async fn fetch_assets(&self) -> Option<Result<Vec<super::assets::AssetInfo>>> {
        Some(self.assets.as_ref()?.fetch_assets().await)
    }

    /// The token lines for `pair` from the venue's coin list, or `None`
    /// when the venue has no `assets` capability.
    pub async fn fetch_token_addresses(
        &self,
        pair: &super::pair::Pair,
    ) -> Option<Result<Vec<super::assets::TokenAddress>>> {
        let client = self.assets.as_ref()?;
        let quote = pair
            .quote
            .clone()
            .unwrap_or_else(|| self.descriptor.symbol.default_quote.clone());
        Some(
            client
                .fetch_assets()
                .await
                .map(|assets| super::assets::token_addresses(&pair.base, &quote, &assets)),
        )
    }

    /// Fetch OHLC candlesticks (if supported).
    pub async fn fetch_ohlc(&self, pair: &str, interval: &str, limit: u32) -> Result<Vec<Candle>> {
        self.ohlc
            .as_ref()
            .ok_or_else(|| venue_error(&self.venue_id, "does not support OHLC"))?
            .fetch_ohlc(pair, interval, limit)
            .await
    }

    // =========================================================================
    // Combined fetch
    // =========================================================================

    /// Fetch all available market data for a pair in one call, with up to
    /// `trades_limit` recent trades.
    ///
    /// Each capability is fetched independently; failures in one capability
    /// do not prevent others from succeeding (they just produce `None`).
    pub async fn fetch_market_snapshot(&self, pair: &str, trades_limit: u32) -> MarketSnapshot {
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
            self.fetch_recent_trades(pair, trades_limit).await.ok()
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
            .field("has_ohlc", &self.has_ohlc())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::descriptor::{CapabilitySet, SymbolCase, SymbolConfig, VenueDescriptor};
    use crate::market::pair::Pair;
    use crate::market::registry::VenueRegistry;
    use std::collections::HashMap;

    fn make_empty_descriptor() -> VenueDescriptor {
        VenueDescriptor {
            id: "empty".to_string(),
            name: "Empty".to_string(),
            base_url: "https://example.com".to_string(),
            timeout_secs: Some(5),
            rate_limit_per_sec: None,
            symbol: SymbolConfig {
                template: "{base}{quote}".to_string(),
                default_quote: "USDT".to_string(),
                case: SymbolCase::Upper,
            },
            headers: HashMap::new(),
            error_envelope: None,
            capabilities: CapabilitySet::default(),
        }
    }

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
        assert_eq!(client.venue_symbol(&Pair::parse("ETH_USD")), "ETHUSD");
        assert_eq!(client.venue_symbol(&Pair::parse("BTC")), "BTCUSDT");
    }

    /// The audit input: the CLI and the web both send `DAIUSDC`, not
    /// `DAI_USDCUSDT`.
    #[test]
    fn test_dai_usdc_on_binance_is_daiusdc() {
        let registry = VenueRegistry::default();
        let client = ExchangeClient::from_descriptor(registry.get("binance").unwrap());
        assert_eq!(client.venue_symbol(&Pair::parse("DAI_USDC")), "DAIUSDC");
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

    #[test]
    fn test_exchange_client_has_ohlc_for_binance() {
        let registry = VenueRegistry::default();
        let desc = registry.get("binance").unwrap();
        let client = ExchangeClient::from_descriptor(desc);
        assert!(client.has_ohlc());
    }

    #[test]
    fn test_empty_descriptor_has_no_capabilities() {
        let desc = make_empty_descriptor();
        let client = ExchangeClient::from_descriptor(&desc);

        assert!(!client.has_order_book());
        assert!(!client.has_ticker());
        assert!(!client.has_trade_history());
    }

    #[tokio::test]
    async fn test_fetch_order_book_without_capability_returns_error() {
        let desc = make_empty_descriptor();
        let client = ExchangeClient::from_descriptor(&desc);

        let err = client.fetch_order_book("BTCUSDT").await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("(empty)"));
        assert!(msg.contains("does not support order book"));
    }

    #[tokio::test]
    async fn test_fetch_ticker_without_capability_returns_error() {
        let desc = make_empty_descriptor();
        let client = ExchangeClient::from_descriptor(&desc);

        let err = client.fetch_ticker("BTCUSDT").await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("(empty)"));
        assert!(msg.contains("does not support ticker"));
    }

    #[tokio::test]
    async fn test_fetch_ohlc_without_capability_returns_error() {
        let desc = make_empty_descriptor();
        let client = ExchangeClient::from_descriptor(&desc);

        let err = client.fetch_ohlc("BTCUSDT", "1h", 100).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("(empty)"));
        assert!(msg.contains("does not support OHLC"));
    }

    #[tokio::test]
    async fn test_fetch_recent_trades_without_capability_returns_error() {
        let desc = make_empty_descriptor();
        let client = ExchangeClient::from_descriptor(&desc);

        let err = client.fetch_recent_trades("BTCUSDT", 50).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("(empty)"));
        assert!(msg.contains("does not support trades"));
    }

    #[tokio::test]
    async fn test_fetch_market_snapshot_empty_descriptor_returns_all_none() {
        let desc = make_empty_descriptor();
        let client = ExchangeClient::from_descriptor(&desc);

        let snapshot = client
            .fetch_market_snapshot("BTCUSDT", DEFAULT_SNAPSHOT_TRADES)
            .await;

        assert!(snapshot.order_book.is_none());
        assert!(snapshot.ticker.is_none());
        assert!(snapshot.recent_trades.is_none());
    }
}
