//! The one chart component of the TUI and the monitor (#58).
//!
//! [`Sparkline`] draws a series as a braille line (two values per column,
//! four levels per row) or, in one row or with color off, as bars. A
//! missing value is a gap; a threshold is a dim line across the chart.
//! [`HealthStrip`] draws one
//! `✓` or `✗` per run. Both show the newest values that fit, at the right
//! edge. Styles come from the caller (theme roles); nothing here names a
//! color.

use crate::cli::theme::{ERR, OK, SECONDARY};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;
use scope::display::theme::color_enabled;
use scope::market::series::Threshold;

/// Bar glyphs from 1/8 to a full cell.
const BARS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Eighths of a cell, the vertical step of a bar.
const STEPS_PER_ROW: u32 = 8;

/// Values side by side in one braille cell.
const DOTS_PER_COL: usize = 2;

/// Dot levels in one braille cell, the vertical step of a line.
const DOTS_PER_ROW: u32 = 4;

/// The empty braille cell; each dot adds its bit to it.
const BRAILLE_BLANK: u32 = 0x2800;

/// The bit of each dot, by column (left, right) and row (top first).
const BRAILLE_DOTS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// A line needs this many rows; a 1-row chart keeps the finer bars.
const MIN_BRAILLE_ROWS: u16 = 2;

/// The glyph of a threshold line in the cells no bar fills.
const THRESHOLD_GLYPH: &str = "─";

/// A run that passed and one that failed, in a [`HealthStrip`].
const PASS_GLYPH: &str = "✓";
const FAIL_GLYPH: &str = "✗";

/// A series as a braille line, or as bars. `None` is a gap, never a zero.
pub struct Sparkline<'a> {
    values: &'a [Option<f64>],
    threshold: Option<Threshold>,
    style: Style,
    /// A braille line when the area is tall enough. Off with color off,
    /// where a braille glyph may not draw (the TUI spinner's rule).
    braille: bool,
}

impl<'a> Sparkline<'a> {
    /// A chart of `values` (oldest first) in `style`.
    pub fn new(values: &'a [Option<f64>], style: Style) -> Self {
        Self {
            values,
            threshold: None,
            style,
            braille: color_enabled(),
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

/// The finite value the threshold draws its line at, if any.
fn line_value(threshold: Option<Threshold>) -> Option<f64> {
    threshold.map(threshold_value).filter(|v| v.is_finite())
}

/// The low and high ends of the scale of `shown` and the threshold `line`,
/// or `None` when there is no value to draw.
fn scale(shown: &[Option<f64>], line: Option<f64>) -> Option<(f64, f64)> {
    let known = shown.iter().flatten().copied().filter(|v| v.is_finite());
    let (mut lo, mut hi) = known.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
        (lo.min(v), hi.max(v))
    });
    if let Some(t) = line {
        // The scale includes the threshold, so its line is on the chart.
        lo = lo.min(t);
        hi = hi.max(t);
    }
    lo.is_finite().then_some((lo, hi))
}

/// The braille glyph with the dots `bits`.
fn braille(bits: u8) -> String {
    char::from_u32(BRAILLE_BLANK + u32::from(bits))
        .unwrap_or(' ')
        .to_string()
}

impl Widget for Sparkline<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }
        if self.braille && area.height >= MIN_BRAILLE_ROWS {
            self.render_line(area, buf);
        } else {
            self.render_bars(area, buf);
        }
    }
}

