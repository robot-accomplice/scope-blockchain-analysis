//! Bot-pattern signals in a venue's recent trades and order book, for the
//! market summary's health advisory.
//!
//! Market making is legitimate, so a signal never fails a summary: it warns,
//! with the evidence, where the tape or the book looks machine-made. The
//! signals:
//!
//! - **Repeated sizes:** one exact trade size makes up a large share of the
//!   tape, or a few sizes make up half of it.
//! - **Metronome timing:** trades come at an almost fixed interval.
//! - **Cadence:** most gaps between trades are one interval, even when a bot
//!   trades in bursts with pauses between them (so the gaps as a whole are
//!   not a metronome).
//! - **Jittered timer:** the gaps are not one interval but stay in a band
//!   (30–60 s, say), never much shorter: a timer with random jitter. Real
//!   order flow has many near-zero gaps.
//! - **Ping-pong:** a buy and a sell of the same size at nearly the same
//!   price, seconds apart (the shape of wash trading).
//! - **Ladder quotes:** consecutive book levels at even price steps whose
//!   sizes are equal, or rise or fall level by level (a quoting curve).
//! - **Round notionals:** most of a side's levels are whole quote amounts
//!   (560.00 USDT) though their sizes are not whole: a bot sizes its quotes
//!   by value; people size orders by quantity.
//!
//! Across the runs of a repeating summary, one more: a **depth cycle**, a
//! side of the book refilled on a timer and drained between the refills
//! ([`depth_cycle`]). One run cannot see it.
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

/// How many sizes the few-sizes signal adds up.
pub const FEW_SIZES: usize = 3;

/// The [`FEW_SIZES`] most common sizes in at least this share of the trades
/// are a signal.
pub const FEW_SIZES_SHARE: f64 = 0.50;

/// A size counts toward the few-sizes signal only with at least this share
/// of the trades on its own.
pub const FEW_SIZES_MIN_SHARE: f64 = 0.05;

/// Fewest gaps between trade events the timing signal needs.
pub const MIN_GAPS: usize = 20;

/// Gaps whose standard deviation is under this share of their mean are a
/// metronome.
pub const METRONOME_MAX_CV: f64 = 0.10;

/// At least this share of the gaps within [`TIMER_BAND`] of the median gap,
/// with the floor below, is a jittered timer. Random arrivals put about 35%
/// there. A share, not the gaps' spread: a bot that pauses between bursts
/// has a few long gaps that blow the spread up but leave the share.
pub const TIMER_BAND_SHARE: f64 = 0.70;

/// The band around the median gap that a jittered timer stays in: ±50%.
pub const TIMER_BAND: f64 = 0.50;

/// The 5th-percentile gap at or above this share of the median is a floor:
/// a timer never fires early. Real order flow puts it near 0.
pub const TIMER_MIN_FLOOR: f64 = 0.40;

/// Gaps within this share of an interval count as that interval.
pub const CADENCE_TOLERANCE: f64 = 0.02;

/// Gaps at one interval, as a share of all gaps, at or above this are a
/// cadence. Random arrivals put at most about 1.5% of gaps in any ±2% band.
pub const CADENCE_SHARE: f64 = 0.30;

/// Shortest interval the cadence signal judges: some venues print whole
/// seconds, which would put many short random gaps on one value.
pub const CADENCE_MIN_GAP_MS: f64 = 5_000.0;

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

/// A level at most this much larger (or smaller) than the one before does
/// not end a falling (or rising) ladder: one bump in a quoting curve.
pub const LADDER_SHAPE_SLACK: f64 = 0.05;

/// The round-notional signal reads this many levels from the top of a side.
pub const ROUND_LEVELS: usize = 10;

/// At least this many of those levels with a whole quote amount are a
/// signal.
pub const ROUND_MIN_LEVELS: usize = 8;

/// A notional within this of a whole number is whole.
const ROUND_TOLERANCE: f64 = 0.005;

/// Price steps of a ladder differ by at most this share of the first step.
pub const LADDER_STEP_TOLERANCE: f64 = 0.05;

/// A run-to-run rise of at least this share of the depth is a refill.
pub const CYCLE_JUMP: f64 = 0.25;

/// Fewest whole cycles (refills after the first) that make a depth cycle.
pub const CYCLE_MIN: usize = 3;

/// Fewest runs between two refills: a depth that jumps every other run is
/// noise, not a cycle.
pub const CYCLE_MIN_RUNS: usize = 3;

/// Periods between refills whose standard deviation is under this share of
/// their mean are a timer.
pub const CYCLE_MAX_CV: f64 = 0.20;

/// Between refills, at least this share of the run-to-run steps must be
/// flat or down: the depth drains, it does not grow.
pub const CYCLE_FALLING_SHARE: f64 = 0.60;

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
    /// `count` of `of` trades have one of the exact sizes `sizes`, most
    /// common first.
    FewSizes {
        sizes: Vec<f64>,
        count: usize,
        of: usize,
    },
    /// Gaps between trade events average `mean_gap_ms`, with a coefficient
    /// of variation `cv`, over `gaps` gaps. `dominant_size` is set when the
    /// cadence is that of the repeated-size trades only.
    Metronome {
        mean_gap_ms: f64,
        cv: f64,
        gaps: usize,
        dominant_size: Option<f64>,
    },
    /// `share` of `gaps` gaps between trade events are `gap_ms` apart
    /// (within [`CADENCE_TOLERANCE`]).
    Cadence {
        gap_ms: f64,
        share: f64,
        gaps: usize,
    },
    /// `share` of `gaps` gaps between trade events run from `low_ms` to
    /// `high_ms` around `median_ms` (a timer with jitter); `pauses` gaps are
    /// far longer (the bot resting between bursts). `cv` is the spread of
    /// all the gaps.
    Timer {
        low_ms: f64,
        median_ms: f64,
        high_ms: f64,
        cv: f64,
        gaps: usize,
        share: f64,
        pauses: usize,
    },
    /// `pairs` buy/sell pairs of the same size, close in time and price,
    /// cover `share` of the trades.
    PingPong { pairs: usize, share: f64 },
    /// `levels` consecutive `side` levels at steps of about `step_pct`
    /// percent, from `size` at the first to `last_size` (base units),
    /// shaped as `shape`.
    Ladder {
        side: BookSide,
        levels: usize,
        shape: LadderShape,
        size: f64,
        last_size: f64,
        step_pct: f64,
    },
    /// `levels` of the top `of` `side` levels are whole quote amounts (the
    /// first few in `examples`) though their sizes are not whole.
    RoundNotional {
        side: BookSide,
        levels: usize,
        of: usize,
        examples: Vec<f64>,
    },
}

