//! The repeating view (#58): a command that runs with `--every` draws one
//! in-place view instead of one report per run.
//!
//! Left (or top, when the pane is narrow): the latest snapshot, as wide and
//! as tall as its content (the compact market view when the run has its
//! data). Right (or below): the trends, one row per key value with a fixed
//! numbers column, a T bar and a chart that fills its track. A rule keeps
//! the footer apart. The series comes from the command's output
//! (`Output::series_hook`); this module only draws it.

use crate::cli::charts::{HealthStrip, Sparkline};
use crate::cli::output::{RunRecord, SeriesHook};
use crate::cli::theme::{ACCENT, ERR, SECONDARY, WARN};
use crate::cli::tui::{fit, market_view};
use ansi_to_tui::IntoText;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use scope::metrics::{Metric, MetricId, Threshold, Unit};

/// Columns of a trends label, its limit and its value.
const LABEL_COLS: usize = 17;
const LIMIT_COLS: usize = 9;
const VALUE_COLS: usize = 10;

/// The numbers part of a trends row: a space, the three columns, a space.
const NUMBERS_COLS: u16 = (1 + LABEL_COLS + LIMIT_COLS + VALUE_COLS + 1) as u16;

/// The narrowest trends: the numbers, the T bar and 20 chart cells.
const TRENDS_MIN_WIDTH: u16 = NUMBERS_COLS + 1 + 20;

/// The tallest chart, in rows.
const MAX_CHART_ROWS: u16 = 4;

/// The words that tell the user how to leave a finished view.
pub(crate) const LEAVE_HINT: &str = "Press any key to leave the view (the key still counts).";

/// How a finished repeat ended: its footer text and style.
#[derive(Clone, Debug)]
pub(crate) struct Ended {
    pub text: String,
    pub style: Style,
}

/// A repeating view: the series, and how the run ended once it has.
#[derive(Clone)]
pub(crate) struct RepeatView {
    pub series: SeriesHook,
    pub ended: Option<Ended>,
    /// The command's other output (notes, error text). It goes into the
    /// scrollback, with the final snapshot, when the user leaves.
    pub pending: String,
}

/// True when `series` holds at least one run.
pub(crate) fn has_runs(series: &SeriesHook) -> bool {
    !lock(series).runs.is_empty()
}

fn lock(series: &SeriesHook) -> std::sync::MutexGuard<'_, crate::cli::output::Series> {
    series.lock().unwrap_or_else(|e| e.into_inner())
}

/// What one draw needs, copied out so the lock is not held while drawing.
struct Copy {
    metrics: Vec<Metric>,
    runs: Vec<RunRecord>,
    dropped: u64,
}

fn copy(series: &SeriesHook) -> Copy {
    let s = lock(series);
    Copy {
        metrics: s.metrics.clone(),
        runs: s.runs.iter().cloned().collect(),
        dropped: s.dropped,
    }
}

/// The latest run's snapshot, as plain text (Ctrl-Y and `save`).
pub(crate) fn final_snapshot(series: &SeriesHook) -> Option<String> {
    lock(series).runs.back().map(|r| r.snapshot.clone())
}

/// The series as CSV, in the same columns as `--csv` writes.
pub(crate) fn series_csv(series: &SeriesHook) -> Option<String> {
    let c = copy(series);
    let columns = scope::metrics::csv_columns(&c.metrics).ok()?;
    let mut text = scope::metrics::csv_header(&columns);
    for r in &c.runs {
        let time = r.at.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        text.push_str(&scope::metrics::csv_row(
            &columns, &c.metrics, &time, r.run, &r.sample,
        ));
    }
    Some(text)
}

/// A bot on a timer, read across the runs: `bid and ask depth refill every
/// 6m 30s (8 cycles)`. One run's book cannot show it; the depth series can.
fn depth_cycle_note(c: &Copy) -> Option<String> {
    let side = |id: MetricId| {
        let i = c.metrics.iter().position(|m| m.id == id)?;
        let points: Vec<(i64, f64)> = c
            .runs
            .iter()
            .filter_map(|r| Some((r.at.timestamp_millis(), r.sample.values.get(i).copied()??)))
            .collect();
        scope::market::patterns::depth_cycle(&points)
    };
    let found: Vec<(&str, scope::market::patterns::DepthCycle)> = [
        ("bid", side(MetricId::BidDepth)),
        ("ask", side(MetricId::AskDepth)),
    ]
    .into_iter()
    .filter_map(|(name, c)| Some((name, c?)))
    .collect();
    let clause = |names: &str, verb: &str, c: &scope::market::patterns::DepthCycle| {
        let every = short_span(chrono::TimeDelta::milliseconds(c.period_ms.round() as i64));
        format!("{names} depth {verb} every {every} ({} cycles)", c.cycles)
    };
    match found.as_slice() {
        [] => None,
        [(name, c)] => Some(clause(name, "refills", c)),
        // One timer for both sides reads as one clause.
        [(_, b), (_, a)] if (b.period_ms - a.period_ms).abs() <= b.period_ms * 0.1 => {
            Some(clause("bid and ask", "refill", b))
        }
        [(bn, b), (an, a), ..] => Some(format!(
            "{}; {}",
            clause(bn, "refills", b),
            clause(an, "refills", a)
        )),
    }
}

