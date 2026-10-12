//! # Unified TUI
//!
//! `scope tui` (also `scope interactive` and `scope shell`) opens one
//! full-screen terminal UI. Every CLI command runs inside it with the same
//! flags and the same handlers as on the command line (issue #47).
//!
//! ```text
//! ┌ scope v0.6.0 ──────────────────────────────────────────┐
//! │ scope:auto> address 0x742d…                             │
//! │ ┌─ Address Analysis ─…                                  │
//! └─────────────────────────────────────────────────────────┘
//! ┌scope:auto> ─────────────────────────────────────────────┐
//! │ market summary USDC --venue binance                     │
//! └─────────────────────────────────────────────────────────┘
//!  chain auto · format Table · limit 100      help · exit
//! ```
//!
//! - [`exec`] parses a line: session commands, or the CLI's clap definition.
//! - [`app`] holds the state and draws the screen.
//! - [`journal`] keeps the history and the event log.
//!
//! A command that reads stdin or draws its own screen (the setup wizard, a
//! token search with a selection prompt, the live monitor) runs in suspend
//! mode: the TUI gives the terminal back while it runs.

pub mod app;
pub mod complete;
pub mod exec;
pub mod hint;
pub mod journal;
pub mod palette;
pub mod vocab;

use app::{Action, Ending, TuiState, run_invocation};
use crossterm::event::{Event, EventStream, KeyEventKind};
use exec::{Plan, Route};
use futures::StreamExt;
use futures::stream::BoxStream;
use journal::Journal;
use scope::chains::ChainClientFactory;
use scope::config::Config;
use scope::error::{Result, ScopeError};
use std::future::Future;
use std::io::{self, BufRead, Write};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::cli::interactive::SessionContext;
use crate::cli::output::Output;

/// How often the screen redraws while a command runs, so its output and
/// elapsed time stay current.
const LIVE_REDRAW: Duration = Duration::from_millis(100);

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

/// A process signal the TUI listens for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Runs the TUI until the user leaves or a signal ends it.
///
/// # Errors
///
/// Returns an error when the signal handlers cannot be installed, a
/// terminal I/O error, or the error from saving the history or the
/// session context on exit.
pub async fn run(
    no_banner: bool,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<TuiExit> {
    let signals = signal_stream()?;
    let mut state = TuiState::new(
        SessionContext::load(),
        Journal::in_data_dir(),
        vocab::Vocab::load(config),
    );
    welcome(&mut state, no_banner);
    let mut host = RealHost {
        terminal: ratatui::init(),
        events: None,
        signals,
    };
    session(&mut host, &mut state, config, clients, SessionContext::save).await
}

/// Runs the event loop, then restores the terminal and saves, also after
/// a signal. `save_ctx` saves the session context.
async fn session<H: Host>(
    host: &mut H,
    state: &mut TuiState,
    config: &Config,
    clients: &dyn ChainClientFactory,
    save_ctx: impl FnOnce(&SessionContext) -> Result<()>,
) -> Result<TuiExit> {
    let result = event_loop(host, state, config, clients).await;
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
        let mut handler = signal(kind)?;
        streams.push(futures::stream::poll_fn(move |cx| {
            handler.poll_recv(cx).map(|got| got.map(|()| sig))
        }));
    }
    Ok(futures::stream::select_all(streams).boxed())
}

/// No signal handling off unix.
#[cfg(not(unix))]
fn signal_stream() -> Result<BoxStream<'static, Signal>> {
    Ok(futures::stream::pending().boxed())
}

fn welcome(state: &mut TuiState, no_banner: bool) {
    if !no_banner {
        state.push_ansi(BANNER);
    }
    state.push_note("Type `help` for commands. `exit` or Ctrl-D leaves.");
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
    fn resume(&mut self);
    /// Gives the terminal back for good when the TUI ends.
    fn restore(&mut self);
    /// The output a suspend-mode command writes to.
    fn suspended_output(&self) -> Output;
    /// Waits until the user is ready to return to the TUI.
    fn wait_to_return(&mut self) -> impl Future<Output = io::Result<()>>;
}

