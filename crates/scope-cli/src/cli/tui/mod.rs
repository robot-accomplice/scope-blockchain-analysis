//! # Unified TUI
//!
//! `scope tui` (also `scope interactive` and `scope shell`) opens one
//! full-screen terminal UI. Every CLI command runs inside it with the same
//! flags and the same handlers as on the command line (issue #47).
//!
//! ```text
//!  scope vX.Y.Z ──────────────────────────────────────── idle
//! scope:auto> address 0x742d…
//! ┌─ Address Analysis ─…
//! ──────────────────────────────────────────────────────────
//! ┌scope:auto> ────────────────────────────────────────────┐
//! │ market summary USDC --venue binance                    │
//! └────────────────────────────────────────────────────────┘
//!  <next argument hint>
//!  chain auto · format Table · limit 100      help · exit
//! ```
//!
//! The output pane has a top and a bottom rule and no side borders. The
//! top rule names the version and the pane state (idle, running, failed).
//!
//! - [`exec`] parses a line: session commands, or the CLI's clap definition.
//! - [`app`] holds the state and draws the screen.
//! - [`journal`] keeps the history and the event log.
//!
//! A command that reads stdin or draws its own screen (the setup wizard,
//! the live monitor) runs in suspend mode: the TUI gives the terminal back
//! while it runs. What it writes through its output also joins the
//! scrollback when the TUI returns.

pub mod app;
pub mod clip;
pub mod complete;
pub mod crash;
pub mod exec;
pub mod fit;
pub mod grid;
pub mod hint;
pub mod journal;
pub mod palette;
pub(crate) mod repeat_view;
pub mod vocab;

use app::{Action, Ending, TuiState, run_invocation};
use crossterm::event::{Event, EventStream, KeyEventKind};
use exec::{Plan, Route, WatchCommand};
use futures::StreamExt;
use futures::stream::BoxStream;
use journal::Journal;
use scope::chains::ChainClientFactory;
use scope::config::Config;
use scope::error::{Result, ScopeError};
use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::cli::interactive::SessionContext;
use crate::cli::output::{Captured, Output};
use crate::cli::prompt;
use crate::cli::screen;

/// How long `run_suspended` waits for the signal behind a stopped
/// prompt. The signal is normally already queued.
const PROMPT_SIGNAL_WAIT: Duration = Duration::from_secs(1);

/// How often the screen redraws while a command runs, so its output and
/// elapsed time stay current.
const LIVE_REDRAW: Duration = Duration::from_millis(100);

/// How many bytes of a suspend-mode command's output the TUI keeps to
/// show after it returns: about the scrollback limit of 200-column lines.
/// A long crawl cannot grow it further.
const OUTSIDE_COPY_LIMIT: usize = app::SCROLLBACK_LIMIT * 200;

/// The startup banner.
const BANNER: &str = include_str!("../../../assets/banner.txt");

/// The shell convention for the exit code after a fatal signal: 128 plus
/// the signal number.
const SIGNAL_EXIT_BASE: i32 = 128;

/// How the TUI ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuiExit {
    /// The process exit code: 0 when the user left, 128+N after signal N.
    pub code: i32,
}

/// A process signal the TUI listens for. Off Unix only Ctrl-C (`Int`)
/// arrives; the other variants stay so one match covers every platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) enum Signal {
    Term,
    Hup,
    Int,
    Quit,
    Tstp,
    Cont,
}

impl Signal {
    /// The name without `SIG`, as the journal records it.
    fn name(self) -> &'static str {
        match self {
            Signal::Term => "TERM",
            Signal::Hup => "HUP",
            Signal::Int => "INT",
            Signal::Quit => "QUIT",
            Signal::Tstp => "TSTP",
            Signal::Cont => "CONT",
        }
    }

    /// True for a signal that ends a suspend-mode command: INT cancels
    /// it, TERM, HUP and QUIT end the session.
    fn ends_suspended(self) -> bool {
        matches!(
            self,
            Signal::Int | Signal::Term | Signal::Hup | Signal::Quit
        )
    }

    /// The exit code for a signal that ends the TUI. `None` for a signal
    /// that does not end it. The numbers are fixed by POSIX.
    fn exit_code(self) -> Option<i32> {
        let number = match self {
            Signal::Hup => 1,
            Signal::Int => 2,
            Signal::Quit => 3,
            Signal::Term => 15,
            Signal::Tstp | Signal::Cont => return None,
        };
        Some(SIGNAL_EXIT_BASE + number)
    }
}

/// Checks that the TUI has a terminal to draw on and read keys from.
///
/// `scope tui` needs stdin and stdout on a terminal, and a `TERM` that is
/// not `dumb`. Pure: the caller passes the facts.
///
/// # Errors
///
/// Returns the message to print when any of them is missing.
pub fn tui_terminal_ok(
    stdin_tty: bool,
    stdout_tty: bool,
    term: Option<&str>,
) -> std::result::Result<(), String> {
    if stdin_tty && stdout_tty && term != Some("dumb") {
        return Ok(());
    }
    Err("scope tui needs an interactive terminal; run the command directly, e.g. scope market summary …".to_string())
}

/// What the TUI's commands run with: the config, and the chain clients
/// built from it.
pub struct Settings {
    pub config: Config,
    pub clients: Box<dyn ChainClientFactory>,
}

/// Reads the [`Settings`] again, after `setup` saved new ones (#99).
pub type Reload<'a> = &'a dyn Fn() -> Result<Settings>;

/// Runs the TUI until the user leaves or a signal ends it.
///
/// # Errors
///
/// Returns an error when the signal handlers cannot be installed, a
/// terminal I/O error, or the error from saving the history or the
/// session context on exit.
pub async fn run(no_banner: bool, settings: Settings, reload: Reload<'_>) -> Result<TuiExit> {
    let signals = signal_stream()?;
    // A second stream gets the same signals (tokio gives each one to
    // every listener). Only `scope tui` installs these handlers; a plain
    // CLI run keeps the default Ctrl-C.
    tokio::spawn(cancel_prompts_on(signal_stream()?, &prompt::CANCEL));
    // The crash hook, then raw mode before the loads, so keys typed
    // during startup are not read in cooked mode (issue #63). A failed
    // `enter` has given the screen back already. Nothing after it can
    // return an error before `session`, which restores the terminal.
    crash::install_panic_hook();
    let mut host = RealHost {
        terminal: screen::enter(screen::Mouse::Off)?,
        events: None,
        signals,
        on_screen: true,
    };
    let mut state = TuiState::new(
        SessionContext::load(),
        Journal::in_data_dir(),
        vocab::Vocab::load(&settings.config),
    );
    welcome(&mut state, no_banner);
    session(
        &mut host,
        &mut state,
        settings,
        reload,
        SessionContext::save,
    )
    .await
}

/// Runs the event loop, then restores the terminal and saves, also after
/// a signal. `save_ctx` saves the session context.
async fn session<H: Host>(
    host: &mut H,
    state: &mut TuiState,
    settings: Settings,
    reload: Reload<'_>,
    save_ctx: impl FnOnce(&SessionContext) -> Result<()>,
) -> Result<TuiExit> {
    let result = event_loop(host, state, settings, reload).await;
    // The CLI after the TUI, also after an error or a signal, uses the
    // real width.
    host.set_width(None);
    host.restore();

    // Save both even if one fails, then report the first failure.
    let saved_history = state.journal().save_history(&state.history);
    let saved_ctx = save_ctx(&state.ctx);
    let exit = result?;
    saved_history?;
    saved_ctx?;
    Ok(exit)
}

/// The signals the TUI listens for, as one stream. Installing a handler
/// replaces the default action, so these no longer kill the process.
#[cfg(unix)]
fn signal_stream() -> Result<BoxStream<'static, Signal>> {
    use tokio::signal::unix::{SignalKind, signal};
    let kinds = [
        (SignalKind::terminate(), Signal::Term),
        (SignalKind::hangup(), Signal::Hup),
        (SignalKind::interrupt(), Signal::Int),
        (SignalKind::quit(), Signal::Quit),
        (SignalKind::from_raw(libc::SIGTSTP), Signal::Tstp),
        (SignalKind::from_raw(libc::SIGCONT), Signal::Cont),
    ];
    let mut streams = Vec::with_capacity(kinds.len());
    for (kind, sig) in kinds {
        let mut handler = signal(kind).map_err(|e| {
            ScopeError::io_context(e, format!("cannot install the SIG{} handler", sig.name()))
        })?;
        streams.push(futures::stream::poll_fn(move |cx| {
            handler.poll_recv(cx).map(|got| got.map(|()| sig))
        }));
    }
    Ok(futures::stream::select_all(streams).boxed())
}

/// Ctrl-C at the console, as INT. The TUI's raw mode reads Ctrl-C as a
/// key, so this fires only while a suspend-mode command has the cooked
/// console. Installing the handler replaces the default, which ends the
/// process.
#[cfg(windows)]
fn signal_stream() -> Result<BoxStream<'static, Signal>> {
    let mut handler = tokio::signal::windows::ctrl_c()
        .map_err(|e| ScopeError::io_context(e, "cannot install the Ctrl-C handler"))?;
    Ok(futures::stream::poll_fn(move |cx| {
        handler.poll_recv(cx).map(|got| got.map(|()| Signal::Int))
    })
    .boxed())
}

/// No signal handling off unix and Windows.
#[cfg(not(any(unix, windows)))]
fn signal_stream() -> Result<BoxStream<'static, Signal>> {
    Ok(futures::stream::pending().boxed())
}

fn welcome(state: &mut TuiState, no_banner: bool) {
    if !no_banner {
        state.push_ansi(BANNER);
    }
    state.push_note("Type `help` for commands. `exit` or Ctrl-D leaves.");
    // First steps (#66): a few commands to try, and the two keys that find
    // the rest.
    state.push_note(
        "Try: `market summary USDC --venue binance` · `venues list` · `insights <address>`",
    );
    state.push_note("Tab completes a command or flag; Ctrl-K opens the command palette.");
}

