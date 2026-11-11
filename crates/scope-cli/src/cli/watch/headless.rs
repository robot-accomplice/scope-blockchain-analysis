//! # Headless `scope watch` (#123)
//!
//! Runs a saved watchlist with no screen, for cron, CI and scripts. It uses
//! the engine and the runner of the grid ([`run_ticks`]). It prints each
//! tick, writes the CSV trail and the report, and ends with an exit code
//! that a script can read. Design: `docs/design/2026-11-10-headless-watch-design.md`.
//!
//! The handler [`run`] always returns a code. It never returns an error:
//! an error would reach `main` as exit code 1, and here 1 means "breach".

use super::fetch::TickTiming;
use super::run::{
    CellRecord, cell_record, csv_header, csv_rows, run_ticks, start_csv, write_report,
};
use super::state::{self, RunState};
use super::{Ready, Sources, Tick, monitor, prepare, watchlist_path};
use crate::cli::output::Output;
use crate::{note, outln};
use clap::{Args, ValueEnum};
use scope::chains::ChainClientFactory;
use scope::config::Config;
use scope::error::ScopeError;
use scope::watch::time::format_duration;
use scope::watch::{Event, RuleStatus, Snapshot, TargetStatus, Watchlist, file};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// No row breached and no row is unknown on the last tick.
pub const EXIT_PASS: i32 = 0;
/// At least one row breached on the last tick.
pub const EXIT_BREACH: i32 = 1;
/// No row breached, but at least one row is unknown (a cell is `Error`).
pub const EXIT_UNKNOWN: i32 = 2;
/// The run could not start: the watchlist or a flag is not valid.
pub const EXIT_NOT_STARTED: i32 = 3;
/// The CSV file or the report could not be written.
pub const EXIT_WRITE_ERROR: i32 = 4;

/// The time of a `ts` field.
const TIMESTAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";
/// The time that starts a line of the text format.
const CLOCK_FORMAT: &str = "%H:%M:%S";
/// The width of the label of an event line in the text format.
const EVENT_LABEL_WIDTH: usize = 8;
/// The note for a run that has no end.
const NO_DURATION_NOTE: &str = "No duration: runs until stopped. Use --once or --duration.";

const AFTER_HELP: &str = "\x1b[1mExamples:\x1b[0m
  scope watch pegs --once                          \x1b[2m# one tick, for cron and CI\x1b[0m
  scope watch pegs --once --every 5m               \x1b[2m# cron every 5 minutes: remembers the last run\x1b[0m
  scope watch pegs --every 30s --duration 1h --format json
  scope watch ./pegs.yaml --once --csv trail.csv --append

\x1b[1mExit codes:\x1b[0m
  0  No row breached and no row is unknown on the last tick (warnings pass)
  1  At least one row breached on the last tick
  2  No row breached, but a row is unknown (a venue error). Also: a usage error,
     and a stop before the first tick ended
  3  The run could not start: the watchlist, a flag or a token is not valid
  4  The CSV file or the report could not be written

The exit code comes from the last tick. The JSON output and the CSV file keep the history.
Stop a run with Ctrl-C or SIGTERM. A run with no duration goes on until you stop it.";

/// What goes to stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum WatchFormat {
    /// A summary line for each tick, then a line for each event.
    Text,
    /// JSON Lines: one object for each start, tick, event and end.
    Json,
    /// The CSV rows of the trail file.
    Csv,
}

/// Arguments for `scope watch`.
///
/// Runs a saved watchlist with no screen and ends with an exit code.
#[derive(Debug, Clone, Args)]
#[command(after_help = AFTER_HELP)]
pub struct WatchArgs {
    /// A saved watchlist name, or the path of a watchlist file.
    ///
    /// It is a path when it ends in `.yaml` or has a path separator.
    #[arg(value_name = "NAME|PATH")]
    pub list: String,

    /// Run one tick and stop.
    #[arg(long, conflicts_with = "duration")]
    pub once: bool,

    /// Time from one tick start to the next (30s, 5m). Overrides the file.
    /// With --once it does not wait: it says how far apart the runs are
    /// (the look-back of the first tick and the age at which the saved
    /// state is stale).
    #[arg(long, value_name = "INTERVAL")]
    pub every: Option<String>,

    /// Do not load or save what this list remembers between runs.
    #[arg(long)]
    pub no_state: bool,

    /// How long the run lasts (10m, 1h, or `forever`). Overrides the file.
    #[arg(long, value_name = "DURATION")]
    pub duration: Option<String>,

    /// What goes to stdout.
    #[arg(long, value_enum, default_value_t = WatchFormat::Text)]
    pub format: WatchFormat,

    /// Write one CSV row for each cell and tick. Overrides the file.
    #[arg(long, value_name = "PATH")]
    pub csv: Option<PathBuf>,

    /// Keep the rows already in the CSV file and add the new rows.
    #[arg(long)]
    pub append: bool,

    /// Write the report when the run ends or stops. Overrides the file.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
}

/// Why a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// `--once`: the one tick ran.
    Once,
    /// The duration ended.
    Duration,
    /// Ctrl-C or SIGTERM.
    Signal,
    /// The CSV file could not be written.
    OutputError,
    /// A reader closed stdout.
    StdoutClosed,
}

impl StopReason {
    /// The name in the `end` record.
    pub fn name(self) -> &'static str {
        match self {
            StopReason::Once => "once",
            StopReason::Duration => "duration",
            StopReason::Signal => "signal",
            StopReason::OutputError => "output_error",
            StopReason::StdoutClosed => "stdout_closed",
        }
    }
}

/// What a run needs from the process.
pub struct Context<'a> {
    pub config: &'a Config,
    pub clients: &'a dyn ChainClientFactory,
    /// The output of the run. It must not prompt.
    pub out: &'a Output,
    /// The folder of the saved watchlists.
    pub dir: PathBuf,
    /// The folder of the state files (what a run remembers for the next).
    pub state_dir: PathBuf,
    /// The venue IDs a watchlist may name.
    pub venues: Vec<String>,
}

/// The exit code of a run. The first rule that matches wins: a write error
/// (4), a breached row on the last tick (1), an unknown row (2), else 0.
/// A run with no finished tick (`last` is `None`) has no verdict: 2. A run
/// that could not start is [`EXIT_NOT_STARTED`]; it never reaches this
/// function.
pub fn exit_code(last: Option<&Snapshot>, write_failed: bool) -> i32 {
    if write_failed {
        return EXIT_WRITE_ERROR;
    }
    let Some(snapshot) = last else {
        return EXIT_UNKNOWN;
    };
    let any = |status| snapshot.targets.iter().any(|t| t.status == status);
    if any(TargetStatus::Breached) {
        EXIT_BREACH
    } else if any(TargetStatus::Unknown) {
        EXIT_UNKNOWN
    } else {
        EXIT_PASS
    }
}