/// How a ladder's sizes run, from the best level out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LadderShape {
    /// One size at every level.
    Even,
    /// Each level is smaller than or as large as the one before (or at
    /// most [`LADDER_SHAPE_SLACK`] larger).
    Falling,
    /// Each level is larger than or as large as the one before (or at most
    /// [`LADDER_SHAPE_SLACK`] smaller).
    Rising,
}

/// A side of the book refilled at a near-fixed period and drained between
/// the refills: a bot quoting on a timer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DepthCycle {
    /// Mean time between refills, in milliseconds.
    pub period_ms: f64,
    /// Whole cycles seen (refills after the first).
    pub cycles: usize,
    /// Coefficient of variation of the periods.
    pub cv: f64,
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

/// The exact sizes, most common first, each with the trades that have it.
fn sizes_by_count(trades: &[Trade]) -> Vec<(f64, Vec<usize>)> {
    let mut by_size: std::collections::HashMap<i64, Vec<usize>> = Default::default();
    for (i, t) in trades.iter().enumerate() {
        if t.quantity > 0.0 {
            by_size.entry(size_key(t.quantity)).or_default().push(i);
        }
    }
    let mut sizes: Vec<Vec<usize>> = by_size.into_values().collect();
    // Ties go to the size seen first, so the order does not depend on the
    // hash order.
    sizes.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].cmp(&b[0])));
    sizes
        .into_iter()
        .map(|idx| (trades[idx[0]].quantity, idx))
        .collect()
}

