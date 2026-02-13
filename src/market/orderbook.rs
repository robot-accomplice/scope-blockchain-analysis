//! Order book fetching and peg/health analysis.
//!
//! Supports configurable exchange APIs and health check thresholds for
//! stablecoin market monitoring.

use crate::error::{Result, ScopeError};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

// =============================================================================
// Types
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
}

/// Health check thresholds for order book validation.
///
/// Default values (min_levels=6, min_depth=3000, peg_range=0.001) originated from
/// the PUSD Hummingbot market-making config. Override via CLI (`--min-levels`,
/// `--min-depth`, `--peg-range`, `--min-bid-ask-ratio`, `--max-bid-ask-ratio`).
#[derive(Debug, Clone)]
pub struct HealthThresholds {
    /// Peg target (e.g., 1.0 for USD stablecoins).
    pub peg_target: f64,
    /// Price range for "near peg" orders (outliers excluded outside peg ± range×5).
    pub peg_range: f64,
    /// Minimum levels per side.
    pub min_levels: usize,
    /// Minimum depth per side in quote terms (e.g., USDT).
    pub min_depth: f64,
    /// Bid/ask ratio below which to warn (bid side thin).
    pub min_bid_ask_ratio: f64,
    /// Bid/ask ratio above which to warn (ask side thin).
    pub max_bid_ask_ratio: f64,
}

impl Default for HealthThresholds {
    fn default() -> Self {
        Self {
            peg_target: 1.0,
            peg_range: 0.001,
            min_levels: 6,
            min_depth: 3000.0,
            min_bid_ask_ratio: 0.2,
            max_bid_ask_ratio: 5.0,
        }
    }
}

/// Outcome of a single health check.
#[derive(Debug, Clone, PartialEq)]
pub enum HealthCheck {
    Pass(String),
    Fail(String),
}

/// Aggregated market summary with order book snapshot and health results.
#[derive(Debug, Clone)]
pub struct MarketSummary {
    /// Pair label (e.g., "PUSD/USDT").
    pub pair: String,
    /// Peg target.
    pub peg_target: f64,
    /// Best bid (raw, no outlier filtering).
    pub best_bid: Option<f64>,
    /// Best ask.
    pub best_ask: Option<f64>,
    /// Mid price.
    pub mid_price: Option<f64>,
    /// Spread.
    pub spread: Option<f64>,
    /// Asks within peg range (for display).
    pub asks: Vec<OrderBookLevel>,
    /// Bids within peg range.
    pub bids: Vec<OrderBookLevel>,
    /// Count of ask levels excluded as outliers.
    pub ask_outliers: usize,
    /// Count of bid levels excluded as outliers.
    pub bid_outliers: usize,
    /// Ask depth (quote) within range.
    pub ask_depth: f64,
    /// Bid depth (quote) within range.
    pub bid_depth: f64,
    /// Health check results.
    pub checks: Vec<HealthCheck>,
    /// Overall healthy (no failures).
    pub healthy: bool,
}

