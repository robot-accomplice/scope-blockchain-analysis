//! What a run remembers from one tick to the next, for the metrics that
//! compare a tick with the one before ([`MetricId::since_last_tick`]).
//!
//! The memory is per market: a target on one venue, named by a
//! [`MarketKey`]. A target that watches several venues keeps one memory
//! for each, and two targets with the same id, chain and venue keep one
//! each (the key has an ordinal).
//!
//! The memory can be saved and restored ([`History::entries`],
//! [`History::restore`]) so that a run resumes where the one before ended.
//! This module does no I/O: the file code is in scope-cli.

use super::RuleStatus;
use crate::market::Trade;
use crate::metrics::MetricId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One reading of a since-last-tick metric: its value (`None` when the
/// source gave none), or the status its cell shows instead.
pub type Reading = Result<Option<f64>, RuleStatus>;

/// Which market a memory belongs to: the target (`id`, `chain`, `ordinal`)
/// on a venue. The `ordinal` is 1 for the first target of a list with this
/// id, chain and venue, 2 for the second, and so on. The row label
/// (`USDC@binance`) is for display; two rows can have one label, never one
/// key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MarketKey {
    pub id: String,
    pub chain: Option<String>,
    pub venue: String,
    pub ordinal: u32,
}

impl std::fmt::Display for MarketKey {
    /// `USDC@binance`, with `#2` for a second target and `[chain]` when it
    /// has one.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}", self.id, self.venue)?;
        if self.ordinal > 1 {
            write!(f, "#{}", self.ordinal)?;
        }
        if let Some(chain) = &self.chain {
            write!(f, "[{chain}]")?;
        }
        Ok(())
    }
}

/// What a run remembers of one market.
#[derive(Debug, Clone, Default, PartialEq)]
struct Memory {
    /// The newest trade time seen on the tape, in ms since the epoch.
    last_trade_ms: Option<u64>,
    /// The 24h volume of the last tick that had one.
    volume_24h: Option<f64>,
    /// When a tick last wrote this memory, in ms since the epoch.
    updated_ms: Option<u64>,
}

/// One saved memory: the market and what is remembered of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(flatten)]
    pub key: MarketKey,
    pub last_trade_ms: Option<u64>,
    pub volume_24h: Option<f64>,
    pub updated_ms: u64,
}

/// Whether a saved entry was used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// It was young enough and is in the restored [`History`].
    Loaded,
    /// It was too old, or from the future (the clock moved back). It was
    /// dropped, and the market has a first tick.
    Stale,
}

impl Basis {
    pub fn name(self) -> &'static str {
        match self {
            Self::Loaded => "loaded",
            Self::Stale => "stale",
        }
    }
}

/// What [`History::restore`] did with one entry: the record that explains
/// a first tick after the fact.
#[derive(Debug, Clone, PartialEq)]
pub struct Restored {
    pub entry: Entry,
    pub basis: Basis,
    /// `now_ms - updated_ms`; `None` when `updated_ms` is in the future.
    pub age_ms: Option<u64>,
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
    markets: HashMap<MarketKey, Memory>,
}

impl History {
    /// The memory a run starts with: the `entries` that are not stale at
    /// `now_ms`. An entry is stale when it is older than `max_age_ms`, or
    /// when its `updated_ms` is after `now_ms`. An entry exactly
    /// `max_age_ms` old is kept. Returns what was done with every entry.
    pub fn restore(entries: Vec<Entry>, now_ms: u64, max_age_ms: u64) -> (Self, Vec<Restored>) {
        let mut history = Self::default();
        let mut report = Vec::with_capacity(entries.len());
        for entry in entries {
            let age_ms = now_ms.checked_sub(entry.updated_ms);
            let basis = match age_ms {
                Some(age) if age <= max_age_ms => Basis::Loaded,
                _ => Basis::Stale,
            };
            if basis == Basis::Loaded {
                history.markets.insert(
                    entry.key.clone(),
                    Memory {
                        last_trade_ms: entry.last_trade_ms,
                        volume_24h: entry.volume_24h,
                        updated_ms: Some(entry.updated_ms),
                    },
                );
            }
            report.push(Restored {
                entry,
                basis,
                age_ms,
            });
        }
        (history, report)
    }

    /// The memories that a tick wrote, for saving, in key order.
    pub fn entries(&self) -> Vec<Entry> {
        let mut entries: Vec<Entry> = self
            .markets
            .iter()
            .filter_map(|(key, m)| {
                Some(Entry {
                    key: key.clone(),
                    last_trade_ms: m.last_trade_ms,
                    volume_24h: m.volume_24h,
                    updated_ms: m.updated_ms?,
                })
            })
            .collect();
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        entries
    }