/// The one-line summary pushed into the scrollback when the user leaves:
/// `42 runs · 1 failed (run 16)`, and a bot timer when the depth showed one.
pub(crate) fn summary(series: &SeriesHook) -> String {
    let c = copy(series);
    let timer = depth_cycle_note(&c)
        .map(|n| format!(" · bot timer: {n}"))
        .unwrap_or_default();
    let total = c.dropped + c.runs.len() as u64;
    let failed: Vec<u64> = c
        .runs
        .iter()
        .filter(|r| !r.sample.healthy)
        .map(|r| r.run)
        .collect();
    let runs = if total == 1 { "run" } else { "runs" };
    let scope = if c.dropped > 0 {
        format!(" in the last {}", c.runs.len())
    } else {
        String::new()
    };
    match failed.last() {
        None => format!("{total} {runs} · none failed{scope}{timer}"),
        Some(last) => format!(
            "{total} {runs} · {} failed{scope} (run {last}){timer}",
            failed.len()
        ),
    }
}

/// The value of `metric` as the trends show it.
fn format_value(metric: &Metric, value: Option<f64>) -> String {
    let Some(v) = value else {
        return "n/a".to_string();
    };
    match metric.unit {
        Unit::Price => scope::display::format::format_price(v),
        Unit::Percent => format!("{v:.3}%"),
        Unit::Quote => scope::display::format::format_large_number(v),
        Unit::Flag => if v != 0.0 { "pass" } else { "fail" }.to_string(),
        Unit::Usd => format!("${}", scope::display::format::format_large_number(v)),
        Unit::Count => format!("{v:.0}"),
        Unit::Native => scope::display::format::format_price(v),
        Unit::Score => format!("{v:.1}"),
    }
}

/// The metrics the trends chart. Best bid and ask and the absolute spread
/// repeat the mid and the spread %; health has its own strip.
fn charted(id: MetricId) -> bool {
    !matches!(
        id,
        MetricId::BestBid | MetricId::BestAsk | MetricId::Spread | MetricId::Healthy
    )
}

/// Draws `view` in `area` (design 2026-10-21). `status` goes on the last
/// rows, under a rule: the run's progress while it runs, or the footer and
/// the leave hint after.
pub(crate) fn draw(f: &mut Frame, area: Rect, view: &RepeatView, status: Vec<Line<'static>>) {
    let c = copy(&view.series);
    let mut foot_lines = vec![one_line(status)];
    if let Some(ended) = &view.ended {
        foot_lines = vec![
            Line::styled(ended.text.clone(), ended.style),
            Line::styled(LEAVE_HINT, SECONDARY),
        ];
    }
    // When the window hides or squeezes part of the view, a line under
    // the status says how much larger it needs to be (its own row too).
    let rows_for = |lines: usize| (lines as u16 + 1).min(area.height);
    let plain = area.height.saturating_sub(rows_for(foot_lines.len()));
    if let Some((cols, rows)) = shortfall(
        Rect {
            height: plain,
            ..area
        },
        &c,
    ) {
        foot_lines.push(Line::styled(enlarge_hint(cols, rows), WARN));
    }
    // J5: a rule keeps the footer off the view above it.
    let foot_rows = rows_for(foot_lines.len());
    let [body, foot] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(foot_rows)]).areas(area);
    if foot.height > 0 {
        let rule = Rect { height: 1, ..foot };
        f.render_widget(
            Paragraph::new(Line::styled("─".repeat(foot.width as usize), SECONDARY)),
            rule,
        );
        let rest = Rect {
            y: foot.y + 1,
            height: foot.height - 1,
            ..foot
        };
        f.render_widget(Paragraph::new(Text::from(foot_lines)), rest);
    }
    let layout = Layout2::new(body, c.runs.last());
    if let Some(snap) = layout.snap {
        f.render_widget(Paragraph::new(Text::from(layout.lines.clone())), snap);
    }
    if let Some(rule) = layout.rule {
        let glyph = if layout.side_by_side { "│" } else { "─" };
        let lines: Vec<Line> = if layout.side_by_side {
            (0..rule.height)
                .map(|_| Line::styled(glyph, SECONDARY))
                .collect()
        } else {
            vec![Line::styled(glyph.repeat(rule.width as usize), SECONDARY)]
        };
        f.render_widget(Paragraph::new(lines), rule);
    }
    draw_trends(f, layout.trends, &c);
}

/// Fewest rows a chart needs for its shape to read.
const READABLE_CHART_ROWS: u16 = 2;

/// How many columns and rows `body` lacks to show the last run's full
/// snapshot (every level, nothing cut) beside the trends with every chart
/// readable; `None` when it shows it all.
fn shortfall(body: Rect, c: &Copy) -> Option<(u16, u16)> {
    let view = c.runs.last()?.view.as_ref()?;
    let full = market_view::lines_with(
        view,
        market_view::Opts {
            with_trends: true,
            ..market_view::Opts::of(market_view::Size::Full)
        },
    );
    let width = market_view::width(&full) as u16 + 2 + TRENDS_MIN_WIDTH;
    let charts = c.metrics.iter().filter(|m| charted(m.id)).count() as u16;
    let failed = c.runs.iter().any(|r| !r.sample.healthy);
    // The trends' title, health strip, last failure and time axis.
    let trends = 3 + u16::from(failed) + charts * READABLE_CHART_ROWS;
    let height = (full.len() as u16).max(trends);
    let short = (
        width.saturating_sub(body.width),
        height.saturating_sub(body.height),
    );
    (short != (0, 0)).then_some(short)
}

