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
pub mod exec;
pub mod journal;

use app::{Action, Ending, TuiState, run_invocation};
use crossterm::event::{Event, EventStream, KeyEventKind};
use exec::{Plan, Route};
use futures::StreamExt;
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

/// Runs the TUI until the user leaves.
///
/// # Errors
///
/// Returns a terminal I/O error, or the error from saving the history or
/// the session context on exit.
pub async fn run(no_banner: bool, config: &Config, clients: &dyn ChainClientFactory) -> Result<()> {
    let mut state = TuiState::new(SessionContext::load(), Journal::in_data_dir());
    if !no_banner {
        state.push_ansi(BANNER);
    }
    state.push_note("Type `help` for commands. `exit` or Ctrl-D leaves.");

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut state, config, clients).await;
    ratatui::restore();

    // Save both even if one fails, then report the first failure.
    let saved_history = state.journal().save_history(&state.history);
    let saved_ctx = state.ctx.save();
    result?;
    saved_history?;
    saved_ctx
}

/// Serializes tests that redirect the process-global diagnostics sink
/// (every test that calls `TuiState::start`). Without it, parallel tests
/// swap each other's sink. tokio's Mutex, so async tests can hold it
/// across `.await` without `clippy::await_holding_lock`.
#[cfg(test)]
pub(crate) static DIAG_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type PaneFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    state: &mut TuiState,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    // Library warnings between commands land here and join the scrollback.
    let idle_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut idle_diag = Some(scope::diag::redirect(idle_buf.clone()));
    let mut events = EventStream::new();
    let mut pending: Option<PaneFuture<'_>> = None;
    let mut tick = tokio::time::interval(LIVE_REDRAW);

    loop {
        drain_into(&idle_buf, state);
        terminal.draw(|f| state.render(f))?;

        tokio::select! {
            ev = events.next() => {
                let key = match ev {
                    Some(Ok(Event::Key(k))) if k.kind == KeyEventKind::Press => k,
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return Err(ScopeError::IoError(e)),
                    None => return Ok(()),
                };
                match state.handle_key(key) {
                    Action::None => {}
                    Action::Exit => return Ok(()),
                    Action::Cancel => {
                        // Dropping the future cancels the handler at its
                        // next await point.
                        pending = None;
                        state.finish(Ending::Cancelled);
                    }
                    Action::Submit(line) => match state.submit(&line, config) {
                        Plan::Done => {}
                        Plan::Exit => return Ok(()),
                        Plan::Run(inv) => match inv.route {
                            Route::Pane => {
                                let out = state.start(&line, &inv);
                                pending = Some(Box::pin(async move {
                                    run_invocation(*inv, config, clients, &out).await
                                }));
                            }
                            Route::Suspend => {
                                // The event reader must not consume the
                                // keys the suspended command reads.
                                drop(events);
                                drop(idle_diag.take());
                                ratatui::restore();
                                run_suspended(&line, *inv, state, config, clients).await;
                                *terminal = ratatui::init();
                                idle_diag.replace(scope::diag::redirect(idle_buf.clone()));
                                events = EventStream::new();
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
            res = async { pending.as_mut().expect("guarded by precondition").await }, if pending.is_some() => {
                pending = None;
                state.finish(Ending::Finished(res));
            }
            _ = tick.tick(), if state.running.is_some() => {}
        }
    }
}

/// Runs one command on the plain terminal, then waits for Enter.
async fn run_suspended(
    line: &str,
    inv: exec::Invocation,
    state: &mut TuiState,
    config: &Config,
    clients: &dyn ChainClientFactory,
) {
    let started = Instant::now();
    let started_at = chrono::Utc::now().to_rfc3339();
    let argv = inv.argv.clone();
    let out = Output::stdio();
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
    let wait = tokio::task::spawn_blocking(|| {
        let mut s = String::new();
        io::stdin().lock().read_line(&mut s).map(|_| ())
    })
    .await;
    if let Ok(Err(e)) = wait {
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
        let mut s = TuiState::new(SessionContext::default(), Journal::default());
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

    #[tokio::test]
    async fn test_library_warning_lands_in_the_running_command_output() {
        // scope-core warnings use diag::notice; while a pane command runs
        // they must join its output, not print over the TUI screen.
        let _lock = DIAG_LOCK.lock().await;
        let mut s = TuiState::new(SessionContext::default(), Journal::default());
        let Plan::Run(inv) = s.submit("venues list", &Config::default()) else {
            panic!()
        };
        let _out = s.start("venues list", &inv);
        scope::notice!("  ⚠ library warning");
        s.finish(Ending::Finished(Ok(())));
        assert!(s.scrollback_text().contains("⚠ library warning"));
    }
}