/// Runs `args` until it ends or `stop` resolves, and returns the exit code.
/// `stop` is Ctrl-C and SIGTERM in the process ([`os_stop`]); a test passes
/// its own future.
pub async fn run(args: &WatchArgs, ctx: &Context<'_>, stop: impl Future<Output = ()>) -> i32 {
    let out = ctx.out;
    let path = watchlist_path(&args.list, &ctx.dir);
    let Some(mut list) = load(args, &path, ctx) else {
        return EXIT_NOT_STARTED;
    };
    if let Err(message) = apply_flags(&mut list, args) {
        note!(out, "{message}");
        return EXIT_NOT_STARTED;
    }
    let src = Sources {
        config: ctx.config,
        clients: ctx.clients,
    };
    let ready = match prepare(&list, src, out).await {
        Ok(ready) => ready,
        Err(e) => {
            note!(out, "{}: {}", path.display(), prepare_message(&e));
            return EXIT_NOT_STARTED;
        }
    };
    let list = &ready.list;
    if let Some(csv) = &list.outputs.csv
        && let Err(e) = start_csv(csv, args.append)
    {
        note!(out, "{e}");
        return EXIT_WRITE_ERROR;
    }

    let (state, start_history) = RunState::start(
        !args.no_state,
        &ctx.state_dir,
        state::identity(&args.list, &path),
        list,
        now_ms(),
    );
    let state = RefCell::new(state);

    if announce_start(args, &path, &ready, &state.borrow(), out).is_err() {
        // Stdout closed before the first tick: nothing to run for.
        let end = End {
            reason: StopReason::StdoutClosed,
            write_failed: false,
            runs: 0,
        };
        return finish(args, &ready, None, end, &state.borrow(), out);
    }

    let last: RefCell<Option<Tick>> = RefCell::new(None);
    let runs = Cell::new(0u64);
    let stdout_closed = Cell::new(false);
    let closed = tokio::sync::Notify::new();
    let ended = {
        let ticks = run_ticks(
            &ready,
            src,
            out,
            start_history,
            |n, at, tick, events, history| {
                runs.set(n);
                log_tick(n, tick, events);
                // The memory is a cache: a failure is a note, not an exit code.
                if let Some(why) = state.borrow_mut().save(history, now_ms()) {
                    note!(out, "{why}");
                }
                // The CSV row is written already. A closed stdout ends the run
                // after this tick; the tick still counts as the last one.
                if let Err(e) = print_tick(args.format, out, n, at, tick, events) {
                    note!(out, "Cannot write to stdout ({e}); stopping.");
                    stdout_closed.set(true);
                    closed.notify_one();
                }
                *last.borrow_mut() = Some(tick.clone());
            },
        );
        // Biased: a stop that is ready wins over a tick that is ready, so a
        // signal never waits for the next tick to end.
        tokio::select! {
            biased;
            () = stop => None,
            () = closed.notified() => None,
            r = ticks => Some(r),
        }
    };

    let mut write_failed = false;
    let reason = match ended {
        Some(Err(e)) => {
            note!(out, "{e}");
            write_failed = true;
            StopReason::OutputError
        }
        _ if stdout_closed.get() => StopReason::StdoutClosed,
        Some(Ok(_)) if args.once => StopReason::Once,
        Some(Ok(_)) => StopReason::Duration,
        None => StopReason::Signal,
    };
    let last = last.into_inner();
    let end = End {
        reason,
        write_failed,
        runs: runs.get(),
    };
    finish(args, &ready, last, end, &state.borrow(), out)
}

/// How a run ended, for [`finish`].
struct End {
    reason: StopReason,
    /// The CSV file could not be written.
    write_failed: bool,
    runs: u64,
}

/// Writes the report, prints the `end` record and returns the exit code.
fn finish(
    args: &WatchArgs,
    ready: &Ready,
    last: Option<Tick>,
    end: End,
    state: &RunState,
    out: &Output,
) -> i32 {
    let End {
        reason,
        mut write_failed,
        runs,
    } = end;
    let mut report_written = false;
    if let Some(path) = &ready.list.outputs.report {
        // A stop before the first tick still writes a report, from no rows.
        let tick = last.clone().unwrap_or_default();
        match write_report(path, &ready.list, &tick) {
            Ok(()) => {
                report_written = true;
                out.note_saved("Report", path.display());
            }
            Err(e) => {
                note!(out, "{e}");
                write_failed = true;
            }
        }
    }
    let code = exit_code(last.as_ref().map(|t| &t.snapshot), write_failed);
    tracing::info!(
        name = %ready.list.name,
        runs,
        reason = reason.name(),
        exit_code = code,
        report_written,
        write_failed,
        "watch run ended"
    );
    match args.format {
        WatchFormat::Json => {
            let mut end = json!({
                "type": "end",
                "ts": now(),
                "runs": runs,
                "reason": reason.name(),
                "exit_code": code,
                "report_written": report_written,
            });
            if let (Some(end), Value::Object(fields)) = (end.as_object_mut(), state.end_record()) {
                end.extend(fields);
            }
            // A closed stdout cannot take the record; the code still tells.
            let _ = outln!(out, "{end}");
        }
        WatchFormat::Text | WatchFormat::Csv => note!(
            out,
            "Run ended: {} after {runs} run(s). Exit code {code}.",
            reason.name()
        ),
    }
    let _ = out.flush_out();
    code
}

/// The watchlist at `path`, or `None` after the errors are printed.
fn load(args: &WatchArgs, path: &Path, ctx: &Context<'_>) -> Option<Watchlist> {
    let out = ctx.out;
    match file::load(path, Some(&ctx.venues)) {
        Ok(list) if list.targets.is_empty() => {
            note!(out, "{}: the watchlist has no targets", path.display());
            None
        }
        Ok(list) => Some(list),
        Err(errors) => {
            for e in errors.0 {
                match e.line {
                    Some(line) => note!(out, "{}:{line}: {}", path.display(), e.message),
                    None => note!(out, "{}: {}", path.display(), e.message),
                }
            }
            if !super::is_watchlist_path(&args.list) {
                let names = file::list(&ctx.dir);
                if names.is_empty() {
                    note!(out, "No saved watchlists in {}.", ctx.dir.display());
                } else {
                    note!(out, "Saved watchlists: {}", names.join(", "));
                }
            }
            None
        }
    }
}

/// Puts the flags over the file.
fn apply_flags(list: &mut Watchlist, args: &WatchArgs) -> Result<(), String> {
    if let Some(every) = &args.every {
        list.interval = monitor::parse_interval(every)?;
    }
    if args.once {
        // A run of length zero is one tick.
        list.duration = Some(Duration::ZERO);
    } else if let Some(duration) = &args.duration {
        list.duration = monitor::parse_run_duration(duration)?;
    }
    if let Some(csv) = &args.csv {
        list.outputs.csv = Some(csv.clone());
    }
    if let Some(report) = &args.report {
        list.outputs.report = Some(report.clone());
    }
    Ok(())
}

/// The text for a `prepare` error. The run never asks, so a symbol on many
/// chains is an error that says where the contract goes.
fn prepare_message(e: &ScopeError) -> String {
    match e {
        ScopeError::AmbiguousToken { input, candidates } => format!(
            "`{input}` is on more than one chain. Put the contract address in the `id` field of the target in the watchlist file. Candidates:\n  {}",
            candidates.join("\n  ")
        ),
        other => other.to_string(),
    }
}

/// Now, in ms since the epoch.
fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

fn now() -> String {
    chrono::Utc::now().format(TIMESTAMP_FORMAT).to_string()
}

