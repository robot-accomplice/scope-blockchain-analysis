//! Order book health analysis and market summary.
//!
//! Provides configurable health checks for stablecoin market monitoring:
//! peg deviation, spread, bid/ask balance, level count, and depth thresholds.

use super::types::{BookSource, ExecutionEstimate, HealthCheck, OrderBook, OrderBookLevel};
use serde::{Deserialize, Serialize};

/// Default peg target (USD stablecoins).
pub const DEFAULT_PEG_TARGET: f64 = 1.0;
/// Default peg range for outlier filtering (peg ± range×5).
pub const DEFAULT_PEG_RANGE: f64 = 0.001;
/// Default minimum valid levels per side.
pub const DEFAULT_MIN_LEVELS: usize = 10;
/// Default minimum in-band depth per side (quote, e.g. USDT).
pub const DEFAULT_MIN_DEPTH: f64 = 3000.0;
/// Default bid/ask ratio floor.
pub const DEFAULT_MIN_BID_ASK_RATIO: f64 = 0.2;
/// Default bid/ask ratio ceiling.
pub const DEFAULT_MAX_BID_ASK_RATIO: f64 = 5.0;
/// Default maximum best-bid/best-ask spread, in percent of mid price.
pub const DEFAULT_MAX_SPREAD_PCT: f64 = 3.0;
/// Default minimum depth of the top 3 valid levels per side (quote).
pub const DEFAULT_MIN_TOP3_DEPTH: f64 = 300.0;
/// Default minimum depth of the top 10 valid levels per side (quote).
pub const DEFAULT_MIN_TOP10_DEPTH: f64 = 2000.0;

/// Default half-width of the depth window for the depth-to-volume rule,
/// in percent of the mid price.
pub const DEFAULT_DEPTH_WINDOW_PCT: f64 = 2.0;
/// Default floor: depth within the window, in percent of 24h volume.
/// Below it the book is too thin for the volume traded (fail).
pub const DEFAULT_MIN_DEPTH_VOLUME_PCT: f64 = 5.0;
/// Default ceiling: above it the volume is low for the visible
/// liquidity (warning).
pub const DEFAULT_MAX_DEPTH_VOLUME_PCT: f64 = 10.0;

/// Default price step between levels of a synthetic AMM book, in percent.
pub const DEFAULT_AMM_STEP_PCT: f64 = 0.1;

/// Number of levels summed for the top-3 depth check.
pub const TOP3_LEVELS: usize = 3;
/// Number of levels summed for the top-10 depth check.
pub const TOP10_LEVELS: usize = 10;

/// Health check thresholds for order book validation.
///
/// Built-in defaults are the `DEFAULT_*` constants in this module. The
/// `market.health` section of the config file replaces any of them, and
/// [`HealthOverrides`] (CLI flags, web request fields) replaces the result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthThresholds {
    /// Peg target (e.g., 1.0 for USD stablecoins).
    pub peg_target: f64,
    /// Price range for "near peg" orders (outliers excluded outside peg ± range×5).
    pub peg_range: f64,
    /// Minimum valid levels (price > 0, quantity > 0) per side.
    pub min_levels: usize,
    /// Minimum in-band depth per side in quote terms (e.g., USDT).
    pub min_depth: f64,
    /// Bid/ask ratio below which to warn (bid side thin).
    pub min_bid_ask_ratio: f64,
    /// Bid/ask ratio above which to warn (ask side thin).
    pub max_bid_ask_ratio: f64,
    /// Maximum spread between best valid bid and ask, in percent of mid price.
    pub max_spread_pct: f64,
    /// Minimum depth of the top 3 valid levels per side (quote).
    pub min_top3_depth: f64,
    /// Minimum depth of the top 10 valid levels per side (quote).
    pub min_top10_depth: f64,
    /// Price step between levels of a synthetic AMM book, in percent.
    /// With 0.1%, top-3 and top-10 depth mean depth within ±0.3% and ±1%.
    pub amm_step_pct: f64,
    /// Half-width of the depth window for the depth-to-volume rule, in
    /// percent of the mid price.
    pub depth_window_pct: f64,
    /// Depth within the window, in percent of 24h volume: below this fails.
    pub min_depth_volume_pct: f64,
    /// Depth within the window, in percent of 24h volume: above this warns.
    pub max_depth_volume_pct: f64,
}

