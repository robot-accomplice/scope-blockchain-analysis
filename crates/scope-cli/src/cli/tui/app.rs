//! # TUI State and Rendering
//!
//! [`TuiState`] holds everything the screen shows: the scrollback, the
//! input line, the history and the command that runs now. It does no
//! terminal I/O, so tests drive it with key events and render it on a
//! `TestBackend`.

use super::exec::{self, Invocation, Plan, Route};
use super::journal::{Event, Journal};
use super::vocab::Vocab;
use crate::cli::interactive::SessionContext;
use crate::cli::output::{Captured, Output};
use ansi_to_tui::IntoText;
use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph, Wrap};
use scope::config::Config;
use std::time::Instant;

/// The most lines kept in the scrollback. Older lines are dropped.
pub const SCROLLBACK_LIMIT: usize = 5000;

/// Lines moved by one PageUp / PageDown.
const PAGE: usize = 10;

/// The most completion candidates shown at once.
const MENU_ROWS: usize = 8;

/// A floating list over the screen.
pub enum Overlay {
    /// Completion candidates.
    Menu(Menu),
}

/// The open completion menu.
pub struct Menu {
    /// Byte index where the completed word starts.
    pub start: usize,
    /// The candidates.
    pub candidates: Vec<String>,
    /// The selected candidate.
    pub selected: usize,
}

fn common_prefix(items: &[String]) -> String {
    let first = &items[0];
    let mut end = first.len();
    for s in &items[1..] {
        end = end.min(
            first
                .bytes()
                .zip(s.bytes())
                .take_while(|(a, b)| a == b)
                .count(),
        );
    }
    while !first.is_char_boundary(end) {
        end -= 1;
    }
    first[..end].to_string()
}

/// What the event loop must do after a key.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing beyond a redraw.
    None,
    /// Run this line.
    Submit(String),
    /// Cancel the running command.
    Cancel,
    /// Leave the TUI.
    Exit,
}

/// A command that runs in the output pane now.
pub struct Running {
    /// The line the user typed.
    pub input: String,
    /// The parsed argument vector, for the event log.
    pub argv: Vec<String>,
    /// When it started.
    pub started: Instant,
    /// When it started, for the event log.
    pub started_at: String,
    /// The command's output so far.
    pub captured: Captured,
    /// The writer the handler got; the error display writes to it too.
    out: Output,
    /// Routes library warnings into this command's output while it runs.
    _diag: scope::diag::RedirectGuard,
}

/// How a pane command ended.
pub enum Ending {
    /// The handler returned.
    Finished(scope::error::Result<()>),
    /// The user cancelled it.
    Cancelled,
}

/// The complete TUI state.
pub struct TuiState {
    /// The input line.
    pub input: String,
    /// Cursor position in `input`, in chars.
    cursor: usize,
    /// Submitted lines, oldest first.
    pub history: Vec<String>,
    /// Position while browsing the history with ↑/↓.
    hist_pos: Option<usize>,
    /// The input line from before history browsing started.
    draft: String,
    /// Everything shown in the output pane, except the running command.
    scrollback: Vec<Line<'static>>,
    /// How many lines the view is scrolled up from the bottom.
    scroll: usize,
    /// The session context.
    pub ctx: SessionContext,
    /// The command that runs in the pane now.
    pub running: Option<Running>,
    /// The event log.
    journal: Journal,
    /// Completion values from the user's stores.
    vocab: Vocab,
    /// The open completion menu or palette, if any.
    pub overlay: Option<Overlay>,
}

impl TuiState {
    /// Creates the state with a context and a history.
    pub fn new(ctx: SessionContext, journal: Journal, vocab: Vocab) -> Self {
        let history = journal.load_history();
        Self {
            input: String::new(),
            cursor: 0,
            history,
            hist_pos: None,
            draft: String::new(),
            scrollback: Vec::new(),
            scroll: 0,
            ctx,
            running: None,
            journal,
            vocab,
            overlay: None,
        }
    }

    /// The journal, for saving the history on exit.
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    // ---------------------------------------------------------------- keys

