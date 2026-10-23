//! Bot-pattern signals in a venue's recent trades and order book, for the
//! market summary's health advisory.
//!
//! Market making is legitimate, so a signal never fails a summary: it warns,
//! with the evidence, where the tape or the book looks machine-made. Four
//! signals:
//!
//! - **Repeated sizes:** one exact trade size makes up a large share of the
//!   tape.
//! - **Metronome timing:** trades come at an almost fixed interval.
//! - **Ping-pong:** a buy and a sell of the same size at nearly the same
//!   price, seconds apart (the shape of wash trading).
//! - **Ladder quotes:** consecutive book levels of the same size at even
//!   price steps.
//!
//! The functions are pure: they read trades and levels and fetch nothing.

use super::types::{BookSide, OrderBookLevel, Trade, TradeSide};
use serde::Serialize;

/// How many recent trades the summary asks the venue for.
pub const TRADE_SAMPLE: u32 = 500;

/// Fewest trades the tape signals need. Below this they are not judged.
pub const MIN_TRADES: usize = 30;

/// One exact size in at least this share of the trades is a signal.
pub const REPEATED_SIZE_SHARE: f64 = 0.30;

/// Fewest gaps between trade events the timing signal needs.
pub const MIN_GAPS: usize = 20;

/// Gaps whose standard deviation is under this share of their mean are a
/// metronome.
pub const METRONOME_MAX_CV: f64 = 0.10;

/// A ping-pong pair is at most this far apart in time.
pub const PING_PONG_WINDOW_MS: u64 = 5_000;

/// A ping-pong pair's prices differ by at most this, in percent.
pub const PING_PONG_PRICE_TOLERANCE_PCT: f64 = 0.05;

/// Trades in ping-pong pairs, as a share of all trades, at or above this
/// are a signal.
pub const PING_PONG_SHARE: f64 = 0.20;

/// Fewest ping-pong pairs that count as a signal.
pub const MIN_PING_PONG_PAIRS: usize = 5;

/// Fewest consecutive levels that make a ladder.
pub const LADDER_MIN_LEVELS: usize = 8;

/// Levels of a ladder differ in size by at most this share.
pub const LADDER_SIZE_TOLERANCE: f64 = 0.01;

/// Price steps of a ladder differ by at most this share of the first step.
pub const LADDER_STEP_TOLERANCE: f64 = 0.05;

/// How many levels from the top of each side the ladder signal reads.
pub const LADDER_DEPTH: usize = 25;

/// Sizes are compared at this many decimal places, so that floats parsed
/// from the same decimal string compare equal.
const SIZE_DECIMALS: f64 = 1e8;

/// One signal that was found, with its evidence.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Finding {
    /// `count` of `of` trades have the exact size `size` (base units).
    RepeatedSize { size: f64, count: usize, of: usize },
    /// Gaps between trade events average `mean_gap_ms`, with a coefficient
    /// of variation `cv`, over `gaps` gaps. `dominant_size` is set when the
    /// cadence is that of the repeated-size trades only.
    Metronome {
        mean_gap_ms: f64,
        cv: f64,
        gaps: usize,
        dominant_size: Option<f64>,
    },
    /// `pairs` buy/sell pairs of the same size, close in time and price,
    /// cover `share` of the trades.
    PingPong { pairs: usize, share: f64 },
    /// `levels` consecutive `side` levels of about `size` (base units), at
    /// steps of about `step_pct` percent.
    Ladder {
        side: BookSide,
        levels: usize,
        size: f64,
        step_pct: f64,
    },
}