/// Serializes tests that redirect the process-global diagnostics sink
/// (every test that calls `TuiState::start`). Without it, parallel tests
/// swap each other's sink. tokio's Mutex, so async tests can hold it
/// across `.await` without `clippy::await_holding_lock`.
#[cfg(test)]
pub(crate) static DIAG_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The terminal side of the event loop. Production uses the real terminal;
/// tests use a `TestBackend` and a scripted key stream.
pub(crate) trait Host {
    type B: ratatui::backend::Backend;
    type Events: futures::Stream<Item = io::Result<Event>> + Unpin;
    type Signals: futures::Stream<Item = Signal> + Unpin;
    fn terminal(&mut self) -> &mut ratatui::Terminal<Self::B>;
    /// The terminal events and the process signals. One method, so the
    /// event loop can wait on both at once.
    fn streams(&mut self) -> (&mut Self::Events, &mut Self::Signals);
    /// Gives the terminal back for a suspend-mode command.
    fn suspend(&mut self);
    /// Takes the terminal again after a suspend-mode command.
    ///
    /// # Errors
    ///
    /// The terminal cannot be taken ([`screen::not_taken`]).
    fn resume(&mut self) -> Result<()>;
    /// Gives the terminal back for good when the TUI ends.
    fn restore(&mut self);
    /// Stops the process until the shell continues it (`fg`). Call it
    /// after `suspend`, so the shell gets a clean terminal. The kernel
    /// sends SIGCONT when the process continues.
    fn stop_self(&mut self) -> io::Result<()>;
    /// Clears the screen, so the next draw repaints all of it.
    fn clear(&mut self) -> io::Result<()>;
    /// The output a suspend-mode command writes to.
    fn suspended_output(&self) -> Output;
    /// Writes `text` to the terminal as is (the OSC 52 copy sequence).
    fn write_raw(&mut self, text: &str) -> io::Result<()>;
    /// Sets the width that command output fits (`None`: the real terminal
    /// width). Production sets the process-wide width override.
    fn set_width(&mut self, width: Option<usize>);
}

/// The real terminal.
struct RealHost {
    terminal: ratatui::DefaultTerminal,
    /// `None` while a suspend-mode command owns stdin: the event reader
    /// must not consume the keys that command reads.
    events: Option<EventStream>,
    signals: BoxStream<'static, Signal>,
    /// True while the TUI owns the screen (raw mode, alternate screen).
    on_screen: bool,
}

impl Host for RealHost {
    type B = ratatui::backend::CrosstermBackend<io::Stdout>;
    type Events = EventStream;
    type Signals = BoxStream<'static, Signal>;

    fn terminal(&mut self) -> &mut ratatui::Terminal<Self::B> {
        &mut self.terminal
    }
    fn streams(&mut self) -> (&mut EventStream, &mut Self::Signals) {
        (
            self.events.get_or_insert_with(EventStream::new),
            &mut self.signals,
        )
    }
    fn suspend(&mut self) {
        self.events = None;
        self.restore();
    }
    fn resume(&mut self) -> Result<()> {
        self.terminal = screen::enter(screen::Mouse::Off)?;
        self.on_screen = true;
        Ok(())
    }
    /// Does nothing when the screen is already given back: leaving the
    /// alternate screen twice can move the shell's cursor.
    fn restore(&mut self) {
        if std::mem::take(&mut self.on_screen) {
            screen::restore();
        }
    }
    /// SIGSTOP, not SIGTSTP: the TUI handles SIGTSTP itself, and SIGSTOP
    /// cannot be caught. The call returns when the process continues.
    #[cfg(unix)]
    fn stop_self(&mut self) -> io::Result<()> {
        // SAFETY: kill takes plain integers and has no memory effects.
        let rc = unsafe { libc::kill(libc::getpid(), libc::SIGSTOP) };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(not(unix))]
    fn stop_self(&mut self) -> io::Result<()> {
        Err(io::Error::other("job control needs a unix host"))
    }
    fn clear(&mut self) -> io::Result<()> {
        self.terminal.clear()
    }
    fn suspended_output(&self) -> Output {
        Output::stdio()
    }
    fn write_raw(&mut self, text: &str) -> io::Result<()> {
        let mut out = io::stdout();
        out.write_all(text.as_bytes())?;
        out.flush()
    }
    fn set_width(&mut self, width: Option<usize>) {
        scope::display::terminal::set_width_override(width);
    }
}

/// The output of a command that runs outside the TUI (suspend mode).
struct Outside {
    /// The plain terminal. The header goes only here: it is stale once
    /// the TUI is back.
    terminal: Output,
    /// What the command writes to: the terminal, and a copy the TUI shows
    /// in the scrollback when it returns. Its hints use TUI wording,
    /// because they are read in the TUI.
    out: Output,
    copy: Captured,
}

impl Outside {
    fn new(terminal: Output) -> Self {
        let (out, copy) = terminal.teed(OUTSIDE_COPY_LIMIT);
        Self {
            terminal,
            out,
            copy,
        }
    }
}

type PaneFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

/// After `setup` in the TUI: the session read the config again.
const SETUP_RELOADED_NOTE: &str = "The TUI now uses the saved settings.";

/// After `setup`, when the config could not be read again.
fn setup_reload_failed_note(e: &ScopeError) -> String {
    format!(
        "The settings could not be read again ({e}). Leave the TUI (exit) and start it again to use them."
    )
}

