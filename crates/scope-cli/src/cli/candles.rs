//! The candlestick chart of `market ohlc` (mockup approved 2026-10-26).
//!
//! One column per candle, newest on the right, sized to the terminal (or
//! the TUI pane). Each row holds two price steps: a body is drawn with half
//! blocks (`▀` `▄` `█`), a wick with thin lines (`│` `╵` `╷`). Up candles
//! (close ≥ open) are green, down candles red. A price axis on the left and
//! a time axis under the candles. Pure: no I/O.

use scope::display::format::price_decimals;
use scope::display::theme::{Painter, Role};
use scope::market::Candle;

/// Rows of the chart (each two price steps).
pub const CHART_ROWS: usize = 12;

/// Fewest candles the chart is drawn for.
pub const MIN_COLUMNS: usize = 10;

/// The candle columns that fit in `width` terminal columns, after the
/// rail (`│ `) and the price axis.
pub fn columns_for(width: usize, axis_cols: usize) -> usize {
    width.saturating_sub(2 + axis_cols + 2).max(MIN_COLUMNS)
}

/// The decimals of every price axis label for candles between `low` and
/// `high`: the finer of the two prices' own, so labels line up.
fn axis_decimals(low: f64, high: f64) -> usize {
    price_decimals(low).max(price_decimals(high))
}

/// The width of the price axis labels for candles between `low` and `high`.
pub fn axis_cols(low: f64, high: f64) -> usize {
    let d = axis_decimals(low, high);
    format!("{low:.d$}").len().max(format!("{high:.d$}").len())
}

/// The chart of `candles` (oldest first) in `rows` rows, one column per
/// candle, each line starting with the price axis. `time` formats a candle's
/// open time (ms) for the time axis.
pub fn chart(
    candles: &[Candle],
    rows: usize,
    painter: Painter,
    time: impl Fn(u64) -> String,
) -> Vec<String> {
    if candles.is_empty() || rows == 0 {
        return Vec::new();
    }
    let low = candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    let high = candles
        .iter()
        .map(|c| c.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let span = (high - low).max(f64::EPSILON);
    let steps = rows * 2;
    // The half-row step a price falls on, 0 at the bottom.
    let step = |v: f64| (((v - low) / span) * (steps - 1) as f64).round() as usize;
    let label_cols = axis_cols(low, high);
    let decimals = axis_decimals(low, high);
    let mut lines = Vec::with_capacity(rows + 1);
    for r in 0..rows {
        let (top, bottom) = ((rows - 1 - r) * 2 + 1, (rows - 1 - r) * 2);
        let price = high - span * r as f64 / (rows - 1).max(1) as f64;
        // A label on the first and last rows and about every fourth between.
        let labelled = r == 0 || r == rows - 1 || (r % 4 == 0 && r + 2 < rows);
        let axis = if labelled {
            format!("{price:>label_cols$.decimals$} ┤")
        } else {
            format!("{:>label_cols$} │", "")
        };
        let mut line = painter.paint(Role::Secondary, &axis);
        for c in candles {
            let (wick_hi, wick_lo) = (step(c.high), step(c.low));
            let (body_hi, body_lo) = (step(c.open.max(c.close)), step(c.open.min(c.close)));
            let body = |s: usize| body_lo <= s && s <= body_hi;
            let wick = |s: usize| wick_lo <= s && s <= wick_hi;
            let glyph = match (body(top), body(bottom)) {
                (true, true) => "█",
                (true, false) => "▀",
                (false, true) => "▄",
                (false, false) => match (wick(top), wick(bottom)) {
                    (true, true) => "│",
                    (true, false) => "╵",
                    (false, true) => "╷",
                    (false, false) => " ",
                },
            };
            if glyph == " " {
                line.push(' ');
            } else {
                let role = if c.close >= c.open {
                    Role::Ok
                } else {
                    Role::Err
                };
                line.push_str(&painter.paint(role, glyph));
            }
        }
        lines.push(line);
    }
    lines.push(painter.paint(Role::Secondary, &time_axis(candles, label_cols + 2, &time)));
    lines
}

/// The time axis under the candles: the first candle's time under the
/// first column, the last's ending under the last, when both fit.
fn time_axis(candles: &[Candle], indent: usize, time: &impl Fn(u64) -> String) -> String {
    let first = time(candles[0].open_time);
    let last = time(candles[candles.len() - 1].open_time);
    let width = candles.len();
    let mut axis = format!("{:indent$}", "");
    if first.chars().count() + 2 + last.chars().count() <= width {
        let gap = width - first.chars().count() - last.chars().count();
        axis.push_str(&format!("{first}{:gap$}{last}", ""));
    } else if last.chars().count() <= width {
        axis.push_str(&format!("{last:>width$}"));
    }
    axis
}

/// What the candles of a window add up to.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    /// Close against open, in percent; `None` when the open is zero.
    pub change_pct: Option<f64>,
    /// The candles' base volume, summed.
    pub volume: f64,
}

