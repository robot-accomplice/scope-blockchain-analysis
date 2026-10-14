//! Core market data types.
//!
//! Defines order book levels, full order books, execution estimates, and
//! the newer trade/ticker/snapshot types used by the exchange system.

use std::ops::RangeBounds;

// =============================================================================
// Order Book
// =============================================================================

/// A single price level in the order book.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderBookLevel {
    /// Price (e.g., 1.0001)
    pub price: f64,
    /// Quantity in base asset (e.g., DAI)
    pub quantity: f64,
}

impl OrderBookLevel {
    /// A level is valid when it has a positive price and quantity. Every
    /// price, spread and level count reads only valid levels.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.price > 0.0 && self.quantity > 0.0
    }

    /// Value in quote asset (price × quantity, e.g., USDT).
    #[inline]
    pub fn value(&self) -> f64 {
        self.price * self.quantity
    }
}

/// Where an order book came from. Health checks and displays depend on it:
/// a synthetic book has no real level count or spread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BookSource {
    /// A real level-2 book from an exchange API.
    #[default]
    Exchange,
    /// Built from a DEX pool's reserves on the constant-product curve
    /// (x·y=k), one level per fixed price step.
    SyntheticAmm,
    /// A rough estimate from total DEX liquidity, used when the pool
    /// reserves are not reported. Its depth is not depth near the price.
    SyntheticEstimate,
}

impl BookSource {
    /// Short label for text and markdown output.
    pub fn label(self) -> &'static str {
        match self {
            BookSource::Exchange => "exchange order book",
            BookSource::SyntheticAmm => "synthetic (AMM curve x·y=k)",
            BookSource::SyntheticEstimate => "synthetic estimate (pool reserves not reported)",
        }
    }
}

/// One side of an order book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookSide {
    Bid,
    Ask,
}

/// Full order book snapshot with bids and asks.
#[derive(Debug, Clone)]
pub struct OrderBook {
    /// Trading pair label (e.g., "DAI/USDT").
    pub pair: String,
    /// Where the book came from.
    pub source: BookSource,
    /// Bids sorted by price descending (best bid first).
    pub bids: Vec<OrderBookLevel>,
    /// Asks sorted by price ascending (best ask first).
    pub asks: Vec<OrderBookLevel>,
}

impl OrderBook {
    /// The one JSON view of a book (web API and web monitor). Prices,
    /// spread and depths come from this book's own methods. `max_levels`
    /// caps the levels listed per side; the depths count all levels.
    pub fn to_json(&self, max_levels: Option<usize>) -> serde_json::Value {
        let levels = |side: &[OrderBookLevel]| {
            side.iter()
                .take(max_levels.unwrap_or(usize::MAX))
                .map(|l| serde_json::json!({"price": l.price, "quantity": l.quantity, "value": l.value()}))
                .collect::<Vec<_>>()
        };
        serde_json::json!({
            "pair": self.pair,
            "best_bid": self.best_bid(),
            "best_ask": self.best_ask(),
            "mid_price": self.mid_price(),
            "spread": self.spread(),
            "total_bid_depth": self.total_depth(BookSide::Bid),
            "total_ask_depth": self.total_depth(BookSide::Ask),
            "bids": levels(&self.bids),
            "asks": levels(&self.asks),
        })
    }

    /// Best valid bid price, or None when no bid is valid.
    pub fn best_bid(&self) -> Option<f64> {
        self.bids.iter().find(|l| l.is_valid()).map(|l| l.price)
    }

    /// Best valid ask price, or None when no ask is valid.
    pub fn best_ask(&self) -> Option<f64> {
        self.asks.iter().find(|l| l.is_valid()).map(|l| l.price)
    }

