//! The watchlist grid (#47): the live view of a running watchlist.
//!
//! One row per target, breached rows first. A breach that starts rings the
//! bell once and shows a banner until the next key. `Enter` opens the
//! detail of the selected row: each cell with its value, its status and a
//! chart of its last values. For a target on two or more venues, the
//! detail is the venue charts (`venue_charts`): one chart per
//! market cell with a line per venue. Evidence keys put the matching CLI
//! command in the shell and run it there.
//!
//! The state is pure: the event loop feeds it [`GridMsg`]s from the run and
//! keys from the user; [`draw`] draws it.

use super::venue_charts::{self, Selection, Series, VenueChart, VenueCharts};
use crate::cli::charts::{HealthStrip, Sparkline, enlarge_hint};
use crate::cli::output::ProgressHook;
use crate::cli::theme::{ACCENT, ERR, KEY, OK, SECONDARY, WARN};
use crate::cli::watch::{Ready, Tick, deviation};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use scope::market::MarketSummary;
use scope::market::health::{DeviationGap, PegBands};
use scope::watch::{
    Event, RuleStatus, Snapshot, TargetKind, TargetSnapshot, TargetStatus, Watchlist,
};
use std::collections::{HashMap, VecDeque};

/// The most values a cell keeps for its chart.
const HISTORY: usize = 120;

/// Rows a cell's chart takes in the detail pane.
const CHART_ROWS: u16 = 1;

/// The width of the target column.
const TARGET_COLS: usize = 24;

/// The key legend of the grid and of the detail pane.
pub const GRID_LEGEND: &str = "↑↓ select · Enter detail · Ctrl-G shell · Esc stop the run";
pub const DETAIL_LEGEND: &str = "b book · t trades · h holders · r risk · x address · Esc back";
pub const VENUE_DETAIL_LEGEND: &str =
    "←→ venue · Space isolate · Tab chart · PgUp PgDn page · b book · t trades · Esc back";
pub const ENDED_LEGEND: &str = "Esc or Ctrl-G back to the shell · Enter detail";

/// Shown while a run uses the settings from before `setup` saved new ones.
pub const STALE_NOTE: &str = "The settings changed: this run still uses the old ones. Stop it (Esc) and start it again to use them.";

/// One market row's peg deviation, as the venue chart needs it.
#[derive(Debug, Default)]
struct Deviation {
    /// The deviation in bps of each tick; `None` is a gap, never zero.
    values: VecDeque<Option<f64>>,
    /// The peg bands of the newest summary: the thresholds of the verdicts.
    bands: Option<PegBands>,
    /// The newest deviation, for the legend glyph.
    last_bps: Option<f64>,
    /// Why the newest tick has no deviation.
    gap: Option<DeviationGap>,
}

/// What the run sends the grid.
#[derive(Debug, Clone)]
pub enum GridMsg {
    /// The targets are resolved; the first tick starts.
    Ready(Ready),
    /// One tick.
    Tick {
        run: u64,
        tick: Box<Tick>,
        events: Vec<Event>,
    },
}

/// What a key in the grid asks the event loop to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GridAction {
    None,
    /// Show the shell screen; the run goes on.
    Shell,
    /// Stop the run (it stays on screen with its footer).
    Stop,
    /// Leave the finished grid for the shell.
    Close,
    /// Run this CLI command in the shell.
    Run(String),
}

/// The grid's state.
#[derive(Debug)]
pub struct GridState {
    /// The watchlist as the user gave it.
    pub list: Watchlist,
    /// The watchlist as it runs, once resolved.
    pub ready: Option<Ready>,
    /// The last tick, once there is one.
    pub last: Option<Tick>,
    /// The run number of the last tick.
    pub run: u64,
    /// Each run's health, oldest first (true = no breach).
    pub health: Vec<bool>,
    /// The last values of each cell, by target and cell label.
    history: HashMap<(String, String), VecDeque<Option<f64>>>,
    /// The peg deviation of each market row, by row label.
    deviation: HashMap<String, Deviation>,
    /// Targets whose breach cleared on the last tick: drawn dim once.
    cleared: Vec<String>,
    /// The banner of the last breach that started, until the next key.
    pub banner: Option<String>,
    /// True once to ring the bell for a new breach; the loop takes it.
    pub bell: bool,
    /// The selected row, in sorted order.
    pub selected: usize,
    /// The detail pane of the selected row is open.
    pub detail: bool,
    /// The footer once the run ended (`Done`, `Stopped`, or the error).
    pub ended: Option<(String, Style)>,
    /// The run's progress (run N of M, next in Ns).
    pub progress: Option<ProgressHook>,
    /// `setup` saved new settings while the run went on: the run still
    /// uses the ones it started with.
    pub stale: bool,
    /// What the keys select in the venue charts.
    pub(crate) venues: Selection,
    /// Charts per page at the last draw, for PgUp and PgDn.
    page_size: std::cell::Cell<usize>,
}

impl GridState {
    /// A grid for `list`, before its first tick.
    pub fn new(list: Watchlist, progress: Option<ProgressHook>) -> Self {
        Self {
            list,
            ready: None,
            last: None,
            run: 0,
            health: Vec::new(),
            history: HashMap::new(),
            deviation: HashMap::new(),
            cleared: Vec::new(),
            banner: None,
            bell: false,
            selected: 0,
            detail: false,
            ended: None,
            progress,
            stale: false,
            venues: Selection::default(),
            page_size: std::cell::Cell::new(1),
        }
    }

    /// Takes one message from the run.
    pub fn apply(&mut self, msg: GridMsg) {
        match msg {
            GridMsg::Ready(ready) => self.ready = Some(ready),
            GridMsg::Tick { run, tick, events } => {
                self.run = run;
                self.health.push(tick.snapshot.breaches() == 0);
                for t in &tick.snapshot.targets {
                    for c in &t.cells {
                        let h = self
                            .history
                            .entry((t.target.clone(), c.label.clone()))
                            .or_default();
                        h.push_back(c.value);
                        if h.len() > HISTORY {
                            h.pop_front();
                        }
                    }
                }
                self.record_deviation(&tick);
                self.cleared.clear();
                for e in &events {
                    match e {
                        Event::BreachStarted { .. } => {
                            self.banner = Some(e.banner());
                            self.bell = true;
                        }
                        Event::BreachCleared { target, .. } => self.cleared.push(target.clone()),
                        _ => {}
                    }
                }
                self.last = Some(*tick);
            }
        }
    }

    /// Keeps the peg deviation of each market row of the tick. The tick's
    /// summaries line up with the rows of its snapshot by index
    /// (`watch/fetch.rs`). A failed read or a row with no mid is a gap.
    fn record_deviation(&mut self, tick: &Tick) {
        for (row, summary) in tick.snapshot.targets.iter().zip(&tick.summaries) {
            if row.venue.is_none() {
                continue;
            }
            let d = self.deviation.entry(row.target.clone()).or_default();
            let bps = summary
                .as_ref()
                .and_then(MarketSummary::mid_peg_deviation_bps);
            d.gap = match summary {
                Some(s) => s.peg_deviation_gap(),
                None => Some(DeviationGap::NoPrice),
            };
            if let Some(s) = summary {
                d.bands = Some(s.peg_bands());
            }
            d.last_bps = bps;
            d.values.push_back(bps);
            if d.values.len() > HISTORY {
                d.values.pop_front();
            }
        }
    }

    /// The run ended: `footer` says how.
    pub fn end(&mut self, footer: String, style: Style) {
        self.ended = Some((footer, style));
    }

    /// Whether the run still goes on.
    pub fn is_running(&self) -> bool {
        self.ended.is_none()
    }

    /// How many targets are breached now.
    pub fn breaches(&self) -> usize {
        self.last.as_ref().map_or(0, |t| t.snapshot.breaches())
    }

    /// The rows in grid order: breached, warned, unknown, passed; the
    /// watchlist order within each.
    pub fn rows(&self) -> Vec<&TargetSnapshot> {
        let mut rows: Vec<&TargetSnapshot> = self
            .last
            .as_ref()
            .map(|t| t.snapshot.targets.iter().collect())
            .unwrap_or_default();
        rows.sort_by_key(|r| r.status);
        rows
    }

    /// The target of the selected row, from the watchlist, and the row's
    /// venue.
    fn selected_target(&self) -> Option<(&scope::watch::Target, Option<String>)> {
        let row = self.rows().get(self.selected).copied()?;
        let list = self.ready.as_ref().map_or(&self.list, |r| &r.list);
        let t = list
            .targets
            .iter()
            .find(|t| t.rows().into_iter().any(|v| t.row_label(v) == row.target))?;
        Some((t, row.venue.clone()))
    }