    /// Reads this tick of `market` against the last one and remembers it.
    ///
    /// `trades` is the venue's tape (or why there is none) and `volume_24h`
    /// its 24h volume. The first tick has no tick before it: its largest
    /// trade is over the last `window_ms` (one interval) before `now_ms`,
    /// and its volume change waits for a second reading.
    pub fn read(
        &mut self,
        market: &MarketKey,
        trades: Result<&[Trade], &str>,
        volume_24h: Option<f64>,
        now_ms: u64,
        window_ms: u64,
    ) -> Readings {
        let memory = self.markets.entry(market.clone()).or_default();
        // A tick that got a tape or a volume wrote the memory.
        if trades.is_ok() || volume_24h.is_some() {
            memory.updated_ms = Some(now_ms);
        }
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

    fn key(id: &str, venue: &str) -> MarketKey {
        MarketKey {
            id: id.into(),
            chain: None,
            venue: venue.into(),
            ordinal: 1,
        }
    }

    /// A tape with one trade, 10 s before `now`.
    fn tape_at(now: u64) -> Vec<Trade> {
        vec![trade(now - 10_000, 1.0, 700.0)]
    }

    /// Why: the state file must not change the numbers. A run that resumes
    /// reads like a run that never stopped.
    #[test]
    fn a_restored_history_reads_like_the_one_that_was_saved() {
        let k = key("USDN", "weex");
        let (t1, t2) = (10 * MIN, 15 * MIN);
        let mut straight = History::default();
        straight.read(&k, Ok(&tape_at(t1)), Some(1_000.0), t1, MIN);
        let second_straight = straight.read(&k, Ok(&tape_at(t2)), Some(1_500.0), t2, MIN);

        let mut saved = History::default();
        saved.read(&k, Ok(&tape_at(t1)), Some(1_000.0), t1, MIN);
        // Three intervals of five minutes would keep it; here one gap does.
        let (mut restored, report) = History::restore(saved.entries(), t2, 3 * (t2 - t1));
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].basis, Basis::Loaded);
        assert_eq!(report[0].age_ms, Some(t2 - t1));
        let second_restored = restored.read(&k, Ok(&tape_at(t2)), Some(1_500.0), t2, MIN);

