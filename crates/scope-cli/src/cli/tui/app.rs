//! # TUI State and Rendering
//!
//! [`TuiState`] holds everything the screen shows: the scrollback, the
//! input line, the history and the command that runs now. It does no
//! terminal I/O, so tests drive it with key events and render it on a
//! `TestBackend`.

use super::clip::{self, SaveError};
use super::exec::{self, Invocation, Plan, Route};
use super::fit;
use super::grid::{self, GridAction, GridMsg, GridState};
use super::journal::{Event, Journal};
use super::repeat_view::{self, Ended, RepeatView};
use super::vocab::Vocab;
use super::wizard::{self, WizardAction, WizardState};
use crate::cli::interactive::SessionContext;
use crate::cli::output::{Captured, Output, SeriesHook};
use crate::cli::textfit::{ELLIPSIS, cols, cut_with_ellipsis, draw_note_alone};
use crate::cli::theme::{ACCENT, ERR, INFO, KEY, MATCH, OK, PaneState, SECONDARY, SELECTED, WARN};
use ansi_to_tui::IntoText;
use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use scope::config::Config;
use scope::display::theme::color_enabled;
use std::cell::Cell;
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

/// The most lines kept in the scrollback. Older lines are dropped.
pub const SCROLLBACK_LIMIT: usize = 5000;

/// The smallest terminal the TUI draws in, in columns and rows.
pub const MIN_W: u16 = 40;
/// See [`MIN_W`].
pub const MIN_H: u16 = 10;

/// The message drawn alone when the terminal is smaller than [`MIN_W`] x
/// [`MIN_H`].
fn too_small_note() -> String {
    format!("Terminal too small: need at least {MIN_W}×{MIN_H}")
}

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
    /// The Ctrl-F search of the scrollback.
    Search(SearchState),
    /// The Ctrl-R search of the history.
    HistorySearch(HistorySearchState),
    /// A question a running command asks (which token a symbol means).
    Choice(ChoiceState),
}

/// The open question from a running command: a list to choose from.
pub struct ChoiceState {
    prompt: String,
    options: Vec<String>,
    selected: usize,
    /// Where the answer goes; `None` once it is sent.
    reply: Option<tokio::sync::oneshot::Sender<Option<usize>>>,
}

/// What Esc does in the current state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EscAction {
    ClosePalette,
    CloseMenu,
    /// Answers the open question with "cancelled".
    AnswerChoice,
    CloseSearch,
    CloseHistorySearch,
    /// Cancels the running command.
    Cancel,
    /// Idle with no overlay: clears the input line.
    ClearLine,
}

impl EscAction {
    /// The status bar words, or `None` when the bar says nothing (clearing
    /// the line is not worth the room).
    fn hint(self) -> Option<&'static str> {
        match self {
            Self::ClosePalette => Some("Esc closes palette"),
            Self::CloseMenu => Some("Esc closes menu"),
            Self::AnswerChoice => Some("↑↓ Enter chooses · Esc cancels"),
            Self::CloseSearch => Some("Esc closes search"),
            Self::CloseHistorySearch => Some("Esc closes history search"),
            Self::Cancel => Some("Esc cancels"),
            Self::ClearLine => None,
        }
    }
}

/// What the status bar says when a search finds nothing.
const NO_MATCH: &str = "no match";

/// The glyph that starts a note line.
const NOTE_GLYPH: &str = "ℹ";

/// The gutter of a matched row: it marks the match without color.
const MATCH_MARK: &str = "▶ ";

/// The gutter of a row that is not a match, so rows do not shift.
const MATCH_BLANK: &str = "  ";

/// What a draw needs of an open search.
struct SearchView {
    matches: Vec<usize>,
    current: Option<usize>,
    text: String,
}

/// The open scrollback search.
pub struct SearchState {
    /// What is searched for. Matches ignore case.
    pub query: String,
    /// Typing edits the query. Off, `n` and `N` walk the matches.
    editing: bool,
    /// The scrollback entry of the match the view shows now.
    current: Option<usize>,
    /// The scroll position from before the search opened. Esc restores it.
    saved_scroll: usize,
}

/// The open history search.
pub struct HistorySearchState {
    /// What is searched for. Matches ignore case.
    pub query: String,
    /// The history entry that is in the input now.
    current: Option<usize>,
    /// The input line from before the search opened. Esc restores it.
    draft: String,
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
#[derive(Debug, PartialEq)]
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
    /// Stop the running watchlist (Esc in the grid).
    StopWatch,
    /// Start this watchlist in the grid (the wizard's Start).
    StartWatch(Box<scope::watch::Watchlist>),
}

/// True when Alt is held (and Ctrl is not).
fn alt_key(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::ALT) && !key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Ctrl-G or `watch` with no grid.
const NO_GRID_NOTE: &str =
    "No watchlist runs. Start one with `watch open <name>` (`watch list` lists them).";

/// Marks the selected palette row.
const PALETTE_MARKER: &str = "> ";