    /// The venue charts of the selected row: when the detail is open on a
    /// market row of a target on two or more venues. One chart per cell of
    /// the row, each with every venue's history of that cell.
    pub(crate) fn venue_charts(&self) -> Option<VenueCharts> {
        let (row, t) = self.venue_row()?;
        let last = self.last.as_ref()?;
        let (deviation, gap) = self.deviation_chart(t);
        let cells = row
            .cells
            .iter()
            .map(|c| VenueChart {
                label: c.label.clone(),
                threshold: c.rule.map(rule_threshold),
                unit: c.rule.map(|r| {
                    scope::metrics::describe(r.metric, &scope::market::HealthThresholds::default())
                        .unit
                }),
                band: None,
                series: t
                    .venues
                    .iter()
                    .map(|v| {
                        let label = t.row_label(Some(v));
                        let values = self
                            .history
                            .get(&(label.clone(), c.label.clone()))
                            .map(|h| h.iter().copied().collect())
                            .unwrap_or_default();
                        let status = last
                            .snapshot
                            .targets
                            .iter()
                            .find(|r| r.target == label)
                            .and_then(|r| r.cells.iter().find(|x| x.label == c.label))
                            .map_or_else(
                                || RuleStatus::NotApplicable("no reading".to_string()),
                                |x| x.status.clone(),
                            );
                        Series {
                            venue: v.clone(),
                            values,
                            status,
                        }
                    })
                    .collect(),
            })
            .filter(VenueChart::has_values);
        // The deviation chart is first: the first page shows the peg.
        let charts = deviation.into_iter().chain(cells).collect::<Vec<_>>();
        (!charts.is_empty()).then_some(VenueCharts {
            charts,
            interval: self.list.interval,
            gap,
        })
    }

    /// The selected row and its target, when the detail is open on a market
    /// row of a target on two or more venues: the case of the venue charts.
    fn venue_row(&self) -> Option<(&TargetSnapshot, &scope::watch::Target)> {
        if !self.detail {
            return None;
        }
        let row = self.rows().get(self.selected).copied()?;
        row.venue.as_ref()?;
        let (t, _) = self.selected_target()?;
        (t.venues.len() >= 2).then_some((row, t))
    }

    /// The peg deviation chart of `t`: one line per venue, from the
    /// deviation each tick recorded. When no venue has a value, no chart
    /// and the reason (a peg that is not positive wins over a missing mid).
    fn deviation_chart(
        &self,
        t: &scope::watch::Target,
    ) -> (Option<VenueChart>, Option<DeviationGap>) {
        let devs: Vec<Option<&Deviation>> = t
            .venues
            .iter()
            .map(|v| self.deviation.get(&t.row_label(Some(v))))
            .collect();
        fn bands_of(d: Option<&Deviation>) -> Option<&PegBands> {
            d?.bands.as_ref()
        }
        let chart = VenueChart {
            label: venue_charts::DEVIATION_LABEL.to_string(),
            threshold: None,
            unit: None,
            band: devs
                .iter()
                .find_map(|d| bands_of(*d))
                .and_then(venue_charts::deviation_band),
            series: t
                .venues
                .iter()
                .zip(&devs)
                .map(|(v, d)| Series {
                    venue: venue_charts::deviation_venue(v),
                    values: d
                        .map(|d| d.values.iter().copied().collect())
                        .unwrap_or_default(),
                    status: deviation::status(
                        bands_of(*d),
                        d.and_then(|d| d.last_bps),
                        d.and_then(|d| d.gap),
                    ),
                })
                .collect(),
        };
        if chart.has_values() {
            return (Some(chart), None);
        }
        let no_peg = devs
            .iter()
            .flatten()
            .any(|d| d.gap == Some(DeviationGap::NoPeg));
        let gap = if no_peg {
            DeviationGap::NoPeg
        } else {
            DeviationGap::NoPrice
        };
        (None, Some(gap))
    }

    /// Why the deviation chart is missing from the open detail, for the
    /// footer when there are no venue charts (so no legend) to say it.
    fn deviation_note(&self) -> Option<String> {
        let (_, t) = self.venue_row()?;
        let gap = self.deviation_chart(t).1?;
        Some(venue_charts::gap_note(gap))
    }

    /// Starts the venue charts on the venue of the selected row.
    fn reset_venues(&mut self) {
        let venue = self.selected_target().and_then(|(t, v)| {
            let v = v?;
            t.venues.iter().position(|x| *x == v)
        });
        self.venues = Selection {
            venue: venue.unwrap_or(0),
            ..Selection::default()
        };
    }

    /// A key of the venue charts.
    fn venue_key(&mut self, code: KeyCode, charts: &VenueCharts) {
        self.venues.apply(
            code,
            charts.charts[0].series.len(),
            charts.charts.len(),
            self.page_size.get(),
        );
    }

    /// The CLI command an evidence key runs for the selected row; in the
    /// venue charts, for the venue the legend selects.
    pub fn evidence(&self, key: char) -> Option<String> {
        let (t, row_venue) = self.selected_target()?;
        let row_venue = match self.venue_charts() {
            Some(_) => t.venues.get(self.venues.venue).cloned(),
            None => row_venue,
        };
        let venue = row_venue
            .as_deref()
            .map(|v| format!(" --venue {v}"))
            .unwrap_or_default();
        let chain = t
            .chain
            .as_deref()
            .map(|c| format!(" --chain {c}"))
            .unwrap_or_default();
        let id = quote(&t.id);
        match (key, t.kind) {
            ('b', TargetKind::Token | TargetKind::Pair) if row_venue.is_some() => {
                Some(format!("market summary {id}{venue}"))
            }
            ('t', TargetKind::Token | TargetKind::Pair) if row_venue.is_some() => {
                Some(format!("market trades {id}{venue}"))
            }
            ('h', TargetKind::Token) => Some(format!("crawl {id}{chain}")),
            ('r', TargetKind::Address) => Some(format!("compliance risk {id}{chain}")),
            ('x', TargetKind::Address) => Some(format!("address {id}{chain} --include-txs")),
            _ => None,
        }
    }

    /// Applies one key.
    pub fn key(&mut self, key: KeyEvent) -> GridAction {
        // Any key dismisses the banner, and still does its job.
        self.banner = None;
        let n = self.rows().len();
        match key.code {
            KeyCode::Down if n > 0 => {
                self.selected = (self.selected + 1) % n;
                self.reset_venues();
            }
            KeyCode::Up if n > 0 => {
                self.selected = (self.selected + n - 1) % n;
                self.reset_venues();
            }
            KeyCode::Enter if n > 0 => {
                self.detail = !self.detail;
                self.reset_venues();
            }
            code if Selection::is_key(code) && self.detail => {
                if let Some(charts) = self.venue_charts() {
                    self.venue_key(key.code, &charts);
                }
            }
            KeyCode::Esc if self.detail => self.detail = false,
            KeyCode::Esc if self.is_running() => return GridAction::Stop,
            KeyCode::Esc => return GridAction::Close,
            KeyCode::Char(c) if self.detail => {
                if let Some(line) = self.evidence(c) {
                    return GridAction::Run(line);
                }
            }
            _ => {}
        }
        GridAction::None
    }

    /// The status-line part for the shell screen while a run goes on.
    pub fn status_part(&self) -> String {
        let state = if self.is_running() {
            "watch"
        } else {
            "watch ended"
        };
        let old = if self.stale && self.is_running() {
            " · old settings"
        } else {
            ""
        };
        format!(
            "{state} {}: {} breached{old} · Ctrl-G grid",
            self.list.name,
            self.breaches()
        )
    }

    /// The footer: progress while it runs, or how it ended.
    fn footer(&self) -> (String, Style) {
        if let Some(ended) = &self.ended {
            return ended.clone();
        }
        let Some(hook) = &self.progress else {
            return (format!("run {}", self.run), SECONDARY);
        };
        let p = hook.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let of = p.total.map(|t| format!(" of {t}")).unwrap_or_default();
        let next = p
            .next_at
            .map(|at| {
                let secs = at
                    .saturating_duration_since(std::time::Instant::now())
                    .as_secs();
                format!(" · next in {secs}s")
            })
            .unwrap_or_else(|| " · fetching".to_string());
        if p.run == 0 {
            return ("resolving the targets…".to_string(), SECONDARY);
        }
        (format!("run {}{of}{next}", p.run), SECONDARY)
    }
}