    /// Applies one key event.
    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(Overlay::Menu(menu)) = &mut self.overlay {
            match key.code {
                KeyCode::Tab | KeyCode::Down => {
                    menu.selected = (menu.selected + 1) % menu.candidates.len();
                    return Action::None;
                }
                KeyCode::BackTab | KeyCode::Up => {
                    menu.selected =
                        (menu.selected + menu.candidates.len() - 1) % menu.candidates.len();
                    return Action::None;
                }
                KeyCode::Enter => {
                    let (start, pick) = (menu.start, menu.candidates[menu.selected].clone());
                    self.overlay = None;
                    self.replace_word(start, &pick, true);
                    return Action::None;
                }
                KeyCode::Esc => {
                    self.overlay = None;
                    return Action::None;
                }
                _ => self.overlay = None,
            }
        }
        match key.code {
            KeyCode::Char('c') if ctrl => {
                if self.running.is_some() {
                    return Action::Cancel;
                }
                if self.input.is_empty() {
                    self.push_note("Type `exit` or press Ctrl-D to leave.");
                } else {
                    self.set_input(String::new());
                }
            }
            KeyCode::Char('d') if ctrl => {
                if self.input.is_empty() && self.running.is_none() {
                    return Action::Exit;
                }
            }
            KeyCode::Esc => {
                if self.running.is_some() {
                    return Action::Cancel;
                }
                self.set_input(String::new());
            }
            KeyCode::Enter => {
                // One command at a time; the line stays for after it ends.
                if self.running.is_some() {
                    return Action::None;
                }
                let line = std::mem::take(&mut self.input);
                self.cursor = 0;
                self.hist_pos = None;
                self.scroll = 0;
                if !line.trim().is_empty() && self.history.last() != Some(&line) {
                    self.history.push(line.clone());
                }
                return Action::Submit(line);
            }
            KeyCode::Tab => self.tab_complete(),
            KeyCode::Up => self.history_prev(),
            KeyCode::Down => self.history_next(),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_add(PAGE),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(PAGE),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.char_len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.char_len(),
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.char_len(),
            KeyCode::Char('u') if ctrl => {
                let rest: String = self.input.chars().skip(self.cursor).collect();
                self.input = rest;
                self.cursor = 0;
            }
            KeyCode::Char('w') if ctrl => self.delete_word(),
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.remove_char(self.cursor);
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.char_len() {
                    self.remove_char(self.cursor);
                }
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                let at = self.byte_index(self.cursor);
                self.input.insert(at, c);
                self.cursor += 1;
            }
            _ => {}
        }
        Action::None
    }

    fn tab_complete(&mut self) {
        if self.cursor != self.char_len() {
            return;
        }
        let c = super::complete::complete(&self.input, &self.vocab);
        match c.candidates.len() {
            0 => {}
            1 => self.replace_word(c.start, &c.candidates[0], true),
            _ => {
                let prefix = common_prefix(&c.candidates);
                self.replace_word(c.start, &prefix, false);
                self.overlay = Some(Overlay::Menu(Menu {
                    start: c.start,
                    candidates: c.candidates,
                    selected: 0,
                }));
            }
        }
    }

    /// Replaces `input[start..]` with `word`, optionally followed by a space.
    fn replace_word(&mut self, start: usize, word: &str, space: bool) {
        self.input.truncate(start);
        self.input.push_str(word);
        if space {
            self.input.push(' ');
        }
        self.cursor = self.char_len();
    }

    fn char_len(&self) -> usize {
        self.input.chars().count()
    }

    fn byte_index(&self, char_idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_idx)
            .map_or(self.input.len(), |(i, _)| i)
    }

    fn remove_char(&mut self, char_idx: usize) {
        let at = self.byte_index(char_idx);
        self.input.remove(at);
    }

    fn delete_word(&mut self) {
        let chars: Vec<char> = self.input.chars().collect();
        let mut start = self.cursor;
        while start > 0 && chars[start - 1] == ' ' {
            start -= 1;
        }
        while start > 0 && chars[start - 1] != ' ' {
            start -= 1;
        }
        self.input = chars[..start].iter().chain(&chars[self.cursor..]).collect();
        self.cursor = start;
    }

    fn set_input(&mut self, s: String) {
        self.cursor = s.chars().count();
        self.input = s;
    }

    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let pos = match self.hist_pos {
            None => {
                self.draft = self.input.clone();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(p) => p - 1,
        };
        self.hist_pos = Some(pos);
        self.set_input(self.history[pos].clone());
    }

    fn history_next(&mut self) {
        match self.hist_pos {
            None => {}
            Some(p) if p + 1 < self.history.len() => {
                self.hist_pos = Some(p + 1);
                self.set_input(self.history[p + 1].clone());
            }
            Some(_) => {
                self.hist_pos = None;
                let draft = std::mem::take(&mut self.draft);
                self.set_input(draft);
            }
        }
    }

    // ---------------------------------------------------------- scrollback

    /// Appends text that may hold ANSI color codes.
    pub fn push_ansi(&mut self, text: &str) {
        let text = text.strip_suffix('\n').unwrap_or(text);
        if text.is_empty() {
            return;
        }
        let lines = match text.into_text() {
            Ok(t) => t.lines,
            // Not valid ANSI: show the raw text rather than lose it.
            Err(_) => text.lines().map(|l| Line::raw(l.to_string())).collect(),
        };
        self.push_lines(lines);
    }

    /// Appends a dim note line.
    pub fn push_note(&mut self, note: &str) {
        self.push_lines(vec![Line::styled(
            note.to_string(),
            Style::new().fg(Color::DarkGray),
        )]);
    }

    fn push_lines(&mut self, lines: Vec<Line<'static>>) {
        self.scrollback.extend(lines);
        let excess = self.scrollback.len().saturating_sub(SCROLLBACK_LIMIT);
        self.scrollback.drain(..excess);
    }

    fn push_echo(&mut self, line: &str) {
        self.push_lines(vec![Line::from(vec![
            Span::styled(
                self.prompt(),
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::raw(line.to_string()),
        ])]);
    }

    fn prompt(&self) -> String {
        format!("scope:{}> ", self.ctx.chain)
    }

    /// The scrollback as plain text, one line per row. For tests and logs.
    pub fn scrollback_text(&self) -> String {
        let mut s = String::new();
        for line in &self.scrollback {
            for span in &line.spans {
                s.push_str(&span.content);
            }
            s.push('\n');
        }
        s
    }

    // ------------------------------------------------------------ commands

    /// Plans a submitted line. Session commands and errors finish here; a
    /// CLI command comes back for the event loop to run.
    pub fn submit(&mut self, line: &str, config: &Config) -> Plan {
        self.push_echo(line);
        let (out, captured) = Output::capture_merged();
        let started = Instant::now();
        let started_at = Utc::now().to_rfc3339();
        let plan = match exec::plan(line, &mut self.ctx, config, &out) {
            Ok(plan) => plan,
            // A capture buffer cannot fail to write; keep this path loud anyway.
            Err(e) => {
                self.push_note(&format!("Output error: {}", e));
                Plan::Done
            }
        };
        self.push_ansi(&captured.out());
        if !line.trim().is_empty() && !matches!(plan, Plan::Run(_)) {
            self.log(Event {
                started: started_at,
                duration_ms: started.elapsed().as_millis(),
                input: line,
                argv: &[],
                route: "session",
                outcome: "session",
                error: None,
            });
        }
        plan
    }

    /// Starts a pane command. Returns the output its handler must write to.
    pub fn start(&mut self, input: &str, inv: &Invocation) -> Output {
        let (out, captured) = Output::capture_merged();
        let diag = scope::diag::redirect(captured.out_sink());
        self.running = Some(Running {
            input: input.to_string(),
            argv: inv.argv.clone(),
            started: Instant::now(),
            started_at: Utc::now().to_rfc3339(),
            captured,
            out: out.clone(),
            _diag: diag,
        });
        out
    }

    /// Ends the pane command: moves its output into the scrollback, shows
    /// the error if it failed, and logs the event.
    pub fn finish(&mut self, ending: Ending) {
        let Some(run) = self.running.take() else {
            return;
        };
        let (outcome, error) = match &ending {
            Ending::Finished(Ok(())) => ("ok", None),
            Ending::Finished(Err(e)) => ("error", Some(e.to_string())),
            Ending::Cancelled => ("cancelled", None),
        };
        if let Ending::Finished(Err(e)) = &ending {
            // Same text and hints as the CLI prints on failure.
            if let Err(write_err) = crate::cli::errors::display_error(e, &run.out) {
                self.push_note(&format!("Output error: {}", write_err));
            }
        }
        self.push_ansi(&run.captured.out());
        match &ending {
            Ending::Cancelled => self.push_note("Cancelled."),
            Ending::Finished(_) => self.push_note(&format!(
                "Done in {:.1}s.",
                run.started.elapsed().as_secs_f64()
            )),
        }
        self.log(Event {
            started: run.started_at,
            duration_ms: run.started.elapsed().as_millis(),
            input: &run.input,
            argv: &run.argv,
            route: "pane",
            outcome,
            error,
        });
    }

    /// Records a command that ran outside the TUI (suspend mode).
    /// `duration` is the command's own run time, without the wait for Enter.
    pub fn record_suspended(
        &mut self,
        input: &str,
        inv_argv: &[String],
        duration: std::time::Duration,
        started_at: String,
        result: &scope::error::Result<()>,
    ) {
        let (outcome, error) = match result {
            Ok(()) => ("ok", None),
            Err(e) => ("error", Some(e.to_string())),
        };
        self.push_note(&format!(
            "Ran outside the TUI: {} ({}).",
            input,
            if result.is_ok() { "ok" } else { "failed" }
        ));
        self.log(Event {
            started: started_at,
            duration_ms: duration.as_millis(),
            input,
            argv: inv_argv,
            route: "suspend",
            outcome,
            error,
        });
    }

    fn log(&mut self, event: Event<'_>) {
        if let Err(e) = self.journal.record(&event) {
            self.push_note(&format!("Could not write the TUI event log: {}", e));
        }
    }

    // -------------------------------------------------------------- render

    /// Draws the screen: output pane, input line, status line.
    pub fn render(&self, f: &mut Frame) {
        let [pane, input, hint_row, status] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(f.area());

        // Output pane: scrollback plus the live output of a running command.
        let mut lines = self.scrollback.clone();
        if let Some(run) = &self.running
            && let Ok(t) = run.captured.out().into_text()
        {
            lines.extend(t.lines);
        }
        let block = Block::bordered().title(format!(" scope v{} ", scope::VERSION));
        let inner = block.inner(pane);
        let para = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
        // Count wrapped rows at the inner width, without the borders, so the
        // newest line sits on the last inner row.
        let total = para.line_count(inner.width);
        let max_top = total.saturating_sub(inner.height as usize);
        let top = max_top.saturating_sub(self.scroll.min(max_top));
        f.render_widget(para.block(block).scroll((top as u16, 0)), pane);

        // Input line with the prompt in the border title.
        let block = Block::bordered().title(self.prompt());
        f.render_widget(Paragraph::new(self.input.as_str()).block(block), input);
        let cursor_x = input.x + 1 + self.cursor as u16;
        f.set_cursor_position(Position::new(
            cursor_x.min(input.right().saturating_sub(2)),
            input.y + 1,
        ));

        if let Some(Overlay::Menu(m)) = &self.overlay {
            let rows = m.candidates.len().min(MENU_ROWS);
            let first = m.selected.saturating_sub(rows - 1);
            let height = (rows as u16 + 2).min(input.y);
            let width = m
                .candidates
                .iter()
                .map(|c| c.chars().count())
                .max()
                .unwrap_or(0) as u16
                + 4;
            let area = ratatui::layout::Rect {
                x: input.x,
                y: input.y.saturating_sub(height),
                width: width.min(input.width),
                height,
            };
            let lines: Vec<Line> = m.candidates[first..first + rows]
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let style = if first + i == m.selected {
                        Style::new().add_modifier(Modifier::REVERSED)
                    } else {
                        Style::new()
                    };
                    Line::styled(c.clone(), style)
                })
                .collect();
            f.render_widget(ratatui::widgets::Clear, area);
            f.render_widget(Paragraph::new(lines).block(Block::bordered()), area);
        }

        if let Some(h) = super::hint::hint(&self.input) {
            let mut spans = vec![Span::styled(
                format!(" {}", h.text),
                Style::new().fg(Color::DarkGray),
            )];
            if let Some(next) = h.next {
                spans.push(Span::styled(
                    format!("  next: {}", next),
                    Style::new().fg(Color::Yellow),
                ));
            }
            f.render_widget(Paragraph::new(Line::from(spans)), hint_row);
        }
        f.render_widget(Paragraph::new(self.status_line()), status);
    }

    fn status_line(&self) -> Line<'static> {
        let ctx = &self.ctx;
        let left = format!(
            " chain {} · format {:?} · limit {}{}{}",
            ctx.chain,
            ctx.format,
            ctx.limit,
            if ctx.include_tokens {
                " · +tokens"
            } else {
                ""
            },
            if ctx.include_txs { " · +txs" } else { "" },
        );
        let right = match &self.running {
            Some(run) => format!(
                "  running {:.0}s · Esc cancels · `{}`",
                run.started.elapsed().as_secs_f64(),
                run.input
            ),
            None if self.scroll > 0 => format!("  scrolled up {} lines · PgDn", self.scroll),
            None => "  help · exit".to_string(),
        };
        Line::from(vec![
            Span::styled(left, Style::new().fg(Color::DarkGray)),
            Span::styled(right, Style::new().fg(Color::Yellow)),
        ])
    }
}