/// The real terminal.
struct RealHost {
    terminal: ratatui::DefaultTerminal,
    /// `None` while a suspend-mode command owns stdin: the event reader
    /// must not consume the keys that command reads.
    events: Option<EventStream>,
    signals: BoxStream<'static, Signal>,
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
        ratatui::restore();
    }
    fn resume(&mut self) {
        self.terminal = ratatui::init();
    }
    fn restore(&mut self) {
        ratatui::restore();
    }
    fn suspended_output(&self) -> Output {
        Output::stdio()
    }
    async fn wait_to_return(&mut self) -> io::Result<()> {
        tokio::task::spawn_blocking(|| {
            let mut s = String::new();
            io::stdin().lock().read_line(&mut s).map(|_| ())
        })
        .await
        .map_err(io::Error::other)?
    }
}

type PaneFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

async fn event_loop<H: Host>(
    host: &mut H,
    state: &mut TuiState,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<TuiExit> {
    const LEFT: TuiExit = TuiExit { code: 0 };
    // Library warnings between commands land here and join the scrollback.
    let idle_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut idle_diag = Some(scope::diag::redirect(idle_buf.clone()));
    let mut pending: Option<PaneFuture<'_>> = None;
    let mut tick = tokio::time::interval(LIVE_REDRAW);

    loop {
        drain_into(&idle_buf, state);
        host.terminal()
            .draw(|f| state.render(f))
            .map_err(|e| ScopeError::Other(format!("terminal draw failed: {}", e)))?;

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
            _ = tick.tick(), if state.running.is_some() => {}
            Some(sig) = signals.next() => {
                // TSTP and CONT do not end the TUI.
                if let Some(code) = sig.exit_code() {
                    // Drop the command future first: that cancels it.
                    drop(pending.take());
                    state.finish(Ending::Cancelled);
                    state.record_signal(sig.name());
                    return Ok(TuiExit { code });
                }
            }
            ev = events.next() => {
                let key = match ev {
                    Some(Ok(Event::Key(k))) if k.kind == KeyEventKind::Press => k,
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return Err(ScopeError::IoError(e)),
                    None => return Ok(LEFT),
                };
                match state.handle_key(key) {
                    Action::None => {}
                    Action::Exit => return Ok(LEFT),
                    Action::Cancel => {
                        // Dropping the future cancels the handler at its
                        // next await point.
                        pending = None;
                        state.finish(Ending::Cancelled);
                    }
                    Action::Submit(line) => match state.submit(&line, config) {
                        Plan::Done => {}
                        Plan::Exit => return Ok(LEFT),
                        Plan::Run(inv) => match inv.route {
                            Route::Pane => {
                                let out = state.start(&line, &inv);
                                pending = Some(Box::pin(async move {
                                    run_invocation(*inv, config, clients, &out).await
                                }));
                            }
                            Route::Suspend => {
                                drop(idle_diag.take());
                                host.suspend();
                                run_suspended(host, &line, *inv, state, config, clients).await;
                                host.resume();
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

/// Runs one command outside the TUI, then waits for the user to return.
async fn run_suspended<H: Host>(
    host: &mut H,
    line: &str,
    inv: exec::Invocation,
    state: &mut TuiState,
    config: &Config,
    clients: &dyn ChainClientFactory,
) {
    let started = Instant::now();
    let started_at = chrono::Utc::now().to_rfc3339();
    let argv = inv.argv.clone();
    let out = host.suspended_output();
    let mut terminal_errors = Vec::new();
    if let Err(e) = crate::errln!(out, "\n── scope {} ──", line) {
        terminal_errors.push(e.to_string());
    }
    let result = run_invocation(inv, config, clients, &out).await;
    let duration = started.elapsed();
    if let Err(e) = &result
        && let Err(write_err) = crate::cli::errors::display_error(e, &out)
    {
        terminal_errors.push(write_err.to_string());
    }
    let prompt = crate::err!(out, "\nPress Enter to return to the TUI. ")
        .and_then(|()| io::stderr().flush());
    if let Err(e) = prompt {
        terminal_errors.push(e.to_string());
    }
    if let Err(e) = host.wait_to_return().await {
        terminal_errors.push(e.to_string());
    }
    state.record_suspended(line, &argv, duration, started_at, &result);
    for e in terminal_errors {
        state.push_note(&format!("Terminal I/O error while outside the TUI: {}", e));
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

    /// Runs the same line through the CLI path; returns output plus the
    /// error display, as the terminal would show them.
    async fn via_cli(line: &str, factory: &MockClientFactory) -> String {
        let mut argv = vec!["scope".to_string()];
        argv.extend(shlex::split(line).unwrap());
        let cli = Cli::try_parse_from(argv).unwrap();
        let (o, cap) = Output::capture_merged();
        let res =
            crate::cli::dispatch::dispatch_with(cli.command, &Config::default(), factory, &o).await;
        if let Err(e) = &res {
            crate::cli::errors::display_error(e, &o).unwrap();
        }
        plain(&cap.out())
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
        suspended: Output,
        suspends: usize,
        resumes: usize,
        restores: usize,
    }

    impl Host for TestHost {
        type B = TestBackend;
        type Events = BoxStream<'static, io::Result<Event>>;
        type Signals = mpsc::UnboundedReceiver<Signal>;
        fn terminal(&mut self) -> &mut Terminal<TestBackend> {
            &mut self.terminal
        }
        fn streams(&mut self) -> (&mut Self::Events, &mut Self::Signals) {
            (&mut self.keys, &mut self.signals)
        }
        fn suspend(&mut self) {
            self.suspends += 1;
        }
        fn resume(&mut self) {
            self.resumes += 1;
        }
        fn restore(&mut self) {
            self.restores += 1;
        }
        fn suspended_output(&self) -> Output {
            self.suspended.clone()
        }
        async fn wait_to_return(&mut self) -> io::Result<()> {
            Ok(())
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
        let (sig_tx, sig_rx) = mpsc::unbounded();
        let mut then = Some((signals, sig_tx));
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
        let mut host = TestHost {
            terminal: Terminal::new(TestBackend::new(100, 30)).unwrap(),
            keys: futures::stream::iter(events).chain(tail).boxed(),
            signals: sig_rx,
            suspended,
            suspends: 0,
            resumes: 0,
            restores: 0,
        };
        let mut state = TuiState::new(SessionContext::default(), journal, vocab::Vocab::default());
        welcome(&mut state, true);
        let factory = MockClientFactory::new();
        let res = session(&mut host, &mut state, &Config::default(), &factory, |_| {
            Ok(())
        })
        .await;
        (res, state, host, captured)
    }

    /// Types `lines`, then delivers `signal`.
    async fn drive_signal_after(lines: &[&str], signal: Signal) -> Driven {
        drive_with(Journal::default(), script(lines), vec![signal]).await
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
        let (res, state, _, _) = drive_signal_after(
            &["market summary USDC --venue eth --every 30s --duration 1h"],
            Signal::Term,
        )
        .await;
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
        // T2 acts on SIGTSTP and SIGCONT. Until then they must not end the
        // session: the loop ends only when the keys end, with code 0.
        let _lock = DIAG_LOCK.lock().await;
        for sig in [Signal::Tstp, Signal::Cont] {
            let (res, _, host, _) = drive_signal_after(&[], sig).await;
            assert_eq!(res.unwrap().code, 0, "{sig:?}");
            assert_eq!(host.restores, 1, "{sig:?}");
        }
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

    #[tokio::test]
    async fn test_loop_esc_cancels_a_running_repeat_command() {
        // `--every` sleeps between runs; Esc must stop it, not wait 1 minute.
        let _lock = DIAG_LOCK.lock().await;
        let (res, state, _, _) = drive(script(&[
            "market summary USDC --venue eth --every 30s --duration 1h",
            "<esc>",
        ]))
        .await;
        res.unwrap();
        let text = state.scrollback_text();
        assert!(text.contains("Cancelled."), "{}", text);
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
        assert!(
            suspended
                .out()
                .contains("Press Enter to return to the TUI.")
        );
        assert!(
            state
                .scrollback_text()
                .contains("Ran outside the TUI: setup --key nosuchkey (ok).")
        );
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
}