/// A word with a space or a quote, quoted for the shell line.
fn quote(s: &str) -> String {
    if s.contains([' ', '"', '\'']) {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// A status's glyph and style.
pub(super) fn status_cell(s: &RuleStatus) -> (&'static str, Style) {
    match s {
        RuleStatus::Pass => ("✓", OK),
        RuleStatus::Warn(_) => ("!", WARN),
        RuleStatus::Fail(_) => ("✗", ERR),
        RuleStatus::NotApplicable(_) => ("-", SECONDARY),
        RuleStatus::Error(_) => ("?", WARN),
    }
}

/// A target status's style.
fn target_style(s: TargetStatus) -> Style {
    match s {
        TargetStatus::Breached => ERR.add_modifier(Modifier::BOLD),
        TargetStatus::Warned => WARN,
        TargetStatus::Unknown => WARN,
        TargetStatus::Passed => OK,
    }
}

/// A value as the grid shows it.
pub(super) fn value_text(v: Option<f64>) -> String {
    match v {
        None => "-".to_string(),
        Some(v) if v.abs() >= 10_000.0 => scope::display::format::format_large_number(v),
        // The decimals of the value's size (`price_decimals`), so a tiny
        // price keeps its digits: PEPE at 0.0000106 read as `0` before.
        Some(v) => {
            let s = scope::display::format::format_price(v);
            if s.contains('.') {
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            } else {
                s
            }
        }
    }
}

/// Draws the grid in `area`.
pub fn draw(f: &mut Frame, area: Rect, g: &GridState) {
    let (footer, footer_style) = g.footer();
    let block = Block::new()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(ACCENT)
        .title(Line::styled(
            format!(" watchlist {} ", g.list.name),
            ACCENT.add_modifier(Modifier::BOLD),
        ))
        .title(
            Line::styled(
                format!(" {} breached ", g.breaches()),
                target_style(if g.breaches() > 0 {
                    TargetStatus::Breached
                } else {
                    TargetStatus::Passed
                }),
            )
            .right_aligned(),
        );
    let inner = block.inner(area);
    f.render_widget(block, area);
    let stale = g.stale && g.is_running();
    let banner_rows = u16::from(g.banner.is_some()) + u16::from(stale);
    let [banner, body, strip, foot, legend] = Layout::vertical([
        Constraint::Length(banner_rows),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    let mut banners = Vec::new();
    if let Some(text) = &g.banner {
        banners.push(Line::styled(
            format!(" ⚠ {text}"),
            ERR.add_modifier(Modifier::BOLD),
        ));
    }
    if stale {
        banners.push(Line::styled(format!(" {STALE_NOTE}"), WARN));
    }
    if !banners.is_empty() {
        f.render_widget(Paragraph::new(banners), banner);
    }
    if let Some(charts) = g.venue_charts() {
        match venue_charts::shortfall(body, &charts) {
            None => {
                let mut sel = g.venues;
                sel.venue = sel.venue.min(charts.charts[0].series.len() - 1);
                sel.chart = sel.chart.min(charts.charts.len() - 1);
                g.page_size.set(venue_charts::draw(f, body, &charts, sel));
            }
            // Too small for the charts: the one-venue detail, and how much
            // to enlarge the window.
            Some((cols, rows)) => {
                let text = format!(" {}", enlarge_hint(cols, rows, "the venue charts"));
                let hint_rows = text
                    .chars()
                    .count()
                    .div_ceil(usize::from(body.width.max(1)));
                let [detail, hint] =
                    Layout::vertical([Constraint::Min(0), Constraint::Length(hint_rows as u16)])
                        .areas(body);
                draw_detail(f, detail, g);
                f.render_widget(
                    Paragraph::new(Line::styled(text, WARN))
                        .wrap(ratatui::widgets::Wrap { trim: false }),
                    hint,
                );
            }
        }
    } else if g.detail {
        draw_detail(f, body, g);
    } else {
        draw_rows(f, body, g);
    }
    f.render_widget(HealthStrip::new(&g.health), strip);
    // With no venue charts there is no legend, so the footer says why the
    // deviation chart is missing.
    let note = g
        .deviation_note()
        .filter(|_| g.venue_charts().is_none())
        .map(|n| format!(" · {n}"))
        .unwrap_or_default();
    f.render_widget(
        Paragraph::new(Line::styled(format!(" {footer}{note}"), footer_style)),
        foot,
    );
    let keys = match (g.detail, g.is_running()) {
        (true, _) if g.venue_charts().is_some() => VENUE_DETAIL_LEGEND,
        (true, _) => DETAIL_LEGEND,
        (false, true) => GRID_LEGEND,
        (false, false) => ENDED_LEGEND,
    };
    f.render_widget(
        Paragraph::new(Line::styled(format!(" {keys}"), KEY)),
        legend,
    );
}

/// The rows: target, status, then one glyph per cell.
fn draw_rows(f: &mut Frame, area: Rect, g: &GridState) {
    let rows = g.rows();
    if rows.is_empty() {
        let text = if g.ready.is_none() && g.is_running() {
            " Resolving the targets…"
        } else {
            " Waiting for the first tick…"
        };
        f.render_widget(Paragraph::new(Line::styled(text, SECONDARY)), area);
        return;
    }
    let height = area.height as usize;
    let first = g.selected.saturating_sub(height.saturating_sub(1));
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, r)| {
            let marker = if i == g.selected { "> " } else { "  " };
            let mut style = target_style(r.status);
            if g.cleared.contains(&r.target) {
                style = style.add_modifier(Modifier::DIM);
            }
            let mut spans = vec![
                Span::styled(marker, KEY),
                Span::styled(
                    format!("{:<w$.w$} ", r.target, w = TARGET_COLS),
                    if i == g.selected {
                        style.add_modifier(Modifier::REVERSED)
                    } else {
                        style
                    },
                ),
                Span::styled(format!("{:<9}", r.status.name()), style),
            ];
            for c in &r.cells {
                let (glyph, st) = status_cell(&c.status);
                spans.push(Span::styled(glyph, st));
            }
            // The first problem, so the row says why without the detail.
            if let Some((why, st)) = r
                .cells
                .iter()
                .find(|c| matches!(c.status, RuleStatus::Fail(_)))
                .or_else(|| {
                    r.cells
                        .iter()
                        .find(|c| matches!(c.status, RuleStatus::Error(_) | RuleStatus::Warn(_)))
                })
                .and_then(|c| Some((c.status.message()?, status_cell(&c.status).1)))
            {
                spans.push(Span::styled(format!("  {why}"), st));
            }
            Line::from(spans)
        })
        .collect();
    f.render_widget(Paragraph::new(lines), area);
}

/// The detail of the selected row: one line per cell, and a chart of the
/// cell's last values when it has some.
fn draw_detail(f: &mut Frame, area: Rect, g: &GridState) {
    let rows = g.rows();
    let Some(r) = rows.get(g.selected) else {
        return;
    };
    let mut y = area.y;
    let bottom = area.bottom();
    let put = |f: &mut Frame, y: &mut u16, line: Line| {
        if *y < bottom {
            f.render_widget(Paragraph::new(line), Rect::new(area.x, *y, area.width, 1));
            *y += 1;
        }
    };
    put(
        f,
        &mut y,
        Line::styled(
            format!(" {} — {}", r.target, r.status.name()),
            target_style(r.status),
        ),
    );
    for c in &r.cells {
        let (glyph, st) = status_cell(&c.status);
        let mut spans = vec![
            Span::styled(format!(" {glyph} "), st),
            Span::styled(format!("{:<34.34} ", c.label), Style::new()),
            Span::styled(format!("{:>12} ", value_text(c.value)), ACCENT),
        ];
        // A passing check has no status message; its text keeps the numbers.
        if let Some(m) = c.status.message().or(c.detail.as_deref()) {
            // A warning or failure reads in its colour; only n/a is dimmed.
            let text = match c.status {
                RuleStatus::Warn(_) | RuleStatus::Fail(_) | RuleStatus::Error(_) => st,
                _ => SECONDARY,
            };
            spans.push(Span::styled(m.to_string(), text));
        }
        put(f, &mut y, Line::from(spans));
        let values: Vec<Option<f64>> = g
            .history
            .get(&(r.target.clone(), c.label.clone()))
            .map(|h| h.iter().copied().collect())
            .unwrap_or_default();
        if values.iter().any(Option::is_some) && y + CHART_ROWS <= bottom {
            let threshold = c.rule.map(rule_threshold);
            let chart = Rect::new(area.x + 3, y, area.width.saturating_sub(4), CHART_ROWS);
            f.render_widget(Sparkline::new(&values, ACCENT).threshold(threshold), chart);
            y += CHART_ROWS;
        }
    }
}

/// The line a rule draws on its chart: its limit.
fn rule_threshold(rule: scope::watch::Rule) -> scope::metrics::Threshold {
    use scope::metrics::Threshold;
    use scope::watch::Op;
    match rule.op {
        Op::Lt | Op::Le => Threshold::Max(rule.value),
        Op::Gt | Op::Ge => Threshold::Min(rule.value),
        Op::Eq | Op::Ne => Threshold::Target(rule.value),
    }
}

/// The scrollback summary when the grid closes.
pub fn summary(g: &GridState) -> String {
    let runs = g.health.len();
    let failed = g.health.iter().filter(|h| !**h).count();
    format!(
        "watchlist {}: {runs} run(s) · {failed} with a breach · {} target(s) breached at the end",
        g.list.name,
        g.breaches()
    )
}

/// The snapshot as plain text, for the scrollback copy when the grid closes.
pub fn snapshot_text(s: &Snapshot) -> String {
    let mut text = String::new();
    for t in &s.targets {
        text.push_str(&format!(
            "{:<w$} {}\n",
            t.target,
            t.status.name(),
            w = TARGET_COLS
        ));
        for c in &t.cells {
            let detail = c.status.message().unwrap_or("");
            text.push_str(&format!(
                "    {:<5} {:<34} {:>12} {}\n",
                c.status.name(),
                c.label,
                value_text(c.value),
                detail
            ));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn test_a_tiny_value_keeps_its_digits() {
        // The 0.8.0 smoke test: PEPE's price read as 0.
        assert_eq!(value_text(Some(0.0000106)), "0.0000106");
        assert_eq!(value_text(Some(1.5)), "1.5");
        assert_eq!(value_text(Some(42.0)), "42");
        assert_eq!(value_text(Some(25_000.0)), "25.00K");
        assert_eq!(value_text(None), "-");
    }
    use crate::cli::theme::series_style;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use scope::watch::{Cell, Rule, Target};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn cell(label: &str, status: RuleStatus, value: Option<f64>) -> Cell {
        Cell {
            label: label.into(),
            rule: Rule::parse(label).ok(),
            value,
            status,
            detail: None,
        }
    }

    fn list() -> Watchlist {
        let mut w = Watchlist::new("stables");
        let mut a = Target::new(TargetKind::Pair, "USDC");
        a.venues = vec!["binance".into()];
        let mut b = Target::new(TargetKind::Address, "0xabc");
        b.chain = Some("ethereum".into());
        let c = Target::new(TargetKind::Token, "PEPE");
        w.targets = vec![a, b, c];
        w
    }

    fn snapshot(spread: f64) -> Snapshot {
        let fail = spread > 1.0;
        Snapshot {
            targets: vec![
                TargetSnapshot::new(
                    "USDC@binance",
                    vec![cell(
                        "spread_pct < 1",
                        if fail {
                            RuleStatus::Fail(format!("spread_pct {spread}, needs < 1"))
                        } else {
                            RuleStatus::Pass
                        },
                        Some(spread),
                    )],
                )
                .on_venue(Some("binance")),
                TargetSnapshot::new(
                    "0xabc",
                    vec![cell("risk_score <= 5", RuleStatus::Pass, Some(2.0))],
                ),
                TargetSnapshot::new(
                    "PEPE",
                    vec![cell(
                        "holders > 1",
                        RuleStatus::Error("timeout".into()),
                        None,
                    )],
                ),
            ],
        }
    }

    fn tick(g: &mut GridState, run: u64, spread: f64) {
        let prev = g
            .last
            .as_ref()
            .map(|t| t.snapshot.clone())
            .unwrap_or_default();
        let s = snapshot(spread);
        let events = scope::watch::diff(&prev, &s);
        g.apply(GridMsg::Tick {
            run,
            tick: Box::new(Tick {
                snapshot: s,
                summaries: vec![None, None, None],
            }),
            events,
        });
    }

    fn screen(g: &GridState, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, f.area(), g)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut s = String::new();
        for y in 0..h {
            for x in 0..w {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn rows_sort_breached_then_unknown_then_passed() {
        let mut g = GridState::new(list(), None);
        tick(&mut g, 1, 3.4);
        let order: Vec<&str> = g.rows().iter().map(|r| r.target.as_str()).collect();
        assert_eq!(order, ["USDC@binance", "PEPE", "0xabc"]);
        assert_eq!(g.breaches(), 1);
    }

    #[test]
    fn a_new_breach_shows_a_banner_and_rings_once_and_a_key_clears_it() {
        let mut g = GridState::new(list(), None);
        tick(&mut g, 1, 0.5);
        assert!(g.banner.is_none() && !g.bell);
        tick(&mut g, 2, 3.4);
        assert_eq!(
            g.banner.as_deref(),
            Some("USDC@binance: spread_pct 3.4, needs < 1")
        );
        assert!(std::mem::take(&mut g.bell));
        tick(&mut g, 3, 3.5);
        assert!(!g.bell, "the same breach does not ring again");
        g.key(key(KeyCode::Down));
        assert!(g.banner.is_none());
    }

    #[test]
    fn a_cleared_row_is_dim_for_one_tick() {
        let mut g = GridState::new(list(), None);
        tick(&mut g, 1, 3.4);
        tick(&mut g, 2, 0.5);
        assert_eq!(g.cleared, vec!["USDC@binance".to_string()]);
        tick(&mut g, 3, 0.5);
        assert!(g.cleared.is_empty());
    }

    #[test]
    fn esc_stops_a_run_then_closes_and_backs_out_of_the_detail_first() {
        let mut g = GridState::new(list(), None);
        tick(&mut g, 1, 0.5);
        assert_eq!(g.key(key(KeyCode::Enter)), GridAction::None);
        assert!(g.detail);
        assert_eq!(g.key(key(KeyCode::Esc)), GridAction::None);
        assert!(!g.detail);
        assert_eq!(g.key(key(KeyCode::Esc)), GridAction::Stop);
        g.end("Stopped.".into(), SECONDARY);
        assert_eq!(g.key(key(KeyCode::Esc)), GridAction::Close);
    }

    // Design 2026-11-04, section 2.4: each venue row of one target runs
    // its own venue's evidence.
    #[test]
    fn evidence_keys_use_the_venue_of_the_selected_row() {
        let mut w = Watchlist::new("pegs");
        let mut t = Target::new(TargetKind::Pair, "USDN");
        t.venues = vec!["weex".into(), "binance".into()];
        t.preset = Some(scope::watch::Preset::MarketHealth);
        w.targets = vec![t];
        let mut g = GridState::new(w, None);
        let row = |venue: &str| {
            TargetSnapshot::new(
                format!("USDN@{venue}"),
                vec![cell("spread_pct < 1", RuleStatus::Pass, Some(0.1))],
            )
            .on_venue(Some(venue))
        };
        tick_rows(&mut g, 1, vec![row("weex"), row("binance")]);
        g.key(key(KeyCode::Down));
        g.key(key(KeyCode::Enter));
        assert_eq!(
            g.key(key(KeyCode::Char('b'))),
            GridAction::Run("market summary USDN --venue binance".into())
        );
    }

    #[test]
    fn evidence_keys_run_the_matching_command() {
        let mut g = GridState::new(list(), None);
        tick(&mut g, 1, 3.4);
        g.key(key(KeyCode::Enter));
        assert_eq!(
            g.key(key(KeyCode::Char('b'))),
            GridAction::Run("market summary USDC --venue binance".into())
        );
        assert_eq!(
            g.key(key(KeyCode::Char('t'))),
            GridAction::Run("market trades USDC --venue binance".into())
        );
        assert_eq!(
            g.key(key(KeyCode::Char('r'))),
            GridAction::None,
            "not for a pair"
        );
        // Sorted order: USDC, PEPE, 0xabc.
        g.key(key(KeyCode::Esc));
        g.key(key(KeyCode::Down));
        g.key(key(KeyCode::Enter));
        assert_eq!(
            g.key(key(KeyCode::Char('h'))),
            GridAction::Run("crawl PEPE".into())
        );
        g.key(key(KeyCode::Esc));
        g.key(key(KeyCode::Down));
        g.key(key(KeyCode::Enter));
        assert_eq!(
            g.key(key(KeyCode::Char('r'))),
            GridAction::Run("compliance risk 0xabc --chain ethereum".into())
        );
        assert_eq!(
            g.key(key(KeyCode::Char('x'))),
            GridAction::Run("address 0xabc --chain ethereum --include-txs".into())
        );
    }

    #[test]
    fn draws_rows_banner_detail_and_footer_at_several_sizes() {
        let mut g = GridState::new(list(), None);
        tick(&mut g, 1, 0.5);
        tick(&mut g, 2, 3.4);
        for (w, h) in [(120, 36), (80, 24), (40, 10)] {
            let s = screen(&g, w, h);
            assert!(s.contains("watchlist stables"), "{w}x{h}\n{s}");
            assert!(s.contains("USDC@binance"), "{w}x{h}\n{s}");
        }
        let s = screen(&g, 120, 36);
        assert!(
            s.contains("⚠ USDC@binance: spread_pct 3.4, needs < 1"),
            "{s}"
        );
        assert!(s.contains("breached"), "{s}");
        assert!(s.contains("Esc stop the run"), "{s}");
        g.key(key(KeyCode::Enter));
        let s = screen(&g, 120, 36);
        assert!(s.contains("spread_pct < 1"), "{s}");
        assert!(s.contains("b book"), "{s}");
        // The chart of the two values.
        assert!(s.contains('█') || s.contains('▁'), "{s}");
    }

    #[test]
    fn summary_and_snapshot_text() {
        let mut g = GridState::new(list(), None);
        tick(&mut g, 1, 3.4);
        tick(&mut g, 2, 0.5);
        assert_eq!(
            summary(&g),
            "watchlist stables: 2 run(s) · 1 with a breach · 0 target(s) breached at the end"
        );
        let text = snapshot_text(&g.last.as_ref().unwrap().snapshot);
        assert!(text.contains("USDC@binance"), "{text}");
        assert!(
            text.contains("error timeout") || text.contains("timeout"),
            "{text}"
        );
    }

    #[test]
    fn a_stale_run_says_to_restart_it_until_it_ends() {
        let mut g = GridState::new(list(), None);
        tick(&mut g, 1, 0.5);
        g.stale = true;
        assert!(screen(&g, 120, 36).contains("The settings changed"));
        assert!(g.status_part().contains("old settings"));
        g.end("Stopped.".into(), SECONDARY);
        assert!(!screen(&g, 120, 36).contains("The settings changed"));
    }

    #[test]
    fn status_part_names_the_list_and_the_breaches() {
        let mut g = GridState::new(list(), None);
        tick(&mut g, 1, 3.4);
        assert_eq!(g.status_part(), "watch stables: 1 breached · Ctrl-G grid");
    }

    /// One tick of hand-made rows, with no events.
    fn tick_rows(g: &mut GridState, run: u64, targets: Vec<TargetSnapshot>) {
        let summaries = targets.iter().map(|_| None).collect();
        g.apply(GridMsg::Tick {
            run,
            tick: Box::new(Tick {
                snapshot: Snapshot { targets },
                summaries,
            }),
            events: Vec::new(),
        });
    }

    /// The row of `s` that holds `text`.
    fn line_with<'a>(s: &'a str, text: &str) -> &'a str {
        s.lines().find(|l| l.contains(text)).unwrap_or("")
    }

    #[test]
    fn a_cell_keeps_its_last_values_only() {
        let mut g = GridState::new(list(), None);
        for run in 1..=HISTORY as u64 + 10 {
            tick(&mut g, run, run as f64 / 100.0);
        }
        let h = &g.history[&("USDC@binance".to_string(), "spread_pct < 1".to_string())];
        assert_eq!(h.len(), HISTORY);
        assert_eq!(h.front(), Some(&Some(0.11)), "the oldest ten are gone");
        assert_eq!(h.back(), Some(&Some(1.3)));
    }

    #[test]
    fn up_and_down_wrap_and_do_nothing_with_no_rows() {
        let mut g = GridState::new(list(), None);
        g.key(key(KeyCode::Up));
        assert_eq!(g.selected, 0, "no rows yet");
        assert_eq!(g.key(key(KeyCode::Enter)), GridAction::None);
        assert!(!g.detail, "no row to open");
        tick(&mut g, 1, 3.4);
        g.key(key(KeyCode::Up));
        assert_eq!(g.selected, 2, "up from the first row is the last");
        g.key(key(KeyCode::Up));
        assert_eq!(g.selected, 1);
        g.key(key(KeyCode::Down));
        g.key(key(KeyCode::Down));
        assert_eq!(g.selected, 0, "down from the last row is the first");
    }

    #[test]
    fn an_ended_run_says_so_in_the_status_part_and_drops_the_old_settings_note() {
        let mut g = GridState::new(list(), None);
        tick(&mut g, 1, 3.4);
        g.stale = true;
        g.end("Done.".into(), OK);
        assert!(!g.is_running());
        assert_eq!(
            g.status_part(),
            "watch ended stables: 1 breached · Ctrl-G grid"
        );
        let s = screen(&g, 120, 36);
        assert!(s.contains("Done."), "{s}");
        assert!(s.contains(ENDED_LEGEND), "{s}");
    }

    #[test]
    fn an_id_with_a_space_or_a_quote_is_quoted_in_the_command() {
        assert_eq!(quote("PEPE"), "PEPE");
        assert_eq!(quote("MY TOKEN"), "\"MY TOKEN\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
        let mut w = Watchlist::new("odd");
        let mut t = Target::new(TargetKind::Token, "MY TOKEN");
        t.chain = Some("solana".into());
        w.targets = vec![t];
        let label = w.targets[0].label();
        let mut g = GridState::new(w, None);
        tick_rows(
            &mut g,
            1,
            vec![TargetSnapshot::new(
                label,
                vec![cell("holders > 1", RuleStatus::Pass, Some(5.0))],
            )],
        );
        g.key(key(KeyCode::Enter));
        assert_eq!(
            g.key(key(KeyCode::Char('h'))),
            GridAction::Run("crawl \"MY TOKEN\" --chain solana".into())
        );
    }

    #[test]
    fn a_warned_row_comes_before_a_passed_one_and_shows_its_warning() {
        let mut g = GridState::new(list(), None);
        tick_rows(
            &mut g,
            1,
            vec![
                TargetSnapshot::new(
                    "0xabc",
                    vec![cell("risk_score <= 5", RuleStatus::Pass, Some(2.0))],
                ),
                TargetSnapshot::new(
                    "USDC@binance",
                    vec![
                        cell("spread_pct < 1", RuleStatus::Warn("thin book".into()), None),
                        cell(
                            "holders > 1",
                            RuleStatus::NotApplicable("not a token".into()),
                            None,
                        ),
                    ],
                ),
            ],
        );
        let order: Vec<&str> = g.rows().iter().map(|r| r.target.as_str()).collect();
        assert_eq!(order, ["USDC@binance", "0xabc"]);
        assert_eq!(g.breaches(), 0, "a warning is not a breach");
        let s = screen(&g, 120, 36);
        let row = line_with(&s, "USDC@binance");
        assert!(row.contains("warned"), "{row}");
        assert!(
            row.contains("!-  thin book"),
            "one glyph per cell, then why: {row}"
        );
        let row = line_with(&s, "0xabc");
        assert!(row.contains("passed") && row.contains('✓'), "{row}");
        // The detail: the warning reads in its colour, n/a dimmed.
        g.key(key(KeyCode::Enter));
        let mut t = Terminal::new(TestBackend::new(120, 36)).unwrap();
        t.draw(|f| draw(f, f.area(), &g)).unwrap();
        let buf = t.backend().buffer().clone();
        let style_of = |needle: &str| {
            (0..36u16)
                .find_map(|y| {
                    let row: String = (0..120u16).map(|x| buf[(x, y)].symbol()).collect();
                    let x = row.find(needle)?;
                    // Columns, not bytes: the glyphs before are multi-byte.
                    let col = row[..x].chars().count() as u16;
                    (row.matches(needle).count() == 1 && !row.contains("warned"))
                        .then(|| buf[(col, y)].style())
                })
                .unwrap_or_else(|| panic!("no {needle} in the detail"))
        };
        assert_eq!(style_of("thin book").fg, WARN.fg);
        assert!(style_of("not a token").add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn with_no_rows_it_says_what_it_waits_for() {
        let mut g = GridState::new(list(), None);
        assert!(screen(&g, 80, 24).contains("Resolving the targets…"));
        g.apply(GridMsg::Ready(Ready {
            list: list(),
            contracts: vec![None, None, None],
        }));
        assert!(screen(&g, 80, 24).contains("Waiting for the first tick…"));
        // A run that ended before it resolved waits for nothing.
        let mut g = GridState::new(list(), None);
        g.end("Stopped.".into(), SECONDARY);
        assert!(screen(&g, 80, 24).contains("Waiting for the first tick…"));
    }

    #[test]
    fn a_cleared_row_is_drawn_dim_for_one_tick() {
        let mut g = GridState::new(list(), None);
        tick(&mut g, 1, 3.4);
        tick(&mut g, 2, 0.5);
        let dim = |g: &GridState| {
            let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
            t.draw(|f| draw(f, f.area(), g)).unwrap();
            let buf = t.backend().buffer().clone();
            // The rows start under the title; the target name after `> `.
            (0..24).any(|y| {
                let row: String = (0..80).map(|x| buf[(x, y)].symbol()).collect();
                row.contains("USDC@binance")
                    && buf[(row.find("USDC").unwrap() as u16, y)]
                        .modifier
                        .contains(Modifier::DIM)
            })
        };
        assert!(dim(&g), "cleared on this tick");
        tick(&mut g, 3, 0.5);
        assert!(!dim(&g), "only once");
    }

    #[test]
    fn the_detail_draws_the_threshold_of_each_rule_on_its_chart() {
        let mut g = GridState::new(list(), None);
        for run in 1..=3 {
            let v = run as f64;
            tick_rows(
                &mut g,
                run,
                vec![TargetSnapshot::new(
                    "PEPE",
                    vec![
                        cell("holders > 10", RuleStatus::Pass, Some(20.0 + v)),
                        cell("holders == 22", RuleStatus::Pass, Some(20.0 + v)),
                        cell("holders != 99", RuleStatus::Pass, Some(20.0 + v)),
                        cell("risk_score <= 5", RuleStatus::Pass, Some(v)),
                    ],
                )],
            );
        }
        g.key(key(KeyCode::Enter));
        let s = screen(&g, 80, 24);
        let lines: Vec<&str> = s.lines().collect();
        for label in [
            "holders > 10",
            "holders == 22",
            "holders != 99",
            "risk_score <= 5",
        ] {
            let i = lines.iter().position(|l| l.contains(label)).unwrap();
            let chart = lines[i + 1];
            assert!(chart.contains('─'), "{label}: the threshold line\n{s}");
            assert!(
                chart.chars().any(|c| ('▁'..='█').contains(&c)),
                "{label}: the bars\n{s}"
            );
        }
    }

    #[test]
    fn a_value_that_is_not_a_number_reads_as_such() {
        assert_eq!(value_text(Some(f64::NAN)), "NaN");
    }

    // ---- Venue charts (design 2026-11-04, section 2.9) ----

    /// USDN on weex and binance, with `cells` market rules, after `ticks`
    /// ticks; `value(venue, cell, tick)` gives each value. The detail is
    /// open on the first row (weex: binance fails, so weex sorts after it
    /// unless both pass; every row passes here).
    fn on_two_venues(
        cells: &[&str],
        ticks: u64,
        value: impl Fn(usize, usize, u64) -> f64,
    ) -> GridState {
        let mut w = Watchlist::new("pegs");
        let mut t = Target::new(TargetKind::Pair, "USDN");
        t.venues = vec!["weex".into(), "binance".into()];
        t.rules = cells.iter().map(|c| Rule::parse(c).unwrap()).collect();
        w.targets = vec![t];
        let mut g = GridState::new(w, None);
        for run in 1..=ticks {
            let rows = ["weex", "binance"]
                .iter()
                .enumerate()
                .map(|(v, venue)| {
                    let cells = cells
                        .iter()
                        .enumerate()
                        .map(|(c, label)| cell(label, RuleStatus::Pass, Some(value(v, c, run))))
                        .collect();
                    TargetSnapshot::new(format!("USDN@{venue}"), cells).on_venue(Some(venue))
                })
                .collect();
            tick_rows(&mut g, run, rows);
        }
        g.key(key(KeyCode::Enter));
        g
    }

    fn peg(v: usize, _c: usize, run: u64) -> f64 {
        1.0 + (run % 5) as f64 * 0.0001 + v as f64 * 0.0003
    }

    fn is_braille(s: &str) -> bool {
        s.chars().any(|c| ('\u{2801}'..='\u{28FF}').contains(&c))
    }

    #[test]
    fn venue_charts_show_a_chart_per_cell_with_a_line_per_venue_and_one_legend() {
        let g = on_two_venues(&["mid_price > 0", "spread_pct < 1"], 30, peg);
        for (w, h) in [(120, 36), (80, 24)] {
            let s = screen(&g, w, h);
            assert!(s.contains(" mid_price > 0 "), "{w}x{h}\n{s}");
            assert!(s.contains(" spread_pct < 1 "), "{w}x{h}\n{s}");
            assert!(s.contains(" venues: mid_price > 0 "), "{w}x{h}\n{s}");
            assert!(is_braille(&s), "braille lines: {w}x{h}\n{s}");
            // The range of the last values across venues, then each venue.
            let all = line_with(&s, "  all  ");
            assert!(all.contains("1–1.0003"), "{all}");
            assert!(line_with(&s, "> ■ weex").contains('✓'), "{s}");
            assert!(line_with(&s, "  ■ binance").contains("1.0003"), "{s}");
            assert!(s.contains("now"), "the time axis: {s}");
            assert!(s.contains("←→ venue · Space isolate"), "{s}");
        }
    }

    #[test]
    fn the_selected_venue_is_drawn_on_top_where_lines_touch() {
        // The same values on both venues: every braille cell is shared.
        let mut g = on_two_venues(&["mid_price > 0"], 30, |_, _, run| 1.0 + run as f64);
        let colours = |g: &GridState| {
            let mut t = Terminal::new(TestBackend::new(120, 36)).unwrap();
            t.draw(|f| draw(f, f.area(), g)).unwrap();
            let buf = t.backend().buffer().clone();
            buf.content
                .iter()
                .filter(|c| is_braille(c.symbol()))
                .map(|c| c.fg)
                .collect::<std::collections::HashSet<_>>()
        };
        let fg = |i| {
            series_style(i)
                .fg
                .unwrap_or(crate::cli::theme::DEFAULT_COLOR)
        };
        assert_eq!(colours(&g), [fg(0)].into());
        g.key(key(KeyCode::Right));
        assert_eq!(colours(&g), [fg(1)].into());
    }

    #[test]
    fn keys_select_a_venue_isolate_it_and_change_the_legend_chart() {
        let mut g = on_two_venues(&["mid_price > 0", "spread_pct < 1"], 30, peg);
        g.key(key(KeyCode::Right));
        assert!(line_with(&screen(&g, 120, 36), "> ■ binance").contains("1.0003"));
        // The evidence keys follow the legend's venue.
        assert_eq!(
            g.key(key(KeyCode::Char('b'))),
            GridAction::Run("market summary USDN --venue binance".into())
        );
        g.key(key(KeyCode::Char(' ')));
        let s = screen(&g, 120, 36);
        assert!(!is_braille(&s), "one venue is bars: {s}");
        assert!(s.contains('█') || s.contains('▄'), "{s}");
        let mid = line_with(&s, " mid_price > 0 ");
        assert!(mid.contains("binance") && !mid.contains("weex"), "{mid}");
        g.key(key(KeyCode::Char(' ')));
        assert!(is_braille(&screen(&g, 120, 36)), "all venues again");
        g.key(key(KeyCode::Tab));
        assert!(screen(&g, 120, 36).contains(" venues: spread_pct < 1 "));
        g.key(key(KeyCode::Left));
        g.key(key(KeyCode::Left));
        assert!(
            line_with(&screen(&g, 120, 36), "> ■ ").contains("binance"),
            "wraps"
        );
    }

    #[test]
    fn charts_that_do_not_fit_go_on_pages() {
        let cells = [
            "mid_price > 0",
            "spread_pct < 1",
            "bid_depth > 0",
            "ask_depth > 0",
            "peg_deviation_pct < 1",
            "largest_trade < 50000",
        ];
        let mut g = on_two_venues(&cells, 10, peg);
        let s = screen(&g, 100, 20);
        let page = line_with(&s, " charts 1–");
        assert!(page.contains(" of 6 "), "{s}");
        g.key(key(KeyCode::PageDown));
        let s = screen(&g, 100, 20);
        assert!(!s.contains(" charts 1–"), "the next page: {s}");
        assert!(!s.contains(" mid_price > 0 "), "{s}");
        g.key(key(KeyCode::PageUp));
        assert!(screen(&g, 100, 20).contains(" charts 1–"));
    }

    #[test]
    fn a_window_too_small_for_the_charts_shows_the_detail_and_says_how_much_to_enlarge_it() {
        let g = on_two_venues(&["mid_price > 0"], 5, peg);
        let s = screen(&g, 50, 12);
        assert!(!is_braille(&s), "{s}");
        assert!(
            s.contains("USDN@weex — passed"),
            "the one-venue detail: {s}"
        );
        // A narrow window wraps the hint.
        let text = s.lines().map(str::trim_end).collect::<Vec<_>>().join(" ");
        assert!(text.contains("! Window too small: enlarge it"), "{s}");
        assert!(text.contains("wider to see the venue charts"), "{s}");
    }

    #[test]
    fn with_colour_off_every_venue_still_has_its_name_and_value() {
        let g = on_two_venues(&["mid_price > 0"], 30, peg);
        let mut t = Terminal::new(TestBackend::new(120, 36)).unwrap();
        t.draw(|f| {
            draw(f, f.area(), &g);
            crate::cli::plain::strip_color(f.buffer_mut());
        })
        .unwrap();
        let buf = t.backend().buffer().clone();
        assert!(
            buf.content
                .iter()
                .all(|c| c.fg == crate::cli::theme::DEFAULT_COLOR)
        );
        let s = screen(&g, 120, 36);
        let mid = line_with(&s, " mid_price > 0 ");
        assert!(
            mid.contains("weex 1.0003") || mid.contains("weex 1"),
            "{mid}"
        );
        assert!(mid.contains("binance"), "{mid}");
    }

    #[test]
    fn one_venue_keeps_the_detail_of_today() {
        let mut g = GridState::new(list(), None);
        tick(&mut g, 1, 0.5);
        g.key(key(KeyCode::Enter));
        assert!(g.venue_charts().is_none());
        assert!(screen(&g, 120, 36).contains(DETAIL_LEGEND));
    }

    // ---- The peg deviation chart (design 2026-11-11, section 3) ----

    /// A summary whose mid is `mid`, on a peg of `target` with `range`.
    fn summary_at(mid: f64, target: f64, range: f64) -> MarketSummary {
        use scope::market::{BookSource, HealthThresholds, OrderBook, OrderBookLevel};
        let level = |price| OrderBookLevel {
            price,
            quantity: 5_000.0,
        };
        let book = OrderBook {
            pair: "USDN/USDT".into(),
            source: BookSource::Exchange,
            bids: vec![level(mid - 0.00005)],
            asks: vec![level(mid + 0.00005)],
        };
        let thresholds = HealthThresholds {
            peg_target: target,
            peg_range: range,
            ..HealthThresholds::default()
        };
        MarketSummary::from_order_book(&book, &thresholds, None)
    }

    /// How a test describes one tick of the deviation scenarios: the mid of
    /// each venue (`None`: the read failed), the peg and its range.
    struct Peg {
        target: f64,
        range: f64,
    }

    const PEG: Peg = Peg {
        target: 1.0,
        range: 0.001,
    };

    /// `venues` on one target with the market rules `cells` (value 0.2
    /// each), after `ticks` ticks; `mid(venue, run)` gives each venue's mid.
    /// The detail is open on the first row.
    fn deviation_grid(
        venues: [&str; 2],
        cells: &[&str],
        ticks: u64,
        peg: &Peg,
        mid: impl Fn(usize, u64) -> Option<f64>,
    ) -> GridState {
        let mut w = Watchlist::new("pegs");
        let mut t = Target::new(TargetKind::Pair, "USDN");
        t.venues = venues.iter().map(|v| v.to_string()).collect();
        t.rules = cells.iter().map(|c| Rule::parse(c).unwrap()).collect();
        w.targets = vec![t];
        let mut g = GridState::new(w, None);
        for run in 1..=ticks {
            let rows: Vec<TargetSnapshot> = venues
                .iter()
                .map(|venue| {
                    let cells = cells
                        .iter()
                        .map(|label| cell(label, RuleStatus::Pass, Some(0.2)))
                        .collect();
                    TargetSnapshot::new(format!("USDN@{venue}"), cells).on_venue(Some(venue))
                })
                .collect();
            let summaries = (0..venues.len())
                .map(|v| mid(v, run).map(|m| summary_at(m, peg.target, peg.range)))
                .collect();
            g.apply(GridMsg::Tick {
                run,
                tick: Box::new(Tick {
                    snapshot: Snapshot { targets: rows },
                    summaries,
                }),
                events: Vec::new(),
            });
        }
        g.key(key(KeyCode::Enter));
        g
    }

    /// weex at +2 bps and binance at -3 bps, steady.
    fn steady(v: usize, _run: u64) -> Option<f64> {
        Some([1.0002, 0.9997][v])
    }

    fn buffer_of(g: &GridState, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, f.area(), g)).unwrap();
        t.backend().buffer().clone()
    }

    // Why: the chart answers "is each venue inside the band?" at a glance,
    // and the band is named in the legend so colour is not needed.
    #[test]
    fn the_deviation_chart_is_the_first_chart_with_its_band_and_signed_values() {
        let g = deviation_grid(["weex", "binance"], &["spread_pct < 1"], 20, &PEG, steady);
        let charts = g.venue_charts().unwrap();
        assert_eq!(charts.charts[0].label, "Peg deviation (bps)");
        assert_eq!(charts.charts[1].label, "spread_pct < 1", "Q-D2: first");
        for (w, h) in [(120, 36), (80, 24)] {
            let s = screen(&g, w, h);
            assert!(s.contains(" Peg deviation (bps) "), "{w}x{h}\n{s}");
            assert!(s.contains(" venues: Peg deviation (bps) "), "{w}x{h}\n{s}");
            assert!(line_with(&s, "band ±").contains("band ±10 bps"), "{s}");
            assert!(line_with(&s, "> ■ weex").contains("+2.0"), "{s}");
            assert!(line_with(&s, "> ■ weex").contains('✓'), "{s}");
            assert!(line_with(&s, "  ■ binance").contains("-3.0"), "{s}");
            assert!(s.contains('•'), "the band lines are dots: {w}x{h}\n{s}");
            assert!(is_braille(&s));
        }
        // The value box in the border shows when the pane is wide enough.
        let s = screen(&g, 200, 36);
        let title = line_with(&s, " Peg deviation (bps) ");
        assert!(
            title.contains("weex +2.0") && title.contains("binance -3.0"),
            "{title}"
        );
    }

    // Why: a missing conversion draws a line 100 times too small; the
    // helper must return basis points, not percent.
    #[test]
    fn a_mid_of_1_0012_on_a_peg_of_1_reads_plus_12_bps() {
        let g = deviation_grid(["weex", "binance"], &["spread_pct < 1"], 5, &PEG, |_, _| {
            Some(1.0012)
        });
        let s = screen(&g, 120, 36);
        assert!(line_with(&s, "> ■ weex").contains("+12.0"), "{s}");
        assert!(!s.contains("0.12"), "not percent: {s}");
        // Past the 10 bps band, inside the 50 bps one: the warning glyph.
        assert!(line_with(&s, "> ■ weex").contains('!'), "{s}");
    }

    // Why: the series must line up with the rows by index. A shifted index
    // puts one venue's line on another venue. A failed venue is a gap.
    #[test]
    fn each_row_keeps_its_own_deviation_and_a_failed_read_is_a_gap() {
        let g = deviation_grid(
            ["weex", "binance"],
            &["spread_pct < 1"],
            3,
            &PEG,
            |v, run| match (v, run) {
                (1, 2) => None,
                (0, _) => Some(1.0002),
                _ => Some(0.9990 + run as f64 * 0.0001),
            },
        );
        let bps = |label: &str| -> Vec<Option<f64>> {
            g.deviation[label]
                .values
                .iter()
                .map(|v| v.map(|v| (v * 10.0).round() / 10.0))
                .collect()
        };
        assert_eq!(bps("USDN@weex"), [Some(2.0), Some(2.0), Some(2.0)]);
        assert_eq!(bps("USDN@binance"), [Some(-9.0), None, Some(-7.0)]);
        assert_eq!(
            g.deviation["USDN@binance"].gap, None,
            "the newest tick read"
        );
    }

    // Why: the band must follow the thresholds the verdicts use. A
    // hard-coded 10 bps fails this.
    #[test]
    fn the_band_follows_the_peg_range_of_the_summaries() {
        let peg = Peg {
            target: 1.0,
            range: 0.002,
        };
        let g = deviation_grid(["weex", "binance"], &["spread_pct < 1"], 5, &peg, steady);
        let s = screen(&g, 120, 36);
        assert!(s.contains("band ±20 bps"), "{s}");
        assert!(!s.contains("band ±10 bps"), "{s}");
    }

    // Why: the chart exists to show "inside or outside". Data inside the
    // band still gives the band's scale. Data at 80 bps widens the scale and
    // keeps the band lines.
    #[test]
    fn the_scale_always_includes_the_band() {
        let near = deviation_grid(["weex", "binance"], &["spread_pct < 1"], 5, &PEG, steady);
        let s = screen(&near, 120, 36);
        let title = line_with(&s, " Peg deviation (bps) ");
        assert!(!title.is_empty());
        assert!(s.contains("+10.0") && s.contains("-10.0"), "{s}");
        let far = deviation_grid(["weex", "binance"], &["spread_pct < 1"], 5, &PEG, |v, _| {
            Some([1.008, 0.9997][v])
        });
        let s = screen(&far, 120, 36);
        assert!(s.contains("+80.0"), "the data widens the scale: {s}");
        assert!(s.contains('•'), "the band lines stay: {s}");
    }

    // Why: the owner's rule: the lines use the venue colours, never a
    // health colour. The band dots are the secondary tone, and they are dots
    // as well, so they differ from the lines by glyph and not by colour only.
    #[test]
    fn the_lines_use_venue_colours_and_the_band_uses_the_secondary_tone() {
        let g = deviation_grid(["weex", "binance"], &["spread_pct < 1"], 20, &PEG, steady);
        let buf = buffer_of(&g, 120, 36);
        let fg = |style: Style| style.fg.unwrap_or(crate::cli::theme::DEFAULT_COLOR);
        let venue = [fg(series_style(0)), fg(series_style(1))];
        // The first chart is the deviation chart: its left half of the plot.
        let mut lines = std::collections::HashSet::new();
        let mut dots = std::collections::HashSet::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width / 3 {
                let c = &buf[(x, y)];
                if is_braille(c.symbol()) {
                    lines.insert(c.fg);
                } else if c.symbol() == "•" {
                    dots.insert(c.fg);
                }
            }
        }
        assert!(
            !lines.is_empty() && lines.iter().all(|c| venue.contains(c)),
            "{lines:?}"
        );
        assert_eq!(dots, [fg(SECONDARY)].into(), "{dots:?}");
        for health in [OK, WARN, ERR] {
            assert!(!lines.contains(&fg(health)), "no health colour in a line");
        }
    }

    // Why: with colour off the chart must still say everything in text.
    #[test]
    fn with_colour_off_the_deviation_chart_still_holds_names_values_and_band() {
        let g = deviation_grid(["weex", "binance"], &["spread_pct < 1"], 20, &PEG, steady);
        let mut t = Terminal::new(TestBackend::new(120, 36)).unwrap();
        t.draw(|f| {
            draw(f, f.area(), &g);
            crate::cli::plain::strip_color(f.buffer_mut());
        })
        .unwrap();
        assert!(
            t.backend()
                .buffer()
                .content
                .iter()
                .all(|c| c.fg == crate::cli::theme::DEFAULT_COLOR)
        );
        let s = screen(&g, 200, 36);
        assert!(line_with(&s, " Peg deviation (bps) ").contains("weex +2.0"));
        assert!(s.contains("band ±10 bps") && s.contains('•'));
    }

    // Why: at the lowest pane the plot has too few rows. Band lines would
    // sit on the data and hide it. The legend still names the band.
    #[test]
    fn at_the_lowest_pane_no_band_line_is_drawn_and_the_legend_names_the_band() {
        let g = deviation_grid(["weex", "binance"], &["spread_pct < 1"], 20, &PEG, steady);
        // 72 columns: one chart beside the legend. 10 rows: a 5-row pane.
        let s = screen(&g, 72, 10);
        assert!(s.contains(" Peg deviation (bps) "), "{s}");
        assert!(!s.contains('•'), "no band line in a low plot: {s}");
        assert!(s.contains("band ±10 bps"), "{s}");
        // Taller, the lines return.
        assert!(screen(&g, 72, 14).contains('•'));
        // Below the pane, the hint says how much to enlarge the window.
        let s = screen(&g, 50, 8);
        let text = s.lines().map(str::trim_end).collect::<Vec<_>>().join(" ");
        assert!(text.contains("Window too small: enlarge it"), "{s}");
    }

    // Q-D4 b. Why: bars hide the sign and cannot carry two band lines.
    #[test]
    fn isolating_a_venue_keeps_a_braille_line_with_the_band() {
        let mut g = deviation_grid(["weex", "binance"], &["spread_pct < 1"], 20, &PEG, steady);
        g.key(key(KeyCode::Char(' ')));
        let s = screen(&g, 200, 36);
        let first = &s.lines().map(str::to_string).collect::<Vec<_>>();
        let title = line_with(&s, " Peg deviation (bps) ");
        assert!(
            title.contains("weex +2.0") && !title.contains("binance"),
            "{title}"
        );
        assert!(is_braille(&s) && s.contains('•'), "{s}");
        assert!(
            !first.iter().any(|l| l.contains('█')),
            "no bars on this chart: {s}"
        );
    }

    // Q-D6 a. Why: a DEX mid is in USD and a CEX mid is in the quote. The
    // reader must know why two lines differ.
    #[test]
    fn a_dex_venue_is_tagged_usd_in_the_legend_and_the_value_box() {
        let g = deviation_grid(["eth", "binance"], &["spread_pct < 1"], 20, &PEG, steady);
        let s = screen(&g, 200, 36);
        assert!(line_with(&s, "> ■ eth usd").contains("+2.0"), "{s}");
        assert!(
            line_with(&s, " Peg deviation (bps) ").contains("eth usd +2.0"),
            "{s}"
        );
        assert!(!s.contains("binance usd"), "a CEX venue has no tag: {s}");
        // The tag is only a name: the evidence key still reads the venue.
        assert_eq!(
            g.evidence('b'),
            Some("market summary USDN --venue eth".to_string())
        );
    }

    // Why: a chart that is missing silently cannot be diagnosed. The
    // legend (or, with no legend, the footer) says why.
    #[test]
    fn a_missing_deviation_chart_says_why() {
        let peg = Peg {
            target: 0.0,
            range: 0.001,
        };
        let g = deviation_grid(["weex", "binance"], &["spread_pct < 1"], 5, &peg, steady);
        let charts = g.venue_charts().unwrap();
        assert_eq!(charts.gap, Some(DeviationGap::NoPeg));
        assert_eq!(
            charts.charts[0].label, "spread_pct < 1",
            "no deviation chart"
        );
        let text = screen(&g, 120, 36)
            .lines()
            .map(str::trim)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("peg deviation: peg"), "{text}");
        // No venue has a mid: the reason is the missing price.
        let g = deviation_grid(["weex", "binance"], &["spread_pct < 1"], 5, &PEG, |_, _| {
            None
        });
        assert_eq!(g.venue_charts().unwrap().gap, Some(DeviationGap::NoPrice));
        // With no chart at all there is no legend: the footer says it.
        let mut g = deviation_grid(["weex", "binance"], &["spread_pct < 1"], 0, &PEG, steady);
        for run in 1..=3 {
            let rows = ["weex", "binance"]
                .iter()
                .map(|v| {
                    TargetSnapshot::new(
                        format!("USDN@{v}"),
                        vec![cell("spread_pct < 1", RuleStatus::Pass, None)],
                    )
                    .on_venue(Some(v))
                })
                .collect();
            tick_rows(&mut g, run, rows);
        }
        g.key(key(KeyCode::Enter));
        assert!(g.venue_charts().is_none());
        let s = screen(&g, 120, 36);
        assert!(s.contains("peg deviation: no mid price"), "{s}");
    }

    // Q-D5 a. Why: a rule `peg_deviation_pct` is the user's own chart with
    // its own limit. The new chart does not replace it.
    #[test]
    fn a_peg_deviation_rule_gives_its_own_chart_beside_the_bps_chart() {
        let g = deviation_grid(
            ["weex", "binance"],
            &["peg_deviation_pct < 0.1"],
            10,
            &PEG,
            steady,
        );
        let labels: Vec<String> = g
            .venue_charts()
            .unwrap()
            .charts
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(labels, ["Peg deviation (bps)", "peg_deviation_pct < 0.1"]);
    }

    // Why: one venue keeps the detail of today. The venue charts, and so
    // the deviation chart, exist for two or more venues.
    #[test]
    fn a_target_on_one_venue_has_no_deviation_chart() {
        let mut g = GridState::new(list(), None);
        tick(&mut g, 1, 0.5);
        g.key(key(KeyCode::Enter));
        assert!(g.venue_charts().is_none());
        assert!(
            g.deviation_note().is_none(),
            "no note for a one-venue target"
        );
    }
}