/// The palette's key legend (#66): Enter fills the line, as an entry may
/// still need its arguments; a second Enter runs it.
const PALETTE_LEGEND: &str = "↑↓ select · Enter puts it on the line (Enter again runs) · Esc close";

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
    /// The runs of a repeating command (#58), drawn as the repeating view.
    series: Option<SeriesHook>,
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
    scroll: Cell<usize>,
    /// The last draw was the too-small message. Enter does not run then.
    too_small: Cell<bool>,
    /// The most the view can scroll up, at the last draw. PageUp stops here.
    last_max_top: Cell<usize>,
    /// The inner height of the output pane at the last draw. A page is one
    /// row less.
    last_pane_height: Cell<usize>,
    /// The session context.
    pub ctx: SessionContext,
    /// The command that runs in the pane now.
    pub running: Option<Running>,
    /// The last pane command failed and no command has started since. It
    /// turns the pane rule red.
    failed: bool,
    /// The first display column of the input line that is shown. It moves
    /// only when the cursor leaves the window (set while drawing).
    scroll_x: Cell<usize>,
    /// The inner width of the output pane at the last draw; 0 before the
    /// first. The event loop makes it the width override.
    pane_width: Cell<u16>,
    /// Show the busy note in the hint row until the next key press.
    busy_hint: bool,
    /// How many old lines the pane dropped (it keeps [`SCROLLBACK_LIMIT`]).
    dropped_lines: usize,
    /// The line that Esc, Ctrl-U or Alt-K last cleared. ↑ on an empty line
    /// brings it back (#67).
    cleared: Option<String>,
    /// The plain output of the last pane command that succeeded. Ctrl-Y
    /// copies it and `save` writes it.
    last_output: Option<String>,
    /// A finished repeating view, kept on screen until the next key (#58).
    repeat_view: Option<RepeatView>,
    /// The series of the last repeating command, for `save x.csv`.
    last_series: Option<SeriesHook>,
    /// The event log.
    journal: Journal,
    /// Completion values from the user's stores.
    vocab: Vocab,
    /// The open completion menu, palette or search, if any.
    pub overlay: Option<Overlay>,
    /// The slot through which a pane command asks a question.
    pub(crate) asker: crate::cli::output::Asker,
    /// The view must move to the current search match at the next draw,
    /// which knows the fitted rows.
    reveal: Cell<bool>,
    /// The pane width the last search draw used. A resize changes the
    /// rows, so the view moves to the match again.
    search_width: Cell<usize>,
    /// The watchlist grid, while a watchlist runs or until its finished
    /// grid is closed (#47).
    pub grid: Option<GridState>,
    /// The grid is on screen (in place of the output pane).
    pub grid_shown: bool,
    /// The run's messages to the grid.
    grid_rx: Option<std::sync::mpsc::Receiver<GridMsg>>,
    /// What the watchlist run wrote (notes, errors), for the scrollback.
    grid_out: Option<(Output, Captured)>,
    /// When the watchlist run started.
    grid_started: Option<Instant>,
    /// The monitor wizard, while it is open (#47).
    pub wizard: Option<WizardState>,
    /// A command to run when the loop starts: `scope monitor <args>` opens
    /// the TUI with its wizard (#47).
    pub start: Option<crate::cli::Commands>,
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
            scroll: Cell::new(0),
            too_small: Cell::new(false),
            last_max_top: Cell::new(0),
            last_pane_height: Cell::new(0),
            ctx,
            running: None,
            failed: false,
            scroll_x: Cell::new(0),
            pane_width: Cell::new(0),
            busy_hint: false,
            last_output: None,
            dropped_lines: 0,
            cleared: None,
            repeat_view: None,
            last_series: None,
            journal,
            vocab,
            overlay: None,
            asker: crate::cli::output::Asker::default(),
            reveal: Cell::new(false),
            search_width: Cell::new(0),
            grid: None,
            grid_shown: false,
            grid_rx: None,
            grid_out: None,
            grid_started: None,
            wizard: None,
            start: None,
        }
    }

    /// The journal, for saving the history on exit.
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Replaces the completion words, after the config was read again.
    pub fn set_vocab(&mut self, vocab: Vocab) {
        self.vocab = vocab;
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
        // An open question takes every key until it is answered. While the
        // terminal is too small the list is not drawn, so only Esc (cancel
        // the question) and Ctrl-C (cancel the command) act.
        if let Some(Overlay::Choice(ch)) = &mut self.overlay {
            let n = ch.options.len().max(1);
            match key.code {
                KeyCode::Esc => return self.press_esc(),
                KeyCode::Char('c') if ctrl => return Action::Cancel,
                _ if self.too_small.get() => {}
                KeyCode::Down | KeyCode::Tab => ch.selected = (ch.selected + 1) % n,
                KeyCode::Up | KeyCode::BackTab => ch.selected = (ch.selected + n - 1) % n,
                KeyCode::Enter => {
                    let pick = ch.selected;
                    self.answer_choice(Some(pick));
                }
                _ => {}
            }
            return Action::None;
        }
        // The monitor wizard takes every key while it is open (#47).
        if self.wizard.is_some() && !self.too_small.get() {
            return self.wizard_key(key);
        }
        // Ctrl-G shows or hides the grid while there is one (#47).
        if ctrl && key.code == KeyCode::Char('g') {
            if self.grid.is_some() {
                self.grid_shown = !self.grid_shown;
            } else {
                self.push_note(NO_GRID_NOTE);
            }
            return Action::None;
        }
        if self.grid_shown && !self.too_small.get() {
            return self.grid_key(key);
        }
        // A finished repeating view stays until the next key. That key
        // leaves it and then does its normal job, so typing goes on.
        if self.running.is_none()
            && let Some(view) = self.repeat_view.take()
        {
            self.leave_repeat_view(view);
        }
        // Ctrl-C closes the palette and keeps its normal meaning (cancel a
        // running command, clear the line), so a command can always be
        // stopped with one key.
        if ctrl
            && key.code == KeyCode::Char('c')
            && matches!(
                self.overlay,
                Some(Overlay::Palette(_) | Overlay::Search(_) | Overlay::HistorySearch(_))
            )
        {
            self.close_overlay();
        }
        // The overlays are not drawn while the terminal is too small, so
        // they must not act: Esc or any key closes one and does no more.
        if self.too_small.get() && self.overlay.is_some() {
            self.close_overlay();
            return Action::None;
        }
        if key.code == KeyCode::Esc {
            return self.press_esc();
        }
        match self.overlay.take() {
            Some(Overlay::Search(st)) => {
                self.search_key(st, key);
                return Action::None;
            }
            Some(Overlay::HistorySearch(st)) => {
                self.history_search_key(st, key);
                return Action::None;
            }
            other => self.overlay = other,
        }
        if let Some(Overlay::Palette(p)) = &mut self.overlay {
            let count = p.matches().len();
            match key.code {
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
            KeyCode::Enter => {
                // The screen shows no output now: the line stays.
                if self.too_small.get() {
                    return Action::None;
                }
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
                self.scroll.set(0);
                if !line.trim().is_empty() && self.history.last() != Some(&line) {
                    self.history.push(line.clone());
                }
                return Action::Submit(line);
            }
            // Tab and Ctrl-K open overlays, which are not drawn while the
            // terminal is too small.
            KeyCode::Tab if !self.too_small.get() => self.tab_complete(),
            KeyCode::Up if self.input.is_empty() && self.cleared.is_some() => {
                if let Some(line) = self.cleared.take() {
                    self.set_input(line);
                }
            }
            KeyCode::Up => self.history_prev(),
            KeyCode::Down => self.history_next(),
            KeyCode::PageUp => {
                let max = self.last_max_top.get();
                self.scroll.set(
                    self.scroll
                        .get()
                        .min(max)
                        .saturating_add(self.page())
                        .min(max),
                );
            }
            KeyCode::PageDown => {
                self.scroll.set(
                    self.scroll
                        .get()
                        .min(self.last_max_top.get())
                        .saturating_sub(self.page()),
                );
            }
            KeyCode::Home if ctrl => self.scroll.set(self.last_max_top.get()),
            KeyCode::End if ctrl => self.scroll.set(0),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.char_len()),
            KeyCode::Home => self.cursor = 0,
            // On an empty line there is no cursor to move: End goes to the
            // newest output.
            KeyCode::End if self.input.is_empty() => self.scroll.set(0),
            KeyCode::End => self.cursor = self.char_len(),
            KeyCode::Char('f') if ctrl && !self.too_small.get() => self.open_search(),
            KeyCode::Char('/')
                if !ctrl
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && self.input.is_empty()
                    && !self.too_small.get() =>
            {
                self.open_search()
            }
            KeyCode::Char('r') if ctrl && !self.too_small.get() => self.open_history_search(),
            KeyCode::Char('k') if ctrl && !self.too_small.get() => {
                self.overlay = Some(Overlay::Palette(super::palette::PaletteState::new(
                    super::palette::items(&self.history),
                )));
            }
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            // Readline moves (#67). Ctrl-F searches, Ctrl-Y copies and
            // Ctrl-K opens the palette here, so Right moves forward and
            // Alt-K deletes to the end of the line.
            KeyCode::Char('b') if ctrl => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Char('b') if alt_key(&key) => self.cursor = self.word_left(),
            KeyCode::Char('f') if alt_key(&key) => self.cursor = self.word_right(),
            KeyCode::Char('k') if alt_key(&key) => {
                let kept: String = self.input.chars().take(self.cursor).collect();
                let cut: String = self.input.chars().skip(self.cursor).collect();
                self.keep_cleared(cut);
                self.input = kept;
            }
            KeyCode::Char('e') if ctrl => self.cursor = self.char_len(),
            KeyCode::Char('u') if ctrl => {
                let cut: String = self.input.chars().take(self.cursor).collect();
                let rest: String = self.input.chars().skip(self.cursor).collect();
                self.keep_cleared(cut);
                self.input = rest;
                self.cursor = 0;
            }
            KeyCode::Char('w') if ctrl => self.delete_word(),
            KeyCode::Backspace => {
                self.scroll.set(0);
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.remove_char(self.cursor);
                }
            }
            KeyCode::Delete => {
                self.scroll.set(0);
                if self.cursor < self.char_len() {
                    self.remove_char(self.cursor);
                }
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.scroll.set(0);
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
        // An open question keeps the focus: a paste neither answers nor
        // closes it.
        if clean.is_empty() || matches!(self.overlay, Some(Overlay::Choice(_))) {
            return;
        }
        // A search keeps the view where it is.
        if !matches!(
            self.overlay,
            Some(Overlay::Search(_) | Overlay::HistorySearch(_))
        ) {
            self.scroll.set(0);
        }
        let truncated = clean.chars().count() > PASTE_LIMIT;
        if truncated {
            clean = clean.chars().take(PASTE_LIMIT).collect();
        }
        match self.overlay.take() {
            Some(Overlay::Search(st)) => self.search_type(st, &clean),
            Some(Overlay::HistorySearch(st)) => self.history_search_type(st, &clean),
            // The palette has the focus: the paste goes into its query.
            Some(Overlay::Palette(mut p)) => {
                p.query.push_str(&clean);
                p.selected = 0;
                self.overlay = Some(Overlay::Palette(p));
            }
            _ => {
                let at = self.byte_index(self.cursor);
                self.input.insert_str(at, &clean);
                self.cursor += clean.chars().count();
            }
        }
        if lines > 1 {
            self.push_note(&format!("pasted {} lines as one line", lines));
        }
        if truncated {
            self.push_note(&format!("paste truncated to {} characters", PASTE_LIMIT));
        }
    }

    // -------------------------------------------------------------- search

    /// Closes any overlay. A search puts back what it moved: the scroll
    /// position, or the input line.
    fn close_overlay(&mut self) {
        match self.overlay.take() {
            Some(Overlay::Search(st)) => self.scroll.set(st.saved_scroll),
            Some(Overlay::HistorySearch(st)) => self.set_input(st.draft),
            _ => {}
        }
    }

    fn open_search(&mut self) {
        self.overlay = Some(Overlay::Search(SearchState {
            query: String::new(),
            editing: true,
            current: None,
            saved_scroll: self.scroll.get().min(self.last_max_top.get()),
        }));
    }

    fn open_history_search(&mut self) {
        self.hist_pos = None;
        self.overlay = Some(Overlay::HistorySearch(HistorySearchState {
            query: String::new(),
            current: None,
            draft: self.input.clone(),
        }));
    }

    /// The output of the running command so far, as the pane shows it.
    /// The spinner lines are not output, so they are not in it.
    fn live_output(&self) -> Vec<Line<'static>> {
        self.running
            .as_ref()
            .and_then(|run| run.captured.out().into_text().ok())
            .map(|t| t.lines)
            .unwrap_or_default()
    }

    /// The entries that hold `query`, oldest first. An index past the end
    /// of the scrollback is a line of the running command's output.
    fn search_matches(&self, query: &str) -> Vec<usize> {
        self.matches_in(query, &self.live_output())
    }

    fn matches_in(&self, query: &str, live: &[Line<'static>]) -> Vec<usize> {
        if query.is_empty() {
            return Vec::new();
        }
        let needle = query.to_lowercase();
        self.scrollback
            .iter()
            .chain(live)
            .enumerate()
            .filter(|(_, line)| line.to_string().to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect()
    }

    /// The history entries that hold `query`, oldest first.
    fn history_matches(&self, query: &str) -> Vec<usize> {
        if query.is_empty() {
            return Vec::new();
        }
        let needle = query.to_lowercase();
        self.history
            .iter()
            .enumerate()
            .filter(|(_, line)| line.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect()
    }

    /// Where `current` sits in `matches`, counted from the newest, and how
    /// many matches there are.
    fn rank(matches: &[usize], current: Option<usize>) -> Option<(usize, usize)> {
        let at = matches.binary_search(&current?).ok()?;
        Some((matches.len() - at, matches.len()))
    }

    /// The match before (`older`) or after `current`, wrapping.
    fn step(matches: &[usize], current: Option<usize>, older: bool) -> Option<usize> {
        let last = matches.len().checked_sub(1)?;
        let at = current.and_then(|c| matches.binary_search(&c).ok());
        let next = match (at, older) {
            (None, _) => last,
            (Some(0), true) => last,
            (Some(a), true) => a - 1,
            (Some(a), false) if a == last => 0,
            (Some(a), false) => a + 1,
        };
        Some(matches[next])
    }

    /// Adds `text` to the query and jumps to the newest match.
    fn search_type(&mut self, mut st: SearchState, text: &str) {
        st.editing = true;
        st.query.push_str(text);
        self.search_refind(&mut st);
        self.overlay = Some(Overlay::Search(st));
    }

    fn search_refind(&self, st: &mut SearchState) {
        st.current = self.search_matches(&st.query).last().copied();
        self.reveal.set(st.current.is_some());
    }

    fn search_key(&mut self, mut st: SearchState, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let walk = |s: &mut Self, st: &mut SearchState, older: bool| {
            let found = Self::step(&s.search_matches(&st.query), st.current, older);
            if found.is_some() {
                st.current = found;
                s.reveal.set(true);
            }
        };
        match key.code {
            KeyCode::Char('f') if ctrl => st.editing = !st.editing,
            KeyCode::Enter if st.editing => st.editing = false,
            // In navigate mode Enter steps to the next older match, as `n`.
            KeyCode::Enter => walk(self, &mut st, true),
            KeyCode::Backspace if st.editing => {
                st.query.pop();
                self.search_refind(&mut st);
            }
            KeyCode::Backspace => st.editing = true,
            KeyCode::Char(c) if !ctrl && !alt && st.editing => {
                let mut buf = [0u8; 4];
                return self.search_type(st, c.encode_utf8(&mut buf));
            }
            KeyCode::Char('n') if !ctrl && !alt => walk(self, &mut st, true),
            KeyCode::Char('N') if !ctrl && !alt => walk(self, &mut st, false),
            _ => {}
        }
        self.overlay = Some(Overlay::Search(st));
    }

    /// Adds `text` to the history query and shows the newest match.
    fn history_search_type(&mut self, mut st: HistorySearchState, text: &str) {
        st.query.push_str(text);
        self.history_refind(&mut st);
        self.overlay = Some(Overlay::HistorySearch(st));
    }

    /// Shows the newest match in the input, or the draft when none match.
    fn history_refind(&mut self, st: &mut HistorySearchState) {
        st.current = self.history_matches(&st.query).last().copied();
        self.show_history_match(st);
    }

    fn show_history_match(&mut self, st: &HistorySearchState) {
        let line = match st.current {
            Some(i) => self.history[i].clone(),
            None => st.draft.clone(),
        };
        self.set_input(line);
    }

    fn history_search_key(&mut self, mut st: HistorySearchState, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            // The match stays in the input. It does not run.
            KeyCode::Enter => return,
            KeyCode::Char('r') if ctrl => {
                let found = Self::step(&self.history_matches(&st.query), st.current, true);
                if found.is_some() {
                    st.current = found;
                    self.show_history_match(&st);
                }
            }
            KeyCode::Backspace => {
                st.query.pop();
                self.history_refind(&mut st);
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                let mut buf = [0u8; 4];
                return self.history_search_type(st, c.encode_utf8(&mut buf));
            }
            _ => {}
        }
        self.overlay = Some(Overlay::HistorySearch(st));
    }

    /// The open search as the draw needs it: the matches, the current
    /// match and the status text. History search has no pane matches.
    fn search_view(&self, live: &[Line<'static>]) -> Option<SearchView> {
        match &self.overlay {
            Some(Overlay::Search(st)) => {
                let matches = self.matches_in(&st.query, live);
                // A dropped current entry leaves the newest match current.
                let current = st
                    .current
                    .filter(|c| matches.binary_search(c).is_ok())
                    .or(matches.last().copied());
                let text = Self::search_text("search", &st.query, &matches, current);
                Some(SearchView {
                    matches,
                    current,
                    text,
                })
            }
            Some(Overlay::HistorySearch(st)) => {
                let matches = self.history_matches(&st.query);
                let text = Self::search_text("history search", &st.query, &matches, st.current);
                Some(SearchView {
                    matches: Vec::new(),
                    current: None,
                    text,
                })
            }
            _ => None,
        }
    }

    fn search_text(label: &str, query: &str, matches: &[usize], cur: Option<usize>) -> String {
        if query.is_empty() {
            return format!("{label}: type to search");
        }
        match Self::rank(matches, cur) {
            Some((n, total)) => format!("{label}: {query} {n}/{total}"),
            None => format!("{label}: {query} {NO_MATCH}"),
        }
    }

    /// Rows moved by PageUp and PageDown: the pane height less one, so the
    /// top row of one page is the bottom row of the next.
    fn page(&self) -> usize {
        self.last_pane_height.get().saturating_sub(1).max(1)
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
        self.input
            .split_whitespace()
            .next()
            .is_some_and(|w| exec::session_command_name(w) == "exit")
    }

    /// Says why exit waits. Does not repeat when the last line is the note.
    fn push_busy_note(&mut self) {
        let last = self.scrollback.last().map(|l| l.to_string());
        if last.as_deref() != Some(format!("{NOTE_GLYPH} {BUSY_NOTE}").as_str()) {
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

    /// Keeps a cleared piece of the line for ↑ to bring back (#67).
    fn keep_cleared(&mut self, text: String) {
        if !text.trim().is_empty() {
            self.cleared = Some(text);
        }
    }

    /// The start of the word left of the cursor (Alt-B).
    fn word_left(&self) -> usize {
        let chars: Vec<char> = self.input.chars().collect();
        let mut at = self.cursor;
        while at > 0 && chars[at - 1] == ' ' {
            at -= 1;
        }
        while at > 0 && chars[at - 1] != ' ' {
            at -= 1;
        }
        at
    }

    /// The end of the word right of the cursor (Alt-F).
    fn word_right(&self) -> usize {
        let chars: Vec<char> = self.input.chars().collect();
        let mut at = self.cursor;
        while at < chars.len() && chars[at] == ' ' {
            at += 1;
        }
        while at < chars.len() && chars[at] != ' ' {
            at += 1;
        }
        at
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

    /// Appends a note line: the `ℹ` glyph, then the dim text.
    pub fn push_note(&mut self, note: &str) {
        self.push_lines(vec![Line::from(vec![
            Span::styled(NOTE_GLYPH, INFO),
            Span::styled(format!(" {note}"), SECONDARY),
        ])]);
    }

    /// Appends a note line in `style`.
    pub(crate) fn push_styled_note(&mut self, note: &str, style: Style) {
        self.push_lines(vec![Line::styled(note.to_string(), style)]);
    }

    fn push_lines(&mut self, lines: Vec<Line<'static>>) {
        self.scrollback.extend(lines);
        let excess = self.scrollback.len().saturating_sub(SCROLLBACK_LIMIT);
        self.scrollback.drain(..excess);
        self.dropped_lines += excess;
        // Entries moved up: the search keeps its match, or loses it when
        // that entry was dropped.
        if let Some(Overlay::Search(st)) = &mut self.overlay {
            st.current = st.current.and_then(|c| c.checked_sub(excess));
        }
    }

    fn push_echo(&mut self, line: &str) {
        self.push_lines(vec![Line::from(vec![
            Span::styled("scope", ACCENT),
            Span::styled(format!(":{}", self.ctx.chain), KEY),
            Span::raw("> "),
            Span::raw(line.to_string()),
        ])]);
    }

    /// The prompt: `scope` in the accent, the chain in the key color.
    fn prompt(&self) -> Line<'static> {
        Line::from(vec![
            Span::styled("scope", ACCENT),
            Span::styled(format!(":{}", self.ctx.chain), KEY),
            Span::raw("> "),
        ])
    }

    /// What the output pane shows: running, failed or idle.
    fn pane_state(&self) -> PaneState {
        if self.running.is_some() {
            PaneState::Running
        } else if self.failed {
            PaneState::Failed
        } else {
            PaneState::Idle
        }
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

    // ---------------------------------------------------------------- grid

    /// A key while the grid is on screen.
    fn grid_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('d') {
            return Action::Exit;
        }
        let Some(g) = self.grid.as_mut() else {
            self.grid_shown = false;
            return Action::None;
        };
        // Ctrl-C stops the run, as Esc does.
        let action = if ctrl && key.code == KeyCode::Char('c') {
            if g.is_running() {
                GridAction::Stop
            } else {
                GridAction::Close
            }
        } else {
            g.key(key)
        };
        match action {
            GridAction::None => Action::None,
            GridAction::Shell => {
                self.grid_shown = false;
                Action::None
            }
            GridAction::Stop => Action::StopWatch,
            GridAction::Close => {
                self.close_grid();
                Action::None
            }
            GridAction::Run(line) => {
                if self.running.is_some() {
                    self.busy_hint = true;
                    return Action::None;
                }
                self.grid_shown = false;
                Action::Submit(line)
            }
        }
    }

    /// Opens the monitor wizard over `list`, with the notes from the
    /// arguments.
    pub fn open_wizard(&mut self, list: scope::watch::Watchlist, notes: Vec<String>) {
        self.close_overlay();
        for n in &notes {
            self.push_note(n);
        }
        self.wizard = Some(WizardState::new(
            list,
            notes,
            self.vocab.venues.clone(),
            scope::paths::watchlists_dir(),
        ));
    }

    /// A key while the wizard is open.
    fn wizard_key(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('d') {
            return Action::Exit;
        }
        let Some(w) = self.wizard.as_mut() else {
            return Action::None;
        };
        match w.key(key) {
            WizardAction::None => Action::None,
            WizardAction::Leave => {
                self.wizard = None;
                self.push_note("Left the monitor wizard.");
                Action::None
            }
            WizardAction::Saved(path) => {
                let name = w.list.name.clone();
                self.push_note(&format!(
                    "Saved the watchlist to {}. `watch open {name}` runs it again.",
                    path.display()
                ));
                Action::None
            }
            WizardAction::Start(list) => {
                self.wizard = None;
                Action::StartWatch(list)
            }
        }
    }

    /// Whether a watchlist runs now.
    pub fn watch_running(&self) -> bool {
        self.grid.as_ref().is_some_and(GridState::is_running)
    }

    /// Starts the grid for `list`. Returns the output the run writes to
    /// (it asks its questions in the TUI), the sink for its library
    /// warnings, and the sender for its messages.
    pub fn start_watch(
        &mut self,
        list: scope::watch::Watchlist,
    ) -> (Output, scope::diag::Sink, std::sync::mpsc::Sender<GridMsg>) {
        let (out, captured) = Output::capture_merged();
        // The run's own sink: its warnings never join a pane command's
        // output (the redirect is process-wide).
        let sink = captured.out_sink();
        let out = out.with_asker(self.asker.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        self.push_note(&format!(
            "Watchlist {} started: {} target(s), every {}. Ctrl-G switches between the grid and the shell.",
            list.name,
            list.targets.len(),
            scope::watch::time::format_duration(list.interval.as_secs())
        ));
        self.grid = Some(GridState::new(list, out.progress_hook()));
        self.grid_shown = true;
        self.grid_rx = Some(rx);
        self.grid_out = Some((out.clone(), captured));
        self.grid_started = Some(Instant::now());
        (out, sink, tx)
    }

    /// `setup` saved new settings. A running watchlist keeps the ones it
    /// started with; the grid says to restart it.
    pub fn settings_reloaded(&mut self) {
        if let Some(g) = self.grid.as_mut().filter(|g| g.is_running()) {
            g.stale = true;
            self.push_note(grid::STALE_NOTE);
        }
    }

    /// Moves the run's messages into the grid.
    pub fn poll_watch(&mut self) {
        let (Some(rx), Some(g)) = (&self.grid_rx, self.grid.as_mut()) else {
            return;
        };
        while let Ok(msg) = rx.try_recv() {
            g.apply(msg);
        }
    }

    /// Takes the bell the grid asks for (a breach started).
    pub fn take_bell(&mut self) -> bool {
        self.grid
            .as_mut()
            .is_some_and(|g| std::mem::take(&mut g.bell))
    }

    fn grid_secs(&self) -> f64 {
        self.grid_started.map_or(0.0, |t| t.elapsed().as_secs_f64())
    }

    /// The run ended by itself: done, or failed with `res`'s error.
    pub fn watch_finished(&mut self, res: scope::error::Result<()>) {
        self.poll_watch();
        let secs = self.grid_secs();
        let (footer, style) = match &res {
            Ok(()) => (format!("✓ Done in {secs:.1}s."), OK),
            Err(e) => (
                format!("✗ Failed in {secs:.1}s: {e}"),
                ERR.add_modifier(Modifier::BOLD),
            ),
        };
        if let Some((out, _)) = &self.grid_out
            && let Err(e) = &res
        {
            let _ = crate::cli::errors::display_error(e, out);
        }
        if let Some(g) = self.grid.as_mut() {
            g.end(footer, style);
        }
        self.grid_rx = None;
        self.drop_watch_question();
    }

    /// The user stopped the run: the grid keeps its last tick, and the
    /// report is written from it when the watchlist has a report path.
    pub fn stop_watch(&mut self) {
        self.poll_watch();
        let secs = self.grid_secs();
        let mut footer = format!("Stopped after {secs:.1}s.");
        if let Some(g) = &self.grid {
            let list = g.ready.as_ref().map_or(&g.list, |r| &r.list);
            if let (Some(path), Some(last)) = (&list.outputs.report, &g.last) {
                match crate::cli::watch::run::write_report(path, list, last) {
                    Ok(()) => footer.push_str(&format!(" Report saved to {}.", path.display())),
                    Err(e) => footer.push_str(&format!(" The report was not written: {e}")),
                }
            }
        }
        if let Some(g) = self.grid.as_mut() {
            g.end(footer, WARN);
        }
        self.grid_rx = None;
        self.drop_watch_question();
    }

    /// A question the stopped run asked has nobody to answer it.
    fn drop_watch_question(&mut self) {
        if self.running.is_none() {
            if matches!(self.overlay, Some(Overlay::Choice(_))) {
                self.overlay = None;
            }
            self.asker.lock().unwrap_or_else(|e| e.into_inner()).take();
        }
    }

    /// Closes a finished grid: the scrollback gets the last snapshot, what
    /// the run wrote, and a summary.
    pub fn close_grid(&mut self) {
        let Some(g) = self.grid.take() else {
            return;
        };
        self.grid_shown = false;
        if let Some(last) = &g.last {
            self.push_ansi(&grid::snapshot_text(&last.snapshot));
        }
        if let Some((_, captured)) = self.grid_out.take() {
            self.push_ansi(&captured.out());
        }
        self.push_note(&grid::summary(&g));
        if let Some((footer, style)) = &g.ended {
            self.push_styled_note(footer, *style);
        }
    }

    /// Shows the grid, or says there is none.
    pub fn show_grid(&mut self) {
        if self.grid.is_some() {
            self.grid_shown = true;
        } else {
            self.push_note(NO_GRID_NOTE);
        }
    }

    // ------------------------------------------------------------ commands

    /// Plans a submitted line. Session commands and errors finish here; a
    /// CLI command comes back for the event loop to run.
    pub fn submit(&mut self, line: &str, config: &Config) -> Plan {
        self.push_echo(line);
        // The failed rule is about the last thing the user did. Any line
        // they enter (a pane command, a builtin, a command run outside the
        // TUI) is that now.
        if !line.trim().is_empty() {
            self.failed = false;
        }
        // `clear` empties the pane, as in a shell (#67); `reset` resets the
        // session context.
        if line
            .split_whitespace()
            .next()
            .map(exec::session_command_name)
            .as_deref()
            == Some("clear")
        {
            self.scrollback.clear();
            self.dropped_lines = 0;
            self.scroll.set(0);
            return Plan::Done;
        }
        if line
            .split_whitespace()
            .next()
            .is_some_and(|w| w.eq_ignore_ascii_case("save"))
        {
            let words = exec::split_line(line);
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
                let home = scope::paths::user_home();
                match std::env::current_dir() {
                    Ok(cwd) => {
                        // `save x.csv` after a repeating run writes its series.
                        let csv = args
                            .iter()
                            .any(|a| a.to_ascii_lowercase().ends_with(".csv"))
                            .then(|| self.last_series.as_ref().and_then(repeat_view::series_csv))
                            .flatten();
                        let text = csv.as_deref().or(self.last_output.as_deref());
                        clip::save(&args, text, &cwd, home.as_deref())
                    }
                    Err(e) => Err(SaveError::failed(e, "Could not read the current directory")),
                }
            }
        };
        match result {
            Ok(note) => {
                self.push_note(&note);
                self.log_action(line, started, "session", None);
            }
            Err(e) => {
                self.push_note(&e.message());
                let outcome = match e {
                    SaveError::Refused(_) => "refused",
                    SaveError::Failed(_) => "error",
                };
                self.log_action(line, started, outcome, Some(e.message()));
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
        // A question the command asks shows as a list in the TUI.
        let out = out.with_asker(self.asker.clone());
        let diag = scope::diag::redirect(captured.out_sink());
        self.failed = false;
        self.last_series = None;
        self.running = Some(Running {
            input: input.to_string(),
            argv: inv.argv.clone(),
            started: Instant::now(),
            started_at: Utc::now().to_rfc3339(),
            captured,
            series: out.series_hook(),
            out: out.clone(),
            _diag: diag,
        });
        out
    }

    /// Ends the pane command: moves its output into the scrollback, shows
    /// the error if it failed, and logs the event.
    pub fn finish(&mut self, ending: Ending) {
        // The command is gone: so is the question it asked.
        if matches!(self.overlay, Some(Overlay::Choice(_))) {
            self.overlay = None;
        }
        self.asker.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.finish_with(ending);
    }

    /// Opens the question a running command put in the ask slot, when no
    /// question is open.
    pub fn poll_ask(&mut self) {
        if matches!(self.overlay, Some(Overlay::Choice(_))) {
            return;
        }
        let ask = self.asker.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(ask) = ask {
            // A search puts back what it moved before the list replaces it.
            self.close_overlay();
            self.overlay = Some(Overlay::Choice(ChoiceState {
                prompt: ask.prompt,
                options: ask.options,
                selected: 0,
                reply: Some(ask.reply),
            }));
        }
    }

    /// Sends `answer` to the command that asked and closes the list.
    fn answer_choice(&mut self, answer: Option<usize>) {
        if let Some(Overlay::Choice(mut ch)) = self.overlay.take()
            && let Some(reply) = ch.reply.take()
        {
            // The command may be gone already; then nobody waits.
            let _ = reply.send(answer);
        }
    }

    /// `finish`. Every footer carries its glyph or words, and its style
    /// holds the color; a frame with color off strips it.
    fn finish_with(&mut self, ending: Ending) {
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
        let secs = run.started.elapsed().as_secs_f64();
        let (footer, style) = match &ending {
            Ending::Cancelled => (format!("Cancelled after {:.1}s.", secs), WARN),
            Ending::Finished(Err(_)) => {
                self.failed = true;
                (
                    format!("✗ Failed in {:.1}s.", secs),
                    ERR.add_modifier(Modifier::BOLD),
                )
            }
            Ending::Finished(Ok(())) => (format!("✓ Done in {:.1}s.", secs), OK),
        };
        match run.series.filter(repeat_view::has_runs) {
            // A repeating run keeps its view, with the footer, until the next
            // key. Ctrl-Y and save take its final snapshot and its series.
            Some(series) => {
                self.last_output =
                    repeat_view::final_snapshot(&series).map(|t| crate::cli::ansi::strip(&t));
                self.last_series = Some(series.clone());
                self.repeat_view = Some(RepeatView {
                    series,
                    ended: Some(Ended {
                        text: footer,
                        style,
                    }),
                    pending: run.captured.out(),
                });
            }
            None => {
                self.keep_copy(&run.captured, &ending);
                self.push_ansi(&run.captured.out());
                self.push_styled_note(&footer, style);
            }
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

    /// Leaves a finished repeating view: the scrollback gets one copy of
    /// the final snapshot, then the command's notes, the run summary and
    /// the footer, so the closing lines stay in view together.
    fn leave_repeat_view(&mut self, view: RepeatView) {
        if let Some(snapshot) = repeat_view::final_snapshot(&view.series) {
            self.push_ansi(&snapshot);
        }
        self.push_ansi(&view.pending);
        self.push_note(&repeat_view::summary(&view.series));
        if let Some(ended) = view.ended {
            self.push_styled_note(&ended.text, ended.style);
        }
    }

    /// The repeating view to draw now: the running command's, once it has
    /// a run, or the finished one waiting for a key.
    fn active_view(&self) -> Option<RepeatView> {
        if let Some(run) = &self.running {
            let series = run.series.as_ref().filter(|s| repeat_view::has_runs(s))?;
            return Some(RepeatView {
                series: series.clone(),
                ended: None,
                pending: String::new(),
            });
        }
        self.repeat_view.clone()
    }

    /// Keeps the data of a successful run for Ctrl-Y and `save`. The copy
    /// is the command's own data: no warnings, error text or footer. A
    /// success with no data, or with only the end of its data kept, clears
    /// it; a failed or cancelled run keeps the earlier copy.
    fn keep_copy(&mut self, captured: &Captured, ending: &Ending) {
        if matches!(ending, Ending::Finished(Ok(()))) {
            let data = crate::cli::ansi::strip(&captured.data());
            self.last_output = (!captured.trimmed() && !data.trim().is_empty()).then_some(data);
        }
    }

    /// Shows the output of a command that ran outside the TUI (suspend
    /// mode), and keeps its data as `finish` does for a pane command.
    /// Call it before [`TuiState::record_suspended`], so the text comes
    /// above the note.
    pub fn show_outside(&mut self, copy: &Captured, ending: &Ending) {
        if copy.trimmed() {
            self.push_note(
                "Its output was long: only the end is kept here, and Ctrl-Y and save do not have it.",
            );
        }
        self.keep_copy(copy, ending);
        self.push_ansi(&copy.out());
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
        // As for a pane command: the rule shows the failure until the next
        // line.
        self.failed = matches!(ending, Ending::Finished(Err(_)));
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
        let area = f.area();
        let too_small = area.width < MIN_W || area.height < MIN_H;
        self.too_small.set(too_small);
        if too_small {
            // Everything else stays in the state; only the message is drawn.
            // No pane is drawn, so there is no pane width: the event loop
            // then sets no width override.
            self.pane_width.set(0);
            self.last_pane_height.set(0);
            draw_note_alone(f, &too_small_note());
            return;
        }
        let [pane, input, hint_row, status] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(f.area());

        // Output pane: scrollback plus the live output of a running command.
        let live_out = self.live_output();
        // Drawn only: these lines are not output, so they never reach the
        // scrollback, the copy or the journal.
        let liveness = match &self.running {
            Some(run) => self.liveness_lines(run, color_enabled()),
            None => Vec::new(),
        };
        // The rule and title show the state by color; the word says it too.
        let state = self.pane_state();
        let block = Block::new()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_style(state.style())
            .title(Line::styled(
                format!(" scope v{} ", scope::VERSION),
                state.style().add_modifier(Modifier::BOLD),
            ))
            .title(Line::styled(format!(" {} ", state.word()), state.style()).right_aligned());
        let inner = block.inner(pane);
        self.pane_width.set(inner.width);
        // The matches are found once per draw; the status bar uses them too.
        let view = self.search_view(&live_out);
        if let Some(w) = &self.wizard {
            wizard::draw(f, pane, w);
            self.last_max_top.set(0);
            self.last_pane_height.set(inner.height as usize);
        } else if let (true, Some(g)) = (self.grid_shown, &self.grid) {
            // The watchlist grid takes the pane (#47).
            grid::draw(f, pane, g);
            self.last_max_top.set(0);
            self.last_pane_height.set(inner.height as usize);
        } else if let Some(repeating) = self.active_view() {
            // A repeating run draws its view, not one report per run.
            let status = if self.running.is_some() {
                liveness
            } else {
                Vec::new()
            };
            f.render_widget(block, pane);
            repeat_view::draw(f, inner, &repeating, status);
            self.last_max_top.set(0);
            self.last_pane_height.set(inner.height as usize);
        } else {
            // Fit the lines to the inner width here, not with ratatui wrap: a
            // wrapped row keeps its gutter and a rule is cut. Every line then
            // is one row, so the newest line sits on the last inner row.
            let width = inner.width as usize;
            // While a search is open, matched rows get a marker and reverse
            // video, and `starts` holds the first fitted row of each entry.
            let in_pane_search = matches!(self.overlay, Some(Overlay::Search(_)));
            let (rows, starts) = match (&view, in_pane_search) {
                (Some(v), true) => {
                    let entries: Vec<_> =
                        self.scrollback.iter().chain(&live_out).cloned().collect();
                    marked_rows(&entries, liveness.clone(), &v.matches, v.current, width)
                }
                _ => {
                    let mut lines = Vec::new();
                    if self.dropped_lines > 0 {
                        // Drawn only: the pane says it is not the whole session.
                        lines.push(Line::styled(
                            format!(
                                "… {} older lines dropped (the pane keeps the last {SCROLLBACK_LIMIT})",
                                self.dropped_lines
                            ),
                            SECONDARY,
                        ));
                    }
                    lines.extend(self.scrollback.iter().cloned());
                    lines.extend(live_out.iter().cloned());
                    lines.extend(liveness.iter().cloned());
                    (fit::fit(lines, width), Vec::new())
                }
            };
            let para = Paragraph::new(Text::from(rows));
            let total = para.line_count(inner.width);
            let max_top = total.saturating_sub(inner.height as usize);
            self.last_max_top.set(max_top);
            self.last_pane_height.set(inner.height as usize);
            if let (Some(v), true) = (&view, in_pane_search) {
                let before = self.search_width.replace(width);
                let height = inner.height as usize;
                if self.reveal.replace(false) {
                    self.reveal_match(&starts, v.current, height, max_top);
                } else if before == 0 && self.scroll.get() > 0 {
                    // The search just opened: the gutter changes the wrapping.
                    // Keep the entry at the top of the view where it was.
                    let lines: Vec<_> = self.scrollback.iter().chain(&live_out).cloned().collect();
                    let old = entry_starts(&lines, width);
                    let total =
                        old.last().copied().unwrap_or(0) + fit::fit(liveness.clone(), width).len();
                    let old_max = total.saturating_sub(height);
                    let old_top = old_max - self.scroll.get().min(old_max);
                    let entry = old.partition_point(|&s| s <= old_top).saturating_sub(1);
                    let top = starts.get(entry).copied().unwrap_or(max_top).min(max_top);
                    self.scroll.set(max_top - top);
                } else if before != width {
                    self.reveal_match(&starts, v.current, height, max_top);
                }
            } else {
                self.search_width.set(0);
            }
            let top = max_top.saturating_sub(self.scroll.get().min(max_top));
            f.render_widget(para.block(block).scroll((top as u16, 0)), pane);
        }

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
                        SELECTED
                    } else {
                        Style::new()
                    };
                    Line::styled(c.clone(), style)
                })
                .collect();
            f.render_widget(ratatui::widgets::Clear, area);
            f.render_widget(Paragraph::new(lines).block(Block::bordered()), area);
        }

        if let Some(Overlay::Choice(ch)) = &self.overlay {
            let frame = f.area();
            // The rows that fit above the input line, inside the borders:
            // the scroll follows these, so the selection stays in view.
            let fit = usize::from(input.y.saturating_sub(2)).max(1);
            let rows = ch.options.len().clamp(1, MENU_ROWS).min(fit);
            let first = ch.selected.saturating_sub(rows - 1);
            let height = (rows as u16 + 2).min(input.y);
            // Each row is "NN. option"; the borders add two columns.
            const NUMBER_COLS: usize = 4;
            let widest = ch
                .options
                .iter()
                .map(|o| UnicodeWidthStr::width(o.as_str()) + NUMBER_COLS)
                .chain([UnicodeWidthStr::width(ch.prompt.as_str())])
                .max()
                .unwrap_or(0) as u16;
            let area = ratatui::layout::Rect {
                x: input.x,
                y: input.y.saturating_sub(height),
                width: (widest + 2).min(frame.width.saturating_sub(input.x)),
                height,
            };
            let shown = ch.options.iter().enumerate().skip(first).take(rows);
            let lines: Vec<Line> = shown
                .map(|(i, o)| {
                    let style = if i == ch.selected {
                        SELECTED
                    } else {
                        Style::new()
                    };
                    Line::styled(format!("{:>2}. {o}", i + 1), style)
                })
                .collect();
            f.render_widget(ratatui::widgets::Clear, area);
            f.render_widget(
                Paragraph::new(lines).block(Block::bordered().title(ch.prompt.as_str())),
                area,
            );
        }

        if self.busy_hint {
            f.render_widget(
                Paragraph::new(Line::styled(format!(" {}", BUSY_NOTE), WARN)),
                hint_row,
            );
        } else if let Some(h) = super::hint::hint(&self.input) {
            let mut spans = vec![Span::styled(format!(" {}", h.text), SECONDARY)];
            if let Some(next) = h.next {
                spans.push(Span::styled("  next: ", SECONDARY));
                spans.push(Span::styled(next, KEY));
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
            // Borders, the query line and the legend take four rows.
            let rows = height.saturating_sub(4) as usize;
            let matches = p.matches();
            let first = p.selected.saturating_sub(rows.saturating_sub(1));
            let mut lines = vec![Line::from(format!("> {}", p.query))];
            for (n, &i) in matches.iter().enumerate().skip(first).take(rows) {
                let it = &p.items[i];
                // The marker shows the selection with color off too (#66).
                let (marker, style) = if n == p.selected {
                    (PALETTE_MARKER, SELECTED)
                } else {
                    ("  ", ACCENT)
                };
                lines.push(Line::from(vec![
                    Span::styled(marker, KEY),
                    Span::styled(format!("{:<28}", it.label), style),
                    Span::styled(format!(" {}", it.description), SECONDARY),
                ]));
            }
            lines.push(Line::styled(PALETTE_LEGEND, SECONDARY));
            f.render_widget(ratatui::widgets::Clear, rect);
            f.render_widget(
                Paragraph::new(lines).block(Block::bordered().title(" Commands (Ctrl-K) ")),
                rect,
            );
        }
        f.render_widget(
            Paragraph::new(fit_status(
                &self.status_parts_with(view.map(|v| v.text)),
                status.width as usize,
            )),
            status,
        );
    }

    /// Scrolls so the rows of scrollback entry `current` are in the middle
    /// of the pane. `starts` holds the first fitted row of each entry and,
    /// last, the row after the scrollback.
    fn reveal_match(
        &self,
        starts: &[usize],
        current: Option<usize>,
        height: usize,
        max_top: usize,
    ) {
        let Some(at) = current.filter(|&c| c + 1 < starts.len()) else {
            return;
        };
        let (first, rows) = (starts[at], starts[at + 1] - starts[at]);
        let top = first
            .saturating_sub(height.saturating_sub(rows) / 2)
            .min(max_top);
        self.scroll.set(max_top - top);
    }

    /// The spinner line and, for a repeating command, its run status.
    fn liveness_lines(&self, run: &Running, color: bool) -> Vec<Line<'static>> {
        let dim = SECONDARY;
        let elapsed = run.started.elapsed();
        let mut lines = vec![Line::from(vec![
            Span::styled(spinner_glyph(elapsed, color), WARN),
            Span::styled(
                format!(" running {}… {}s", run.input, elapsed.as_secs()),
                dim,
            ),
        ])];
        if let Some(hook) = run.out.progress_hook() {
            // Copy the record out: the lock is not held while drawing.
            let p = hook.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if p.run > 0 {
                let mut text = match p.total {
                    Some(total) => format!("run {} of {}", p.run, total),
                    None => format!("run {}", p.run),
                };
                if let Some(at) = p.next_at {
                    let wait = at.saturating_duration_since(Instant::now());
                    text.push_str(&format!(" · next in {}s", wait.as_secs_f64().ceil() as u64));
                }
                lines.push(Line::styled(text, dim));
            }
        }
        lines
    }

    /// What Esc does now. An open overlay takes Esc first; then a running
    /// command; idle with no overlay, Esc clears the line. The key handling
    /// and the status bar both read it.
    fn esc_action(&self) -> EscAction {
        match (&self.overlay, &self.running) {
            (Some(Overlay::Choice(_)), _) => EscAction::AnswerChoice,
            (Some(Overlay::Palette(_)), _) => EscAction::ClosePalette,
            (Some(Overlay::Menu(_)), _) => EscAction::CloseMenu,
            (Some(Overlay::Search(_)), _) => EscAction::CloseSearch,
            (Some(Overlay::HistorySearch(_)), _) => EscAction::CloseHistorySearch,
            (None, Some(_)) => EscAction::Cancel,
            (None, None) => EscAction::ClearLine,
        }
    }

    /// Esc: does what [`Self::esc_action`] says. Closing an overlay puts
    /// back what it changed (the scroll for a search, the line for a
    /// history search).
    fn press_esc(&mut self) -> Action {
        match self.esc_action() {
            EscAction::ClosePalette
            | EscAction::CloseMenu
            | EscAction::CloseSearch
            | EscAction::CloseHistorySearch => self.close_overlay(),
            EscAction::AnswerChoice => self.answer_choice(None),
            EscAction::Cancel => return Action::Cancel,
            EscAction::ClearLine => {
                let line = std::mem::take(&mut self.input);
                self.keep_cleared(line);
                self.cursor = 0;
            }
        }
        Action::None
    }

    /// What the status bar says now, before it is fitted to a width. Tests
    /// use it; a draw calls `status_parts_with`.
    #[cfg(test)]
    fn status_parts(&self) -> StatusParts {
        let search = self.search_view(&self.live_output()).map(|v| v.text);
        self.status_parts_with(search)
    }

    /// As [`Self::status_parts`], with the search text a draw already built.
    fn status_parts_with(&self, search: Option<String>) -> StatusParts {
        let ctx = &self.ctx;
        let mut limit = labeled("limit", ctx.limit.to_string());
        for (on, flag) in [(ctx.include_tokens, "+tokens"), (ctx.include_txs, "+txs")] {
            if on {
                limit.extend([sep(), Span::styled(flag, ACCENT)]);
            }
        }
        // Highest priority first: the fit drops from the end.
        let mut context = vec![
            labeled("chain", ctx.chain.clone()),
            labeled("format", format!("{:?}", ctx.format)),
            limit,
        ];
        // A watchlist that runs behind the shell says so first.
        if let (false, Some(g)) = (self.grid_shown, &self.grid) {
            let style = if g.breaches() > 0 { ERR } else { ACCENT };
            context.insert(0, vec![Span::styled(g.status_part(), style)]);
        }
        // The view is clamped to the content when it draws; say so here too.
        let scrolled = self.scroll.get().min(self.last_max_top.get());
        let scrolled = (scrolled > 0).then(|| {
            let mut part = vec![Span::styled(
                format!("scrolled up {} lines", scrolled),
                ACCENT,
            )];
            // PgDn does nothing while a search is open, so the bar does
            // not say it.
            if search.is_none() {
                part.extend([sep(), Span::styled("PgDn", ACCENT)]);
            }
            part
        });
        let esc = self
            .esc_action()
            .hint()
            .map(|hint| Span::styled(hint, Style::new().add_modifier(Modifier::BOLD)));
        let (state, scrolled, command) = match (&self.running, esc) {
            (Some(run), esc) => {
                let mut state = vec![Span::styled(
                    format!("running {:.0}s", run.started.elapsed().as_secs_f64()),
                    WARN,
                )];
                state.extend(esc.into_iter().flat_map(|esc| [sep(), esc]));
                (state, scrolled, Some(run.input.clone()))
            }
            // Idle with an overlay open: what Esc does is the state.
            (None, Some(esc)) => (vec![esc], scrolled, None),
            // Idle: the scroll text is the state.
            (None, None) => (
                scrolled.unwrap_or_else(|| {
                    vec![
                        Span::styled("help", ACCENT),
                        sep(),
                        Span::styled("exit", ACCENT),
                    ]
                }),
                None,
                None,
            ),
        };
        StatusParts {
            context,
            state,
            scrolled,
            search: search.map(|text| vec![Span::styled(text, ACCENT)]),
            command,
        }
    }
}

/// Styled text: each span carries the theme style of its part.
type Spans = Vec<Span<'static>>;

/// The parts of the status bar, in priority order. Each part is built
/// with its styles, so a change of wording cannot change them.
struct StatusParts {
    /// Chain, format and limit: each a dim label and an accent value.
    /// Dropped from the end when the bar is narrow.
    context: Vec<Spans>,
    /// `running Ns · Esc cancels` (or what Esc does with an open overlay),
    /// `scrolled up N lines · PgDn` or `help · exit`. Kept at every width.
    state: Spans,
    /// `scrolled up N lines · PgDn` while a command runs. Idle, it is the
    /// state. Dropped after the context and before the command.
    scrolled: Option<Spans>,
    /// `search: alpha 1/2` or `search: alpha no match` while a search is
    /// open. Kept before the command text; dropped only after it.
    search: Option<Spans>,
    /// The command that runs. The first of these parts to go once the
    /// context is gone.
    command: Option<String>,
}

/// The spinner frames with color on.
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The spinner frames with color off, where a braille glyph may not draw.
const SPINNER_ASCII: [&str; 4] = ["-", "\\", "|", "/"];

/// The spinner moves one frame per redraw tick. It adds no timer of its own.
fn spinner_glyph(elapsed: std::time::Duration, color: bool) -> &'static str {
    let step = (elapsed.as_millis() / super::LIVE_REDRAW.as_millis()) as usize;
    if color {
        SPINNER_FRAMES[step % SPINNER_FRAMES.len()]
    } else {
        SPINNER_ASCII[step % SPINNER_ASCII.len()]
    }
}

/// Columns around a command in the bar: ` · ` before it, a backtick on
/// each side.
const COMMAND_FRAME_COLS: usize = 5;

/// The shortest cut command worth showing: one column and the `…`.
const MIN_CUT_COMMAND_COLS: usize = 2;

/// The status bar separator between parts.
const STATUS_SEP: &str = " · ";

/// The separator span: dim.
fn sep() -> Span<'static> {
    Span::styled(STATUS_SEP, SECONDARY)
}

/// A context part: the label dim, the value in the accent.
fn labeled(label: &str, value: String) -> Spans {
    vec![
        Span::styled(format!("{label} "), SECONDARY),
        Span::styled(value, ACCENT),
    ]
}

/// The width of `spans` in display columns.
fn spans_cols(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|sp| cols(&sp.content)).sum()
}

/// The parts joined in order, each run of `parts` after the first led by
/// the separator.
fn joined<'a>(parts: impl IntoIterator<Item = &'a Spans>) -> Spans {
    let mut out = Spans::new();
    for (i, part) in parts.into_iter().enumerate() {
        if i > 0 {
            out.push(sep());
        }
        out.extend(part.iter().cloned());
    }
    out
}