        assert_eq!(second_restored, second_straight);
        assert_eq!(second_restored.volume_change_pct, Ok(Some(50.0)));
    }

    /// Why: the limit is a contract. An entry exactly at the limit is kept,
    /// one ms more is dropped, and a clock that moved back never gives a
    /// negative age.
    #[test]
    fn the_stale_rule_holds_at_the_edge_and_when_the_clock_moves_back() {
        let entry = |id: &str, updated_ms| Entry {
            key: key(id, "v"),
            last_trade_ms: Some(1),
            volume_24h: Some(2.0),
            updated_ms,
        };
        let (now, limit) = (100 * MIN, 3 * MIN);
        let (h, report) = History::restore(
            vec![
                entry("edge", now - limit),
                entry("over", now - limit - 1),
                entry("future", now + 1),
                entry("now", now),
            ],
            now,
            limit,
        );
        let basis: Vec<_> = report
            .iter()
            .map(|r| (r.entry.key.id.as_str(), r.basis))
            .collect();
        assert_eq!(
            basis,
            [
                ("edge", Basis::Loaded),
                ("over", Basis::Stale),
                ("future", Basis::Stale),
                ("now", Basis::Loaded),
            ]
        );
        assert_eq!(report[2].age_ms, None);
        let kept: Vec<_> = h.entries().into_iter().map(|e| e.key.id).collect();
        assert_eq!(kept, ["edge", "now"]);
    }

    /// Why: a stale reading must never give a fake percentage. A dropped
    /// entry is a first tick: no second reading, and the tape is read back
    /// one window only.
    #[test]
    fn a_dropped_entry_gives_a_first_tick() {
        let k = key("USDN", "weex");
        let mut old = History::default();
        old.read(&k, Ok(&tape_at(MIN)), Some(1_000.0), MIN, MIN);
        let now = 100 * MIN;
        let (mut h, report) = History::restore(old.entries(), now, 3 * MIN);
        assert_eq!(report[0].basis, Basis::Stale);
        // A big trade 4 minutes ago is outside the window of one minute.
        let tape = [
            trade(now - 4 * MIN, 1.0, 90_000.0),
            trade(now - 5_000, 1.0, 10.0),
        ];
        let r = h.read(&k, Ok(&tape), Some(5_000.0), now, MIN);
        assert!(matches!(
            r.volume_change_pct,
            Err(RuleStatus::NotApplicable(_))
        ));
        assert_eq!(r.largest_trade, Ok(Some(10.0)));
    }

    /// Why: two targets on one market have two memories. Each starts with
    /// no reading, and the entries keep the two apart.
    #[test]
    fn the_ordinal_keeps_two_targets_of_one_market_apart() {
        let (first, mut second) = (key("USDC", "v"), key("USDC", "v"));
        second.ordinal = 2;
        let mut h = History::default();
        let a = h.read(&first, Ok(&[]), Some(100.0), 0, MIN);
        let b = h.read(&second, Ok(&[]), Some(150.0), 0, MIN);
        assert!(matches!(
            a.volume_change_pct,
            Err(RuleStatus::NotApplicable(_))
        ));
        assert!(matches!(
            b.volume_change_pct,
            Err(RuleStatus::NotApplicable(_))
        ));
        let ordinals: Vec<_> = h.entries().iter().map(|e| e.key.ordinal).collect();
        assert_eq!(ordinals, [1, 2]);
        assert_eq!(second.to_string(), "USDC@v#2");
    }

    /// Why: a market that no tick wrote has nothing to save.
    #[test]
    fn a_read_with_no_data_saves_no_entry() {
        let mut h = History::default();
        h.read(&key("X", "dex"), Err("no tape"), None, 5, MIN);
        assert!(h.entries().is_empty());
    }

    #[test]
    fn the_first_tick_reads_one_interval_back_and_later_ticks_only_new_trades() {
        let mut h = History::default();
        let now = 10 * MIN;
        let tape = [
            trade(now - 5 * MIN, 1.0, 90_000.0), // before the window
            trade(now - 30_000, 1.0, 2_000.0),
            trade(now - 10_000, 1.0, 500.0),
        ];
        let first = h.read(&key("USDC", "binance"), Ok(&tape), None, now, MIN);
        assert_eq!(first.largest_trade, Ok(Some(2_000.0)));

        // Next tick: the old tape again plus one new trade.
        let mut next = tape.to_vec();
        next.push(trade(now + 20_000, 1.0, 700.0));
        let second = h.read(&key("USDC", "binance"), Ok(&next), None, now + MIN, MIN);
        assert_eq!(second.largest_trade, Ok(Some(700.0)));

        // No new trade: nothing traded, which is a value, not an error.
        let third = h.read(&key("USDC", "binance"), Ok(&next), None, now + 2 * MIN, MIN);
        assert_eq!(third.largest_trade, Ok(Some(0.0)));
    }

    #[test]
    fn a_trade_uses_its_quote_quantity_when_the_venue_gives_one() {
        let mut big = trade(5_000, 2.0, 10.0);
        big.quote_quantity = Some(19.5);
        let mut h = History::default();
        let r = h.read(&key("X", "v"), Ok(&[big]), None, 6_000, MIN);
        assert_eq!(r.largest_trade, Ok(Some(19.5)));
    }

    #[test]
    fn a_venue_without_a_tape_says_why() {
        let mut h = History::default();
        let r = h.read(
            &key("X", "dex"),
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
        let first = h.read(&key("X", "v"), Ok(&[]), Some(1_000.0), 0, MIN);
        assert!(matches!(
            first.volume_change_pct,
            Err(RuleStatus::NotApplicable(_))
        ));
        let second = h.read(&key("X", "v"), Ok(&[]), Some(1_500.0), MIN, MIN);
        assert_eq!(second.volume_change_pct, Ok(Some(50.0)));
        // A tick with no volume gives none and keeps the last one.
        let gap = h.read(&key("X", "v"), Ok(&[]), None, 2 * MIN, MIN);
        assert_eq!(gap.volume_change_pct, Ok(None));
        let third = h.read(&key("X", "v"), Ok(&[]), Some(750.0), 3 * MIN, MIN);
        assert_eq!(third.volume_change_pct, Ok(Some(-50.0)));
    }

    #[test]
    fn a_zero_volume_before_gives_no_percent() {
        let mut h = History::default();
        h.read(&key("X", "v"), Ok(&[]), Some(0.0), 0, MIN);
        let r = h.read(&key("X", "v"), Ok(&[]), Some(10.0), MIN, MIN);
        assert!(
            matches!(r.volume_change_pct, Err(RuleStatus::NotApplicable(m)) if m.contains("was 0"))
        );
    }

    #[test]
    fn each_market_has_its_own_memory() {
        let mut h = History::default();
        h.read(&key("X", "a"), Ok(&[]), Some(100.0), 0, MIN);
        let other = h.read(&key("X", "b"), Ok(&[]), Some(200.0), MIN, MIN);
        assert!(matches!(
            other.volume_change_pct,
            Err(RuleStatus::NotApplicable(_))
        ));
        let again = h.read(&key("X", "a"), Ok(&[]), Some(110.0), MIN, MIN);
        let pct = again.volume_change_pct.clone().unwrap().unwrap();
        assert!((pct - 10.0).abs() < 1e-9, "{pct}");
        assert_eq!(again.get(MetricId::VolumeChangePct), Some(&Ok(Some(pct))));
        assert_eq!(again.get(MetricId::Spread), None);
    }
}
