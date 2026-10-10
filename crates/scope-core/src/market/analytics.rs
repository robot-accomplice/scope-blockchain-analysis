//! DEX analytics utilities.
//!
//! Builds synthetic order books from DexScreener liquidity data for
//! Ethereum and Solana DEX venues (#37).
//!
//! With the pool's base reserve `x0` and price `P0`, a constant-product
//! pool (x·y=k) holds `x(P) = x0·√(P0/P)` base at price `P`. One level is
//! placed at each price step; its quantity is the base the pool buys
//! (bids) or sells (asks) between two steps. Without the reserve, the old
//! single-level estimate is kept and marked as such.

use super::health::{DEFAULT_AMM_STEP_PCT, HealthThresholds, TOP10_LEVELS};
use super::types::{BookSource, OrderBook, OrderBookLevel};

/// The most levels per side of a synthetic AMM book. It bounds the work for
/// a very small step or a very wide band.
pub const MAX_AMM_LEVELS: usize = 1000;

/// Spread of the single-level estimate around the price (±0.1%).
const ESTIMATE_HALF_SPREAD: f64 = 0.001;

/// Builds a synthetic order book from a DEX pair.
///
/// Uses the constant-product curve when the pair reports its base reserve,
/// otherwise a single-level estimate (`BookSource::SyntheticEstimate`).
pub fn order_book_from_analytics(
    _chain: &str,
    pair: &crate::chains::DexPair,
    symbol: &str,
    thresholds: &HealthThresholds,
) -> OrderBook {
    let label = format!("{}/USDT", symbol);
    match pair.liquidity_base {
        Some(x0) if x0.is_finite() && x0 > 0.0 && pair.price_usd > 0.0 => {
            amm_book(label, pair.price_usd, x0, thresholds)
        }
        _ => estimate_book(label, pair.price_usd, pair.liquidity_usd),
    }
}

/// Levels per side: enough to cover the outlier band, and at least the
/// levels the top-10 depth check reads.
fn amm_levels_per_side(step: f64, t: &HealthThresholds) -> usize {
    let band = if t.peg_target > 0.0 {
        t.peg_range * 5.0 / t.peg_target
    } else {
        0.0
    };
    let to_band = if band.is_finite() && band > 0.0 {
        (band / step - 1e-9).ceil() as usize
    } else {
        0
    };
    to_band.clamp(TOP10_LEVELS, MAX_AMM_LEVELS)
}

fn amm_book(label: String, p0: f64, x0: f64, t: &HealthThresholds) -> OrderBook {
    let step_pct = if t.amm_step_pct.is_finite() && t.amm_step_pct > 0.0 {
        t.amm_step_pct
    } else {
        DEFAULT_AMM_STEP_PCT
    };
    let step = step_pct / 100.0;
    let n = amm_levels_per_side(step, t);
    let base_at = |p: f64| x0 * (p0 / p).sqrt();

    let mut bids = Vec::with_capacity(n);
    let mut prev = x0;
    for i in 1..=n {
        let p = p0 * (1.0 - i as f64 * step);
        if p <= 0.0 {
            break;
        }
        let x = base_at(p);
        bids.push(OrderBookLevel {
            price: p,
            quantity: x - prev,
        });
        prev = x;
    }

    let mut asks = Vec::with_capacity(n);
    let mut prev = x0;
    for i in 1..=n {
        let p = p0 * (1.0 + i as f64 * step);
        let x = base_at(p);
        asks.push(OrderBookLevel {
            price: p,
            quantity: prev - x,
        });
        prev = x;
    }

    OrderBook {
        pair: label,
        source: BookSource::SyntheticAmm,
        bids,
        asks,
    }
}

