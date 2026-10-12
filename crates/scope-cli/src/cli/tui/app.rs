//! # TUI State and Rendering
//!
//! [`TuiState`] holds everything the screen shows: the scrollback, the
//! input line, the history and the command that runs now. It does no
//! terminal I/O, so tests drive it with key events and render it on a
//! `TestBackend`.

use super::clip::{self, SaveError};
use super::exec::{self, Invocation, Plan, Route};
use super::fit;
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
use ratatui::widgets::{Block, Borders, Paragraph};
use scope::config::Config;
use scope::display::terminal::color_enabled;
use std::cell::Cell;
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

/// The most lines kept in the scrollback. Older lines are dropped.
pub const SCROLLBACK_LIMIT: usize = 5000;

/// Lines moved by one PageUp / PageDown.
const PAGE: usize = 10;

/// The most characters one paste adds to the input line.
pub const PASTE_LIMIT: usize = 10_000;

/// Bidirectional controls: they can disguise what the input line says.
const BIDI_CONTROLS: [std::ops::RangeInclusive<char>; 2] =
    ['\u{202A}'..='\u{202E}', '\u{2066}'..='\u{2069}'];

/// Said when Ctrl-D or `exit` is pressed while a command runs.
pub const BUSY_NOTE: &str = "A command is running; Esc or Ctrl-C cancels it first.";

/// The most completion candidates shown at once.
const MENU_ROWS: usize = 8;

/// A floating list over the screen.
pub enum Overlay {
    /// Completion candidates.
    Menu(Menu),
    /// The Ctrl-K command palette.
    Palette(super::palette::PaletteState),
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

/// Reads a line feed as Enter. Keys typed before raw mode is on pass
/// through the TTY's ICRNL, so Enter arrives as `\n`, which crossterm
/// reports as Ctrl-J (or `Char('\n')`). Issue #63.
fn line_feed_as_enter(key: KeyEvent) -> KeyEvent {
    let ctrl_j = key.code == KeyCode::Char('j') && key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl_j || key.code == KeyCode::Char('\n') {
        KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
            ..key
        }
    } else {
        key
    }
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
    /// Stop the process (Ctrl-Z); it comes back on `fg`.
    Suspend,
    /// Put this text on the clipboard (Ctrl-Y). The loop writes the OSC 52
    /// sequence through the host and notes the result.
    Copy(String),
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

/// How a command ended, in the pane or outside the TUI.
pub enum Ending {
    /// The handler returned.
    Finished(scope::error::Result<()>),
    /// The user cancelled it.
    Cancelled,
}

impl Ending {
    /// The journal outcome (`ok`, `error` or `cancelled`) and the error
    /// text.
    fn outcome(&self) -> (&'static str, Option<String>) {
        match self {
            Ending::Finished(Ok(())) => ("ok", None),
            Ending::Finished(Err(e)) => ("error", Some(e.to_string())),
            Ending::Cancelled => ("cancelled", None),
        }
    }
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
    /// The first display column of the input line that is shown. It moves
    /// only when the cursor leaves the window (set while drawing).
    scroll_x: Cell<usize>,
    /// The inner width of the output pane at the last draw; 0 before the
    /// first. The event loop makes it the width override.
    pane_width: Cell<u16>,
    /// Show the busy note in the hint row until the next key press.
    busy_hint: bool,
    /// The plain output of the last pane command that succeeded. Ctrl-Y
    /// copies it and `save` writes it.
    last_output: Option<String>,
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
            scroll_x: Cell::new(0),
            pane_width: Cell::new(0),
            busy_hint: false,
            last_output: None,
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
        let key = line_feed_as_enter(key);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        self.busy_hint = false;
        // Raw mode delivers Ctrl-Z as a key, not as SIGTSTP. It works in
        // every mode and keeps the draft and any overlay.
        if ctrl && key.code == KeyCode::Char('z') {
            return Action::Suspend;
        }
        // Ctrl-C closes the palette and keeps its normal meaning (cancel a
        // running command, clear the line), so a command can always be
        // stopped with one key.
        if ctrl
            && key.code == KeyCode::Char('c')
            && matches!(self.overlay, Some(Overlay::Palette(_)))
        {
            self.overlay = None;
        }
        if let Some(Overlay::Palette(p)) = &mut self.overlay {
            let count = p.matches().len();
            match key.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Enter => {
                    let fill = p
                        .matches()
                        .get(p.selected)
                        .map(|&i| p.items[i].fill.clone());
                    self.overlay = None;
                    if let Some(fill) = fill {
                        self.set_input(fill);
                    }
                }
                KeyCode::Down | KeyCode::Tab if count > 0 => p.selected = (p.selected + 1) % count,
                KeyCode::Up | KeyCode::BackTab if count > 0 => {
                    p.selected = (p.selected + count - 1) % count
                }
                KeyCode::Backspace => {
                    p.query.pop();
                    p.selected = 0;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    p.query.push(c);
                    p.selected = 0;
                }
                _ => {}
            }
            return Action::None;
        }
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
            KeyCode::Char('y') if ctrl => return self.copy_key(),
            KeyCode::Char('d') if ctrl => {
                if self.running.is_some() {
                    self.busy_hint = true;
                    self.push_busy_note();
                } else if self.input.is_empty() {
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
                    if !self.input.trim().is_empty() {
                        self.busy_hint = true;
                        if self.input_is_exit() {
                            self.push_busy_note();
                        }
                    }
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
            KeyCode::Char('k') if ctrl => {
                self.overlay = Some(Overlay::Palette(super::palette::PaletteState::new(
                    super::palette::items(&self.history),
                )));
            }
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

    /// Inserts pasted text at the cursor as one line. A paste never runs
    /// the line, also when it ends in a newline. Line breaks become
    /// spaces, a tab becomes a space, and other control characters are
    /// dropped, so a paste cannot put an escape sequence in the input.
    pub fn paste(&mut self, text: &str) {
        let unified = text.replace("\r\n", "\n").replace('\r', "\n");
        let body = unified.strip_suffix('\n').unwrap_or(&unified);
        let lines = body.split('\n').count();
        let mut clean: String = body
            .replace('\n', " ")
            .chars()
            .map(|c| if c == '\t' { ' ' } else { c })
            .filter(|c| !c.is_control() && !BIDI_CONTROLS.iter().any(|r| r.contains(c)))
            .collect();
        if clean.is_empty() {
            return;
        }
        let truncated = clean.chars().count() > PASTE_LIMIT;
        if truncated {
            clean = clean.chars().take(PASTE_LIMIT).collect();
        }
        if let Some(Overlay::Palette(p)) = &mut self.overlay {
            // The palette has the focus: the paste goes into its query.
            p.query.push_str(&clean);
            p.selected = 0;
        } else {
            self.overlay = None;
            let at = self.byte_index(self.cursor);
            self.input.insert_str(at, &clean);
            self.cursor += clean.chars().count();
        }
        if lines > 1 {
            self.push_note(&format!("pasted {} lines as one line", lines));
        }
        if truncated {
            self.push_note(&format!("paste truncated to {} characters", PASTE_LIMIT));
        }
    }

    /// Picks the first shown column of the input line for a window of
    /// `width` columns. It moves only when the cursor would leave the
    /// window, and it always sits on a character boundary, so a wide
    /// character is never cut at the left edge or the cursor.
    fn input_scroll(&self, width: usize) -> usize {
        let mut bounds = vec![0usize];
        let mut col = 0;
        for c in self.input.chars() {
            col += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            bounds.push(col);
        }
        let total = col;
        let cursor_col = self.cursor_column();
        let cursor_w = self
            .input
            .chars()
            .nth(self.cursor)
            .map_or(1, |c| {
                unicode_width::UnicodeWidthChar::width(c)
                    .unwrap_or(0)
                    .max(1)
            })
            .min(width);
        let down = |x: usize| bounds.iter().copied().rfind(|&b| b <= x).unwrap_or(0);
        let up = |x: usize| bounds.iter().copied().find(|&b| b >= x).unwrap_or(total);
        let mut scroll = down(self.scroll_x.get().min((total + 1).saturating_sub(width)));
        if cursor_col < scroll {
            scroll = cursor_col;
        }
        if cursor_col + cursor_w > scroll + width {
            scroll = up(cursor_col + cursor_w - width);
        }
        self.scroll_x.set(scroll);
        scroll
    }

    /// The display column of the cursor: wide characters count as two.
    fn cursor_column(&self) -> usize {
        UnicodeWidthStr::width(&self.input[..self.byte_index(self.cursor)])
    }

    /// True when the line is an exit command (the same words `exec` takes).
    fn input_is_exit(&self) -> bool {
        self.input.split_whitespace().next().is_some_and(|w| {
            matches!(
                w.to_lowercase().as_str(),
                "exit" | "quit" | "q" | ".exit" | ".quit"
            )
        })
    }

    /// Says why exit waits. Does not repeat when the last line is the note.
    fn push_busy_note(&mut self) {
        let last = self.scrollback.last().map(|l| l.to_string());
        if last.as_deref() != Some(BUSY_NOTE) {
            self.push_note(BUSY_NOTE);
        }
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

    /// Appends a note line in `style`.
    fn push_styled_note(&mut self, note: &str, style: Style) {
        self.push_lines(vec![Line::styled(note.to_string(), style)]);
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
        if line
            .split_whitespace()
            .next()
            .is_some_and(|w| w.eq_ignore_ascii_case("save"))
        {
            let words = shlex::split(line);
            self.save(line, words.as_ref().map(|w| &w[1..]));
            return Plan::Done;
        }
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
                outcome: "session".into(),
                error: None,
            });
        }
        plan
    }

