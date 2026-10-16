//! The repeating view (#58): a command that runs with `--every` draws one
//! in-place view instead of one report per run.
//!
//! Left (or top, when the pane is narrow): the latest snapshot. Right (or
//! below): the trends, a health strip with the last failure under it, then
//! one chart per key value, as many as fit. The series comes from the
//! command's output (`Output::series_hook`); this module only draws it.

use crate::cli::charts::{HealthStrip, Sparkline};
use crate::cli::output::{RunRecord, SeriesHook};
use crate::cli::theme::{ACCENT, SECONDARY};
use crate::cli::tui::fit;
use ansi_to_tui::IntoText;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use scope::metrics::{Metric, MetricId, Unit};

/// The pane width from which the snapshot and the trends sit side by side.
pub(crate) const SIDE_BY_SIDE_MIN_WIDTH: u16 = 80;

/// Rows one chart takes: its label row and its bar row.
const CHART_ROWS: u16 = 2;

/// Rows the health part takes: the strip and the last failure.
const HEALTH_ROWS: u16 = 2;

/// Width of a chart label, so the latest values line up.
const LABEL_COLS: usize = 18;

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

/// The one-line summary pushed into the scrollback when the user leaves:
/// `42 runs · 1 failed (run 16)`.
pub(crate) fn summary(series: &SeriesHook) -> String {
    let c = copy(series);
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
        None => format!("{total} {runs} · none failed{scope}"),
        Some(last) => format!(
            "{total} {runs} · {} failed{scope} (run {last})",
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
        Unit::Price => format!("{v:.6}"),
        Unit::Percent => format!("{v:.3}%"),
        Unit::Quote => scope::display::format::format_large_number(v),
        Unit::Flag => if v != 0.0 { "pass" } else { "fail" }.to_string(),
        Unit::Usd => format!("${}", scope::display::format::format_large_number(v)),
        Unit::Count => format!("{v:.0}"),
        Unit::Native => format!("{v:.6}"),
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

/// Draws `view` in `area`. `status` goes on the last rows: the run's
/// progress while it runs, or the footer and the leave hint after.
pub(crate) fn draw(f: &mut Frame, area: Rect, view: &RepeatView, status: Vec<Line<'static>>) {
    let c = copy(&view.series);
    let mut status = status;
    if let Some(ended) = &view.ended {
        status.push(Line::styled(ended.text.clone(), ended.style));
        status.push(Line::styled(LEAVE_HINT, SECONDARY));
    }
    let status_rows = (status.len() as u16).min(area.height);
    let [body, foot] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(status_rows)]).areas(area);
    f.render_widget(Paragraph::new(Text::from(status)), foot);

    let (snap, trends) = if body.width >= SIDE_BY_SIDE_MIN_WIDTH {
        let [l, r] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .areas(body);
        (l, r)
    } else {
        let [t, b] =
            Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(body);
        (t, b)
    };
    draw_snapshot(f, snap, c.runs.last());
    draw_trends(f, trends, &c);
}

fn draw_snapshot(f: &mut Frame, area: Rect, last: Option<&RunRecord>) {
    let Some(last) = last else {
        return;
    };
    let text = last
        .snapshot
        .as_str()
        .into_text()
        .unwrap_or_else(|_| Text::raw(last.snapshot.clone()));
    let rows = fit::fit(text.lines, area.width as usize);
    f.render_widget(Paragraph::new(Text::from(rows)), area);
}

fn draw_trends(f: &mut Frame, area: Rect, c: &Copy) {
    let run = c.runs.last().map_or(0, |r| r.run);
    let block = Block::new()
        .borders(Borders::LEFT | Borders::TOP)
        .border_style(SECONDARY)
        .title(Line::styled(format!(" Trends · run {run} "), ACCENT));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < HEALTH_ROWS || inner.width <= LABEL_COLS as u16 {
        return;
    }
    let label = |text: &str| Span::styled(format!("{text:<LABEL_COLS$}"), SECONDARY);
    let chart_x = inner.x + LABEL_COLS as u16;
    let chart_w = inner.width - LABEL_COLS as u16;

    // Health: the strip, then the last failure under it.
    let healthy: Vec<bool> = c.runs.iter().map(|r| r.sample.healthy).collect();
    f.render_widget(
        Paragraph::new(Line::from(label("Health"))),
        Rect::new(inner.x, inner.y, LABEL_COLS as u16, 1),
    );
    f.render_widget(
        HealthStrip::new(&healthy),
        Rect::new(chart_x, inner.y, chart_w, 1),
    );
    let last_failure = c
        .runs
        .iter()
        .rev()
        .find(|r| !r.sample.healthy)
        .map(|r| {
            let why = r
                .sample
                .failed_checks
                .first()
                .map_or("unhealthy", String::as_str);
            format!("run {}: {why}", r.run)
        })
        .unwrap_or_else(|| "no failed runs".to_string());
    f.render_widget(
        Paragraph::new(Line::styled(last_failure, SECONDARY)),
        Rect::new(inner.x, inner.y + 1, inner.width, 1),
    );

    // One chart per key value, while two rows still fit: never overlap.
    let mut y = inner.y + HEALTH_ROWS;
    for (i, metric) in c.metrics.iter().enumerate().filter(|(_, m)| charted(m.id)) {
        if y + CHART_ROWS > inner.bottom() {
            break;
        }
        let values: Vec<Option<f64>> = c
            .runs
            .iter()
            .map(|r| r.sample.values.get(i).copied().flatten())
            .collect();
        let latest = values.last().copied().flatten();
        f.render_widget(
            Paragraph::new(Line::from(vec![
                label(metric.label),
                Span::raw(format_value(metric, latest)),
            ])),
            Rect::new(inner.x, y, inner.width, 1),
        );
        f.render_widget(
            Sparkline::new(&values, ACCENT).threshold(metric.threshold),
            Rect::new(chart_x, y + 1, chart_w, 1),
        );
        y += CHART_ROWS;
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

    /// A series of `n` runs; `failed` runs are unhealthy.
    fn series(n: u64, failed: &[u64]) -> SeriesHook {
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
    fn test_narrow_view_stacks_and_shows_fewer_charts_without_overlap() {
        let wide = render(&view(3, &[]), 120, 36).join("\n");
        let narrow_rows = render(&view(3, &[]), 79, 24);
        let narrow = narrow_rows.join("\n");
        let charts = |t: &str| t.matches("depth").count() + t.matches("Mid price").count();
        assert!(charts(&narrow) < charts(&wide), "{narrow}");
        // Stacked: the snapshot is on top, the trends below it.
        assert!(
            narrow_rows[0].starts_with("Market Summary run 3"),
            "{narrow}"
        );
        assert!(
            narrow_rows
                .iter()
                .position(|r| r.contains("Trends"))
                .unwrap()
                > 1
        );
        // Every label row has exactly one label: no chart drew over another.
        for row in narrow_rows.iter().filter(|r| r.contains("depth")) {
            assert_eq!(row.matches("depth").count(), 1, "{narrow}");
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
    }
}
