//! The one chart component of the TUI and the monitor (#58).
//!
//! [`Sparkline`] draws a series as bars, with a gap where a value is
//! missing and a line where its threshold is. [`HealthStrip`] draws one
//! `✓` or `✗` per run. Both show the newest values that fit, at the right
//! edge. Styles come from the caller (theme roles); nothing here names a
//! color.

use crate::cli::theme::{ERR, OK, SECONDARY};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;
use scope::metrics::Threshold;

/// Bar glyphs from 1/8 to a full cell.
const BARS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Eighths of a cell, the vertical step of a bar.
const STEPS_PER_ROW: u32 = 8;

/// The glyph of a threshold line in the cells no bar fills.
const THRESHOLD_GLYPH: &str = "─";

/// A run that passed and one that failed, in a [`HealthStrip`].
const PASS_GLYPH: &str = "✓";
const FAIL_GLYPH: &str = "✗";

/// A series as bars. `None` is a gap, never a zero.
pub struct Sparkline<'a> {
    values: &'a [Option<f64>],
    threshold: Option<Threshold>,
    style: Style,
}

impl<'a> Sparkline<'a> {
    /// Bars of `values` (oldest first) in `style`.
    pub fn new(values: &'a [Option<f64>], style: Style) -> Self {
        Self {
            values,
            threshold: None,
            style,
        }
    }

    /// Adds the line of `threshold`, when there is one.
    pub fn threshold(mut self, threshold: Option<Threshold>) -> Self {
        self.threshold = threshold;
        self
    }
}

/// The value a threshold draws its line at.
fn threshold_value(t: Threshold) -> f64 {
    match t {
        Threshold::Max(v) | Threshold::Min(v) | Threshold::Target(v) => v,
    }
}

/// The newest `width` items of `items`.
fn newest<T>(items: &[T], width: usize) -> &[T] {
    &items[items.len().saturating_sub(width)..]
}

impl Widget for Sparkline<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }
        let shown = newest(self.values, area.width as usize);
        let line = self
            .threshold
            .map(threshold_value)
            .filter(|v| v.is_finite());
        let known = shown.iter().flatten().copied().filter(|v| v.is_finite());
        let (mut lo, mut hi) = known.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        });
        if let Some(t) = line {
            // The scale includes the threshold, so its line is on the chart.
            lo = lo.min(t);
            hi = hi.max(t);
        }
        if !lo.is_finite() {
            return; // no value to draw
        }
        let steps = u32::from(area.height) * STEPS_PER_ROW;
        let range = hi - lo;
        // A flat series draws half height: it is there, but not high or low.
        let height_of = |v: f64| -> u32 {
            if range <= 0.0 {
                return steps / 2;
            }
            let h = ((v - lo) / range * f64::from(steps - 1)).round() as u32 + 1;
            h.min(steps)
        };
        let x0 = area.right() - shown.len() as u16;
        for (i, value) in shown.iter().enumerate() {
            let x = x0 + i as u16;
            let Some(v) = value.filter(|v| v.is_finite()) else {
                continue; // a gap
            };
            let mut left = height_of(v);
            for row in (area.top()..area.bottom()).rev() {
                if left == 0 {
                    break;
                }
                let eighths = left.min(STEPS_PER_ROW);
                buf[(x, row)]
                    .set_symbol(BARS[eighths as usize - 1])
                    .set_style(self.style);
                left -= eighths;
            }
        }
        if let Some(t) = line {
            let rows_up = height_of(t).saturating_sub(1) / STEPS_PER_ROW;
            let row = area.bottom() - 1 - rows_up as u16;
            for x in area.left()..area.right() {
                let cell = &mut buf[(x, row)];
                if cell.symbol() == " " {
                    cell.set_symbol(THRESHOLD_GLYPH).set_style(SECONDARY);
                }
            }
        }
    }
}

/// One `✓` or `✗` per run, the newest at the right.
pub struct HealthStrip<'a> {
    runs: &'a [bool],
}

impl<'a> HealthStrip<'a> {
    /// A strip of `runs` (true = healthy), oldest first.
    pub fn new(runs: &'a [bool]) -> Self {
        Self { runs }
    }
}

impl Widget for HealthStrip<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }
        let shown = newest(self.runs, area.width as usize);
        let x0 = area.right() - shown.len() as u16;
        for (i, healthy) in shown.iter().enumerate() {
            let (glyph, style) = if *healthy {
                (PASS_GLYPH, OK)
            } else {
                (FAIL_GLYPH, ERR)
            };
            buf[(x0 + i as u16, area.top())]
                .set_symbol(glyph)
                .set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(widget: impl Widget, width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn test_a_missing_value_is_a_gap_not_a_zero() {
        let values = [Some(1.0), None, Some(3.0)];
        let rows = draw(Sparkline::new(&values, Style::new()), 3, 1);
        let cells: Vec<char> = rows[0].chars().collect();
        assert_eq!(cells[1], ' ', "{rows:?}");
        assert_ne!(cells[0], ' ');
        assert_eq!(cells[2], '█', "the top value fills its cell");
    }

    #[test]
    fn test_only_the_newest_values_that_fit_are_drawn() {
        // The first value is far above the rest. If it were drawn, the
        // others would be short bars; cut off, the scale is theirs alone.
        let values = [Some(100.0), Some(1.0), Some(2.0)];
        let rows = draw(Sparkline::new(&values, Style::new()), 2, 1);
        assert_eq!(rows[0], "▁█", "{rows:?}");
    }

    #[test]
    fn test_the_threshold_line_fills_the_empty_cells_of_its_row() {
        // Values 0 and 10 over 2 rows; a max of 8 sits in the top row.
        let values = [Some(0.0), Some(10.0), Some(0.0)];
        let chart = Sparkline::new(&values, Style::new()).threshold(Some(Threshold::Max(8.0)));
        let rows = draw(chart, 3, 2);
        assert_eq!(rows[0], "─█─", "{rows:?}");
        assert!(!rows[1].contains('─'), "{rows:?}");
    }

    #[test]
    fn test_the_scale_includes_a_threshold_beyond_the_values() {
        // A min depth of 100 with depths near 10: the line is at the top
        // and the bars stay low, so the shortfall shows.
        let values = [Some(10.0), Some(12.0)];
        let chart = Sparkline::new(&values, Style::new()).threshold(Some(Threshold::Min(100.0)));
        let rows = draw(chart, 2, 2);
        assert_eq!(rows[0], "──", "{rows:?}");
        assert!(rows[1].chars().all(|c| c != '█'), "{rows:?}");
    }

    #[test]
    fn test_no_values_draw_nothing() {
        let values = [None, None];
        let rows = draw(Sparkline::new(&values, Style::new()), 2, 1);
        assert_eq!(rows[0], "  ");
    }

    #[test]
    fn test_the_health_strip_marks_each_run_newest_last() {
        let runs = [true, false, true, true];
        let rows = draw(HealthStrip::new(&runs), 3, 1);
        assert_eq!(rows[0], "✗✓✓");
    }
}
