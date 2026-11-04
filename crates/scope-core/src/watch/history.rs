//! What a run remembers from one tick to the next, for the metrics that
//! compare a tick with the one before ([`MetricId::since_last_tick`]).
//!
//! The memory is per market: a target on one venue, named by its label
//! (`USDC@binance`). A target that watches several venues keeps one memory
//! for each.

use super::RuleStatus;
use crate::market::Trade;
use crate::metrics::MetricId;
use std::collections::HashMap;

/// One reading of a since-last-tick metric: its value (`None` when the
/// source gave none), or the status its cell shows instead.
pub type Reading = Result<Option<f64>, RuleStatus>;

/// What a run remembers of one market.
#[derive(Debug, Clone, Default, PartialEq)]
struct Memory {
    /// The newest trade time seen on the tape, in ms since the epoch.
    last_trade_ms: Option<u64>,
    /// The 24h volume of the last tick that had one.
    volume_24h: Option<f64>,
}

/// The since-last-tick readings of one market for one tick.
#[derive(Debug, Clone, PartialEq)]
pub struct Readings {
    pub largest_trade: Reading,
    pub volume_change_pct: Reading,
}

impl Readings {
    /// The reading of `id`, if it is a since-last-tick metric.
    pub fn get(&self, id: MetricId) -> Option<&Reading> {
        match id {
            MetricId::LargestTrade => Some(&self.largest_trade),
            MetricId::VolumeChangePct => Some(&self.volume_change_pct),
            _ => None,
        }
    }
}

/// What a run remembers from tick to tick, per market.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct History {
    markets: HashMap<String, Memory>,
}

impl History {
    /// Reads this tick of `market` against the last one and remembers it.
    ///
    /// `trades` is the venue's tape (or why there is none) and `volume_24h`
    /// its 24h volume. The first tick has no tick before it: its largest
    /// trade is over the last `window_ms` (one interval) before `now_ms`,
    /// and its volume change waits for a second reading.
    pub fn read(
        &mut self,
        market: &str,
        trades: Result<&[Trade], &str>,
        volume_24h: Option<f64>,
        now_ms: u64,
        window_ms: u64,
    ) -> Readings {
        let memory = self.markets.entry(market.to_string()).or_default();
        Readings {
            largest_trade: largest_trade(memory, trades, now_ms, window_ms),
            volume_change_pct: volume_change(memory, volume_24h),
        }
    }
}

/// A trade's size in the quote asset.
fn quote_value(t: &Trade) -> f64 {
    t.quote_quantity.unwrap_or(t.price * t.quantity)
}

fn largest_trade(
    memory: &mut Memory,
    trades: Result<&[Trade], &str>,
    now_ms: u64,
    window_ms: u64,
) -> Reading {
    let trades = trades
        .map_err(|why| RuleStatus::Error(format!("{}: {why}", MetricId::LargestTrade.name())))?;
    let after = memory
        .last_trade_ms
        .unwrap_or_else(|| now_ms.saturating_sub(window_ms));
    // No trade since the last tick is a reading of its own: nothing traded.
    let largest = trades
        .iter()
        .filter(|t| t.timestamp_ms > after)
        .map(quote_value)
        .fold(0.0, f64::max);
    let newest = trades.iter().map(|t| t.timestamp_ms).max();
    memory.last_trade_ms = memory.last_trade_ms.max(newest);
    Ok(Some(largest))
}