impl Default for HealthThresholds {
    fn default() -> Self {
        Self {
            peg_target: DEFAULT_PEG_TARGET,
            peg_range: DEFAULT_PEG_RANGE,
            min_levels: DEFAULT_MIN_LEVELS,
            min_depth: DEFAULT_MIN_DEPTH,
            min_bid_ask_ratio: DEFAULT_MIN_BID_ASK_RATIO,
            max_bid_ask_ratio: DEFAULT_MAX_BID_ASK_RATIO,
            max_spread_pct: DEFAULT_MAX_SPREAD_PCT,
            min_top3_depth: DEFAULT_MIN_TOP3_DEPTH,
            min_top10_depth: DEFAULT_MIN_TOP10_DEPTH,
            amm_step_pct: DEFAULT_AMM_STEP_PCT,
            depth_window_pct: DEFAULT_DEPTH_WINDOW_PCT,
            min_depth_volume_pct: DEFAULT_MIN_DEPTH_VOLUME_PCT,
            max_depth_volume_pct: DEFAULT_MAX_DEPTH_VOLUME_PCT,
        }
    }
}

/// Per-request threshold overrides. A `None` field keeps the base value.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthOverrides {
    /// Peg target. The web API sends this as `peg`.
    #[serde(alias = "peg")]
    pub peg_target: Option<f64>,
    pub peg_range: Option<f64>,
    pub min_levels: Option<usize>,
    pub min_depth: Option<f64>,
    pub min_bid_ask_ratio: Option<f64>,
    pub max_bid_ask_ratio: Option<f64>,
    pub max_spread_pct: Option<f64>,
    pub min_top3_depth: Option<f64>,
    pub min_top10_depth: Option<f64>,
    pub amm_step_pct: Option<f64>,
    pub depth_window_pct: Option<f64>,
    pub min_depth_volume_pct: Option<f64>,
    pub max_depth_volume_pct: Option<f64>,
}

impl HealthThresholds {
    /// Returns a copy of `self` with each `Some` field in `o` applied.
    pub fn with_overrides(&self, o: &HealthOverrides) -> Self {
        Self {
            peg_target: o.peg_target.unwrap_or(self.peg_target),
            peg_range: o.peg_range.unwrap_or(self.peg_range),
            min_levels: o.min_levels.unwrap_or(self.min_levels),
            min_depth: o.min_depth.unwrap_or(self.min_depth),
            min_bid_ask_ratio: o.min_bid_ask_ratio.unwrap_or(self.min_bid_ask_ratio),
            max_bid_ask_ratio: o.max_bid_ask_ratio.unwrap_or(self.max_bid_ask_ratio),
            max_spread_pct: o.max_spread_pct.unwrap_or(self.max_spread_pct),
            min_top3_depth: o.min_top3_depth.unwrap_or(self.min_top3_depth),
            min_top10_depth: o.min_top10_depth.unwrap_or(self.min_top10_depth),
            amm_step_pct: o.amm_step_pct.unwrap_or(self.amm_step_pct),
            depth_window_pct: o.depth_window_pct.unwrap_or(self.depth_window_pct),
            min_depth_volume_pct: o.min_depth_volume_pct.unwrap_or(self.min_depth_volume_pct),
            max_depth_volume_pct: o.max_depth_volume_pct.unwrap_or(self.max_depth_volume_pct),
        }
    }
}

/// Sum of quote value over the first `n` valid levels (book order = best first).
fn top_n_depth(levels: &[OrderBookLevel], n: usize) -> f64 {
    levels
        .iter()
        .filter(|l| l.is_valid())
        .take(n)
        .map(OrderBookLevel::value)
        .sum()
}

/// Quote depth of the top valid levels per side, summed once per book.
///
/// The health checks and the repeating-view series both read these values.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TopDepth {
    pub bid_top3: f64,
    pub bid_top10: f64,
    pub ask_top3: f64,
    pub ask_top10: f64,
}

impl TopDepth {
    fn from_book(book: &OrderBook) -> Self {
        Self {
            bid_top3: top_n_depth(&book.bids, TOP3_LEVELS),
            bid_top10: top_n_depth(&book.bids, TOP10_LEVELS),
            ask_top3: top_n_depth(&book.asks, TOP3_LEVELS),
            ask_top10: top_n_depth(&book.asks, TOP10_LEVELS),
        }
    }
}

/// Quote depth of valid levels within ±`window_pct`% of `mid`, both sides.
///
/// The second value is `true` when a side's last fetched level is still
/// inside the window: the book may continue past what was fetched, so the
/// depth is a lower bound.
fn depth_within(book: &OrderBook, mid: f64, window_pct: f64) -> (f64, bool) {
    let lo = mid * (1.0 - window_pct / 100.0);
    let hi = mid * (1.0 + window_pct / 100.0);
    let in_window = |l: &&OrderBookLevel| l.is_valid() && l.price >= lo && l.price <= hi;
    let depth = book
        .bids
        .iter()
        .chain(&book.asks)
        .filter(in_window)
        .map(OrderBookLevel::value)
        .sum();
    let ends_inside = |levels: &[OrderBookLevel]| {
        levels
            .iter()
            .rev()
            .find(|l| l.is_valid())
            .is_some_and(|l| l.price >= lo && l.price <= hi)
    };
    (depth, ends_inside(&book.bids) || ends_inside(&book.asks))
}

