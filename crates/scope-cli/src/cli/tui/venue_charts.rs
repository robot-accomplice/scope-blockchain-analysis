//! The venue charts of the watchlist grid (design 2026-11-04, section 2.9).
//!
//! The detail pane of a target on two or more venues shows one chart per
//! market cell, with one line per venue on a shared scale, and one venue
//! legend for all the charts. With every venue shown, the lines are
//! braille, so they overlay; one venue alone (isolated) is the bar
//! [`Sparkline`], the readable view of a noisy series (the #110 finding).
//! The selected venue is drawn last, so it stays on top where lines touch.
//! A venue's colour is its place in the target's venue list; with colour
//! off, the legend and each chart's value box still name every venue with
//! its value.

use super::grid::{status_cell, value_text};
use crate::cli::charts::{Sparkline, min_span};
use crate::cli::theme::{ACCENT, SECONDARY, series_style};
use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, Chart, Dataset, GraphType, Paragraph};
use scope::metrics::{Threshold, Unit};
use scope::watch::RuleStatus;
use std::time::Duration;

/// The narrowest chart pane, borders included.
pub(crate) const MIN_CHART_COLS: u16 = 40;

/// The lowest chart pane: two border rows and three plot rows.
pub(crate) const MIN_CHART_ROWS: u16 = 5;

/// Columns of a value in the legend.
const VALUE_COLS: usize = 10;

/// The series from this one on repeat the colours of the first ones, so
/// they draw with dots instead of braille to stay apart.
const DOT_FROM: usize = scope::display::theme::SERIES_TONES.len();

/// One venue's series on one chart.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Series {
    pub venue: String,
    /// The last values, oldest first; `None` is a gap.
    pub values: Vec<Option<f64>>,
    /// The cell's status on this venue at the last tick.
    pub status: RuleStatus,
}

impl Series {
    /// The newest value.
    fn last(&self) -> Option<f64> {
        self.values.iter().rev().flatten().next().copied()
    }
}

/// One market cell's chart: a series per venue, in venue order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VenueChart {
    pub label: String,
    pub threshold: Option<Threshold>,
    /// The metric's unit, for the smallest span; `None` for a preset check.
    pub unit: Option<Unit>,
    pub series: Vec<Series>,
}

/// Every chart of the selected target.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VenueCharts {
    pub charts: Vec<VenueChart>,
    /// The time between two values, for the time axis.
    pub interval: Duration,
}

impl VenueCharts {
    fn venues(&self) -> Vec<&str> {
        self.charts
            .first()
            .map(|c| c.series.iter().map(|s| s.venue.as_str()).collect())
            .unwrap_or_default()
    }
}

/// What the keys select in the venue charts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Selection {
    /// The selected venue, by its place in the venue list.
    pub venue: usize,
    /// Only the selected venue is shown, as bars.
    pub isolated: bool,
    /// The selected chart, whose values the legend shows.
    pub chart: usize,
}

impl Selection {
    /// Applies a key of the venue charts: `←` `→` the venue, `Space` the
    /// isolation, `Tab` the chart, `PgUp` `PgDn` the page of `page`
    /// charts. `venues` and `charts` are the counts. The watch grid and the
    /// repeat view of `market summary` both call it, so the keys are one.
    pub(crate) fn apply(&mut self, code: KeyCode, venues: usize, charts: usize, page: usize) {
        let page = page.max(1);
        match code {
            KeyCode::Right => self.venue = (self.venue + 1) % venues,
            KeyCode::Left => self.venue = (self.venue + venues - 1) % venues,
            KeyCode::Char(' ') => self.isolated = !self.isolated,
            KeyCode::Tab => self.chart = (self.chart + 1) % charts,
            KeyCode::PageDown => self.chart = ((self.chart / page + 1) * page).min(charts - 1),
            KeyCode::PageUp => self.chart = (self.chart / page).saturating_sub(1) * page,
            _ => {}
        }
    }