/// `! Window too small: enlarge it 12 columns wider and 8 rows taller …`.
fn enlarge_hint(cols: u16, rows: u16) -> String {
    let n = |k: u16, one: &str, many: &str| format!("{k} {}", if k == 1 { one } else { many });
    let by = match (cols, rows) {
        (0, r) => format!("{} taller", n(r, "row", "rows")),
        (c, 0) => format!("{} wider", n(c, "column", "columns")),
        (c, r) => format!(
            "{} wider and {} taller",
            n(c, "column", "columns"),
            n(r, "row", "rows")
        ),
    };
    format!("! Window too small: enlarge it {by} to see everything")
}

/// The status lines as one line, the progress before the long command:
/// `⠙ run 3 of 4 · next in 1s · running market summary … 12s`.
fn one_line(status: Vec<Line<'static>>) -> Line<'static> {
    let mut lines = status.into_iter();
    let Some(first) = lines.next() else {
        return Line::default();
    };
    let mut first = first.spans.into_iter();
    // The first span of the running line is its spinner.
    let mut spans: Vec<Span<'static>> = first.next().into_iter().collect();
    for line in lines {
        spans.push(Span::raw(" "));
        spans.extend(line.spans);
        spans.push(Span::styled(" ·", SECONDARY));
    }
    spans.extend(first);
    Line::from(spans)
}

/// The snapshot of the last run: the compact market view when the run has
/// its data, else its text report.
/// The snapshot in at most `rows` lines when it can. The trends chart the
/// key values, so the market view leaves them out.
fn snapshot_lines(
    last: Option<&RunRecord>,
    size: market_view::Size,
    rows: usize,
) -> Vec<Line<'static>> {
    let Some(last) = last else {
        return Vec::new();
    };
    if let Some(view) = &last.view {
        let opts = market_view::Opts {
            with_trends: true,
            ..market_view::Opts::of(size)
        };
        return market_view::fit(view, opts, rows);
    }
    last.snapshot
        .as_str()
        .into_text()
        .unwrap_or_else(|_| Text::raw(last.snapshot.clone()))
        .lines
}

/// Where the snapshot, the rule between and the trends go (J2): the
/// snapshot is as wide and as tall as its content; the trends get the rest.
struct Layout2 {
    lines: Vec<Line<'static>>,
    snap: Option<Rect>,
    rule: Option<Rect>,
    trends: Rect,
    side_by_side: bool,
}

impl Layout2 {
    fn new(body: Rect, last: Option<&RunRecord>) -> Self {
        let full = snapshot_lines(last, market_view::Size::Full, body.height as usize);
        // One blank column between the snapshot and its rule.
        let full_width = market_view::width(&full) as u16 + 1;
        let side_by_side = full_width > 0 && body.width >= full_width + 1 + TRENDS_MIN_WIDTH;
        if side_by_side {
            let snap = Rect {
                width: full_width,
                height: (full.len() as u16).min(body.height),
                ..body
            };
            let rule = Rect {
                x: body.x + full_width,
                width: 1,
                ..body
            };
            let trends = Rect {
                x: body.x + full_width + 1,
                width: body.width - full_width - 1,
                ..body
            };
            return Self {
                lines: full,
                snap: Some(snap),
                rule: Some(rule),
                trends,
                side_by_side,
            };
        }
        // Stacked: the compact snapshot on top, at its height, but never
        // more than half the body; a rule; the trends under it.
        let compact = snapshot_lines(last, market_view::Size::Compact, (body.height / 2) as usize);
        let lines = fit::fit(compact, body.width as usize);
        let rows = (lines.len() as u16).min(body.height / 2);
        if rows == 0 {
            return Self {
                lines,
                snap: None,
                rule: None,
                trends: body,
                side_by_side,
            };
        }
        let snap = Rect {
            height: rows,
            ..body
        };
        let rule = Rect {
            y: body.y + rows,
            height: 1,
            ..body
        };
        let trends = Rect {
            y: body.y + rows + 1,
            height: body.height.saturating_sub(rows + 1),
            ..body
        };
        Self {
            lines,
            snap: Some(snap),
            rule: Some(rule),
            trends,
            side_by_side,
        }
    }
}

/// The limit of a metric as the trends show it: `≤3.000%`, `≥3.00K`.
fn limit_text(metric: &Metric) -> String {
    let n = |v: f64| {
        let number = market_view::compact_number(v);
        match metric.unit {
            Unit::Percent => format!("{number}%"),
            _ => number,
        }
    };
    match metric.threshold {
        Some(Threshold::Max(v)) => format!("≤{}", n(v)),
        Some(Threshold::Min(v)) => format!("≥{}", n(v)),
        Some(Threshold::Target(v)) => format!("peg {}", n(v)),
        None => String::new(),
    }
}

/// The smallest range a metric's chart shows, so noise reads as a flat
/// line: 10 bps of the price, 0.05 percentage points, 5% of an amount.
fn min_span(metric: &Metric, values: &[Option<f64>]) -> f64 {
    let latest = values
        .iter()
        .rev()
        .flatten()
        .next()
        .copied()
        .unwrap_or(0.0)
        .abs();
    match metric.unit {
        Unit::Price | Unit::Native => (latest * 0.001).max(1e-9),
        Unit::Percent => 0.05,
        Unit::Quote | Unit::Usd => (latest * 0.05).max(1.0),
        Unit::Count => 1.0,
        Unit::Score => 0.5,
        Unit::Flag => 1.0,
    }
}