    /// Journals a save or copy: `session` when it worked, `refused` when
    /// the request was turned down, `error` when the OS failed it.
    fn log_action(
        &mut self,
        input: &str,
        started: Instant,
        outcome: &'static str,
        error: Option<String>,
    ) {
        self.log(Event {
            started: Utc::now().to_rfc3339(),
            duration_ms: started.elapsed().as_millis(),
            input,
            argv: &[],
            route: "session",
            outcome: outcome.into(),
            error,
        });
    }

    /// Runs the `save` session command over the last output. `words` is
    /// the split line, or `None` when the quotes did not balance.
    fn save(&mut self, line: &str, words: Option<&[String]>) {
        let started = Instant::now();
        let result = match words {
            None => Err(SaveError::Refused(format!(
                "Unbalanced quotes. {}",
                clip::SAVE_USAGE
            ))),
            Some(args) => {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
                match std::env::current_dir() {
                    Ok(cwd) => {
                        clip::save(&args, self.last_output.as_deref(), &cwd, home.as_deref())
                    }
                    Err(e) => Err(SaveError::Failed(format!(
                        "Could not read the current directory: {}",
                        e
                    ))),
                }
            }
        };
        match result {
            Ok(note) => {
                self.push_note(&note);
                self.log_action(line, started, "session", None);
            }
            Err(e) => {
                self.push_note(e.message());
                let outcome = match e {
                    SaveError::Refused(_) => "refused",
                    SaveError::Failed(_) => "error",
                };
                self.log_action(line, started, outcome, Some(e.message().to_string()));
            }
        }
    }

    /// Ctrl-Y: returns the copy action, or notes and journals why there
    /// is nothing to send.
    fn copy_key(&mut self) -> Action {
        let started = Instant::now();
        let refusal = match self.last_output.clone() {
            None => "nothing to copy yet".to_string(),
            Some(text) if clip::fits_osc52(&text) => return Action::Copy(text),
            Some(text) => format!(
                "too large to copy ({} lines); use save <file>",
                text.lines().count()
            ),
        };
        self.push_note(&refusal);
        self.log_action("Ctrl-Y", started, "refused", Some(refusal));
        Action::None
    }

    /// Notes and journals the result of writing the copy sequence.
    pub fn copied(&mut self, lines: usize, result: std::io::Result<()>) {
        let started = Instant::now();
        match result {
            Ok(()) => {
                self.push_note(&format!("copied {} lines", lines));
                self.log_action("Ctrl-Y", started, "session", None);
            }
            Err(e) => {
                let note = format!("Could not copy: {}", e);
                self.push_note(&note);
                self.log_action("Ctrl-Y", started, "error", Some(note));
            }
        }
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
        self.finish_with(ending, color_enabled());
    }

    /// `finish`, with the color choice given. The failure footer always
    /// carries the `✗` glyph and its words; `color` only adds the red.
    fn finish_with(&mut self, ending: Ending, color: bool) {
        let Some(run) = self.running.take() else {
            return;
        };
        let (outcome, error) = ending.outcome();
        if let Ending::Finished(Err(e)) = &ending {
            // Same text and hints as the CLI prints on failure.
            if let Err(write_err) = crate::cli::errors::display_error(e, &run.out) {
                self.push_note(&format!("Output error: {}", write_err));
            }
        }
        let output = run.captured.out();
        // The copy is the command's own data: no warnings, error text or
        // footer. A success with no data clears it; a failed or cancelled
        // run keeps the earlier copy.
        if matches!(ending, Ending::Finished(Ok(()))) {
            let data = clip::strip_ansi(&run.captured.data());
            self.last_output = (!data.trim().is_empty()).then_some(data);
        }
        self.push_ansi(&output);
        let secs = run.started.elapsed().as_secs_f64();
        match &ending {
            Ending::Cancelled => self.push_note(&format!("Cancelled after {:.1}s.", secs)),
            Ending::Finished(Err(_)) => {
                let style = if color {
                    Style::new().fg(Color::Red)
                } else {
                    Style::new().add_modifier(Modifier::BOLD)
                };
                self.push_styled_note(&format!("✗ Failed in {:.1}s.", secs), style);
            }
            Ending::Finished(Ok(())) => self.push_note(&format!("Done in {:.1}s.", secs)),
        }
        self.log(Event {
            started: run.started_at,
            duration_ms: run.started.elapsed().as_millis(),
            input: &run.input,
            argv: &run.argv,
            route: "pane",
            outcome: outcome.into(),
            error,
        });
    }