/// Cuts `spans` to `max` columns with a `…`, keeping each span's style.
fn cut_spans(spans: Spans, max: usize) -> Spans {
    let full: String = spans.iter().map(|sp| sp.content.as_ref()).collect();
    let cut = cut_with_ellipsis(&full, max);
    if cut == full {
        return spans;
    }
    let ellipsis = cut.ends_with(ELLIPSIS);
    // The cut is a prefix of the text, then the `…`.
    let mut keep = cut.len() - if ellipsis { ELLIPSIS.len() } else { 0 };
    let mut out = Spans::new();
    for sp in spans {
        if keep == 0 {
            break;
        }
        let n = keep.min(sp.content.len());
        out.push(Span::styled(sp.content[..n].to_string(), sp.style));
        keep -= n;
    }
    if ellipsis {
        out.push(Span::styled(ELLIPSIS, SECONDARY));
    }
    out
}

/// Fits the status bar to `width` display columns. Drops limit, then
/// format, then chain, then the command text. The state text stays at
/// every width, and a `…` shows that something was cut or dropped.
fn fit_status(parts: &StatusParts, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }
    let lead = || Span::raw(" ");
    let more = || Span::styled(format!(" {ELLIPSIS}"), SECONDARY);
    let command = |c: &str| vec![sep(), Span::styled(format!("`{c}`"), ACCENT)];
    let note = |part: &Option<Spans>| {
        part.as_ref()
            .map(|p| joined([&Spans::new(), p]))
            .unwrap_or_default()
    };
    let scroll_note = note(&parts.scrolled);
    let search_note = note(&parts.search);
    let fits = |line: &Spans| spans_cols(line) <= width;
    let total = parts.context.len();
    for kept in (0..=total).rev() {
        let mut line = vec![lead()];
        if kept > 0 {
            line.extend(joined(&parts.context[..kept]));
            line.push(Span::raw("  "));
        }
        line.extend(parts.state.iter().cloned());
        line.extend(scroll_note.iter().cloned());
        line.extend(search_note.iter().cloned());
        if let Some(c) = &parts.command {
            line.extend(command(c));
        }
        if kept < total {
            line.push(more());
        }
        if fits(&line) {
            return Line::from(line);
        }
    }
    // The whole context is gone. Cut the command text to what is left. The
    // search note stays until the command is gone.
    let both = [scroll_note.clone(), search_note.clone()].concat();
    for note in [both, search_note.clone()] {
        let used = 1 + spans_cols(&parts.state) + spans_cols(&note) + COMMAND_FRAME_COLS;
        if let Some(c) = &parts.command
            && width >= used + MIN_CUT_COMMAND_COLS
        {
            let cut = cut_with_ellipsis(c, width - used);
            let line = [vec![lead()], parts.state.clone(), note, command(&cut)].concat();
            return Line::from(line);
        }
    }
    // The command is gone too. A `…` after the state shows it, if it fits.
    let head = [vec![lead()], parts.state.clone()].concat();
    if parts.search.is_some() {
        let line = [head.clone(), search_note, vec![more()]].concat();
        if fits(&line) {
            return Line::from(line);
        }
    }
    let line = [head.clone(), vec![more()]].concat();
    if fits(&line) {
        return Line::from(line);
    }
    Line::from(cut_spans(head, width))
}