/// The numbers part of a trends row, always `NUMBERS_COLS` wide (J3):
/// the label, the limit and the value, right-aligned so it is never cut.
fn numbers(label: &str, limit: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {:<w$}", cut(label, LABEL_COLS - 1), w = LABEL_COLS),
            SECONDARY,
        ),
        Span::styled(
            format!("{:<w$}", cut(limit, LIMIT_COLS), w = LIMIT_COLS),
            SECONDARY,
        ),
        Span::raw(format!("{:>w$} ", cut(value, VALUE_COLS), w = VALUE_COLS)),
    ])
}

fn cut(text: &str, cols: usize) -> String {
    if text.chars().count() <= cols {
        text.to_string()
    } else {
        let mut t: String = text.chars().take(cols.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// The T bar between the numbers and a chart `rows` high (J4): `┤` on the
/// chart's base row, `│` above it.
fn tee(f: &mut Frame, x: u16, y: u16, rows: u16) {
    let lines: Vec<Line> = (0..rows)
        .map(|r| Line::styled(if r + 1 == rows { "┤" } else { "│" }, SECONDARY))
        .collect();
    f.render_widget(Paragraph::new(lines), Rect::new(x, y, 1, rows));
}

fn draw_trends(f: &mut Frame, area: Rect, c: &Copy) {
    let run = c.runs.last().map_or(0, |r| r.run);
    if area.height < 2 || area.width <= NUMBERS_COLS + 2 {
        return;
    }
    let mut title = vec![Span::styled(format!(" Trends · run {run}"), ACCENT)];
    if let Some(note) = depth_cycle_note(c) {
        title.push(Span::styled(format!("  ! Bot timer: {note}"), WARN));
    }
    f.render_widget(
        Paragraph::new(Line::from(title)),
        Rect { height: 1, ..area },
    );
    let chart_x = area.x + NUMBERS_COLS + 1;
    let chart_w = area.width - NUMBERS_COLS - 1;
    let charted: Vec<(usize, &Metric)> = c
        .metrics
        .iter()
        .enumerate()
        .filter(|(_, m)| charted(m.id))
        .collect();
    let last_failure = c.runs.iter().rev().find(|r| !r.sample.healthy);
    // Rows: the header, health (and its last failure), the charts, then
    // the time axis under them.
    let axis = !c.runs.is_empty();
    let fixed = 2 + u16::from(last_failure.is_some()) + u16::from(axis);
    let free = area.height.saturating_sub(fixed);
    // Q5: rows left over after one per metric make the charts taller.
    // The rows that stay after an even share go one each to the first
    // charts, so no row is left empty under the last one.
    let n = (charted.len() as u16).max(1);
    let per_chart = (free / n).clamp(1, MAX_CHART_ROWS);
    let mut extra = if per_chart < MAX_CHART_ROWS {
        free.saturating_sub(per_chart * n)
    } else {
        0
    };

    let mut y = area.y + 1;
    let healthy: Vec<bool> = c.runs.iter().map(|r| r.sample.healthy).collect();
    let failed = healthy.iter().filter(|h| !**h).count();
    let verdict = if failed == 0 {
        "ok".to_string()
    } else {
        format!("{failed} failed")
    };
    f.render_widget(
        Paragraph::new(numbers("Health", "", &verdict)),
        Rect::new(area.x, y, NUMBERS_COLS, 1),
    );
    tee(f, area.x + NUMBERS_COLS, y, 1);
    f.render_widget(
        HealthStrip::new(&healthy).track(),
        Rect::new(chart_x, y, chart_w, 1),
    );
    y += 1;
    if let Some(r) = last_failure {
        let why = r
            .sample
            .failed_checks
            .first()
            .map_or("unhealthy", String::as_str);
        f.render_widget(
            Paragraph::new(Line::styled(
                format!("   last failure: run {}: {why}", r.run),
                ERR,
            )),
            Rect::new(area.x, y, area.width, 1),
        );
        y += 1;
    }

    for (i, metric) in charted {
        let rows = per_chart + u16::from(extra > 0);
        extra = extra.saturating_sub(1);
        if y + rows > area.bottom() {
            break;
        }
        let values: Vec<Option<f64>> = c
            .runs
            .iter()
            .map(|r| r.sample.values.get(i).copied().flatten())
            .collect();
        let latest = values.last().copied().flatten();
        let base = y + rows - 1;
        f.render_widget(
            Paragraph::new(numbers(
                metric.label,
                &limit_text(metric),
                &format_value(metric, latest),
            )),
            Rect::new(area.x, base, NUMBERS_COLS, 1),
        );
        tee(f, area.x + NUMBERS_COLS, y, rows);
        f.render_widget(
            Sparkline::new(&values, ACCENT)
                .threshold(metric.threshold)
                .min_span(min_span(metric, &values))
                .track(),
            Rect::new(chart_x, y, chart_w, rows),
        );
        y += rows;
    }

    // The time axis under the charts, in local time: when the runs shown
    // start, how long they span, and the latest run under its bar.
    if axis && y < area.bottom() {
        let now = chrono::Local::now();
        f.render_widget(
            Paragraph::new(numbers(&axis_label(now.offset()), "", "")),
            Rect::new(area.x, y, NUMBERS_COLS, 1),
        );
        f.render_widget(
            Paragraph::new(Line::styled(
                time_axis(&c.runs, chart_w, &chrono::Local),
                SECONDARY,
            )),
            Rect::new(chart_x, y, chart_w, 1),
        );
    }
}

/// The axis label: the time zone the axis times are in.
fn axis_label(offset: &impl chrono::Offset) -> String {
    let secs = offset.fix().local_minus_utc();
    if secs == 0 {
        "Time (UTC)".to_string()
    } else {
        format!(
            "Time (UTC{}{:02}:{:02})",
            if secs < 0 { '-' } else { '+' },
            secs.abs() / 3600,
            secs.abs() % 3600 / 60
        )
    }
}

/// A span of time in short form: `45s`, `2m 30s`, `1h 05m`.
fn short_span(span: chrono::TimeDelta) -> String {
    let s = span.num_seconds().max(0);
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m", s / 3600, s % 3600 / 60),
    }
}

/// The time axis under a track `width` cells wide. The track fills from
/// the left, one cell per run, and scrolls when full, so the axis shows
/// the newest `width` runs the same way: the first one's clock time at
/// the left, the latest's ending under its bar, and the span between
/// them. What does not fit is left out: the span first, then the start.
fn time_axis<Tz: chrono::TimeZone>(runs: &[RunRecord], width: u16, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let shown = &runs[runs.len().saturating_sub(width as usize)..];
    let (Some(first), Some(last)) = (shown.first(), shown.last()) else {
        return String::new();
    };
    let clock = |r: &RunRecord| r.at.with_timezone(tz).format("%H:%M:%S").to_string();
    let (start, end) = (clock(first), clock(last));
    let filled = shown.len();
    if filled < start.len() + end.len() + 1 {
        // Only the latest time: ending under the newest bar when it can.
        let lead = filled.saturating_sub(end.len());
        return format!("{}{end}", " ".repeat(lead));
    }
    let mid = short_span(last.at - first.at);
    let gap = filled - start.len() - end.len();
    if gap >= mid.len() + 2 {
        let left = (gap - mid.len()) / 2;
        let right = gap - mid.len() - left;
        format!("{start}{}{mid}{}{end}", " ".repeat(left), " ".repeat(right))
    } else {
        format!("{start}{}{end}", " ".repeat(gap))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::output::Output;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use scope::market::HealthThresholds;
    use scope::metrics::{Sample, summary_metrics};

    /// The market data of a run: a 20-level book.
    fn summary_view() -> std::sync::Arc<crate::cli::output::SummaryView> {
        use scope::market::{BookSource, MarketSummary, OrderBook, OrderBookLevel};
        let side = |start: f64, step: f64| -> Vec<OrderBookLevel> {
            (0..20)
                .map(|i| OrderBookLevel {
                    price: start + step * f64::from(i),
                    quantity: 40_000.0,
                })
                .collect()
        };
        let book = OrderBook {
            pair: "USDC/USDT".to_string(),
            source: BookSource::Exchange,
            bids: side(0.9999, -0.0001),
            asks: side(1.0001, 0.0001),
        };
        std::sync::Arc::new(crate::cli::output::SummaryView {
            summary: MarketSummary::from_order_book(&book, &HealthThresholds::default(), None),
            venue: "binance".to_string(),
            tokens: Vec::new(),
        })
    }

    /// A series of `n` runs; `failed` runs are unhealthy.
    fn series(n: u64, failed: &[u64]) -> SeriesHook {
        series_with(n, failed, false)
    }

    /// A series of `n` runs, with the market data of each when `data`.
    fn series_with(n: u64, failed: &[u64], data: bool) -> SeriesHook {
        let metrics = summary_metrics(&HealthThresholds::default());
        let (out, _cap) = Output::capture_merged();
        for run in 1..=n {
            let healthy = !failed.contains(&run);
            let sample = Sample {
                values: metrics
                    .iter()
                    .map(|_| Some(1.0 + run as f64 / 100.0))
                    .collect(),
                healthy,
                failed_checks: if healthy {
                    vec![]
                } else {
                    vec![format!("spread 3.{run}%")]
                },
            };
            out.record_run(
                &metrics,
                RunRecord {
                    run,
                    at: chrono::Utc::now(),
                    sample,
                    snapshot: format!("Market Summary run {run}\nMid 1.0\n"),
                    view: data.then(summary_view),
                },
            );
        }
        out.series_hook().unwrap()
    }

    fn render(view: &RepeatView, w: u16, h: u16) -> Vec<String> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, f.area(), view, vec![Line::raw("run 3 of 10")]))
            .unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect())
            .collect()
    }

    fn view(n: u64, failed: &[u64]) -> RepeatView {
        RepeatView {
            series: series(n, failed),
            ended: None,
            pending: String::new(),
        }
    }

    /// A view whose runs carry their market data (the compact snapshot).
    fn market(n: u64) -> RepeatView {
        RepeatView {
            series: series_with(n, &[], true),
            ended: None,
            pending: String::new(),
        }
    }

    /// The column of the first `┤` in each row that has one.
    fn tees(rows: &[String]) -> Vec<usize> {
        rows.iter()
            .filter_map(|r| r.chars().position(|c| c == '┤'))
            .collect()
    }

    #[test]
    fn test_the_snapshot_is_as_wide_as_its_content_and_the_trends_take_the_rest() {
        // J2: at 120 and at 160 columns the snapshot keeps its width; the
        // trends get the extra columns (more history).
        let at = |w: u16| {
            let rows = render(&market(3), w, 36);
            let rule = rows[1].chars().position(|c| c == '│').unwrap();
            let track = rows.iter().find(|r| r.contains("Mid price")).unwrap();
            (rule, track.chars().filter(|c| *c == '─').count(), rows)
        };
        let (rule_120, track_120, rows) = at(120);
        let (rule_160, track_160, _) = at(160);
        assert_eq!(rule_120, rule_160, "the snapshot does not grow");
        assert!(track_160 > track_120 + 30, "the track does");
        assert!(
            rows[0].contains("USDC/USDT · binance"),
            "{}",
            rows.join("\n")
        );
        assert!(rows[0].contains("✓ HEALTHY"));
    }

    #[test]
    fn test_the_numbers_column_has_one_width_and_a_tee_before_each_chart() {
        // J3, J4: every `┤` is in the same column, whatever the values.
        for (w, h) in [(120, 36), (160, 48), (79, 24)] {
            let rows = render(&market(3), w, h);
            let cols = tees(&rows);
            assert!(cols.len() >= 4, "{w}x{h}\n{}", rows.join("\n"));
            assert!(cols.windows(2).all(|p| p[0] == p[1]), "{w}x{h}: {cols:?}");
        }
    }

    #[test]
    fn test_extra_height_makes_the_charts_taller() {
        // Q5: rows under a `┤` with `│` above it are a chart taller than one.
        let short = render(&market(3), 120, 24);
        let tall = render(&market(3), 120, 48);
        let above = |rows: &[String]| {
            let col = tees(rows)[0];
            rows.iter()
                .filter(|r| r.chars().nth(col) == Some('│'))
                .count()
        };
        assert!(above(&tall) > above(&short), "{}", tall.join("\n"));
    }

    /// The left column of a side-by-side render, up to the rule.
    fn left(rows: &[String]) -> Vec<String> {
        let col = rows[1].chars().position(|c| c == '│').unwrap();
        rows.iter().map(|r| r.chars().take(col).collect()).collect()
    }

    #[test]
    fn test_no_value_shows_twice_beside_the_trends() {
        let rows = render(&market(3), 120, 36);
        let snap = left(&rows).join(
            "
",
        );
        for charted in ["Mid ", "Spread ", "Depth ", "Top-3 ", "Top-10 "] {
            assert!(
                !snap.lines().any(|l| l.trim_start().starts_with(charted)),
                "{charted} is in the trends: {snap}"
            );
        }
        let text = rows.join(
            "
",
        );
        assert!(
            text.contains("Mid price") && text.contains("Bid depth"),
            "{text}"
        );
    }

    #[test]
    fn test_the_snapshot_fits_its_rows() {
        // The last line of the snapshot is on screen at 120×24: it gives
        // up ladder levels first.
        let rows = render(&market(3), 120, 24);
        let snap = left(&rows).join(
            "
",
        );
        assert!(snap.contains("Checks · "), "{snap}");
        assert!(snap.contains("mid 1.000000"), "the ladder stays: {snap}");
        // The body is the pane less the footer's rule and line.
        let view = summary_view();
        let opts = market_view::Opts {
            with_trends: true,
            ..market_view::Opts::of(market_view::Size::Full)
        };
        let fitted = market_view::fit(&view, opts, 22);
        assert!(fitted.len() <= 22);
        assert!(fitted.len() < market_view::lines_with(&view, opts).len());
        let wanted = fitted.last().unwrap().to_string();
        assert!(
            snap.contains(wanted.trim()),
            "the last line is shown: {wanted}
{snap}"
        );
    }

    #[test]
    fn test_limits_are_short_and_health_says_ok() {
        let text = render(&market(3), 120, 36).join(
            "
",
        );
        assert!(text.contains("≤3%"), "{text}");
        assert!(text.contains("≥3K"), "{text}");
        assert!(text.contains("peg 1 "), "{text}");
        assert!(!text.contains("3.000%"), "{text}");
        let health = text.lines().find(|l| l.contains("Health")).unwrap();
        assert!(health.contains(" ok ┤"), "{health}");
    }

    #[test]
    fn test_a_window_too_small_says_how_much_to_enlarge_it() {
        // Narrow and short: the compact snapshot, the levels cut.
        let rows = render(&market(3), 79, 24);
        let last = rows.last().unwrap();
        assert!(
            last.starts_with("! Window too small: enlarge it "),
            "{rows:#?}"
        );
        assert!(last.contains("columns wider and"), "{last}");
        assert!(rows[rows.len() - 2].starts_with("run 3 of 10"), "{rows:#?}");
        // Large enough for it all: no hint.
        let rows = render(&market(3), 200, 80);
        assert!(
            !rows.iter().any(|r| r.contains("Window too small")),
            "{rows:#?}"
        );
        // Wide enough but too short: rows only.
        let rows = render(&market(3), 200, 20);
        let last = rows.last().unwrap();
        assert!(last.contains("rows taller to see everything"), "{last}");
        assert!(!last.contains("wider"), "{last}");
    }

    #[test]
    fn test_a_rule_keeps_the_footer_off_the_view() {
        // J5: the row above the status line is a rule across the pane.
        let rows = render(&market(3), 120, 36);
        let rule = &rows[34];
        assert!(rule.chars().all(|c| c == '─'), "{rule}");
        assert!(rows[35].starts_with("run 3 of 10"));
    }

    #[test]
    fn test_narrow_market_view_stacks_the_compact_snapshot_over_the_trends() {
        let rows = render(&market(3), 79, 24);
        let text = rows.join("\n");
        assert!(rows[0].contains("USDC/USDT · binance"), "{text}");
        assert!(
            text.contains("Book ask 1.000100"),
            "the one-line book: {text}"
        );
        let rule = rows
            .iter()
            .position(|r| r.chars().all(|c| c == '─'))
            .unwrap();
        let trends = rows.iter().position(|r| r.contains("Trends")).unwrap();
        assert!(rule < trends, "{text}");
        for row in rows.iter().filter(|r| r.contains("depth")) {
            assert_eq!(row.matches("depth").count(), 1, "{text}");
        }
    }

    #[test]
    fn test_wide_view_shows_snapshot_beside_the_trends() {
        let rows = render(&view(3, &[2]), 120, 36);
        let text = rows.join("\n");
        assert!(rows[0].starts_with("Market Summary run 3"), "{text}");
        assert!(text.contains("Trends · run 3"), "{text}");
        for label in ["Health", "Mid price", "Spread %", "Bid depth"] {
            assert!(text.contains(label), "{label}\n{text}");
        }
        assert!(text.contains("run 2: spread 3.2%"), "{text}");
        assert!(text.contains("✓✗✓"), "{text}");
        assert!(rows[35].starts_with("run 3 of 10"), "{text}");
    }

    #[test]
    fn test_a_text_snapshot_is_sized_to_its_content() {
        // A command without market data shows its text: beside the trends
        // when they fit next to it, else above them at its own height.
        let wide = render(&view(3, &[]), 120, 24);
        assert!(wide[0].starts_with("Market Summary run 3"));
        assert!(
            wide[0].contains("Trends · run 3"),
            "{}",
            wide.join(
                "
"
            )
        );
        let narrow = render(&view(3, &[]), 79, 24);
        let text = narrow.join(
            "
",
        );
        assert!(narrow[0].starts_with("Market Summary run 3"), "{text}");
        assert!(
            narrow[2].chars().all(|c| c == '─'),
            "two rows of text, then the rule: {text}"
        );
        for row in narrow.iter().filter(|r| r.contains("depth")) {
            assert_eq!(row.matches("depth").count(), 1, "{text}");
        }
    }

    #[test]
    fn test_a_finished_view_shows_its_footer_and_how_to_leave() {
        let mut v = view(2, &[]);
        v.ended = Some(Ended {
            text: "Cancelled after 4.0s.".into(),
            style: Style::new(),
        });
        let text = render(&v, 120, 36).join("\n");
        assert!(text.contains("Cancelled after 4.0s."), "{text}");
        assert!(text.contains(LEAVE_HINT), "{text}");
    }

    #[test]
    fn test_summary_counts_runs_and_names_the_last_failure() {
        assert_eq!(
            summary(&series(42, &[3, 16])),
            "42 runs · 2 failed (run 16)"
        );
        assert_eq!(summary(&series(1, &[])), "1 run · none failed");
    }

    #[test]
    fn test_series_csv_has_the_cli_header_and_one_row_per_run() {
        let csv = series_csv(&series(3, &[])).unwrap();
        let lines: Vec<&str> = csv.lines().collect();
        assert!(lines[0].starts_with("timestamp,run,best_bid,"), "{csv}");
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[3].split(',').nth(1), Some("3"));
    }

    #[test]
    fn test_values_read_in_their_unit() {
        let m = summary_metrics(&HealthThresholds::default());
        let by = |id| m.iter().find(|x| x.id == id).unwrap();
        assert_eq!(format_value(by(MetricId::SpreadPct), Some(0.25)), "0.250%");
        assert_eq!(
            format_value(by(MetricId::BidDepth), Some(1_500_000.0)),
            "1.50M"
        );
        assert_eq!(format_value(by(MetricId::Mid), None), "n/a");
        // A meme-coin price keeps its digits (the 0.8.0 smoke test: PEPE
        // read as 0).
        assert_eq!(
            format_value(by(MetricId::Mid), Some(0.0000106)),
            "0.0000106000"
        );
    }

    /// `n` runs, `every` seconds apart from 10:00:00 UTC.
    fn runs_every(n: u64, every: i64) -> Vec<RunRecord> {
        use chrono::TimeZone;
        let t0 = chrono::Utc
            .with_ymd_and_hms(2026, 10, 22, 10, 0, 0)
            .unwrap();
        (0..n)
            .map(|i| RunRecord {
                run: i + 1,
                at: t0 + chrono::TimeDelta::seconds(i as i64 * every),
                sample: Sample {
                    values: vec![],
                    healthy: true,
                    failed_checks: vec![],
                },
                snapshot: String::new(),
                view: None,
            })
            .collect()
    }

    #[test]
    fn test_the_time_axis_starts_at_the_first_bar_and_ends_under_the_newest() {
        // 31 runs, 5s apart, on a 40-cell track: they fill cells 0-30.
        let axis = time_axis(&runs_every(31, 5), 40, &chrono::Utc);
        assert!(axis.starts_with("10:00:00"), "{axis:?}");
        assert!(axis.contains(" 2m 30s "), "{axis:?}");
        assert_eq!(
            axis.chars().count(),
            31,
            "ends under the newest bar: {axis:?}"
        );
        assert!(axis.ends_with("10:02:30"), "{axis:?}");
    }

    #[test]
    fn test_a_full_track_shows_the_newest_runs_that_fit() {
        // 100 runs, 60s apart, 30 cells: the newest 30 (runs 71-100).
        let axis = time_axis(&runs_every(100, 60), 30, &chrono::Utc);
        assert_eq!(axis, "11:10:00   29m 00s    11:39:00");
    }

    #[test]
    fn test_few_runs_show_only_the_latest_time() {
        // Room for both times needs 17 cells; with fewer, only the latest,
        // ending under its bar when it can.
        assert_eq!(time_axis(&runs_every(1, 5), 40, &chrono::Utc), "10:00:00");
        assert_eq!(time_axis(&runs_every(3, 5), 40, &chrono::Utc), "10:00:10");
        assert_eq!(
            time_axis(&runs_every(16, 5), 40, &chrono::Utc),
            format!("{}10:01:15", " ".repeat(8))
        );
        assert_eq!(time_axis(&[], 40, &chrono::Utc), "");
    }

    #[test]
    fn test_axis_times_are_in_the_given_zone_and_the_label_names_it() {
        let cet = chrono::FixedOffset::east_opt(2 * 3600).unwrap();
        let axis = time_axis(&runs_every(31, 5), 40, &cet);
        assert!(
            axis.starts_with("12:00:00") && axis.ends_with("12:02:30"),
            "{axis:?}"
        );
        assert_eq!(
            axis_label(&chrono::FixedOffset::east_opt(0).unwrap()),
            "Time (UTC)"
        );
        assert_eq!(axis_label(&cet), "Time (UTC+02:00)");
        let west = chrono::FixedOffset::west_opt(5 * 3600 + 1800).unwrap();
        assert_eq!(axis_label(&west), "Time (UTC-05:30)");
    }

    #[test]
    fn test_short_spans() {
        let s = |secs| short_span(chrono::TimeDelta::seconds(secs));
        assert_eq!(s(45), "45s");
        assert_eq!(s(150), "2m 30s");
        assert_eq!(s(3_900), "1h 05m");
    }

    #[test]
    fn test_the_trends_end_with_the_time_axis_under_the_charts() {
        let rows = render(&view(3, &[]), 120, 36);
        let text = rows.join(
            "
",
        );
        let axis = rows
            .iter()
            .position(|r| r.contains("Time (UTC"))
            .unwrap_or_else(|| {
                panic!(
                    "no axis:
{text}"
                )
            });
        let last_chart = rows.iter().rposition(|r| r.contains('┤')).unwrap();
        assert!(
            axis > last_chart,
            "the axis is under the charts:
{text}"
        );
    }

    /// `n` runs 30 s apart whose bid and ask depth are refilled every
    /// `every` runs and drained a step a run between (the weex USDN book).
    fn depth_series(n: u64, every: u64) -> SeriesHook {
        let metrics = summary_metrics(&HealthThresholds::default());
        let (out, _cap) = Output::capture_merged();
        let start = chrono::Utc::now() - chrono::TimeDelta::seconds(30 * n as i64);
        for run in 1..=n {
            let depth = 3_300.0 - ((run - 1) % every) as f64 * 250.0;
            let values = metrics
                .iter()
                .map(|m| match m.id {
                    MetricId::BidDepth | MetricId::AskDepth => Some(depth),
                    _ => Some(1.0),
                })
                .collect();
            out.record_run(
                &metrics,
                RunRecord {
                    run,
                    at: start + chrono::TimeDelta::seconds(30 * run as i64),
                    sample: Sample {
                        values,
                        healthy: true,
                        failed_checks: vec![],
                    },
                    snapshot: format!("run {run}\n"),
                    view: None,
                },
            );
        }
        out.series_hook().unwrap()
    }

    #[test]
    fn test_a_book_refilled_on_a_timer_is_named_in_the_title_and_summary() {
        let series = depth_series(110, 13);
        assert_eq!(
            summary(&series),
            "110 runs · none failed · bot timer: bid and ask depth refill every 6m 30s (7 cycles)"
        );
        let view = RepeatView {
            series,
            ..market(1)
        };
        let rows = render(&view, 160, 30);
        let title = rows
            .iter()
            .find(|r| r.contains("Trends · run 110"))
            .unwrap();
        assert!(
            title.contains("! Bot timer: bid and ask depth refill every 6m 30s (7 cycles)"),
            "{title}"
        );
        // Too few cycles yet: nothing.
        assert_eq!(summary(&depth_series(30, 13)), "30 runs · none failed");
    }

    #[test]
    fn test_one_side_on_a_timer_or_two_timers_are_told_apart() {
        let c = |bid_every: u64, ask_every: Option<u64>| {
            let metrics = summary_metrics(&HealthThresholds::default());
            let start = chrono::Utc::now();
            let runs = (0..120u64)
                .map(|i| {
                    let saw = |every: u64| 3_300.0 - (i % every) as f64 * 200.0;
                    RunRecord {
                        run: i + 1,
                        at: start + chrono::TimeDelta::seconds(30 * i as i64),
                        sample: Sample {
                            values: metrics
                                .iter()
                                .map(|m| match m.id {
                                    MetricId::BidDepth => Some(saw(bid_every)),
                                    MetricId::AskDepth => {
                                        Some(ask_every.map_or(2_000.0 + (i % 7) as f64, saw))
                                    }
                                    _ => None,
                                })
                                .collect(),
                            healthy: true,
                            failed_checks: vec![],
                        },
                        snapshot: String::new(),
                        view: None,
                    }
                })
                .collect();
            depth_cycle_note(&Copy {
                metrics,
                runs,
                dropped: 0,
            })
        };
        assert_eq!(
            c(10, None).as_deref(),
            Some("bid depth refills every 5m 00s (10 cycles)")
        );
        assert_eq!(
            c(10, Some(15)).as_deref(),
            Some(
                "bid depth refills every 5m 00s (10 cycles); ask depth refills every 7m 30s (6 cycles)"
            )
        );
    }
}