impl MarketSummary {
    /// Build summary from order book with given peg and thresholds.
    pub fn from_order_book(
        book: &OrderBook,
        peg_target: f64,
        thresholds: &HealthThresholds,
    ) -> Self {
        let price_lo = peg_target - thresholds.peg_range * 5.0;
        let price_hi = peg_target + thresholds.peg_range * 5.0;

        let asks: Vec<OrderBookLevel> = book
            .asks
            .iter()
            .filter(|l| l.price <= price_hi)
            .cloned()
            .collect();
        let bids: Vec<OrderBookLevel> = book
            .bids
            .iter()
            .filter(|l| l.price >= price_lo)
            .cloned()
            .collect();

        let ask_outliers = book.asks.len().saturating_sub(asks.len());
        let bid_outliers = book.bids.len().saturating_sub(bids.len());

        let ask_depth: f64 = asks.iter().map(OrderBookLevel::value).sum();
        let bid_depth: f64 = bids.iter().map(OrderBookLevel::value).sum();

        let mut checks = Vec::new();

        // Peg safety: sells below peg
        let below_peg: Vec<_> = asks.iter().filter(|a| a.price < peg_target).collect();
        if below_peg.is_empty() {
            checks.push(HealthCheck::Pass("No sells below peg".to_string()));
        } else {
            let usdt = below_peg.iter().map(|a| a.value()).sum::<f64>();
            checks.push(HealthCheck::Fail(format!(
                "{} sell(s) below peg ({:.0} USDT)",
                below_peg.len(),
                usdt
            )));
        }

        // Bid/ask ratio
        let ratio = if ask_depth > 0.0 {
            bid_depth / ask_depth
        } else {
            0.0
        };
        if ratio < thresholds.min_bid_ask_ratio || ratio > thresholds.max_bid_ask_ratio {
            checks.push(HealthCheck::Fail(format!("Bid/Ask ratio: {:.2}x", ratio)));
        } else {
            checks.push(HealthCheck::Pass(format!("Bid/Ask ratio: {:.2}x", ratio)));
        }

        // Bid levels
        if bids.len() < thresholds.min_levels {
            checks.push(HealthCheck::Fail(format!(
                "Bid levels: {} < {} minimum",
                bids.len(),
                thresholds.min_levels
            )));
        }

        // Bid depth
        if bid_depth < thresholds.min_depth {
            checks.push(HealthCheck::Fail(format!(
                "Bid depth: {:.0} USDT < {:.0} USDT minimum",
                bid_depth, thresholds.min_depth
            )));
        }

        // Ask levels
        if asks.len() < thresholds.min_levels {
            checks.push(HealthCheck::Fail(format!(
                "Ask levels: {} < {} minimum",
                asks.len(),
                thresholds.min_levels
            )));
        }

        // Ask depth
        if ask_depth < thresholds.min_depth {
            checks.push(HealthCheck::Fail(format!(
                "Ask depth: {:.0} USDT < {:.0} USDT minimum",
                ask_depth, thresholds.min_depth
            )));
        }

        let healthy = checks.iter().all(|c| matches!(c, HealthCheck::Pass(_)));

        Self {
            pair: book.pair.clone(),
            peg_target,
            best_bid: book.best_bid(),
            best_ask: book.best_ask(),
            mid_price: book.mid_price(),
            spread: book.spread(),
            asks,
            bids,
            ask_outliers,
            bid_outliers,
            ask_depth,
            bid_depth,
            checks,
            healthy,
        }
    }

    /// Format as human-readable text report.
    /// When `chain` is provided, it is displayed in the header.
    pub fn format_text(&self, chain: Option<&str>) -> String {
        let mut out = String::new();

        let title = match chain {
            Some(c) => format!("{} Market Summary ({})", self.pair, c),
            None => format!("{} Market Summary", self.pair),
        };
        out.push_str(&format!("\n  {}\n", title));
        out.push_str(&format!("  {}\n", "─".repeat(44)));
        if let Some(c) = chain {
            out.push_str(&format!("  Chain:          {}\n", c));
        }
        out.push_str(&format!("  Peg Target:     {:.4}\n", self.peg_target));

        if let Some(bb) = self.best_bid {
            let pct = (bb - self.peg_target) / self.peg_target * 100.0;
            out.push_str(&format!("  Best Bid:       {:.4}  ({:+.3}%)\n", bb, pct));
        } else {
            out.push_str("  Best Bid:       NONE\n");
        }

        if let Some(ba) = self.best_ask {
            let pct = (ba - self.peg_target) / self.peg_target * 100.0;
            out.push_str(&format!("  Best Ask:       {:.4}  ({:+.3}%)\n", ba, pct));
        } else {
            out.push_str("  Best Ask:       NONE\n");
        }

        if let Some(mid) = self.mid_price {
            let pct = (mid - self.peg_target) / self.peg_target * 100.0;
            out.push_str(&format!("  Mid Price:      {:.4}  ({:+.3}%)\n", mid, pct));
        }
        if let (Some(spread), Some(mid)) = (self.spread, self.mid_price)
            && mid > 0.0
        {
            out.push_str(&format!(
                "  Spread:         {:.4}  ({:.3}%)\n",
                spread,
                spread / mid * 100.0
            ));
        }

        out.push('\n');

        // Ask side
        let mut ask_label = format!(
            "  Ask Side:  {:>3} levels   {:>10.0} USDT depth",
            self.asks.len(),
            self.ask_depth
        );
        if self.ask_outliers > 0 {
            ask_label.push_str(&format!("  (+{} outlier(s) excluded)", self.ask_outliers));
        }
        ask_label.push('\n');
        out.push_str(&ask_label);

        let base_symbol = self.pair.split('/').next().unwrap_or("BASE");
        for level in self.asks.iter().take(8) {
            let flag = if level.price < self.peg_target {
                " ⚠ BELOW PEG"
            } else {
                ""
            };
            out.push_str(&format!(
                "    {:.4}  {:>10.2} {}  {:>10.2} USDT{}\n",
                level.price,
                level.quantity,
                base_symbol,
                level.value(),
                flag
            ));
        }
        if self.asks.len() > 8 {
            out.push_str(&format!("    ... +{} more levels\n", self.asks.len() - 8));
        }
        out.push('\n');

        // Bid side
        let mut bid_label = format!(
            "  Bid Side:  {:>3} levels   {:>10.0} USDT depth",
            self.bids.len(),
            self.bid_depth
        );
        if self.bid_outliers > 0 {
            bid_label.push_str(&format!("  (+{} outlier(s) excluded)", self.bid_outliers));
        }
        bid_label.push('\n');
        out.push_str(&bid_label);

        for level in self.bids.iter().take(8) {
            out.push_str(&format!(
                "    {:.4}  {:>10.2} {}  {:>10.2} USDT\n",
                level.price,
                level.quantity,
                base_symbol,
                level.value()
            ));
        }
        if self.bids.len() > 8 {
            out.push_str(&format!("    ... +{} more levels\n", self.bids.len() - 8));
        }
        out.push('\n');

        // Health checks
        for check in &self.checks {
            match check {
                HealthCheck::Pass(msg) => {
                    out.push_str(&format!("  ✓  {}\n", msg));
                }
                HealthCheck::Fail(msg) => {
                    out.push_str(&format!("  ⚠  {}\n", msg));
                }
            }
        }
        out.push('\n');

        let fail_count = self
            .checks
            .iter()
            .filter(|c| matches!(c, HealthCheck::Fail(_)))
            .count();
        if self.healthy {
            out.push_str("  Book: ✓ HEALTHY\n");
        } else {
            out.push_str(&format!("  Book: ⚠  {} issue(s) found\n", fail_count));
        }
        out.push('\n');

        out
    }
}