/// Runs a parsed command with the given output. `--ai` switches this one
/// command to markdown, as on the command line.
pub async fn run_invocation(
    inv: Invocation,
    config: &Config,
    clients: &dyn scope::chains::ChainClientFactory,
    out: &Output,
) -> scope::error::Result<()> {
    debug_assert!(!matches!(inv.route, Route::Refuse(_)));
    if inv.ai {
        let mut config = config.clone();
        config.output.format = scope::config::OutputFormat::Markdown;
        crate::cli::dispatch::dispatch_with(inv.command, &config, clients, out).await
    } else {
        crate::cli::dispatch::dispatch_with(inv.command, config, clients, out).await
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

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

    fn venues_list() -> crate::cli::Commands {
        use clap::Parser;
        crate::cli::Cli::try_parse_from(["scope", "venues", "list"])
            .unwrap()
            .command
    }

    fn state() -> TuiState {
        TuiState::new(
            SessionContext::default(),
            Journal::default(),
            Vocab::default(),
        )
    }

    fn type_str(s: &mut TuiState, text: &str) {
        for c in text.chars() {
            s.handle_key(key(KeyCode::Char(c)));
        }
    }

    fn screen(s: &TuiState, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| s.render(f)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn vocab_state() -> TuiState {
        TuiState::new(
            SessionContext::default(),
            Journal::default(),
            Vocab {
                venues: vec!["binance".into(), "bitget".into(), "okx".into()],
                ..Vocab::default()
            },
        )
    }

    #[test]
    fn test_tab_single_candidate_completes_and_adds_space() {
        let mut s = vocab_state();
        type_str(&mut s, "market su");
        s.handle_key(key(KeyCode::Tab));
        assert_eq!(s.input, "market summary ");
        assert!(s.overlay.is_none());
    }

    #[test]
    fn test_tab_many_candidates_fills_prefix_and_opens_menu() {
        let mut s = vocab_state();
        type_str(&mut s, "market summary USDC --venue b");
        s.handle_key(key(KeyCode::Tab));
        assert_eq!(s.input, "market summary USDC --venue bi");
        let Some(Overlay::Menu(m)) = &s.overlay else {
            panic!("menu expected")
        };
        assert_eq!(m.candidates, vec!["binance", "bitget"]);
        s.handle_key(key(KeyCode::Tab));
        s.handle_key(key(KeyCode::Enter));
        assert_eq!(s.input, "market summary USDC --venue bitget ");
        assert!(
            s.overlay.is_none(),
            "Enter in the menu accepts; it does not submit"
        );
    }

    #[test]
    fn test_tab_mid_line_does_nothing() {
        // Review Focus 1.
        let mut s = vocab_state();
        type_str(&mut s, "market su");
        s.handle_key(key(KeyCode::Left));
        s.handle_key(key(KeyCode::Tab));
        assert_eq!(s.input, "market su");
        assert!(s.overlay.is_none());
    }

    #[test]
    fn test_menu_escape_and_other_keys_close_it() {
        let mut s = vocab_state();
        type_str(&mut s, "ad");
        s.handle_key(key(KeyCode::Tab));
        assert!(matches!(s.overlay, Some(Overlay::Menu(_))));
        s.handle_key(key(KeyCode::Esc));
        assert!(s.overlay.is_none());
        assert_eq!(s.input, "address", "Esc closes the menu, not the line");
        s.handle_key(key(KeyCode::Tab));
        type_str(&mut s, "-");
        assert!(s.overlay.is_none());
        assert_eq!(s.input, "address-");
    }

    #[test]
    fn test_tab_works_while_a_command_runs() {
        // Review Focus 5.
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = vocab_state();
        let inv = Invocation {
            command: venues_list(),
            route: Route::Pane,
            ai: false,
            argv: vec![],
        };
        s.start("venues list", &inv);
        type_str(&mut s, "market su");
        s.handle_key(key(KeyCode::Tab));
        assert_eq!(s.input, "market summary ");
        assert_eq!(s.handle_key(key(KeyCode::Enter)), Action::None);
        s.finish(Ending::Cancelled);
    }

    #[test]
    fn test_menu_renders_bounded_and_tiny_terminal_is_safe() {
        // Review Focus 4.
        let mut s = TuiState::new(
            SessionContext::default(),
            Journal::default(),
            Vocab {
                venues: (0..40).map(|i| format!("v{:02}", i)).collect(),
                ..Vocab::default()
            },
        );
        type_str(&mut s, "market summary X --venue v");
        s.handle_key(key(KeyCode::Tab));
        for _ in 0..20 {
            s.handle_key(key(KeyCode::Tab));
        }
        let text = screen(&s, 60, 20);
        assert!(
            text.contains("v20"),
            "the selection stays visible: {}",
            text
        );
        assert!(!text.contains("v00"), "at most 8 rows: {}", text);
        screen(&s, 10, 5);
    }

    #[test]
    fn test_editing_keys() {
        let mut s = state();
        type_str(&mut s, "adress");
        s.handle_key(key(KeyCode::Left));
        s.handle_key(key(KeyCode::Left));
        s.handle_key(key(KeyCode::Left));
        s.handle_key(key(KeyCode::Left));
        type_str(&mut s, "d");
        assert_eq!(s.input, "address");
        s.handle_key(key(KeyCode::End));
        type_str(&mut s, " 0xab cd");
        s.handle_key(ctrl('w'));
        assert_eq!(s.input, "address 0xab ");
        s.handle_key(key(KeyCode::Backspace));
        s.handle_key(ctrl('u'));
        assert_eq!(s.input, "");
    }

    #[test]
    fn test_cursor_keys_and_idle_escape() {
        let mut s = state();
        type_str(&mut s, "bc");
        s.handle_key(key(KeyCode::Home));
        type_str(&mut s, "a");
        s.handle_key(ctrl('e'));
        type_str(&mut s, "d");
        assert_eq!(s.input, "abcd");
        s.handle_key(ctrl('a'));
        s.handle_key(key(KeyCode::Delete));
        assert_eq!(s.input, "bcd");
        s.handle_key(key(KeyCode::End));
        s.handle_key(key(KeyCode::Delete));
        assert_eq!(s.input, "bcd", "Delete at the end does nothing");
        s.handle_key(key(KeyCode::Right));
        type_str(&mut s, "e");
        assert_eq!(s.input, "bcde");
        // Alt-modified chars are not text.
        s.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT));
        assert_eq!(s.input, "bcde");
        s.handle_key(key(KeyCode::Esc));
        assert_eq!(s.input, "", "Esc clears the line when idle");
        // Ctrl-C on an empty idle line explains how to leave.
        s.handle_key(ctrl('c'));
        assert!(s.scrollback_text().contains("Ctrl-D to leave"));
        // History keys with no history do nothing.
        s.handle_key(key(KeyCode::Up));
        s.handle_key(key(KeyCode::Down));
        assert_eq!(s.input, "");
        s.handle_key(key(KeyCode::PageDown));
    }

    #[test]
    fn test_multibyte_input_does_not_panic() {
        let mut s = state();
        type_str(&mut s, "tokens add €UR");
        s.handle_key(key(KeyCode::Left));
        s.handle_key(key(KeyCode::Left));
        s.handle_key(key(KeyCode::Backspace));
        assert_eq!(s.input, "tokens add UR");
    }

    #[test]
    fn test_enter_submits_and_records_history() {
        let mut s = state();
        type_str(&mut s, "venues list");
        assert_eq!(
            s.handle_key(key(KeyCode::Enter)),
            Action::Submit("venues list".into())
        );
        assert_eq!(s.input, "");
        type_str(&mut s, "venues list");
        s.handle_key(key(KeyCode::Enter));
        // Consecutive duplicates are stored once.
        assert_eq!(s.history, vec!["venues list"]);
    }

    #[test]
    fn test_history_browsing_restores_draft() {
        let mut s = state();
        for cmd in ["one", "two"] {
            type_str(&mut s, cmd);
            s.handle_key(key(KeyCode::Enter));
        }
        type_str(&mut s, "draft");
        s.handle_key(key(KeyCode::Up));
        assert_eq!(s.input, "two");
        s.handle_key(key(KeyCode::Up));
        s.handle_key(key(KeyCode::Up));
        assert_eq!(s.input, "one");
        s.handle_key(key(KeyCode::Down));
        assert_eq!(s.input, "two");
        s.handle_key(key(KeyCode::Down));
        assert_eq!(s.input, "draft");
    }

    #[test]
    fn test_ctrl_c_and_esc_cancel_only_when_running() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = state();
        type_str(&mut s, "abc");
        assert_eq!(s.handle_key(ctrl('c')), Action::None);
        assert_eq!(s.input, "", "Ctrl-C clears the line when idle");
        assert_eq!(s.handle_key(ctrl('d')), Action::Exit);

        let inv = Invocation {
            command: venues_list(),
            route: Route::Pane,
            ai: false,
            argv: vec![],
        };
        s.start("venues list", &inv);
        assert_eq!(s.handle_key(ctrl('c')), Action::Cancel);
        assert_eq!(s.handle_key(key(KeyCode::Esc)), Action::Cancel);
        assert_eq!(
            s.handle_key(ctrl('d')),
            Action::None,
            "no exit while running"
        );
        type_str(&mut s, "x");
        assert_eq!(s.handle_key(key(KeyCode::Enter)), Action::None);
        assert_eq!(s.input, "x", "the next line waits for the command to end");
        s.finish(Ending::Cancelled);
        assert!(s.scrollback_text().contains("Cancelled."));
    }

    #[test]
    fn test_session_command_output_reaches_scrollback() {
        let mut s = state();
        let plan = s.submit("chain base", &Config::default());
        assert!(matches!(plan, Plan::Done));
        let text = s.scrollback_text();
        assert!(text.contains("scope:auto> chain base"));
        assert!(text.contains("Chain pinned to: base"));
        assert_eq!(s.ctx.chain, "base");
    }

    #[test]
    fn test_ansi_colors_become_styles_not_escape_codes() {
        let mut s = state();
        s.push_ansi("\x1b[31mred\x1b[0m plain\n");
        assert_eq!(s.scrollback_text(), "red plain\n");
        let first = &s.scrollback[0].spans[0];
        assert_eq!(first.style.fg, Some(Color::Red));
    }

    #[test]
    fn test_scrollback_is_capped() {
        let mut s = state();
        let big: String = (0..SCROLLBACK_LIMIT + 50)
            .map(|i| format!("l{}\n", i))
            .collect();
        s.push_ansi(&big);
        assert_eq!(s.scrollback.len(), SCROLLBACK_LIMIT);
        assert!(s.scrollback_text().starts_with("l50\n"));
    }

    #[test]
    fn test_render_shows_latest_output_prompt_and_status() {
        let mut s = state();
        for i in 0..40 {
            s.push_ansi(&format!("line {}\n", i));
        }
        type_str(&mut s, "venues");
        let text = screen(&s, 60, 15);
        assert!(text.contains("line 39"), "{}", text);
        assert!(!text.contains("line 0 "), "{}", text);
        assert!(text.contains("scope:auto>"));
        assert!(text.contains("venues"));
        assert!(text.contains("chain auto"));

        s.handle_key(key(KeyCode::PageUp));
        let text = screen(&s, 60, 15);
        assert!(!text.contains("line 39"), "{}", text);
        assert!(text.contains("scrolled up 10 lines"));
    }

    #[test]
    fn test_render_shows_live_output_of_a_running_command() {
        // Output must appear while the command runs, not only when it ends.
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = state();
        let inv = Invocation {
            command: venues_list(),
            route: Route::Pane,
            ai: false,
            argv: vec![],
        };
        let out = s.start("market summary USDC --every 30s", &inv);
        crate::outln!(out, "\x1b[32mrun 1: HEALTHY\x1b[0m").unwrap();
        let text = screen(&s, 80, 15);
        assert!(text.contains("run 1: HEALTHY"), "{}", text);
        assert!(text.contains("running 0s · Esc cancels"), "{}", text);
        s.finish(Ending::Finished(Ok(())));
        assert!(s.scrollback_text().contains("run 1: HEALTHY"));
    }

    #[test]
    fn test_newest_line_is_on_the_last_pane_row() {
        // A gap under the newest output hides it behind blank rows on short
        // terminals.
        let mut s = state();
        for i in 0..40 {
            s.push_ansi(&format!("line {}\n", i));
        }
        let rows: Vec<String> = screen(&s, 40, 15).lines().map(str::to_string).collect();
        // Layout: pane rows 0..=9 (border at 0 and 9), input 10..=12, hint 13, status 14.
        assert!(rows[8].contains("line 39"), "{:#?}", rows);
        assert!(rows[9].starts_with('└'), "{:#?}", rows);
    }

    #[test]
    fn test_hint_row_shows_usage_while_typing() {
        let mut s = state();
        type_str(&mut s, "market summary");
        let text = screen(&s, 100, 15);
        assert!(text.contains("scope market summary"), "{}", text);
        assert!(text.contains("next: <SYMBOL>"), "{}", text);
    }

    #[test]
    fn test_render_tiny_terminal_does_not_panic() {
        let mut s = state();
        s.push_ansi("x\n");
        screen(&s, 10, 5);
        screen(&s, 1, 1);
    }

    #[tokio::test]
    async fn test_finish_with_error_shows_cli_error_text_and_logs() {
        let _lock = super::super::DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::at(dir.path().join("h"), dir.path().join("e.jsonl"));
        let mut s = TuiState::new(SessionContext::default(), journal, Vocab::default());
        let inv = Invocation {
            command: venues_list(),
            route: Route::Pane,
            ai: false,
            argv: vec!["scope".into(), "venues".into(), "list".into()],
        };
        s.start("venues list", &inv);
        s.finish(Ending::Finished(Err(
            scope::error::ScopeError::InvalidAddress("0xbad".into()),
        )));
        let text = s.scrollback_text();
        assert!(text.contains("Invalid address format: 0xbad"), "{}", text);
        assert!(text.contains("EVM: 0x followed by 40 hex characters"));
        let log = std::fs::read_to_string(dir.path().join("e.jsonl")).unwrap();
        let ev: serde_json::Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert_eq!(ev["outcome"], "error");
        assert_eq!(ev["route"], "pane");
        assert_eq!(ev["argv"][1], "venues");
    }
}