/// The first fitted row of each entry at `width`, and the row after the
/// last entry.
fn entry_starts(lines: &[Line<'static>], width: usize) -> Vec<usize> {
    let mut rows = 0;
    let mut starts = Vec::with_capacity(lines.len() + 1);
    for line in lines {
        starts.push(rows);
        rows += fit::fit(vec![line.clone()], width).len();
    }
    starts.push(rows);
    starts
}

/// Fits the scrollback to `width` for a search: every row gets a two-column
/// gutter, `▶ ` for a row of a match and blanks for the rest. A match also
/// gets reverse video, and the current one bold and underline, so the
/// marker and the style both show it. Returns the rows and the first row
/// of each scrollback entry, with the row after the scrollback last.
fn marked_rows(
    scrollback: &[Line<'static>],
    extra: Vec<Line<'static>>,
    matches: &[usize],
    current: Option<usize>,
    width: usize,
) -> (Vec<Line<'static>>, Vec<usize>) {
    let text_width = width.saturating_sub(cols(MATCH_MARK)).max(1);
    let gutter = |mut row: Line<'static>, hit: bool, now: bool| {
        let mark = if hit { MATCH_MARK } else { MATCH_BLANK };
        row.spans.insert(0, Span::raw(mark));
        if hit {
            let mut style = MATCH;
            if now {
                style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
            }
            row = row.patch_style(style);
        }
        row
    };
    let mut rows = Vec::new();
    let mut starts = Vec::with_capacity(scrollback.len() + 1);
    for (i, line) in scrollback.iter().enumerate() {
        starts.push(rows.len());
        let hit = matches.binary_search(&i).is_ok();
        for row in fit::fit(vec![line.clone()], text_width) {
            rows.push(gutter(row, hit, current == Some(i)));
        }
    }
    starts.push(rows.len());
    for row in fit::fit(extra, text_width) {
        rows.push(gutter(row, false, false));
    }
    (rows, starts)
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
    use ratatui::style::Color;

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

    fn asked(s: &mut TuiState, options: &[&str]) -> tokio::sync::oneshot::Receiver<Option<usize>> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        *s.asker.lock().unwrap() = Some(crate::cli::output::Ask {
            prompt: "Choose one:".into(),
            options: options.iter().map(|o| o.to_string()).collect(),
            reply,
        });
        s.poll_ask();
        rx
    }

    #[test]
    fn test_the_choice_list_answers_with_the_selected_row() {
        let mut s = state();
        let mut rx = asked(&mut s, &["ethereum 0xA USDN", "bsc 0xB USDN"]);
        assert!(matches!(s.overlay, Some(Overlay::Choice(_))));
        s.handle_key(key(KeyCode::Down));
        s.handle_key(key(KeyCode::Enter));
        assert_eq!(rx.try_recv().unwrap(), Some(1));
        assert!(s.overlay.is_none());
    }

    #[test]
    fn test_escape_answers_none_and_does_not_cancel_the_command() {
        let mut s = state();
        let mut rx = asked(&mut s, &["a", "b"]);
        assert!(matches!(s.handle_key(key(KeyCode::Esc)), Action::None));
        assert_eq!(rx.try_recv().unwrap(), None);
        assert!(s.overlay.is_none());
    }

    #[test]
    fn test_cancelling_the_command_closes_the_choice() {
        // Review focus 1: the command is gone, so is its receiver.
        let mut s = state();
        let rx = asked(&mut s, &["a", "b"]);
        drop(rx);
        s.finish(Ending::Cancelled);
        assert!(s.overlay.is_none());
        assert!(s.asker.lock().unwrap().is_none());
    }

    #[test]
    fn test_a_long_choice_scrolls_with_the_selection() {
        // Review focus 2: 20 options; the 15th must be on screen.
        let mut s = state();
        let options: Vec<String> = (1..=20).map(|i| format!("chain{i} 0x{i} T")).collect();
        let refs: Vec<&str> = options.iter().map(String::as_str).collect();
        let _rx = asked(&mut s, &refs);
        for _ in 0..14 {
            s.handle_key(key(KeyCode::Down));
        }
        let text = screen(&s, 100, 30);
        assert!(text.contains("chain15 0x15 T"), "{text}");
        assert!(text.contains("Choose one:"), "{text}");
    }

    #[test]
    fn test_the_choice_keeps_the_selection_in_view_at_the_smallest_size() {
        // Review focus 3: at MIN_W x MIN_H only a few rows fit above the
        // input line; the selected one must be among them.
        let mut s = state();
        let options: Vec<String> = (1..=8).map(|i| format!("opt{i}")).collect();
        let refs: Vec<&str> = options.iter().map(String::as_str).collect();
        let _rx = asked(&mut s, &refs);
        for _ in 0..5 {
            s.handle_key(key(KeyCode::Down));
        }
        let text = screen(&s, MIN_W, MIN_H);
        assert!(text.contains("opt6"), "{text}");
    }

    #[test]
    fn test_a_hidden_choice_is_not_answered_by_enter() {
        let mut s = state();
        let mut rx = asked(&mut s, &["a", "b"]);
        let _ = screen(&s, 20, 6); // too small: the list is not drawn
        s.handle_key(key(KeyCode::Enter));
        assert!(rx.try_recv().is_err(), "no answer yet");
        assert!(matches!(s.overlay, Some(Overlay::Choice(_))));
        s.handle_key(key(KeyCode::Esc));
        assert_eq!(rx.try_recv().unwrap(), None);
    }

    #[test]
    fn test_a_paste_leaves_the_choice_open() {
        let mut s = state();
        let mut rx = asked(&mut s, &["a", "b"]);
        s.paste("USDC");
        assert!(matches!(s.overlay, Some(Overlay::Choice(_))));
        assert!(rx.try_recv().is_err(), "no answer yet");
        assert!(s.input.is_empty());
    }

    #[test]
    fn test_a_question_puts_back_the_history_search_draft() {
        let mut s = state();
        type_str(&mut s, "draft");
        s.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert!(matches!(s.overlay, Some(Overlay::HistorySearch(_))));
        let _rx = asked(&mut s, &["a", "b"]);
        assert!(matches!(s.overlay, Some(Overlay::Choice(_))));
        assert_eq!(s.input, "draft");
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

    /// Starts a pane command and hands it `n` repeat runs, as
    /// `repeat::run` does in the TUI. Run `fail` is unhealthy.
    fn repeating(s: &mut TuiState, n: u64, fail: u64) {
        let inv = Invocation {
            command: venues_list(),
            route: Route::Pane,
            ai: false,
            argv: vec![],
        };
        let out = s.start("market summary USDC --every 1s", &inv);
        let metrics = scope::metrics::summary_metrics(&scope::market::HealthThresholds::default());
        for run in 1..=n {
            out.record_run(
                &metrics,
                crate::cli::output::RunRecord {
                    run,
                    at: Utc::now(),
                    sample: scope::metrics::Sample {
                        values: vec![Some(1.0); metrics.len()],
                        healthy: run != fail,
                        failed_checks: vec![format!("spread {run}%")],
                    },
                    snapshot: format!("SNAPSHOT {run}\n"),
                },
            );
        }
    }

    #[test]
    fn test_a_repeat_run_draws_the_view_not_one_report_per_run() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = state();
        repeating(&mut s, 3, 2);
        let text = screen(&s, 120, 36);
        assert!(text.contains("SNAPSHOT 3"), "{text}");
        assert!(!text.contains("SNAPSHOT 1"), "{text}");
        assert!(text.contains("Trends · run 3"), "{text}");
        assert!(text.contains("run 2: spread 2%"), "{text}");
        s.finish(Ending::Cancelled);
    }

    #[test]
    fn test_the_view_stays_after_esc_and_the_next_key_leaves_it_once() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = state();
        repeating(&mut s, 3, 0);
        assert_eq!(s.handle_key(key(KeyCode::Esc)), Action::Cancel);
        s.finish(Ending::Cancelled);
        let text = screen(&s, 120, 36);
        assert!(text.contains("Cancelled after"), "{text}");
        assert!(text.contains(repeat_view::LEAVE_HINT), "{text}");
        assert!(!s.scrollback_text().contains("SNAPSHOT"), "not yet pushed");
        // The next key leaves the view and still types: nothing is lost.
        assert_eq!(s.handle_key(key(KeyCode::Char('x'))), Action::None);
        assert_eq!(s.input, "x");
        let back = s.scrollback_text();
        assert_eq!(back.matches("SNAPSHOT 3").count(), 1, "{back}");
        assert!(!back.contains("SNAPSHOT 1"), "{back}");
        assert!(back.contains("3 runs · none failed"), "{back}");
        assert!(back.contains("Cancelled after"), "{back}");
        assert!(!screen(&s, 120, 36).contains("Trends"), "the view is gone");
    }

    #[test]
    fn test_copy_and_save_take_the_final_snapshot_and_the_series() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = state();
        repeating(&mut s, 2, 0);
        s.finish(Ending::Finished(Ok(())));
        assert_eq!(s.last_output.as_deref(), Some("SNAPSHOT 2\n"));
        let csv = s
            .last_series
            .as_ref()
            .and_then(repeat_view::series_csv)
            .unwrap();
        assert_eq!(csv.lines().count(), 3, "header and two runs: {csv}");
    }

    fn alt(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
    }

    #[test]
    fn test_readline_word_moves_and_kill_to_end() {
        let mut s = state();
        type_str(&mut s, "market summary USDC");
        s.handle_key(alt('b'));
        assert_eq!(s.cursor, 15, "Alt-B: start of USDC");
        s.handle_key(alt('b'));
        assert_eq!(s.cursor, 7, "Alt-B: start of summary");
        s.handle_key(alt('f'));
        assert_eq!(s.cursor, 14, "Alt-F: end of summary");
        s.handle_key(ctrl('b'));
        assert_eq!(s.cursor, 13, "Ctrl-B: one left");
        s.handle_key(alt('k'));
        assert_eq!(s.input, "market summar", "Alt-K: deletes to the end");
    }

    #[test]
    fn test_up_brings_back_a_line_that_esc_cleared() {
        let mut s = state();
        type_str(&mut s, "market summary USDC --venue binance --every 30s");
        s.handle_key(key(KeyCode::Esc));
        assert_eq!(s.input, "");
        s.handle_key(key(KeyCode::Up));
        assert_eq!(s.input, "market summary USDC --venue binance --every 30s");
    }

    #[test]
    fn test_clear_empties_the_pane_and_reset_resets_the_context() {
        let mut s = state();
        s.push_note("old output");
        s.ctx.limit = 7;
        s.submit("clear", &Config::default());
        assert!(!s.scrollback_text().contains("old output"));
        assert_eq!(s.ctx.limit, 7, "clear leaves the context");
        s.submit("reset", &Config::default());
        assert_eq!(s.ctx.limit, SessionContext::default().limit);
    }

    #[test]
    fn test_the_pane_says_when_it_dropped_old_lines() {
        let mut s = state();
        let big: String = (0..SCROLLBACK_LIMIT + 7)
            .map(|i| format!("l{i}\n"))
            .collect();
        s.push_ansi(&big);
        assert!(s.dropped_lines > 0);
        let rows = 200;
        s.scroll.set(usize::MAX / 2);
        let text = screen(&s, 100, rows);
        assert!(text.contains("older lines dropped"), "{text}");
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
        // ↑ brings back the line Esc cleared (#67).
        s.handle_key(key(KeyCode::Up));
        assert_eq!(s.input, "bcde");
        // History keys with no history (and nothing cleared) do nothing.
        s.cleared = None;
        s.set_input(String::new());
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
        // A page is the pane height less one row: inner height 8, page 7.
        assert!(text.contains("scrolled up 7 lines"), "{}", text);
    }

    /// The status bar text at a width. It is fitted directly: a screen
    /// below the minimum size draws no status bar.
    fn status_row(s: &TuiState, w: u16) -> String {
        let line = fit_status(&s.status_parts(), w as usize);
        let text: String = line.spans.iter().map(|sp| sp.content.as_ref()).collect();
        format!("{:<width$}", text, width = w as usize)
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

    /// The status bar text of `s` at a wide width.
    fn status_text(s: &TuiState) -> String {
        let line = fit_status(&s.status_parts(), 160);
        line.spans.iter().map(|sp| sp.content.as_ref()).collect()
    }

    #[test]
    fn test_esc_hint_says_what_esc_does_with_each_overlay_open() {
        // While a command runs, Esc first closes an open overlay. The bar
        // must name that, not promise a cancel that Esc does not do.
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        type Open = fn(&mut TuiState);
        let opens: [(&str, Open); 4] = [
            ("Esc closes palette", |s| {
                s.handle_key(ctrl('k'));
            }),
            ("Esc closes menu", |s| {
                type_str(s, "ad");
                s.handle_key(key(KeyCode::Tab));
            }),
            ("Esc closes search", |s| {
                s.handle_key(ctrl('f'));
            }),
            ("Esc closes history search", |s| {
                s.handle_key(ctrl('r'));
            }),
        ];
        for (hint, open) in opens {
            let mut s = vocab_state();
            let inv = Invocation {
                command: venues_list(),
                route: Route::Pane,
                ai: false,
                argv: vec![],
            };
            let _out = s.start("venues list", &inv);
            screen(&s, 80, 24);
            open(&mut s);
            assert!(s.overlay.is_some(), "{hint}: no overlay");
            let bar = status_text(&s);
            assert!(bar.contains(hint), "{hint}: {bar:?}");
            assert!(!bar.contains("Esc cancels"), "{hint}: {bar:?}");
            // The bar tells the truth: Esc closes the overlay and the
            // command keeps running.
            assert_eq!(s.handle_key(key(KeyCode::Esc)), Action::None, "{hint}");
            assert!(s.overlay.is_none(), "{hint}");
            assert!(status_text(&s).contains("Esc cancels"), "{hint}");
        }
    }

    #[test]
    fn test_idle_esc_hint_shows_only_while_an_overlay_is_open() {
        let mut s = state();
        assert!(!status_text(&s).contains("Esc"), "{}", status_text(&s));
        screen(&s, 80, 24);
        s.handle_key(ctrl('k'));
        assert!(status_text(&s).contains("Esc closes palette"));
        s.handle_key(key(KeyCode::Esc));
        assert!(status_text(&s).contains("help"), "{}", status_text(&s));
    }

    #[test]
    fn test_trimmed_outside_output_is_not_kept_for_copy_or_save() {
        // Only the end of a long outside run is kept. Ctrl-Y or save must
        // not hand out that part as if it were the whole output.
        let mut s = state();
        let (o, done) = Output::capture_merged();
        crate::outln!(o, "earlier data").unwrap();
        s.show_outside(&done, &Ending::Finished(Ok(())));
        assert_eq!(s.last_output.as_deref(), Some("earlier data\n"));
        let (terminal, _screen) = Output::capture();
        let (o, long) = terminal.teed(64);
        for i in 0..100 {
            crate::outln!(o, "row {i}").unwrap();
        }
        assert!(long.trimmed());
        s.show_outside(&long, &Ending::Finished(Ok(())));
        assert_eq!(s.last_output, None, "neither the part nor the old copy");
        assert!(
            s.scrollback_text()
                .contains("only the end is kept here, and Ctrl-Y and save do not have it."),
            "{}",
            s.scrollback_text()
        );
        assert!(matches!(s.handle_key(ctrl('y')), Action::None));
        assert!(s.scrollback_text().contains("nothing to copy yet"));
    }

    #[test]
    fn test_an_outside_run_that_prompted_keeps_only_its_data_for_copy() {
        // `crawl USDC` asks to pick a token and to save it. Ctrl-Y and save
        // after it must get the report, not the list and the questions.
        let mut s = state();
        let (terminal, _screen) = Output::capture();
        let (o, copy) = terminal.teed(1 << 20);
        let pick = crate::cli::prompt::pick(
            &mut std::io::Cursor::new(b"1\n".to_vec()),
            &o,
            "1  USDC\n",
            "token",
            1,
        );
        assert_eq!(pick.unwrap(), 0);
        crate::cli::prompt::ask(
            &mut std::io::Cursor::new(b"n\n".to_vec()),
            &o,
            "Save this token for future use? [y/N]: ",
        )
        .unwrap();
        crate::outln!(o, "report").unwrap();
        s.show_outside(&copy, &Ending::Finished(Ok(())));
        assert_eq!(s.last_output.as_deref(), Some("report\n"));
        let shown = s.scrollback_text();
        assert!(
            shown.contains("Select token (1-1)"),
            "the user still sees it"
        );
    }

    #[test]
    fn test_a_failed_outside_command_sets_the_failed_pane_state() {
        // Same as a pane command: the rule says the last command failed.
        let mut s = state();
        let err = Ending::Finished(Err(scope::error::ScopeError::Chain("boom".into())));
        s.record_suspended(
            "crawl USDC",
            &[],
            std::time::Duration::from_secs(1),
            String::new(),
            &err,
        );
        assert_eq!(s.pane_state(), PaneState::Failed);
        for ok in [Ending::Finished(Ok(())), Ending::Cancelled] {
            s.submit("crawl USDC", &Config::default());
            s.record_suspended(
                "crawl USDC",
                &[],
                std::time::Duration::from_secs(1),
                String::new(),
                &ok,
            );
            assert_eq!(s.pane_state(), PaneState::Idle);
        }
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
        screen(&s, 40, 12);
        s.handle_key(key(KeyCode::PageUp));
        let row = status_row(&s, 40);
        assert!(row.contains("scrolled up 4 lines"), "{row:?}");
    }

    /// 30 lines in a 60x15 screen: the pane shows 8 rows, 22 can scroll.
    fn scrollable() -> TuiState {
        let mut s = state();
        for i in 0..30 {
            s.push_ansi(&format!("line {}\n", i));
        }
        screen(&s, 60, 15);
        s
    }

    #[test]
    fn test_page_up_stops_at_the_top_so_page_down_always_moves() {
        let mut s = scrollable();
        for _ in 0..50 {
            s.handle_key(key(KeyCode::PageUp));
        }
        let top = screen(&s, 60, 15);
        assert!(top.contains("scrolled up 22 lines"), "{top}");
        s.handle_key(key(KeyCode::PageDown));
        let after = screen(&s, 60, 15);
        assert_ne!(top, after, "PageDown must change the view");
        assert!(after.contains("scrolled up 15 lines"), "{after}");
    }

    #[test]
    fn test_ctrl_home_and_ctrl_end_jump_to_the_top_and_the_bottom() {
        let mut s = scrollable();
        s.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
        assert!(screen(&s, 60, 15).contains("line 0"));
        assert!(screen(&s, 60, 15).contains("scrolled up 22 lines"));
        s.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL));
        let text = screen(&s, 60, 15);
        assert!(text.contains("line 29"), "{text}");
        assert!(!text.contains("scrolled up"), "{text}");
    }

    #[test]
    fn test_end_on_an_empty_input_jumps_to_the_bottom_else_moves_the_cursor() {
        let mut s = scrollable();
        s.handle_key(key(KeyCode::PageUp));
        s.handle_key(key(KeyCode::End));
        assert_eq!(s.scroll.get(), 0);
        type_str(&mut s, "abc");
        s.handle_key(key(KeyCode::Home));
        s.handle_key(key(KeyCode::PageUp));
        s.handle_key(key(KeyCode::End));
        assert_eq!(s.cursor, 3, "End moves the cursor on a non-empty input");
        assert_eq!(s.scroll.get(), 7, "and leaves the view alone");
    }

    #[test]
    fn test_keys_that_edit_the_input_snap_the_view_to_the_bottom() {
        for edit in ["char", "backspace", "delete", "paste"] {
            let mut s = scrollable();
            type_str(&mut s, "ab");
            s.handle_key(key(KeyCode::Left));
            s.handle_key(key(KeyCode::PageUp));
            assert!(s.scroll.get() > 0, "{edit}");
            match edit {
                "char" => drop(s.handle_key(key(KeyCode::Char('a')))),
                "backspace" => drop(s.handle_key(key(KeyCode::Backspace))),
                "delete" => drop(s.handle_key(key(KeyCode::Delete))),
                _ => s.paste("x"),
            }
            assert_eq!(s.scroll.get(), 0, "{edit}");
        }
    }

    #[test]
    fn test_navigation_keys_and_modifiers_alone_do_not_snap_the_view() {
        let mut s = scrollable();
        s.handle_key(key(KeyCode::PageUp));
        for k in [
            key(KeyCode::Left),
            key(KeyCode::Right),
            key(KeyCode::Home),
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
            key(KeyCode::Tab),
        ] {
            s.handle_key(k);
            assert_eq!(s.scroll.get(), 7, "{k:?}");
        }
    }

    #[test]
    fn test_status_shows_the_running_state_and_the_scroll_state_together() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let (mut s, _out) = running_state("market summary USDC --every 2s");
        for i in 0..40 {
            s.push_ansi(&format!("line {}\n", i));
        }
        screen(&s, 120, 12);
        s.handle_key(key(KeyCode::PageUp));
        let row = status_row(&s, 120);
        assert!(row.contains("running "), "{row:?}");
        assert!(row.contains("scrolled up 4 lines"), "{row:?}");
        // Narrow: the cancel hint is still kept.
        let row = status_row(&s, 40);
        assert!(row.contains("Esc cancels"), "{row:?}");
    }

    const TOO_SMALL: &str = "Terminal too small: need at least 40×10";

    #[test]
    fn test_the_too_small_text_is_the_spec_text() {
        assert_eq!(too_small_note(), TOO_SMALL);
    }

    #[test]
    fn test_below_the_minimum_size_only_the_too_small_line_is_drawn() {
        let mut s = state();
        s.push_ansi("some output\n");
        type_str(&mut s, "draft");
        // 39 columns hold the whole line.
        let text = screen(&s, 39, 9);
        assert!(text.contains(TOO_SMALL), "{text}");
        let drawn: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(drawn.len(), 1, "only the message: {text}");
        assert!(!text.contains("draft"), "{text}");
        assert_eq!(s.input, "draft", "state is kept");
        let text = screen(&s, 40, 10);
        assert!(!text.contains(TOO_SMALL), "{text}");
        assert!(text.contains("draft"), "{text}");
        assert!(screen(&s, 40, 9).contains(TOO_SMALL));
        assert!(screen(&s, 39, 10).contains(TOO_SMALL));
    }

    #[test]
    fn test_the_too_small_message_wraps_on_words_and_is_centered() {
        let s = state();
        let text = screen(&s, 30, 8);
        let rows: Vec<&str> = text.lines().collect();
        let at = rows
            .iter()
            .position(|r| r.contains("Terminal too small: need at"))
            .expect(&text);
        assert!(rows[at + 1].contains("least 40×10"), "{text}");
        assert_eq!(at, 3, "centered vertically: {text}");
        assert!(rows[at].starts_with(' '), "centered horizontally: {text}");
        // Only a height that cannot hold the lines cuts the message.
        assert!(screen(&s, 30, 1).contains("Terminal too small"));
        for w in 0..12 {
            for h in 0..4 {
                screen(&s, w, h);
            }
        }
    }

    #[test]
    fn test_too_small_clears_the_pane_width_so_no_override_is_set() {
        let s = state();
        screen(&s, 80, 24);
        assert!(s.pane_width().is_some());
        screen(&s, 30, 8);
        assert_eq!(s.pane_width(), None);
    }

    #[test]
    fn test_overlays_do_not_act_while_the_terminal_is_too_small() {
        let mut s = state();
        type_str(&mut s, "dra");
        s.handle_key(ctrl('k'));
        assert!(s.overlay.is_some());
        screen(&s, 30, 8);
        assert!(matches!(s.handle_key(key(KeyCode::Enter)), Action::None));
        assert!(s.overlay.is_none(), "the overlay is closed");
        assert_eq!(s.input, "dra", "the palette did not fill the input");
        // Another key closes an overlay opened before the shrink, and
        // does not edit.
        screen(&s, 80, 24);
        s.handle_key(ctrl('k'));
        screen(&s, 30, 8);
        s.handle_key(key(KeyCode::Char('x')));
        assert!(s.overlay.is_none());
        assert_eq!(s.input, "dra");
        // Esc closes it too.
        screen(&s, 80, 24);
        s.handle_key(ctrl('k'));
        screen(&s, 30, 8);
        s.handle_key(key(KeyCode::Esc));
        assert!(s.overlay.is_none());
        assert_eq!(s.input, "dra");
    }

    #[test]
    fn test_enter_does_nothing_while_too_small_and_a_command_runs() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let (mut s, _out) = running_state("market summary USDC --every 2s");
        type_str(&mut s, "ctx");
        screen(&s, 30, 8);
        assert!(matches!(s.handle_key(key(KeyCode::Enter)), Action::None));
        assert_eq!(s.input, "ctx");
        assert!(s.running.is_some());
    }

    #[test]
    fn test_enter_does_nothing_while_the_terminal_is_too_small() {
        let mut s = state();
        type_str(&mut s, "help");
        screen(&s, 30, 8);
        assert!(matches!(s.handle_key(key(KeyCode::Enter)), Action::None));
        assert_eq!(s.input, "help", "the draft is not lost or run");
        // Keys still edit the draft.
        s.handle_key(key(KeyCode::Char('x')));
        assert_eq!(s.input, "helpx");
        // Once the terminal is big enough, Enter runs it again.
        screen(&s, 80, 24);
        assert!(matches!(
            s.handle_key(key(KeyCode::Enter)),
            Action::Submit(_)
        ));
    }

    #[test]
    fn test_fit_status_never_panics_and_never_exceeds_the_width() {
        // Width math runs on every redraw: width 0 and tiny widths must
        // give a line, and no line may be wider than the bar.
        let text = |t: &str| vec![Span::styled(t.to_string(), ACCENT)];
        let mut parts = StatusParts {
            context: vec![
                labeled("chain", "auto".into()),
                labeled("format", "Table".into()),
                labeled("limit", "100".into()),
            ],
            state: text("running 12s · Esc cancels"),
            scrolled: Some(text("scrolled up 7 lines · PgDn")),
            search: None,
            command: Some("market summary ✓✓ --every 2s".into()),
        };
        for search in [None, Some(text("search: alpha ✓ 1/2"))] {
            parts.search = search;
            for w in 0..=130 {
                let line = fit_status(&parts, w);
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                assert!(cols(&text) <= w, "{w}: {text:?}");
            }
        }
        parts.search = None;
        let text: String = fit_status(&parts, 0)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.is_empty());
    }

    use crate::cli::output::RunProgress;
    use std::time::Duration;

    #[test]
    fn test_running_pane_shows_a_spinner_and_the_repeat_status() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let (s, out) = running_state("market summary USDC --every 2s");
        let hook = out.progress_hook().expect("the pane output has a hook");
        *hook.lock().unwrap() = RunProgress {
            run: 2,
            total: Some(4),
            next_at: Some(Instant::now() + Duration::from_secs(3)),
        };
        let text = screen(&s, 80, 12);
        assert!(text.contains("run 2 of 4 · next in 3s"), "{text}");
        assert!(
            text.contains("running market summary USDC --every 2s…"),
            "{text}"
        );
        // The glyph leads the line: braille with color, ASCII without.
        let row = text.lines().find(|l| l.contains("running market")).unwrap();
        let glyph = row.trim_start().chars().next().unwrap().to_string();
        assert!(
            SPINNER_FRAMES.contains(&glyph.as_str()) || SPINNER_ASCII.contains(&glyph.as_str()),
            "{row:?}"
        );
    }

    #[test]
    fn test_spinner_uses_ascii_without_color_and_advances_with_time() {
        assert_eq!(spinner_glyph(Duration::from_millis(0), false), "-");
        assert_eq!(spinner_glyph(Duration::from_millis(100), false), "\\");
        assert_eq!(spinner_glyph(Duration::from_millis(200), false), "|");
        assert_eq!(spinner_glyph(Duration::from_millis(300), false), "/");
        assert_eq!(spinner_glyph(Duration::from_millis(400), false), "-");
        assert_ne!(
            spinner_glyph(Duration::from_millis(0), true),
            spinner_glyph(Duration::from_millis(100), true)
        );
    }

    #[test]
    fn test_spinner_and_run_status_never_enter_the_record() {
        // They are drawn, not output: the copy, the scrollback and the
        // journal must not hold them.
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::at(dir.path().join("history"), dir.path().join("events"));
        let mut s = TuiState::new(SessionContext::default(), journal, Vocab::default());
        let inv = Invocation {
            command: venues_list(),
            route: Route::Pane,
            ai: false,
            argv: vec![],
        };
        let out = s.start("market summary USDC --every 2s", &inv);
        crate::outln!(out, "data line").unwrap();
        *out.progress_hook().unwrap().lock().unwrap() = RunProgress {
            run: 2,
            total: Some(4),
            next_at: Some(Instant::now() + Duration::from_secs(3)),
        };
        let drawn = screen(&s, 80, 12);
        assert!(drawn.contains("run 2 of 4"), "{drawn}");
        s.finish(Ending::Finished(Ok(())));
        let kept = format!(
            "{}\n{}",
            s.scrollback_text(),
            s.last_output.clone().unwrap_or_default()
        );
        assert!(kept.contains("data line"));
        for gone in [
            "run 2 of",
            "next in",
            "running market",
            "⠋",
            "Running market",
        ] {
            assert!(!kept.contains(gone), "{gone:?} leaked into: {kept}");
        }
        let events = std::fs::read_to_string(dir.path().join("events")).unwrap();
        assert!(
            !events.contains("run 2 of") && !events.contains("next in"),
            "{events}"
        );
        // And the finished screen shows none of it.
        let after = screen(&s, 80, 12);
        assert!(!after.contains("run 2 of"), "{after}");
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
        assert!(row.starts_with("ℹ hello"), "{row:?}");
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
            regex::Regex::new(r"^✓ Done in \d+\.\ds\.$")
                .unwrap()
                .is_match(&text),
            "{text}"
        );
        assert_eq!(last_line(&s).style, OK);
    }

    #[test]
    fn test_failed_run_ends_with_a_red_failed_footer() {
        let mut s = state();
        let err = Ending::Finished(Err(scope::error::ScopeError::Chain("boom".into())));
        s.start("x", &pane_invocation());
        s.finish_with(err);
        let line = last_line(&s);
        let text = line.to_string();
        assert!(
            regex::Regex::new(r"^✗ Failed in \d+\.\ds\.$")
                .unwrap()
                .is_match(&text),
            "{text}"
        );
        assert_eq!(line.style, ERR.add_modifier(Modifier::BOLD));
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
        assert_eq!(last_line(&s).style, WARN);
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

    // ------------------------------------------------------------- search

    fn alpha_state() -> TuiState {
        let mut s = state();
        s.push_ansi("alpha\nbeta\nalpha2\n");
        screen(&s, 80, 20);
        s
    }

    fn open_search(s: &mut TuiState, query: &str) {
        s.handle_key(ctrl('f'));
        type_str(s, query);
        screen(s, 80, 20);
    }

    #[test]
    fn test_search_goes_to_the_newest_match_and_n_and_shift_n_walk_the_matches() {
        let mut s = alpha_state();
        open_search(&mut s, "alpha");
        assert!(
            status_row(&s, 80).contains("search: alpha 1/2"),
            "newest first"
        );
        assert_eq!(s.input, "", "typing goes to the query, not the input");
        s.handle_key(key(KeyCode::Enter));
        s.handle_key(key(KeyCode::Char('n')));
        assert!(status_row(&s, 80).contains("search: alpha 2/2"));
        s.handle_key(key(KeyCode::Char('n')));
        assert!(status_row(&s, 80).contains("1/2"), "n wraps to the newest");
        s.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT));
        assert!(status_row(&s, 80).contains("2/2"), "N wraps to the oldest");
        s.handle_key(key(KeyCode::Esc));
        assert!(s.overlay.is_none());
        assert!(!status_row(&s, 80).contains("search:"));
    }

    #[test]
    fn test_n_is_a_query_character_while_editing() {
        let mut s = alpha_state();
        open_search(&mut s, "an");
        assert!(status_row(&s, 80).contains("search: an"));
        assert!(status_row(&s, 80).contains("no match"));
    }

    #[test]
    fn test_slash_opens_search_only_on_an_empty_input() {
        let mut s = alpha_state();
        s.handle_key(key(KeyCode::Char('/')));
        assert!(matches!(s.overlay, Some(Overlay::Search(_))));
        assert_eq!(s.input, "");
        s.handle_key(key(KeyCode::Esc));
        type_str(&mut s, "a/b");
        assert!(s.overlay.is_none());
        assert_eq!(s.input, "a/b", "a slash inside text is text");
    }

    #[test]
    fn test_search_is_case_insensitive_and_marks_matched_rows() {
        let mut s = alpha_state();
        open_search(&mut s, "ALPHA");
        let text = screen(&s, 80, 20);
        assert!(status_row(&s, 80).contains("1/2"));
        assert_eq!(text.matches('▶').count(), 2, "{text}");
    }

    #[test]
    fn test_no_match_says_so_and_n_does_nothing() {
        let mut s = alpha_state();
        open_search(&mut s, "zzz");
        assert!(status_row(&s, 80).contains("no match"));
        s.handle_key(key(KeyCode::Enter));
        s.handle_key(key(KeyCode::Char('n')));
        s.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT));
        assert!(status_row(&s, 80).contains("no match"));
        assert!(!screen(&s, 80, 20).contains('▶'));
    }

    fn needle_state() -> TuiState {
        let mut s = state();
        s.push_ansi("needle first\n");
        for i in 0..40 {
            s.push_ansi(&format!("line {}\n", i));
        }
        screen(&s, 60, 15);
        s
    }

    #[test]
    fn test_search_scrolls_to_an_old_match_and_esc_restores_the_view() {
        let mut s = needle_state();
        s.handle_key(key(KeyCode::PageUp));
        let before = screen(&s, 60, 15);
        assert!(!before.contains("needle"));
        s.handle_key(ctrl('f'));
        type_str(&mut s, "needle");
        let during = screen(&s, 60, 15);
        assert!(during.contains("needle first"), "{during}");
        assert!(during.contains('▶'));
        s.handle_key(key(KeyCode::Esc));
        assert_eq!(screen(&s, 60, 15), before, "the view the user had");
    }

    fn long_match_state() -> TuiState {
        let mut s = state();
        s.push_ansi(&format!("needle {}\n", "word ".repeat(40)));
        for i in 0..30 {
            s.push_ansi(&format!("line {}\n", i));
        }
        screen(&s, 60, 15);
        s
    }

    #[test]
    fn test_a_long_match_highlights_every_row_it_spans() {
        let mut s = long_match_state();
        s.handle_key(ctrl('f'));
        type_str(&mut s, "needle");
        let text = screen(&s, 60, 15);
        assert!(text.matches('▶').count() >= 2, "{text}");
        // The match is drawn with reverse video as well as the marker.
        let mut term = Terminal::new(TestBackend::new(60, 15)).unwrap();
        term.draw(|f| s.render(f)).unwrap();
        let buf = term.backend().buffer().clone();
        let reversed = (0..15)
            .filter(|&y| buf[(5, y)].modifier.contains(Modifier::REVERSED))
            .count();
        assert!(reversed >= 2, "{text}");
    }

    #[test]
    fn test_search_follows_a_resize() {
        let mut s = long_match_state();
        s.handle_key(ctrl('f'));
        type_str(&mut s, "needle");
        assert!(screen(&s, 60, 15).contains("needle"));
        assert!(
            screen(&s, 100, 20).contains("needle"),
            "visible after a resize"
        );
        assert!(screen(&s, 45, 12).contains("needle"), "and again");
    }

    #[test]
    fn test_a_paste_goes_into_the_search_query() {
        let mut s = alpha_state();
        s.handle_key(ctrl('f'));
        s.paste("alpha\n");
        screen(&s, 80, 20);
        assert_eq!(s.input, "");
        assert!(status_row(&s, 80).contains("search: alpha 1/2"));
    }

    #[test]
    fn test_esc_closes_the_search_before_it_cancels_a_running_command() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let (mut s, _out) = running_state("market summary USDC --every 2s");
        s.handle_key(ctrl('f'));
        assert_eq!(s.handle_key(key(KeyCode::Esc)), Action::None);
        assert!(s.overlay.is_none());
        assert_eq!(s.handle_key(key(KeyCode::Esc)), Action::Cancel);
    }

    #[test]
    fn test_ctrl_c_still_cancels_a_running_command_with_a_search_open() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let (mut s, _out) = running_state("market summary USDC --every 2s");
        s.handle_key(ctrl('f'));
        assert_eq!(s.handle_key(ctrl('c')), Action::Cancel);
        assert!(s.overlay.is_none());
    }

    #[test]
    fn test_status_keeps_search_before_the_command_text_and_always_the_cancel_hint() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let (mut s, _out) = running_state("market summary USDC --every 2s");
        for i in 0..5 {
            s.push_ansi(&format!("alpha {}\n", i));
        }
        screen(&s, 140, 12);
        s.handle_key(ctrl('f'));
        type_str(&mut s, "alpha");
        screen(&s, 140, 12);
        let wide = status_row(&s, 140);
        assert!(
            wide.contains("search: alpha 1/5") && wide.contains("market summary"),
            "{wide}"
        );
        // Below 32 columns the state itself is cut.
        for w in 32..140 {
            let row = status_row(&s, w);
            assert!(row.contains("Esc closes search"), "{w}: {row:?}");
            if row.contains("market") {
                assert!(
                    row.contains("search:"),
                    "{w}: command without search: {row:?}"
                );
            }
        }
        let search_without_command = (26..140).any(|w| {
            let row = status_row(&s, w);
            row.contains("search:") && !row.contains("market")
        });
        assert!(search_without_command, "the command goes first");
    }

    #[test]
    fn test_search_closes_when_the_terminal_is_too_small_and_restores_the_view() {
        let mut s = needle_state();
        s.handle_key(key(KeyCode::PageUp));
        let before = screen(&s, 60, 15);
        s.handle_key(ctrl('f'));
        type_str(&mut s, "needle");
        screen(&s, 30, 8);
        assert_eq!(s.handle_key(key(KeyCode::Char('x'))), Action::None);
        assert!(s.overlay.is_none());
        assert_eq!(s.input, "");
        assert_eq!(screen(&s, 60, 15), before);
    }

    fn history_state() -> TuiState {
        let mut s = state();
        s.history = vec![
            "venues list".into(),
            "ctx".into(),
            "market summary USDC".into(),
        ];
        s
    }

    #[test]
    fn test_ctrl_r_filters_the_history_and_enter_accepts_without_running() {
        let mut s = history_state();
        s.handle_key(ctrl('r'));
        type_str(&mut s, "ven");
        assert_eq!(s.input, "venues list");
        screen(&s, 80, 20);
        assert!(status_row(&s, 80).contains("1/1"), "{}", status_row(&s, 80));
        assert_eq!(s.handle_key(key(KeyCode::Enter)), Action::None);
        assert!(s.overlay.is_none());
        assert_eq!(s.input, "venues list", "accepted into the input, not run");
    }

    #[test]
    fn test_ctrl_r_again_cycles_to_older_matches() {
        let mut s = history_state();
        s.history.push("venues x".into());
        s.handle_key(ctrl('r'));
        type_str(&mut s, "ven");
        assert_eq!(s.input, "venues x", "most recent first");
        s.handle_key(ctrl('r'));
        assert_eq!(s.input, "venues list");
        s.handle_key(ctrl('r'));
        assert_eq!(s.input, "venues x", "wraps");
    }

    #[test]
    fn test_esc_in_history_search_restores_the_draft() {
        let mut s = history_state();
        type_str(&mut s, "dra");
        s.handle_key(ctrl('r'));
        type_str(&mut s, "ven");
        assert_eq!(s.input, "venues list");
        s.handle_key(key(KeyCode::Esc));
        assert_eq!(s.input, "dra");
        assert!(s.overlay.is_none());
    }

    #[test]
    fn test_history_search_with_no_match_says_so_and_keeps_the_draft() {
        let mut s = history_state();
        type_str(&mut s, "dra");
        s.handle_key(ctrl('r'));
        type_str(&mut s, "zzz");
        screen(&s, 80, 20);
        assert!(status_row(&s, 80).contains("no match"));
        assert_eq!(s.input, "dra");
        for _ in 0..3 {
            s.handle_key(key(KeyCode::Backspace));
        }
        assert_eq!(s.input, "dra", "an empty query matches nothing");
    }

    #[test]
    fn test_a_paste_goes_into_the_history_query() {
        let mut s = history_state();
        s.handle_key(ctrl('r'));
        s.paste("market");
        assert_eq!(s.input, "market summary USDC");
    }

    #[test]
    fn test_n_and_shift_n_go_in_opposite_directions_over_three_matches() {
        let mut s = state();
        s.push_ansi("alpha\nalpha2\nbeta\nalpha3\n");
        screen(&s, 80, 20);
        open_search(&mut s, "alpha");
        let at = |s: &TuiState| status_row(s, 80);
        assert!(at(&s).contains("1/3"));
        s.handle_key(key(KeyCode::Enter));
        let shift_n = KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT);
        s.handle_key(shift_n);
        assert!(
            at(&s).contains("3/3"),
            "N wraps from the newest to the oldest"
        );
        s.handle_key(key(KeyCode::Char('n')));
        assert!(
            at(&s).contains("1/3"),
            "n wraps from the oldest to the newest"
        );
        s.handle_key(key(KeyCode::Char('n')));
        assert!(at(&s).contains("2/3"));
        s.handle_key(shift_n);
        assert!(at(&s).contains("1/3"), "N from the middle goes newer");
        s.handle_key(key(KeyCode::Char('n')));
        s.handle_key(key(KeyCode::Char('n')));
        assert!(at(&s).contains("3/3"));
        s.handle_key(key(KeyCode::Char('n')));
        assert!(at(&s).contains("1/3"));
    }

    fn fill_to_limit(s: &mut TuiState, filler_first: usize) {
        for i in 0..filler_first {
            s.push_ansi(&format!("filler {i}\n"));
        }
        s.push_ansi("needle one\n");
        for i in 0..100 {
            s.push_ansi(&format!("more {i}\n"));
        }
        s.push_ansi("needle two\n");
        while s.scrollback.len() < SCROLLBACK_LIMIT {
            s.push_ansi("tail\n");
        }
    }

    #[test]
    fn test_search_keeps_its_match_when_the_scrollback_drops_old_lines() {
        let mut s = state();
        fill_to_limit(&mut s, 100);
        assert_eq!(s.scrollback.len(), SCROLLBACK_LIMIT);
        screen(&s, 80, 20);
        open_search(&mut s, "needle");
        s.handle_key(key(KeyCode::Enter));
        s.handle_key(key(KeyCode::Char('n')));
        assert!(status_row(&s, 80).contains("2/2"), "on the older match");
        for i in 0..50 {
            s.push_ansi(&format!("new {i}\n"));
        }
        let row = screen_status(&s);
        assert!(
            row.contains("search: needle 2/2"),
            "still the same match: {row}"
        );
        s.handle_key(key(KeyCode::Char('n')));
        assert!(status_row(&s, 80).contains("1/2"));
        s.handle_key(key(KeyCode::Char('n')));
        for i in 0..100 {
            s.push_ansi(&format!("newer {i}\n"));
        }
        // The older match fell off the scrollback: one match is left.
        let row = screen_status(&s);
        assert!(row.contains("needle 1/1"), "{row}");
    }

    fn screen_status(s: &TuiState) -> String {
        screen(s, 80, 20);
        status_row(s, 80)
    }

    #[test]
    fn test_search_finds_the_live_output_of_a_running_command() {
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let (mut s, out) = running_state("market summary USDC --every 2s");
        s.push_ansi("needle done\n");
        out.write_out(format_args!("needle live\nother\n")).unwrap();
        screen(&s, 80, 20);
        open_search(&mut s, "needle");
        let row = status_row(&s, 140);
        assert!(row.contains("search: needle 1/2"), "{row}");
        assert!(screen(&s, 80, 20).matches('▶').count() == 2);
    }

    #[test]
    fn test_opening_a_search_keeps_the_top_of_the_view_where_it_was() {
        // Rows of 59 columns fit the pane but wrap next to the 2-column
        // search gutter, so the layout differs while a search is open.
        let mut s = state();
        for i in 0..40 {
            s.push_ansi(&format!("r{i:02} {}\n", "x".repeat(55)));
        }
        screen(&s, 60, 15);
        s.handle_key(key(KeyCode::PageUp));
        s.handle_key(key(KeyCode::PageUp));
        let top_label = |text: &str| text.lines().nth(1).unwrap()[..3].trim().to_string();
        let before = top_label(&screen(&s, 60, 15));
        s.handle_key(ctrl('f'));
        let during = screen(&s, 60, 15);
        assert!(
            during.lines().nth(1).unwrap().contains(&before),
            "{before} stays at the top: {during}"
        );
        s.handle_key(key(KeyCode::Esc));
        assert_eq!(top_label(&screen(&s, 60, 15)), before);
    }

    #[test]
    fn test_the_status_does_not_offer_pgdn_while_a_search_is_open() {
        let mut s = scrollable();
        s.handle_key(key(KeyCode::PageUp));
        assert!(status_row(&s, 80).contains("PgDn"));
        s.handle_key(ctrl('f'));
        screen(&s, 60, 15);
        let row = status_row(&s, 80);
        assert!(row.contains("scrolled up"), "{row}");
        assert!(!row.contains("PgDn"), "{row}");
    }

    #[test]
    fn test_search_matches_the_text_of_colored_output_not_its_codes() {
        let mut s = state();
        s.push_ansi("al\x1b[31mpha\x1b[0m\n");
        screen(&s, 80, 20);
        open_search(&mut s, "alpha");
        assert!(status_row(&s, 80).contains("1/1"));
        s.handle_key(key(KeyCode::Esc));
        open_search(&mut s, "31m");
        assert!(status_row(&s, 80).contains("no match"));
    }

    // ---- Color theme: the mapping from state to style ----

    fn buffer(s: &TuiState, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| s.render(f)).unwrap();
        term.backend().buffer().clone()
    }

    fn row_text(buf: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect()
    }

    /// The foreground of a rule cell in the top row of the pane.
    fn rule_fg(buf: &ratatui::buffer::Buffer) -> Color {
        let x = (0..buf.area.width)
            .find(|&x| buf[(x, 0)].symbol() == "─")
            .expect("a rule cell");
        buf[(x, 0)].fg
    }

    fn failed_state() -> TuiState {
        let mut s = state();
        s.start("x", &pane_invocation());
        s.finish(Ending::Finished(Err(scope::error::ScopeError::Chain(
            "boom".into(),
        ))));
        s
    }

    #[test]
    fn test_pane_rule_is_accent_idle_warn_running_and_err_after_a_failure() {
        // The rule color is the one signal of state a glance can read.
        let idle = state();
        assert_eq!(rule_fg(&buffer(&idle, 60, 14)), Color::Cyan);
        let (running, _out) = running_state("venues list");
        assert_eq!(rule_fg(&buffer(&running, 60, 14)), Color::Yellow);
        let failed = failed_state();
        assert_eq!(rule_fg(&buffer(&failed, 60, 14)), Color::Red);
    }

    #[test]
    fn test_failed_rule_lasts_until_the_next_command_starts() {
        let mut s = failed_state();
        assert_eq!(s.pane_state(), PaneState::Failed);
        s.start("venues list", &pane_invocation());
        assert_eq!(s.pane_state(), PaneState::Running);
        s.finish(Ending::Finished(Ok(())));
        assert_eq!(s.pane_state(), PaneState::Idle);
    }

    #[test]
    fn test_cancelled_run_leaves_the_rule_idle() {
        let mut s = state();
        run_pane(&mut s, "x\n", Ending::Cancelled);
        assert_eq!(s.pane_state(), PaneState::Idle);
    }

    #[test]
    fn test_state_word_is_at_the_right_end_of_the_top_rule_and_title_stays_left() {
        let buf = buffer(&state(), 60, 14);
        let top = row_text(&buf, 0);
        let title = format!(" scope v{} ", scope::VERSION);
        assert!(top.starts_with(&title), "{top:?}");
        assert!(top.trim_end().ends_with(" idle"), "{top:?}");
        assert!(buf[(1, 0)].modifier.contains(Modifier::BOLD), "title bold");
        assert_eq!(buf[(1, 0)].fg, Color::Cyan);
    }

    #[test]
    fn test_prompt_spans_scope_accent_chain_key_rest_default() {
        let mut s = state();
        s.ctx.chain = "base".into();
        let prompt = s.prompt();
        assert_eq!(prompt.to_string(), "scope:base> ");
        assert_eq!(prompt.spans[0].style, ACCENT);
        assert_eq!(prompt.spans[1].style, KEY);
        assert_eq!(prompt.spans[2].style, Style::new());
        s.push_echo("venues list");
        let echo = last_line(&s);
        assert_eq!(echo.spans[0].style, ACCENT);
        assert_eq!(echo.spans[1].style, KEY);
        assert_eq!(echo.spans[3].content, "venues list");
        assert_eq!(echo.spans[3].style, Style::new());
    }

    #[test]
    fn test_notes_carry_an_info_glyph_and_dim_text() {
        let mut s = state();
        s.push_note("hello");
        let note = last_line(&s);
        assert_eq!(note.to_string(), "ℹ hello");
        assert_eq!(note.spans[0].style, INFO);
        assert_eq!(note.spans[1].style, SECONDARY);
    }

    #[test]
    fn test_selected_palette_row_is_reversed_cyan() {
        let mut s = state();
        s.handle_key(ctrl_k());
        let buf = buffer(&s, 100, 24);
        let selected = buf.content.iter().any(|c| {
            c.fg == Color::Cyan && c.modifier.contains(Modifier::REVERSED) && c.symbol() != " "
        });
        assert!(selected, "a reversed cyan palette row");
        // The rows that are not selected show their names in the accent.
        let names = buf
            .content
            .iter()
            .any(|c| c.fg == Color::Cyan && !c.modifier.contains(Modifier::REVERSED));
        assert!(names);
    }

    #[test]
    fn test_search_match_is_reversed_yellow() {
        let mut s = alpha_state();
        open_search(&mut s, "alpha");
        let buf = buffer(&s, 80, 20);
        let hits = buf
            .content
            .iter()
            .filter(|c| c.fg == Color::Yellow && c.modifier.contains(Modifier::REVERSED))
            .count();
        assert!(hits >= 2 * "alpha".len(), "{hits} matched cells");
    }

    #[test]
    fn test_status_values_are_accent_and_labels_secondary() {
        let line = fit_status(&state().status_parts(), 100);
        let find = |text: &str| {
            line.spans
                .iter()
                .find(|sp| sp.content.contains(text))
                .unwrap_or_else(|| panic!("no span with {text:?}"))
        };
        assert_eq!(find("chain").style, SECONDARY);
        assert_eq!(find("auto").style, ACCENT);
        assert_eq!(find("format").style, SECONDARY);
        assert_eq!(find("help").style, ACCENT);
    }

    #[test]
    fn test_running_status_is_warn_and_esc_cancels_is_bold() {
        let (s, _out) = running_state("venues list");
        let line = fit_status(&s.status_parts(), 120);
        let find = |text: &str| {
            line.spans
                .iter()
                .find(|sp| sp.content.contains(text))
                .unwrap_or_else(|| panic!("no span with {text:?}"))
        };
        assert_eq!(find("running").style, WARN);
        assert_eq!(
            find("Esc cancels").style,
            Style::new().add_modifier(Modifier::BOLD)
        );
        assert_eq!(find("`venues list`").style, ACCENT);
    }

    #[test]
    fn test_hint_next_argument_is_a_key_and_busy_note_is_warn() {
        let mut s = state();
        type_str(&mut s, "address");
        let buf = buffer(&s, 100, 14);
        let hint_row = 14 - 2;
        let next = (0..100u16)
            .rfind(|&x| buf[(x, hint_row)].symbol() == "<")
            .unwrap();
        assert_eq!(buf[(next, hint_row)].fg, Color::Magenta);
        assert!(buf[(next, hint_row)].modifier.contains(Modifier::BOLD));
        let (mut busy, _out) = running_state("venues list");
        busy.busy_hint = true;
        let buf = buffer(&busy, 100, 14);
        let x = (0..100u16)
            .find(|&x| buf[(x, hint_row)].symbol() == "A")
            .unwrap();
        assert_eq!(buf[(x, hint_row)].fg, Color::Yellow);
    }

    #[test]
    fn test_with_color_off_every_state_still_differs_by_text() {
        // `finish_frame` strips color in the real loop; strip it the same way.
        let strip = |s: &TuiState| {
            let mut buf = buffer(s, 60, 14);
            crate::cli::plain::strip_color(&mut buf);
            assert!(buf.content.iter().all(|c| c.fg == Color::Reset));
            buf
        };
        let idle = row_text(&strip(&state()), 0);
        let (running, _out) = running_state("venues list");
        let running = row_text(&strip(&running), 0);
        let failed = row_text(&strip(&failed_state()), 0);
        assert!(idle.trim_end().ends_with(" idle"), "{idle:?}");
        assert!(running.trim_end().ends_with(" running"), "{running:?}");
        assert!(failed.trim_end().ends_with(" failed"), "{failed:?}");
        // The footers keep their glyphs and words.
        let mut s = state();
        run_pane(&mut s, "x\n", Ending::Finished(Ok(())));
        assert!(last_line(&s).to_string().starts_with("✓ Done in "));
        let failed = failed_state();
        assert!(last_line(&failed).to_string().starts_with("✗ Failed in "));
        // Bold is a modifier, so the failed footer stays bold without color.
        assert!(
            last_line(&failed)
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        let buf = strip(&failed);
        assert!(
            buf.content
                .iter()
                .any(|c| c.symbol() == "✗" && c.modifier.contains(Modifier::BOLD))
        );
    }

    #[test]
    fn test_ctrl_k_and_tab_open_no_overlay_while_the_terminal_is_too_small() {
        // A hidden overlay would take the next keys unseen.
        let mut s = state();
        type_str(&mut s, "m");
        screen(&s, 80, 24);
        s.handle_key(key(KeyCode::Tab));
        assert!(
            matches!(s.overlay, Some(Overlay::Menu(_))),
            "Tab offers a menu at a normal size"
        );
        s.handle_key(key(KeyCode::Esc));
        screen(&s, 30, 8);
        s.handle_key(key(KeyCode::Tab));
        assert!(s.overlay.is_none());
        assert_eq!(s.input, "m", "Tab did not complete unseen");
        s.handle_key(ctrl('k'));
        assert!(s.overlay.is_none());
    }

    #[test]
    fn test_status_styles_come_from_the_parts_not_from_their_text() {
        // A search for text that looks like a state word keeps the style
        // of the search note: styles must not depend on the wording.
        let mut s = alpha_state();
        s.push_ansi("x · running\n");
        open_search(&mut s, "x · running");
        let line = fit_status(&s.status_parts(), 120);
        let at = |text: &str| {
            line.spans
                .iter()
                .position(|sp| sp.content.contains(text))
                .unwrap_or_else(|| panic!("no {text:?} in {line:?}"))
        };
        for sp in &line.spans[at("search:")..=at("1/1")] {
            assert_eq!(sp.style, ACCENT, "{sp:?} in {line:?}");
        }
        // Each part keeps its role: label dim, value accent, the
        // separator dim.
        let (run, _out) = running_state("venues list");
        let line = fit_status(&run.status_parts(), 120);
        let style_of = |text: &str| {
            line.spans
                .iter()
                .find(|sp| sp.content == text)
                .unwrap_or_else(|| panic!("no span {text:?} in {line:?}"))
                .style
        };
        assert_eq!(
            style_of("Esc cancels"),
            Style::new().add_modifier(Modifier::BOLD)
        );
        assert_eq!(style_of(STATUS_SEP), SECONDARY);
        assert_eq!(style_of("`venues list`"), ACCENT);
    }

    #[test]
    fn test_help_shows_key_names_in_the_key_style() {
        let mut s = state();
        assert!(matches!(s.submit("help", &Config::default()), Plan::Done));
        let span = s
            .scrollback
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|sp| sp.content == "Ctrl-K")
            .expect("help names Ctrl-K");
        assert_eq!(span.style.fg, Some(Color::Magenta));
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
        // Each help span follows an ANSI reset, so it is the theme style
        // over a reset style.
        assert_eq!(span.style, Style::reset().patch(KEY));
        let what = s
            .scrollback
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|sp| sp.content == "palette")
            .expect("help says what Ctrl-K does");
        assert_eq!(what.style, Style::reset().patch(SECONDARY));
    }

    #[test]
    fn test_any_accepted_line_clears_the_failed_state() {
        // The red rule says the last thing the user did failed. A builtin
        // or an outside command after the failure is the last thing now.
        for line in ["ctx", "format json", "setup"] {
            let mut s = failed_state();
            assert_eq!(s.pane_state(), PaneState::Failed);
            s.submit(line, &Config::default());
            assert_eq!(s.pane_state(), PaneState::Idle, "{line}");
        }
        // An empty line is not a new action.
        let mut s = failed_state();
        s.submit("  ", &Config::default());
        assert_eq!(s.pane_state(), PaneState::Failed);
    }
}