    /// True for a key that [`Selection::apply`] reads.
    pub(crate) fn is_key(code: KeyCode) -> bool {
        matches!(
            code,
            KeyCode::Left
                | KeyCode::Right
                | KeyCode::Char(' ')
                | KeyCode::Tab
                | KeyCode::PageUp
                | KeyCode::PageDown
        )
    }
}

/// The range of the last values across venues: `0.9998–1.0003`.
fn range_text(chart: &VenueChart) -> String {
    let last: Vec<f64> = chart.series.iter().filter_map(Series::last).collect();
    let lo = last.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = last.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if lo.is_finite() {
        format!("{}–{}", value_text(Some(lo)), value_text(Some(hi)))
    } else {
        "-".to_string()
    }
}

/// The legend's width: the widest venue row or `all` row, and borders.
fn legend_width(charts: &VenueCharts) -> u16 {
    let name = charts
        .venues()
        .iter()
        .map(|v| v.chars().count())
        .max()
        .unwrap_or(0);
    // `> ■ name value ✓`
    let venue_row = 4 + name + 1 + VALUE_COLS + 2;
    let all_row = charts
        .charts
        .iter()
        .map(|c| 7 + range_text(c).chars().count())
        .max()
        .unwrap_or(0);
    (venue_row.max(all_row) + 2) as u16
}

/// Columns and rows the venue charts need beyond `area`, when they do not
/// fit: one chart beside the legend.
pub(crate) fn shortfall(area: Rect, charts: &VenueCharts) -> Option<(u16, u16)> {
    let short = (
        (MIN_CHART_COLS + legend_width(charts)).saturating_sub(area.width),
        MIN_CHART_ROWS.saturating_sub(area.height),
    );
    (short != (0, 0)).then_some(short)
}

/// Draws the charts and the legend in `area`, which [`shortfall`] says
/// they fit. Returns how many charts a page shows.
pub(crate) fn draw(f: &mut Frame, area: Rect, charts: &VenueCharts, sel: Selection) -> usize {
    let legend_w = legend_width(charts);
    let [plots, legend] = Layout::horizontal([
        Constraint::Min(MIN_CHART_COLS),
        Constraint::Length(legend_w),
    ])
    .areas(area);
    let n = charts.charts.len().max(1);
    let fit_cols = usize::from((plots.width / MIN_CHART_COLS).max(1));
    let fit_rows = usize::from((plots.height / MIN_CHART_ROWS).max(1));
    let cols = fit_cols.min(n);
    let rows = fit_rows.min(n.div_ceil(cols));
    let per_page = cols * rows;
    let page = sel.chart / per_page;
    let shown = &charts.charts[(page * per_page).min(charts.charts.len())..];
    let row_areas = Layout::vertical(vec![Constraint::Ratio(1, rows as u32); rows]).split(plots);
    for (r, row_area) in row_areas.iter().enumerate() {
        let cells =
            Layout::horizontal(vec![Constraint::Ratio(1, cols as u32); cols]).split(*row_area);
        for (c, cell) in cells.iter().enumerate() {
            let i = r * cols + c;
            if let Some(chart) = shown.get(i) {
                let focused = page * per_page + i == sel.chart;
                draw_chart(f, *cell, chart, sel, focused, charts.interval);
            }
        }
    }
    let pages = charts.charts.len() > per_page;
    let page_text = pages.then(|| {
        let first = page * per_page + 1;
        let last = (first + per_page - 1).min(charts.charts.len());
        format!(" charts {first}–{last} of {} ", charts.charts.len())
    });
    draw_legend(f, legend, charts, sel, page_text);
    per_page
}

/// The series a chart shows: every venue, or the isolated one.
fn shown(chart: &VenueChart, sel: Selection) -> Vec<usize> {
    if sel.isolated {
        vec![sel.venue.min(chart.series.len().saturating_sub(1))]
    } else {
        // The selected venue last, so it is on top where lines touch.
        let mut order: Vec<usize> = (0..chart.series.len())
            .filter(|&i| i != sel.venue)
            .collect();
        if sel.venue < chart.series.len() {
            order.push(sel.venue);
        }
        order
    }
}