/// The bot-pattern result of one summary.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BotPatterns {
    /// Trades the tape signals read (0 when there was no tape).
    pub trades_sampled: usize,
    /// Why the tape signals did not run, when they did not.
    pub tape_note: Option<String>,
    /// The signals found.
    pub findings: Vec<Finding>,
    /// Share of the sampled trades, in percent, that a repeated-size or
    /// ping-pong signal covers. Reported only: it changes no volume figure.
    pub bot_like_pct: Option<f64>,
    /// The same trades' share of the sampled volume (price × quantity), in
    /// percent: many tiny bot trades can be little of the volume.
    pub bot_like_volume_pct: Option<f64>,
    /// True when a real exchange book was read for ladders.
    pub book_checked: bool,
}

/// A trade size as a comparable key.
fn size_key(quantity: f64) -> i64 {
    (quantity * SIZE_DECIMALS).round() as i64
}

/// The most common exact size and the trades that have it.
fn dominant_size(trades: &[Trade]) -> Option<(f64, Vec<usize>)> {
    let mut by_size: std::collections::HashMap<i64, Vec<usize>> = Default::default();
    for (i, t) in trades.iter().enumerate() {
        if t.quantity > 0.0 {
            by_size.entry(size_key(t.quantity)).or_default().push(i);
        }
    }
    by_size
        .into_values()
        .max_by(|a, b| a.len().cmp(&b.len()).then(b[0].cmp(&a[0])))
        .map(|idx| (trades[idx[0]].quantity, idx))
}

/// Repeated sizes: the dominant size and the trades it covers, when it
/// reaches [`REPEATED_SIZE_SHARE`].
pub fn repeated_size(trades: &[Trade]) -> Option<(Finding, Vec<usize>)> {
    if trades.len() < MIN_TRADES {
        return None;
    }
    let (size, idx) = dominant_size(trades)?;
    (idx.len() as f64 / trades.len() as f64 >= REPEATED_SIZE_SHARE).then_some((
        Finding::RepeatedSize {
            size,
            count: idx.len(),
            of: trades.len(),
        },
        idx,
    ))
}

/// Gaps between trade events. Trades with the same timestamp are one event:
/// one taker order that fills several makers prints them together.
fn event_gaps(mut times: Vec<u64>) -> Vec<f64> {
    times.sort_unstable();
    times.dedup();
    times.windows(2).map(|w| (w[1] - w[0]) as f64).collect()
}

/// Mean and coefficient of variation of `gaps`.
fn mean_cv(gaps: &[f64]) -> Option<(f64, f64)> {
    let n = gaps.len() as f64;
    let mean = gaps.iter().sum::<f64>() / n;
    if mean <= 0.0 {
        return None;
    }
    let var = gaps.iter().map(|g| (g - mean).powi(2)).sum::<f64>() / n;
    Some((mean, var.sqrt() / mean))
}

/// Metronome timing: the whole tape, else the dominant size's trades, at a
/// near-fixed interval.
pub fn metronome(trades: &[Trade]) -> Option<Finding> {
    if trades.len() < MIN_TRADES {
        return None;
    }
    let judge = |times: Vec<u64>, dominant: Option<f64>| {
        let gaps = event_gaps(times);
        if gaps.len() < MIN_GAPS {
            return None;
        }
        let (mean, cv) = mean_cv(&gaps)?;
        (cv < METRONOME_MAX_CV).then_some(Finding::Metronome {
            mean_gap_ms: mean,
            cv,
            gaps: gaps.len(),
            dominant_size: dominant,
        })
    };
    judge(trades.iter().map(|t| t.timestamp_ms).collect(), None).or_else(|| {
        let (size, idx) = dominant_size(trades)?;
        judge(
            idx.iter().map(|&i| trades[i].timestamp_ms).collect(),
            Some(size),
        )
    })
}