/// The most common exact size and the trades that have it.
fn dominant_size(trades: &[Trade]) -> Option<(f64, Vec<usize>)> {
    sizes_by_count(trades).into_iter().next()
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

/// Few sizes: the [`FEW_SIZES`] most common sizes (each with at least
/// [`FEW_SIZES_MIN_SHARE`]) cover [`FEW_SIZES_SHARE`] of the trades. A bot
/// that rotates through a few sizes passes under [`repeated_size`].
pub fn few_sizes(trades: &[Trade]) -> Option<(Finding, Vec<usize>)> {
    if trades.len() < MIN_TRADES {
        return None;
    }
    let n = trades.len() as f64;
    let top: Vec<(f64, Vec<usize>)> = sizes_by_count(trades)
        .into_iter()
        .take(FEW_SIZES)
        .filter(|(_, idx)| idx.len() as f64 / n >= FEW_SIZES_MIN_SHARE)
        .collect();
    let idx: Vec<usize> = top
        .iter()
        .flat_map(|(_, idx)| idx.iter().copied())
        .collect();
    (top.len() > 1 && idx.len() as f64 / n >= FEW_SIZES_SHARE).then(|| {
        (
            Finding::FewSizes {
                sizes: top.iter().map(|(s, _)| *s).collect(),
                count: idx.len(),
                of: trades.len(),
            },
            idx,
        )
    })
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

/// Cadence: the interval (of at least [`CADENCE_MIN_GAP_MS`]) that the most
/// gaps between trade events are at, within [`CADENCE_TOLERANCE`], when
/// those gaps are at least [`CADENCE_SHARE`] of all gaps.
pub fn cadence(trades: &[Trade]) -> Option<Finding> {
    if trades.len() < MIN_TRADES {
        return None;
    }
    let mut gaps = event_gaps(trades.iter().map(|t| t.timestamp_ms).collect());
    if gaps.len() < MIN_GAPS {
        return None;
    }
    gaps.sort_by(f64::total_cmp);
    // For each gap as the interval, the gaps in its band: two pointers over
    // the sorted gaps.
    let (mut lo, mut hi) = (0, 0);
    let mut best: Option<(usize, usize)> = None; // (count, index of the interval)
    for (i, &g) in gaps.iter().enumerate() {
        if g < CADENCE_MIN_GAP_MS {
            continue;
        }
        while gaps[lo] < g * (1.0 - CADENCE_TOLERANCE) {
            lo += 1;
        }
        hi = hi.max(i);
        while hi + 1 < gaps.len() && gaps[hi + 1] <= g * (1.0 + CADENCE_TOLERANCE) {
            hi += 1;
        }
        let count = hi + 1 - lo;
        if best.is_none_or(|(c, _)| count > c) {
            best = Some((count, i));
        }
    }
    let (count, i) = best?;
    let share = count as f64 / gaps.len() as f64;
    (share >= CADENCE_SHARE).then_some(Finding::Cadence {
        gap_ms: gaps[i],
        share,
        gaps: gaps.len(),
    })
}

/// Depth cycle: in `points` (time in ms, depth), oldest first, a refill (a
/// rise of [`CYCLE_JUMP`]) at least [`CYCLE_MIN`] + 1 times, at least
/// [`CYCLE_MIN_RUNS`] runs apart, at a period within [`CYCLE_MAX_CV`], with
/// the depth flat or falling for [`CYCLE_FALLING_SHARE`] of the steps
/// between.
pub fn depth_cycle(points: &[(i64, f64)]) -> Option<DepthCycle> {
    let refills: Vec<usize> = (1..points.len())
        .filter(|&i| {
            let (a, b) = (points[i - 1].1, points[i].1);
            a > 0.0 && b >= a * (1.0 + CYCLE_JUMP)
        })
        .collect();
    if refills.len() < CYCLE_MIN + 1 || refills.windows(2).any(|w| w[1] - w[0] < CYCLE_MIN_RUNS) {
        return None;
    }
    let periods: Vec<f64> = refills
        .windows(2)
        .map(|w| (points[w[1]].0 - points[w[0]].0) as f64)
        .collect();
    let (mean, cv) = mean_cv(&periods)?;
    if cv >= CYCLE_MAX_CV {
        return None;
    }
    let steps: Vec<bool> = refills
        .windows(2)
        .flat_map(|w| w[0] + 1..w[1])
        .map(|i| points[i].1 <= points[i - 1].1 * (1.0 + LADDER_SIZE_TOLERANCE))
        .collect();
    let falling = steps.iter().filter(|f| **f).count() as f64 / steps.len() as f64;
    (falling >= CYCLE_FALLING_SHARE).then_some(DepthCycle {
        period_ms: mean,
        cycles: refills.len() - 1,
        cv,
    })
}

/// Jittered timer: gaps between trade events with a median of at least
/// [`CADENCE_MIN_GAP_MS`], at least [`TIMER_BAND_SHARE`] of them within
/// [`TIMER_BAND`] of the median, and a 5th percentile at least
/// [`TIMER_MIN_FLOOR`] of the median.
pub fn timer(trades: &[Trade]) -> Option<Finding> {
    if trades.len() < MIN_TRADES {
        return None;
    }
    let mut gaps = event_gaps(trades.iter().map(|t| t.timestamp_ms).collect());
    if gaps.len() < MIN_GAPS {
        return None;
    }
    let (_, cv) = mean_cv(&gaps)?;
    gaps.sort_by(f64::total_cmp);
    let at = |q: f64| gaps[((gaps.len() - 1) as f64 * q).round() as usize];
    let (floor, median) = (at(0.05), at(0.5));
    let band: Vec<f64> = gaps
        .iter()
        .copied()
        .filter(|g| (g - median).abs() <= median * TIMER_BAND)
        .collect();
    let share = band.len() as f64 / gaps.len() as f64;
    (median >= CADENCE_MIN_GAP_MS && share >= TIMER_BAND_SHARE && floor >= median * TIMER_MIN_FLOOR)
        .then(|| Finding::Timer {
            // The band's own range: what the timer's gaps are.
            low_ms: band[0],
            median_ms: median,
            high_ms: band[band.len() - 1],
            cv,
            gaps: gaps.len(),
            share,
            pauses: gaps
                .iter()
                .filter(|g| **g > median * (1.0 + TIMER_BAND) * 4.0)
                .count(),
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

/// -1, 0 or 1 as `b` is smaller than, about (within
/// [`LADDER_SIZE_TOLERANCE`]) or larger than `a`.
fn size_direction(a: f64, b: f64) -> i8 {
    if (b - a).abs() <= a * LADDER_SIZE_TOLERANCE {
        0
    } else if b > a {
        1
    } else {
        -1
    }
}

/// The longest ladder in `levels` (best first): consecutive levels at about
/// one price step whose sizes stay equal or run one way, never both.
fn ladder_side(levels: &[OrderBookLevel], side: BookSide) -> Option<Finding> {
    let levels: Vec<&OrderBookLevel> = levels
        .iter()
        .filter(|l| l.price > 0.0 && l.quantity > 0.0)
        .take(LADDER_DEPTH)
        .collect();
    let mut best: Option<(usize, usize, i8)> = None; // (start, length, direction)
    let mut start = 0;
    while start + 1 < levels.len() {
        let step = (levels[start + 1].price - levels[start].price).abs();
        let mut dir = 0;
        let mut end = start + 1;
        while end < levels.len() {
            let this_step = (levels[end].price - levels[end - 1].price).abs();
            let same_step = step > 0.0 && (this_step - step).abs() <= step * LADDER_STEP_TOLERANCE;
            let (a, b) = (levels[end - 1].quantity, levels[end].quantity);
            let mut d = size_direction(a, b);
            // One bump of a few percent against the run does not end it.
            if d != 0 && dir != 0 && d != dir && (b - a).abs() <= a * LADDER_SHAPE_SLACK {
                d = 0;
            }
            if !same_step || (d != 0 && dir != 0 && d != dir) {
                break;
            }
            if d != 0 {
                dir = d;
            }
            end += 1;
        }
        let len = end - start;
        if best.is_none_or(|(_, l, _)| len > l) {
            best = Some((start, len, dir));
        }
        // The level a run ended on can start the next one (the top of a
        // falling curve after a rising first level).
        start = (end - 1).max(start + 1);
    }
    let (s, len, dir) = best?;
    if len < LADDER_MIN_LEVELS {
        return None;
    }
    let (first, last) = (levels[s].quantity, levels[s + len - 1].quantity);
    let even = levels[s..s + len]
        .iter()
        .all(|l| size_direction(first, l.quantity) == 0);
    let shape = match dir {
        _ if even => LadderShape::Even,
        1 => LadderShape::Rising,
        _ => LadderShape::Falling,
    };
    let step = (levels[s + 1].price - levels[s].price).abs();
    Some(Finding::Ladder {
        side,
        levels: len,
        shape,
        size: first,
        last_size: last,
        step_pct: step / levels[s].price * 100.0,
    })
}

/// Round notionals on one side: of the top [`ROUND_LEVELS`] levels, those
/// whose price × size is a whole quote amount while the size is not whole.
/// A whole size at a whole price is a person's order, not a sign.
fn round_notional_side(levels: &[OrderBookLevel], side: BookSide) -> Option<Finding> {
    let top: Vec<&OrderBookLevel> = levels
        .iter()
        .filter(|l| l.price > 0.0 && l.quantity > 0.0)
        .take(ROUND_LEVELS)
        .collect();
    let round: Vec<f64> = top
        .iter()
        .filter_map(|l| {
            let notional = l.price * l.quantity;
            let whole = (notional - notional.round()).abs() < ROUND_TOLERANCE;
            let whole_size = (l.quantity - l.quantity.round()).abs() < 1e-9;
            (whole && !whole_size && notional.round() > 0.0).then_some(notional.round())
        })
        .collect();
    (round.len() >= ROUND_MIN_LEVELS).then(|| Finding::RoundNotional {
        side,
        levels: round.len(),
        of: top.len(),
        examples: round.into_iter().take(3).collect(),
    })
}

/// Round notionals on either side of a real exchange book.
pub fn round_notionals(bids: &[OrderBookLevel], asks: &[OrderBookLevel]) -> Vec<Finding> {
    [
        round_notional_side(bids, BookSide::Bid),
        round_notional_side(asks, BookSide::Ask),
    ]
    .into_iter()
    .flatten()
    .collect()
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
            // A few sizes only when no one size is enough on its own.
            if let Some((f, idx)) = repeated_size(t).or_else(|| few_sizes(t)) {
                idx.iter().for_each(|&i| covered[i] = true);
                findings.push(f);
            }
            // A cadence only when the gaps are not a metronome already.
            // A jittered timer only when no stricter timing signal fits.
            if let Some(f) = metronome(t).or_else(|| cadence(t)).or_else(|| timer(t)) {
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
            // `+ 0.0`: an empty f64 sum is -0.0, which would print as `-0.0`.
            bot_like_volume_pct = (total > 0.0).then(|| bot / total * 100.0 + 0.0);
        }
    }
    if let Some((bids, asks)) = book {
        findings.extend(ladders(bids, asks));
        findings.extend(round_notionals(bids, asks));
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

/// A price step in percent, to 2 significant digits: a 1-tick step of
/// 0.010002% reads `0.01`.
fn step(pct: f64) -> String {
    if pct <= 0.0 {
        return short(pct);
    }
    let decimals = (1 - pct.log10().floor() as i32).max(0) as usize;
    let s = format!("{pct:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

impl Finding {
    /// The short name of the signal: `Jittered timer`, `Bid ladder`.
    pub fn label(&self) -> String {
        let side = |side: &BookSide| match side {
            BookSide::Bid => "Bid",
            BookSide::Ask => "Ask",
        };
        match self {
            Finding::RepeatedSize { .. } => "Repeated size".to_string(),
            Finding::FewSizes { .. } => "Few sizes".to_string(),
            Finding::Metronome { .. } => "Metronome".to_string(),
            Finding::Cadence { .. } => "Cadence".to_string(),
            Finding::Timer { .. } => "Jittered timer".to_string(),
            Finding::PingPong { .. } => "Ping-pong".to_string(),
            Finding::Ladder { side: s, .. } => format!("{} ladder", side(s)),
            Finding::RoundNotional { side: s, .. } => format!("{} round notionals", side(s)),
        }
    }

    /// True for a signal read in the book, false for one in the trades.
    pub fn in_book(&self) -> bool {
        matches!(self, Finding::Ladder { .. } | Finding::RoundNotional { .. })
    }

    /// The evidence, without the label.
    pub fn detail(&self) -> String {
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
            Finding::FewSizes { sizes, count, of } => format!(
                "{:.0}% of {of} trades are one of {} sizes: {}",
                *count as f64 / *of as f64 * 100.0,
                sizes.len(),
                sizes
                    .iter()
                    .map(|s| short(*s))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Finding::Cadence {
                gap_ms,
                share,
                gaps,
            } => format!(
                "{:.0}% of {gaps} gaps between trades are {:.1}s ±{:.0}%",
                share * 100.0,
                gap_ms / 1000.0,
                CADENCE_TOLERANCE * 100.0
            ),
            Finding::Timer {
                low_ms,
                median_ms,
                high_ms,
                gaps,
                share,
                pauses,
                ..
            } => format!(
                "{:.0}% of {gaps} gaps between trades are {:.0}–{:.0}s (median {:.0}s){}",
                share * 100.0,
                low_ms / 1000.0,
                high_ms / 1000.0,
                median_ms / 1000.0,
                match pauses {
                    0 => String::new(),
                    1 => ", in bursts with 1 long pause".to_string(),
                    n => format!(", in bursts with {n} long pauses"),
                }
            ),
            Finding::RoundNotional {
                levels,
                of,
                examples,
                ..
            } => format!(
                "{levels} of {of} levels are whole quote amounts ({}, …)",
                examples
                    .iter()
                    .map(|v| short(*v))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Finding::PingPong { pairs, share } => format!(
                "{pairs} buy/sell pairs of one size, seconds apart ({:.0}% of trades)",
                share * 100.0
            ),
            Finding::Ladder {
                levels,
                shape,
                size,
                last_size,
                step_pct,
                ..
            } => {
                let sizes = match shape {
                    LadderShape::Even => format!("× {}", short(*size)),
                    LadderShape::Falling => {
                        format!("falling {} → {}", short(*size), short(*last_size))
                    }
                    LadderShape::Rising => {
                        format!("rising {} → {}", short(*size), short(*last_size))
                    }
                };
                format!("{levels} levels {sizes} at {}% steps", step(*step_pct))
            }
        }
    }

    /// The label and the evidence: `Bid ladder: 9 levels falling …`.
    pub fn describe(&self) -> String {
        format!("{}: {}", self.label(), self.detail())
    }
}

impl BotPatterns {
    /// The share of the sampled trades, and of their volume, that a
    /// repeated-size or ping-pong signal marks as bot-made, when any is.
    pub fn share(&self) -> Option<String> {
        let p = self.bot_like_pct.filter(|p| *p > 0.0)?;
        Some(match self.bot_like_volume_pct {
            Some(v) => format!("{p:.0}% of sampled trades ({v:.1}% of their volume) look bot-made"),
            None => format!("{p:.0}% of sampled trades look bot-made"),
        })
    }

    /// The report section: the signals in the trades, then those in the
    /// book, one line each under its group, then the share.
    ///
    /// ```text
    /// ├── Bot Patterns · likely market-making bots
    /// │  Trades            500 sampled
    /// │  ! Jittered timer         90% of 499 gaps between trades are 32–58s (median 46s)
    /// │  Book              top levels of each side
    /// │  ! Bid ladder             9 levels falling 560.112 → 110.1101 at 0.01% steps
    /// ```
    pub fn format_text(&self) -> String {
        use crate::display::terminal as t;
        let width = self
            .findings
            .iter()
            .map(|f| f.label().chars().count())
            .max()
            .unwrap_or(0)
            + 2;
        let mut out = t::subsection_header("Bot Patterns · likely market-making bots");
        out.push('\n');
        let groups = [
            ("Trades", false, format!("{} sampled", self.trades_sampled)),
            ("Book", true, "top levels of each side".to_string()),
        ];
        for (name, in_book, about) in groups {
            let found: Vec<&Finding> = self
                .findings
                .iter()
                .filter(|f| f.in_book() == in_book)
                .collect();
            if found.is_empty() {
                continue;
            }
            out.push_str(&t::kv_row(name, &about));
            out.push('\n');
            for f in found {
                out.push_str(&t::labelled_warn(&f.label(), width, &f.detail()));
                out.push('\n');
            }
        }
        if let Some(note) = self
            .tape_note
            .as_ref()
            .filter(|_| self.findings.iter().all(Finding::in_book))
        {
            out.push_str(&t::kv_row("Trades", &format!("not checked ({note})")));
            out.push('\n');
        }
        if let Some(share) = self.share() {
            out.push_str(&t::kv_row("Bot-like share", &share));
            out.push('\n');
        }
        out
    }

    /// Every finding on its own line (`label: evidence`), then the share,
    /// for a report that lists them.
    pub fn evidence(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.findings.iter().map(Finding::describe).collect();
        lines.extend(self.share());
        lines
    }

    /// The health check line: a count and the signals' names, short enough
    /// for a list of checks; the evidence goes in its own section. Nothing
    /// found passes when the tape was read; with no tape and no book signal
    /// it does not apply.
    pub fn check(&self) -> super::types::HealthCheck {
        use super::types::HealthCheck;
        if !self.findings.is_empty() {
            let n = self.findings.len();
            let names: Vec<String> = self
                .findings
                .iter()
                .map(|f| f.label().to_lowercase())
                .collect();
            return HealthCheck::Warn(format!(
                "Bot patterns: {n} {} of market-making bots ({})",
                if n == 1 { "sign" } else { "signs" },
                names.join(", ")
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
                "Bot patterns: none in the book; trades not checked ({why})"
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

    /// `text` without ANSI colour codes (a test run in a terminal paints).
    fn plain(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c == '' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
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
        let crate::market::types::HealthCheck::Warn(_) = p.check() else {
            panic!()
        };
        let m = p.evidence().join("\n");
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
        // Even steps, but sizes up and down: no ladder.
        let bids: Vec<_> = (0..12)
            .map(|i| level(0.9999 - i as f64 * 0.0001, 100.0 + (i * 53 % 211) as f64))
            .collect();
        let found = ladders(&bids, &asks);
        assert_eq!(found.len(), 1);
        let Finding::Ladder {
            side,
            levels,
            shape,
            size,
            step_pct,
            ..
        } = &found[0]
        else {
            panic!()
        };
        assert_eq!(
            (*side, *levels, *shape, *size),
            (BookSide::Ask, 12, LadderShape::Even, 5_000.0)
        );
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
        let crate::market::types::HealthCheck::Warn(_) = p.check() else {
            panic!("{:?}", p.check())
        };
        // The check names the signals; the evidence is one line each.
        let m = p.evidence().join("\n");
        assert!(
            m.starts_with("Repeated size: 50% of 100 trades are exactly 1000"),
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
        assert!(d.starts_with("Bid ladder: 9 levels × 250 at "), "{d}");
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
        let crate::market::types::HealthCheck::Warn(_) = p.check() else {
            panic!("{:?}", p.check())
        };
        // The check names the signals; the evidence is one line each.
        let m = p.evidence().join("\n");
        assert!(m.ends_with("\n100% of sampled trades look bot-made"), "{m}");
    }

    /// The USDN/USDT book on weex, read live on 2026-10-24: ten levels a
    /// side at one tick, sizes falling level by level from the second.
    fn weex_book() -> (Vec<OrderBookLevel>, Vec<OrderBookLevel>) {
        let bids = [
            (0.9999, 410.046),
            (0.9998, 540.108),
            (0.9997, 450.135),
            (0.9996, 370.148),
            (0.9995, 370.185),
            (0.9994, 360.2161),
            (0.9993, 300.2101),
            (0.9992, 220.1761),
            (0.9991, 160.1441),
            (0.9990, 120.1201),
            (0.0022, 3636.3636),
        ];
        let asks = [
            (1.0001, 427.318),
            (1.0002, 1349.73),
            (1.0003, 1209.6371),
            (1.0004, 1029.5881),
            (1.0005, 959.5202),
            (1.0006, 919.4483),
            (1.0007, 799.4403),
            (1.0008, 619.5043),
            (1.0009, 399.6403),
            (1.0010, 299.7002),
            (1.0500, 4.8046),
        ];
        let side = |l: &[(f64, f64)]| l.iter().map(|&(p, q)| level(p, q)).collect();
        (side(&bids), side(&asks))
    }

    /// The USDN/USDT tape on weex, as its bot prints it: every ~29 minutes
    /// two sells, four buys of one size (498.11 or 68.01) exactly 45 s
    /// apart, then three shrinking buys, each 45 s after the last.
    fn weex_tape(cycles: u64) -> Vec<Trade> {
        let mut t = Vec::new();
        for c in 0..cycles {
            let start = 1_000_000 + c * 1_760_000;
            let burst = if c % 2 == 0 { 498.11 } else { 68.01 };
            let script = [
                (0.9999, 500.0, TradeSide::Sell),
                (0.9999, 60.05 + (c % 3) as f64 * 10.0, TradeSide::Sell),
                (1.0001, burst, TradeSide::Buy),
                (1.0001, burst, TradeSide::Buy),
                (1.0001, burst, TradeSide::Buy),
                (1.0001, burst, TradeSide::Buy),
                (1.0001, 300.0 + c as f64 * 7.3, TradeSide::Buy),
                (1.0001, 60.0 + c as f64 * 1.9, TradeSide::Buy),
                (1.0001, 12.0 + c as f64 * 0.37, TradeSide::Buy),
            ];
            for (k, (price, qty, side)) in script.into_iter().enumerate() {
                // A few ms of jitter, as the live prints have.
                let jitter = (c * 7 + k as u64 * 13) % 40;
                t.push(trade(start + k as u64 * 45_000 + jitter, price, qty, side));
            }
        }
        t
    }

    #[test]
    fn test_the_weex_usdn_bot_is_caught_on_the_tape_and_the_book() {
        let tape = weex_tape(40);
        let (bids, asks) = weex_book();
        let p = detect(Ok(&tape), Some((&bids, &asks)));
        let kinds: Vec<&str> = p
            .findings
            .iter()
            .map(|f| match f {
                Finding::RepeatedSize { .. } => "repeated",
                Finding::FewSizes { .. } => "few",
                Finding::Metronome { .. } => "metronome",
                Finding::Cadence { .. } => "cadence",
                Finding::Timer { .. } => "timer",
                Finding::RoundNotional { .. } => "round",
                Finding::PingPong { .. } => "ping-pong",
                Finding::Ladder { .. } => "ladder",
            })
            .collect();
        assert_eq!(
            kinds,
            ["few", "cadence", "ladder", "ladder", "round", "round"],
            "{:?}",
            p.findings
        );
        let crate::market::types::HealthCheck::Warn(_) = p.check() else {
            panic!("{:?}", p.check())
        };
        // The check names the signals; the evidence is one line each.
        let m = p.evidence().join("\n");
        assert!(
            m.contains("trades are one of 3 sizes: 498.11, 68.01, 500"),
            "{m}"
        );
        assert!(m.contains("gaps between trades are 45.0s ±2%"), "{m}");
        assert!(
            m.contains("Bid ladder: 9 levels falling 540.108 → 120.1201 at 0.01% steps"),
            "{m}"
        );
        assert!(
            m.contains("Ask ladder: 9 levels falling 1350 → 299.7002 at 0.01% steps"),
            "{m}"
        );
    }

    #[test]
    fn test_a_few_sizes_in_half_the_tape_is_a_signal_one_rare_size_is_not() {
        let mut t = organic(100);
        for (i, tr) in t.iter_mut().enumerate() {
            match i % 10 {
                0 | 1 => tr.quantity = 498.11,
                2 | 3 => tr.quantity = 68.01,
                4 | 5 => tr.quantity = 500.0,
                _ => {}
            }
        }
        // 20 trades each, 60 in all; no one size reaches 30%.
        assert!(repeated_size(&t).is_none());
        let (f, idx) = few_sizes(&t).unwrap();
        assert_eq!(idx.len(), 60);
        assert!(
            matches!(&f, Finding::FewSizes { sizes, count: 60, of: 100 } if sizes == &[498.11, 68.01, 500.0]),
            "{f:?}"
        );
        // 45% is not half.
        let mut t = organic(100);
        for (i, tr) in t.iter_mut().enumerate() {
            if i % 20 < 9 {
                tr.quantity = if i % 2 == 0 { 498.11 } else { 68.01 };
            }
        }
        assert!(few_sizes(&t).is_none(), "{:?}", few_sizes(&t));
        assert!(few_sizes(&organic(200)).is_none());
    }

    #[test]
    fn test_short_tapes_and_few_gaps_are_not_judged() {
        let short_tape = weex_tape(3);
        assert!(short_tape.len() < MIN_TRADES);
        assert!(few_sizes(&short_tape).is_none());
        assert!(cadence(&short_tape).is_none());
        // Enough trades, but printed in fewer than MIN_GAPS + 1 instants.
        let bunched: Vec<Trade> = (0..60u64)
            .map(|i| trade((i % 10) * 45_000, 1.0, 5.0 + i as f64, TradeSide::Buy))
            .collect();
        assert!(cadence(&bunched).is_none());
        // Every trade in the same millisecond has no gaps to judge.
        let same: Vec<Trade> = (0..40)
            .map(|i| trade(7, 1.0, 5.0 + i as f64, TradeSide::Buy))
            .collect();
        assert!(metronome(&same).is_none());
    }

    #[test]
    fn test_steps_read_to_two_significant_digits() {
        assert_eq!(step(0.010002), "0.01");
        assert_eq!(step(0.123), "0.12");
        assert_eq!(step(0.5), "0.5");
        assert_eq!(step(12.3), "12");
        assert_eq!(step(0.0), "0");
    }

    #[test]
    fn test_bursts_at_one_interval_are_a_cadence_but_not_a_metronome() {
        let t = weex_tape(30);
        assert!(metronome(&t).is_none());
        let Some(Finding::Cadence { gap_ms, share, .. }) = cadence(&t) else {
            panic!("{:?}", cadence(&t))
        };
        assert!((gap_ms - 45_000.0).abs() < 50.0, "{gap_ms}");
        assert!(share > 0.85, "{share}");
        assert!(cadence(&organic(300)).is_none());
    }

    #[test]
    fn test_random_gaps_in_whole_seconds_are_not_a_cadence() {
        // Gaps of 1 to 40 s, printed in whole seconds, in a fixed mix.
        let mut ms = 0;
        let t: Vec<Trade> = (0..400u64)
            .map(|i| {
                ms += 1_000 * (1 + (i * 7919 + i * i * 31) % 40);
                trade(ms, 1.0, 5.0 + i as f64, TradeSide::Buy)
            })
            .collect();
        assert!(cadence(&t).is_none(), "{:?}", cadence(&t));
        // Short gaps are not judged, however regular in whole seconds.
        let fast: Vec<Trade> = (0..100u64)
            .map(|i| {
                trade(
                    i * 1_000 + (i % 2) * 2_000,
                    1.0,
                    5.0 + i as f64,
                    TradeSide::Buy,
                )
            })
            .collect();
        assert!(cadence(&fast).is_none(), "{:?}", cadence(&fast));
    }

    #[test]
    fn test_sizes_that_fall_or_rise_level_by_level_are_a_shaped_ladder() {
        let (bids, asks) = weex_book();
        let found = ladders(&bids, &asks);
        assert_eq!(found.len(), 2, "{found:?}");
        for f in &found {
            assert!(
                matches!(
                    f,
                    Finding::Ladder {
                        levels: 9,
                        shape: LadderShape::Falling,
                        ..
                    }
                ),
                "{f:?}"
            );
        }
        let rising: Vec<_> = (0..10)
            .map(|i| level(0.9999 - i as f64 * 0.0001, 100.0 + i as f64 * 25.0))
            .collect();
        let found = ladders(&rising, &[]);
        assert!(
            matches!(
                found[..],
                [Finding::Ladder {
                    levels: 10,
                    shape: LadderShape::Rising,
                    ..
                }]
            ),
            "{found:?}"
        );
        assert!(
            found[0].describe().contains("rising 100 → 325"),
            "{}",
            found[0].describe()
        );
        // Up then down then up: no ladder of 8.
        let zigzag: Vec<_> = [
            100.0, 200.0, 150.0, 300.0, 250.0, 400.0, 350.0, 500.0, 450.0,
        ]
        .iter()
        .enumerate()
        .map(|(i, &q)| level(1.0001 + i as f64 * 0.0001, q))
        .collect();
        assert!(ladders(&[], &zigzag).is_empty());
    }

    /// A depth sampled every 30 s: refilled to 3300 every `every` samples,
    /// drained by a step each run between, as the weex USDN bot does.
    fn sawtooth(samples: usize, every: usize) -> Vec<(i64, f64)> {
        (0..samples)
            .map(|i| {
                let into = i % every;
                let depth = (3_300.0 - into as f64 * 250.0).max(500.0);
                (i as i64 * 30_000 + (i as i64 * 7) % 400, depth)
            })
            .collect()
    }

    #[test]
    fn test_a_book_refilled_on_a_timer_is_a_depth_cycle() {
        // 13 runs a cycle (6.5 min): 8 refills, 7 cycles between them.
        let c = depth_cycle(&sawtooth(110, 13)).unwrap();
        assert_eq!(c.cycles, 7);
        assert!((c.period_ms - 390_000.0).abs() < 1_000.0, "{}", c.period_ms);
        assert!(c.cv < 0.01, "{}", c.cv);
        // Three refills are two cycles: not yet.
        assert!(depth_cycle(&sawtooth(30, 13)).is_none());
    }

    #[test]
    fn test_irregular_refills_and_noise_are_not_a_depth_cycle() {
        // Refills at irregular times.
        let mut pts = Vec::new();
        let mut t = 0;
        for (k, len) in [5, 19, 7, 23, 6, 15, 31].into_iter().enumerate() {
            for j in 0..len {
                pts.push((t, 3_000.0 * 0.85f64.powi(j) + k as f64));
                t += 30_000;
            }
        }
        assert!(depth_cycle(&pts).is_none(), "{:?}", depth_cycle(&pts));
        // A depth that flips up and down every run is noise.
        let flip: Vec<(i64, f64)> = (0..60)
            .map(|i| (i * 30_000, if i % 2 == 0 { 1_000.0 } else { 2_000.0 }))
            .collect();
        assert!(depth_cycle(&flip).is_none());
        // Regular jumps but a depth that grows between them: not a drain.
        let grow: Vec<(i64, f64)> = (0..100)
            .map(|i| {
                let into = (i % 10) as f64;
                (
                    i * 30_000,
                    if into == 0.0 {
                        5_000.0
                    } else {
                        1_000.0 + into * 100.0
                    },
                )
            })
            .collect();
        assert!(depth_cycle(&grow).is_none(), "{:?}", depth_cycle(&grow));
        // A flat book and an empty one.
        let flat: Vec<(i64, f64)> = (0..100).map(|i| (i * 30_000, 2_000.0)).collect();
        assert!(depth_cycle(&flat).is_none());
        assert!(depth_cycle(&[]).is_none());
    }

    /// The USDN/USDT book on weex, read live on 2026-10-26: whole USDT
    /// amounts at every level but the partly filled best bid, and one bump
    /// (370 → 380) in the falling bids.
    fn weex_book_oct26() -> (Vec<OrderBookLevel>, Vec<OrderBookLevel>) {
        let bids = [
            (0.9999, 389.6396),
            (0.9998, 560.112),
            (0.9997, 460.138),
            (0.9996, 370.148),
            (0.9995, 380.19),
            (0.9994, 340.2041),
            (0.9993, 310.2171),
            (0.9992, 230.1841),
            (0.9991, 160.1441),
            (0.9990, 110.1101),
            (0.0022, 3636.3636),
        ];
        let asks = [
            (1.0001, 2379.762),
            (1.0002, 1439.712),
            (1.0003, 1189.6431),
            (1.0004, 1009.5962),
            (1.0005, 989.5052),
            (1.0006, 919.4483),
            (1.0007, 769.4614),
            (1.0008, 579.5363),
            (1.0009, 399.6403),
            (1.0010, 309.6903),
        ];
        let side = |l: &[(f64, f64)]| l.iter().map(|&(p, q)| level(p, q)).collect();
        (side(&bids), side(&asks))
    }

    /// The weex USDN tape on 2026-10-26: a trade every 30–60 s (spread
    /// evenly, never sooner), every size different, buys and sells in
    /// blocks.
    fn jittered_tape(n: u64) -> Vec<Trade> {
        let mut ms = 1_000_000;
        (0..n)
            .map(|i| {
                ms += 30_000 + (i * 7_919 + i * i * 13) % 30_000;
                let side = if (i / 9) % 2 == 0 {
                    TradeSide::Buy
                } else {
                    TradeSide::Sell
                };
                trade(
                    ms,
                    1.0001,
                    5.0 + (i * 37 % 431) as f64 + i as f64 / 100.0,
                    side,
                )
            })
            .collect()
    }

    #[test]
    fn test_the_oct26_weex_bot_is_caught_by_timer_round_notionals_and_ladders() {
        let tape = jittered_tape(500);
        let (bids, asks) = weex_book_oct26();
        let p = detect(Ok(&tape), Some((&bids, &asks)));
        let crate::market::types::HealthCheck::Warn(_) = p.check() else {
            panic!("{:?}", p.check())
        };
        // The check names the signals; the evidence is one line each.
        let m = p.evidence().join("\n");
        assert!(
            m.contains("100% of 499 gaps between trades are 30–60s"),
            "{m}"
        );
        assert!(m.contains("Jittered timer: 100% of"), "{m}");
        assert!(
            m.contains("Bid ladder: 9 levels falling 560.112 → 110.1101"),
            "{m}"
        );
        assert!(
            m.contains("Ask ladder: 10 levels falling 2380 → 309.6903"),
            "{m}"
        );
        assert!(
            m.contains(
                "Bid round notionals: 9 of 10 levels are whole quote amounts (560, 460, 370, …)"
            ),
            "{m}"
        );
        assert!(
            m.contains(
                "Ask round notionals: 10 of 10 levels are whole quote amounts (2380, 1440, 1190, …)"
            ),
            "{m}"
        );
        // No trade-share claim: a timer marks no one trade as the bot's.
        assert!(!m.contains("look bot-made"), "{m}");
        assert_eq!(
            p.bot_like_volume_pct.map(|v| v.to_string()),
            Some("0".to_string())
        );
    }

    #[test]
    fn test_a_jittered_timer_is_found_and_real_order_flow_is_not() {
        let Some(Finding::Timer {
            low_ms,
            median_ms,
            high_ms,
            cv,
            ..
        }) = timer(&jittered_tape(400))
        else {
            panic!("{:?}", timer(&jittered_tape(400)))
        };
        assert!(
            low_ms >= 30_000.0 && high_ms <= 60_000.0,
            "{low_ms} {high_ms}"
        );
        assert!((40_000.0..50_000.0).contains(&median_ms), "{median_ms}");
        assert!(cv < 0.25, "{cv}");
        // Real order flow: many near-zero gaps and a long tail (gaps that
        // grow geometrically through each block, as Poisson arrivals do).
        let mut ms = 0;
        let poisson: Vec<Trade> = (0..400u64)
            .map(|i| {
                ms += [50, 120, 400, 900, 2_500, 6_000, 15_000, 40_000, 110_000]
                    [(i * 7 % 9) as usize];
                trade(ms, 1.0, 5.0 + i as f64, TradeSide::Buy)
            })
            .collect();
        assert!(timer(&poisson).is_none(), "{:?}", timer(&poisson));
        assert!(timer(&organic(300)).is_none());
        // A jitter band under 5 s is not judged (whole-second stamps).
        let fast: Vec<Trade> = (0..100u64)
            .map(|i| {
                trade(
                    i * 2_000 + (i % 3) * 500,
                    1.0,
                    5.0 + i as f64,
                    TradeSide::Buy,
                )
            })
            .collect();
        assert!(timer(&fast).is_none(), "{:?}", timer(&fast));
    }

    #[test]
    fn test_round_notionals_need_most_levels_and_sizes_that_are_not_whole() {
        let (bids, asks) = weex_book_oct26();
        assert_eq!(round_notionals(&bids, &asks).len(), 2);
        // Six of ten, as on gate.io USDC/USDT: not enough.
        let six: Vec<_> = (0..10)
            .map(|i| {
                let price = 0.9999 - i as f64 * 0.0001;
                let qty = if i < 6 {
                    (300.0 + i as f64 * 10.0) / price
                } else {
                    123.4567 + i as f64
                };
                level(price, qty)
            })
            .collect();
        assert!(round_notionals(&six, &[]).is_empty());
        // Whole sizes at a whole price are people's orders.
        let people: Vec<_> = (0..10)
            .map(|i| level(1.0, 100.0 + i as f64 * 50.0))
            .collect();
        assert!(round_notionals(&[], &people).is_empty());
    }

    #[test]
    fn test_one_small_bump_does_not_end_a_shaped_ladder_but_a_big_one_does() {
        let (bids, _) = weex_book_oct26();
        let found = ladders(&bids, &[]);
        assert!(
            matches!(
                found[..],
                [Finding::Ladder {
                    levels: 9,
                    shape: LadderShape::Falling,
                    ..
                }]
            ),
            "{found:?}"
        );
        // A 10% bump ends the run: 4 + 6 levels, no ladder of 8.
        let sizes = [
            900.0, 800.0, 700.0, 600.0, 660.0, 500.0, 400.0, 300.0, 200.0, 100.0,
        ];
        let bumped: Vec<_> = sizes
            .iter()
            .enumerate()
            .map(|(i, &q)| level(0.9999 - i as f64 * 0.0001, q))
            .collect();
        assert!(
            ladders(&bumped, &[]).is_empty(),
            "{:?}",
            ladders(&bumped, &[])
        );
    }

    #[test]
    fn test_the_check_names_the_signals_and_the_section_lists_them() {
        let tape = jittered_tape(500);
        let (bids, asks) = weex_book_oct26();
        let p = detect(Ok(&tape), Some((&bids, &asks)));
        assert_eq!(
            p.check().message(),
            "Bot patterns: 5 signs of market-making bots (jittered timer, bid ladder, ask ladder, bid round notionals, ask round notionals)"
        );
        let text = plain(&p.format_text());
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines
                .iter()
                .take(2)
                .any(|l| l.contains("Bot Patterns · likely market-making bots")),
            "{text}"
        );
        // The trades group, then the book group, one signal a line.
        let at = |needle: &str| {
            lines
                .iter()
                .position(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("no {needle} in\n{text}"))
        };
        assert!(at("Trades") < at("! Jittered timer"), "{text}");
        assert!(at("! Jittered timer") < at("Book"), "{text}");
        assert!(at("Book") < at("! Bid ladder"), "{text}");
        assert!(at("! Bid ladder") < at("! Ask round notionals"), "{text}");
        assert!(lines[at("Trades")].contains("500 sampled"), "{text}");
        // The evidence starts in one column after every label.
        let column = |needle: &str| {
            let l = lines[at(needle)];
            l.find(needle).unwrap()
                + needle.len()
                + l[l.find(needle).unwrap() + needle.len()..].len()
                - l[l.find(needle).unwrap() + needle.len()..]
                    .trim_start()
                    .len()
        };
        assert_eq!(
            column("! Jittered timer"),
            column("! Ask round notionals"),
            "{text}"
        );
        // No share line: a timer and the book mark no one trade as bot-made.
        assert!(!text.contains("Bot-like share"), "{text}");
    }

    #[test]
    fn test_a_share_line_follows_trade_signals_and_a_book_only_finding_says_why_no_trades() {
        let mut t = organic(100);
        for tr in t.iter_mut().step_by(2) {
            tr.quantity = 1000.0;
        }
        let text = plain(&detect(Ok(&t), None).format_text());
        assert!(text.contains("! Repeated size"), "{text}");
        assert!(text.contains("Bot-like share"), "{text}");
        assert!(text.contains("50% of sampled trades ("), "{text}");
        assert!(
            !text.contains("Book "),
            "no book group without book signals: {text}"
        );
        let (bids, asks) = weex_book_oct26();
        let book_only = detect(Err("the venue has no trade endpoint"), Some((&bids, &asks)));
        let text = plain(&book_only.format_text());
        assert!(
            text.contains("not checked (the venue has no trade endpoint)"),
            "{text}"
        );
        assert!(text.contains("! Bid ladder"), "{text}");
    }

    /// The weex USDN tape on the evening of 2026-10-26: the same 30–60 s
    /// jittered timer, but in bursts of nine with an 18–25 minute pause.
    fn bursts_with_pauses(n: u64) -> Vec<Trade> {
        let mut ms = 1_000_000;
        (0..n)
            .map(|i| {
                ms += if i % 9 == 8 {
                    1_080_000 + (i * 7_919) % 420_000
                } else {
                    30_000 + (i * 7_919 + i * i * 13) % 30_000
                };
                trade(
                    ms,
                    1.0001,
                    5.0 + (i * 37 % 431) as f64 + i as f64 / 100.0,
                    TradeSide::Buy,
                )
            })
            .collect()
    }

    #[test]
    fn test_a_timer_that_pauses_between_bursts_is_still_a_timer() {
        let t = bursts_with_pauses(500);
        let Some(
            f @ Finding::Timer {
                share, pauses, cv, ..
            },
        ) = timer(&t)
        else {
            panic!("{:?}", timer(&t))
        };
        assert!(cv > 1.0, "the pauses blow the spread up: {cv}");
        assert!(share > 0.85, "{share}");
        assert!(pauses >= 50, "{pauses}");
        assert!(f.detail().contains("in bursts with"), "{}", f.detail());
    }

    #[test]
    fn test_random_arrivals_with_a_long_median_gap_are_not_a_timer() {
        // Exponential gaps (mean 60 s) from evenly spread quantiles: a quiet
        // market where real orders arrive at random.
        let mut ms = 0u64;
        let t: Vec<Trade> = (0..500u64)
            .map(|i| {
                let u = ((i as f64 + 0.5) * 0.618_034) % 1.0;
                ms += (-(1.0 - u).ln() * 60_000.0) as u64 + 1;
                trade(ms, 1.0, 5.0 + i as f64, TradeSide::Buy)
            })
            .collect();
        assert!(timer(&t).is_none(), "{:?}", timer(&t));
    }

    #[test]
    fn test_the_timer_needs_a_tape_and_names_one_pause_in_the_singular() {
        // Too short a tape, and too few distinct instants.
        assert!(timer(&jittered_tape(10)).is_none());
        let bunched: Vec<Trade> = (0..60u64)
            .map(|i| trade((i % 5) * 45_000, 1.0, 5.0 + i as f64, TradeSide::Buy))
            .collect();
        assert!(timer(&bunched).is_none());
        let one = Finding::Timer {
            low_ms: 30_000.0,
            median_ms: 45_000.0,
            high_ms: 60_000.0,
            cv: 1.2,
            gaps: 99,
            share: 0.97,
            pauses: 1,
        };
        assert!(
            one.detail().ends_with(", in bursts with 1 long pause"),
            "{}",
            one.detail()
        );
        let none = Finding::Timer {
            low_ms: 30_000.0,
            median_ms: 45_000.0,
            high_ms: 60_000.0,
            cv: 1.2,
            gaps: 99,
            share: 0.97,
            pauses: 0,
        };
        assert!(none.detail().ends_with("(median 45s)"), "{}", none.detail());
    }
}