/// The summary of `candles` (oldest first), or `None` when there are none.
pub fn summary(candles: &[Candle]) -> Option<Summary> {
    let (first, last) = (candles.first()?, candles.last()?);
    Some(Summary {
        open: first.open,
        high: candles
            .iter()
            .map(|c| c.high)
            .fold(f64::NEG_INFINITY, f64::max),
        low: candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min),
        close: last.close,
        change_pct: (first.open != 0.0).then(|| (last.close - first.open) / first.open * 100.0),
        volume: candles.iter().map(|c| c.volume).sum::<f64>() + 0.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(t: u64, open: f64, high: f64, low: f64, close: f64) -> Candle {
        Candle {
            open_time: t,
            open,
            high,
            low,
            close,
            volume: 10.0,
            close_time: t + 59,
        }
    }

    fn plain(lines: &[String]) -> Vec<String> {
        lines.to_vec()
    }

    fn time(ms: u64) -> String {
        format!("t{ms}")
    }

    #[test]
    fn an_up_and_a_down_candle_have_bodies_and_wicks_in_their_columns() {
        // Range 100..110 over 4 rows = 8 half-steps.
        let candles = [
            candle(1, 102.0, 110.0, 100.0, 108.0),
            candle(2, 108.0, 109.0, 101.0, 103.0),
        ];
        let lines = plain(&chart(&candles, 4, Painter::new(false), time));
        assert_eq!(lines.len(), 5, "{lines:#?}");
        let col = |i: usize| -> String {
            lines[..4]
                .iter()
                .map(|l| l.chars().rev().nth(1 - i).unwrap())
                .collect()
        };
        // The up candle, body 102..108: a cell holds a body half or a wick
        // half, so the wick ends beside the body halves are not drawn.
        assert_eq!(col(0), "▄██▀", "{lines:#?}");
        // The down candle, body 103..108: its lower wick shows under it.
        assert_eq!(col(1), "▄██╵", "{lines:#?}");
        assert!(lines[0].starts_with("110.0000 ┤"), "{lines:#?}");
        assert!(lines[3].starts_with("100.0000 ┤"), "{lines:#?}");
    }

    #[test]
    fn colour_follows_the_close_against_the_open() {
        let candles = [candle(1, 1.0, 2.0, 0.5, 1.5), candle(2, 1.5, 2.0, 0.5, 1.0)];
        let lines = chart(&candles, 3, Painter::new(true), time);
        let painted = |role: Role, glyph: &str| Painter::new(true).paint(role, glyph);
        assert!(lines.iter().any(|l| l.contains(&painted(Role::Ok, "█"))));
        assert!(lines.iter().any(|l| l.contains(&painted(Role::Err, "█"))));
    }

    #[test]
    fn a_flat_candle_still_shows_and_a_flat_window_does_not_divide_by_zero() {
        let candles = vec![candle(1, 1.0, 1.0, 1.0, 1.0); 3];
        let lines = chart(&candles, 4, Painter::new(false), time);
        let marks: usize = lines[..4]
            .iter()
            .map(|l| l.chars().filter(|c| "▀▄█".contains(*c)).count())
            .sum();
        assert_eq!(marks, 3, "{lines:#?}");
        assert!(chart(&[], 4, Painter::new(false), time).is_empty());
    }

    #[test]
    fn the_time_axis_spans_the_candles_or_keeps_only_the_last() {
        let candles: Vec<Candle> = (0..20).map(|i| candle(i, 1.0, 2.0, 0.5, 1.5)).collect();
        let lines = chart(&candles, 2, Painter::new(false), |ms| format!("{ms:>02}h"));
        let axis = lines.last().unwrap();
        assert!(axis.trim_start().starts_with("00h"), "{axis}");
        assert!(axis.ends_with("19h"), "{axis}");
        assert_eq!(axis.chars().count(), lines[0].chars().count(), "{axis}");
        let narrow = chart(&candles[..6], 2, Painter::new(false), |_| {
            "10-26 08:00".into()
        });
        assert!(
            narrow.last().unwrap().trim().is_empty(),
            "too wide for both"
        );
    }

    #[test]
    fn the_summary_spans_the_window() {
        let candles = [
            candle(1, 100.0, 105.0, 99.0, 104.0),
            candle(2, 104.0, 112.0, 101.0, 110.0),
        ];
        let s = summary(&candles).unwrap();
        assert_eq!(
            (s.open, s.high, s.low, s.close, s.volume),
            (100.0, 112.0, 99.0, 110.0, 20.0)
        );
        assert!((s.change_pct.unwrap() - 10.0).abs() < 1e-9);
        assert!(summary(&[]).is_none());
        assert_eq!(
            summary(&[candle(1, 0.0, 1.0, 0.0, 1.0)])
                .unwrap()
                .change_pct,
            None
        );
    }

    #[test]
    fn columns_fit_the_width_but_never_fall_below_the_minimum() {
        assert_eq!(columns_for(100, 8), 100 - 2 - 8 - 2);
        assert_eq!(columns_for(12, 8), MIN_COLUMNS);
        assert_eq!(axis_cols(0.9999, 1.0001), "1.000100".len());
        // One precision for the whole axis: 0.5 needs six decimals, so 2.0 gets them too.
        assert_eq!(axis_cols(0.5, 2.0), "2.000000".len());
    }
}