/// Ping-pong: buy/sell pairs of the same size within
/// [`PING_PONG_WINDOW_MS`] and [`PING_PONG_PRICE_TOLERANCE_PCT`]. Each trade
/// is in one pair at most. Returns the finding and the paired trades.
pub fn ping_pong(trades: &[Trade]) -> Option<(Finding, Vec<usize>)> {
    if trades.len() < MIN_TRADES {
        return None;
    }
    let mut order: Vec<usize> = (0..trades.len()).collect();
    order.sort_by_key(|&i| trades[i].timestamp_ms);
    let mut paired = vec![false; trades.len()];
    let mut members = Vec::new();
    for (pos, &i) in order.iter().enumerate() {
        if paired[i] {
            continue;
        }
        let a = &trades[i];
        for &j in &order[pos + 1..] {
            let b = &trades[j];
            if b.timestamp_ms - a.timestamp_ms > PING_PONG_WINDOW_MS {
                break;
            }
            let opposite = matches!(
                (&a.side, &b.side),
                (TradeSide::Buy, TradeSide::Sell) | (TradeSide::Sell, TradeSide::Buy)
            );
            let near = a.price > 0.0
                && ((b.price - a.price).abs() / a.price * 100.0) <= PING_PONG_PRICE_TOLERANCE_PCT;
            if !paired[j] && opposite && near && size_key(a.quantity) == size_key(b.quantity) {
                paired[i] = true;
                paired[j] = true;
                members.extend([i, j]);
                break;
            }
        }
    }
    let pairs = members.len() / 2;
    let share = members.len() as f64 / trades.len() as f64;
    (pairs >= MIN_PING_PONG_PAIRS && share >= PING_PONG_SHARE)
        .then_some((Finding::PingPong { pairs, share }, members))
}

/// The longest ladder in `levels` (best first): consecutive levels of about
/// one size at about one price step.
fn ladder_side(levels: &[OrderBookLevel], side: BookSide) -> Option<Finding> {
    let levels: Vec<&OrderBookLevel> = levels
        .iter()
        .filter(|l| l.price > 0.0 && l.quantity > 0.0)
        .take(LADDER_DEPTH)
        .collect();
    let mut best: Option<(usize, usize)> = None; // (start, length)
    let mut start = 0;
    while start + 1 < levels.len() {
        let size = levels[start].quantity;
        let step = (levels[start + 1].price - levels[start].price).abs();
        let mut end = start + 1;
        while end < levels.len() {
            let same_size = (levels[end].quantity - size).abs() <= size * LADDER_SIZE_TOLERANCE;
            let this_step = (levels[end].price - levels[end - 1].price).abs();
            let same_step = step > 0.0 && (this_step - step).abs() <= step * LADDER_STEP_TOLERANCE;
            if !(same_size && same_step) {
                break;
            }
            end += 1;
        }
        let len = end - start;
        if best.is_none_or(|(_, l)| len > l) {
            best = Some((start, len));
        }
        start = end.max(start + 1);
    }
    let (s, len) = best?;
    if len < LADDER_MIN_LEVELS {
        return None;
    }
    let step = (levels[s + 1].price - levels[s].price).abs();
    Some(Finding::Ladder {
        side,
        levels: len,
        size: levels[s].quantity,
        step_pct: step / levels[s].price * 100.0,
    })
}