    /// Mid price between the best valid bid and ask.
    pub fn mid_price(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some((bid + ask) / 2.0),
            _ => None,
        }
    }

    /// Spread (ask - bid) of the best valid levels. `None` for a synthetic
    /// AMM book: its real spread is the pool fee, which is not reported.
    pub fn spread(&self) -> Option<f64> {
        if self.source == BookSource::SyntheticAmm {
            return None;
        }
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some(ask - bid),
            _ => None,
        }
    }

    /// The spread in percent of the mid of the same two levels.
    pub fn spread_pct(&self) -> Option<f64> {
        self.spread()
            .zip(self.mid_price())
            .map(|(spread, mid)| spread / mid * PERCENT_PER_RATIO)
    }

    /// The levels of one side, best first.
    pub fn levels(&self, side: BookSide) -> &[OrderBookLevel] {
        match side {
            BookSide::Bid => &self.bids,
            BookSide::Ask => &self.asks,
        }
    }

    /// The valid levels of one side whose price is in `band`, best first.
    /// This is the one eligible-levels set: depth, level counts and the
    /// peg checks all read it.
    pub fn valid_levels_in_band(
        &self,
        side: BookSide,
        band: impl RangeBounds<f64>,
    ) -> impl Iterator<Item = &OrderBookLevel> {
        self.levels(side)
            .iter()
            .filter(move |l| l.is_valid() && band.contains(&l.price))
    }

    /// Quote depth (sum of price × quantity) of the eligible levels of one
    /// side in `band`. This is the one depth sum; every other depth is a call
    /// to it with a band.
    pub fn depth_in_band(&self, side: BookSide, band: impl RangeBounds<f64>) -> f64 {
        self.valid_levels_in_band(side, band)
            .map(OrderBookLevel::value)
            .sum()
    }

    /// Quote depth of the whole side, at any price.
    pub fn total_depth(&self, side: BookSide) -> f64 {
        self.depth_in_band(side, ..)
    }

    /// Quote depth of the first `n` eligible levels of one side in `band`.
    pub fn top_depth(&self, side: BookSide, band: impl RangeBounds<f64>, n: usize) -> f64 {
        self.valid_levels_in_band(side, band)
            .take(n)
            .map(OrderBookLevel::value)
            .sum()
    }

    /// Estimate slippage for buying a given USDT notional by walking the ask
    /// side. See [`Self::estimate_execution`].
    pub fn estimate_buy_execution(&self, notional_usdt: f64) -> Option<ExecutionEstimate> {
        self.estimate_execution(ExecutionSide::Buy, notional_usdt)
    }

    /// Estimate slippage for selling (hitting bids) a given USDT notional.
    pub fn estimate_sell_execution(&self, notional_usdt: f64) -> Option<ExecutionEstimate> {
        self.estimate_execution(ExecutionSide::Sell, notional_usdt)
    }

    /// Slippage of filling `notional_usdt` against the opposite side, against
    /// the mid. `None` when the book has no mid price (a side has no valid
    /// level); `fillable` is false when the side cannot fill the notional.
    pub fn estimate_execution(
        &self,
        side: ExecutionSide,
        notional_usdt: f64,
    ) -> Option<ExecutionEstimate> {
        let mid = self.mid_price()?;
        let (levels, direction) = match side {
            ExecutionSide::Buy => (&self.asks, 1.0),
            ExecutionSide::Sell => (&self.bids, -1.0),
        };
        let (vwap, fillable) = walk_levels(levels, notional_usdt, mid);
        Some(ExecutionEstimate {
            notional_usdt,
            side,
            vwap,
            // Paying more than the mid (buy) or receiving less (sell) is positive.
            slippage_bps: direction * (vwap - mid) / mid * BPS_PER_RATIO,
            fillable,
        })
    }
}

/// Percent in a ratio of 1.
pub const PERCENT_PER_RATIO: f64 = 100.0;

/// Basis points in one percent.
pub const BPS_PER_PERCENT: f64 = 100.0;

/// Basis points in a ratio of 1.
pub const BPS_PER_RATIO: f64 = BPS_PER_PERCENT * PERCENT_PER_RATIO;

/// Walks the valid levels (best first) until `notional` of quote value is
/// taken. Returns the volume-weighted price (the mid when nothing fills) and
/// whether the notional was filled.
fn walk_levels(levels: &[OrderBookLevel], notional: f64, mid: f64) -> (f64, bool) {
    let mut remaining = notional;
    let mut filled_value = 0.0;
    let mut filled_qty = 0.0;
    for level in levels.iter().filter(|l| l.is_valid()) {
        if remaining <= 0.0 {
            break;
        }
        let take_value = level.value().min(remaining);
        filled_value += take_value;
        filled_qty += take_value / level.price;
        remaining -= take_value;
    }
    let vwap = if filled_qty > 0.0 {
        filled_value / filled_qty
    } else {
        mid
    };
    (vwap, remaining <= 0.01)
}