/// The depth-to-volume rule: depth within the window should be
/// `min..=max` percent of 24h volume. Below fails, above warns.
fn depth_volume_check(
    book: &OrderBook,
    thresholds: &HealthThresholds,
    volume_24h: Option<f64>,
) -> HealthCheck {
    let w = thresholds.depth_window_pct;
    let (lo, hi) = (
        thresholds.min_depth_volume_pct,
        thresholds.max_depth_volume_pct,
    );
    let (Some(volume), Some(mid)) = (volume_24h.filter(|v| *v > 0.0), book.mid_price()) else {
        return HealthCheck::NotApplicable(
            "Depth/volume: n/a (no 24h volume or mid price from this source)".to_string(),
        );
    };
    let (depth, truncated) = depth_within(book, mid, w);
    let pct = depth / volume * 100.0;
    let at_least = if truncated { "≥" } else { "" };
    let msg = format!(
        "Depth/volume: {at_least}{pct:.1}% ({at_least}{depth:.0} / {volume:.0} USDT within ±{w}%; target {lo}–{hi}%)"
    );
    if pct > hi {
        // A lower bound above the ceiling is still above it.
        HealthCheck::Warn(msg)
    } else if pct < lo && truncated {
        HealthCheck::NotApplicable(format!(
            "{msg}; n/a: the book ends before ±{w}%, so the depth may be higher"
        ))
    } else if pct < lo {
        HealthCheck::Fail(msg)
    } else {
        HealthCheck::Pass(msg)
    }
}