/// Ladder quotes on either side of a real exchange book.
pub fn ladders(bids: &[OrderBookLevel], asks: &[OrderBookLevel]) -> Vec<Finding> {
    [
        ladder_side(bids, BookSide::Bid),
        ladder_side(asks, BookSide::Ask),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Runs every signal. `trades` is the venue's recent tape, or why there is
/// none. `book` is the bids and asks of a real exchange book, or `None`
/// for a synthetic one (a DEX pool's curve is an even ladder by design).
pub fn detect(
    trades: Result<&[Trade], &str>,
    book: Option<(&[OrderBookLevel], &[OrderBookLevel])>,
) -> BotPatterns {
    let mut findings = Vec::new();
    let mut tape_note = None;
    let mut bot_like_pct = None;
    let mut bot_like_volume_pct = None;
    let mut sampled = 0;
    match trades {
        Err(why) => tape_note = Some(why.to_string()),
        Ok(t) if t.len() < MIN_TRADES => {
            sampled = t.len();
            tape_note = Some(format!(
                "only {} recent trades; the tape signals need {MIN_TRADES}",
                t.len()
            ));
        }
        Ok(t) => {
            sampled = t.len();
            let mut covered = vec![false; t.len()];
            if let Some((f, idx)) = repeated_size(t) {
                idx.iter().for_each(|&i| covered[i] = true);
                findings.push(f);
            }
            if let Some(f) = metronome(t) {
                findings.push(f);
            }
            if let Some((f, idx)) = ping_pong(t) {
                idx.iter().for_each(|&i| covered[i] = true);
                findings.push(f);
            }
            let n = covered.iter().filter(|c| **c).count();
            bot_like_pct = Some(n as f64 / t.len() as f64 * 100.0);
            let value = |tr: &Trade| tr.quote_quantity.unwrap_or(tr.price * tr.quantity);
            let total: f64 = t.iter().map(value).sum();
            let bot: f64 = t
                .iter()
                .zip(&covered)
                .filter(|(_, c)| **c)
                .map(|(tr, _)| value(tr))
                .sum();
            bot_like_volume_pct = (total > 0.0).then(|| bot / total * 100.0);
        }
    }
    if let Some((bids, asks)) = book {
        findings.extend(ladders(bids, asks));
    }
    BotPatterns {
        trades_sampled: sampled,
        tape_note,
        findings,
        bot_like_pct,
        bot_like_volume_pct,
        book_checked: book.is_some(),
    }
}

/// A size or step in short form: up to 6 significant digits.
fn short(v: f64) -> String {
    let s = format!("{:.6}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if v.abs() >= 1000.0 {
        format!("{v:.0}")
    } else {
        s.to_string()
    }
}

impl Finding {
    /// One clause of evidence for the advisory line.
    pub fn describe(&self) -> String {
        match self {
            Finding::RepeatedSize { size, count, of } => format!(
                "{:.0}% of {of} trades are exactly {}",
                *count as f64 / *of as f64 * 100.0,
                short(*size)
            ),
            Finding::Metronome {
                mean_gap_ms,
                cv,
                gaps,
                dominant_size,
            } => {
                let whose = dominant_size
                    .map(|s| format!("size-{} trades", short(s)))
                    .unwrap_or_else(|| "trades".to_string());
                format!(
                    "{whose} every {:.1}s ±{:.0}% over {gaps} gaps",
                    mean_gap_ms / 1000.0,
                    cv * 100.0
                )
            }
            Finding::PingPong { pairs, share } => format!(
                "{pairs} buy/sell ping-pong pairs ({:.0}% of trades)",
                share * 100.0
            ),
            Finding::Ladder {
                side,
                levels,
                size,
                step_pct,
            } => format!(
                "{} ladder of {levels} levels × {} at {}% steps",
                match side {
                    BookSide::Bid => "bid",
                    BookSide::Ask => "ask",
                },
                short(*size),
                short(*step_pct)
            ),
        }
    }
}

impl BotPatterns {
    /// The health check line. A finding warns; nothing found passes when
    /// the tape was read; with no tape and no ladder it does not apply.
    pub fn check(&self) -> super::types::HealthCheck {
        use super::types::HealthCheck;
        if !self.findings.is_empty() {
            let evidence: Vec<String> = self.findings.iter().map(Finding::describe).collect();
            let share = match (
                self.bot_like_pct.filter(|p| *p > 0.0),
                self.bot_like_volume_pct,
            ) {
                (Some(p), Some(v)) => {
                    format!("; {p:.0}% of sampled trades ({v:.1}% of their volume) look bot-made")
                }
                (Some(p), None) => format!("; {p:.0}% of sampled trades look bot-made"),
                _ => String::new(),
            };
            return HealthCheck::Warn(format!(
                "Bot patterns (likely market-making bots): {}{share}",
                evidence.join("; ")
            ));
        }
        let book = if self.book_checked {
            " or the book"
        } else {
            ""
        };
        match &self.tape_note {
            None => HealthCheck::Pass(format!(
                "No bot patterns in {} recent trades{book}",
                self.trades_sampled
            )),
            Some(why) if self.book_checked => HealthCheck::NotApplicable(format!(
                "Bot patterns: no ladder in the book; trades not checked ({why})"
            )),
            Some(why) => HealthCheck::NotApplicable(format!("Bot patterns: not checked ({why})")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trade(ms: u64, price: f64, qty: f64, side: TradeSide) -> Trade {
        Trade {
            price,
            quantity: qty,
            quote_quantity: None,
            timestamp_ms: ms,
            side,
            id: None,
        }
    }

    /// An organic-looking tape: varied sizes, irregular gaps, sides mixed.
    fn organic(n: usize) -> Vec<Trade> {
        let mut ms = 1_000_000;
        (0..n)
            .map(|i| {
                ms += 300 + (i as u64 * 7919) % 4_000;
                let qty = 10.0 + ((i * 37) % 97) as f64 * 1.37;
                let side = if (i * 13) % 3 == 0 {
                    TradeSide::Buy
                } else {
                    TradeSide::Sell
                };
                trade(ms, 1.0 + ((i * 11) % 7) as f64 * 0.0001, qty, side)
            })
            .collect()
    }

    fn level(price: f64, quantity: f64) -> OrderBookLevel {
        OrderBookLevel { price, quantity }
    }

    #[test]
    fn test_an_organic_tape_and_book_show_nothing() {
        let t = organic(200);
        let bids: Vec<_> = (0..20)
            .map(|i| {
                level(
                    0.999 - i as f64 * 0.0001 * (1.0 + (i % 3) as f64),
                    100.0 + (i * 53 % 211) as f64,
                )
            })
            .collect();
        let p = detect(Ok(&t), Some((&bids, &[])));
        assert!(p.findings.is_empty(), "{:?}", p.findings);
        assert_eq!(p.bot_like_pct, Some(0.0));
        assert!(matches!(
            p.check(),
            crate::market::types::HealthCheck::Pass(_)
        ));
    }

    #[test]
    fn test_one_size_in_a_third_of_the_tape_is_repeated_size() {
        let mut t = organic(100);
        for tr in t.iter_mut().step_by(3) {
            tr.quantity = 1000.0;
        }
        let (f, idx) = repeated_size(&t).unwrap();
        assert_eq!(idx.len(), 34);
        assert!(matches!(f, Finding::RepeatedSize { size, count: 34, of: 100 } if size == 1000.0));
        assert!(
            f.describe().contains("34% of 100 trades are exactly 1000"),
            "{}",
            f.describe()
        );
        // A quarter is not enough.
        let mut t = organic(100);
        for tr in t.iter_mut().step_by(4) {
            tr.quantity = 1000.0;
        }
        assert!(repeated_size(&t).is_none());
    }

    #[test]
    fn test_a_fixed_cadence_is_a_metronome_and_same_ms_fills_are_one_event() {
        // 60 trades every 2s, some printed twice in the same millisecond.
        let mut t = Vec::new();
        for i in 0..60u64 {
            t.push(trade(i * 2_000, 1.0, 5.0 + i as f64, TradeSide::Buy));
            if i % 5 == 0 {
                t.push(trade(i * 2_000, 1.0, 7.0, TradeSide::Buy));
            }
        }
        let Some(Finding::Metronome {
            mean_gap_ms,
            cv,
            dominant_size: None,
            ..
        }) = metronome(&t)
        else {
            panic!("no metronome")
        };
        assert_eq!(mean_gap_ms, 2_000.0);
        assert!(cv < 1e-9);
        assert!(metronome(&organic(200)).is_none());
    }

    #[test]
    fn test_a_bot_cadence_hidden_in_organic_trades_is_found_by_its_size() {
        // Organic trades plus a bot printing size 250 every 3s.
        let mut t = organic(80);
        let start = t[0].timestamp_ms;
        for i in 0..45u64 {
            t.push(trade(start + 7 + i * 3_000, 1.0, 250.0, TradeSide::Sell));
        }
        let Some(Finding::Metronome {
            dominant_size: Some(s),
            mean_gap_ms,
            ..
        }) = metronome(&t)
        else {
            panic!("no metronome: {:?}", metronome(&t))
        };
        assert_eq!(s, 250.0);
        assert_eq!(mean_gap_ms, 3_000.0);
    }

    #[test]
    fn test_buy_sell_pairs_of_one_size_are_ping_pong() {
        let mut t = organic(60);
        let start = t.last().unwrap().timestamp_ms + 10_000;
        for i in 0..15u64 {
            let ms = start + i * 20_000;
            let qty = 100.0 + i as f64; // a new size per pair
            t.push(trade(ms, 1.0, qty, TradeSide::Buy));
            t.push(trade(ms + 800, 1.0002, qty, TradeSide::Sell));
        }
        let (f, idx) = ping_pong(&t).unwrap();
        assert_eq!(idx.len(), 30);
        assert!(matches!(f, Finding::PingPong { pairs: 15, .. }), "{f:?}");
        // Too far apart in time, or in price, is not a pair.
        let mut slow = organic(60);
        for i in 0..15u64 {
            let ms = 9_000_000 + i * 20_000;
            slow.push(trade(ms, 1.0, 50.5, TradeSide::Buy));
            slow.push(trade(
                ms + PING_PONG_WINDOW_MS + 1,
                1.0,
                50.5,
                TradeSide::Sell,
            ));
        }
        assert!(ping_pong(&slow).is_none());
        let mut far = organic(60);
        for i in 0..15u64 {
            let ms = 9_000_000 + i * 20_000;
            far.push(trade(ms, 1.0, 50.5, TradeSide::Buy));
            far.push(trade(ms + 100, 1.01, 50.5, TradeSide::Sell));
        }
        assert!(ping_pong(&far).is_none());
    }

    #[test]
    fn test_many_dust_bot_trades_are_little_of_the_volume() {
        // Seen live on BTC/USDT: most trades are one tiny size.
        let mut t: Vec<Trade> = (0..90)
            .map(|i| trade(1_000 + i * 977, 100.0, 0.001, TradeSide::Buy))
            .collect();
        t.extend(
            (0..10).map(|i| trade(500_000 + i * 3_331, 100.0, 50.0 + i as f64, TradeSide::Sell)),
        );
        let p = detect(Ok(&t), None);
        assert_eq!(p.bot_like_pct, Some(90.0));
        let v = p.bot_like_volume_pct.unwrap();
        assert!(v < 0.05, "{v}");
        let crate::market::types::HealthCheck::Warn(m) = p.check() else {
            panic!()
        };
        assert!(
            m.contains("90% of sampled trades (0.0% of their volume) look bot-made"),
            "{m}"
        );
    }

    #[test]
    fn test_equal_levels_at_even_steps_are_a_ladder() {
        let asks: Vec<_> = (0..12)
            .map(|i| level(1.0001 + i as f64 * 0.0001, 5_000.0))
            .collect();
        let bids: Vec<_> = (0..12)
            .map(|i| level(0.9999 - i as f64 * 0.0001, 100.0 + i as f64 * 9.0))
            .collect();
        let found = ladders(&bids, &asks);
        assert_eq!(found.len(), 1);
        let Finding::Ladder {
            side,
            levels,
            size,
            step_pct,
        } = &found[0]
        else {
            panic!()
        };
        assert_eq!((*side, *levels, *size), (BookSide::Ask, 12, 5_000.0));
        assert!((step_pct - 0.01).abs() < 0.001, "{step_pct}");
        // Seven levels are not a ladder.
        let short_run: Vec<_> = (0..7)
            .map(|i| level(1.0 + i as f64 * 0.0001, 5_000.0))
            .collect();
        assert!(ladders(&[], &short_run).is_empty());
    }

    #[test]
    fn test_findings_warn_with_their_evidence_and_the_bot_like_share() {
        let mut t = organic(100);
        for tr in t.iter_mut().step_by(2) {
            tr.quantity = 1000.0;
        }
        let p = detect(Ok(&t), None);
        let crate::market::types::HealthCheck::Warn(m) = p.check() else {
            panic!("{:?}", p.check())
        };
        assert!(
            m.starts_with(
                "Bot patterns (likely market-making bots): 50% of 100 trades are exactly 1000"
            ),
            "{m}"
        );
        assert!(m.contains("50% of sampled trades ("), "{m}");
        assert!(m.contains("% of their volume) look bot-made"), "{m}");
        assert_eq!(p.bot_like_pct, Some(50.0));
    }

    #[test]
    fn test_no_tape_or_a_short_tape_says_why_and_still_reads_the_book() {
        let p = detect(Err("the venue gives no trade feed"), None);
        assert_eq!(p.trades_sampled, 0);
        let crate::market::types::HealthCheck::NotApplicable(m) = p.check() else {
            panic!()
        };
        assert!(m.contains("the venue gives no trade feed"), "{m}");
        // A ladder still warns without a tape.
        let asks: Vec<_> = (0..10)
            .map(|i| level(1.0 + i as f64 * 0.001, 42.0))
            .collect();
        let p = detect(Err("x"), Some((&[], &asks)));
        assert!(matches!(
            p.check(),
            crate::market::types::HealthCheck::Warn(_)
        ));
        // Fewer than MIN_TRADES: the tape signals do not run.
        let t = organic(MIN_TRADES - 1);
        let p = detect(Ok(&t), None);
        assert!(p.tape_note.unwrap().contains("only 29 recent trades"));
        assert!(p.bot_like_pct.is_none());
    }

    #[test]
    fn test_each_tape_signal_alone_refuses_a_short_tape() {
        // detect() checks the length first, but the signals are public and
        // must not judge a tape too short on their own either: 29 trades of
        // one size, every 2s, in buy/sell pairs, would trip all three.
        let t: Vec<Trade> = (0..MIN_TRADES as u64 - 1)
            .map(|i| {
                let side = if i % 2 == 0 {
                    TradeSide::Buy
                } else {
                    TradeSide::Sell
                };
                trade(i * 2_000, 1.0, 10.0, side)
            })
            .collect();
        assert!(repeated_size(&t).is_none());
        assert!(metronome(&t).is_none());
        assert!(ping_pong(&t).is_none());
    }

    #[test]
    fn test_a_bid_ladder_is_described_as_a_bid_ladder() {
        let bids: Vec<_> = (0..9)
            .map(|i| level(0.999 - i as f64 * 0.001, 250.0))
            .collect();
        let found = ladders(&bids, &[]);
        assert_eq!(found.len(), 1);
        let d = found[0].describe();
        assert!(d.starts_with("bid ladder of 9 levels × 250 at "), "{d}");
    }

    #[test]
    fn test_a_tape_with_no_volume_gives_the_trade_share_only() {
        // Zero-price prints have no volume to take a share of, so the
        // advisory gives the trade share alone.
        let t: Vec<Trade> = (0..40)
            .map(|i| trade(1_000 + i * 977, 0.0, 3.0, TradeSide::Buy))
            .collect();
        let p = detect(Ok(&t), None);
        assert_eq!(p.bot_like_pct, Some(100.0));
        assert_eq!(p.bot_like_volume_pct, None);
        let crate::market::types::HealthCheck::Warn(m) = p.check() else {
            panic!("{:?}", p.check())
        };
        assert!(m.ends_with("; 100% of sampled trades look bot-made"), "{m}");
    }
}