impl Sparkline<'_> {
    /// A connected line of braille dots, two values per column.
    fn render_line(self, area: Rect, buf: &mut Buffer) {
        let (width, height) = (usize::from(area.width), usize::from(area.height));
        let dots_wide = width * DOTS_PER_COL;
        let shown = newest(self.values, dots_wide);
        let line = line_value(self.threshold);
        let Some((lo, hi)) = scale(shown, line) else {
            return; // no value to draw
        };
        let levels = u32::from(area.height) * DOTS_PER_ROW;
        let range = hi - lo;
        // A dot level from the bottom, 0 to levels - 1. A flat series sits
        // half way up, as the bars do.
        let level_of = |v: f64| -> u32 {
            if range <= 0.0 {
                return (levels - 1) / 2;
            }
            (((v - lo) / range * f64::from(levels - 1)).round() as u32).min(levels - 1)
        };
        // A dot level as its cell row (top first) and its row in that cell.
        let place = |level: u32| -> (usize, usize) {
            let from_top = (levels - 1 - level) as usize;
            let per_row = DOTS_PER_ROW as usize;
            (from_top / per_row, from_top % per_row)
        };
        let mut cells = vec![0u8; width * height];
        let dx0 = dots_wide - shown.len();
        let mut prev: Option<u32> = None;
        for (i, value) in shown.iter().enumerate() {
            let Some(level) = value.filter(|v| v.is_finite()).map(level_of) else {
                prev = None; // a gap breaks the line
                continue;
            };
            // Fill from just past the previous point to this one, so a
            // steep step still reads as one line.
            let (from, to) = match prev {
                Some(p) if p < level => (p + 1, level),
                Some(p) if p > level => (level, p - 1),
                _ => (level, level),
            };
            let dx = dx0 + i;
            for l in from..=to {
                let (row, dot) = place(l);
                cells[row * width + dx / DOTS_PER_COL] |= BRAILLE_DOTS[dx % DOTS_PER_COL][dot];
            }
            prev = Some(level);
        }
        let threshold = line.map(|t| place(level_of(t)));
        for (row, bits_of_row) in cells.chunks(width).enumerate() {
            let y = area.top() + row as u16;
            for (col, &bits) in bits_of_row.iter().enumerate() {
                let cell = &mut buf[(area.left() + col as u16, y)];
                if bits != 0 {
                    cell.set_symbol(&braille(bits)).set_style(self.style);
                } else if let Some((_, dot)) = threshold.filter(|(r, _)| *r == row) {
                    // A dotted line: one dot per cell, in its left column.
                    cell.set_symbol(&braille(BRAILLE_DOTS[0][dot]))
                        .set_style(SECONDARY);
                }
            }
        }
    }

    /// One bar per column, in eighths of a cell.
    fn render_bars(self, area: Rect, buf: &mut Buffer) {
        let shown = newest(self.values, area.width as usize);
        let line = line_value(self.threshold);
        let Some((lo, hi)) = scale(shown, line) else {
            return; // no value to draw
        };
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

    /// A chart that draws bars, as with color off.
    fn bars(values: &[Option<f64>]) -> Sparkline<'_> {
        Sparkline {
            braille: false,
            ..Sparkline::new(values, Style::new())
        }
    }

    /// A chart that draws a braille line where it fits, as with color on.
    fn line(values: &[Option<f64>]) -> Sparkline<'_> {
        Sparkline {
            braille: true,
            ..Sparkline::new(values, Style::new())
        }
    }

    fn render(widget: impl Widget, width: u16, height: u16) -> Buffer {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        buf
    }

    fn draw(widget: impl Widget, width: u16, height: u16) -> Vec<String> {
        let buf = render(widget, width, height);
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
        let rows = draw(bars(&values), 3, 1);
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
        let rows = draw(bars(&values), 2, 1);
        assert_eq!(rows[0], "▁█", "{rows:?}");
    }

    #[test]
    fn test_the_threshold_line_fills_the_empty_cells_of_its_row() {
        // Values 0 and 10 over 2 rows; a max of 8 sits in the top row.
        let values = [Some(0.0), Some(10.0), Some(0.0)];
        let chart = bars(&values).threshold(Some(Threshold::Max(8.0)));
        let rows = draw(chart, 3, 2);
        assert_eq!(rows[0], "─█─", "{rows:?}");
        assert!(!rows[1].contains('─'), "{rows:?}");
    }

    #[test]
    fn test_the_scale_includes_a_threshold_beyond_the_values() {
        // A min depth of 100 with depths near 10: the line is at the top
        // and the bars stay low, so the shortfall shows.
        let values = [Some(10.0), Some(12.0)];
        let chart = bars(&values).threshold(Some(Threshold::Min(100.0)));
        let rows = draw(chart, 2, 2);
        assert_eq!(rows[0], "──", "{rows:?}");
        assert!(rows[1].chars().all(|c| c != '█'), "{rows:?}");
    }

    #[test]
    fn test_no_values_draw_nothing() {
        let values = [None, None];
        let rows = draw(bars(&values), 2, 1);
        assert_eq!(rows[0], "  ");
    }

    /// The rows of a chart of braille cells, with `bits` per cell.
    fn cells(bits: &[&[u8]]) -> Vec<String> {
        bits.iter()
            .map(|row| {
                row.iter()
                    .map(|&b| if b == 0 { " ".into() } else { braille(b) })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn test_a_braille_cell_holds_two_values_joined_by_dots() {
        // 100 is cut off: one column holds the newest two values. 0 is the
        // bottom-left dot; 10 the top right, filled down to just above 0.
        let values = [Some(100.0), Some(0.0), Some(10.0)];
        let rows = draw(line(&values), 1, 2);
        assert_eq!(rows, cells(&[&[0xB8], &[0x40 | 0x38]]), "{rows:?}");
    }

    #[test]
    fn test_a_gap_leaves_its_half_column_empty_and_breaks_the_line() {
        let values = [Some(0.0), None, Some(0.0), Some(10.0)];
        let rows = draw(line(&values), 2, 2);
        // Column 0: the left dot only. Column 1: a lone dot (nothing to
        // join on the left), then the climb to 10 on the right.
        assert_eq!(rows, cells(&[&[0, 0xB8], &[0x40, 0x40 | 0x38]]), "{rows:?}");
    }

    #[test]
    fn test_a_flat_braille_series_is_a_level_line_half_way_up() {
        let values = [Some(5.0); 4];
        let rows = draw(line(&values), 2, 2);
        // Level 3 of 0-7: the top dot of the bottom row, in both columns.
        assert_eq!(rows, cells(&[&[0, 0], &[0x09, 0x09]]), "{rows:?}");
    }

    #[test]
    fn test_the_braille_threshold_is_a_dotted_row_in_secondary() {
        // Values 0 and 10 over 2 rows (levels 0-7); a max of 8 is level 6,
        // the second dot of the top row.
        let values = [Some(0.0), Some(10.0)];
        let chart = line(&values).threshold(Some(Threshold::Max(8.0)));
        let buf = render(chart, 3, 2);
        let mut dim = ratatui::buffer::Cell::default();
        dim.set_style(SECONDARY);
        for x in 0..2 {
            assert_eq!(buf[(x, 0)].symbol(), braille(0x02));
            assert_eq!(buf[(x, 0)].style(), dim.style());
            assert_eq!(buf[(x, 1)].symbol(), " ");
        }
        // The series keeps its own cell; the dots do not overwrite it.
        assert_eq!(buf[(2, 0)].symbol(), braille(0xB8));
    }

    #[test]
    fn test_a_one_row_chart_keeps_the_bars() {
        let values = [Some(1.0), None, Some(3.0)];
        let rows = draw(line(&values), 3, 1);
        assert_eq!(rows[0], "▁ █", "{rows:?}");
    }

    #[test]
    fn test_the_health_strip_marks_each_run_newest_last() {
        let runs = [true, false, true, true];
        let rows = draw(HealthStrip::new(&runs), 3, 1);
        assert_eq!(rows[0], "✗✓✓");
    }
}
