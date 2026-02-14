//! Order book health analysis and market summary.
//!
//! Provides configurable health checks for stablecoin market monitoring:
//! peg deviation, spread, bid/ask balance, and minimum depth thresholds.

use super::types::{ExecutionEstimate, HealthCheck, OrderBook, OrderBookLevel};

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
    /// 24h volume in quote (e.g. USDT) if available.
    pub volume_24h: Option<f64>,
    /// Execution estimate for 10k USDT buy (slippage).
    pub execution_10k_buy: Option<ExecutionEstimate>,
    /// Execution estimate for 10k USDT sell (slippage).
    pub execution_10k_sell: Option<ExecutionEstimate>,
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
    /// Optionally includes 24h volume (from venue ticker) and execution estimates for 10k USDT.
    pub fn from_order_book(
        book: &OrderBook,
        peg_target: f64,
        thresholds: &HealthThresholds,
        volume_24h: Option<f64>,
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

        let execution_10k_buy = book.estimate_buy_execution(10_000.0);
        let execution_10k_sell = book.estimate_sell_execution(10_000.0);

        Self {
            pair: book.pair.clone(),
            peg_target,
            best_bid: book.best_bid(),
            best_ask: book.best_ask(),
            mid_price: book.mid_price(),
            spread: book.spread(),
            volume_24h,
            execution_10k_buy,
            execution_10k_sell,
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
    /// When `venue_or_chain` is provided, it is displayed in the header (e.g. binance, ethereum).
    pub fn format_text(&self, venue_or_chain: Option<&str>) -> String {
        use crate::display::terminal as t;

        let mut out = String::new();

        let title = match venue_or_chain {
            Some(c) => format!("{} ({})", self.pair, c),
            None => self.pair.clone(),
        };
        out.push_str(&t::section_header(&title));
        out.push('\n');

        // Metrics subsection
        out.push_str(&t::subsection_header("Metrics"));
        out.push('\n');
        if let Some(c) = venue_or_chain {
            out.push_str(&t::kv_row("Venue", c));
            out.push('\n');
        }
        out.push_str(&t::kv_row("Peg Target", &format!("{:.4}", self.peg_target)));
        out.push('\n');

        if let Some(bb) = self.best_bid {
            let pct = (bb - self.peg_target) / self.peg_target * 100.0;
            out.push_str(&t::kv_row(
                "Best Bid",
                &format!(
                    "{}  ({:+.3}%)",
                    t::format_price_peg(bb, self.peg_target),
                    pct
                ),
            ));
            out.push('\n');
        }
        if let Some(ba) = self.best_ask {
            let pct = (ba - self.peg_target) / self.peg_target * 100.0;
            out.push_str(&t::kv_row(
                "Best Ask",
                &format!(
                    "{}  ({:+.3}%)",
                    t::format_price_peg(ba, self.peg_target),
                    pct
                ),
            ));
            out.push('\n');
        }
        if let Some(mid) = self.mid_price {
            let pct = (mid - self.peg_target) / self.peg_target * 100.0;
            out.push_str(&t::kv_row(
                "Mid Price",
                &format!(
                    "{}  ({:+.3}%)",
                    t::format_price_peg(mid, self.peg_target),
                    pct
                ),
            ));
            out.push('\n');
        }
        if let (Some(spread), Some(mid)) = (self.spread, self.mid_price)
            && mid > 0.0
        {
            out.push_str(&t::kv_row(
                "Spread",
                &format!("{:.4}  ({:.3}%)", spread, spread / mid * 100.0),
            ));
            out.push('\n');
        }

        if let Some(v) = self.volume_24h {
            out.push_str(&t::kv_row("Volume (24h)", &format!("{:.0} USDT", v)));
            out.push('\n');
        }

        if let Some(e) = &self.execution_10k_buy {
            let msg = if e.fillable {
                format!("~{:.2} bps slippage", e.slippage_bps)
            } else {
                "insufficient liquidity".to_string()
            };
            out.push_str(&t::kv_row("Exec 10K buy", &msg));
            out.push('\n');
        }
        if let Some(e) = &self.execution_10k_sell {
            let msg = if e.fillable {
                format!("~{:.2} bps slippage", e.slippage_bps)
            } else {
                "insufficient liquidity".to_string()
            };
            out.push_str(&t::kv_row("Exec 10K sell", &msg));
            out.push('\n');
        }

        // Ask side
        let base_symbol = self.pair.split('/').next().unwrap_or("BASE");
        let ask_label = if self.ask_outliers > 0 {
            format!(
                "Ask Side — {} levels, {:.0} USDT (+{} outliers excl.)",
                self.asks.len(),
                self.ask_depth,
                self.ask_outliers
            )
        } else {
            format!(
                "Ask Side — {} levels, {:.0} USDT",
                self.asks.len(),
                self.ask_depth
            )
        };
        out.push_str(&t::subsection_header(&ask_label));
        out.push('\n');
        for level in self.asks.iter().take(8) {
            out.push_str(&t::orderbook_level(
                level.price,
                level.quantity,
                base_symbol,
                level.value(),
                self.peg_target,
            ));
            out.push('\n');
        }
        if self.asks.len() > 8 {
            out.push_str(&t::blank_row());
            out.push_str(&format!("    ... +{} more levels\n", self.asks.len() - 8));
        }

        // Bid side
        let bid_label = if self.bid_outliers > 0 {
            format!(
                "Bid Side — {} levels, {:.0} USDT (+{} outliers excl.)",
                self.bids.len(),
                self.bid_depth,
                self.bid_outliers
            )
        } else {
            format!(
                "Bid Side — {} levels, {:.0} USDT",
                self.bids.len(),
                self.bid_depth
            )
        };
        out.push_str(&t::subsection_header(&bid_label));
        out.push('\n');
        for level in self.bids.iter().take(8) {
            out.push_str(&t::orderbook_level(
                level.price,
                level.quantity,
                base_symbol,
                level.value(),
                self.peg_target,
            ));
            out.push('\n');
        }
        if self.bids.len() > 8 {
            out.push_str(&t::blank_row());
            out.push_str(&format!("    ... +{} more levels\n", self.bids.len() - 8));
        }

        // Health checks
        out.push_str(&t::subsection_header("Health Checks"));
        out.push('\n');
        for check in &self.checks {
            match check {
                HealthCheck::Pass(msg) => {
                    out.push_str(&t::check_pass(msg));
                    out.push('\n');
                }
                HealthCheck::Fail(msg) => {
                    out.push_str(&t::check_fail(msg));
                    out.push('\n');
                }
            }
        }
        out.push_str(&t::blank_row());
        out.push('\n');
        out.push_str(&t::status_line(self.healthy));
        out.push('\n');
        out.push_str(&t::section_footer());
        out.push('\n');

        out
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

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
        let summary = MarketSummary::from_order_book(&book, 1.0, &thresholds, None);

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
        let summary =
            MarketSummary::from_order_book(&book, 1.0, &HealthThresholds::default(), None);
        let out = summary.format_text(Some("biconomy"));
        assert!(out.contains("biconomy"));
        assert!(out.contains("Venue"));
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
        let summary =
            MarketSummary::from_order_book(&book, 1.0, &HealthThresholds::default(), None);
        let out = summary.format_text(None);
        assert!(out.contains("X/Y"));
        assert!(!out.contains("Venue"));
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
                    price: 0.9990,
                    quantity: 100.0,
                }, // below peg
                OrderBookLevel {
                    price: 1.0001,
                    quantity: 500.0,
                },
            ],
        };

        let summary =
            MarketSummary::from_order_book(&book, 1.0, &HealthThresholds::default(), None);

        assert!(!summary.healthy);
        let has_fail = summary
            .checks
            .iter()
            .any(|c| matches!(c, HealthCheck::Fail(m) if m.contains("sell")));
        assert!(has_fail);
    }

    #[test]
    fn test_format_text_with_volume_and_spread() {
        let book = OrderBook {
            pair: "USDC/USDT".to_string(),
            bids: vec![
                OrderBookLevel {
                    price: 0.9999,
                    quantity: 1000.0,
                },
                OrderBookLevel {
                    price: 0.9998,
                    quantity: 1000.0,
                },
            ],
            asks: vec![
                OrderBookLevel {
                    price: 1.0001,
                    quantity: 1000.0,
                },
                OrderBookLevel {
                    price: 1.0002,
                    quantity: 1000.0,
                },
            ],
        };
        let summary = MarketSummary::from_order_book(
            &book,
            1.0,
            &HealthThresholds::default(),
            Some(50_000.0),
        );
        let out = summary.format_text(Some("binance"));
        assert!(out.contains("Volume (24h)"));
        assert!(out.contains("50000"));
        assert!(out.contains("Spread"));
    }

    #[test]
    fn test_format_text_with_outliers_and_many_levels() {
        // Book with levels outside peg range (peg_range*5 = 0.005) -> outliers
        // price_lo=0.995, price_hi=1.005 - asks above 1.005 excluded
        let mut asks: Vec<OrderBookLevel> = (0..12)
            .map(|i| OrderBookLevel {
                price: 1.0 + 0.0001 * (i + 1) as f64, // 1.0001 .. 1.0012
                quantity: 500.0,
            })
            .collect();
        asks.push(OrderBookLevel {
            price: 1.01, // outlier
            quantity: 100.0,
        });
        let book = OrderBook {
            pair: "PUSD/USDT".to_string(),
            bids: vec![
                OrderBookLevel {
                    price: 0.9999,
                    quantity: 600.0,
                },
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
            ],
            asks,
        };
        let summary =
            MarketSummary::from_order_book(&book, 1.0, &HealthThresholds::default(), None);
        let out = summary.format_text(None);
        assert!(
            out.contains("outliers excl.") || summary.ask_outliers > 0 || summary.bid_outliers > 0
        );
        assert!(out.contains("... +")); // truncation for > 8 levels
    }

    #[test]
    fn test_format_text_execution_fillable() {
        // Sufficient liquidity for 10k buy/sell -> fillable
        let book = OrderBook {
            pair: "USDC/USDT".to_string(),
            bids: vec![OrderBookLevel {
                price: 0.9999,
                quantity: 20_000.0,
            }],
            asks: vec![OrderBookLevel {
                price: 1.0001,
                quantity: 20_000.0,
            }],
        };
        let summary =
            MarketSummary::from_order_book(&book, 1.0, &HealthThresholds::default(), None);
        let out = summary.format_text(Some("cex"));
        assert!(out.contains("Exec 10K buy"));
        assert!(out.contains("Exec 10K sell"));
        assert!(out.contains("slippage") || out.contains("insufficient"));
    }
}
