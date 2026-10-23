//! The monitor wizard (#47): builds or edits a watchlist in four steps.
//!
//! 1. **Targets:** `a` adds one (`pair USDC binance`, `token PEPE solana`,
//!    `address 0x… ethereum`), `d` deletes the selected one.
//! 2. **Metrics:** for the selected target, the market health preset and
//!    the catalog metrics of its kind. Space turns one on or off, `o`
//!    changes the operator, Enter edits the value.
//! 3. **Timing:** the interval and the duration, with the number of
//!    fetches they cost.
//! 4. **Outputs:** the name, the CSV and the report paths; Save and Start.
//!
//! Tab and Shift-Tab move between the steps; Ctrl-S saves and Ctrl-R
//! starts from any step. The state is pure; [`draw`] draws it.

use crate::cli::theme::{ACCENT, ERR, KEY, OK, SECONDARY, SELECTED, WARN};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use scope::market::HealthThresholds;
use scope::metrics::{MetricGroup, MetricId, Threshold, describe};
use scope::watch::time::{format_duration, is_until_cancelled, parse_duration};
use scope::watch::{Op, Preset, Rule, Target, TargetKind, Watchlist};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The wizard's steps, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Targets,
    Metrics,
    Timing,
    Outputs,
}

impl Step {
    const ALL: [Step; 4] = [Step::Targets, Step::Metrics, Step::Timing, Step::Outputs];

    fn name(self) -> &'static str {
        match self {
            Step::Targets => "Targets",
            Step::Metrics => "Metrics",
            Step::Timing => "Timing",
            Step::Outputs => "Outputs",
        }
    }

    fn index(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

/// The line being typed, and what it is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditKind {
    AddTarget,
    /// The value of the rule on this metric of the selected target.
    Value(MetricId),
    Interval,
    Duration,
    Name,
    Csv,
    Report,
}

/// What a wizard key asks the TUI to do.
#[derive(Debug, Clone, PartialEq)]
pub enum WizardAction {
    None,
    /// Close the wizard.
    Leave,
    /// Start this watchlist in the grid.
    Start(Box<Watchlist>),
    /// The watchlist was saved to this file.
    Saved(PathBuf),
}

/// The rows of the Outputs step.
const OUTPUT_ROWS: [&str; 5] = ["Name", "CSV", "Report", "Save", "Start"];

/// The key legend of each step.
fn legend(step: Step) -> &'static str {
    match step {
        Step::Targets => "a add · d delete · ↑↓ select · Tab next step · Ctrl-R start · Esc leave",
        Step::Metrics => {
            "Space on/off · o operator · Enter value · ←→ target · Tab next · Ctrl-R start"
        }
        Step::Timing | Step::Outputs => {
            "↑↓ select · Enter edit · Ctrl-S save · Ctrl-R start · Tab next · Esc leave"
        }
    }
}

/// The wizard's state.
#[derive(Debug)]
pub struct WizardState {
    pub list: Watchlist,
    /// Notes from the arguments (flags that do nothing, dropped alerts).
    pub notes: Vec<String>,
    pub step: Step,
    /// The selected target (Targets and Metrics).
    pub target: usize,
    /// The selected row of the Metrics, Timing or Outputs step.
    pub row: usize,
    /// The line being typed.
    pub edit: Option<(EditKind, String)>,
    /// The venue IDs a target may name.
    pub venues: Vec<String>,
    /// Where Save writes.
    pub dir: PathBuf,
    /// Something changed since the last save.
    pub dirty: bool,
    /// Esc asked "Leave without saving?".
    pub confirm_leave: bool,
    /// The last problem, until the next key.
    pub error: Option<String>,
}

impl WizardState {
    /// A wizard over `list`. `dir` holds the saved watchlists.
    pub fn new(list: Watchlist, notes: Vec<String>, venues: Vec<String>, dir: PathBuf) -> Self {
        // A list from arguments already has its targets: show their metrics.
        let step = if list.targets.is_empty() {
            Step::Targets
        } else {
            Step::Metrics
        };
        Self {
            list,
            notes,
            step,
            target: 0,
            row: 0,
            edit: None,
            venues,
            dir,
            dirty: false,
            confirm_leave: false,
            error: None,
        }
    }