    /// Records a command that ran outside the TUI (suspend mode), with a
    /// note such as `Ran outside the TUI: crawl USDC (cancelled, 3.2s).`
    pub fn record_suspended(
        &mut self,
        input: &str,
        inv_argv: &[String],
        duration: std::time::Duration,
        started_at: String,
        ending: &Ending,
    ) {
        let (outcome, error) = ending.outcome();
        // The note says "failed"; the journal keeps "error".
        let word = if outcome == "error" {
            "failed"
        } else {
            outcome
        };
        self.push_note(&format!(
            "Ran outside the TUI: {} ({}, {:.1}s).",
            input,
            word,
            duration.as_secs_f64()
        ));
        self.log(Event {
            started: started_at,
            duration_ms: duration.as_millis(),
            input,
            argv: inv_argv,
            route: "suspend",
            outcome: outcome.into(),
            error,
        });
    }

    /// Records that signal `name` (for example `TERM`) ended the TUI.
    /// `route` is `session`, or `suspend` with the suspend-mode command
    /// that was running (`input`, `argv`).
    pub fn record_signal(&mut self, name: &str, route: &'static str, input: &str, argv: &[String]) {
        self.log(Event {
            started: Utc::now().to_rfc3339(),
            duration_ms: 0,
            input,
            argv,
            route,
            outcome: format!("signal:{}", name).into(),
            error: None,
        });
    }

    fn log(&mut self, event: Event<'_>) {
        if let Err(e) = self.journal.record(&event) {
            self.push_note(&format!("Could not write the TUI event log: {}", e));
        }
    }

    // -------------------------------------------------------------- render

    /// The inner width of the output pane at the last draw, or `None`
    /// before the first draw. Command output is fitted to it.
    pub fn pane_width(&self) -> Option<usize> {
        match self.pane_width.get() {
            0 => None,
            w => Some(w as usize),
        }
    }

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
        let block = Block::new()
            .borders(Borders::TOP | Borders::BOTTOM)
            .title(format!(" scope v{} ", scope::VERSION));
        let inner = block.inner(pane);
        self.pane_width.set(inner.width);
        // Fit the lines to the inner width here, not with ratatui wrap: a
        // wrapped row keeps its gutter and a rule is cut. Every line then
        // is one row, so the newest line sits on the last inner row.
        let para = Paragraph::new(Text::from(fit::fit(lines, inner.width as usize)));
        let total = para.line_count(inner.width);
        let max_top = total.saturating_sub(inner.height as usize);
        let top = max_top.saturating_sub(self.scroll.min(max_top));
        f.render_widget(para.block(block).scroll((top as u16, 0)), pane);

        // Input line with the prompt in the border title.
        let block = Block::bordered().title(self.prompt());
        // Scroll the line so the cursor stays on the last inner column.
        let inner_width = input.width.saturating_sub(2).max(1) as usize;
        let col = self.cursor_column();
        let scroll_x = self.input_scroll(inner_width);
        f.render_widget(
            Paragraph::new(self.input.as_str())
                .block(block)
                .scroll((0, scroll_x.min(u16::MAX as usize) as u16)),
            input,
        );
        let cursor_x = input.x + 1 + (col - scroll_x).min(u16::MAX as usize) as u16;
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
                .map(|c| UnicodeWidthStr::width(c.as_str()))
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

        if self.busy_hint {
            f.render_widget(
                Paragraph::new(Line::styled(
                    format!(" {}", BUSY_NOTE),
                    Style::new().fg(Color::Yellow),
                )),
                hint_row,
            );
        } else if let Some(h) = super::hint::hint(&self.input) {
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
        if let Some(Overlay::Palette(p)) = &self.overlay {
            let area = f.area();
            let width = area.width.saturating_sub(4).min(80);
            let height = area.height.saturating_sub(4).min(16);
            let rect = ratatui::layout::Rect {
                x: area.x + (area.width.saturating_sub(width)) / 2,
                y: area.y + 1,
                width,
                height,
            };
            let rows = height.saturating_sub(3) as usize;
            let matches = p.matches();
            let first = p.selected.saturating_sub(rows.saturating_sub(1));
            let mut lines = vec![Line::from(format!("> {}", p.query))];
            for (n, &i) in matches.iter().enumerate().skip(first).take(rows) {
                let it = &p.items[i];
                let style = if n == p.selected {
                    Style::new().add_modifier(Modifier::REVERSED)
                } else {
                    Style::new()
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("{:<28}", it.label), style),
                    Span::styled(
                        format!(" {}", it.description),
                        Style::new().fg(Color::DarkGray),
                    ),
                ]));
            }
            f.render_widget(ratatui::widgets::Clear, rect);
            f.render_widget(
                Paragraph::new(lines).block(Block::bordered().title(" Commands (Ctrl-K) ")),
                rect,
            );
        }
        f.render_widget(
            Paragraph::new(fit_status(&self.status_parts(), status.width as usize)),
            status,
        );
    }

    /// What the status bar says now, before it is fitted to a width.
    fn status_parts(&self) -> StatusParts {
        let ctx = &self.ctx;
        let limit = format!(
            "limit {}{}{}",
            ctx.limit,
            if ctx.include_tokens {
                " · +tokens"
            } else {
                ""
            },
            if ctx.include_txs { " · +txs" } else { "" },
        );
        // Highest priority first: the fit drops from the end.
        let context = vec![
            format!("chain {}", ctx.chain),
            format!("format {:?}", ctx.format),
            limit,
        ];
        let (state, command) = match &self.running {
            Some(run) => (
                format!(
                    "running {:.0}s · Esc cancels",
                    run.started.elapsed().as_secs_f64()
                ),
                Some(run.input.clone()),
            ),
            None if self.scroll > 0 => (format!("scrolled up {} lines · PgDn", self.scroll), None),
            None => ("help · exit".to_string(), None),
        };
        StatusParts {
            context,
            state,
            command,
        }
    }
}

/// The parts of the status bar, in priority order.
struct StatusParts {
    /// Chain, format and limit. Dropped from the end when the bar is narrow.
    context: Vec<String>,
    /// `running Ns · Esc cancels`, `scrolled up N lines · PgDn` or
    /// `help · exit`. Kept at every width.
    state: String,
    /// The command that runs. The last part to go.
    command: Option<String>,
}

/// Marks text that was cut or dropped.
const ELLIPSIS: &str = "…";

/// Columns around a command in the bar: ` · ` before it, a backtick on
/// each side.
const COMMAND_FRAME_COLS: usize = 5;

/// The shortest cut command worth showing: one column and the `…`.
const MIN_CUT_COMMAND_COLS: usize = 2;