/// Side of execution (buy = hit asks, sell = hit bids).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExecutionSide {
    Buy,
    Sell,
}

/// Result of execution simulation for a given notional size.
#[derive(Debug, Clone)]
pub struct ExecutionEstimate {
    pub notional_usdt: f64,
    pub side: ExecutionSide,
    pub vwap: f64,
    pub slippage_bps: f64,
    pub fillable: bool,
}

/// Outcome of a single health check.
///
/// Serializes as `{"status": "pass" | "fail" | "warn" | "n/a", "message": "…"}`.
/// JSON consumers must handle `warn` and `n/a`.
#[derive(Debug, Clone, PartialEq)]
pub enum HealthCheck {
    Pass(String),
    Fail(String),
    /// The check does not apply to this book; the message says why.
    /// It does not count against `healthy`.
    NotApplicable(String),
    /// The value is outside its target, but not a failure. Shown to the
    /// user; it does not count against `healthy`.
    Warn(String),
}

impl HealthCheck {
    /// `pass`, `fail`, `warn` or `n/a`.
    pub fn status(&self) -> &'static str {
        match self {
            HealthCheck::Pass(_) => "pass",
            HealthCheck::Fail(_) => "fail",
            HealthCheck::Warn(_) => "warn",
            HealthCheck::NotApplicable(_) => "n/a",
        }
    }

    /// The check's message.
    pub fn message(&self) -> &str {
        match self {
            HealthCheck::Pass(m)
            | HealthCheck::Fail(m)
            | HealthCheck::Warn(m)
            | HealthCheck::NotApplicable(m) => m,
        }
    }
}

impl HealthCheck {
    /// The plain icon for markdown and plain-text reports: ✓, ✗, ! or –.
    pub fn icon(&self) -> &'static str {
        match self {
            HealthCheck::Pass(_) => "✓",
            HealthCheck::Fail(_) => "✗",
            HealthCheck::Warn(_) => "!",
            HealthCheck::NotApplicable(_) => "–",
        }
    }

    /// One text row for terminal reports (✓, ✗, ! or –).
    pub fn render_text(&self) -> String {
        use crate::display::terminal as t;
        match self {
            HealthCheck::Pass(m) => t::check_pass(m),
            HealthCheck::Fail(m) => t::check_fail(m),
            HealthCheck::Warn(m) => t::check_warn(m),
            HealthCheck::NotApplicable(m) => t::check_na(m),
        }
    }
}

impl serde::Serialize for HealthCheck {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("HealthCheck", 2)?;
        st.serialize_field("status", self.status())?;
        st.serialize_field("message", self.message())?;
        st.end()
    }
}

// =============================================================================
// Trade & Ticker Types
// =============================================================================

/// Side of a trade. JSON writes `buy` or `sell`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TradeSide {
    Buy,
    Sell,
}

/// A single trade from the recent trades endpoint. Its JSON form is the one
/// the CLI and the web send.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Trade {
    /// Trade price in quote currency.
    pub price: f64,
    /// Quantity in base currency.
    pub quantity: f64,
    /// Quote quantity (price × quantity), if provided.
    pub quote_quantity: Option<f64>,
    /// Timestamp in milliseconds since epoch.
    pub timestamp_ms: u64,
    /// Whether this was a buy or sell from the taker's perspective.
    pub side: TradeSide,
    /// Trade ID, if available.
    pub id: Option<String>,
}

/// 24-hour ticker / market stats. Its JSON form is the one the web sends.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Ticker {
    /// Pair label (e.g., "BTC/USDT").
    pub pair: String,
    /// Last traded price.
    pub last_price: Option<f64>,
    /// 24h high.
    pub high_24h: Option<f64>,
    /// 24h low.
    pub low_24h: Option<f64>,
    /// 24h base volume.
    pub volume_24h: Option<f64>,
    /// 24h quote volume.
    pub quote_volume_24h: Option<f64>,
    /// 24h price change in percent (0.5 = +0.5%), when the venue gives it.
    pub price_change_pct_24h: Option<f64>,
    /// Best bid (if included in ticker).
    pub best_bid: Option<f64>,
    /// Best ask (if included in ticker).
    pub best_ask: Option<f64>,
}