    /// Applies one key.
    pub fn key(&mut self, key: KeyEvent) -> WizardAction {
        self.error = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.confirm_leave {
            self.confirm_leave = false;
            return match key.code {
                KeyCode::Char('y' | 'Y') => WizardAction::Leave,
                _ => WizardAction::None,
            };
        }
        if let Some((kind, mut text)) = self.edit.take() {
            match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => self.finish_edit(kind, text),
                KeyCode::Backspace => {
                    text.pop();
                    self.edit = Some((kind, text));
                }
                KeyCode::Char(c) if !ctrl => {
                    text.push(c);
                    self.edit = Some((kind, text));
                }
                _ => self.edit = Some((kind, text)),
            }
            return WizardAction::None;
        }
        match key.code {
            KeyCode::Char('s') if ctrl => return self.save(),
            KeyCode::Char('r') if ctrl => return self.start(),
            KeyCode::Esc => {
                if self.dirty {
                    self.confirm_leave = true;
                    return WizardAction::None;
                }
                return WizardAction::Leave;
            }
            KeyCode::Tab => self.go(self.step.index() + 1),
            KeyCode::BackTab => self.go(self.step.index() + Step::ALL.len() - 1),
            _ => {
                return match self.step {
                    Step::Targets => self.targets_key(key),
                    Step::Metrics => self.metrics_key(key),
                    Step::Timing => self.timing_key(key),
                    Step::Outputs => self.outputs_key(key),
                };
            }
        }
        WizardAction::None
    }

    fn go(&mut self, index: usize) {
        self.step = Step::ALL[index % Step::ALL.len()];
        self.row = 0;
    }

    fn targets_key(&mut self, key: KeyEvent) -> WizardAction {
        let n = self.list.targets.len();
        match key.code {
            KeyCode::Char('a') => self.edit = Some((EditKind::AddTarget, String::new())),
            KeyCode::Char('d') if n > 0 => {
                self.list.targets.remove(self.target.min(n - 1));
                self.target = self.target.min(n.saturating_sub(2));
                self.dirty = true;
            }
            KeyCode::Down if n > 0 => self.target = (self.target + 1) % n,
            KeyCode::Up if n > 0 => self.target = (self.target + n - 1) % n,
            KeyCode::Enter if n > 0 => self.go(Step::Metrics.index()),
            _ => {}
        }
        WizardAction::None
    }

    /// The Metrics rows of the selected target: the preset (when the kind
    /// takes market metrics), then the kind's metrics.
    pub fn metric_rows(&self) -> Vec<Option<MetricId>> {
        let Some(t) = self.list.targets.get(self.target) else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        if t.kind.accepts(MetricGroup::Market) {
            rows.push(None);
        }
        rows.extend(
            t.kind
                .metrics()
                .into_iter()
                .filter(|m| *m != MetricId::Healthy)
                .map(Some),
        );
        rows
    }

    fn metrics_key(&mut self, key: KeyEvent) -> WizardAction {
        let rows = self.metric_rows();
        let n = rows.len();
        let targets = self.list.targets.len();
        match key.code {
            KeyCode::Down if n > 0 => self.row = (self.row + 1) % n,
            KeyCode::Up if n > 0 => self.row = (self.row + n - 1) % n,
            KeyCode::Right if targets > 0 => {
                self.target = (self.target + 1) % targets;
                self.row = 0;
            }
            KeyCode::Left if targets > 0 => {
                self.target = (self.target + targets - 1) % targets;
                self.row = 0;
            }
            KeyCode::Char(' ') if n > 0 => self.toggle(rows[self.row.min(n - 1)]),
            KeyCode::Char('o') if n > 0 => {
                if let Some(m) = rows[self.row.min(n - 1)]
                    && let Some(rule) = self.rule_mut(m)
                {
                    rule.op = next_op(rule.op);
                    self.dirty = true;
                }
            }
            KeyCode::Enter if n > 0 => {
                if let Some(m) = rows[self.row.min(n - 1)]
                    && let Some(rule) = self.rule_mut(m)
                {
                    self.edit = Some((EditKind::Value(m), rule.value.to_string()));
                }
            }
            _ => {}
        }
        WizardAction::None
    }

    fn rule_mut(&mut self, metric: MetricId) -> Option<&mut Rule> {
        self.list
            .targets
            .get_mut(self.target)?
            .rules
            .iter_mut()
            .find(|r| r.metric == metric)
    }

    /// Turns the preset or a metric's rule on or off.
    fn toggle(&mut self, row: Option<MetricId>) {
        let Some(t) = self.list.targets.get_mut(self.target) else {
            return;
        };
        let needs_venue = row.is_none_or(|m| m.group() == MetricGroup::Market);
        match row {
            None if t.preset.is_some() => t.preset = None,
            Some(m) if t.rules.iter().any(|r| r.metric == m) => t.rules.retain(|r| r.metric != m),
            _ if needs_venue && t.venue.is_none() => {
                self.error = Some(format!(
                    "{} has no venue: market metrics need one (add the target as `pair {} binance`).",
                    t.id, t.id
                ));
                return;
            }
            None => t.preset = Some(Preset::MarketHealth),
            Some(m) => t.rules.push(default_rule(m)),
        }
        self.dirty = true;
    }

    fn timing_key(&mut self, key: KeyEvent) -> WizardAction {
        match key.code {
            KeyCode::Down | KeyCode::Up => self.row = 1 - self.row.min(1),
            KeyCode::Enter => {
                let (kind, text) = if self.row == 0 {
                    (
                        EditKind::Interval,
                        format_duration(self.list.interval.as_secs()),
                    )
                } else {
                    (
                        EditKind::Duration,
                        self.list.duration.map_or_else(
                            || "forever".to_string(),
                            |d| format_duration(d.as_secs()),
                        ),
                    )
                };
                self.edit = Some((kind, text));
            }
            _ => {}
        }
        WizardAction::None
    }

    fn outputs_key(&mut self, key: KeyEvent) -> WizardAction {
        let n = OUTPUT_ROWS.len();
        match key.code {
            KeyCode::Down => self.row = (self.row + 1) % n,
            KeyCode::Up => self.row = (self.row + n - 1) % n,
            KeyCode::Enter => match self.row {
                0 => self.edit = Some((EditKind::Name, self.list.name.clone())),
                1 => self.edit = Some((EditKind::Csv, path_text(&self.list.outputs.csv))),
                2 => self.edit = Some((EditKind::Report, path_text(&self.list.outputs.report))),
                3 => return self.save(),
                _ => return self.start(),
            },
            _ => {}
        }
        WizardAction::None
    }

    /// Applies a typed line.
    fn finish_edit(&mut self, kind: EditKind, text: String) {
        let text = text.trim().to_string();
        let result = match kind {
            EditKind::AddTarget => self.add_target(&text),
            EditKind::Value(m) => match text.replace('_', "").parse::<f64>() {
                Ok(v) if v.is_finite() => {
                    if let Some(rule) = self.rule_mut(m) {
                        rule.value = v;
                    }
                    Ok(())
                }
                _ => Err(format!("`{text}` is not a number")),
            },
            EditKind::Interval => parse_duration(&text).map(|s| {
                self.list.interval = Duration::from_secs(s);
            }),
            EditKind::Duration => {
                if is_until_cancelled(&text) {
                    self.list.duration = None;
                    Ok(())
                } else {
                    parse_duration(&text).map(|s| self.list.duration = Some(Duration::from_secs(s)))
                }
            }
            EditKind::Name => {
                if text.is_empty() || text.contains(['/', '\\', ':']) {
                    Err("a name has no / \\ or : and is not empty".to_string())
                } else {
                    self.list.name = text;
                    Ok(())
                }
            }
            EditKind::Csv => {
                self.list.outputs.csv = (!text.is_empty()).then(|| PathBuf::from(&text));
                Ok(())
            }
            EditKind::Report => {
                self.list.outputs.report = (!text.is_empty()).then(|| PathBuf::from(&text));
                Ok(())
            }
        };
        match result {
            Ok(()) => self.dirty = true,
            Err(e) => self.error = Some(e),
        }
    }

    /// Adds `kind id [venue|chain]`.
    fn add_target(&mut self, text: &str) -> Result<(), String> {
        const USAGE: &str =
            "type `pair USDC binance`, `token PEPE [chain|venue]` or `address 0x… [chain]`";
        let words: Vec<&str> = text.split_whitespace().collect();
        let (kind, id, extra) = match words.as_slice() {
            [kind, id] => (*kind, *id, None),
            [kind, id, extra] => (*kind, *id, Some(*extra)),
            _ => return Err(USAGE.to_string()),
        };
        let kind = match kind.to_ascii_lowercase().as_str() {
            "pair" => TargetKind::Pair,
            "token" => TargetKind::Token,
            "address" => TargetKind::Address,
            _ => return Err(USAGE.to_string()),
        };
        let mut t = Target::new(kind, id);
        if let Some(extra) = extra {
            let is_venue = self.venues.iter().any(|v| v.eq_ignore_ascii_case(extra));
            match (kind, is_venue) {
                (TargetKind::Address, _) | (TargetKind::Token, false) => {
                    t.chain = Some(extra.to_string())
                }
                (_, true) => t.venue = Some(extra.to_string()),
                (TargetKind::Pair, false) => {
                    return Err(format!(
                        "unknown venue `{extra}`; the venues are {}",
                        self.venues.join(", ")
                    ));
                }
            }
        }
        if kind == TargetKind::Pair && t.venue.is_none() {
            return Err("a pair needs a venue: `pair USDC binance`".to_string());
        }
        if t.venue.is_some() {
            t.preset = Some(Preset::MarketHealth);
        }
        self.list.targets.push(t);
        self.target = self.list.targets.len() - 1;
        Ok(())
    }

    /// The problem that stops a save or a start, if any.
    pub fn problem(&self) -> Option<String> {
        if self.list.targets.is_empty() {
            return Some("Add a target first (Targets step, `a`).".to_string());
        }
        self.list
            .targets
            .iter()
            .find(|t| t.preset.is_none() && t.rules.is_empty())
            .map(|t| {
                format!(
                    "{} has nothing to check: turn on a metric or the preset (Metrics step).",
                    t.label()
                )
            })
    }

    /// The file Save writes.
    pub fn path(&self) -> PathBuf {
        scope::watch::file::path_in(&self.dir, &self.list.name)
    }

    fn save(&mut self) -> WizardAction {
        if let Some(p) = self.problem() {
            self.error = Some(p);
            return WizardAction::None;
        }
        let path = self.path();
        match scope::watch::file::save(&path, &self.list) {
            Ok(()) => {
                self.dirty = false;
                WizardAction::Saved(path)
            }
            Err(e) => {
                self.error = Some(format!("Could not save {}: {e}", path.display()));
                WizardAction::None
            }
        }
    }

    fn start(&mut self) -> WizardAction {
        if let Some(p) = self.problem() {
            self.error = Some(p);
            return WizardAction::None;
        }
        WizardAction::Start(Box::new(self.list.clone()))
    }
}