fn volume_change(memory: &mut Memory, volume_24h: Option<f64>) -> Reading {
    let name = MetricId::VolumeChangePct.name();
    let Some(now) = volume_24h else {
        return Ok(None);
    };
    let before = memory.volume_24h.replace(now);
    match before {
        None => Err(RuleStatus::NotApplicable(format!(
            "{name}: needs a second reading"
        ))),
        Some(before) if before <= 0.0 => Err(RuleStatus::NotApplicable(format!(
            "{name}: the last 24h volume was 0"
        ))),
        Some(before) => Ok(Some((now - before) / before * 100.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::TradeSide;

    fn trade(ms: u64, price: f64, quantity: f64) -> Trade {
        Trade {
            price,
            quantity,
            quote_quantity: None,
            timestamp_ms: ms,
            side: TradeSide::Buy,
            id: None,
        }
    }

    const MIN: u64 = 60_000;

    #[test]
    fn the_first_tick_reads_one_interval_back_and_later_ticks_only_new_trades() {
        let mut h = History::default();
        let now = 10 * MIN;
        let tape = [
            trade(now - 5 * MIN, 1.0, 90_000.0), // before the window
            trade(now - 30_000, 1.0, 2_000.0),
            trade(now - 10_000, 1.0, 500.0),
        ];
        let first = h.read("USDC@binance", Ok(&tape), None, now, MIN);
        assert_eq!(first.largest_trade, Ok(Some(2_000.0)));

        // Next tick: the old tape again plus one new trade.
        let mut next = tape.to_vec();
        next.push(trade(now + 20_000, 1.0, 700.0));
        let second = h.read("USDC@binance", Ok(&next), None, now + MIN, MIN);
        assert_eq!(second.largest_trade, Ok(Some(700.0)));

        // No new trade: nothing traded, which is a value, not an error.
        let third = h.read("USDC@binance", Ok(&next), None, now + 2 * MIN, MIN);
        assert_eq!(third.largest_trade, Ok(Some(0.0)));
    }

    #[test]
    fn a_trade_uses_its_quote_quantity_when_the_venue_gives_one() {
        let mut big = trade(5_000, 2.0, 10.0);
        big.quote_quantity = Some(19.5);
        let mut h = History::default();
        let r = h.read("X@v", Ok(&[big]), None, 6_000, MIN);
        assert_eq!(r.largest_trade, Ok(Some(19.5)));
    }

    #[test]
    fn a_venue_without_a_tape_says_why() {
        let mut h = History::default();
        let r = h.read(
            "X@dex",
            Err("a DEX pool has no trade tape here"),
            None,
            0,
            MIN,
        );
        assert_eq!(
            r.largest_trade,
            Err(RuleStatus::Error(
                "largest_trade: a DEX pool has no trade tape here".into()
            ))
        );
    }

    #[test]
    fn volume_change_waits_for_a_second_reading_then_compares_ticks() {
        let mut h = History::default();
        let first = h.read("X@v", Ok(&[]), Some(1_000.0), 0, MIN);
        assert!(matches!(
            first.volume_change_pct,
            Err(RuleStatus::NotApplicable(_))
        ));
        let second = h.read("X@v", Ok(&[]), Some(1_500.0), MIN, MIN);
        assert_eq!(second.volume_change_pct, Ok(Some(50.0)));
        // A tick with no volume gives none and keeps the last one.
        let gap = h.read("X@v", Ok(&[]), None, 2 * MIN, MIN);
        assert_eq!(gap.volume_change_pct, Ok(None));
        let third = h.read("X@v", Ok(&[]), Some(750.0), 3 * MIN, MIN);
        assert_eq!(third.volume_change_pct, Ok(Some(-50.0)));
    }

    #[test]
    fn a_zero_volume_before_gives_no_percent() {
        let mut h = History::default();
        h.read("X@v", Ok(&[]), Some(0.0), 0, MIN);
        let r = h.read("X@v", Ok(&[]), Some(10.0), MIN, MIN);
        assert!(
            matches!(r.volume_change_pct, Err(RuleStatus::NotApplicable(m)) if m.contains("was 0"))
        );
    }

    #[test]
    fn each_market_has_its_own_memory() {
        let mut h = History::default();
        h.read("X@a", Ok(&[]), Some(100.0), 0, MIN);
        let other = h.read("X@b", Ok(&[]), Some(200.0), MIN, MIN);
        assert!(matches!(
            other.volume_change_pct,
            Err(RuleStatus::NotApplicable(_))
        ));
        let again = h.read("X@a", Ok(&[]), Some(110.0), MIN, MIN);
        let pct = again.volume_change_pct.clone().unwrap().unwrap();
        assert!((pct - 10.0).abs() < 1e-9, "{pct}");
        assert_eq!(again.get(MetricId::VolumeChangePct), Some(&Ok(Some(pct))));
        assert_eq!(again.get(MetricId::Spread), None);
    }
}