// =============================================================================
// Order Book Client Trait
// =============================================================================

/// Trait for fetching order book data from exchange APIs.
#[async_trait]
pub trait OrderBookClient: Send + Sync {
    /// Fetch the current order book for the given pair symbol (e.g., PUSD_USDT).
    async fn fetch_order_book(&self, pair_symbol: &str) -> Result<OrderBook>;
}

// =============================================================================
// Biconomy Client
// =============================================================================

/// Biconomy exchange order book client.
///
/// Uses the public depth API: `GET /api/v1/depth?symbol=PAIR_SYMBOL`
#[derive(Debug, Clone)]
pub struct BiconomyClient {
    base_url: String,
    client: Client,
}

impl BiconomyClient {
    /// Create a new Biconomy client with the given API base URL.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            client: Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("reqwest client build"),
        }
    }

    /// Create client with default Biconomy API URL.
    pub fn default_url() -> Self {
        Self::new("https://api.biconomy.com")
    }
}

#[derive(Debug, Deserialize)]
struct BiconomyDepthResponse {
    asks: Option<Vec<[String; 2]>>,
    bids: Option<Vec<[String; 2]>>,
}

#[async_trait]
impl OrderBookClient for BiconomyClient {
    async fn fetch_order_book(&self, pair_symbol: &str) -> Result<OrderBook> {
        let url = format!(
            "{}/api/v1/depth?symbol={}",
            self.base_url,
            urlencoding::encode(pair_symbol)
        );

        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(ScopeError::Chain(format!(
                "Biconomy API error: HTTP {}",
                resp.status()
            )));
        }

        let raw: BiconomyDepthResponse = resp
            .json()
            .await
            .map_err(|e| ScopeError::Chain(format!("Biconomy depth parse error: {}", e)))?;

        let asks = raw.asks.unwrap_or_default();
        let bids = raw.bids.unwrap_or_default();

        let parse_level = |p: &str, q: &str| -> Result<OrderBookLevel> {
            let price = p
                .parse::<f64>()
                .map_err(|_| ScopeError::Chain(format!("Invalid price: {}", p)))?;
            let quantity = q
                .parse::<f64>()
                .map_err(|_| ScopeError::Chain(format!("Invalid quantity: {}", q)))?;
            Ok(OrderBookLevel { price, quantity })
        };