/// The time axis labels: how long ago the oldest value is, and now.
fn time_labels(values: usize, interval: Duration) -> [String; 2] {
    let secs = interval.as_secs() * values.saturating_sub(1) as u64;
    let ago = match secs {
        0..60 => format!("-{secs}s"),
        60..3600 => format!("-{}m", secs / 60),
        _ if secs.is_multiple_of(3600) => format!("-{}h", secs / 3600),
        _ => format!("-{}h {:02}m", secs / 3600, secs % 3600 / 60),
    };
    [ago, "now".to_string()]
}

/// One chart pane: the cell label, the value box and the plot.
fn draw_chart(
    f: &mut Frame,
    area: Rect,
    chart: &VenueChart,
    sel: Selection,
    focused: bool,
    interval: Duration,
) {
    let order = shown(chart, sel);
    let title_style = if focused {
        ACCENT.add_modifier(Modifier::BOLD)
    } else {
        ACCENT
    };
    let title = format!(" {} ", chart.label);
    let mut block = Block::bordered()
        .border_style(if focused { ACCENT } else { SECONDARY })
        .title(Line::styled(title.clone(), title_style));
    // The value box: each shown venue's last value in its colour, in
    // venue order. Left out when it does not fit in the border.
    let mut by_venue = order.clone();
    by_venue.sort_unstable();
    let boxed: Vec<Span> = by_venue
        .iter()
        .map(|&i| {
            let s = &chart.series[i];
            Span::styled(
                format!(" {} {} ", s.venue, value_text(s.last())),
                series_style(i),
            )
        })
        .collect();
    let box_cols: usize = boxed.iter().map(|s| s.content.chars().count()).sum();
    if title.chars().count() + box_cols + 4 <= usize::from(area.width) {
        block = block.title(Line::from(boxed).right_aligned());
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 2 || inner.width < 2 {
        return;
    }

    let len = chart
        .series
        .iter()
        .map(|s| s.values.len())
        .max()
        .unwrap_or(0);
    let [ago, now] = time_labels(len, interval);
    let selected = &chart.series[sel.venue.min(chart.series.len() - 1)];
    let span = chart.unit.map(|u| min_span(u, &selected.values));

    if sel.isolated {
        // One venue: today's bars, with its threshold, over the time axis.
        let plot = Rect {
            height: inner.height - 1,
            ..inner
        };
        let mut bars =
            Sparkline::new(&selected.values, series_style(sel.venue)).threshold(chart.threshold);
        if let Some(span) = span {
            bars = bars.min_span(span);
        }
        f.render_widget(bars, plot);
        let gap = usize::from(inner.width).saturating_sub(ago.len() + now.len());
        f.render_widget(
            Paragraph::new(Line::styled(
                format!("{ago}{}{now}", " ".repeat(gap)),
                SECONDARY,
            )),
            Rect {
                y: inner.bottom() - 1,
                height: 1,
                ..inner
            },
        );
        return;
    }

    // Every venue: one braille line each, on one scale. Series align at
    // the right (now); a gap splits a line into segments.
    let x_of = |values: &[Option<f64>], i: usize| (len - values.len() + i) as f64;
    let mut segments: Vec<(usize, Vec<(f64, f64)>)> = Vec::new();
    for &i in &order {
        let values = &chart.series[i].values;
        let mut run = Vec::new();
        for (k, v) in values.iter().enumerate() {
            match v.filter(|v| v.is_finite()) {
                Some(v) => run.push((x_of(values, k), v)),
                None if !run.is_empty() => segments.push((i, std::mem::take(&mut run))),
                None => {}
            }
        }
        if !run.is_empty() {
            segments.push((i, run));
        }
    }
    let known = segments.iter().flat_map(|(_, s)| s.iter().map(|p| p.1));
    let (mut lo, mut hi) = known.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
        (lo.min(v), hi.max(v))
    });
    if !lo.is_finite() {
        return; // no value to draw yet
    }
    let span = span.unwrap_or(0.0).max(if hi > lo {
        0.0
    } else {
        lo.abs().max(1.0) * 0.01
    });
    if hi - lo < span {
        let mid = (lo + hi) / 2.0;
        (lo, hi) = (mid - span / 2.0, mid + span / 2.0);
    }
    // The scale is the venues' values, so they are told apart: a far
    // limit (`mid_price > 0` on a peg) would squash them into one row. The
    // limit's line shows when it is on the scale; the legend's glyphs say
    // how each venue stands against it either way.
    let line = chart
        .threshold
        .map(|t| match t {
            Threshold::Max(v) | Threshold::Min(v) | Threshold::Target(v) => v,
        })
        .filter(|t| (lo..=hi).contains(t));
    let x_max = (len.max(2) - 1) as f64;
    let threshold_line: Vec<(f64, f64)> =
        line.map(|t| vec![(0.0, t), (x_max, t)]).unwrap_or_default();
    let mut datasets = Vec::new();
    if !threshold_line.is_empty() {
        datasets.push(
            Dataset::default()
                .marker(Marker::Dot)
                .graph_type(GraphType::Line)
                .style(SECONDARY)
                .data(&threshold_line),
        );
    }
    for (i, points) in &segments {
        datasets.push(
            Dataset::default()
                .marker(if *i >= DOT_FROM {
                    Marker::Dot
                } else {
                    Marker::Braille
                })
                .graph_type(GraphType::Line)
                .style(series_style(*i))
                .data(points),
        );
    }
    let plot = Chart::new(datasets)
        .legend_position(None)
        .x_axis(
            Axis::default()
                .bounds([0.0, x_max])
                .style(SECONDARY)
                .labels([ago, now]),
        )
        .y_axis(
            Axis::default()
                .bounds([lo, hi])
                .style(SECONDARY)
                .labels([value_text(Some(lo)), value_text(Some(hi))]),
        );
    f.render_widget(plot, inner);
}

