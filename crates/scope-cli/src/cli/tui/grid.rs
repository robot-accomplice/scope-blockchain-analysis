//! The watchlist grid (#47): the live view of a running watchlist.
//!
//! One row per target, breached rows first. A breach that starts rings the
//! bell once and shows a banner until the next key. `Enter` opens the
//! detail of the selected row: each cell with its value, its status and a
//! chart of its last values. Evidence keys put the matching CLI command in
//! the shell and run it there.
//!
//! The state is pure: the event loop feeds it [`GridMsg`]s from the run and
//! keys from the user; [`draw`] draws it.

use crate::cli::charts::{HealthStrip, Sparkline};
use crate::cli::output::ProgressHook;
use crate::cli::theme::{ACCENT, ERR, KEY, OK, SECONDARY, WARN};
use crate::cli::watch::{Ready, Tick};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
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
pub const ENDED_LEGEND: &str = "Esc or Ctrl-G back to the shell · Enter detail";

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
            cleared: Vec::new(),
            banner: None,
            bell: false,
            selected: 0,
            detail: false,
            ended: None,
            progress,
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

    /// The target of the selected row, from the watchlist.
    fn selected_target(&self) -> Option<&scope::watch::Target> {
        let label = self.rows().get(self.selected)?.target.clone();
        let list = self.ready.as_ref().map_or(&self.list, |r| &r.list);
        list.targets.iter().find(|t| t.label() == label)
    }

    /// The CLI command an evidence key runs for the selected target.
    pub fn evidence(&self, key: char) -> Option<String> {
        let t = self.selected_target()?;
        let venue = t
            .venue
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
            ('b', TargetKind::Token | TargetKind::Pair) if t.venue.is_some() => {
                Some(format!("market summary {id}{venue}"))
            }
            ('t', TargetKind::Token | TargetKind::Pair) if t.venue.is_some() => {
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
            KeyCode::Down if n > 0 => self.selected = (self.selected + 1) % n,
            KeyCode::Up if n > 0 => self.selected = (self.selected + n - 1) % n,
            KeyCode::Enter if n > 0 => self.detail = !self.detail,
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
        format!(
            "{state} {}: {} breached · Ctrl-G grid",
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
fn status_cell(s: &RuleStatus) -> (&'static str, Style) {
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
fn value_text(v: Option<f64>) -> String {
    match v {
        None => "-".to_string(),
        Some(v) if v.abs() >= 10_000.0 => scope::display::format::format_large_number(v),
        Some(v) => {
            let s = format!("{v:.4}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
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
    let banner_rows = u16::from(g.banner.is_some());
    let [banner, body, strip, foot, legend] = Layout::vertical([
        Constraint::Length(banner_rows),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    if let Some(text) = &g.banner {
        f.render_widget(
            Paragraph::new(Line::styled(
                format!(" ⚠ {text}"),
                ERR.add_modifier(Modifier::BOLD),
            )),
            banner,
        );
    }
    if g.detail {
        draw_detail(f, body, g);
    } else {
        draw_rows(f, body, g);
    }
    f.render_widget(HealthStrip::new(&g.health), strip);
    f.render_widget(
        Paragraph::new(Line::styled(format!(" {footer}"), footer_style)),
        foot,
    );
    let keys = match (g.detail, g.is_running()) {
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
            if let Some(why) = r
                .cells
                .iter()
                .find(|c| matches!(c.status, RuleStatus::Fail(_)))
                .or_else(|| {
                    r.cells
                        .iter()
                        .find(|c| matches!(c.status, RuleStatus::Error(_) | RuleStatus::Warn(_)))
                })
                .and_then(|c| c.status.message())
            {
                spans.push(Span::styled(format!("  {why}"), SECONDARY));
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
        if let Some(m) = c.status.message() {
            spans.push(Span::styled(m.to_string(), SECONDARY));
        }
        put(f, &mut y, Line::from(spans));
        let values: Vec<Option<f64>> = g
            .history
            .get(&(r.target.clone(), c.label.clone()))
            .map(|h| h.iter().copied().collect())
            .unwrap_or_default();
        if values.iter().any(Option::is_some) && y + CHART_ROWS <= bottom {
            let threshold = c.rule.map(|rule| match rule.op {
                scope::watch::Op::Lt | scope::watch::Op::Le => {
                    scope::metrics::Threshold::Max(rule.value)
                }
                scope::watch::Op::Gt | scope::watch::Op::Ge => {
                    scope::metrics::Threshold::Min(rule.value)
                }
                scope::watch::Op::Eq | scope::watch::Op::Ne => {
                    scope::metrics::Threshold::Target(rule.value)
                }
            });
            let chart = Rect::new(area.x + 3, y, area.width.saturating_sub(4), CHART_ROWS);
            f.render_widget(Sparkline::new(&values, ACCENT).threshold(threshold), chart);
            y += CHART_ROWS;
        }
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
        }
    }

    fn list() -> Watchlist {
        let mut w = Watchlist::new("stables");
        let mut a = Target::new(TargetKind::Pair, "USDC");
        a.venue = Some("binance".into());
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
                ),
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
    fn status_part_names_the_list_and_the_breaches() {
        let mut g = GridState::new(list(), None);
        tick(&mut g, 1, 3.4);
        assert_eq!(g.status_part(), "watch stables: 1 breached · Ctrl-G grid");
    }
}