/// The rule a metric starts with: its threshold from the defaults, or
/// `> 0` for a metric with none.
fn default_rule(metric: MetricId) -> Rule {
    let (op, value) = match describe(metric, &HealthThresholds::default()).threshold {
        Some(Threshold::Max(v)) => (Op::Le, v),
        Some(Threshold::Min(v)) => (Op::Ge, v),
        Some(Threshold::Target(_)) | None => (Op::Gt, 0.0),
    };
    Rule { metric, op, value }
}

fn next_op(op: Op) -> Op {
    match op {
        Op::Lt => Op::Le,
        Op::Le => Op::Gt,
        Op::Gt => Op::Ge,
        Op::Ge => Op::Eq,
        Op::Eq => Op::Ne,
        Op::Ne => Op::Lt,
    }
}

fn path_text(p: &Option<PathBuf>) -> String {
    p.as_deref()
        .map(Path::display)
        .map(|d| d.to_string())
        .unwrap_or_default()
}

/// A row: the marker, then the text, selected or not.
fn row(selected: bool, text: String) -> Line<'static> {
    if selected {
        Line::from(vec![Span::styled("> ", KEY), Span::styled(text, SELECTED)])
    } else {
        Line::from(vec![Span::raw("  "), Span::raw(text)])
    }
}