/// The venue legend: the range across venues, then each venue with its
/// colour, its last value on the selected chart and its status glyph.
fn draw_legend(
    f: &mut Frame,
    area: Rect,
    charts: &VenueCharts,
    sel: Selection,
    page_text: Option<String>,
) {
    let Some(chart) = charts.charts.get(sel.chart) else {
        return;
    };
    let mut block = Block::bordered()
        .border_style(SECONDARY)
        .title(Line::styled(format!(" venues: {} ", chart.label), ACCENT));
    if let Some(page) = page_text {
        block = block.title_bottom(Line::styled(page, SECONDARY).right_aligned());
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    let name = charts
        .venues()
        .iter()
        .map(|v| v.chars().count())
        .max()
        .unwrap_or(0);
    let mut lines = vec![Line::from(vec![
        Span::styled("  all  ", SECONDARY),
        Span::raw(range_text(chart)),
    ])];
    // Room for the venue rows under `all`: scroll so the selection shows.
    let room = usize::from(inner.height).saturating_sub(1).max(1);
    let first = sel.venue.saturating_sub(room - 1);
    for (i, s) in chart.series.iter().enumerate().skip(first).take(room) {
        let (glyph, glyph_style) = status_cell(&s.status);
        let dim = |st: Style| {
            if sel.isolated && i != sel.venue {
                st.add_modifier(Modifier::DIM)
            } else {
                st
            }
        };
        let marker = if i == sel.venue { "> " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(marker, ACCENT),
            Span::styled("■ ", dim(series_style(i))),
            Span::styled(format!("{:<name$} ", s.venue), dim(Style::new())),
            Span::styled(
                format!("{:>VALUE_COLS$} ", value_text(s.last())),
                dim(series_style(i)),
            ),
            Span::styled(glyph, glyph_style),
        ]));
    }
    f.render_widget(Paragraph::new(lines), inner);
}