/// The path as the run uses it, absolute when the system can tell.
fn full_path(path: &Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

/// The row labels of the run, in tick order.
fn row_labels(list: &Watchlist) -> Vec<String> {
    list.targets
        .iter()
        .flat_map(|t| t.rows().into_iter().map(|venue| t.row_label(venue)))
        .collect()
}

/// Prints the start of the run: the `start` record (JSON), the CSV header
/// (CSV), and a note with the paths and timing that the run uses.
fn announce_start(
    args: &WatchArgs,
    path: &Path,
    ready: &Ready,
    state: &RunState,
    out: &Output,
) -> std::io::Result<()> {
    let list = &ready.list;
    let csv = list.outputs.csv.as_deref().map(full_path);
    let report = list.outputs.report.as_deref().map(full_path);
    let rows = row_labels(list);
    let duration = list.duration.map(|d| d.as_secs());
    tracing::info!(
        name = %list.name,
        path = %path.display(),
        rows = ?rows,
        interval_secs = list.interval.as_secs(),
        duration_secs = ?duration,
        once = args.once,
        "watch run started"
    );
    let span = match duration {
        Some(0) => "one tick".to_string(),
        Some(secs) => format!("for {}", format_duration(secs)),
        None => "until stopped".to_string(),
    };
    note!(
        out,
        "Watching {} ({} row(s)) every {} {span}. CSV: {}. Report: {}.",
        list.name,
        rows.len(),
        format_duration(list.interval.as_secs()),
        csv.as_deref().unwrap_or("none"),
        report.as_deref().unwrap_or("none"),
    );
    if list.duration.is_none() {
        note!(out, "{NO_DURATION_NOTE}");
    }
    for line in state.start_notes() {
        note!(out, "{line}");
    }
    match args.format {
        WatchFormat::Text => {}
        WatchFormat::Csv => {
            crate::out!(out, "{}", csv_header())?;
        }
        WatchFormat::Json => {
            let start = json!({
                "type": "start",
                "ts": now(),
                "scope_version": scope::VERSION,
                "name": list.name,
                "path": full_path(path),
                "interval_secs": list.interval.as_secs(),
                "duration_secs": duration,
                "once": args.once,
                "rows": rows,
                "csv": csv,
                "report": report,
                "state": state.start_record(),
            });
            outln!(out, "{start}")?;
        }
    }
    out.flush_out()
}

/// One structured trace event for each row of the tick, for `-v`: which
/// row, its status and the exact errors. This is the record that explains
/// an exit code of 2 after the fact.
fn log_tick(run: u64, tick: &Tick, events: &[Event]) {
    for row in &tick.snapshot.targets {
        let errors: Vec<&str> = row
            .cells
            .iter()
            .filter_map(|c| match &c.status {
                RuleStatus::Error(e) => Some(e.as_str()),
                _ => None,
            })
            .collect();
        tracing::info!(
            run,
            row = %row.target,
            status = row.status.name(),
            errors = ?errors,
            "watch tick row"
        );
    }
    tracing::info!(run, events = events.len(), "watch tick events");
}

/// Prints one tick in the chosen format and flushes stdout.
fn print_tick(
    format: WatchFormat,
    out: &Output,
    run: u64,
    at: &str,
    tick: &Tick,
    events: &[Event],
) -> std::io::Result<()> {
    match format {
        WatchFormat::Text => {
            let time = chrono::Utc::now().format(CLOCK_FORMAT).to_string();
            crate::out!(out, "{}", tick_text(&time, run, &tick.snapshot, events))?;
        }
        WatchFormat::Csv => {
            crate::out!(out, "{}", csv_rows(at, &tick.snapshot))?;
        }
        WatchFormat::Json => {
            outln!(out, "{}", tick_json(run, at, tick))?;
            for event in events {
                outln!(out, "{}", event_json(run, at, event))?;
            }
        }
    }
    out.flush_out()
}

/// How many rows have each status.
fn counts(snapshot: &Snapshot) -> [usize; 4] {
    let n = |status| {
        snapshot
            .targets
            .iter()
            .filter(|t| t.status == status)
            .count()
    };
    [
        n(TargetStatus::Breached),
        n(TargetStatus::Warned),
        n(TargetStatus::Unknown),
        n(TargetStatus::Passed),
    ]
}

/// The text format of one tick: a summary line, a line for each event, and
/// a line for each `Error` cell (an error is not an event, but it explains
/// exit code 2).
fn tick_text(time: &str, run: u64, snapshot: &Snapshot, events: &[Event]) -> String {
    let [breached, warned, unknown, passed] = counts(snapshot);
    let mut text = format!(
        "{time} run {run} · {} rows · {breached} breached · {warned} warned · {unknown} unknown · {passed} passed\n",
        snapshot.targets.len()
    );
    for event in events {
        let label = match event {
            Event::BreachStarted { .. } => "BREACH",
            Event::BreachCleared { .. } => "CLEARED",
            Event::WarnStarted { .. } => "WARN",
            Event::WarnCleared { .. } => "WARN CLEARED",
        };
        text.push_str(&format!(
            "  {label:<EVENT_LABEL_WIDTH$} {}\n",
            event.banner()
        ));
    }
    for row in &snapshot.targets {
        for cell in &row.cells {
            if let RuleStatus::Error(e) = &cell.status {
                text.push_str(&format!(
                    "  {:<EVENT_LABEL_WIDTH$} {}: {}: {e}\n",
                    "ERROR", row.target, cell.label
                ));
            }
        }
    }
    text
}

/// The `tick` record: every cell of every row.
fn tick_json(run: u64, at: &str, tick: &Tick) -> Value {
    let snapshot = &tick.snapshot;
    let [breached, warned, unknown, passed] = counts(snapshot);
    let rows: Vec<Value> = snapshot
        .targets
        .iter()
        .map(|t| {
            let cells: Vec<CellRecord> = t.cells.iter().map(cell_record).collect();
            json!({
                "target": t.target,
                "venue": t.venue,
                "status": t.status.name(),
                "cells": cells,
            })
        })
        .collect();
    json!({
        "type": "tick",
        "run": run,
        "ts": at,
        "counts": {
            "rows": snapshot.targets.len(),
            "breached": breached,
            "warned": warned,
            "unknown": unknown,
            "passed": passed,
        },
        "rows": rows,
        "timing": timing_record(&tick.timing),
    })
}

/// The `timing` field of the `tick` record: how long the reads took, so a
/// slow tick or a timeout can be traced to one read after the fact. Times
/// are in ms from the start of the tick. `wall_ms` against `sum_ms` shows
/// what the concurrency saved.
fn timing_record(timing: &TickTiming) -> Value {
    let reads: Vec<Value> = timing
        .reads
        .iter()
        .map(|r| {
            json!({
                "row": r.row,
                "group": r.group,
                "queued_ms": r.queued_ms,
                "started_ms": r.started_ms,
                "took_ms": r.took_ms,
                "outcome": r.outcome.name(),
                "error": r.error,
            })
        })
        .collect();
    json!({
        "wall_ms": timing.wall_ms,
        "sum_ms": timing.sum_ms,
        "bound": timing.bound,
        "reads": reads,
    })
}

/// The `event` record.
fn event_json(run: u64, at: &str, event: &Event) -> Value {
    let (name, target, cell, message) = match event {
        Event::BreachStarted {
            target,
            cell,
            message,
        } => ("breach_started", target, cell, Some(message)),
        Event::BreachCleared { target, cell } => ("breach_cleared", target, cell, None),
        Event::WarnStarted {
            target,
            cell,
            message,
        } => ("warn_started", target, cell, Some(message)),
        Event::WarnCleared { target, cell } => ("warn_cleared", target, cell, None),
    };
    json!({
        "type": "event",
        "run": run,
        "ts": at,
        "event": name,
        "target": target,
        "cell": cell,
        "message": message,
    })
}

/// Resolves when the process gets Ctrl-C or SIGTERM (on Windows: Ctrl-C or
/// Ctrl-Break; there is no SIGTERM there). If the handlers cannot be set,
/// it never resolves, and the run goes on until it ends.
pub async fn os_stop() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let (Ok(mut interrupt), Ok(mut terminate)) = (
            signal(SignalKind::interrupt()),
            signal(SignalKind::terminate()),
        ) {
            tokio::select! {
                _ = interrupt.recv() => {}
                _ = terminate.recv() => {}
            }
            return;
        }
    }
    #[cfg(windows)]
    {
        use tokio::signal::windows::{ctrl_break, ctrl_c};
        if let (Ok(mut c), Ok(mut brk)) = (ctrl_c(), ctrl_break()) {
            tokio::select! {
                _ = c.recv() => {}
                _ = brk.recv() => {}
            }
            return;
        }
    }
    std::future::pending::<()>().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::dispatch::tests::OfflineFactory;
    use crate::cli::output::Captured;
    use crate::cli::watch::fetch::tests::usdc_on_eth;
    use crate::cli::{Cli, Commands};
    use clap::Parser;
    use scope::chains::mocks::MockClientFactory;
    use scope::watch::{Cell as WCell, TargetSnapshot};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// A pair on the `eth` DEX venue, which the mock factory serves with a
    /// healthy market, and no network.
    const HEALTHY: &str =
        "targets:\n  - kind: pair\n    id: USDC\n    venue: eth\n    preset: market health\n";
    /// The same pair with thresholds that no market meets.
    const STRICT: &str = "targets:\n  - kind: pair\n    id: USDC\n    venue: eth\n    preset: market health\n    health:\n      max_spread_pct: 0.000001\n      min_depth: 1000000000000000.0\n";
    /// `eth` works; `binance` has no fixture in the offline transport, so
    /// every read fails: a venue error.
    const TWO_VENUES: &str = "targets:\n  - kind: pair\n    id: USDC\n    venue: [eth, binance]\n    preset: market health\n";
    const TWO_VENUES_STRICT: &str = "targets:\n  - kind: pair\n    id: USDC\n    venue: [eth, binance]\n    preset: market health\n    health:\n      max_spread_pct: 0.000001\n      min_depth: 1000000000000000.0\n";

    fn factory() -> OfflineFactory {
        let mut f = OfflineFactory::new();
        f.mock = usdc_on_eth();
        f
    }

    fn new_dir(list: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pegs.yaml"), list).unwrap();
        dir
    }

    fn list_arg(dir: &tempfile::TempDir) -> String {
        dir.path().join("pegs.yaml").display().to_string()
    }

    /// Runs `scope watch <target> <flags>` in `dir`. `{dir}` in a flag is
    /// the folder.
    async fn exec_with(
        dir: &tempfile::TempDir,
        target: &str,
        flags: &[&str],
        factory: &dyn ChainClientFactory,
        out: &Output,
        stop: impl Future<Output = ()>,
    ) -> i32 {
        let folder = dir.path().display().to_string();
        let mut argv = vec!["scope".to_string(), "watch".into(), target.to_string()];
        argv.extend(flags.iter().map(|f| f.replace("{dir}", &folder)));
        let Commands::Watch(args) = Cli::try_parse_from(&argv).unwrap().command else {
            panic!("not watch")
        };
        let config = Config::default();
        let ctx = Context {
            config: &config,
            clients: factory,
            out,
            dir: dir.path().to_path_buf(),
            state_dir: dir.path().join("state"),
            venues: vec![
                "eth".into(),
                "binance".into(),
                "lat-a".into(),
                "lat-b".into(),
            ],
        };
        run(&args, &ctx, stop).await
    }

    struct Run {
        code: i32,
        cap: Captured,
        dir: tempfile::TempDir,
    }

    async fn exec(list: &str, flags: &[&str]) -> Run {
        let dir = new_dir(list);
        let (out, cap) = Output::capture();
        let code = exec_with(
            &dir,
            &list_arg(&dir),
            flags,
            &factory(),
            &out,
            std::future::pending(),
        )
        .await;
        Run { code, cap, dir }
    }

    fn json_lines(cap: &Captured) -> Vec<Value> {
        cap.out()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}")))
            .collect()
    }

    fn of_type<'a>(records: &'a [Value], kind: &str) -> Vec<&'a Value> {
        records.iter().filter(|r| r["type"] == kind).collect()
    }

    /// The fields of one CSV line (a quoted field may hold commas).
    fn csv_fields(line: &str) -> Vec<String> {
        let (mut fields, mut field, mut quoted) = (Vec::new(), String::new(), false);
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match (c, quoted) {
                ('"', true) if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                ('"', _) => quoted = !quoted,
                (',', false) => fields.push(std::mem::take(&mut field)),
                (c, _) => field.push(c),
            }
        }
        fields.push(field);
        fields
    }

    /// A snapshot of rows; a cell is `pass`, `warn`, `fail` or `error`.
    fn snapshot(rows: &[&[&str]]) -> Snapshot {
        Snapshot {
            targets: rows
                .iter()
                .enumerate()
                .map(|(i, statuses)| {
                    let cells = statuses
                        .iter()
                        .map(|s| WCell {
                            label: "c".into(),
                            rule: None,
                            value: None,
                            status: match *s {
                                "pass" => RuleStatus::Pass,
                                "warn" => RuleStatus::Warn("w".into()),
                                "fail" => RuleStatus::Fail("f".into()),
                                _ => RuleStatus::Error("e".into()),
                            },
                            detail: None,
                        })
                        .collect();
                    TargetSnapshot::new(format!("row{i}"), cells)
                })
                .collect(),
        }
    }

    // A script acts on the code, so each rule of section 8.1 is one row.
    #[test]
    fn the_exit_code_follows_the_rules_in_order() {
        let table: Vec<(Option<Snapshot>, bool, i32)> = vec![
            (Some(snapshot(&[&["pass"]])), false, EXIT_PASS),
            // A warning is a pass.
            (Some(snapshot(&[&["warn", "pass"]])), false, EXIT_PASS),
            (Some(snapshot(&[&["fail"]])), false, EXIT_BREACH),
            (Some(snapshot(&[&["error"]])), false, EXIT_UNKNOWN),
            // A breach is a sure finding: it wins over a missing reading.
            (Some(snapshot(&[&["error"], &["fail"]])), false, EXIT_BREACH),
            // The record of the run is lost: a write error wins over all.
            (Some(snapshot(&[&["pass"]])), true, EXIT_WRITE_ERROR),
            (Some(snapshot(&[&["fail"]])), true, EXIT_WRITE_ERROR),
            (Some(snapshot(&[&["error"]])), true, EXIT_WRITE_ERROR),
            // No tick finished: the verdict was not read, so it is not a pass.
            (None, false, EXIT_UNKNOWN),
            (None, true, EXIT_WRITE_ERROR),
        ];
        for (i, (last, write_failed, expected)) in table.into_iter().enumerate() {
            assert_eq!(exit_code(last.as_ref(), write_failed), expected, "row {i}");
        }
        // The five codes are the codes of the issue.
        assert_eq!(
            [
                EXIT_PASS,
                EXIT_BREACH,
                EXIT_UNKNOWN,
                EXIT_NOT_STARTED,
                EXIT_WRITE_ERROR
            ],
            [0, 1, 2, 3, 4]
        );
    }

    #[tokio::test]
    async fn a_healthy_market_exits_0() {
        let r = exec(HEALTHY, &["--once"]).await;
        assert_eq!(r.code, EXIT_PASS, "{}", r.cap.err());
        assert!(r.cap.out().contains("1 rows"), "{}", r.cap.out());
    }

    #[tokio::test]
    async fn a_failed_check_of_an_unhealthy_market_exits_1() {
        let r = exec(STRICT, &["--once"]).await;
        assert_eq!(r.code, EXIT_BREACH, "{}{}", r.cap.out(), r.cap.err());
        assert!(r.cap.out().contains("BREACH"), "{}", r.cap.out());
    }

    #[tokio::test]
    async fn a_rule_that_fails_exits_1() {
        let list = "targets:\n  - kind: pair\n    id: USDC\n    venue: eth\n    rules: [\"mid_price < 0.0001\"]\n";
        let r = exec(list, &["--once"]).await;
        assert_eq!(r.code, EXIT_BREACH, "{}{}", r.cap.out(), r.cap.err());
    }

    #[tokio::test]
    async fn a_venue_error_with_no_failed_rule_exits_2_and_says_why() {
        let list = "targets:\n  - kind: pair\n    id: USDC\n    venue: binance\n    preset: market health\n";
        let r = exec(list, &["--once"]).await;
        assert_eq!(r.code, EXIT_UNKNOWN, "{}", r.cap.err());
        // Without the ERROR line, code 2 has no visible reason.
        let out = r.cap.out();
        let error = out.lines().find(|l| l.contains("ERROR")).unwrap_or("");
        assert!(error.contains("USDC@binance"), "{out}");
    }

    // Section 12, test 4: no run starts, no tick, nothing on stdout, and
    // the CSV file of the run is not created.
    #[tokio::test]
    async fn a_list_or_flag_that_is_not_valid_exits_3_before_any_tick() {
        let bad: &[(&str, &str, &[&str])] = &[
            ("a yaml error", "targets: [", &[]),
            (
                "an unknown metric",
                "targets:\n  - kind: pair\n    id: USDC\n    venue: eth\n    rules: [\"holdrs > 1\"]\n",
                &[],
            ),
            (
                "a metric the kind does not have",
                "targets:\n  - kind: pair\n    id: USDC\n    venue: eth\n    rules: [\"holders >= 1\"]\n",
                &[],
            ),
            (
                "an unknown venue",
                "targets:\n  - kind: pair\n    id: USDC\n    venue: nowhere\n    preset: market health\n",
                &[],
            ),
            ("a list with no targets", "name: empty\ntargets: []\n", &[]),
            ("a bad --every", HEALTHY, &["--every", "soon"]),
            ("a bad --duration", HEALTHY, &["--duration", "soon"]),
        ];
        for (what, list, flags) in bad {
            let mut flags: Vec<&str> = flags.to_vec();
            flags.extend(["--csv", "{dir}/trail.csv", "--format", "json"]);
            let r = exec(list, &flags).await;
            let err = r.cap.err();
            assert_eq!(r.code, EXIT_NOT_STARTED, "{what}: {err}");
            assert_eq!(r.cap.out(), "", "{what}: no record on stdout");
            assert!(!err.contains("Run #"), "{what}: a tick ran: {err}");
            assert!(
                !r.dir.path().join("trail.csv").exists(),
                "{what}: the run started"
            );
            assert!(
                err.contains("pegs.yaml")
                    || *what == "a bad --every"
                    || *what == "a bad --duration",
                "{what}: {err}"
            );
        }
    }

    #[tokio::test]
    async fn a_file_error_names_the_file_and_the_line() {
        let list =
            "targets:\n  - kind: pair\n    id: USDC\n    venue: eth\n    rules: [\"holdrs > 1\"]\n";
        let r = exec(list, &["--once"]).await;
        let err = r.cap.err();
        let line = err.lines().find(|l| l.contains("holdrs")).unwrap_or("");
        let path = list_arg(&r.dir);
        assert!(line.starts_with(&format!("{path}:")), "{err}");
        assert!(line.contains("unknown metric `holdrs`"), "{err}");
    }

    #[tokio::test]
    async fn a_name_that_does_not_exist_exits_3_and_lists_the_saved_names() {
        let dir = new_dir(HEALTHY);
        let (out, cap) = Output::capture();
        let code = exec_with(
            &dir,
            "missing",
            &["--once"],
            &factory(),
            &out,
            std::future::pending(),
        )
        .await;
        assert_eq!(code, EXIT_NOT_STARTED);
        assert!(
            cap.err().contains("Saved watchlists: pegs"),
            "{}",
            cap.err()
        );
        assert_eq!(cap.out(), "");
    }

    #[tokio::test]
    async fn a_name_runs_the_saved_list() {
        let dir = new_dir(HEALTHY);
        let (out, cap) = Output::capture();
        let code = exec_with(
            &dir,
            "pegs",
            &["--once"],
            &factory(),
            &out,
            std::future::pending(),
        )
        .await;
        assert_eq!(code, EXIT_PASS, "{}", cap.err());
    }

    // A script must not read a prompt. A symbol on many chains is an error
    // that says where the contract goes (section 5, step 5).
    #[tokio::test]
    async fn a_symbol_on_many_chains_does_not_prompt_and_exits_3() {
        let list = "targets:\n  - kind: token\n    id: USDN\n    rules: [\"holders >= 1\"]\n";
        let dir = new_dir(list);
        let mut f = factory();
        let result = |chain: &str, address: &str| scope::chains::TokenSearchResult {
            address: address.to_string(),
            symbol: "USDN".to_string(),
            name: "Neutrino".to_string(),
            chain: chain.to_string(),
            price_usd: Some(1.0),
            volume_24h: 1_000_000.0,
            liquidity_usd: 5_000_000.0,
            market_cap: None,
        };
        f.mock.mock_dex.search_results = vec![
            result("ethereum", "0x1111111111111111111111111111111111111111"),
            result("bsc", "0x2222222222222222222222222222222222222222"),
        ];
        let (out, cap) = Output::capture();
        let code = exec_with(
            &dir,
            &list_arg(&dir),
            &["--once"],
            &f,
            &out,
            std::future::pending(),
        )
        .await;
        let err = cap.err();
        assert_eq!(code, EXIT_NOT_STARTED, "{err}");
        assert!(err.contains("`id`") && err.contains("USDN"), "{err}");
        assert!(err.contains("ethereum") && err.contains("bsc"), "{err}");
        assert!(!err.contains("Run #"), "{err}");
    }

    #[tokio::test]
    async fn a_csv_path_that_cannot_be_written_exits_4_before_any_tick() {
        let r = exec(HEALTHY, &["--once", "--csv", "{dir}/no-such-folder/t.csv"]).await;
        assert_eq!(r.code, EXIT_WRITE_ERROR, "{}", r.cap.err());
        assert!(!r.cap.err().contains("Run #"), "{}", r.cap.err());
        assert_eq!(r.cap.out(), "");
    }

    // The run was complete and its CSV is whole; the lost report still
    // makes the code 4, because the record of the run is not whole.
    #[tokio::test]
    async fn a_report_that_cannot_be_written_exits_4_after_a_complete_run() {
        let r = exec(
            HEALTHY,
            &["--once", "--csv", "{dir}/t.csv", "--report", "{dir}"],
        )
        .await;
        assert_eq!(r.code, EXIT_WRITE_ERROR, "{}", r.cap.err());
        let csv = std::fs::read_to_string(r.dir.path().join("t.csv")).unwrap();
        assert!(
            csv.lines().count() > 1,
            "header and the cells of a tick: {csv}"
        );
        assert!(r.cap.out().contains("run 1"), "{}", r.cap.out());
    }

    #[tokio::test]
    async fn a_write_error_wins_over_a_breach() {
        let r = exec(STRICT, &["--once", "--report", "{dir}"]).await;
        assert_eq!(r.code, EXIT_WRITE_ERROR, "{}", r.cap.err());
        assert!(r.cap.out().contains("BREACH"), "the breach was seen");
    }

    /// A factory that serves `first` until `per_tick` DEX clients are made,
    /// then `then`: the market changes between two ticks.
    struct Flapping {
        base: OfflineFactory,
        first: MockClientFactory,
        then: MockClientFactory,
        made: AtomicUsize,
        per_tick: usize,
    }

    impl ChainClientFactory for Flapping {
        fn create_chain_client(
            &self,
            chain: &str,
        ) -> scope::error::Result<Box<dyn scope::chains::ChainClient>> {
            self.base.create_chain_client(chain)
        }
        fn create_dex_client(&self) -> Box<dyn scope::chains::DexDataSource> {
            if self.made.fetch_add(1, Ordering::SeqCst) < self.per_tick {
                self.first.create_dex_client()
            } else {
                self.then.create_dex_client()
            }
        }
        fn http(&self) -> Arc<dyn scope::http::HttpClient> {
            self.base.http()
        }
    }

    /// A market with this much liquidity.
    fn market_with(liquidity: f64) -> MockClientFactory {
        let mut f = usdc_on_eth();
        if let Some(td) = f.mock_dex.token_data.as_mut() {
            for pair in &mut td.pairs {
                pair.liquidity_usd = liquidity;
                pair.liquidity_base = Some(liquidity / 2.0);
            }
        }
        f
    }

    const DEPTH_FLOOR: &str = "targets:\n  - kind: pair\n    id: USDC\n    venue: eth\n    preset: market health\n    health:\n      min_depth: 1000.0\n";

    /// Runs two ticks with a market that is `first` then `then`.
    async fn two_ticks(first: f64, then: f64) -> Run {
        // How many DEX clients one tick makes.
        let counting = |first: MockClientFactory, then: MockClientFactory, per_tick| Flapping {
            base: factory(),
            first,
            then,
            made: AtomicUsize::new(0),
            per_tick,
        };
        let probe = counting(market_with(first), market_with(first), usize::MAX);
        let dir = new_dir(DEPTH_FLOOR);
        let (out, _) = Output::capture();
        exec_with(
            &dir,
            &list_arg(&dir),
            &["--once"],
            &probe,
            &out,
            std::future::pending(),
        )
        .await;
        let per_tick = probe.made.load(Ordering::SeqCst);
        assert!(per_tick > 0, "a tick makes a DEX client");

        let flapping = counting(market_with(first), market_with(then), per_tick);
        let (out, cap) = Output::capture();
        let code = exec_with(
            &dir,
            &list_arg(&dir),
            &["--every", "1s", "--duration", "1s", "--format", "json"],
            &flapping,
            &out,
            std::future::pending(),
        )
        .await;
        Run { code, cap, dir }
    }

    /// Followups design 2026-11-11, section 4.3 (test 6) and 4.4: a venue
    /// that never answers must not stop a cron job. The run ends at the read
    /// timeout with exit code 2, and the `tick` record names the read that
    /// timed out. The cron job saves this log and not the `-v` output.
    #[tokio::test(start_paused = true)]
    async fn a_venue_that_never_answers_gives_2_and_the_tick_record_names_the_read() {
        use crate::cli::watch::fetch::tests::{Answer, LATENCY, LatencyFactory, READ};
        use crate::cli::watch::fetch::{MAX_CONCURRENT_READS, READ_TIMEOUT};
        let list = "targets:\n  - kind: pair\n    id: USDC\n    venue: lat-a\n    rules: [\"mid_price > 0\"]\n  - kind: pair\n    id: USDC\n    venue: lat-b\n    rules: [\"mid_price > 0\"]\n";
        let factory =
            LatencyFactory::new(&[("lat-a", Answer::After(LATENCY)), ("lat-b", Answer::Never)]);
        let dir = new_dir(list);
        let (out, cap) = Output::capture();
        let code = exec_with(
            &dir,
            &list_arg(&dir),
            &["--once", "--format", "json"],
            &factory,
            &out,
            std::future::pending(),
        )
        .await;
        assert_eq!(code, EXIT_UNKNOWN);
        let records = json_lines(&cap);
        let ticks = of_type(&records, "tick");
        let timing = &ticks[0]["timing"];
        let timeout_ms = READ_TIMEOUT.as_millis() as u64;
        assert_eq!(timing["wall_ms"], timeout_ms);
        assert_eq!(timing["bound"], MAX_CONCURRENT_READS);
        assert_eq!(
            timing["sum_ms"],
            timeout_ms + READ.as_millis() as u64,
            "the sum is more than the wall time: the reads ran together"
        );
        let reads = timing["reads"].as_array().unwrap();
        let summary: Vec<_> = reads
            .iter()
            .map(|r| {
                (
                    r["row"].as_str(),
                    r["group"].as_str(),
                    r["outcome"].as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (Some("USDC@lat-a"), Some("venue:lat-a"), Some("ok")),
                (Some("USDC@lat-b"), Some("venue:lat-b"), Some("timeout")),
            ]
        );
        assert_eq!(reads[0]["error"], Value::Null);
        assert_eq!(reads[1]["error"], "read timed out after 60s");
        assert_eq!(reads[1]["started_ms"], 0);
        assert_eq!(reads[1]["took_ms"], timeout_ms);
    }

    // Section 8.4: the code is the last tick's. A breach that cleared is 0;
    // a breach that began on the last tick is 1.
    #[tokio::test(start_paused = true)]
    async fn the_code_is_the_code_of_the_last_tick() {
        let cleared = two_ticks(10.0, 50_000_000.0).await;
        let records = json_lines(&cleared.cap);
        let counts: Vec<_> = of_type(&records, "tick")
            .iter()
            .map(|t| t["counts"]["breached"].as_u64())
            .collect();
        assert_eq!(counts, [Some(1), Some(0)], "{records:?}");
        assert_eq!(cleared.code, EXIT_PASS);
        assert!(
            of_type(&records, "event")
                .iter()
                .any(|e| e["event"] == "breach_cleared"),
            "{records:?}"
        );

        let started = two_ticks(50_000_000.0, 10.0).await;
        let records = json_lines(&started.cap);
        let counts: Vec<_> = of_type(&records, "tick")
            .iter()
            .map(|t| t["counts"]["breached"].as_u64())
            .collect();
        assert_eq!(counts, [Some(0), Some(1)], "{records:?}");
        assert_eq!(started.code, EXIT_BREACH);
    }

    // Section 9: one failed venue does not hide the other.
    #[tokio::test]
    async fn one_failed_venue_gives_2_and_the_other_venue_is_still_read() {
        let r = exec(
            TWO_VENUES,
            &["--once", "--format", "json", "--csv", "{dir}/t.csv"],
        )
        .await;
        assert_eq!(r.code, EXIT_UNKNOWN, "{}", r.cap.err());
        let records = json_lines(&r.cap);
        let tick = of_type(&records, "tick")[0];
        let rows: Vec<(&str, &str)> = tick["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["target"].as_str().unwrap(),
                    row["status"].as_str().unwrap(),
                )
            })
            .collect();
        // The mock market warns on depth/volume, which is not a failure.
        assert_eq!(rows[0].0, "USDC@eth");
        assert!(["passed", "warned"].contains(&rows[0].1), "{rows:?}");
        assert_eq!(rows[1], ("USDC@binance", "unknown"));
        // Both venues are in the CSV `target` column.
        let csv = std::fs::read_to_string(r.dir.path().join("t.csv")).unwrap();
        let targets: Vec<String> = csv
            .lines()
            .skip(1)
            .map(|l| csv_fields(l)[1].clone())
            .collect();
        assert!(targets.contains(&"USDC@eth".to_string()), "{csv}");
        assert!(targets.contains(&"USDC@binance".to_string()), "{csv}");
    }

    #[tokio::test]
    async fn a_breach_on_one_venue_wins_over_a_failed_other_venue() {
        let r = exec(TWO_VENUES_STRICT, &["--once"]).await;
        assert_eq!(r.code, EXIT_BREACH, "{}", r.cap.err());
        let out = r.cap.out();
        assert!(
            out.contains("1 breached") && out.contains("1 unknown"),
            "{out}"
        );
        assert!(out.contains("BREACH   USDC@eth:"), "{out}");
        assert!(out.contains("ERROR    USDC@binance:"), "{out}");
    }

    // Section 12, test 11: the two outputs come from one record builder.
    #[tokio::test]
    async fn every_json_line_parses_and_its_cells_equal_the_csv_rows() {
        let r = exec(
            TWO_VENUES_STRICT,
            &["--once", "--format", "json", "--csv", "{dir}/t.csv"],
        )
        .await;
        let records = json_lines(&r.cap);
        let known = ["start", "tick", "event", "end"];
        assert!(
            records
                .iter()
                .all(|x| known.contains(&x["type"].as_str().unwrap_or(""))),
            "{records:?}"
        );
        assert_eq!(records[0]["type"], "start");
        assert_eq!(records.last().unwrap()["type"], "end");
        let tick = of_type(&records, "tick")[0];
        let csv = std::fs::read_to_string(r.dir.path().join("t.csv")).unwrap();
        let mut lines = csv.lines().skip(1);
        let number = |v: &Value| v.as_f64().map(|f| f.to_string()).unwrap_or_default();
        let text = |v: &Value| v.as_str().unwrap_or("").to_string();
        let mut cells = 0;
        for row in tick["rows"].as_array().unwrap() {
            for cell in row["cells"].as_array().unwrap() {
                let f = csv_fields(lines.next().expect("a CSV row for the cell"));
                let expected = [
                    text(&tick["ts"]),
                    text(&row["target"]),
                    text(&cell["metric"]),
                    number(&cell["value"]),
                    text(&cell["op"]),
                    number(&cell["threshold"]),
                    text(&cell["status"]),
                    text(&cell["source"]),
                    text(&cell["error"]),
                    text(&cell["detail"]),
                ];
                assert_eq!(f, expected);
                cells += 1;
            }
        }
        assert!(cells >= 2, "{cells}");
        assert!(lines.next().is_none(), "no extra CSV rows");
        // The exact error text is kept, not a summary.
        assert!(csv.contains("error"), "{csv}");
    }

    #[tokio::test]
    async fn the_csv_format_on_stdout_is_the_bytes_of_the_csv_file() {
        let r = exec(
            TWO_VENUES_STRICT,
            &["--once", "--format", "csv", "--csv", "{dir}/t.csv"],
        )
        .await;
        let file = std::fs::read_to_string(r.dir.path().join("t.csv")).unwrap();
        assert_eq!(r.cap.out(), file);
        assert!(file.starts_with(csv_header()));
    }

    // Section 6.1: the first tick compares with an empty snapshot, and an
    // Error cell is not an event but it is on every tick.
    #[tokio::test(start_paused = true)]
    async fn the_text_format_prints_breach_on_the_first_tick_and_error_on_every_tick() {
        let r = exec(TWO_VENUES_STRICT, &["--every", "1s", "--duration", "1s"]).await;
        let out = r.cap.out();
        let lines: Vec<&str> = out.lines().collect();
        let first_run = lines.iter().position(|l| l.contains(" run 1 ")).unwrap();
        let second_run = lines.iter().position(|l| l.contains(" run 2 ")).unwrap();
        assert!(
            lines[first_run + 1..second_run]
                .iter()
                .any(|l| l.contains("BREACH")),
            "{out}"
        );
        assert!(
            !lines[second_run + 1..].iter().any(|l| l.contains("BREACH")),
            "the breach is not new on tick 2: {out}"
        );
        let errors_after = |from: usize, to: usize| {
            lines[from..to]
                .iter()
                .filter(|l| l.contains("ERROR"))
                .count()
        };
        assert!(errors_after(first_run, second_run) >= 1, "{out}");
        assert!(errors_after(second_run, lines.len()) >= 1, "{out}");
    }

    // Section 12, test 14: a script reads data on stdout and notes on stderr.
    #[tokio::test]
    async fn stdout_holds_only_data_and_the_notes_are_on_stderr() {
        let r = exec(HEALTHY, &["--once", "--csv", "{dir}/t.csv"]).await;
        let (out, err) = (r.cap.out(), r.cap.err());
        for line in out.lines() {
            // A summary line, or an event line (two spaces, then its label).
            assert!(
                line.contains(" run ") || line.starts_with("  "),
                "not a tick line: {line}"
            );
        }
        assert!(err.contains("Watching pegs"), "{err}");
        assert!(err.contains("--- Run #1"), "{err}");
        assert!(err.contains("Run ended: once"), "{err}");
        // The path in the note is the full path, for a cron job.
        assert!(err.contains(&r.dir.path().display().to_string()), "{err}");
    }

    #[tokio::test]
    async fn append_keeps_the_old_rows_and_a_run_without_it_truncates() {
        let dir = new_dir(HEALTHY);
        let trail = dir.path().join("t.csv");
        let old = format!(
            "{}2026-01-01T00:00:00Z,old,row,,,,pass,preset,,\n",
            csv_header()
        );
        let tick_rows = |text: &str| text.lines().filter(|l| l.contains("USDC@eth")).count();

        std::fs::write(&trail, &old).unwrap();
        let (out, _) = Output::capture();
        let flags = ["--once", "--csv", "{dir}/t.csv", "--append"];
        exec_with(
            &dir,
            &list_arg(&dir),
            &flags,
            &factory(),
            &out,
            std::future::pending(),
        )
        .await;
        let text = std::fs::read_to_string(&trail).unwrap();
        assert!(text.starts_with(&old), "old rows kept: {text}");
        assert_eq!(text.matches("ts,target").count(), 1, "one header: {text}");
        assert!(tick_rows(&text) >= 1, "{text}");

        // Appending to a file that is not there starts it with a header.
        std::fs::remove_file(&trail).unwrap();
        exec_with(
            &dir,
            &list_arg(&dir),
            &flags,
            &factory(),
            &out,
            std::future::pending(),
        )
        .await;
        assert!(
            std::fs::read_to_string(&trail)
                .unwrap()
                .starts_with(csv_header())
        );

        std::fs::write(&trail, &old).unwrap();
        let flags = ["--once", "--csv", "{dir}/t.csv"];
        exec_with(
            &dir,
            &list_arg(&dir),
            &flags,
            &factory(),
            &out,
            std::future::pending(),
        )
        .await;
        let text = std::fs::read_to_string(&trail).unwrap();
        assert!(!text.contains(",old,row,"), "truncated: {text}");
    }

    #[tokio::test]
    async fn once_makes_one_tick_and_conflicts_with_duration() {
        let r = exec(HEALTHY, &["--once", "--format", "json"]).await;
        let records = json_lines(&r.cap);
        assert_eq!(of_type(&records, "tick").len(), 1);
        let end = of_type(&records, "end")[0];
        assert_eq!(
            (end["reason"].as_str(), end["runs"].as_u64()),
            (Some("once"), Some(1))
        );
        assert_eq!(end["exit_code"], 0);
        let argv = ["scope", "watch", "pegs", "--once", "--duration", "1h"];
        let e = Cli::try_parse_from(argv).unwrap_err();
        assert_eq!(e.kind(), clap::error::ErrorKind::ArgumentConflict);
        // Q7: --every with --once says how far apart the runs are.
        let argv = ["scope", "watch", "pegs", "--once", "--every", "5m"];
        assert!(Cli::try_parse_from(argv).is_ok());
    }

    // The repeat rule: a run of 2s every 1s ends with a tick at 2s.
    #[tokio::test(start_paused = true)]
    async fn a_duration_makes_a_tick_at_its_end() {
        let r = exec(
            HEALTHY,
            &["--every", "1s", "--duration", "2s", "--format", "json"],
        )
        .await;
        let records = json_lines(&r.cap);
        assert_eq!(of_type(&records, "tick").len(), 3, "{records:?}");
        assert_eq!(of_type(&records, "end")[0]["reason"], "duration");
        assert_eq!(of_type(&records, "start")[0]["duration_secs"], 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_with_no_duration_says_so_and_stops_on_the_stop_future() {
        let dir = new_dir(HEALTHY);
        let (out, cap) = Output::capture();
        let probe = cap.clone();
        let stop = async move {
            while probe.out().matches("\"type\":\"tick\"").count() < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        let code = exec_with(
            &dir,
            &list_arg(&dir),
            &[
                "--every",
                "1s",
                "--duration",
                "forever",
                "--format",
                "json",
                "--report",
                "{dir}/r.md",
            ],
            &factory(),
            &out,
            stop,
        )
        .await;
        assert_eq!(code, EXIT_PASS, "the code of tick 2");
        assert!(cap.err().contains(NO_DURATION_NOTE), "{}", cap.err());
        let records = json_lines(&cap);
        assert_eq!(of_type(&records, "tick").len(), 2);
        let end = of_type(&records, "end")[0];
        assert_eq!(
            (end["reason"].as_str(), end["runs"].as_u64()),
            (Some("signal"), Some(2))
        );
        assert_eq!(end["report_written"], true);
        let report = std::fs::read_to_string(dir.path().join("r.md")).unwrap();
        assert!(
            report.contains("| USDC@eth |"),
            "the report holds tick 2: {report}"
        );
    }

    // Question 4: a stop before the first tick has no verdict. A script
    // must not read it as a pass.
    #[tokio::test]
    async fn a_stop_before_the_first_tick_exits_2_and_still_writes_a_report_with_no_rows() {
        let dir = new_dir(HEALTHY);
        let (out, cap) = Output::capture();
        let code = exec_with(
            &dir,
            &list_arg(&dir),
            &["--once", "--format", "json", "--report", "{dir}/r.md"],
            &factory(),
            &out,
            std::future::ready(()),
        )
        .await;
        assert_eq!(code, EXIT_UNKNOWN);
        let records = json_lines(&cap);
        assert!(of_type(&records, "tick").is_empty(), "{records:?}");
        let end = of_type(&records, "end")[0];
        assert_eq!(
            (end["reason"].as_str(), end["runs"].as_u64()),
            (Some("signal"), Some(0))
        );
        assert_eq!(end["exit_code"], 2);
        let report = std::fs::read_to_string(dir.path().join("r.md")).unwrap();
        assert!(report.contains("## Last values"), "{report}");
        assert!(!report.contains("USDC@eth"), "no rows: {report}");
    }

    /// A stdout that a reader has closed, and a stderr we can read.
    struct ClosedPipe;

    impl std::io::Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedBuf {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    // A reader that quits (`| head -1`) must not leave a run that never ends.
    #[tokio::test(start_paused = true)]
    async fn a_closed_stdout_stops_the_run_and_writes_the_report() {
        let dir = new_dir(HEALTHY);
        let err = SharedBuf::default();
        let out = Output::from_writers(ClosedPipe, err.clone());
        let code = exec_with(
            &dir,
            &list_arg(&dir),
            &[
                "--every",
                "1s",
                "--duration",
                "forever",
                "--report",
                "{dir}/r.md",
            ],
            &factory(),
            &out,
            std::future::pending(),
        )
        .await;
        let err = String::from_utf8(err.0.lock().unwrap().clone()).unwrap();
        assert_eq!(code, EXIT_PASS, "the usual code: {err}");
        assert!(err.contains("Cannot write to stdout"), "{err}");
        assert!(
            err.contains("Run ended: stdout_closed after 1 run(s)"),
            "{err}"
        );
        assert!(dir.path().join("r.md").exists());
    }

    // Rule 13: after a night run in text mode, `-v` has the record of each
    // row of each tick: which row, its status and the exact error.
    #[derive(Clone, Default)]
    struct Logs(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Logs {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
        type Writer = Logs;
        fn make_writer(&'a self) -> Logs {
            self.clone()
        }
    }

    /// The log of the whole test process. The subscriber is global, because
    /// a subscriber for one thread loses events: tracing caches "disabled"
    /// for a call site that an earlier test reached with no subscriber. The
    /// test reads its own lines by a name that no other test uses.
    fn process_logs() -> Logs {
        static LOGS: std::sync::OnceLock<Logs> = std::sync::OnceLock::new();
        LOGS.get_or_init(|| {
            let logs = Logs::default();
            let subscriber = tracing_subscriber::fmt()
                .with_writer(logs.clone())
                .with_ansi(false)
                .with_max_level(tracing::Level::INFO)
                .finish();
            tracing::subscriber::set_global_default(subscriber)
                .expect("no other test sets a global subscriber");
            logs
        })
        .clone()
    }

    // Rule 13: after a night run in text mode, `-v` has the record of each
    // row of each tick: which row, its status and the exact error.
    #[tokio::test]
    async fn each_tick_leaves_a_structured_trace_of_every_row_and_its_error() {
        let logs = process_logs();
        let list = "name: tracewatch\ntargets:\n  - kind: pair\n    id: TRCE\n    venue: binance\n    preset: market health\n";
        let r = exec(list, &["--once"]).await;
        assert_eq!(r.code, EXIT_UNKNOWN);
        let log = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        let mine: Vec<&str> = log
            .lines()
            .filter(|l| l.contains("tracewatch") || l.contains("TRCE@binance"))
            .collect();
        let has = |text: &str| {
            mine.iter()
                .find(|l| l.contains(text))
                .copied()
                .unwrap_or("")
        };
        assert!(
            has("watch run started").contains("rows=[\"TRCE@binance\"]"),
            "{mine:?}"
        );
        let row = has("watch tick row");
        assert!(
            row.contains("row=TRCE@binance") && row.contains("status=\"unknown\""),
            "{row}"
        );
        // The exact error text is in the record, not a summary.
        assert!(row.contains("errors=[\""), "{row}");
        let end = has("watch run ended");
        assert!(
            end.contains("name=tracewatch") && end.contains("exit_code=2"),
            "{end}"
        );
    }

    #[test]
    fn help_lists_the_five_exit_codes_and_the_once_example() {
        let e = Cli::try_parse_from(["scope", "watch", "--help"]).unwrap_err();
        let help = e.to_string();
        assert!(help.contains("scope watch pegs --once"), "{help}");
        for code in ["0  ", "1  ", "2  ", "3  ", "4  "] {
            assert!(help.contains(code), "{code}: {help}");
        }
        assert!(help.contains("Exit codes"), "{help}");
    }
}