fn target_text(t: &Target) -> String {
    let mut s = format!("{:<8} {}", t.kind.name(), t.id);
    if let Some(v) = &t.venue {
        s.push_str(&format!(" @{v}"));
    }
    if let Some(c) = &t.chain {
        s.push_str(&format!(" on {c}"));
    }
    if t.preset.is_some() {
        s.push_str("  [market health]");
    }
    if !t.rules.is_empty() {
        s.push_str(&format!("  {} rule(s)", t.rules.len()));
    }
    s
}

/// Draws the wizard in `area`.
pub fn draw(f: &mut Frame, area: Rect, w: &WizardState) {
    let block = Block::new()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(ACCENT)
        .title(Line::styled(
            format!(" monitor wizard · {} ", w.list.name),
            ACCENT.add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines: Vec<Line> = Vec::new();
    let mut tabs = vec![Span::raw(" ")];
    for s in Step::ALL {
        let style = if s == w.step {
            SELECTED.add_modifier(Modifier::BOLD)
        } else {
            SECONDARY
        };
        tabs.push(Span::styled(
            format!(" {} {} ", s.index() + 1, s.name()),
            style,
        ));
    }
    lines.push(Line::from(tabs));
    for n in &w.notes {
        lines.push(Line::styled(format!(" {n}"), WARN));
    }
    lines.push(Line::raw(""));
    match w.step {
        Step::Targets => {
            if w.list.targets.is_empty() {
                lines.push(Line::styled(
                    " No targets yet: press a to add one.",
                    SECONDARY,
                ));
            }
            for (i, t) in w.list.targets.iter().enumerate() {
                lines.push(row(i == w.target, target_text(t)));
            }
        }
        Step::Metrics => match w.list.targets.get(w.target) {
            None => lines.push(Line::styled(" Add a target first.", SECONDARY)),
            Some(t) => {
                lines.push(Line::styled(
                    format!(
                        " {} ({}/{})  ←→ other target",
                        t.label(),
                        w.target + 1,
                        w.list.targets.len()
                    ),
                    ACCENT,
                ));
                for (i, r) in w.metric_rows().iter().enumerate() {
                    let text = match r {
                        None => format!(
                            "[{}] market health (the checks of market summary)",
                            if t.preset.is_some() { "x" } else { " " }
                        ),
                        Some(m) => match t.rules.iter().find(|x| x.metric == *m) {
                            Some(rule) => format!("[x] {rule}"),
                            None => format!("[ ] {}", m.name()),
                        },
                    };
                    lines.push(row(i == w.row, text));
                }
            }
        },
        Step::Timing => {
            let duration = w.list.duration.map_or_else(
                || "until stopped".to_string(),
                |d| format_duration(d.as_secs()),
            );
            lines.push(row(
                w.row == 0,
                format!("Interval  {}", format_duration(w.list.interval.as_secs())),
            ));
            lines.push(row(w.row == 1, format!("Duration  {duration}")));
            let per = w.list.fetches_per_tick();
            let total = w.list.ticks().map_or_else(
                || "until stopped".to_string(),
                |t| format!("{} in all", t * per as u64),
            );
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                format!(" {per} fetch(es) per run · {total}"),
                SECONDARY,
            ));
        }
        Step::Outputs => {
            let values = [
                w.list.name.clone(),
                path_text(&w.list.outputs.csv),
                path_text(&w.list.outputs.report),
                format!("to {}", w.path().display()),
                "the grid".to_string(),
            ];
            for (i, (label, value)) in OUTPUT_ROWS.iter().zip(values).enumerate() {
                lines.push(row(i == w.row, format!("{label:<7} {value}")));
            }
        }
    }
    lines.push(Line::raw(""));
    if let Some((kind, text)) = &w.edit {
        let label = match kind {
            EditKind::AddTarget => "Add".to_string(),
            EditKind::Value(m) => format!("{} value", m.name()),
            EditKind::Interval => "Interval".to_string(),
            EditKind::Duration => "Duration".to_string(),
            EditKind::Name => "Name".to_string(),
            EditKind::Csv => "CSV".to_string(),
            EditKind::Report => "Report".to_string(),
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {label}: "), KEY),
            Span::raw(text.clone()),
            Span::styled("█", ACCENT),
        ]));
        if *kind == EditKind::AddTarget {
            lines.push(Line::styled(
                "   pair USDC binance · token PEPE solana · address 0x… ethereum · Enter adds · Esc cancels",
                SECONDARY,
            ));
        }
    }
    if w.confirm_leave {
        lines.push(Line::styled(
            " Leave without saving? [y/N]",
            WARN.add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(e) = &w.error {
        lines.push(Line::styled(format!(" {e}"), ERR));
    } else if w.problem().is_none() && !w.dirty {
        lines.push(Line::styled(" Ready: Ctrl-R starts the run.", OK));
    }
    // The legend on the last row.
    let body = Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    };
    let first = lines.len().saturating_sub(body.height as usize);
    f.render_widget(
        Paragraph::new(lines[first.min(lines.len())..].to_vec()),
        body,
    );
    if inner.height > 0 {
        let last = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
        f.render_widget(
            Paragraph::new(Line::styled(format!(" {}", legend(w.step)), KEY)),
            last,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn typed(w: &mut WizardState, text: &str) {
        for c in text.chars() {
            w.key(key(KeyCode::Char(c)));
        }
        w.key(key(KeyCode::Enter));
    }

    fn wizard(dir: &Path) -> WizardState {
        WizardState::new(
            Watchlist::new("w"),
            Vec::new(),
            vec!["binance".into(), "weex".into()],
            dir.to_path_buf(),
        )
    }

    fn screen(w: &WizardState, cols: u16, rows: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(cols, rows)).unwrap();
        t.draw(|f| draw(f, f.area(), w)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut s = String::new();
        for y in 0..rows {
            for x in 0..cols {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn add_targets_with_venue_or_chain_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "pair USDC binance");
        let t = &w.list.targets[0];
        assert_eq!(
            (t.kind, t.venue.as_deref(), t.preset),
            (
                TargetKind::Pair,
                Some("binance"),
                Some(Preset::MarketHealth)
            )
        );
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "token PEPE solana");
        assert_eq!(w.list.targets[1].chain.as_deref(), Some("solana"));
        assert_eq!(w.list.targets[1].preset, None);
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "pair DAI nowhere");
        assert!(
            w.error
                .as_deref()
                .unwrap()
                .contains("unknown venue `nowhere`")
        );
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "pair DAI");
        assert!(w.error.as_deref().unwrap().contains("needs a venue"));
        assert_eq!(w.list.targets.len(), 2);
        w.key(key(KeyCode::Char('d')));
        assert_eq!(w.list.targets.len(), 1);
    }

    #[test]
    fn metrics_toggle_change_op_and_value() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "token PEPE");
        w.key(key(KeyCode::Tab));
        assert_eq!(w.step, Step::Metrics);
        // Row 0 is the preset: a token with no venue cannot have it.
        w.key(key(KeyCode::Char(' ')));
        assert!(w.error.as_deref().unwrap().contains("no venue"));
        let rows = w.metric_rows();
        let holders = rows
            .iter()
            .position(|r| *r == Some(MetricId::Holders))
            .unwrap();
        w.row = holders;
        w.key(key(KeyCode::Char(' ')));
        assert_eq!(w.list.targets[0].rules[0].to_string(), "holders > 0");
        w.key(key(KeyCode::Char('o')));
        assert_eq!(w.list.targets[0].rules[0].op, Op::Ge);
        w.key(key(KeyCode::Enter));
        w.key(key(KeyCode::Backspace));
        typed(&mut w, "1000");
        assert_eq!(w.list.targets[0].rules[0].to_string(), "holders >= 1000");
        w.key(key(KeyCode::Char(' ')));
        assert!(w.list.targets[0].rules.is_empty());
    }

    #[test]
    fn market_metrics_start_from_the_health_thresholds() {
        assert_eq!(default_rule(MetricId::SpreadPct).op, Op::Le);
        assert_eq!(
            default_rule(MetricId::SpreadPct).value,
            HealthThresholds::default().max_spread_pct
        );
        assert_eq!(default_rule(MetricId::BidDepth).op, Op::Ge);
    }

    #[test]
    fn timing_edits_parse_and_bad_values_say_why() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        w.step = Step::Timing;
        w.key(key(KeyCode::Enter));
        for _ in 0..5 {
            w.key(key(KeyCode::Backspace));
        }
        typed(&mut w, "0s");
        assert!(w.error.is_some());
        assert_eq!(w.list.interval, Duration::from_secs(60), "unchanged");
        w.key(key(KeyCode::Enter));
        for _ in 0..5 {
            w.key(key(KeyCode::Backspace));
        }
        typed(&mut w, "30s");
        assert_eq!(w.list.interval, Duration::from_secs(30));
        w.key(key(KeyCode::Down));
        w.key(key(KeyCode::Enter));
        for _ in 0..10 {
            w.key(key(KeyCode::Backspace));
        }
        typed(&mut w, "forever");
        assert_eq!(w.list.duration, None);
    }

    #[test]
    fn start_and_save_need_targets_with_checks_and_save_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        assert_eq!(w.key(ctrl('r')), WizardAction::None);
        assert!(w.error.as_deref().unwrap().contains("Add a target"));
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "token PEPE");
        assert_eq!(w.key(ctrl('s')), WizardAction::None);
        assert!(w.error.as_deref().unwrap().contains("nothing to check"));
        w.key(key(KeyCode::Char('a')));
        w.key(key(KeyCode::Esc));
        w.key(key(KeyCode::Char('d')));
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "pair USDC binance");
        let WizardAction::Saved(path) = w.key(ctrl('s')) else {
            panic!("{:?}", w.error)
        };
        assert_eq!(
            scope::watch::file::load(&path, None).unwrap(),
            w.list,
            "the file reads back equal"
        );
        let WizardAction::Start(list) = w.key(ctrl('r')) else {
            panic!("{:?}", w.error)
        };
        assert_eq!(*list, w.list);
    }

    #[test]
    fn esc_with_changes_asks_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        assert_eq!(
            w.key(key(KeyCode::Esc)),
            WizardAction::Leave,
            "nothing changed"
        );
        let mut w = wizard(dir.path());
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "pair USDC binance");
        assert_eq!(w.key(key(KeyCode::Esc)), WizardAction::None);
        assert!(w.confirm_leave);
        assert_eq!(w.key(key(KeyCode::Char('n'))), WizardAction::None);
        assert!(!w.confirm_leave);
        w.key(key(KeyCode::Esc));
        assert_eq!(w.key(key(KeyCode::Char('y'))), WizardAction::Leave);
    }

    #[test]
    fn draws_each_step_at_several_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        w.notes.push("--layout has no effect".into());
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "pair USDN weex");
        for step in Step::ALL {
            w.step = step;
            for (cols, rows) in [(120, 36), (80, 24), (40, 10)] {
                let s = screen(&w, cols, rows);
                assert!(s.contains("monitor wizard"), "{step:?} {cols}x{rows}\n{s}");
            }
        }
        w.step = Step::Metrics;
        let s = screen(&w, 120, 36);
        assert!(s.contains("[x] market health"), "{s}");
        assert!(s.contains("--layout has no effect"), "{s}");
        w.step = Step::Timing;
        let s = screen(&w, 120, 36);
        assert!(s.contains("1 fetch(es) per run · 61 in all"), "{s}");
    }

    fn clear(w: &mut WizardState) {
        for _ in 0..40 {
            w.key(key(KeyCode::Backspace));
        }
    }

    #[test]
    fn targets_step_moves_the_selection_and_enter_opens_its_metrics() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        w.key(key(KeyCode::Down));
        w.key(key(KeyCode::Enter));
        assert_eq!((w.step, w.target), (Step::Targets, 0), "no target yet");
        for t in ["pair USDC binance", "address 0xabc ethereum", "token PEPE"] {
            w.key(key(KeyCode::Char('a')));
            typed(&mut w, t);
        }
        assert_eq!(w.target, 2, "the new target is selected");
        w.key(key(KeyCode::Down));
        assert_eq!(w.target, 0, "down from the last is the first");
        w.key(key(KeyCode::Up));
        w.key(key(KeyCode::Up));
        assert_eq!(w.target, 1);
        w.row = 3;
        w.key(key(KeyCode::Enter));
        assert_eq!((w.step, w.target, w.row), (Step::Metrics, 1, 0));
    }

    #[test]
    fn metrics_step_moves_between_rows_and_targets() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        w.step = Step::Metrics;
        assert!(w.metric_rows().is_empty(), "no target");
        for code in [KeyCode::Down, KeyCode::Right, KeyCode::Char(' ')] {
            w.key(key(code));
        }
        assert_eq!((w.target, w.row, w.dirty), (0, 0, false));
        assert!(screen(&w, 80, 24).contains("Add a target first."));
        w.step = Step::Targets;
        for t in ["pair USDC binance", "token PEPE"] {
            w.key(key(KeyCode::Char('a')));
            typed(&mut w, t);
        }
        w.key(key(KeyCode::Tab));
        let n = w.metric_rows().len();
        w.key(key(KeyCode::Up));
        assert_eq!(w.row, n - 1, "up from the first row is the last");
        w.key(key(KeyCode::Down));
        assert_eq!(w.row, 0);
        w.key(key(KeyCode::Down));
        w.key(key(KeyCode::Right));
        assert_eq!(
            (w.target, w.row),
            (0, 0),
            "right wraps and starts at the top"
        );
        assert_eq!(w.metric_rows()[0], None, "a pair starts with the preset");
        w.key(key(KeyCode::Down));
        w.key(key(KeyCode::Left));
        assert_eq!((w.target, w.row), (1, 0), "left wraps");
        let s = screen(&w, 120, 36);
        assert!(s.contains("(2/2)"), "{s}");
    }

    #[test]
    fn the_preset_turns_off_and_on_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "pair USDC binance");
        w.key(key(KeyCode::Tab));
        w.key(key(KeyCode::Char(' ')));
        assert_eq!(w.list.targets[0].preset, None);
        assert!(screen(&w, 120, 36).contains("[ ] market health"));
        w.key(key(KeyCode::Char(' ')));
        assert_eq!(w.list.targets[0].preset, Some(Preset::MarketHealth));
        // `o` and Enter have nothing to change on the preset row.
        w.key(key(KeyCode::Char('o')));
        w.key(key(KeyCode::Enter));
        assert_eq!(w.edit, None);
    }

    #[test]
    fn o_goes_through_every_operator_and_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "token PEPE");
        w.key(key(KeyCode::Tab));
        w.row = w
            .metric_rows()
            .iter()
            .position(|r| *r == Some(MetricId::Holders))
            .unwrap();
        w.key(key(KeyCode::Char('o')));
        assert!(w.list.targets[0].rules.is_empty(), "no rule to change yet");
        w.key(key(KeyCode::Char(' ')));
        let ops: Vec<Op> = (0..6)
            .map(|_| {
                w.key(key(KeyCode::Char('o')));
                w.list.targets[0].rules[0].op
            })
            .collect();
        assert_eq!(ops, [Op::Ge, Op::Eq, Op::Ne, Op::Lt, Op::Le, Op::Gt]);
    }

    #[test]
    fn a_value_must_be_a_finite_number() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "token PEPE");
        w.key(key(KeyCode::Tab));
        w.row = w
            .metric_rows()
            .iter()
            .position(|r| *r == Some(MetricId::Holders))
            .unwrap();
        w.key(key(KeyCode::Char(' ')));
        for bad in ["lots", "inf", ""] {
            w.key(key(KeyCode::Enter));
            clear(&mut w);
            typed(&mut w, bad);
            assert_eq!(
                w.error.as_deref(),
                Some(format!("`{bad}` is not a number").as_str())
            );
            assert_eq!(w.list.targets[0].rules[0].value, 0.0, "unchanged");
        }
        w.key(key(KeyCode::Enter));
        clear(&mut w);
        typed(&mut w, "1_000");
        assert_eq!(
            w.list.targets[0].rules[0].value, 1000.0,
            "`_` groups digits"
        );
    }

    #[test]
    fn while_typing_ctrl_and_other_keys_do_not_type() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        w.key(key(KeyCode::Char('a')));
        w.key(key(KeyCode::Char('p')));
        for k in [ctrl('s'), ctrl('r'), key(KeyCode::Left), key(KeyCode::Tab)] {
            assert_eq!(w.key(k), WizardAction::None);
        }
        assert_eq!(w.edit, Some((EditKind::AddTarget, "p".to_string())));
        assert_eq!(w.step, Step::Targets);
        assert_eq!(w.error, None, "Ctrl-S did not try to save");
        w.key(key(KeyCode::Esc));
        assert_eq!(w.edit, None, "Esc cancels");
        assert!(w.list.targets.is_empty());
    }

    #[test]
    fn shift_tab_goes_back_and_wraps() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        w.row = 2;
        w.key(key(KeyCode::BackTab));
        assert_eq!((w.step, w.row), (Step::Outputs, 0));
        w.key(key(KeyCode::BackTab));
        assert_eq!(w.step, Step::Timing);
        w.key(key(KeyCode::Tab));
        w.key(key(KeyCode::Tab));
        assert_eq!(w.step, Step::Targets, "Tab wraps too");
    }

    #[test]
    fn the_duration_edit_starts_from_the_current_one_and_counts_the_fetches() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "pair USDC binance");
        w.list.duration = Some(Duration::from_secs(7200));
        w.step = Step::Timing;
        w.key(key(KeyCode::Up));
        assert_eq!(w.row, 1, "up from the interval is the duration");
        w.key(key(KeyCode::Enter));
        assert_eq!(w.edit, Some((EditKind::Duration, format_duration(7200))));
        clear(&mut w);
        typed(&mut w, "soon");
        assert!(w.error.is_some(), "not a duration");
        assert_eq!(w.list.duration, Some(Duration::from_secs(7200)));
        w.key(key(KeyCode::Enter));
        clear(&mut w);
        typed(&mut w, "30m");
        assert_eq!(w.list.duration, Some(Duration::from_secs(1800)));
        let s = screen(&w, 120, 36);
        assert!(
            s.contains(&format!("Duration  {}", format_duration(1800))),
            "{s}"
        );
        // Every 60 s for 30 min: 30 runs and the first.
        assert!(s.contains("1 fetch(es) per run · 31 in all"), "{s}");
    }

    #[test]
    fn outputs_edit_the_name_and_paths_then_save_and_start() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "pair USDC binance");
        w.step = Step::Outputs;
        w.key(key(KeyCode::Up));
        assert_eq!(w.row, 4, "up from Name is Start");
        w.key(key(KeyCode::Down));
        assert_eq!(w.row, 0);
        for bad in ["", "a/b", "c:d"] {
            w.key(key(KeyCode::Enter));
            clear(&mut w);
            typed(&mut w, bad);
            assert!(
                w.error.as_deref().unwrap().contains("a name has no"),
                "{bad:?}"
            );
            assert_eq!(w.list.name, "w", "unchanged");
        }
        w.key(key(KeyCode::Enter));
        assert_eq!(w.edit, Some((EditKind::Name, "w".to_string())));
        clear(&mut w);
        typed(&mut w, " stables ");
        assert_eq!(w.list.name, "stables", "trimmed");
        w.key(key(KeyCode::Down));
        w.key(key(KeyCode::Enter));
        typed(&mut w, "out.csv");
        assert_eq!(w.list.outputs.csv, Some(PathBuf::from("out.csv")));
        w.key(key(KeyCode::Enter));
        assert_eq!(w.edit, Some((EditKind::Csv, "out.csv".to_string())));
        clear(&mut w);
        w.key(key(KeyCode::Enter));
        assert_eq!(w.list.outputs.csv, None, "an empty path is no file");
        w.key(key(KeyCode::Down));
        w.key(key(KeyCode::Enter));
        typed(&mut w, "report.md");
        assert_eq!(w.list.outputs.report, Some(PathBuf::from("report.md")));
        w.key(key(KeyCode::Enter));
        clear(&mut w);
        w.key(key(KeyCode::Enter));
        assert_eq!(w.list.outputs.report, None);
        let s = screen(&w, 120, 36);
        assert!(s.contains(&format!("to {}", w.path().display())), "{s}");
        w.key(key(KeyCode::Down));
        let WizardAction::Saved(path) = w.key(key(KeyCode::Enter)) else {
            panic!("{:?}", w.error)
        };
        assert_eq!(path, w.path());
        assert!(path.exists());
        assert!(!w.dirty);
        w.key(key(KeyCode::Down));
        let WizardAction::Start(list) = w.key(key(KeyCode::Enter)) else {
            panic!("{:?}", w.error)
        };
        assert_eq!(list.name, "stables");
    }

    #[test]
    fn add_target_says_how_when_the_line_is_wrong() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        for bad in ["pair", "pair USDC binance extra", "wallet 0xabc"] {
            w.key(key(KeyCode::Char('a')));
            typed(&mut w, bad);
            assert!(
                w.error.as_deref().unwrap().starts_with("type `pair USDC"),
                "{bad}: {:?}",
                w.error
            );
        }
        assert!(w.list.targets.is_empty());
        // An address's third word is always its chain, even a venue's name;
        // a token's is its venue when it names one.
        for line in ["ADDRESS 0xabc binance", "token PEPE WEEX"] {
            w.key(key(KeyCode::Char('a')));
            typed(&mut w, line);
        }
        let [a, t] = &w.list.targets[..] else {
            panic!("{:?}", w.list.targets)
        };
        assert_eq!(
            (a.kind, a.chain.as_deref(), a.venue.as_deref()),
            (TargetKind::Address, Some("binance"), None)
        );
        assert_eq!(
            (t.venue.as_deref(), t.preset),
            (Some("WEEX"), Some(Preset::MarketHealth))
        );
    }

    #[test]
    fn a_save_that_fails_says_why_and_stays_dirty() {
        let dir = tempfile::tempdir().unwrap();
        // The folder is a file: it cannot hold the watchlist.
        let file = dir.path().join("not-a-folder");
        std::fs::write(&file, "").unwrap();
        let mut w = wizard(&file);
        w.key(key(KeyCode::Char('a')));
        typed(&mut w, "pair USDC binance");
        assert_eq!(w.key(ctrl('s')), WizardAction::None);
        assert!(
            w.error.as_deref().unwrap().starts_with("Could not save"),
            "{:?}",
            w.error
        );
        assert!(w.dirty);
        assert!(screen(&w, 120, 36).contains("Could not save"));
    }

    #[test]
    fn draws_the_targets_the_line_being_typed_and_the_question() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = wizard(dir.path());
        let s = screen(&w, 120, 36);
        assert!(s.contains("No targets yet: press a to add one."), "{s}");
        assert!(!s.contains("Ready"), "nothing to start: {s}");
        for t in ["address 0xabc ethereum", "token PEPE"] {
            w.key(key(KeyCode::Char('a')));
            typed(&mut w, t);
        }
        w.list.targets[1]
            .rules
            .push(default_rule(MetricId::Holders));
        let s = screen(&w, 120, 36);
        assert!(s.contains("0xabc on ethereum"), "{s}");
        assert!(s.contains("PEPE  1 rule(s)"), "{s}");
        for (kind, label) in [
            (EditKind::AddTarget, "Add"),
            (EditKind::Value(MetricId::Holders), "holders value"),
            (EditKind::Interval, "Interval"),
            (EditKind::Duration, "Duration"),
            (EditKind::Name, "Name"),
            (EditKind::Csv, "CSV"),
            (EditKind::Report, "Report"),
        ] {
            let hint = kind == EditKind::AddTarget;
            w.edit = Some((kind, "abc".to_string()));
            let s = screen(&w, 120, 36);
            assert!(s.contains(&format!(" {label}: abc█")), "{label}\n{s}");
            assert_eq!(s.contains("Enter adds · Esc cancels"), hint, "{s}");
        }
        w.edit = None;
        w.key(key(KeyCode::Esc));
        assert!(screen(&w, 120, 36).contains("Leave without saving? [y/N]"));
        w.key(key(KeyCode::Char('n')));
        w.dirty = false;
        let s = screen(&w, 120, 36);
        assert!(!s.contains("Leave without saving"), "{s}");
        assert!(!s.contains("Ready"), "the address checks nothing: {s}");
        w.list.targets[0]
            .rules
            .push(default_rule(MetricId::RiskScore));
        let s = screen(&w, 120, 36);
        assert!(s.contains("Ready: Ctrl-R starts the run."), "{s}");
    }
}