async fn event_loop<H: Host>(
    host: &mut H,
    state: &mut TuiState,
    settings: Settings,
    reload: Reload<'_>,
) -> Result<TuiExit> {
    // Each command holds its own handle on the settings, so `setup` can
    // swap in new ones between commands.
    let mut settings = std::rc::Rc::new(settings);
    const LEFT: TuiExit = TuiExit { code: 0 };
    // Library warnings between commands land here and join the scrollback.
    let idle_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut idle_diag = Some(scope::diag::redirect(idle_buf.clone()));
    let mut pending: Option<PaneFuture<'_>> = None;
    // A running watchlist (#47). It runs next to the pane command, so the
    // shell stays usable while the grid ticks.
    let mut watch_pending: Option<PaneFuture<'_>> = None;
    let mut tick = tokio::time::interval(LIVE_REDRAW);
    // After a stop (Ctrl-Z, then `fg`), fire once, not once per missed
    // tick: the tick arm comes before the keys in the biased select.
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // True after the TUI stopped itself: the SIGCONT that continues it
    // needs no second set-up.
    let mut cont_expected = false;

    loop {
        drain_into(&idle_buf, state);
        state.poll_watch();
        if state.take_bell() {
            // A new breach: one bell. A failed write only loses the sound.
            let _ = host.write_raw("\x07");
        }
        // A question from the running command opens as a list. The live
        // tick wakes the loop while a command runs, so it shows within one.
        state.poll_ask();
        host.terminal()
            .draw(|f| {
                state.render(f);
                crate::cli::plain::finish_frame(f);
            })
            // The backend's error type is generic and need not be Send, so
            // only its text can travel.
            .map_err(|e| {
                ScopeError::io_context(io::Error::other(e.to_string()), "terminal draw failed")
            })?;
        // Command output fits the pane. Set after each draw, so it follows
        // a resize and comes back after the TUI leaves the terminal.
        host.set_width(state.pane_width());

        // Biased: a finished command is handled before the next key, so
        // the key that follows it sees the command as done. A signal comes
        // before the keys.
        let (events, signals) = host.streams();
        tokio::select! {
            biased;
            res = async { pending.as_mut().expect("guarded by precondition").await }, if pending.is_some() => {
                pending = None;
                state.finish(Ending::Finished(res));
            }
            res = async { watch_pending.as_mut().expect("guarded by precondition").await }, if watch_pending.is_some() => {
                watch_pending = None;
                state.watch_finished(res);
            }
            _ = tick.tick(), if state.running.is_some() || state.watch_running() => {}
            Some(sig) = signals.next() => {
                if let Some(code) = sig.exit_code() {
                    // Drop the command future first: that cancels it.
                    drop(pending.take());
                    state.finish(Ending::Cancelled);
                    state.record_signal(sig.name(), "session", "", &[]);
                    return Ok(TuiExit { code });
                }
                // Merge window: the kernel keeps one pending SIGCONT, and
                // the handler stream can merge signals too. So a second
                // SIGCONT that comes before this arm runs arrives as one,
                // and is taken as ours. That is safe: the screen was
                // already taken back and cleared after the stop.
                if sig == Signal::Tstp {
                    cont_expected = stop_and_return(host, state)?;
                } else if !std::mem::take(&mut cont_expected) {
                    // SIGCONT after an outside SIGSTOP: the shell reset the
                    // terminal, so set it up again from scratch.
                    leave(host);
                    retake(host)?;
                }
            }
            ev = events.next() => {
                let key = match ev {
                    Some(Ok(Event::Key(k))) if k.kind == KeyEventKind::Press => k,
                    Some(Ok(Event::Paste(text))) => {
                        state.paste(&text);
                        continue;
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return Err(ScopeError::IoError(e)),
                    None => return Ok(LEFT),
                };
                match state.handle_key(key) {
                    Action::None => {}
                    Action::Exit => return Ok(LEFT),
                    Action::Suspend => cont_expected = stop_and_return(host, state)?,
                    Action::Copy(text) => {
                        let result = host.write_raw(&clip::osc52(&text));
                        state.copied(text.lines().count(), result);
                    }
                    Action::StopWatch => {
                        // Dropping the future stops the run at its next await.
                        watch_pending = None;
                        state.stop_watch();
                    }
                    Action::Cancel => {
                        // Dropping the future cancels the handler at its
                        // next await point.
                        pending = None;
                        state.finish(Ending::Cancelled);
                    }
                    Action::Submit(line) => match state.submit(&line, &settings.config) {
                        Plan::Done => {}
                        Plan::Exit => return Ok(LEFT),
                        Plan::Watch(cmd) => match cmd {
                            WatchCommand::Open(list) => {
                                if state.watch_running() {
                                    state.push_note(WATCH_BUSY_NOTE);
                                } else {
                                    // A finished grid makes room for the new one.
                                    state.close_grid();
                                    let (out, tx) = state.start_watch(*list.clone());
                                    let s = settings.clone();
                                    watch_pending = Some(Box::pin(async move {
                                        run_watch(*list, &s, &out, tx).await
                                    }));
                                }
                            }
                            WatchCommand::Stop => {
                                if state.watch_running() {
                                    watch_pending = None;
                                    state.stop_watch();
                                } else {
                                    state.push_note("No watchlist runs.");
                                }
                            }
                            WatchCommand::Show => state.show_grid(),
                        },
                        Plan::Run(inv) => match inv.route {
                            Route::Pane => {
                                let out = state.start(&line, &inv);
                                let s = settings.clone();
                                pending = Some(Box::pin(async move {
                                    run_invocation(*inv, &s.config, &*s.clients, &out).await
                                }));
                            }
                            Route::Suspend => {
                                drop(idle_diag.take());
                                leave(host);
                                let outside = Outside::new(host.suspended_output());
                                let argv = inv.argv.clone();
                                let command =
                                    run_invocation(*inv, &settings.config, &*settings.clients, &outside.out);
                                let ended =
                                    run_suspended(host, &line, &argv, &outside, command, state)
                                        .await;
                                if let Some(exit) = ended {
                                    // The terminal stays cooked; session()
                                    // restores and saves.
                                    return Ok(exit);
                                }
                                if argv.get(1).is_some_and(|c| c == "setup") {
                                    // Before the screen is taken back, so a
                                    // notice from the reload shows outside.
                                    match reload() {
                                        Ok(new) => {
                                            state.set_vocab(vocab::Vocab::load(&new.config));
                                            settings = std::rc::Rc::new(new);
                                            state.push_note(SETUP_RELOADED_NOTE);
                                        }
                                        Err(e) => state.push_note(&setup_reload_failed_note(&e)),
                                    }
                                }
                                host.resume()?;
                                idle_diag.replace(scope::diag::redirect(idle_buf.clone()));
                            }
                            Route::Refuse(reason) => {
                                // exec::plan writes refusals and never
                                // returns them; keep this loud anyway.
                                state.push_note(&format!("Not available here: {}.", reason));
                            }
                        },
                    },
                }
            }
        }
    }
}

/// `watch open` while a watchlist runs.
const WATCH_BUSY_NOTE: &str =
    "A watchlist runs already. Stop it first (`watch stop`, or Esc in the grid).";

/// Runs a watchlist for the grid: resolves its targets once, then ticks on
/// its schedule. Each step goes to the grid through `tx`; a closed grid
/// only drops the messages.
async fn run_watch(
    list: scope::watch::Watchlist,
    settings: &Settings,
    out: &Output,
    tx: std::sync::mpsc::Sender<grid::GridMsg>,
) -> Result<()> {
    let src = crate::cli::watch::Sources {
        config: &settings.config,
        clients: &*settings.clients,
    };
    let ready = crate::cli::watch::prepare(&list, src, out).await?;
    let _ = tx.send(grid::GridMsg::Ready(ready.clone()));
    crate::cli::watch::run(&ready, src, out, |run, tick, events| {
        let _ = tx.send(grid::GridMsg::Tick {
            run,
            tick: Box::new(tick.clone()),
            events: events.to_vec(),
        });
    })
    .await
    .map(|_| ())
}

/// Gives the terminal back and stops the process (Ctrl-Z, SIGTSTP). When
/// the shell continues it, takes the screen again. A running pane command
/// is kept; it is not polled while the process is stopped. Returns true
/// when the process stopped, so a SIGCONT follows.
fn stop_and_return<H: Host>(host: &mut H, state: &mut TuiState) -> Result<bool> {
    leave(host);
    let stopped = match host.stop_self() {
        Ok(()) => true,
        Err(e) => {
            state.push_note(&format!("Could not stop the process: {}", e));
            false
        }
    };
    retake(host)?;
    Ok(stopped)
}

/// Gives the terminal back. Output written while the TUI is away (a
/// suspend-mode command, the shell after Ctrl-Z) uses the real width.
fn leave<H: Host>(host: &mut H) {
    host.set_width(None);
    host.suspend();
}

/// Takes the terminal again and clears it. The next draw then repaints
/// all of the screen, also what the shell wrote over it.
fn retake<H: Host>(host: &mut H) -> Result<()> {
    host.resume()?;
    host.clear()
        .map_err(|e| ScopeError::io_context(e, "terminal clear failed"))
}

/// Runs one command outside the TUI. It returns to the TUI when the
/// command ends, or when INT (Ctrl-C) cancels it. `command` writes to
/// `outside.out`; when it ends, its output and error text join the
/// scrollback, above the `Ran outside the TUI` note. Returns the exit when
/// TERM, HUP or QUIT ends the session first; the command is dropped and
/// the signal recorded.
async fn run_suspended<H: Host>(
    host: &mut H,
    line: &str,
    argv: &[String],
    outside: &Outside,
    command: impl Future<Output = Result<()>>,
    state: &mut TuiState,
) -> Option<TuiExit> {
    let started = Instant::now();
    let started_at = chrono::Utc::now().to_rfc3339();
    // A cancel from an earlier command must not stop this one's prompts.
    prompt::CANCEL.store(false, Ordering::SeqCst);
    let mut terminal_errors = Vec::new();
    crate::note!(outside.terminal, "\n── scope {} ──", line);
    crate::note!(
        outside.terminal,
        "Running outside the TUI; it returns when the command ends (Ctrl-C cancels)."
    );
    let ended = tokio::select! {
        biased;
        Some(sig) = ending_signal(host.streams().1) => Err(sig),
        result = command => Ok(result),
    };
    // A prompt blocks this task, so the select can miss a signal that
    // came meanwhile. The prompt watcher saw it and set the flag; the
    // stream has it too, because each listener gets every signal. Take it
    // from the stream, or the TUI would see it later. The wait has a
    // limit, so a broken invariant cannot hang the TUI on a cooked
    // terminal; it then counts as a cancel.
    let ended = match ended {
        Ok(_) if prompt::CANCEL.load(Ordering::SeqCst) => {
            let queued =
                tokio::time::timeout(PROMPT_SIGNAL_WAIT, ending_signal(host.streams().1)).await;
            Err(queued.ok().flatten().unwrap_or(Signal::Int))
        }
        ended => ended,
    };
    let ending = match ended {
        Ok(result) => Ending::Finished(result),
        Err(Signal::Int) => Ending::Cancelled,
        Err(sig) => return Some(end_suspended(state, line, argv, sig)),
    };
    let duration = started.elapsed();
    if let Ending::Finished(Err(e)) = &ending
        && let Err(write_err) = crate::cli::errors::display_error(e, &outside.out)
    {
        terminal_errors.push(write_err.to_string());
    }
    state.show_outside(&outside.copy, &ending);
    state.record_suspended(line, argv, duration, started_at, &ending);
    for e in terminal_errors {
        state.push_note(&format!("Terminal I/O error while outside the TUI: {}", e));
    }
    if matches!(ending, Ending::Cancelled)
        && let Some(sig) = drain_extra_ints(host.streams().1)
    {
        return Some(end_suspended(state, line, argv, sig));
    }
    None
}

/// Takes the signals already queued after a cancel, without waiting. A
/// second Ctrl-C must not reach the TUI and end it. Drops INT, TSTP and
/// CONT; returns the first TERM, HUP or QUIT, which still ends the
/// session.
fn drain_extra_ints<S: futures::Stream<Item = Signal> + Unpin>(signals: &mut S) -> Option<Signal> {
    use futures::FutureExt;
    while let Some(Some(sig)) = signals.next().now_or_never() {
        if sig.ends_suspended() && sig != Signal::Int {
            return Some(sig);
        }
    }
    None
}

/// Waits for a signal that ends a suspend-mode command: INT cancels it;
/// TERM, HUP and QUIT end the session. It drops TSTP and CONT, because
/// the command owns the terminal.
async fn ending_signal<S: futures::Stream<Item = Signal> + Unpin>(
    signals: &mut S,
) -> Option<Signal> {
    while let Some(sig) = signals.next().await {
        if sig.ends_suspended() {
            return Some(sig);
        }
    }
    None
}

/// Records that `sig` ended the suspend-mode command `line`; returns the
/// exit for it.
fn end_suspended(state: &mut TuiState, line: &str, argv: &[String], sig: Signal) -> TuiExit {
    state.record_signal(sig.name(), "suspend", line, argv);
    TuiExit {
        code: sig.exit_code().expect("TERM, HUP and QUIT have exit codes"),
    }
}

/// Sets `flag` on each signal that ends a suspend-mode command, so a
/// prompt read stops (see [`prompt`]). It runs as its own task: while a
/// prompt blocks, the event loop's task cannot see the signals.
async fn cancel_prompts_on<S: futures::Stream<Item = Signal> + Unpin>(
    mut signals: S,
    flag: &AtomicBool,
) {
    while let Some(sig) = signals.next().await {
        if sig.ends_suspended() {
            flag.store(true, Ordering::SeqCst);
        }
    }
}

/// Moves buffered library warnings into the scrollback.
fn drain_into(buf: &Mutex<Vec<u8>>, state: &mut TuiState) {
    let bytes = std::mem::take(&mut *buf.lock().unwrap_or_else(|p| p.into_inner()));
    if !bytes.is_empty() {
        state.push_ansi(&String::from_utf8_lossy(&bytes));
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cli::Cli;
    use ansi_to_tui::IntoText;
    use clap::Parser;
    use scope::chains::mocks::MockClientFactory;

    #[test]
    fn test_tui_requires_a_terminal() {
        assert!(tui_terminal_ok(true, true, Some("xterm-256color")).is_ok());
        assert!(tui_terminal_ok(true, true, None).is_ok());
        for (i, o, t) in [
            (false, true, Some("xterm")),
            (true, false, Some("xterm")),
            (true, true, Some("dumb")),
        ] {
            let e = tui_terminal_ok(i, o, t).unwrap_err();
            assert!(
                e.starts_with("scope tui needs an interactive terminal"),
                "{e}"
            );
        }
    }

    const ADDR: &str = "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2";
    const TX: &str = "0xabc123def456789012345678901234567890123456789012345678901234abcd";

    /// Plain text of ANSI output, as the pane shows it.
    fn plain(ansi: &str) -> String {
        ansi.into_text()
            .unwrap()
            .lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Runs a line through the TUI path; returns the pane text.
    async fn via_tui(line: &str, factory: &MockClientFactory) -> String {
        let config = Config::default();
        let mut s = TuiState::new(
            SessionContext::default(),
            Journal::default(),
            vocab::Vocab::default(),
        );
        let Plan::Run(inv) = s.submit(line, &config) else {
            panic!("`{}` did not plan a run", line)
        };
        assert_eq!(inv.route, Route::Pane);
        let out = s.start(line, &inv);
        let res = run_invocation(*inv, &config, factory, &out).await;
        s.finish(Ending::Finished(res));
        s.scrollback_text()
    }

    /// Runs the same line through the CLI path, with the CLI's own
    /// (non-TUI) output; returns the data, then the diagnostics and the
    /// error display, as the terminal would show them.
    async fn via_cli(line: &str, factory: &MockClientFactory) -> String {
        let mut argv = vec!["scope".to_string()];
        argv.extend(shlex::split(line).unwrap());
        let cli = Cli::try_parse_from(argv).unwrap();
        let (o, cap) = Output::capture();
        assert!(!o.is_tui());
        let res =
            crate::cli::dispatch::dispatch_with(cli.command, &Config::default(), factory, &o).await;
        if let Err(e) = &res {
            crate::cli::errors::display_error(e, &o).unwrap();
        }
        format!("{}\n{}", plain(&cap.out()), plain(&cap.err()))
    }

    /// Issue #47, requirement 1: a TUI command calls the same handler as the
    /// CLI and shows the same result.
    async fn assert_same_result(line: &str) {
        let _lock = DIAG_LOCK.lock().await;
        let factory = MockClientFactory::new();
        let cli = via_cli(line, &factory).await;
        let tui = via_tui(line, &factory).await;
        assert!(!cli.trim().is_empty(), "`{}` printed nothing", line);
        for row in cli.lines().filter(|r| !r.trim().is_empty()) {
            assert!(
                tui.contains(row),
                "`{}`: the TUI pane is missing the CLI line {:?}\n--- TUI ---\n{}",
                line,
                row,
                tui
            );
        }
    }

    #[tokio::test]
    async fn test_address_same_as_cli() {
        assert_same_result(&format!("address {}", ADDR)).await;
    }

    #[tokio::test]
    async fn test_tx_same_as_cli() {
        assert_same_result(&format!("tx {}", TX)).await;
    }

    #[tokio::test]
    async fn test_market_summary_dex_same_as_cli() {
        assert_same_result("market summary USDC --venue eth").await;
    }

    #[tokio::test]
    async fn test_venues_list_json_same_as_cli() {
        assert_same_result("venues list --format json").await;
    }

    #[tokio::test]
    async fn test_failed_command_shows_same_error_as_cli() {
        // Error text and remediation hints must match the CLI too.
        assert_same_result("venues validate /nonexistent/venue.yaml").await;
    }

    // ---- Event loop, driven through a scripted host ----

    use crossterm::event::{KeyCode, KeyEvent, KeyEventState, KeyModifiers};
    use futures::channel::mpsc;
    use futures::stream::BoxStream;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::task::Poll;

    /// A host with a `TestBackend` screen, a scripted key stream and a
    /// scripted signal stream.
    struct TestHost {
        terminal: Terminal<TestBackend>,
        keys: BoxStream<'static, io::Result<Event>>,
        signals: mpsc::UnboundedReceiver<Signal>,
        signal_tx: mpsc::UnboundedSender<Signal>,
        /// `suspend` sends these signals: they arrive while a
        /// suspend-mode command runs.
        suspend_signals: Vec<Signal>,
        suspended: Output,
        suspends: usize,
        resumes: usize,
        restores: usize,
        stops: usize,
        clears: usize,
        /// Calls to `terminal()`: one per loop pass (the draw).
        draws: usize,
        /// `draws` when the last `stop_self` ran.
        draws_at_stop: usize,
        /// `stop_self` fails, as `kill` could.
        stop_fails: bool,
        /// Everything `write_raw` got.
        raw: String,
        /// `write_raw` fails.
        raw_fails: bool,
        /// `resume` fails, as for a terminal that went away.
        resume_fails: bool,
        /// The width override at each draw, suspend and restore, in order.
        widths: Vec<(&'static str, Option<usize>)>,
        /// The width the loop set last.
        width: Option<usize>,
    }

    impl Host for TestHost {
        type B = TestBackend;
        type Events = BoxStream<'static, io::Result<Event>>;
        type Signals = mpsc::UnboundedReceiver<Signal>;
        fn terminal(&mut self) -> &mut Terminal<TestBackend> {
            self.draws += 1;
            self.widths.push(("draw", self.width));
            &mut self.terminal
        }
        fn streams(&mut self) -> (&mut Self::Events, &mut Self::Signals) {
            (&mut self.keys, &mut self.signals)
        }
        fn suspend(&mut self) {
            self.suspends += 1;
            self.widths.push(("suspend", self.width));
            for s in self.suspend_signals.drain(..) {
                self.signal_tx.unbounded_send(s).unwrap();
            }
        }
        fn resume(&mut self) -> Result<()> {
            self.resumes += 1;
            if self.resume_fails {
                return Err(crate::cli::screen::not_taken(io::Error::other("no tty")));
            }
            Ok(())
        }
        fn restore(&mut self) {
            self.restores += 1;
            self.widths.push(("restore", self.width));
        }
        /// Does not stop the test process. It sends SIGCONT, as the kernel
        /// does when the shell continues a stopped process.
        fn stop_self(&mut self) -> io::Result<()> {
            self.stops += 1;
            self.draws_at_stop = self.draws;
            if self.stop_fails {
                return Err(io::Error::other("kill failed"));
            }
            self.signal_tx.unbounded_send(Signal::Cont).unwrap();
            Ok(())
        }
        fn clear(&mut self) -> io::Result<()> {
            self.clears += 1;
            self.terminal.clear().map_err(|never| match never {})
        }
        fn suspended_output(&self) -> Output {
            self.suspended.clone()
        }
        fn write_raw(&mut self, text: &str) -> io::Result<()> {
            self.raw.push_str(text);
            if self.raw_fails {
                return Err(io::Error::other("tty gone"));
            }
            Ok(())
        }
        /// Records the width; tests never set the process-wide override.
        fn set_width(&mut self, width: Option<usize>) {
            self.width = width;
        }
    }

    /// What a driven session leaves: its result, state, host and the
    /// suspend-mode output.
    type Driven = (
        Result<TuiExit>,
        TuiState,
        TestHost,
        crate::cli::output::Captured,
    );

    fn press(code: KeyCode) -> io::Result<Event> {
        Ok(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    /// Key events that type each line and press Enter. `<esc>` presses Esc.
    fn script(lines: &[&str]) -> Vec<io::Result<Event>> {
        let mut events = Vec::new();
        for line in lines {
            if *line == "<esc>" {
                events.push(press(KeyCode::Esc));
                continue;
            }
            events.extend(line.chars().map(|c| press(KeyCode::Char(c))));
            events.push(press(KeyCode::Enter));
        }
        events
    }

    /// Runs the event loop over the scripted events. The stream ends after
    /// them, which ends the loop.
    async fn drive(events: Vec<io::Result<Event>>) -> Driven {
        drive_with(Journal::default(), events, Vec::new()).await
    }

    /// Runs a whole session (loop, restore, save) with `journal`. The
    /// signals arrive after the last key; then the key stream ends. The
    /// session context is not saved, because its real path is the user's.
    async fn drive_with(
        journal: Journal,
        events: Vec<io::Result<Event>>,
        signals: Vec<Signal>,
    ) -> Driven {
        let (host, captured) = test_host(events, signals);
        drive_host(journal, host, captured).await
    }

    /// A test host over the scripted keys, with `signals` after the last key.
    fn test_host(
        events: Vec<io::Result<Event>>,
        signals: Vec<Signal>,
    ) -> (TestHost, crate::cli::output::Captured) {
        let (sig_tx, sig_rx) = mpsc::unbounded();
        let mut then = Some((signals, sig_tx.clone()));
        // After the last key, send the signals and stay pending. The send
        // wakes the loop, and its biased select takes the signals before
        // it polls the key stream again and sees the end.
        let tail = futures::stream::poll_fn(move |_| match then.take() {
            Some((sigs, tx)) if !sigs.is_empty() => {
                for s in sigs {
                    tx.unbounded_send(s).unwrap();
                }
                Poll::Pending
            }
            _ => Poll::Ready(None),
        });
        let (suspended, captured) = Output::capture_merged();
        let host = TestHost {
            terminal: Terminal::new(TestBackend::new(100, 30)).unwrap(),
            keys: futures::stream::iter(events).chain(tail).boxed(),
            signals: sig_rx,
            signal_tx: sig_tx,
            suspend_signals: Vec::new(),
            suspended,
            suspends: 0,
            resumes: 0,
            restores: 0,
            stops: 0,
            clears: 0,
            draws: 0,
            draws_at_stop: 0,
            stop_fails: false,
            raw: String::new(),
            raw_fails: false,
            resume_fails: false,
            widths: Vec::new(),
            width: None,
        };
        (host, captured)
    }

    /// Runs a whole session on `host` with `journal`.
    async fn drive_host(
        journal: Journal,
        host: TestHost,
        captured: crate::cli::output::Captured,
    ) -> Driven {
        drive_host_on(journal, host, captured, Box::new(MockClientFactory::new())).await
    }

    /// Settings with the default config and `clients`.
    fn settings_with(clients: Box<dyn ChainClientFactory>) -> Settings {
        Settings {
            config: Config::default(),
            clients,
        }
    }

    /// [`drive_host`] with the client factory `clients`. A reload after
    /// `setup` gives the mock factory.
    async fn drive_host_on(
        journal: Journal,
        host: TestHost,
        captured: crate::cli::output::Captured,
        clients: Box<dyn ChainClientFactory>,
    ) -> Driven {
        let reload = || Ok(settings_with(Box::new(MockClientFactory::new())));
        drive_host_reloading(journal, host, captured, settings_with(clients), &reload).await
    }

    /// [`drive_host`] with `settings`, and `reload` after `setup`.
    async fn drive_host_reloading(
        journal: Journal,
        mut host: TestHost,
        captured: crate::cli::output::Captured,
        settings: Settings,
        reload: Reload<'_>,
    ) -> Driven {
        let mut state = TuiState::new(SessionContext::default(), journal, vocab::Vocab::default());
        welcome(&mut state, true);
        let res = session(&mut host, &mut state, settings, reload, |_| Ok(())).await;
        (res, state, host, captured)
    }

    /// A DEX source whose every lookup waits forever: a command that uses
    /// it stays running until it is cancelled, with no network (#39).
    struct PendingDex;

    #[async_trait::async_trait]
    impl scope::chains::DexDataSource for PendingDex {
        async fn get_token_price(&self, _: &str, _: &str) -> Option<f64> {
            std::future::pending().await
        }
        async fn get_native_token_price(&self, _: &str) -> Option<f64> {
            std::future::pending().await
        }
        async fn get_token_data(
            &self,
            _: &str,
            _: &str,
        ) -> scope::Result<scope::chains::dex::DexTokenData> {
            std::future::pending().await
        }
        async fn search_tokens(
            &self,
            _: &str,
            _: Option<&str>,
        ) -> scope::Result<Vec<scope::chains::dex::TokenSearchResult>> {
            std::future::pending().await
        }
    }

    /// The mock factory with a [`PendingDex`].
    struct PendingFactory(MockClientFactory);

    impl ChainClientFactory for PendingFactory {
        fn http(&self) -> std::sync::Arc<dyn scope::http::HttpClient> {
            scope::http::test_transport()
        }
        fn create_chain_client(
            &self,
            chain: &str,
        ) -> scope::Result<Box<dyn scope::chains::ChainClient>> {
            self.0.create_chain_client(chain)
        }
        fn create_dex_client(&self) -> Box<dyn scope::chains::DexDataSource> {
            Box::new(PendingDex)
        }
    }

    /// The command the cancel tests start: a repeat run whose first
    /// lookup never finishes ([`PendingDex`]).
    const PENDING_REPEAT: &str = "market summary USDC --venue eth --every 30s --duration 1h";

    /// Types `lines`, then delivers `signals`, with the DEX lookups pending.
    async fn drive_pending(events: Vec<io::Result<Event>>, signals: Vec<Signal>) -> Driven {
        let (host, captured) = test_host(events, signals);
        let clients = PendingFactory(MockClientFactory::new());
        drive_host_on(Journal::default(), host, captured, Box::new(clients)).await
    }

    /// Types `lines`, then delivers `signal`.
    async fn drive_signal_after(lines: &[&str], signal: Signal) -> Driven {
        drive_with(Journal::default(), script(lines), vec![signal]).await
    }

    #[tokio::test]
    async fn test_paste_event_fills_the_input_and_does_not_run_it() {
        // A paste that ends in a newline must not submit the line.
        let _lock = DIAG_LOCK.lock().await;
        let (res, state, _, _) = drive(vec![Ok(Event::Paste("ctx\nhelp\n".into()))]).await;
        assert_eq!(res.unwrap().code, 0);
        assert_eq!(state.input, "ctx help");
        assert!(state.history.is_empty(), "nothing was submitted");
    }

    #[tokio::test]
    async fn test_term_signal_saves_restores_and_journals() {
        // A killed TUI must not leave a raw terminal or lose the history.
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::at(dir.path().join("h"), dir.path().join("e.jsonl"));
        let (res, _, host, _) = drive_with(journal, script(&["ctx"]), vec![Signal::Term]).await;
        assert_eq!(res.unwrap().code, 143);
        assert_eq!(host.restores, 1, "terminal restored once");
        let ev = std::fs::read_to_string(dir.path().join("e.jsonl")).unwrap();
        assert!(ev.contains("\"outcome\":\"signal:TERM\""), "{ev}");
        let history = std::fs::read_to_string(dir.path().join("h")).unwrap();
        assert!(history.contains("ctx"), "{history}");
    }

    #[tokio::test]
    async fn test_term_signal_cancels_running_pane_command() {
        // A repeat command sleeps between runs; SIGTERM must end it at
        // once and say so, not wait for the command.
        let _lock = DIAG_LOCK.lock().await;
        let (res, state, _, _) = drive_pending(script(&[PENDING_REPEAT]), vec![Signal::Term]).await;
        assert_eq!(res.unwrap().code, 143);
        assert!(state.scrollback_text().contains("Cancelled"));
        assert!(state.running.is_none());
    }

    #[tokio::test]
    async fn test_each_fatal_signal_exits_with_128_plus_its_number() {
        // Shells and supervisors read 128+N as "ended by signal N".
        let _lock = DIAG_LOCK.lock().await;
        for (sig, code) in [
            (Signal::Hup, 129),
            (Signal::Int, 130),
            (Signal::Quit, 131),
            (Signal::Term, 143),
        ] {
            let (res, _, host, _) = drive_signal_after(&[], sig).await;
            assert_eq!(res.unwrap().code, code, "{sig:?}");
            assert_eq!(host.restores, 1, "{sig:?}");
        }
    }

    #[tokio::test]
    async fn test_stop_and_continue_signals_do_not_end_the_tui() {
        // SIGTSTP and SIGCONT must not end the session: the loop ends only
        // when the keys end, with code 0.
        let _lock = DIAG_LOCK.lock().await;
        for sig in [Signal::Tstp, Signal::Cont] {
            let (res, _, host, _) = drive_signal_after(&[], sig).await;
            assert_eq!(res.unwrap().code, 0, "{sig:?}");
            assert_eq!(host.restores, 1, "{sig:?}");
        }
    }

    fn ctrl_z() -> io::Result<Event> {
        Ok(Event::Key(KeyEvent::new(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL,
        )))
    }

    /// The text on the test screen.
    fn screen_text(host: &TestHost) -> String {
        let buf = host.terminal.backend().buffer().clone();
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    /// The width override at each `stage` (draw, suspend, restore).
    fn widths_at(host: &TestHost, stage: &str) -> Vec<Option<usize>> {
        host.widths
            .iter()
            .filter(|(s, _)| *s == stage)
            .map(|(_, w)| *w)
            .collect()
    }

    #[tokio::test]
    async fn test_width_override_is_the_pane_and_is_removed_when_the_tui_leaves() {
        // Output fits the pane while the TUI owns the screen. A suspend-mode
        // command, the shell after Ctrl-Z and the CLI after exit use the
        // real width (review focus 5).
        let _lock = DIAG_LOCK.lock().await;
        let mut events = script(&["setup --key nosuchkey"]);
        events.push(ctrl_z());
        events.push(press(KeyCode::Char('x')));
        let (res, _, host, _) = drive(events).await;
        res.unwrap();
        // Suspend-mode command, then Ctrl-Z: the override is off both times.
        assert_eq!(widths_at(&host, "suspend"), vec![None, None]);
        assert_eq!(widths_at(&host, "restore"), vec![None]);
        // The pane of the 100-column test screen has no side borders.
        let draws = widths_at(&host, "draw");
        assert_eq!(draws.last(), Some(&Some(100)), "set again after Ctrl-Z");
    }

    /// The mock factory, counting the DEX clients it makes.
    struct CountingFactory(Arc<std::sync::atomic::AtomicUsize>);

    impl ChainClientFactory for CountingFactory {
        fn http(&self) -> std::sync::Arc<dyn scope::http::HttpClient> {
            scope::http::test_transport()
        }
        fn create_chain_client(
            &self,
            chain: &str,
        ) -> scope::Result<Box<dyn scope::chains::ChainClient>> {
            MockClientFactory::new().create_chain_client(chain)
        }
        fn create_dex_client(&self) -> Box<dyn scope::chains::DexDataSource> {
            self.0.fetch_add(1, Ordering::SeqCst);
            MockClientFactory::new().create_dex_client()
        }
    }

    #[tokio::test]
    async fn test_after_setup_the_next_command_uses_the_reloaded_clients() {
        // #99: a key saved by `setup` works without a restart (a stale key
        // gave "Invalid API Key" right after a successful setup).
        let _lock = DIAG_LOCK.lock().await;
        let first = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reloaded = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reloads = std::cell::Cell::new(0);
        let reload = || {
            reloads.set(reloads.get() + 1);
            Ok(settings_with(Box::new(CountingFactory(reloaded.clone()))))
        };
        let (host, captured) = test_host(script(&["setup --key nosuchkey", "crawl USDC"]), vec![]);
        let (res, state, _, _) = drive_host_reloading(
            Journal::default(),
            host,
            captured,
            settings_with(Box::new(CountingFactory(first.clone()))),
            &reload,
        )
        .await;
        res.unwrap();
        assert_eq!(reloads.get(), 1, "setup reads the settings once");
        assert!(state.scrollback_text().contains(SETUP_RELOADED_NOTE));
        assert_eq!(
            first.load(Ordering::SeqCst),
            0,
            "the old clients are not used"
        );
        assert!(
            reloaded.load(Ordering::SeqCst) > 0,
            "crawl used the new clients"
        );
    }

    #[tokio::test]
    async fn test_a_failed_reload_keeps_the_settings_and_says_to_restart() {
        let _lock = DIAG_LOCK.lock().await;
        let reload = || Err(ScopeError::Other("bad yaml".into()));
        let (host, captured) = test_host(script(&["setup --key nosuchkey"]), vec![]);
        let (res, state, _, _) = drive_host_reloading(
            Journal::default(),
            host,
            captured,
            settings_with(Box::new(MockClientFactory::new())),
            &reload,
        )
        .await;
        res.unwrap();
        let text = state.scrollback_text();
        assert!(text.contains("could not be read again"), "{text}");
        assert!(text.contains("bad yaml"), "{text}");
        assert!(!text.contains(SETUP_RELOADED_NOTE), "{text}");
    }

    #[tokio::test]
    async fn test_other_commands_do_not_reload() {
        let _lock = DIAG_LOCK.lock().await;
        let reloads = std::cell::Cell::new(0);
        let reload = || {
            reloads.set(reloads.get() + 1);
            Ok(settings_with(Box::new(MockClientFactory::new())))
        };
        let (host, captured) = test_host(script(&["crawl USDC", "venues list"]), vec![]);
        let (res, state, _, _) = drive_host_reloading(
            Journal::default(),
            host,
            captured,
            settings_with(Box::new(MockClientFactory::new())),
            &reload,
        )
        .await;
        res.unwrap();
        assert_eq!(reloads.get(), 0);
        assert!(!state.scrollback_text().contains(SETUP_RELOADED_NOTE));
    }

    #[tokio::test]
    async fn test_ctrl_z_stops_then_redraws_with_the_draft() {
        // Ctrl-Z must give the shell a clean terminal, stop the process,
        // and on `fg` take the screen back, clear what the shell wrote,
        // and redraw with the draft. The SIGCONT that `fg` sends must not
        // take the screen a second time.
        let _lock = DIAG_LOCK.lock().await;
        let mut events: Vec<_> = "draf".chars().map(|c| press(KeyCode::Char(c))).collect();
        events.push(ctrl_z());
        let (res, state, host, _) = drive(events).await;
        assert_eq!(res.unwrap().code, 0, "Ctrl-Z does not end the TUI");
        assert_eq!(host.stops, 1);
        assert_eq!((host.suspends, host.resumes, host.clears), (1, 1, 1));
        assert_eq!(state.input, "draf");
        assert!(screen_text(&host).contains("draf"));
    }

    #[tokio::test]
    async fn test_tstp_signal_stops_like_ctrl_z() {
        // `kill -TSTP` must stop the TUI the same way; its handler replaced
        // the default stop, so without this the signal does nothing.
        let _lock = DIAG_LOCK.lock().await;
        let (res, _, host, _) = drive_signal_after(&[], Signal::Tstp).await;
        assert_eq!(res.unwrap().code, 0);
        assert_eq!(host.stops, 1);
        assert_eq!((host.suspends, host.resumes, host.clears), (1, 1, 1));
    }

    #[tokio::test]
    async fn test_cont_without_our_stop_retakes_and_clears_the_screen() {
        // After an outside SIGSTOP, the shell reset the terminal and wrote
        // on it. SIGCONT must set raw mode again and repaint everything.
        let _lock = DIAG_LOCK.lock().await;
        let (res, _, host, _) = drive_signal_after(&[], Signal::Cont).await;
        assert_eq!(res.unwrap().code, 0);
        assert_eq!(host.stops, 0, "SIGCONT does not stop the process");
        assert_eq!((host.suspends, host.resumes, host.clears), (1, 1, 1));
    }

    #[tokio::test]
    async fn test_ctrl_z_keeps_a_running_pane_command() {
        // Review Focus 2: a command running in the pane survives Ctrl-Z,
        // and after `fg` the screen shows its running footer again.
        let _lock = DIAG_LOCK.lock().await;
        let mut events = script(&[PENDING_REPEAT]);
        events.push(ctrl_z());
        let (res, state, host, _) = drive_pending(events, Vec::new()).await;
        assert_eq!(res.unwrap().code, 0);
        assert_eq!(host.stops, 1);
        assert!(state.running.is_some(), "the command still runs");
        assert!(!state.scrollback_text().contains("Cancelled"));
        let screen = screen_text(&host);
        assert!(screen.contains("Esc cancels"), "{screen}");
    }

    #[tokio::test(start_paused = true)]
    async fn test_a_long_stop_does_not_replay_the_missed_redraw_ticks() {
        // While the process is stopped the clock runs on, but nothing polls
        // the 100 ms redraw tick. On `fg` the missed ticks must not fire
        // one by one: the tick arm comes first in the biased select, so
        // they would hold back SIGCONT and the keys (10 per second stopped).
        const STOPPED: Duration = Duration::from_secs(120);
        const MAX_DRAWS_AFTER_STOP: usize = 10;
        let _lock = DIAG_LOCK.lock().await;
        let (mut host, _) = test_host(Vec::new(), Vec::new());
        // The clock jumps after the stop, before the next key is read.
        let after_stop = futures::stream::once(async {
            tokio::time::advance(STOPPED).await;
            press(KeyCode::Char('x'))
        });
        host.keys = futures::stream::iter(vec![ctrl_z()])
            .chain(after_stop)
            .boxed();
        // A command shown as running turns the tick on. A real command
        // would need the network, and the paused clock fires its timeouts
        // at once, so the test marks one as running without a future.
        let mut state = TuiState::new(
            SessionContext::default(),
            Journal::default(),
            vocab::Vocab::default(),
        );
        let config = Config::default();
        let Plan::Run(inv) = state.submit("venues list", &config) else {
            panic!("`venues list` did not plan a run")
        };
        let _out = state.start("venues list", &inv);
        let reload = || Ok(settings_with(Box::new(MockClientFactory::new())));
        let settings = settings_with(Box::new(MockClientFactory::new()));
        let res = session(&mut host, &mut state, settings, &reload, |_| Ok(())).await;
        assert_eq!(res.unwrap().code, 0);
        assert!(state.running.is_some(), "the tick stayed on");
        assert_eq!(host.stops, 1);
        assert_eq!(state.input, "x", "the key after the stop is handled");
        let draws = host.draws - host.draws_at_stop;
        assert!(
            draws <= MAX_DRAWS_AFTER_STOP,
            "{draws} draws between the stop and the end"
        );
    }

    #[tokio::test]
    async fn test_a_failed_stop_says_so_and_keeps_the_tui() {
        // If kill fails, the TUI must say so and take the screen back. No
        // SIGCONT follows, so a later outside SIGCONT must still repaint.
        let _lock = DIAG_LOCK.lock().await;
        let (mut host, captured) = test_host(vec![ctrl_z()], vec![Signal::Cont]);
        host.stop_fails = true;
        let (res, state, host, _) = drive_host(Journal::default(), host, captured).await;
        assert_eq!(res.unwrap().code, 0);
        assert!(
            state
                .scrollback_text()
                .contains("Could not stop the process: kill failed")
        );
        assert_eq!(host.stops, 1);
        // One set for the failed stop, one for the outside SIGCONT.
        assert_eq!((host.suspends, host.resumes, host.clears), (2, 2, 2));
    }

    #[tokio::test]
    async fn test_a_terminal_that_cannot_be_taken_back_ends_the_session_with_the_terminal_error() {
        // After Ctrl-Z and `fg`, the terminal may be gone. The TUI must
        // end with the same error the monitor shows, not panic, and still
        // restore and save.
        let _lock = DIAG_LOCK.lock().await;
        let (mut host, captured) = test_host(vec![ctrl_z()], vec![]);
        host.resume_fails = true;
        let (res, _, host, _) = drive_host(Journal::default(), host, captured).await;
        assert_eq!(
            res.unwrap_err().to_string(),
            "I/O error: cannot take the terminal: no tty"
        );
        assert_eq!(host.resumes, 1);
        assert_eq!(
            host.restores, 1,
            "the session still gives the terminal back"
        );
    }

    /// Awaits `fut`, failing the test after 10 s: a swallowed signal must
    /// fail the test, not hang it.
    async fn within_limit<T>(fut: impl Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(10), fut)
            .await
            .expect("the signal did not end the wait")
    }

    /// The journal records in `e.jsonl` under `dir`, parsed.
    fn records(dir: &tempfile::TempDir) -> Vec<serde_json::Value> {
        std::fs::read_to_string(dir.path().join("e.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn test_term_signal_during_a_suspended_command_ends_it() {
        // A suspend-mode command can run for hours (the monitor). On
        // develop SIGTERM killed it; the handler must not swallow it.
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let mut state = TuiState::new(
            SessionContext::default(),
            Journal::at(dir.path().join("h"), dir.path().join("e.jsonl")),
            vocab::Vocab::default(),
        );
        let (mut host, _) = test_host(Vec::new(), Vec::new());
        host.signal_tx.unbounded_send(Signal::Term).unwrap();
        let outside = Outside::new(host.suspended_output());
        let argv = vec!["monitor".to_string()];
        let never = futures::future::pending::<Result<()>>();
        let run = run_suspended(&mut host, "monitor", &argv, &outside, never, &mut state);
        let exit = within_limit(run).await;
        assert_eq!(exit, Some(TuiExit { code: 143 }));
        let rec = records(&dir);
        assert_eq!(rec.len(), 1, "{rec:?}");
        assert_eq!(rec[0]["outcome"], "signal:TERM");
        assert_eq!(rec[0]["route"], "suspend");
        assert_eq!(rec[0]["input"], "monitor");
    }

    /// A state over a journal in `dir`.
    fn journaled_state(dir: &tempfile::TempDir) -> TuiState {
        TuiState::new(
            SessionContext::default(),
            Journal::at(dir.path().join("h"), dir.path().join("e.jsonl")),
            vocab::Vocab::default(),
        )
    }

    /// The scrollback note `run_suspended` leaves for `line`, checked
    /// against the spec format with the outcome word `outcome`.
    fn assert_ran_note(state: &TuiState, line: &str, outcome: &str) {
        let text = state.scrollback_text();
        let note = text
            .lines()
            .find_map(|l| {
                l.strip_prefix("ℹ ")
                    .filter(|n| n.starts_with("Ran outside the TUI:"))
            })
            .unwrap_or_else(|| panic!("no note in {text}"));
        let pattern = format!(
            r"^Ran outside the TUI: {} \({}, \d+\.\ds\)\.$",
            regex::escape(line),
            outcome
        );
        assert!(
            regex::Regex::new(&pattern).unwrap().is_match(note),
            "{note:?} does not match {pattern}"
        );
    }

    #[tokio::test]
    async fn test_suspended_command_is_cancelled_by_sigint() {
        // Ctrl-C outside the TUI must stop a command that never ends (a
        // hung crawl) and return to the TUI, not kill the process.
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let mut state = journaled_state(&dir);
        let (mut host, captured) = test_host(Vec::new(), Vec::new());
        host.signal_tx.unbounded_send(Signal::Int).unwrap();
        let outside = Outside::new(host.suspended_output());
        let argv = vec!["crawl".to_string(), "USDC".to_string()];
        let never = futures::future::pending::<Result<()>>();
        let run = run_suspended(&mut host, "crawl USDC", &argv, &outside, never, &mut state);
        assert_eq!(within_limit(run).await, None, "the session goes on");
        assert_ran_note(&state, "crawl USDC", "cancelled");
        let rec = records(&dir);
        assert_eq!(rec.len(), 1, "{rec:?}");
        assert_eq!(rec[0]["outcome"], "cancelled");
        assert_eq!(rec[0]["route"], "suspend");
        assert!(captured.out().contains(
            "Running outside the TUI; it returns when the command ends (Ctrl-C cancels)."
        ));
    }

    #[tokio::test]
    async fn test_sigint_cancel_returns_to_a_working_loop() {
        // After the cancel the TUI takes the screen back and runs the next
        // line.
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::at(dir.path().join("h"), dir.path().join("e.jsonl"));
        let (mut host, captured) = test_host(script(&["setup --key nosuchkey", "ctx"]), Vec::new());
        host.suspend_signals = vec![Signal::Int];
        let (res, state, host, _) = within_limit(drive_host(journal, host, captured)).await;
        assert_eq!(res.unwrap().code, 0);
        assert_eq!((host.suspends, host.resumes), (1, 1));
        assert_ran_note(&state, "setup --key nosuchkey", "cancelled");
        assert!(state.scrollback_text().contains("Current Context"));
        let rec = records(&dir);
        assert_eq!(rec[0]["outcome"], "cancelled", "{rec:?}");
        assert_eq!(rec[1]["input"], "ctx", "{rec:?}");
    }

    /// Runs a suspend-mode `setup` whose prompt a signal stops: the
    /// command sets the cancel flag and sends `sig`, as the prompt watcher
    /// and the signal stream see one signal, and returns `result` (a
    /// prompt error, or Ok when the command swallowed it). `None` sends no
    /// signal. Returns the exit, the state, the host and the journal dir.
    ///
    /// `prompt::CANCEL` is process-global, so every test that touches it
    /// or calls `run_suspended` must hold `DIAG_LOCK`. This helper resets
    /// the flag before it returns.
    async fn prompt_stopped_by(
        sig: Option<Signal>,
        result: Result<()>,
    ) -> (Option<TuiExit>, TuiState, TestHost, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = journaled_state(&dir);
        let (mut host, _) = test_host(Vec::new(), Vec::new());
        let outside = Outside::new(host.suspended_output());
        let tx = host.signal_tx.clone();
        let argv = vec!["setup".to_string()];
        // The prompt blocks the task, so the select polled the signals
        // before the signal came.
        let prompt_stopped = async move {
            crate::cli::prompt::CANCEL.store(true, Ordering::SeqCst);
            if let Some(sig) = sig {
                tx.unbounded_send(sig).unwrap();
            }
            result
        };
        let run = run_suspended(
            &mut host,
            "setup",
            &argv,
            &outside,
            prompt_stopped,
            &mut state,
        );
        let exit = within_limit(run).await;
        crate::cli::prompt::CANCEL.store(false, Ordering::SeqCst);
        (exit, state, host, dir)
    }

    #[tokio::test]
    async fn test_a_cancelled_prompt_is_recorded_as_cancelled_and_its_int_used_up() {
        // Ctrl-C at a prompt: the cancel must read as a cancel, not a
        // failure, and its INT must not reach the TUI later and end the
        // session with 130.
        let _lock = DIAG_LOCK.lock().await;
        for result in [
            Err(ScopeError::IoError(io::Error::other("cancelled"))),
            Ok(()),
        ] {
            let (exit, state, mut host, dir) = prompt_stopped_by(Some(Signal::Int), result).await;
            assert_eq!(exit, None);
            assert_ran_note(&state, "setup", "cancelled");
            assert_eq!(records(&dir)[0]["outcome"], "cancelled");
            assert!(host.signals.try_next().is_err(), "the INT is used up");
        }
    }

    #[tokio::test]
    async fn test_term_at_a_prompt_ends_the_session_from_suspend_mode() {
        let _lock = DIAG_LOCK.lock().await;
        let failed = Err(ScopeError::IoError(io::Error::other("cancelled")));
        let (exit, _, _, dir) = prompt_stopped_by(Some(Signal::Term), failed).await;
        assert_eq!(exit, Some(TuiExit { code: 143 }));
        let rec = records(&dir);
        assert_eq!(rec.len(), 1, "{rec:?}");
        assert_eq!(rec[0]["outcome"], "signal:TERM");
        assert_eq!(rec[0]["route"], "suspend");
    }

    #[tokio::test]
    async fn test_a_suspended_command_starts_with_the_cancel_flag_clear() {
        // A Ctrl-C from an earlier command must not cancel the next one's
        // first prompt.
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let mut state = journaled_state(&dir);
        let (mut host, _) = test_host(Vec::new(), Vec::new());
        let outside = Outside::new(host.suspended_output());
        crate::cli::prompt::CANCEL.store(true, Ordering::SeqCst);
        let saw_clear = async {
            assert!(!crate::cli::prompt::CANCEL.load(Ordering::SeqCst));
            Ok(())
        };
        let run = run_suspended(&mut host, "setup", &[], &outside, saw_clear, &mut state);
        assert_eq!(within_limit(run).await, None);
        assert_ran_note(&state, "setup", "ok");
        crate::cli::prompt::CANCEL.store(false, Ordering::SeqCst);
    }

    /// The scrollback lines from `first` up to the `Ran outside the TUI`
    /// note; panics when either is missing or out of order.
    fn lines_before_note(state: &TuiState, first: &str) -> Vec<String> {
        let text = state.scrollback_text();
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let start = lines
            .iter()
            .position(|l| l.contains(first))
            .unwrap_or_else(|| panic!("{first:?} is not in the scrollback:\n{text}"));
        let note = lines
            .iter()
            .position(|l| l.contains("Ran outside the TUI:"))
            .unwrap_or_else(|| panic!("no note:\n{text}"));
        assert!(start < note, "the output comes before the note:\n{text}");
        lines[start..note].to_vec()
    }

    #[tokio::test]
    async fn test_a_suspended_report_shows_in_the_scrollback_and_is_the_copy() {
        // The report of a command that ran outside the TUI must not be
        // hidden until the TUI exits; Ctrl-Y and save must have its data.
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let mut state = journaled_state(&dir);
        let (mut host, captured) = test_host(Vec::new(), Vec::new());
        let outside = Outside::new(host.suspended_output());
        let out = &outside.out;
        let report = async {
            crate::outln!(out, "\x1b[1mToken Report\x1b[0m: USDC")?;
            crate::errln!(out, "warning: partial data")?;
            crate::outln!(out, "holders 42")?;
            Ok(())
        };
        let run = run_suspended(&mut host, "crawl USDC", &[], &outside, report, &mut state);
        assert_eq!(within_limit(run).await, None);
        let shown = lines_before_note(&state, "Token Report: USDC");
        assert_eq!(
            shown,
            ["Token Report: USDC", "warning: partial data", "holders 42"]
        );
        // The user saw it live on the terminal too.
        assert!(captured.out().contains("holders 42"));
        // Ctrl-Y sends the data only, without the warning or ANSI codes.
        let Action::Copy(copy) =
            state.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL))
        else {
            panic!("Ctrl-Y has nothing to copy")
        };
        assert_eq!(copy, "Token Report: USDC\nholders 42\n");
    }

    #[tokio::test]
    async fn test_a_failed_suspended_command_shows_its_output_and_error() {
        // A failure outside the TUI must explain itself in the TUI, and must
        // not replace the copy of an earlier success.
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let mut state = journaled_state(&dir);
        let (mut host, _) = test_host(Vec::new(), Vec::new());
        let outside = Outside::new(host.suspended_output());
        let out = &outside.out;
        let failed = async {
            crate::outln!(out, "searching USDC")?;
            Err(ScopeError::Other("crawl blew up".into()))
        };
        let run = run_suspended(&mut host, "crawl USDC", &[], &outside, failed, &mut state);
        assert_eq!(within_limit(run).await, None);
        assert_ran_note(&state, "crawl USDC", "failed");
        let shown = lines_before_note(&state, "searching USDC").join("\n");
        let error = shown.find("✗ crawl blew up").expect(&shown);
        assert!(shown.find("searching USDC").unwrap() < error, "{shown}");
        let copy = state.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        assert!(matches!(copy, Action::None), "no copy after a failure");
    }

    #[tokio::test(start_paused = true)]
    async fn test_a_set_flag_without_a_signal_does_not_hang_the_tui() {
        // The flag means a signal is on the stream. If that ever breaks,
        // the TUI must not wait forever on a cooked terminal.
        let _lock = DIAG_LOCK.lock().await;
        let started = tokio::time::Instant::now();
        let failed = Err(ScopeError::IoError(io::Error::other("cancelled")));
        let (exit, state, _, dir) = prompt_stopped_by(None, failed).await;
        assert_eq!(exit, None);
        assert!(started.elapsed() <= PROMPT_SIGNAL_WAIT + Duration::from_millis(100));
        assert_ran_note(&state, "setup", "cancelled");
        assert_eq!(records(&dir)[0]["outcome"], "cancelled");
    }

    #[tokio::test]
    async fn test_a_second_ctrl_c_does_not_end_the_session() {
        // A user often presses Ctrl-C twice. The second INT must not
        // reach the TUI and end it with 130.
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::at(dir.path().join("h"), dir.path().join("e.jsonl"));
        let (mut host, captured) = test_host(script(&["setup --key nosuchkey", "ctx"]), Vec::new());
        host.suspend_signals = vec![Signal::Int, Signal::Int];
        let (res, state, host, _) = within_limit(drive_host(journal, host, captured)).await;
        assert_eq!(res.unwrap().code, 0);
        assert_eq!((host.suspends, host.resumes), (1, 1));
        assert_ran_note(&state, "setup --key nosuchkey", "cancelled");
        assert!(state.scrollback_text().contains("Current Context"));
        let rec = records(&dir);
        assert_eq!(rec.len(), 2, "{rec:?}");
        assert_eq!(rec[1]["input"], "ctx", "{rec:?}");
    }

    #[tokio::test]
    async fn test_a_term_queued_after_a_cancel_still_ends_the_session() {
        // Draining extra INTs must not drop a TERM behind them.
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::at(dir.path().join("h"), dir.path().join("e.jsonl"));
        let (mut host, captured) = test_host(script(&["setup --key nosuchkey", "ctx"]), Vec::new());
        host.suspend_signals = vec![Signal::Int, Signal::Int, Signal::Term];
        let (res, _, host, _) = within_limit(drive_host(journal, host, captured)).await;
        assert_eq!(res.unwrap().code, 143);
        assert_eq!(host.resumes, 0);
        let rec = records(&dir);
        assert_eq!(rec[0]["outcome"], "cancelled", "{rec:?}");
        assert_eq!(rec[1]["outcome"], "signal:TERM", "{rec:?}");
        assert_eq!(rec[1]["route"], "suspend", "{rec:?}");
    }

    #[tokio::test]
    async fn test_ending_signals_set_the_prompt_cancel_flag() {
        // A prompt blocks the task that watches the signals, so a separate
        // watcher must set the flag. TSTP and CONT do not cancel.
        for (sigs, set) in [
            (vec![Signal::Tstp, Signal::Cont], false),
            (vec![Signal::Int], true),
            (vec![Signal::Term], true),
            (vec![Signal::Hup], true),
            (vec![Signal::Quit], true),
        ] {
            let flag = AtomicBool::new(false);
            cancel_prompts_on(futures::stream::iter(sigs.clone()), &flag).await;
            assert_eq!(flag.load(Ordering::SeqCst), set, "{sigs:?}");
        }
    }

    /// A writer whose terminal is gone.
    struct BrokenTty;

    impl io::Write for BrokenTty {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("tty gone"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_terminal_errors_outside_the_tui_are_reported_after_it() {
        // Nothing outside the TUI can show a broken terminal; the pane
        // must say so when the TUI is back.
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let mut state = journaled_state(&dir);
        let (mut host, _) = test_host(Vec::new(), Vec::new());
        let outside = Outside::new(Output::from_writers(BrokenTty, BrokenTty));
        let failed = async { Err(ScopeError::Other("boom".into())) };
        let run = run_suspended(&mut host, "setup", &[], &outside, failed, &mut state);
        assert_eq!(within_limit(run).await, None);
        assert_ran_note(&state, "setup", "failed");
        let text = state.scrollback_text();
        assert_eq!(
            text.matches("Terminal I/O error while outside the TUI: tty gone")
                .count(),
            1,
            "the error text; the header is a note, and a failed note is dropped: {text}"
        );
        assert_eq!(records(&dir)[0]["outcome"], "error");
    }

    #[tokio::test]
    async fn test_suspended_command_ignores_tstp_and_cont() {
        // The command owns the terminal, so TSTP and CONT do not stop the
        // TUI or take the screen, and do not end the command.
        let _lock = DIAG_LOCK.lock().await;
        let (mut host, captured) = test_host(script(&["setup --key nosuchkey"]), Vec::new());
        host.suspend_signals = vec![Signal::Tstp, Signal::Cont];
        let (res, state, host, _) = drive_host(Journal::default(), host, captured).await;
        assert_eq!(res.unwrap().code, 0);
        assert_eq!((host.suspends, host.resumes), (1, 1));
        assert_eq!((host.stops, host.clears), (0, 0));
        assert_ran_note(&state, "setup --key nosuchkey", "ok");
    }

    #[tokio::test]
    async fn test_event_stream_error_still_restores_once_and_saves() {
        // A broken terminal ends the loop with an error; the terminal is
        // still restored once and the history is still saved.
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::at(dir.path().join("h"), dir.path().join("e.jsonl"));
        let mut events = script(&["ctx"]);
        events.push(Err(io::Error::other("tty gone")));
        let (res, _, host, _) = drive_with(journal, events, Vec::new()).await;
        assert!(res.unwrap_err().to_string().contains("tty gone"));
        assert_eq!(host.restores, 1);
        let history = std::fs::read_to_string(dir.path().join("h")).unwrap();
        assert!(history.contains("ctx"), "{history}");
    }

    #[tokio::test]
    async fn test_normal_exit_restores_and_returns_code_zero() {
        let _lock = DIAG_LOCK.lock().await;
        let (res, _, host, _) = drive(script(&["exit"])).await;
        assert_eq!(res.unwrap().code, 0);
        assert_eq!(host.restores, 1);
    }

    #[tokio::test]
    async fn test_loop_runs_session_and_pane_commands_then_exits() {
        let _lock = DIAG_LOCK.lock().await;
        let (res, state, host, _) =
            drive(script(&["chain base", "venues list", "exit", "never runs"])).await;
        res.unwrap();
        let text = state.scrollback_text();
        assert!(text.contains("Type `help` for commands"));
        assert!(text.contains("Chain pinned to: base"));
        assert!(text.contains("Available Venues"), "{}", text);
        assert!(text.contains("Done in"));
        assert!(!text.contains("never runs"), "nothing runs after exit");
        // The screen shows the pinned chain in the prompt.
        let buf = host.terminal.backend().buffer().clone();
        let screen: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(screen.contains("scope:base>"));
    }

    fn ctrl_y_event() -> io::Result<Event> {
        Ok(Event::Key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::CONTROL,
        )))
    }

    #[tokio::test]
    async fn test_ctrl_y_sends_the_json_and_save_writes_it() {
        use base64::Engine as _;
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("one.json");
        let mut events = script(&[
            "venues list --format json",
            // Quoted: the line splits by shell rules, where `\` in a
            // Windows path is an escape.
            &format!("save '{}'", file.display()),
        ]);
        events.push(ctrl_y_event());
        let (res, state, host, _) = drive(events).await;
        res.unwrap();
        let body = std::fs::read_to_string(&file).unwrap();
        serde_json::from_str::<serde_json::Value>(&body).unwrap();
        let text = state.scrollback_text();
        assert!(text.contains("copied "), "{text}");
        let payload = host
            .raw
            .strip_prefix("\x1b]52;c;")
            .and_then(|r| r.strip_suffix('\x07'))
            .unwrap_or_else(|| panic!("not an OSC 52 sequence: {:?}", host.raw));
        let sent = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .unwrap();
        assert_eq!(String::from_utf8(sent).unwrap(), body);
    }

    #[tokio::test]
    async fn test_save_refuses_to_overwrite_then_force_replaces_and_both_are_journaled() {
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("two.json");
        std::fs::write(&file, "old").unwrap();
        // Quoted, as above.
        let save = format!("save '{}'", file.display());
        let journal = Journal::at(dir.path().join("h"), dir.path().join("e.jsonl"));
        let (res, state, _, _) = drive_with(
            journal,
            script(&[
                "venues list --format json",
                &save,
                &format!("{save} --force"),
            ]),
            Vec::new(),
        )
        .await;
        res.unwrap();
        let text = state.scrollback_text();
        assert!(text.contains("exists; add --force"), "{text}");
        assert!(text.contains("saved "), "{text}");
        let body = std::fs::read_to_string(&file).unwrap();
        assert_ne!(body, "old", "--force replaced the file");
        serde_json::from_str::<serde_json::Value>(&body).unwrap();
        let recs = records(&dir);
        let refused = recs.iter().find(|r| r["outcome"] == "refused").unwrap();
        assert_eq!(refused["input"], save);
        assert!(refused["error"].as_str().unwrap().contains("exists"));
        assert!(
            recs.iter()
                .any(|r| r["input"] == format!("{save} --force") && r["outcome"] == "session")
        );
    }

    #[tokio::test]
    async fn test_ctrl_y_results_are_journaled() {
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::at(dir.path().join("h"), dir.path().join("e.jsonl"));
        let mut events = vec![ctrl_y_event()];
        events.extend(script(&["venues list"]));
        events.push(ctrl_y_event());
        let (mut host, captured) = test_host(events, Vec::new());
        host.raw_fails = true;
        let (_, _, _, _) = drive_host(journal, host, captured).await;
        let recs = records(&dir);
        let ctrl_y: Vec<_> = recs.iter().filter(|r| r["input"] == "Ctrl-Y").collect();
        assert_eq!(ctrl_y.len(), 2, "{recs:?}");
        assert_eq!(ctrl_y[0]["outcome"], "refused");
        assert_eq!(ctrl_y[0]["error"], "nothing to copy yet");
        assert_eq!(ctrl_y[1]["outcome"], "error");
        assert!(ctrl_y[1]["error"].as_str().unwrap().contains("tty gone"));
    }

    #[tokio::test]
    async fn test_a_failed_copy_is_noted() {
        let _lock = DIAG_LOCK.lock().await;
        let mut events = script(&["venues list"]);
        events.push(Ok(Event::Key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::CONTROL,
        ))));
        let (mut host, captured) = test_host(events, Vec::new());
        host.raw_fails = true;
        let (_, state, _, _) = drive_host(Journal::default(), host, captured).await;
        assert!(state.scrollback_text().contains("Could not copy: tty gone"));
    }

    #[tokio::test]
    async fn test_watch_open_runs_the_grid_next_to_the_shell() {
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.yaml");
        std::fs::write(
            &path,
            "name: tokens\ninterval: 1h\ntargets:\n  - kind: token\n    id: \"0x1234567890123456789012345678901234567890\"\n    chain: ethereum\n    rules: [\"price_usd > 0\"]\n",
        )
        .unwrap();
        let line = format!(
            "watch open {}",
            path.display().to_string().replace('\\', "/")
        );
        let (res, mut state, host, _) = drive(script(&[&line])).await;
        res.unwrap();
        state.poll_watch();
        let g = state.grid.as_ref().expect("the grid opened");
        assert!(state.grid_shown);
        assert_eq!(g.list.name, "tokens");
        assert!(g.ready.is_some(), "the targets were resolved");
        let last = g.last.as_ref().expect("the first tick came");
        assert_eq!(
            last.snapshot.targets[0].status,
            scope::watch::TargetStatus::Passed
        );
        assert!(g.is_running(), "a 1h run goes on");
        assert!(
            screen_text(&host).contains("watchlist tokens"),
            "{}",
            screen_text(&host)
        );
        assert!(state.scrollback_text().contains("Watchlist tokens started"));
    }

    #[tokio::test]
    async fn test_esc_in_the_grid_stops_the_run_and_writes_the_report() {
        let _lock = DIAG_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.yaml");
        let report = dir.path().join("w.md");
        std::fs::write(
            &path,
            format!(
                "interval: 1h\noutputs:\n  report: '{}'\ntargets:\n  - kind: token\n    id: \"0x1234567890123456789012345678901234567890\"\n    rules: [\"price_usd > 0\"]\n",
                report.display()
            ),
        )
        .unwrap();
        let line = format!(
            "watch open {}",
            path.display().to_string().replace('\\', "/")
        );
        // A second, idle Enter lets the first tick land before Esc.
        let (res, mut state, _, _) = drive(script(&[&line, "", "<esc>"])).await;
        res.unwrap();
        let g = state.grid.as_ref().unwrap();
        assert!(!g.is_running());
        let (footer, _) = g.ended.clone().unwrap();
        assert!(footer.starts_with("Stopped after"), "{footer}");
        assert!(g.last.is_some(), "the first tick landed before Esc");
        assert!(footer.contains("Report saved to"), "{footer}");
        assert!(
            std::fs::read_to_string(&report)
                .unwrap()
                .contains("# Watchlist Report")
        );
        // Esc again closes the finished grid into the scrollback.
        state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(state.grid.is_none() && !state.grid_shown);
        assert!(
            state.scrollback_text().contains("watchlist w:"),
            "{}",
            state.scrollback_text()
        );
    }

    #[tokio::test]
    async fn test_ctrl_g_without_a_grid_says_how_to_start_one() {
        let _lock = DIAG_LOCK.lock().await;
        let (res, state, _, _) = drive(vec![Ok(Event::Key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
        )))])
        .await;
        res.unwrap();
        assert!(state.scrollback_text().contains("No watchlist runs"));
    }

    #[tokio::test]
    async fn test_loop_esc_cancels_a_running_repeat_command() {
        // `--every` sleeps between runs; Esc must stop it, not wait 1 minute.
        let _lock = DIAG_LOCK.lock().await;
        let (res, state, _, _) =
            drive_pending(script(&[PENDING_REPEAT, "<esc>"]), Vec::new()).await;
        res.unwrap();
        let text = state.scrollback_text();
        assert!(text.contains("Cancelled after"), "{}", text);
        assert!(state.running.is_none());
    }

    #[tokio::test]
    async fn test_loop_suspend_mode_hands_over_the_terminal_and_records() {
        // A suspend-mode command must release the screen, run on the plain
        // terminal, take the screen back, and leave a trace in the pane.
        // `setup --key` with an unknown name returns before reading stdin.
        let _lock = DIAG_LOCK.lock().await;
        let (res, state, host, suspended) = drive(script(&["setup --key nosuchkey"])).await;
        res.unwrap();
        assert_eq!((host.suspends, host.resumes), (1, 1));
        assert!(
            suspended
                .out()
                .contains("── scope setup --key nosuchkey ──")
        );
        assert!(suspended.out().contains(
            "Running outside the TUI; it returns when the command ends (Ctrl-C cancels)."
        ));
        // It returns by itself: there is no Enter to wait for.
        assert!(!suspended.out().contains("Press Enter"));
        assert_ran_note(&state, "setup --key nosuchkey", "ok");
    }

    #[tokio::test]
    async fn test_loop_ignores_non_press_keys_and_ends_on_stream_end() {
        let _lock = DIAG_LOCK.lock().await;
        let mut events = vec![Ok(Event::Resize(80, 24))];
        let mut release = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        release.state = KeyEventState::NONE;
        events.push(Ok(Event::Key(release)));
        let (res, state, _, _) = drive(events).await;
        res.unwrap();
        assert_eq!(state.input, "", "a key release must not type");
    }

    #[tokio::test]
    async fn test_loop_ctrl_d_exits() {
        let _lock = DIAG_LOCK.lock().await;
        let (res, _, _, _) = drive(vec![Ok(Event::Key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
        )))])
        .await;
        res.unwrap();
    }

    #[tokio::test]
    async fn test_loop_returns_event_stream_errors() {
        // A broken terminal must end the TUI with the error, not spin.
        let _lock = DIAG_LOCK.lock().await;
        let (res, _, _, _) = drive(vec![Err(io::Error::other("tty gone"))]).await;
        assert!(res.unwrap_err().to_string().contains("tty gone"));
    }

    #[tokio::test]
    async fn test_library_warning_lands_in_the_running_command_output() {
        // scope-core warnings use diag::notice; while a pane command runs
        // they must join its output, not print over the TUI screen.
        let _lock = DIAG_LOCK.lock().await;
        let mut s = TuiState::new(
            SessionContext::default(),
            Journal::default(),
            vocab::Vocab::default(),
        );
        let Plan::Run(inv) = s.submit("venues list", &Config::default()) else {
            panic!()
        };
        let _out = s.start("venues list", &inv);
        scope::notice!("  ⚠ library warning");
        s.finish(Ending::Finished(Ok(())));
        assert!(s.scrollback_text().contains("⚠ library warning"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_signal_stream_installs_all_handlers() {
        // Installing handlers sends no signal. A failure to install one
        // must stop the TUI before it takes the terminal.
        assert!(signal_stream().is_ok());
    }
}