impl Ticker {
    /// 24h volume in quote terms (e.g. USDT).
    ///
    /// Uses the reported quote volume. When only base volume is reported,
    /// converts it with `mid`. Base volume alone is not a quote amount.
    pub fn quote_volume(&self, mid: Option<f64>) -> Option<f64> {
        self.quote_volume_24h
            .or_else(|| Some(self.volume_24h? * mid?))
    }

    /// 24h volume in USD: the quote volume at the last price, when the
    /// quote is USD or a USD stablecoin; `None` for any other quote (M1).
    pub fn usd_volume(&self) -> Option<f64> {
        let quote = super::Pair::parse(&self.pair).quote?;
        let usd = quote.eq_ignore_ascii_case("USD") || crate::tokens::is_stablecoin_symbol(&quote);
        if usd {
            self.quote_volume(self.last_price)
        } else {
            None
        }
    }
}

/// Aggregated market snapshot combining all available data for a pair.
#[derive(Debug, Clone)]
pub struct MarketSnapshot {
    pub order_book: Option<OrderBook>,
    pub ticker: Option<Ticker>,
    pub recent_trades: Option<Vec<Trade>>,
}

// =============================================================================
// OHLC / Candlestick
// =============================================================================

/// A single OHLC candlestick bar. Its JSON form is the one the CLI and the
/// web send.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Candle {
    /// Candle open time in milliseconds since epoch.
    pub open_time: u64,
    /// Opening price.
    pub open: f64,
    /// Highest price during the interval.
    pub high: f64,
    /// Lowest price during the interval.
    pub low: f64,
    /// Closing price.
    pub close: f64,
    /// Base-asset volume during the interval.
    pub volume: f64,
    /// Candle close time in milliseconds since epoch.
    pub close_time: u64,
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {

    fn ticker(base: Option<f64>, quote: Option<f64>) -> Ticker {
        Ticker {
            pair: "ETH/USDT".into(),
            last_price: Some(2_000.0),
            high_24h: None,
            low_24h: None,
            volume_24h: base,
            quote_volume_24h: quote,
            price_change_pct_24h: None,
            best_bid: None,
            best_ask: None,
        }
    }

    #[test]
    fn test_quote_volume_prefers_the_reported_quote_volume() {
        assert_eq!(
            ticker(Some(10.0), Some(25_000.0)).quote_volume(Some(2_000.0)),
            Some(25_000.0)
        );
    }

    #[test]
    fn test_quote_volume_converts_base_volume_with_the_mid_price() {
        // 10 ETH at 2,000 is 20,000 USDT, not 10.
        assert_eq!(
            ticker(Some(10.0), None).quote_volume(Some(2_000.0)),
            Some(20_000.0)
        );
        assert_eq!(ticker(Some(10.0), None).quote_volume(None), None);
    }
    use super::*;

    #[test]
    fn test_order_book_level_value() {
        let level = OrderBookLevel {
            price: 1.0002,
            quantity: 100.0,
        };
        assert!((level.value() - 100.02).abs() < 1e-6);
    }

    #[test]
    fn test_order_book_empty() {
        let book = OrderBook {
            pair: "DAI/USDT".to_string(),
            source: crate::market::types::BookSource::Exchange,
            bids: vec![],
            asks: vec![],
        };
        assert!(book.best_bid().is_none());
        assert!(book.best_ask().is_none());
        assert!(book.mid_price().is_none());
        assert_eq!(book.total_depth(BookSide::Bid), 0.0);
        assert_eq!(book.total_depth(BookSide::Ask), 0.0);
    }

    #[test]
    fn test_order_book_with_levels() {
        let book = OrderBook {
            pair: "DAI/USDT".to_string(),
            source: crate::market::types::BookSource::Exchange,
            bids: vec![
                OrderBookLevel {
                    price: 0.9998,
                    quantity: 100.0,
                },
                OrderBookLevel {
                    price: 0.9997,
                    quantity: 50.0,
                },
            ],
            asks: vec![
                OrderBookLevel {
                    price: 1.0001,
                    quantity: 200.0,
                },
                OrderBookLevel {
                    price: 1.0002,
                    quantity: 150.0,
                },
            ],
        };
        assert_eq!(book.best_bid(), Some(0.9998));
        assert_eq!(book.best_ask(), Some(1.0001));
        assert_eq!(book.mid_price(), Some(0.99995));
        assert!((book.spread().unwrap() - 0.0003).abs() < 1e-10);
        assert!((book.total_depth(BookSide::Bid) - 99.98 - 49.985).abs() < 0.01);
        assert!((book.total_depth(BookSide::Ask) - 200.02 - 150.03).abs() < 0.01);
    }

    #[test]
    fn test_candle_construction() {
        let candle = Candle {
            open_time: 1700000000000,
            open: 100.0,
            high: 105.0,
            low: 95.0,
            close: 102.0,
            volume: 1_000_000.0,
            close_time: 1700003600000,
        };
        assert_eq!(candle.open_time, 1700000000000);
        assert_eq!(candle.open, 100.0);
        assert_eq!(candle.high, 105.0);
        assert_eq!(candle.low, 95.0);
        assert_eq!(candle.close, 102.0);
        assert_eq!(candle.volume, 1_000_000.0);
        assert_eq!(candle.close_time, 1700003600000);
    }

    #[test]
    fn test_candle_partial_eq() {
        let c1 = Candle {
            open_time: 1000,
            open: 1.0,
            high: 1.1,
            low: 0.9,
            close: 1.0,
            volume: 100.0,
            close_time: 2000,
        };
        let c2 = c1.clone();
        assert_eq!(c1, c2);
    }

    #[test]
    fn test_trade_side_equality() {
        assert_eq!(TradeSide::Buy, TradeSide::Buy);
        assert_ne!(TradeSide::Buy, TradeSide::Sell);
    }

    #[test]
    fn test_trade_construction() {
        let trade = Trade {
            price: 42_000.0,
            quantity: 1.5,
            quote_quantity: Some(63_000.0),
            timestamp_ms: 1700000000000,
            side: TradeSide::Buy,
            id: Some("12345".to_string()),
        };
        assert_eq!(trade.price, 42_000.0);
        assert_eq!(trade.quantity, 1.5);
        assert_eq!(trade.quote_quantity, Some(63_000.0));
        assert_eq!(trade.side, TradeSide::Buy);
        assert_eq!(trade.id, Some("12345".to_string()));
    }

    #[test]
    fn test_trade_optional_fields() {
        let trade = Trade {
            price: 1.0001,
            quantity: 100.0,
            quote_quantity: None,
            timestamp_ms: 1700000000000,
            side: TradeSide::Sell,
            id: None,
        };
        assert!(trade.quote_quantity.is_none());
        assert!(trade.id.is_none());
    }

    #[test]
    fn test_ticker_construction() {
        let ticker = Ticker {
            pair: "BTC/USDT".to_string(),
            last_price: Some(42_000.0),
            high_24h: Some(43_000.0),
            low_24h: Some(41_000.0),
            volume_24h: Some(50_000.0),
            quote_volume_24h: Some(2_100_000_000.0),
            price_change_pct_24h: None,
            best_bid: Some(41_999.0),
            best_ask: Some(42_001.0),
        };
        assert_eq!(ticker.pair, "BTC/USDT");
        assert_eq!(ticker.last_price, Some(42_000.0));
        assert_eq!(ticker.high_24h, Some(43_000.0));
    }

    #[test]
    fn test_ticker_all_none() {
        let ticker = Ticker {
            pair: "UNKNOWN/USD".to_string(),
            last_price: None,
            high_24h: None,
            low_24h: None,
            volume_24h: None,
            quote_volume_24h: None,
            price_change_pct_24h: None,
            best_bid: None,
            best_ask: None,
        };
        assert!(ticker.last_price.is_none());
        assert!(ticker.volume_24h.is_none());
    }

    #[test]
    fn test_market_snapshot_full() {
        let snapshot = MarketSnapshot {
            order_book: Some(OrderBook {
                pair: "BTC/USDT".to_string(),
                source: crate::market::types::BookSource::Exchange,
                bids: vec![OrderBookLevel {
                    price: 42_000.0,
                    quantity: 1.0,
                }],
                asks: vec![OrderBookLevel {
                    price: 42_001.0,
                    quantity: 1.0,
                }],
            }),
            ticker: Some(Ticker {
                pair: "BTC/USDT".to_string(),
                last_price: Some(42_000.0),
                high_24h: None,
                low_24h: None,
                volume_24h: None,
                quote_volume_24h: None,
                price_change_pct_24h: None,
                best_bid: None,
                best_ask: None,
            }),
            recent_trades: Some(vec![Trade {
                price: 42_000.0,
                quantity: 0.5,
                quote_quantity: None,
                timestamp_ms: 1700000000000,
                side: TradeSide::Buy,
                id: None,
            }]),
        };
        assert!(snapshot.order_book.is_some());
        assert!(snapshot.ticker.is_some());
        assert_eq!(snapshot.recent_trades.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_market_snapshot_empty() {
        let snapshot = MarketSnapshot {
            order_book: None,
            ticker: None,
            recent_trades: None,
        };
        assert!(snapshot.order_book.is_none());
        assert!(snapshot.ticker.is_none());
        assert!(snapshot.recent_trades.is_none());
    }

    // ================================================================
    // Execution estimate edge-case tests (zero-price branches)
    // ================================================================

    #[test]
    fn test_estimate_buy_zero_mid_price() {
        // Bids and asks both at price 0 -> mid_price = 0 -> returns None
        let book = OrderBook {
            pair: "X/Y".to_string(),
            source: crate::market::types::BookSource::Exchange,
            bids: vec![OrderBookLevel {
                price: 0.0,
                quantity: 100.0,
            }],
            asks: vec![OrderBookLevel {
                price: 0.0,
                quantity: 100.0,
            }],
        };
        assert!(book.estimate_buy_execution(1000.0).is_none());
    }

    #[test]
    fn test_estimate_sell_zero_mid_price() {
        let book = OrderBook {
            pair: "X/Y".to_string(),
            source: crate::market::types::BookSource::Exchange,
            bids: vec![OrderBookLevel {
                price: 0.0,
                quantity: 100.0,
            }],
            asks: vec![OrderBookLevel {
                price: 0.0,
                quantity: 100.0,
            }],
        };
        assert!(book.estimate_sell_execution(1000.0).is_none());
    }

    #[test]
    fn test_estimate_buy_zero_price_level() {
        // Valid mid price but one ask level has price 0 -> take_qty branch = 0.0
        let book = OrderBook {
            pair: "X/Y".to_string(),
            source: crate::market::types::BookSource::Exchange,
            bids: vec![OrderBookLevel {
                price: 1.0,
                quantity: 100.0,
            }],
            asks: vec![
                OrderBookLevel {
                    price: 0.0,
                    quantity: 100.0,
                },
                OrderBookLevel {
                    price: 1.001,
                    quantity: 10000.0,
                },
            ],
        };
        let est = book.estimate_buy_execution(50.0).unwrap();
        assert!(est.fillable);
    }

    #[test]
    fn test_estimate_sell_zero_price_level() {
        // Valid mid price but one bid level has price 0
        let book = OrderBook {
            pair: "X/Y".to_string(),
            source: crate::market::types::BookSource::Exchange,
            bids: vec![
                OrderBookLevel {
                    price: 0.0,
                    quantity: 100.0,
                },
                OrderBookLevel {
                    price: 0.999,
                    quantity: 10000.0,
                },
            ],
            asks: vec![OrderBookLevel {
                price: 1.0,
                quantity: 100.0,
            }],
        };
        let est = book.estimate_sell_execution(50.0).unwrap();
        assert!(est.fillable);
    }

    #[test]
    fn test_estimate_buy_zero_filled_qty() {
        // The only ask level is invalid, so there is no mid price to estimate
        // from (the same as an empty ask side).
        let book = OrderBook {
            pair: "X/Y".to_string(),
            source: crate::market::types::BookSource::Exchange,
            bids: vec![OrderBookLevel {
                price: 1.0,
                quantity: 100.0,
            }],
            asks: vec![OrderBookLevel {
                price: 0.0,
                quantity: 0.0,
            }],
        };
        assert!(book.estimate_buy_execution(50.0).is_none());
    }

    #[test]
    fn test_estimate_sell_zero_filled_qty() {
        // The only bid level is invalid, so there is no mid price.
        let book = OrderBook {
            pair: "X/Y".to_string(),
            source: crate::market::types::BookSource::Exchange,
            bids: vec![OrderBookLevel {
                price: 0.0,
                quantity: 0.0,
            }],
            asks: vec![OrderBookLevel {
                price: 1.0,
                quantity: 100.0,
            }],
        };
        assert!(book.estimate_sell_execution(50.0).is_none());
    }

    fn level(price: f64, quantity: f64) -> OrderBookLevel {
        OrderBookLevel { price, quantity }
    }

    #[test]
    fn test_prices_use_the_best_valid_level() {
        // The top level of each side has zero quantity: not a real order.
        let book = OrderBook {
            pair: "X/Y".to_string(),
            source: BookSource::Exchange,
            bids: vec![level(1.0, 0.0), level(0.9990, 10.0)],
            asks: vec![level(1.0001, 0.0), level(1.0010, 10.0)],
        };
        assert_eq!(book.best_bid(), Some(0.9990));
        assert_eq!(book.best_ask(), Some(1.0010));
        let mid = book.mid_price().unwrap();
        assert!((mid - 1.0).abs() < 1e-12);
        let spread = book.spread().unwrap();
        assert!((spread - (1.0010 - 0.9990)).abs() < 1e-12);
        assert!((book.spread_pct().unwrap() - spread / mid * 100.0).abs() < 1e-12);
    }

    #[test]
    fn test_synthetic_amm_book_has_prices_but_no_spread() {
        let book = OrderBook {
            pair: "X/Y".to_string(),
            source: BookSource::SyntheticAmm,
            bids: vec![level(0.999, 10.0)],
            asks: vec![level(1.001, 10.0)],
        };
        assert!(book.mid_price().is_some());
        assert_eq!(book.spread(), None);
        assert_eq!(book.spread_pct(), None);
    }

    #[test]
    fn test_depth_in_band_counts_valid_levels_in_the_price_range() {
        let book = OrderBook {
            pair: "X/Y".to_string(),
            source: BookSource::Exchange,
            bids: vec![level(1.0, 0.0), level(0.99, 10.0), level(0.5, 10.0)],
            asks: vec![level(1.01, 10.0), level(2.0, 10.0)],
        };
        // The zero-quantity level adds nothing; the 0.5 bid is out of band.
        let in_band = book.depth_in_band(BookSide::Bid, 0.9..=1.1);
        assert!((in_band - 9.9).abs() < 1e-12);
        let whole = book.total_depth(BookSide::Bid);
        assert!((whole - 14.9).abs() < 1e-12);
        assert!((book.total_depth(BookSide::Ask) - 30.1).abs() < 1e-12);
        assert!((book.top_depth(BookSide::Bid, .., 1) - 9.9).abs() < 1e-12);
    }

    #[test]
    fn test_no_valid_level_on_a_side_gives_no_execution_estimate() {
        // One rule for an empty side and a side of invalid levels: no mid,
        // so no estimate.
        let empty = OrderBook {
            pair: "X/Y".to_string(),
            source: BookSource::Exchange,
            bids: vec![level(1.0, 10.0)],
            asks: vec![],
        };
        let invalid = OrderBook {
            asks: vec![level(1.001, 0.0)],
            ..empty.clone()
        };
        for book in [empty, invalid] {
            assert!(book.estimate_buy_execution(5.0).is_none());
            assert!(book.estimate_sell_execution(5.0).is_none());
        }
    }

    #[test]
    fn test_execution_estimate_skips_invalid_levels_and_reports_unfillable() {
        let book = OrderBook {
            pair: "X/Y".to_string(),
            source: BookSource::Exchange,
            bids: vec![level(1.0, 100.0)],
            asks: vec![level(1.0, 0.0), level(1.002, 10.0)],
        };
        let est = book.estimate_buy_execution(1000.0).unwrap();
        assert!(!est.fillable);
        assert!((est.vwap - 1.002).abs() < 1e-12);
    }
}