/// The single-level estimate: half the pool's liquidity at ±0.1%.
fn estimate_book(label: String, price: f64, liquidity: f64) -> OrderBook {
    let bid_price = price * (1.0 - ESTIMATE_HALF_SPREAD);
    let ask_price = price * (1.0 + ESTIMATE_HALF_SPREAD);
    let half_liq = liquidity / 2.0;
    let qty = |p: f64| if p > 0.0 { half_liq / p } else { 0.0 };
    OrderBook {
        pair: label,
        source: BookSource::SyntheticEstimate,
        bids: vec![OrderBookLevel {
            price: bid_price,
            quantity: qty(bid_price),
        }],
        asks: vec![OrderBookLevel {
            price: ask_price,
            quantity: qty(ask_price),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::DexPair;

    fn make_pair(price: f64, liquidity: f64) -> DexPair {
        DexPair {
            dex_name: "TestDex".into(),
            pair_address: "0x0".into(),
            base_token: "USDC".into(),
            quote_token: "WETH".into(),
            price_usd: price,
            volume_24h: 0.0,
            liquidity_usd: liquidity,
            liquidity_base: None,
            price_change_24h: 0.0,
            buys_24h: 0,
            sells_24h: 0,
            buys_6h: 0,
            sells_6h: 0,
            buys_1h: 0,
            sells_1h: 0,
            pair_created_at: None,
            url: None,
        }
    }

    use crate::market::health::HealthThresholds;
    use crate::market::types::BookSource;

    fn amm_pair(price: f64, base_reserve: f64) -> DexPair {
        DexPair {
            liquidity_base: Some(base_reserve),
            ..make_pair(price, 2.0 * price * base_reserve)
        }
    }

    fn depth(levels: &[OrderBookLevel], n: usize) -> f64 {
        levels.iter().take(n).map(OrderBookLevel::value).sum()
    }

    #[test]
    fn test_amm_depth_within_one_percent_matches_constant_product() {
        // #37 acceptance: depth at ±1% equals the closed form of x·y=k.
        // Bids (pool buys base as the price falls to 0.99·P0) pay
        // x0·P0·(1 − √0.99); asks (pool sells base up to 1.01·P0) cost
        // x0·P0·(√1.01 − 1). Ten 0.1% steps reach ±1%.
        let (p0, x0) = (1.0, 1_000_000.0);
        let book = order_book_from_analytics(
            "ethereum",
            &amm_pair(p0, x0),
            "USDC",
            &HealthThresholds::default(),
        );
        assert_eq!(book.source, BookSource::SyntheticAmm);
        let bid_closed = x0 * p0 * (1.0 - 0.99_f64.sqrt());
        let ask_closed = x0 * p0 * (1.01_f64.sqrt() - 1.0);
        let (bid, ask) = (depth(&book.bids, 10), depth(&book.asks, 10));
        assert!(
            (bid - bid_closed).abs() / bid_closed < 0.005,
            "bid {} vs {}",
            bid,
            bid_closed
        );
        assert!(
            (ask - ask_closed).abs() / ask_closed < 0.005,
            "ask {} vs {}",
            ask,
            ask_closed
        );
        // Book order: best first.
        assert!(book.bids.windows(2).all(|w| w[0].price > w[1].price));
        assert!(book.asks.windows(2).all(|w| w[0].price < w[1].price));
        assert!((book.bids[0].price - 0.999).abs() < 1e-12);
        assert!((book.asks[0].price - 1.001).abs() < 1e-12);
    }

    #[test]
    fn test_amm_level_count_covers_band_and_top10() {
        // Default band is peg ± 0.5% (peg_range 0.001 × 5): 5 steps of 0.1%.
        // Top-10 depth still needs 10 levels.
        let t = HealthThresholds::default();
        let book = order_book_from_analytics("ethereum", &amm_pair(1.0, 1e6), "USDC", &t);
        assert_eq!((book.bids.len(), book.asks.len()), (10, 10));
        // A wider band (±5%) needs 50 levels of 0.1%.
        let wide = HealthThresholds {
            peg_range: 0.01,
            ..t.clone()
        };
        let book = order_book_from_analytics("ethereum", &amm_pair(1.0, 1e6), "USDC", &wide);
        assert_eq!(book.asks.len(), 50);
        // A finer step reaches the same band with more levels.
        let fine = HealthThresholds {
            amm_step_pct: 0.05,
            ..t
        };
        let book = order_book_from_analytics("ethereum", &amm_pair(1.0, 1e6), "USDC", &fine);
        assert_eq!(book.asks.len(), 10);
        assert!((book.asks[0].price - 1.0005).abs() < 1e-12);
    }

    #[test]
    fn test_amm_bad_step_falls_back_to_default() {
        // A zero, negative or NaN step from a config file must not hang
        // or divide by zero.
        for step in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let t = HealthThresholds {
                amm_step_pct: step,
                ..HealthThresholds::default()
            };
            let book = order_book_from_analytics("ethereum", &amm_pair(1.0, 1e6), "USDC", &t);
            assert_eq!(book.asks.len(), 10, "step {}", step);
            assert!((book.asks[0].price - 1.001).abs() < 1e-12, "step {}", step);
        }
    }

    #[test]
    fn test_missing_reserves_keep_the_single_level_estimate() {
        // #37: without reserves keep the old book, and say so.
        let book = order_book_from_analytics(
            "ethereum",
            &make_pair(1.0, 100_000.0),
            "USDC",
            &HealthThresholds::default(),
        );
        assert_eq!(book.source, BookSource::SyntheticEstimate);
        assert_eq!((book.bids.len(), book.asks.len()), (1, 1));
    }

    #[test]
    fn test_order_book_from_analytics_normal() {
        let pair = make_pair(1.0, 100_000.0);
        let book =
            order_book_from_analytics("ethereum", &pair, "USDC", &HealthThresholds::default());
        assert_eq!(book.pair, "USDC/USDT");
        assert_eq!(book.bids.len(), 1);
        assert_eq!(book.asks.len(), 1);
        assert!(book.bids[0].price > 0.0);
        assert!(book.asks[0].price > 0.0);
        assert!(book.bids[0].quantity > 0.0);
        assert!(book.asks[0].quantity > 0.0);
    }

    #[test]
    fn test_order_book_from_analytics_zero_price() {
        // price_usd = 0.0 -> bid_price = 0 and ask_price = 0
        // -> both qty branches hit the else { 0.0 }
        let pair = make_pair(0.0, 100_000.0);
        let book =
            order_book_from_analytics("ethereum", &pair, "TOKEN", &HealthThresholds::default());
        assert_eq!(book.bids[0].quantity, 0.0);
        assert_eq!(book.asks[0].quantity, 0.0);
    }

    #[test]
    fn test_order_book_from_analytics_zero_liquidity() {
        let pair = make_pair(1.0, 0.0);
        let book = order_book_from_analytics("solana", &pair, "SOL", &HealthThresholds::default());
        assert_eq!(book.bids[0].quantity, 0.0);
        assert_eq!(book.asks[0].quantity, 0.0);
    }
}