/// Aggregated market summary with order book snapshot and health results.
#[derive(Debug, Clone)]
pub struct MarketSummary {
    /// Pair label (e.g., "USDC/USDT").
    pub pair: String,
    /// Where the order book came from.
    pub source: BookSource,
    /// Peg target.
    pub peg_target: f64,
    /// Best valid bid (no outlier filtering).
    pub best_bid: Option<f64>,
    /// Best valid ask.
    pub best_ask: Option<f64>,
    /// Mid of the best valid bid and ask.
    pub mid_price: Option<f64>,
    /// Absolute spread between the best valid bid and ask. `None` for a
    /// synthetic AMM book or when a side has no valid level.
    pub spread: Option<f64>,
    /// The same spread in percent of the mid of those two levels.
    pub spread_pct: Option<f64>,
    /// Top-3 and top-10 depth per side, as the checks judge them.
    pub top_depth: TopDepth,
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
    /// Build summary from order book; the peg is `thresholds.peg_target`.
    /// Optionally includes 24h volume (from venue ticker) and execution estimates for 10k USDT.
    pub fn from_order_book(
        book: &OrderBook,
        thresholds: &HealthThresholds,
        volume_24h: Option<f64>,
    ) -> Self {
        let peg_target = thresholds.peg_target;
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

        let valid_bids = book.bids.iter().filter(|l| l.is_valid()).count();
        let valid_asks = book.asks.iter().filter(|l| l.is_valid()).count();
        // A synthetic AMM book has no real level count or spread (#37).
        let amm = book.source == BookSource::SyntheticAmm;
        let top_depth = TopDepth::from_book(book);

        // Spread between best valid bid and best valid ask
        match (amm, book.spread_pct()) {
            (true, _) => checks.push(HealthCheck::NotApplicable(
                "Spread: n/a (synthetic AMM book; the real spread is the pool fee, \
                 which the data source does not report)"
                    .to_string(),
            )),
            (false, Some(spread_pct)) => {
                let msg = format!(
                    "Spread: {:.3}% (max {:.1}%)",
                    spread_pct, thresholds.max_spread_pct
                );
                if spread_pct > thresholds.max_spread_pct {
                    checks.push(HealthCheck::Fail(msg));
                } else {
                    checks.push(HealthCheck::Pass(msg));
                }
            }
            (false, None) => {
                checks.push(HealthCheck::Fail("Spread: no valid bid or ask".to_string()))
            }
        }

        // Bid levels
        if amm {
            checks.push(HealthCheck::NotApplicable(
                "Bid levels: n/a (synthetic AMM book; the step size sets the level count)"
                    .to_string(),
            ));
        } else if valid_bids < thresholds.min_levels {
            checks.push(HealthCheck::Fail(format!(
                "Bid levels: {} < {} minimum",
                valid_bids, thresholds.min_levels
            )));
        } else {
            checks.push(HealthCheck::Pass(format!(
                "Bid levels: {} (min {})",
                valid_bids, thresholds.min_levels
            )));
        }

        // Bid depth
        if bid_depth < thresholds.min_depth {
            checks.push(HealthCheck::Fail(format!(
                "Bid depth: {:.0} USDT < {:.0} USDT minimum",
                bid_depth, thresholds.min_depth
            )));
        } else {
            checks.push(HealthCheck::Pass(format!(
                "Bid depth: {:.0} USDT (min {:.0})",
                bid_depth, thresholds.min_depth
            )));
        }

        // Ask levels
        if amm {
            checks.push(HealthCheck::NotApplicable(
                "Ask levels: n/a (synthetic AMM book; the step size sets the level count)"
                    .to_string(),
            ));
        } else if valid_asks < thresholds.min_levels {
            checks.push(HealthCheck::Fail(format!(
                "Ask levels: {} < {} minimum",
                valid_asks, thresholds.min_levels
            )));
        } else {
            checks.push(HealthCheck::Pass(format!(
                "Ask levels: {} (min {})",
                valid_asks, thresholds.min_levels
            )));
        }

        // Ask depth
        if ask_depth < thresholds.min_depth {
            checks.push(HealthCheck::Fail(format!(
                "Ask depth: {:.0} USDT < {:.0} USDT minimum",
                ask_depth, thresholds.min_depth
            )));
        } else {
            checks.push(HealthCheck::Pass(format!(
                "Ask depth: {:.0} USDT (min {:.0})",
                ask_depth, thresholds.min_depth
            )));
        }

        // Top-N depth per side (valid levels, best first)
        for (side, top3, top10) in [
            ("Bid", top_depth.bid_top3, top_depth.bid_top10),
            ("Ask", top_depth.ask_top3, top_depth.ask_top10),
        ] {
            for (n, depth, min) in [
                (TOP3_LEVELS, top3, thresholds.min_top3_depth),
                (TOP10_LEVELS, top10, thresholds.min_top10_depth),
            ] {
                if depth < min {
                    checks.push(HealthCheck::Fail(format!(
                        "{side} top-{n} depth: {depth:.0} USDT < {min:.0} USDT minimum"
                    )));
                } else {
                    checks.push(HealthCheck::Pass(format!(
                        "{side} top-{n} depth: {depth:.0} USDT (min {min:.0})"
                    )));
                }
            }
        }

        checks.push(depth_volume_check(book, thresholds, volume_24h));

        // n/a checks (#37 decision A) and warnings do not count.
        let healthy = !checks.iter().any(|c| matches!(c, HealthCheck::Fail(_)));

        let execution_10k_buy = book.estimate_buy_execution(10_000.0);
        let execution_10k_sell = book.estimate_sell_execution(10_000.0);

        Self {
            pair: book.pair.clone(),
            source: book.source,
            peg_target,
            best_bid: book.best_bid(),
            best_ask: book.best_ask(),
            mid_price: book.mid_price(),
            spread: book.spread(),
            spread_pct: book.spread_pct(),
            top_depth,
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
        out.push_str(&t::kv_row("Book", self.source.label()));
        out.push('\n');
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
        if let (Some(spread), Some(pct)) = (self.spread, self.spread_pct) {
            out.push_str(&t::kv_row(
                "Spread",
                &format!("{:.4}  ({:.3}%)", spread, pct),
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
            out.push_str(&check.render_text());
            out.push('\n');
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

    /// Side of `qtys.len()` levels stepping away from `start` by `step` per level.
    fn side(start: f64, step: f64, qtys: &[f64]) -> Vec<OrderBookLevel> {
        qtys.iter()
            .enumerate()
            .map(|(i, &quantity)| OrderBookLevel {
                price: start + step * i as f64,
                quantity,
            })
            .collect()
    }

    /// Book that passes every default check: 10 levels × 400 per side near peg.
    fn healthy_book() -> OrderBook {
        OrderBook {
            pair: "USDC/USDT".to_string(),
            source: crate::market::types::BookSource::Exchange,
            bids: side(0.9999, -0.0001, &[400.0; 10]),
            asks: side(1.0001, 0.0001, &[400.0; 10]),
        }
    }

    fn summarize(book: &OrderBook) -> MarketSummary {
        MarketSummary::from_order_book(book, &HealthThresholds::default(), None)
    }

    fn failed(summary: &MarketSummary) -> Vec<String> {
        summary
            .checks
            .iter()
            .filter_map(|c| match c {
                HealthCheck::Fail(m) => Some(m.clone()),
                HealthCheck::Pass(_) | HealthCheck::NotApplicable(_) | HealthCheck::Warn(_) => None,
            })
            .collect()
    }

    // ---- Depth-to-volume rule ----

    /// `healthy_book` plus one level per side beyond ±2%, so the book does
    /// not end inside the depth window.
    fn deep_book() -> OrderBook {
        let mut b = healthy_book();
        b.bids.push(OrderBookLevel {
            price: 0.97,
            quantity: 100.0,
        });
        b.asks.push(OrderBookLevel {
            price: 1.03,
            quantity: 100.0,
        });
        b
    }

    /// Depth within ±2% of mid for `deep_book` (the far levels are outside).
    fn deep_window_depth() -> f64 {
        let b = healthy_book();
        b.bids
            .iter()
            .chain(&b.asks)
            .map(OrderBookLevel::value)
            .sum()
    }

    fn with_volume(book: &OrderBook, volume: Option<f64>) -> MarketSummary {
        MarketSummary::from_order_book(book, &HealthThresholds::default(), volume)
    }

    fn depth_volume(s: &MarketSummary) -> &HealthCheck {
        let found = status_of(s, "Depth/volume");
        assert_eq!(found.len(), 1, "{:?}", s.checks);
        found[0]
    }

    #[test]
    fn test_depth_volume_below_floor_fails() {
        // 4% of volume is visible: too thin for the volume traded.
        let s = with_volume(&deep_book(), Some(deep_window_depth() / 0.04));
        assert!(
            matches!(depth_volume(&s), HealthCheck::Fail(_)),
            "{:?}",
            depth_volume(&s)
        );
        assert!(!s.healthy);
    }

    #[test]
    fn test_depth_volume_at_floor_passes() {
        let s = with_volume(&deep_book(), Some(deep_window_depth() / 0.05));
        assert!(
            matches!(depth_volume(&s), HealthCheck::Pass(_)),
            "{:?}",
            depth_volume(&s)
        );
        assert!(s.healthy, "{:?}", s.checks);
    }

    #[test]
    fn test_depth_volume_above_ceiling_warns_but_stays_healthy() {
        // 12%: visible liquidity is high for the volume. A warning only.
        let s = with_volume(&deep_book(), Some(deep_window_depth() / 0.12));
        let c = depth_volume(&s);
        assert!(matches!(c, HealthCheck::Warn(_)), "{c:?}");
        assert!(c.message().contains("12.0%"), "{c:?}");
        assert!(s.healthy, "a warning must not make the book unhealthy");
    }

    #[test]
    fn test_depth_volume_without_volume_is_not_applicable() {
        let s = with_volume(&deep_book(), None);
        assert!(matches!(depth_volume(&s), HealthCheck::NotApplicable(_)));
        let s = with_volume(&deep_book(), Some(0.0));
        assert!(matches!(depth_volume(&s), HealthCheck::NotApplicable(_)));
    }

    #[test]
    fn test_truncated_book_below_floor_is_not_applicable() {
        // healthy_book ends inside ±2%: the real depth may be higher, so a
        // floor failure would not be conclusive.
        let s = with_volume(&healthy_book(), Some(deep_window_depth() / 0.01));
        let c = depth_volume(&s);
        assert!(matches!(c, HealthCheck::NotApplicable(_)), "{c:?}");
        assert!(c.message().contains('≥'), "{c:?}");
    }

    #[test]
    fn test_truncated_book_above_ceiling_still_warns() {
        let s = with_volume(&healthy_book(), Some(deep_window_depth() / 0.30));
        let c = depth_volume(&s);
        assert!(matches!(c, HealthCheck::Warn(_)), "{c:?}");
        assert!(c.message().contains("≥30.0%"), "{c:?}");
    }

    #[test]
    fn test_depth_volume_thresholds_are_overridable() {
        let t = HealthThresholds {
            min_depth_volume_pct: 1.0,
            max_depth_volume_pct: 50.0,
            ..HealthThresholds::default()
        };
        let s = MarketSummary::from_order_book(&deep_book(), &t, Some(deep_window_depth() / 0.12));
        assert!(matches!(depth_volume(&s), HealthCheck::Pass(_)));
    }

    // ---- Display: every check shows its value and limit ----

    #[test]
    fn test_passing_level_and_depth_checks_are_shown_with_limits() {
        let s = summarize(&healthy_book());
        for prefix in [
            "Bid levels",
            "Ask levels",
            "Bid depth",
            "Ask depth",
            "Bid top-3 depth",
            "Ask top-10 depth",
        ] {
            let found = status_of(&s, prefix);
            assert_eq!(found.len(), 1, "{prefix}: {:?}", s.checks);
            assert!(
                matches!(found[0], HealthCheck::Pass(_)),
                "{prefix}: {:?}",
                found[0]
            );
            assert!(
                found[0].message().contains("(min "),
                "{prefix}: {:?}",
                found[0]
            );
        }
    }

    // ---- #37: synthetic AMM books ----

    fn amm_summary(base_reserve: f64) -> MarketSummary {
        let pair = crate::chains::DexPair {
            dex_name: "Uniswap V2".into(),
            pair_address: "0xpool".into(),
            base_token: "USDC".into(),
            quote_token: "USDT".into(),
            price_usd: 1.0,
            volume_24h: 0.0,
            liquidity_usd: 2.0 * base_reserve,
            liquidity_base: Some(base_reserve),
            price_change_24h: 0.0,
            buys_24h: 0,
            sells_24h: 0,
            buys_6h: 0,
            sells_6h: 0,
            buys_1h: 0,
            sells_1h: 0,
            pair_created_at: None,
            url: None,
        };
        let t = HealthThresholds::default();
        let book =
            super::super::analytics::order_book_from_analytics("ethereum", &pair, "USDC", &t);
        MarketSummary::from_order_book(&book, &t, None)
    }

    fn status_of<'a>(s: &'a MarketSummary, prefix: &str) -> Vec<&'a HealthCheck> {
        s.checks
            .iter()
            .filter(|c| c.message().starts_with(prefix))
            .collect()
    }

    #[test]
    fn test_amm_levels_and_spread_are_not_applicable_and_shown() {
        // #37 decision A: n/a checks are always shown and never fail.
        let s = amm_summary(10_000_000.0);
        assert_eq!(s.source, BookSource::SyntheticAmm);
        let spread = status_of(&s, "Spread");
        assert_eq!(spread.len(), 1);
        assert!(matches!(spread[0], HealthCheck::NotApplicable(_)));
        for side in ["Bid levels", "Ask levels"] {
            let c = status_of(&s, side);
            assert_eq!(c.len(), 1, "{} must always be shown", side);
            assert!(matches!(c[0], HealthCheck::NotApplicable(_)), "{}", side);
        }
    }

    #[test]
    fn test_deep_amm_pool_can_be_healthy() {
        // 10M base at $1: ±0.5% band depth ≈ 25k per side, top-3 ≈ 15k,
        // top-10 ≈ 50k. Every applicable rule passes.
        let s = amm_summary(10_000_000.0);
        assert!(s.healthy, "{:?}", failed(&s));
    }

    #[test]
    fn test_shallow_amm_pool_fails_the_depth_rules() {
        // 1,000 base at $1: band depth ≈ 2.5 USDT per side.
        let s = amm_summary(1_000.0);
        assert!(!s.healthy);
        let f = failed(&s);
        assert!(f.iter().any(|m| m.starts_with("Bid depth")), "{:?}", f);
        assert!(
            f.iter().any(|m| m.starts_with("Ask top-3 depth")),
            "{:?}",
            f
        );
        assert!(
            f.iter().any(|m| m.starts_with("Bid top-10 depth")),
            "{:?}",
            f
        );
    }

    #[test]
    fn test_exchange_book_rules_are_unchanged() {
        // The AMM n/a rules (levels, spread) apply to synthetic AMM books
        // only. Depth/volume is n/a here for another reason: no volume.
        let s = summarize(&healthy_book());
        assert_eq!(s.source, BookSource::Exchange);
        assert!(
            s.checks
                .iter()
                .filter(|c| !c.message().starts_with("Depth/volume"))
                .all(|c| !matches!(c, HealthCheck::NotApplicable(_))),
            "{:?}",
            s.checks
        );
    }

    #[test]
    fn test_book_source_labels_and_json_names() {
        // JSON consumers match on these names; the labels name the source
        // in text and markdown reports.
        for (src, json, label) in [
            (BookSource::Exchange, "exchange", "exchange order book"),
            (
                BookSource::SyntheticAmm,
                "synthetic_amm",
                "synthetic (AMM curve x·y=k)",
            ),
            (
                BookSource::SyntheticEstimate,
                "synthetic_estimate",
                "synthetic estimate (pool reserves not reported)",
            ),
        ] {
            assert_eq!(serde_json::to_value(src).unwrap(), serde_json::json!(json));
            assert_eq!(src.label(), label);
        }
        assert_eq!(BookSource::default(), BookSource::Exchange);
        let na = HealthCheck::NotApplicable("x".into());
        assert_eq!((na.icon(), na.status(), na.message()), ("–", "n/a", "x"));
    }

    #[test]
    fn test_check_json_shape_includes_na() {
        // JSON consumers (the web UI among them) match on these strings.
        let v = serde_json::to_value(HealthCheck::NotApplicable("x".into())).unwrap();
        assert_eq!(v, serde_json::json!({"status": "n/a", "message": "x"}));
        let v = serde_json::to_value(HealthCheck::Fail("y".into())).unwrap();
        assert_eq!(v, serde_json::json!({"status": "fail", "message": "y"}));
    }

    #[test]
    fn test_text_report_names_the_book_source() {
        // The report wraps at terminal_width().
        let _lock = crate::display::terminal::width_lock();
        let s = amm_summary(10_000_000.0);
        let text = s.format_text(Some("eth"));
        assert!(text.contains("synthetic (AMM curve x·y=k)"), "{}", text);
        assert!(text.contains("– "), "n/a rows use the dash icon: {}", text);
    }

    #[test]
    fn test_market_summary_from_order_book() {
        let summary = summarize(&healthy_book());

        assert!(summary.healthy, "{:?}", failed(&summary));
        assert_eq!(summary.bids.len(), 10);
        assert_eq!(summary.asks.len(), 10);
        assert!(summary.bid_depth > 3000.0);
        assert!(summary.ask_depth > 3000.0);
    }

    #[test]
    fn test_peg_comes_from_thresholds() {
        // Asks at 1.0001+ are below a 1.01 peg, so the peg check must fail.
        // Before, callers passed a literal 1.0 and --peg had no effect.
        let thresholds = HealthThresholds {
            peg_target: 1.01,
            peg_range: 0.01,
            ..HealthThresholds::default()
        };
        let summary = MarketSummary::from_order_book(&healthy_book(), &thresholds, None);
        assert_eq!(summary.peg_target, 1.01);
        assert!(
            failed(&summary).iter().any(|m| m.contains("below peg")),
            "{:?}",
            failed(&summary)
        );
    }

    #[test]
    fn test_default_criteria_values() {
        // Pins the agreed "healthy" spec so a silent default change fails here.
        let t = HealthThresholds::default();
        assert_eq!(t.min_levels, 10);
        assert_eq!(t.max_spread_pct, 3.0);
        assert_eq!(t.min_top3_depth, 300.0);
        assert_eq!(t.min_top10_depth, 2000.0);
        assert_eq!(t.min_depth, 3000.0);
        // Depth within ±2% of mid should be 5–10% of 24h volume.
        assert_eq!(t.depth_window_pct, 2.0);
        assert_eq!(t.min_depth_volume_pct, 5.0);
        assert_eq!(t.max_depth_volume_pct, 10.0);
    }

    #[test]
    fn test_nine_levels_is_unhealthy() {
        let mut book = healthy_book();
        // Keep total depth high so only the level-count rule can fail.
        book.bids = side(0.9999, -0.0001, &[1000.0; 9]);
        let summary = summarize(&book);
        assert!(!summary.healthy);
        assert_eq!(failed(&summary), vec!["Bid levels: 9 < 10 minimum"]);
    }

    #[test]
    fn test_zero_quantity_level_is_not_valid() {
        let mut book = healthy_book();
        book.asks[9].quantity = 0.0;
        book.asks.push(OrderBookLevel {
            price: 1.0011,
            quantity: 0.0,
        });
        let summary = summarize(&book);
        assert!(
            failed(&summary).contains(&"Ask levels: 9 < 10 minimum".to_string()),
            "{:?}",
            failed(&summary)
        );
    }

    #[test]
    fn test_spread_over_three_percent_is_unhealthy() {
        let mut book = healthy_book();
        // Best ask 1.0400 vs best bid 0.9999: ~3.94% of mid.
        book.asks = side(1.04, 0.0001, &[400.0; 10]);
        let summary = summarize(&book);
        assert!(!summary.healthy);
        assert!(
            failed(&summary).iter().any(|m| m.starts_with("Spread:")),
            "{:?}",
            failed(&summary)
        );
    }

    #[test]
    fn test_spread_under_three_percent_passes_spread_check() {
        let mut book = healthy_book();
        // Best ask 1.0200 vs best bid 0.9999: ~1.98% of mid.
        book.asks = side(1.02, 0.0001, &[400.0; 10]);
        let summary = summarize(&book);
        assert!(
            !failed(&summary).iter().any(|m| m.starts_with("Spread:")),
            "{:?}",
            failed(&summary)
        );
    }

    #[test]
    fn test_thin_top3_is_unhealthy_even_with_deep_book() {
        let mut book = healthy_book();
        // Top 3 hold ~150 USDT; levels 4-10 hold 3500, so only top-3 fails.
        let mut qtys = vec![50.0; 3];
        qtys.extend([500.0; 7]);
        book.bids = side(0.9999, -0.0001, &qtys);
        let summary = summarize(&book);
        assert_eq!(
            failed(&summary),
            vec!["Bid top-3 depth: 150 USDT < 300 USDT minimum"]
        );
    }

    #[test]
    fn test_top10_counts_only_first_ten_levels() {
        let mut book = healthy_book();
        // First 10 levels hold ~1500; level 11 adds 2000 so total depth passes.
        let mut qtys = vec![150.0; 10];
        qtys.push(2000.0);
        book.asks = side(1.0001, 0.0001, &qtys);
        let summary = summarize(&book);
        assert!(summary.ask_depth >= 3000.0);
        let fails = failed(&summary);
        assert_eq!(fails.len(), 1, "{fails:?}");
        assert!(fails[0].starts_with("Ask top-10 depth:"), "{fails:?}");
    }

    #[test]
    fn test_format_text_with_chain() {
        let book = OrderBook {
            pair: "USDC/USDT".to_string(),
            source: crate::market::types::BookSource::Exchange,
            bids: vec![OrderBookLevel {
                price: 1.0,
                quantity: 100.0,
            }],
            asks: vec![OrderBookLevel {
                price: 1.0,
                quantity: 100.0,
            }],
        };
        let summary = MarketSummary::from_order_book(&book, &HealthThresholds::default(), None);
        let out = summary.format_text(Some("biconomy"));
        assert!(out.contains("biconomy"));
        assert!(out.contains("Venue"));
    }

    #[test]
    fn test_format_text_without_chain() {
        let book = OrderBook {
            pair: "X/Y".to_string(),
            source: crate::market::types::BookSource::Exchange,
            bids: vec![OrderBookLevel {
                price: 1.0,
                quantity: 10.0,
            }],
            asks: vec![],
        };
        let summary = MarketSummary::from_order_book(&book, &HealthThresholds::default(), None);
        let out = summary.format_text(None);
        assert!(out.contains("X/Y"));
        assert!(!out.contains("Venue"));
    }

    #[test]
    fn test_health_check_sells_below_peg() {
        let book = OrderBook {
            pair: "USDC/USDT".to_string(),
            source: crate::market::types::BookSource::Exchange,
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

        let summary = MarketSummary::from_order_book(&book, &HealthThresholds::default(), None);

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
            source: crate::market::types::BookSource::Exchange,
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
        let summary =
            MarketSummary::from_order_book(&book, &HealthThresholds::default(), Some(50_000.0));
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
            pair: "USDC/USDT".to_string(),
            source: crate::market::types::BookSource::Exchange,
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
        let summary = MarketSummary::from_order_book(&book, &HealthThresholds::default(), None);
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
            source: crate::market::types::BookSource::Exchange,
            bids: vec![OrderBookLevel {
                price: 0.9999,
                quantity: 20_000.0,
            }],
            asks: vec![OrderBookLevel {
                price: 1.0001,
                quantity: 20_000.0,
            }],
        };
        let summary = MarketSummary::from_order_book(&book, &HealthThresholds::default(), None);
        let out = summary.format_text(Some("cex"));
        assert!(out.contains("Exec 10K buy"));
        assert!(out.contains("Exec 10K sell"));
        assert!(out.contains("slippage") || out.contains("insufficient"));
    }

    #[test]
    fn test_format_text_bid_outliers_and_many_bid_levels() {
        // Many bid levels + bid outlier → covers lines 328-330, 354-355
        let mut bids: Vec<OrderBookLevel> = (0..12)
            .map(|i| OrderBookLevel {
                price: 1.0 - 0.0001 * (i + 1) as f64,
                quantity: 500.0,
            })
            .collect();
        bids.push(OrderBookLevel {
            price: 0.98, // bid outlier far below peg
            quantity: 100.0,
        });
        let book = OrderBook {
            pair: "USDC/USDT".to_string(),
            source: crate::market::types::BookSource::Exchange,
            bids,
            asks: vec![
                OrderBookLevel {
                    price: 1.0001,
                    quantity: 600.0,
                },
                OrderBookLevel {
                    price: 1.0002,
                    quantity: 600.0,
                },
            ],
        };
        let summary = MarketSummary::from_order_book(&book, &HealthThresholds::default(), None);
        let out = summary.format_text(None);
        assert!(summary.bid_outliers > 0);
        assert!(out.contains("outliers excl."));
        assert!(out.contains("... +"));
    }
}