fn cols(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Cuts `text` to at most `max` display columns, ending in `…` when it cuts.
fn cut_with_ellipsis(text: &str, max: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    if cols(text) <= max {
        return text.to_string();
    }
    let room = max.saturating_sub(cols(ELLIPSIS));
    let mut out = String::new();
    let mut used = 0;
    for g in text.graphemes(true) {
        let w = cols(g);
        if used + w > room {
            break;
        }
        out.push_str(g);
        used += w;
    }
    if max >= cols(ELLIPSIS) {
        out.push_str(ELLIPSIS);
    }
    out
}

/// Fits the status bar to `width` display columns. Drops limit, then
/// format, then chain, then the command text. The state text stays at
/// every width, and a `…` shows that something was cut or dropped.
fn fit_status(parts: &StatusParts, width: usize) -> Line<'static> {
    let dim = Style::new().fg(Color::DarkGray);
    let hot = Style::new().fg(Color::Yellow);
    let bar = |left: String, right: String| {
        Line::from(vec![Span::styled(left, dim), Span::styled(right, hot)])
    };
    if width == 0 {
        return Line::default();
    }
    let command = |c: &str| format!(" · `{}`", c);
    let total = parts.context.len();
    for kept in (0..=total).rev() {
        let left = if kept == 0 {
            " ".to_string()
        } else {
            format!(" {}  ", parts.context[..kept].join(" · "))
        };
        let mut right = parts.state.clone();
        if let Some(c) = &parts.command {
            right.push_str(&command(c));
        }
        if kept < total {
            right.push_str(&format!(" {}", ELLIPSIS));
        }
        if cols(&left) + cols(&right) <= width {
            return bar(left, right);
        }
    }
    // The whole context is gone. Cut the command text to what is left.
    let used = 1 + cols(&parts.state) + COMMAND_FRAME_COLS;
    if let Some(c) = &parts.command
        && width >= used + MIN_CUT_COMMAND_COLS
    {
        let cut = cut_with_ellipsis(c, width - used);
        return bar(" ".to_string(), format!("{}{}", parts.state, command(&cut)));
    }
    // The command is gone too. A `…` after the state shows it, if it fits.
    let text = format!(" {} {}", parts.state, ELLIPSIS);
    if cols(&text) <= width {
        return bar(String::new(), text);
    }
    bar(
        String::new(),
        cut_with_ellipsis(&format!(" {}", parts.state), width),
    )
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

    fn cursor_pos(s: &TuiState, w: u16, h: u16) -> Position {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| s.render(f)).unwrap();
        term.get_cursor_position().unwrap()
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

    fn ctrl_k() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL)
    }

    #[test]
    fn test_ctrl_k_opens_palette_and_enter_fills_input_without_running() {
        let mut s = state();
        s.handle_key(ctrl_k());
        assert!(matches!(s.overlay, Some(Overlay::Palette(_))));
        type_str(&mut s, "msum");
        assert_eq!(s.handle_key(key(KeyCode::Enter)), Action::None);
        assert_eq!(s.input, "market summary ");
        assert!(s.overlay.is_none());
    }

    #[test]
    fn test_ctrl_c_in_palette_still_cancels_a_running_command() {
        // Review I1: the palette must not swallow the cancel key.
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = state();
        let inv = Invocation {
            command: venues_list(),
            route: Route::Pane,
            ai: false,
            argv: vec![],
        };
        s.start("venues list", &inv);
        s.handle_key(ctrl_k());
        assert_eq!(s.handle_key(ctrl('c')), Action::Cancel);
        assert!(s.overlay.is_none(), "Ctrl-C also closes the palette");
        s.finish(Ending::Cancelled);
    }

    #[test]
    fn test_palette_escape_keeps_the_input() {
        let mut s = state();
        type_str(&mut s, "draft");
        s.handle_key(ctrl_k());
        type_str(&mut s, "zzz");
        s.handle_key(key(KeyCode::Esc));
        assert!(s.overlay.is_none());
        assert_eq!(s.input, "draft");
    }

    #[test]
    fn test_palette_with_long_history_renders_bounded() {
        // Review Focus 4.
        let mut s = state();
        for i in 0..60 {
            type_str(&mut s, &format!("venues list {}", i));
            s.handle_key(key(KeyCode::Enter));
        }
        s.handle_key(ctrl_k());
        for _ in 0..30 {
            s.handle_key(key(KeyCode::Down));
        }
        screen(&s, 80, 20);
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

    /// Issue #63: keys typed while the TTY is still in cooked mode pass
    /// through ICRNL, so Enter reaches raw mode as a line feed, which
    /// crossterm reports as Ctrl-J. It must still run the line, not be
    /// dropped or typed into the input.
    #[test]
    fn test_line_feed_from_cooked_mode_submits_like_enter() {
        for lf in [ctrl('j'), key(KeyCode::Char('\n'))] {
            let mut s = state();
            type_str(&mut s, "ctx");
            assert_eq!(s.handle_key(lf), Action::Submit("ctx".into()), "{lf:?}");
            assert_eq!(s.input, "");
        }
    }

    /// The line feed acts as Enter in the palette too: it picks the entry
    /// instead of typing a newline into the query.
    #[test]
    fn test_line_feed_picks_palette_entry_like_enter() {
        let mut s = state();
        s.handle_key(ctrl('k'));
        s.handle_key(ctrl('j'));
        assert!(s.overlay.is_none());
        assert!(!s.input.contains('\n'));
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
    fn test_ctrl_z_suspends_and_keeps_the_draft_and_the_palette() {
        // Raw mode turns Ctrl-Z into a key, so the TUI must act on it, in
        // every mode. The draft and the open palette stay for after `fg`.
        let mut s = state();
        type_str(&mut s, "draf");
        assert_eq!(s.handle_key(ctrl('z')), Action::Suspend);
        assert_eq!(s.input, "draf");
        s.handle_key(ctrl('k'));
        assert_eq!(s.handle_key(ctrl('z')), Action::Suspend);
        assert!(matches!(s.overlay, Some(Overlay::Palette(_))));
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
        assert!(s.scrollback_text().contains("Cancelled after"));
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

    /// The status bar row (the last row) of a screen.
    fn status_row(s: &TuiState, w: u16) -> String {
        screen(s, w, 12).lines().last().unwrap().to_string()
    }

    fn running_state(input: &str) -> (TuiState, Output) {
        let mut s = state();
        let inv = Invocation {
            command: venues_list(),
            route: Route::Pane,
            ai: false,
            argv: vec![],
        };
        let out = s.start(input, &inv);
        (s, out)
    }

    #[test]
    fn test_status_bar_keeps_the_running_state_at_every_width() {
        // The cancel hint is the only way out of a stuck command: no width
        // may hide it.
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let (s, _out) = running_state("market summary USDC --venue binance --every 2s");
        for w in [120, 60, 40, 30] {
            let row = status_row(&s, w);
            assert!(row.contains("running "), "{w}: {row:?}");
            assert!(row.contains("Esc cancels"), "{w}: {row:?}");
        }
        let wide = status_row(&s, 120);
        assert!(
            wide.contains("limit") && wide.contains("format"),
            "{wide:?}"
        );
        assert!(wide.contains("market summary USDC"), "{wide:?}");
    }

    #[test]
    fn test_status_bar_drops_limit_and_format_before_the_cancel_hint() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let (s, _out) = running_state("market summary USDC --venue binance --every 2s");
        let row = status_row(&s, 40);
        assert!(!row.contains("limit"), "{row:?}");
        assert!(!row.contains("format"), "{row:?}");
        // Something was cut, and the bar says so.
        assert!(row.contains('…'), "{row:?}");
    }

    #[test]
    fn test_idle_status_bar_keeps_help_at_a_narrow_width() {
        let s = state();
        let row = status_row(&s, 30);
        assert!(row.contains("help"), "{row:?}");
        assert!(!row.contains("limit"), "{row:?}");
    }

    #[test]
    fn test_scrolled_status_bar_keeps_the_scroll_hint_at_a_narrow_width() {
        let mut s = state();
        for i in 0..40 {
            s.push_ansi(&format!("line {}\n", i));
        }
        s.handle_key(key(KeyCode::PageUp));
        let row = status_row(&s, 40);
        assert!(row.contains("scrolled up 10 lines"), "{row:?}");
    }

    #[test]
    fn test_fit_status_never_panics_and_never_exceeds_the_width() {
        // Width math runs on every redraw: width 0 and tiny widths must
        // give a line, and no line may be wider than the bar.
        let parts = StatusParts {
            context: vec![
                "chain auto".into(),
                "format Table".into(),
                "limit 100".into(),
            ],
            state: "running 12s · Esc cancels".into(),
            command: Some("market summary ✓✓ --every 2s".into()),
        };
        for w in 0..=130 {
            let line = fit_status(&parts, w);
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(cols(&text) <= w, "{w}: {text:?}");
        }
        let text: String = fit_status(&parts, 0)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.is_empty());
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
        assert!(rows[9].starts_with('─'), "{:#?}", rows);
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

    fn palette_selected(s: &TuiState) -> usize {
        let Some(Overlay::Palette(p)) = &s.overlay else {
            panic!("palette expected")
        };
        p.selected
    }

    #[test]
    fn test_palette_selection_wraps_and_a_new_query_resets_it() {
        // Down/Up must wrap, or the top and bottom rows are unreachable by
        // keyboard. Editing the query must restart at the best match.
        let mut s = state();
        s.handle_key(ctrl_k());
        let count = match &s.overlay {
            Some(Overlay::Palette(p)) => p.matches().len(),
            _ => panic!("palette expected"),
        };
        assert!(count > 2);
        s.handle_key(key(KeyCode::Up));
        assert_eq!(palette_selected(&s), count - 1);
        s.handle_key(key(KeyCode::Down));
        assert_eq!(palette_selected(&s), 0);
        s.handle_key(key(KeyCode::Tab));
        s.handle_key(key(KeyCode::BackTab));
        assert_eq!(palette_selected(&s), 0);
        s.handle_key(key(KeyCode::Down));
        s.handle_key(key(KeyCode::Backspace));
        assert_eq!(palette_selected(&s), 0);
        s.handle_key(key(KeyCode::Down));
        type_str(&mut s, "m");
        assert_eq!(palette_selected(&s), 0);
    }

    #[test]
    fn test_palette_esc_closes_and_enter_without_a_match_keeps_the_draft() {
        let mut s = state();
        type_str(&mut s, "draft");
        s.handle_key(ctrl_k());
        s.handle_key(key(KeyCode::Esc));
        assert!(s.overlay.is_none());
        s.handle_key(ctrl_k());
        type_str(&mut s, "zzzzqqqq");
        s.handle_key(key(KeyCode::Down));
        s.handle_key(key(KeyCode::Enter));
        assert!(s.overlay.is_none());
        assert_eq!(s.input, "draft", "no match: nothing is filled in");
    }

    #[test]
    fn test_menu_up_wraps_to_the_last_candidate() {
        let mut s = vocab_state();
        type_str(&mut s, "market summary USDC --venue b");
        s.handle_key(key(KeyCode::Tab));
        s.handle_key(key(KeyCode::Up));
        s.handle_key(key(KeyCode::Enter));
        assert_eq!(s.input, "market summary USDC --venue bitget ");
        s.set_input("market summary USDC --venue b".to_string());
        s.handle_key(key(KeyCode::Tab));
        s.handle_key(key(KeyCode::Down));
        s.handle_key(key(KeyCode::BackTab));
        s.handle_key(key(KeyCode::Enter));
        assert_eq!(s.input, "market summary USDC --venue binance ");
    }

    #[test]
    fn test_record_signal_writes_the_signal_outcome_to_the_journal() {
        // An RCA of a killed session reads this row: which signal, and
        // which suspend-mode command it interrupted.
        let dir = tempfile::tempdir().unwrap();
        let events = dir.path().join("e.jsonl");
        let mut s = TuiState::new(
            SessionContext::default(),
            Journal::at(dir.path().join("h"), events.clone()),
            Vocab::default(),
        );
        s.record_signal("HUP", "suspend", "setup", &["scope".into(), "setup".into()]);
        let row: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(events).unwrap().trim()).unwrap();
        assert_eq!(row["outcome"], "signal:HUP");
        assert_eq!(row["route"], "suspend");
        assert_eq!(row["input"], "setup");
        assert_eq!(row["argv"][1], "setup");
    }

    #[test]
    fn test_failed_suspended_command_is_journaled_as_error_with_its_text() {
        let dir = tempfile::tempdir().unwrap();
        let events = dir.path().join("e.jsonl");
        let mut s = TuiState::new(
            SessionContext::default(),
            Journal::at(dir.path().join("h"), events.clone()),
            Vocab::default(),
        );
        let failed = Err(scope::error::ScopeError::Other("setup broke".into()));
        s.record_suspended(
            "setup",
            &["scope".into(), "setup".into()],
            std::time::Duration::from_millis(5),
            "2026-01-01T00:00:00Z".into(),
            &Ending::Finished(failed),
        );
        let row: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(events).unwrap().trim()).unwrap();
        assert_eq!(row["outcome"], "error");
        assert_eq!(row["route"], "suspend");
        assert!(row["error"].as_str().unwrap().contains("setup broke"));
    }

    #[test]
    fn test_unwritable_event_log_is_reported_in_the_pane() {
        // A journal that cannot be written must say so, not lose events
        // silently (the journal is the RCA record).
        let dir = tempfile::tempdir().unwrap();
        // The events path is a directory, so appending to it fails.
        let mut s = TuiState::new(
            SessionContext::default(),
            Journal::at(dir.path().join("h"), dir.path().to_path_buf()),
            Vocab::default(),
        );
        s.record_signal("TERM", "session", "", &[]);
        assert!(
            s.scrollback_text()
                .contains("Could not write the TUI event log"),
            "{}",
            s.scrollback_text()
        );
    }

    #[test]
    fn test_paste_joins_lines_and_never_runs() {
        let mut s = state();
        s.paste("venues list\nmarket summary USDC\n");
        assert_eq!(s.input, "venues list market summary USDC");
        assert!(s.scrollback_text().contains("pasted 2 lines as one line"));
    }

    #[test]
    fn test_paste_one_line_adds_no_note_and_crlf_counts_once() {
        let mut s = state();
        s.paste("help\r\n");
        assert_eq!(s.input, "help");
        assert!(!s.scrollback_text().contains("pasted"));
        s.set_input(String::new());
        s.paste("a\r\nb\rc");
        assert_eq!(s.input, "a b c");
        assert!(s.scrollback_text().contains("pasted 3 lines as one line"));
    }

    #[test]
    fn test_paste_inserts_at_the_cursor() {
        let mut s = state();
        type_str(&mut s, "ac");
        s.handle_key(key(KeyCode::Left));
        s.paste("b");
        assert_eq!(s.input, "abc");
        type_str(&mut s, "!");
        assert_eq!(s.input, "ab!c", "the cursor sits after the paste");
    }

    #[test]
    fn test_paste_cannot_inject_control_characters() {
        let mut s = state();
        s.paste("a\x1b[2Jb\tc\x07\x7fd\u{9b}e");
        assert_eq!(s.input, "a[2Jb cde");
    }

    #[test]
    fn test_paste_closes_the_completion_menu() {
        let mut s = vocab_state();
        type_str(&mut s, "market summary X --venue b");
        s.handle_key(key(KeyCode::Tab));
        assert!(s.overlay.is_some());
        s.paste("z");
        assert!(s.overlay.is_none());
    }

    #[test]
    fn test_paste_while_a_command_runs_goes_to_the_input() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = state();
        let inv = Invocation {
            command: venues_list(),
            route: Route::Pane,
            ai: false,
            argv: vec![],
        };
        s.start("venues list", &inv);
        s.paste("market summary USDC\n");
        assert_eq!(s.input, "market summary USDC");
        assert!(s.running.is_some(), "the paste does not run or cancel");
        s.finish(Ending::Cancelled);
    }

    #[test]
    fn test_long_input_keeps_cursor_visible() {
        let mut s = state();
        s.paste(&"x".repeat(300));
        // 60 columns: the inner width is 58, and the cursor is the last cell.
        assert_eq!(cursor_pos(&s, 60, 12).x, 58);
        let sc = screen(&s, 60, 12);
        let row = sc.lines().find(|l| l.contains("xxxx")).unwrap();
        let cells: Vec<char> = row.chars().collect();
        assert_eq!(cells[0], '│');
        assert!(
            cells[1..58].iter().all(|&c| c == 'x'),
            "text ends at the cursor"
        );
        assert_eq!(cells[58], ' ', "the cursor cell is empty");
        assert_eq!(cells[59], '│', "the border is not overwritten");
    }

    #[test]
    fn test_cursor_scrolls_back_with_home() {
        let mut s = state();
        s.paste(&"x".repeat(100));
        s.handle_key(key(KeyCode::Home));
        assert_eq!(
            cursor_pos(&s, 60, 12).x,
            1,
            "Home shows the start of the line"
        );
        s.handle_key(key(KeyCode::End));
        assert_eq!(cursor_pos(&s, 60, 12).x, 58);
    }

    #[test]
    fn test_cursor_follows_display_width_of_cjk() {
        // 日本 is 4 columns, then a space and "ok": the cursor is at 1 + 7.
        let mut s = state();
        type_str(&mut s, "日本 ok");
        s.handle_key(key(KeyCode::End));
        assert_eq!(cursor_pos(&s, 60, 12).x, 1 + 7);
        s.handle_key(key(KeyCode::Left));
        s.handle_key(key(KeyCode::Left));
        s.handle_key(key(KeyCode::Left));
        assert_eq!(cursor_pos(&s, 60, 12).x, 1 + 4, "after 日本");
    }

    #[test]
    fn test_cursor_follows_display_width_of_emoji() {
        let mut s = state();
        type_str(&mut s, "😀x");
        assert_eq!(cursor_pos(&s, 60, 12).x, 1 + 3);
    }

    #[test]
    fn test_wide_input_scrolls_by_columns_not_chars() {
        // 40 CJK chars are 80 columns; the cursor stays on the last inner column.
        let mut s = state();
        s.paste(&"日".repeat(40));
        // 80 columns; the window starts on a boundary at or after 23.
        assert_eq!(cursor_pos(&s, 60, 12).x, 57);
    }

    #[test]
    fn test_menu_width_uses_display_width() {
        let wide_a = "日本日本日本a";
        let wide_b = "日本日本日本b";
        let mut s = TuiState::new(
            SessionContext::default(),
            Journal::default(),
            Vocab {
                venues: vec![wide_a.into(), wide_b.into()],
                ..Vocab::default()
            },
        );
        type_str(&mut s, "market summary X --venue ");
        s.handle_key(key(KeyCode::Tab));
        assert!(matches!(s.overlay, Some(Overlay::Menu(_))));
        let mut term = Terminal::new(TestBackend::new(60, 14)).unwrap();
        term.draw(|f| s.render(f)).unwrap();
        let buf = term.backend().buffer().clone();
        // Each candidate is 13 columns wide: the box is 17 columns, so the
        // right border is at x = 16.
        let row = (0..14)
            .find(|&y| (0..60).any(|x| buf[(x, y)].symbol() == "a"))
            .unwrap();
        assert_eq!(buf[(16, row)].symbol(), "│");
    }

    fn start_pane_command(s: &mut TuiState) {
        let inv = Invocation {
            command: venues_list(),
            route: Route::Pane,
            ai: false,
            argv: vec![],
        };
        s.start("venues list", &inv);
    }

    #[test]
    fn test_ctrl_d_while_running_explains_and_does_not_exit() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = state();
        start_pane_command(&mut s);
        assert_eq!(s.handle_key(ctrl('d')), Action::None);
        assert!(s.scrollback_text().contains(BUSY_NOTE));
        s.finish(Ending::Cancelled);
    }

    #[test]
    fn test_exit_typed_while_running_explains_and_keeps_the_line() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        for word in ["exit", "quit", "q", "EXIT"] {
            let mut s = state();
            start_pane_command(&mut s);
            type_str(&mut s, word);
            assert_eq!(s.handle_key(key(KeyCode::Enter)), Action::None, "{word}");
            assert!(s.scrollback_text().contains(BUSY_NOTE), "{word}");
            assert_eq!(s.input, word, "the line stays");
            s.finish(Ending::Cancelled);
        }
    }

    #[test]
    fn test_any_line_while_running_shows_the_hint_and_keeps_the_input() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = state();
        start_pane_command(&mut s);
        type_str(&mut s, "ctx");
        assert_eq!(s.handle_key(key(KeyCode::Enter)), Action::None);
        assert_eq!(s.input, "ctx");
        assert!(screen(&s, 100, 14).contains(BUSY_NOTE), "hint row");
        assert!(
            !s.scrollback_text().contains(BUSY_NOTE),
            "no scrollback note"
        );
        s.finish(Ending::Cancelled);
    }

    #[test]
    fn test_empty_enter_while_running_shows_no_hint() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = state();
        start_pane_command(&mut s);
        s.handle_key(key(KeyCode::Enter));
        assert!(!screen(&s, 100, 14).contains(BUSY_NOTE));
        s.finish(Ending::Cancelled);
    }

    #[test]
    fn test_busy_hint_clears_on_the_next_key() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = state();
        start_pane_command(&mut s);
        s.handle_key(ctrl('d'));
        let hint_row = screen(&s, 100, 14).lines().nth(12).unwrap().to_string();
        assert!(hint_row.contains(BUSY_NOTE));
        s.handle_key(key(KeyCode::Char('x')));
        let hint_row = screen(&s, 100, 14).lines().nth(12).unwrap().to_string();
        assert!(!hint_row.contains(BUSY_NOTE));
        s.finish(Ending::Cancelled);
    }

    #[test]
    fn test_busy_note_is_not_repeated_in_a_row() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = state();
        start_pane_command(&mut s);
        s.handle_key(ctrl('d'));
        s.handle_key(ctrl('d'));
        assert_eq!(s.scrollback_text().matches(BUSY_NOTE).count(), 1);
        s.finish(Ending::Cancelled);
    }

    #[test]
    fn test_ctrl_d_exits_once_the_command_ended() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = state();
        start_pane_command(&mut s);
        s.handle_key(ctrl('d'));
        s.finish(Ending::Cancelled);
        assert_eq!(s.handle_key(ctrl('d')), Action::Exit);
    }

    #[test]
    fn test_ctrl_d_with_text_and_no_command_does_nothing() {
        let mut s = state();
        type_str(&mut s, "ab");
        assert_eq!(s.handle_key(ctrl('d')), Action::None);
        assert!(!s.scrollback_text().contains(BUSY_NOTE));
    }

    #[test]
    fn test_paste_drops_bidi_controls() {
        let mut s = state();
        s.paste("a\u{202E}b\u{2066}c\u{2069}d\u{202A}");
        assert_eq!(s.input, "abcd");
    }

    #[test]
    fn test_paste_is_capped_with_a_note() {
        let mut s = state();
        s.paste(&"x".repeat(PASTE_LIMIT + 50));
        assert_eq!(s.input.chars().count(), PASTE_LIMIT);
        assert!(
            s.scrollback_text()
                .contains("paste truncated to 10000 characters")
        );
        let mut t = state();
        t.paste(&"x".repeat(PASTE_LIMIT));
        assert!(
            !t.scrollback_text().contains("truncated"),
            "exact limit is not truncated"
        );
    }

    #[test]
    fn test_paste_goes_into_the_open_palette_query() {
        let mut s = state();
        s.handle_key(ctrl('k'));
        s.paste("ven\nues");
        match &s.overlay {
            Some(Overlay::Palette(p)) => assert_eq!(p.query, "ven ues"),
            _ => panic!("the palette stays open"),
        }
        assert_eq!(s.input, "", "the input line is untouched");
    }

    fn row_cells(s: &TuiState, w: u16, h: u16, needle: &str) -> (Vec<String>, u16) {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| s.render(f)).unwrap();
        let buf = term.backend().buffer().clone();
        let y = (0..h)
            .find(|&y| (0..w).any(|x| buf[(x, y)].symbol() == needle))
            .unwrap();
        (
            (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect(),
            y,
        )
    }

    #[test]
    fn test_left_mid_line_keeps_the_text_right_of_the_cursor_visible() {
        let mut s = state();
        s.paste(&format!("{}{}", "a".repeat(100), "b".repeat(5)));
        let _ = cursor_pos(&s, 60, 12);
        for _ in 0..3 {
            s.handle_key(key(KeyCode::Left));
        }
        let (cells, _) = row_cells(&s, 60, 12, "b");
        assert_eq!(cursor_pos(&s, 60, 12).x, 55, "the window did not move");
        assert_eq!(cells[55], "b", "text under and right of the cursor stays");
        assert_eq!(cells[57], "b", "the last char is still shown");
    }

    #[test]
    fn test_window_moves_back_when_the_cursor_leaves_on_the_left() {
        let mut s = state();
        s.paste(&"x".repeat(100));
        let _ = cursor_pos(&s, 60, 12);
        for _ in 0..80 {
            s.handle_key(key(KeyCode::Left));
        }
        assert_eq!(cursor_pos(&s, 60, 12).x, 1, "cursor at the left edge");
    }

    #[test]
    fn test_wide_chars_are_never_cut_at_the_left_edge() {
        // 40 wide chars are 80 columns. The raw offset would be 23 (odd,
        // inside a char); the window starts at the next boundary, 24.
        let mut s = state();
        s.paste(&"日".repeat(40));
        let (cells, _) = row_cells(&s, 60, 12, "日");
        assert_eq!(cells[1], "日", "the first shown cell holds a whole char");
        assert_eq!(cursor_pos(&s, 60, 12).x, 1 + 80 - 24);
        assert_eq!(cells[59], "│", "border intact");
    }

    #[test]
    fn test_cursor_on_a_wide_char_is_never_half_visible() {
        let mut s = state();
        s.paste(&"x".repeat(56));
        s.paste("日日");
        s.handle_key(key(KeyCode::Left));
        // The cursor sits on the second 日 (columns 58..60): it must fit.
        let p = cursor_pos(&s, 60, 12);
        assert!(
            p.x + 2 <= 59,
            "wide char at the cursor fits inside the border"
        );
    }

    /// Starts a pane command and ends it with `ending`, after it wrote `text`.
    fn run_pane(s: &mut TuiState, text: &str, ending: Ending) {
        let inv = Invocation {
            command: venues_list(),
            route: Route::Pane,
            ai: false,
            argv: vec![],
        };
        let out = s.start("venues list", &inv);
        out.write_out(format_args!("{}", text)).unwrap();
        s.finish(ending);
    }

    #[test]
    fn test_ctrl_y_copies_the_plain_output_of_the_last_success() {
        let mut s = state();
        assert_eq!(s.handle_key(ctrl('y')), Action::None);
        assert!(s.scrollback_text().contains("nothing to copy yet"));
        run_pane(&mut s, "\x1b[1;36mA\x1b[0m\nb\n", Ending::Finished(Ok(())));
        assert_eq!(s.handle_key(ctrl('y')), Action::Copy("A\nb\n".into()));
    }

    #[test]
    fn test_failed_or_cancelled_run_keeps_the_earlier_copy() {
        let mut s = state();
        run_pane(&mut s, "good\n", Ending::Finished(Ok(())));
        run_pane(
            &mut s,
            "partial\n",
            Ending::Finished(Err(scope::error::ScopeError::Chain("boom".into()))),
        );
        run_pane(&mut s, "partial\n", Ending::Cancelled);
        assert_eq!(s.handle_key(ctrl('y')), Action::Copy("good\n".into()));
    }

    #[test]
    fn test_save_without_output_says_so_and_escape_is_refused() {
        let mut s = state();
        s.submit("save out.json", &Config::default());
        assert!(s.scrollback_text().contains("nothing to save yet"));
        run_pane(&mut s, "x\n", Ending::Finished(Ok(())));
        s.submit("save ../escape.json", &Config::default());
        assert!(
            s.scrollback_text()
                .contains("save writes inside the current directory or to an absolute path")
        );
        s.submit("save", &Config::default());
        assert!(s.scrollback_text().contains("Usage: save <file> [--force]"));
    }

    #[test]
    fn test_pane_has_no_side_borders() {
        let mut s = state();
        s.push_note("hello");
        let text = screen(&s, 40, 12);
        let row = text.lines().find(|l| l.contains("hello")).unwrap();
        assert!(row.starts_with("hello"), "{row:?}");
    }

    fn last_line(s: &TuiState) -> &Line<'static> {
        s.scrollback.last().unwrap()
    }

    #[test]
    fn test_ok_run_ends_with_done_footer() {
        let mut s = state();
        run_pane(&mut s, "x\n", Ending::Finished(Ok(())));
        let text = last_line(&s).to_string();
        assert!(
            regex::Regex::new(r"^Done in \d+\.\ds\.$")
                .unwrap()
                .is_match(&text),
            "{text}"
        );
    }

    #[test]
    fn test_failed_run_ends_with_a_red_failed_footer() {
        let mut s = state();
        let err = Ending::Finished(Err(scope::error::ScopeError::Chain("boom".into())));
        s.start("x", &pane_invocation());
        s.finish_with(err, true);
        let line = last_line(&s);
        let text = line.to_string();
        assert!(
            regex::Regex::new(r"^✗ Failed in \d+\.\ds\.$")
                .unwrap()
                .is_match(&text),
            "{text}"
        );
        assert_eq!(line.style.fg, Some(Color::Red));
    }

    #[test]
    fn test_failed_footer_keeps_glyph_and_words_without_color() {
        let mut s = state();
        s.start("x", &pane_invocation());
        s.finish_with(
            Ending::Finished(Err(scope::error::ScopeError::Chain("boom".into()))),
            false,
        );
        let line = last_line(&s);
        assert!(line.to_string().starts_with("✗ Failed in "));
        assert_eq!(line.style.fg, None, "no color when color is off");
    }

    #[test]
    fn test_cancelled_run_ends_with_cancelled_after_footer() {
        let mut s = state();
        run_pane(&mut s, "x\n", Ending::Cancelled);
        let text = last_line(&s).to_string();
        assert!(
            regex::Regex::new(r"^Cancelled after \d+\.\ds\.$")
                .unwrap()
                .is_match(&text),
            "{text}"
        );
    }

    fn pane_invocation() -> Invocation {
        Invocation {
            command: venues_list(),
            route: Route::Pane,
            ai: false,
            argv: vec![],
        }
    }

    fn warned_json_run(s: &mut TuiState) {
        let inv = pane_invocation();
        let out = s.start("address 0x1 --format json", &inv);
        crate::errln!(out, "\u{26a0} Token balances unavailable").unwrap();
        crate::outln!(out, "{{\"a\": 1}}").unwrap();
        s.finish(Ending::Finished(Ok(())));
    }

    #[test]
    fn test_last_output_excludes_warnings_the_pane_still_shows() {
        let mut s = state();
        warned_json_run(&mut s);
        let Action::Copy(text) = s.handle_key(ctrl('y')) else {
            panic!("expected a copy");
        };
        serde_json::from_str::<serde_json::Value>(&text).unwrap();
        assert!(!text.contains("Token balances"));
        assert!(s.scrollback_text().contains("Token balances unavailable"));
    }

    #[test]
    fn test_a_success_with_no_output_clears_the_copy() {
        let mut s = state();
        run_pane(&mut s, "data\n", Ending::Finished(Ok(())));
        run_pane(&mut s, "", Ending::Finished(Ok(())));
        assert_eq!(s.handle_key(ctrl('y')), Action::None);
        s.submit("save out.json", &Config::default());
        assert!(s.scrollback_text().contains("nothing to save yet"));
    }

    #[test]
    fn test_ctrl_y_refuses_output_over_the_osc52_cap() {
        let mut s = state();
        let big = "x".repeat(clip::OSC52_MAX_PAYLOAD);
        run_pane(&mut s, &format!("{big}\n"), Ending::Finished(Ok(())));
        assert_eq!(s.handle_key(ctrl('y')), Action::None);
        assert!(
            s.scrollback_text()
                .contains("too large to copy (1 lines); use save <file>")
        );
    }

    #[test]
    fn test_save_with_unbalanced_quotes_gets_the_save_usage() {
        let mut s = state();
        run_pane(&mut s, "x\n", Ending::Finished(Ok(())));
        s.submit("save \"unclosed", &Config::default());
        let text = s.scrollback_text();
        assert!(
            text.contains("Unbalanced quotes. Usage: save <file> [--force]"),
            "{text}"
        );
    }

    /// The pane rows of a `w`-column screen tall enough to show all output.
    fn pane_rows(s: &TuiState, w: u16) -> Vec<String> {
        let h = 200;
        let rows: Vec<String> = screen(s, w, h).lines().map(str::to_string).collect();
        // Pane border rows are the first and the one above the input box.
        rows[1..(h as usize - 6)].to_vec()
    }

    #[tokio::test]
    async fn test_venues_list_rules_never_wrap_in_a_narrow_pane() {
        // R11: a rule wider than the pane must not continue on the next row,
        // and a wrapped row inside a section keeps its `│` gutter. The
        // output is written without a width override (a resize after the
        // run does the same), so the pane itself must fit it.
        let _lock = super::super::DIAG_LOCK.lock().await;
        let mut s = state();
        let inv = pane_invocation();
        let out = s.start("venues list", &inv);
        let factory = scope::chains::mocks::MockClientFactory::new();
        let res = run_invocation(inv, &Config::default(), &factory, &out).await;
        s.finish(Ending::Finished(res));
        let rows = pane_rows(&s, 40);
        for pair in rows.windows(2) {
            let continued = pair[0].trim_end().ends_with('─') && pair[1].starts_with('─');
            assert!(!continued, "a rule wrapped:\n{}", rows.join("\n"));
        }
        let first = rows
            .iter()
            .position(|r| r.starts_with("┌─ Available Venues"));
        let last = rows.iter().position(|r| r.starts_with('└'));
        let (Some(first), Some(last)) = (first, last) else {
            panic!("no section:\n{}", rows.join("\n"));
        };
        for row in &rows[first + 1..last] {
            assert!(
                row.starts_with('│') || row.starts_with('├'),
                "row lost its gutter: {row:?}\n{}",
                rows.join("\n")
            );
        }
        for row in &rows {
            assert!(row.width() <= 40, "{row:?}");
        }
    }

    #[test]
    fn test_pane_wraps_never_reach_the_copy() {
        // Ctrl-Y and `save` give the command's own text: no row breaks or
        // gutters that the pane adds to fit its width.
        let mut s = state();
        let long = format!("│  {}", "word ".repeat(30).trim_end());
        run_pane(&mut s, &format!("{long}\n"), Ending::Finished(Ok(())));
        let rows = pane_rows(&s, 40);
        let wrapped: Vec<_> = rows.iter().filter(|r| r.contains("word")).collect();
        assert!(wrapped.len() > 1, "{rows:#?}");
        assert!(
            wrapped.iter().all(|r| r.starts_with("│  word")),
            "{rows:#?}"
        );
        let Action::Copy(text) = s.handle_key(ctrl('y')) else {
            panic!("expected a copy");
        };
        assert_eq!(text, format!("{long}\n"));
    }

    #[test]
    fn test_render_records_the_pane_width() {
        // The event loop sets the width override from it; with no side
        // borders the pane is as wide as the screen.
        let s = state();
        assert_eq!(s.pane_width(), None, "nothing drawn yet");
        screen(&s, 40, 20);
        assert_eq!(s.pane_width(), Some(40));
    }
}