        let mut ask_levels = Vec::with_capacity(asks.len());
        for [p, q] in &asks {
            ask_levels.push(parse_level(p, q)?);
        }
        ask_levels.sort_by(|a, b| {
            a.price
                .partial_cmp(&b.price)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut bid_levels = Vec::with_capacity(bids.len());
        for [p, q] in &bids {
            bid_levels.push(parse_level(p, q)?);
        }
        bid_levels.sort_by(|a, b| {
            b.price
                .partial_cmp(&a.price)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let pair = pair_symbol.replace('_', "/");

        Ok(OrderBook {
            pair,
            bids: bid_levels,
            asks: ask_levels,
        })
    }
}

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
    fn test_market_summary_from_order_book() {
        // Use quantities large enough to exceed min_depth (3000 USDT) per side
        let book = OrderBook {
            pair: "PUSD/USDT".to_string(),
            bids: vec![
                OrderBookLevel {
                    price: 0.9998,
                    quantity: 600.0,
                },
                OrderBookLevel {
                    price: 0.9997,
                    quantity: 600.0,
                },
                OrderBookLevel {
                    price: 0.9996,
                    quantity: 600.0,
                },
                OrderBookLevel {
                    price: 0.9995,
                    quantity: 600.0,
                },
                OrderBookLevel {
                    price: 0.9994,
                    quantity: 600.0,
                },
                OrderBookLevel {
                    price: 0.9993,
                    quantity: 600.0,
                },
            ],
            asks: vec![
                OrderBookLevel {
                    price: 1.0001,
                    quantity: 600.0,
                },
                OrderBookLevel {
                    price: 1.0002,
                    quantity: 600.0,
                },
                OrderBookLevel {
                    price: 1.0003,
                    quantity: 600.0,
                },
                OrderBookLevel {
                    price: 1.0004,
                    quantity: 600.0,
                },
                OrderBookLevel {
                    price: 1.0005,
                    quantity: 600.0,
                },
                OrderBookLevel {
                    price: 1.0006,
                    quantity: 600.0,
                },
            ],
        };

        let thresholds = HealthThresholds::default();
        let summary = MarketSummary::from_order_book(&book, 1.0, &thresholds);

        assert!(summary.healthy);
        assert_eq!(summary.bids.len(), 6);
        assert_eq!(summary.asks.len(), 6);
        assert!(summary.bid_depth > 3000.0);
        assert!(summary.ask_depth > 3000.0);
    }

    #[test]
    fn test_format_text_with_chain() {
        let book = OrderBook {
            pair: "PUSD/USDT".to_string(),
            bids: vec![OrderBookLevel {
                price: 1.0,
                quantity: 100.0,
            }],
            asks: vec![OrderBookLevel {
                price: 1.0,
                quantity: 100.0,
            }],
        };
        let summary = MarketSummary::from_order_book(&book, 1.0, &HealthThresholds::default());
        let out = summary.format_text(Some("biconomy"));
        assert!(out.contains("biconomy"));
        assert!(out.contains("Chain:"));
    }

    #[test]
    fn test_format_text_without_chain() {
        let book = OrderBook {
            pair: "X/Y".to_string(),
            bids: vec![OrderBookLevel {
                price: 1.0,
                quantity: 10.0,
            }],
            asks: vec![],
        };
        let summary = MarketSummary::from_order_book(&book, 1.0, &HealthThresholds::default());
        let out = summary.format_text(None);
        assert!(out.contains("X/Y Market Summary"));
        assert!(!out.contains("Chain:"));
    }

    #[test]
    fn test_health_check_sells_below_peg() {
        let book = OrderBook {
            pair: "PUSD/USDT".to_string(),
            bids: vec![OrderBookLevel {
                price: 0.9995,
                quantity: 1000.0,
            }],
            asks: vec![
                OrderBookLevel {
                    price: 0.9990, // below peg
                    quantity: 100.0,
                },
                OrderBookLevel {
                    price: 1.0001,
                    quantity: 500.0,
                },
            ],
        };

        let summary = MarketSummary::from_order_book(&book, 1.0, &HealthThresholds::default());

        assert!(!summary.healthy);
        let has_fail = summary
            .checks
            .iter()
            .any(|c| matches!(c, HealthCheck::Fail(m) if m.contains("sell")));
        assert!(has_fail);
    }
}
