//! Core market data types.
//!
//! Defines order book levels, full order books, execution estimates, and
//! the newer trade/ticker/snapshot types used by the exchange system.

// =============================================================================
// Order Book
// =============================================================================

/// A single price level in the order book.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderBookLevel {
    /// Price (e.g., 1.0001)
    pub price: f64,
    /// Quantity in base asset (e.g., PUSD)
    pub quantity: f64,
}

impl OrderBookLevel {
    /// Value in quote asset (price × quantity, e.g., USDT).
    #[inline]
    pub fn value(&self) -> f64 {
        self.price * self.quantity
    }
}

/// Full order book snapshot with bids and asks.
#[derive(Debug, Clone)]
pub struct OrderBook {
    /// Trading pair label (e.g., "PUSD/USDT").
    pub pair: String,
    /// Bids sorted by price descending (best bid first).
    pub bids: Vec<OrderBookLevel>,
    /// Asks sorted by price ascending (best ask first).
    pub asks: Vec<OrderBookLevel>,
}

impl OrderBook {
    /// Best bid price, or None if empty.
    pub fn best_bid(&self) -> Option<f64> {
        self.bids.first().map(|l| l.price)
    }

    /// Best ask price, or None if empty.
    pub fn best_ask(&self) -> Option<f64> {
        self.asks.first().map(|l| l.price)
    }

    /// Mid price between best bid and ask.
    pub fn mid_price(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some((bid + ask) / 2.0),
            _ => None,
        }
    }

    /// Spread (ask - bid).
    pub fn spread(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some(ask - bid),
            _ => None,
        }
    }

    /// Total bid depth in quote terms (sum of price × quantity).
    pub fn bid_depth(&self) -> f64 {
        self.bids.iter().map(OrderBookLevel::value).sum()
    }

    /// Total ask depth in quote terms.
    pub fn ask_depth(&self) -> f64 {
        self.asks.iter().map(OrderBookLevel::value).sum()
    }

    /// Estimate slippage for buying a given USDT notional by walking the ask side.
    /// Returns (vwap, slippage_bps) if fillable, or None if insufficient liquidity.
    pub fn estimate_buy_execution(&self, notional_usdt: f64) -> Option<ExecutionEstimate> {
        let mid = self.mid_price()?;
        if mid <= 0.0 {
            return None;
        }
        let mut remaining = notional_usdt;
        let mut filled_value = 0.0;
        let mut filled_qty = 0.0;
        for level in &self.asks {
            let level_value = level.value();
            if remaining <= 0.0 {
                break;
            }
            let take_value = level_value.min(remaining);
            let take_qty = if level.price > 0.0 {
                take_value / level.price
            } else {
                0.0
            };
            filled_value += take_value;
            filled_qty += take_qty;
            remaining -= take_value;
        }
        let fillable = remaining <= 0.01;
        let vwap = if filled_qty > 0.0 {
            filled_value / filled_qty
        } else {
            mid
        };
        let slippage_bps = (vwap - mid) / mid * 10_000.0;
        Some(ExecutionEstimate {
            notional_usdt,
            side: ExecutionSide::Buy,
            vwap,
            slippage_bps,
            fillable,
        })
    }

    /// Estimate slippage for selling (hitting bids) a given USDT notional.
    pub fn estimate_sell_execution(&self, notional_usdt: f64) -> Option<ExecutionEstimate> {
        let mid = self.mid_price()?;
        if mid <= 0.0 {
            return None;
        }
        let mut remaining = notional_usdt;
        let mut filled_value = 0.0;
        let mut filled_qty = 0.0;
        for level in &self.bids {
            if remaining <= 0.0 {
                break;
            }
            let level_value = level.value();
            let take_value = level_value.min(remaining);
            let take_qty = if level.price > 0.0 {
                take_value / level.price
            } else {
                0.0
            };
            filled_value += take_value;
            filled_qty += take_qty;
            remaining -= take_value;
        }
        let fillable = remaining <= 0.01;
        let vwap = if filled_qty > 0.0 {
            filled_value / filled_qty
        } else {
            mid
        };
        let slippage_bps = (mid - vwap) / mid * 10_000.0;
        Some(ExecutionEstimate {
            notional_usdt,
            side: ExecutionSide::Sell,
            vwap,
            slippage_bps,
            fillable,
        })
    }
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
#[derive(Debug, Clone, PartialEq)]
pub enum HealthCheck {
    Pass(String),
    Fail(String),
}

// =============================================================================
// Trade & Ticker Types
// =============================================================================

/// Side of a trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeSide {
    Buy,
    Sell,
}

/// A single trade from the recent trades endpoint.
#[derive(Debug, Clone)]
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

/// 24-hour ticker / market stats.
#[derive(Debug, Clone)]
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
    /// Best bid (if included in ticker).
    pub best_bid: Option<f64>,
    /// Best ask (if included in ticker).
    pub best_ask: Option<f64>,
}

/// Aggregated market snapshot combining all available data for a pair.
#[derive(Debug, Clone)]
pub struct MarketSnapshot {
    pub order_book: Option<OrderBook>,
    pub ticker: Option<Ticker>,
    pub recent_trades: Option<Vec<Trade>>,
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
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
            pair: "PUSD/USDT".to_string(),
            bids: vec![],
            asks: vec![],
        };
        assert!(book.best_bid().is_none());
        assert!(book.best_ask().is_none());
        assert!(book.mid_price().is_none());
        assert_eq!(book.bid_depth(), 0.0);
        assert_eq!(book.ask_depth(), 0.0);
    }

    #[test]
    fn test_order_book_with_levels() {
        let book = OrderBook {
            pair: "PUSD/USDT".to_string(),
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
        assert!((book.bid_depth() - 99.98 - 49.985).abs() < 0.01);
        assert!((book.ask_depth() - 200.02 - 150.03).abs() < 0.01);
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
}
