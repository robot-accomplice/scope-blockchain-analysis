//! # Command Output
//!
//! The writers that command handlers print through. The CLI binds them to
//! stdout and stderr. The TUI and the tests bind them to in-memory buffers,
//! so the same handler can run in both places without printing over the
//! terminal UI.
//!
//! ## The one policy
//!
//! | What | Macro | Channel | A failed write |
//! |---|---|---|---|
//! | Data (the command's result) | `outln!`, `out!` | out | is the command's error |
//! | Error reports, only in `errors.rs` and `tui/exec.rs` (a TUI session command's error) | `errln!` | err | is the command's error |
//! | A prompt's question, only in `prompt.rs` | `err!` | err | is the prompt's error |
//! | The macros above, defined in `output.rs` | `write_err` | err | is returned |
//! | Everything else: status, notices, warnings, "saved to" | [`note!`](crate::note), [`Output::note_saved`] | err | is dropped |
//!
//! Data goes to `out`, so piped JSON and CSV stay clean. A note ("Fetching
//! …", "Selected: …", "⚠ … unavailable") is best effort: when it cannot be
//! written, it must not take the place of the command's own result, and
//! it does not change the exit code. Library code in `scope` (core) has the
//! same kind of best-effort line: `scope::diag::notice`.
//!
//! The guard tests (`tests/one_output_path.rs`) keep `errln!`, `err!` and
//! `write_err` in the files this table names (they read the table), and
//! status text off `outln!` and `out!`.
//!
//! ```rust,ignore
//! use scope_cli::cli::output::Output;
//! use scope_cli::{note, outln};
//!
//! let out = Output::stdio();
//! outln!(out, "Balance: {}", 42)?;    // data → stdout
//! note!(out, "Fetching balances..."); // status → stderr
//! ```

use scope::metrics::{Metric, Sample};
use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A shared, thread-safe writer.
type Sink = Arc<Mutex<dyn Write + Send>>;

/// Where a repeating command is, for a screen that draws it.
///
/// It is not output. It never reaches the pane text, the copy or the
/// journal. Only the command writes it; the TUI reads it.
#[derive(Debug, Clone, Default)]
pub struct RunProgress {
    /// The run that is in progress or just ended. 0 before the first.
    pub run: u64,
    /// How many runs the command will make, when it is known.
    pub total: Option<u64>,
    /// When the next run starts. None while a run is in progress.
    pub next_at: Option<Instant>,
}

/// A progress record shared between a command and the screen.
pub type ProgressHook = Arc<Mutex<RunProgress>>;

/// One run of a repeating command, for a screen that charts it (#58).
#[derive(Debug, Clone)]
pub struct RunRecord {
    /// The run number, from 1.
    pub run: u64,
    /// When the run ended.
    pub at: chrono::DateTime<chrono::Utc>,
    /// The run's key values, in the order of [`Series::metrics`].
    pub sample: Sample,
    /// The report the run printed in the CLI.
    pub snapshot: String,
    /// The run's data, for a screen that draws its own view of it.
    pub view: Option<Arc<SummaryView>>,
}

/// A market summary, for a screen that draws it (the TUI's compact view).
#[derive(Debug, Clone)]
pub struct SummaryView {
    pub summary: scope::market::MarketSummary,
    /// The venue the summary is for.
    pub venue: String,
}

/// The view a pane command left for the screen (one-shot `market summary`).
pub type ViewHook = Arc<Mutex<Option<SummaryView>>>;

/// The runs a screen keeps, oldest first, and the metric table they read in.
#[derive(Debug, Default)]
pub struct Series {
    /// The metric table of every run's sample.
    pub metrics: Vec<Metric>,
    /// The newest [`MAX_SERIES_RUNS`] runs.
    pub runs: VecDeque<RunRecord>,
    /// How many older runs the ring dropped.
    pub dropped: u64,
}

/// The series shared between a repeating command and the screen.
pub type SeriesHook = Arc<Mutex<Series>>;

/// A question a command asks the TUI: the TUI shows `options` in a list
/// and replies with the chosen index, or `None` when the user cancels.
pub struct Ask {
    pub prompt: String,
    pub options: Vec<String>,
    pub reply: tokio::sync::oneshot::Sender<Option<usize>>,
}

/// The slot through which a pane command asks the TUI one question at a
/// time.
pub type Asker = Arc<Mutex<Option<Ask>>>;

/// The most runs a series keeps. Older runs are dropped.
pub const MAX_SERIES_RUNS: usize = 500;

/// The output channels for one command run.
///
/// Cloning is cheap: clones share the same writers.
#[derive(Clone)]
pub struct Output {
    out: Sink,
    err: Sink,
    progress: bool,
    hook: Option<ProgressHook>,
    series: Option<SeriesHook>,
    tui: bool,
    /// False for a server: no command may wait for an answer on stdin.
    prompts: bool,
    /// The TUI's ask slot, when the TUI pane shows this output.
    asker: Option<Asker>,
    /// The view slot, when a screen draws the command's data itself.
    view: Option<ViewHook>,
}

impl Output {
    /// Output bound to the process stdout and stderr, with progress
    /// indicators enabled (they still hide themselves when stderr is not a TTY).
    pub fn stdio() -> Self {
        Self {
            out: Arc::new(Mutex::new(io::stdout())),
            err: Arc::new(Mutex::new(io::stderr())),
            progress: true,
            hook: None,
            series: None,
            tui: false,
            prompts: true,
            asker: None,
            view: None,
        }
    }

    /// Output bound to in-memory buffers, with progress indicators and
    /// prompts disabled (nobody can answer a buffer).
    ///
    /// Returns the output and a handle that reads what was written.
    pub fn capture() -> (Self, Captured) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let err = Arc::new(Mutex::new(Vec::new()));
        let captured = Captured {
            data: out.clone(),
            out: out.clone(),
            err: err.clone(),
            trimmed: Arc::default(),
        };
        let output = Self {
            out,
            err,
            progress: false,
            hook: None,
            series: None,
            tui: false,
            prompts: false,
            asker: None,
            view: None,
        };
        (output, captured)
    }

    /// Output whose two channels write to one in-memory buffer, in the order
    /// of the writes, with progress indicators disabled.
    ///
    /// The TUI uses it: its output pane shows data and diagnostics as one
    /// stream, as a terminal does. [`Captured::out`] returns the whole
    /// stream and [`Captured::err`] returns the same text.
    /// [`Captured::data`] returns only what went to the data channel.
    pub fn capture_merged() -> (Self, Captured) {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let data = Arc::new(Mutex::new(Vec::new()));
        let captured = Captured {
            out: buf.clone(),
            err: buf.clone(),
            data: data.clone(),
            trimmed: Arc::default(),
        };
        let output = Self {
            out: Arc::new(Mutex::new(Tee {
                merged: buf.clone(),
                data,
            })),
            err: buf,
            progress: false,
            hook: Some(ProgressHook::default()),
            series: Some(SeriesHook::default()),
            tui: true,
            prompts: false,
            asker: None,
            view: Some(ViewHook::default()),
        };
        (output, captured)
    }

    /// Output that still writes to this output's writers and also keeps a
    /// copy of the text, as [`Output::capture_merged`] does.
    ///
    /// The TUI uses it for a command that runs outside it: the user sees
    /// the text live on the terminal, and the TUI shows the copy in its
    /// scrollback after it returns. Progress indicators stay as they are
    /// here (they draw on the real terminal and are not copied). The copy
    /// keeps at most about `limit` bytes, the end of the text; whole lines
    /// are dropped from its start. [`Captured::trimmed`] says when that
    /// happened.
    ///
    /// The result is a TUI output ([`Output::is_tui`]): its hints are read
    /// in the TUI scrollback.
    pub fn teed(&self, limit: usize) -> (Self, Captured) {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let data = Arc::new(Mutex::new(Vec::new()));
        let trimmed = Arc::new(AtomicBool::new(false));
        let captured = Captured {
            out: buf.clone(),
            err: buf.clone(),
            data: data.clone(),
            trimmed: trimmed.clone(),
        };
        let copy = |real: &Sink, data: Option<Arc<Mutex<Vec<u8>>>>| -> Sink {
            Arc::new(Mutex::new(TeeSink {
                real: real.clone(),
                merged: buf.clone(),
                data,
                limit,
                trimmed: trimmed.clone(),
            }))
        };
        let output = Self {
            out: copy(&self.out, Some(data.clone())),
            err: copy(&self.err, None),
            progress: self.progress,
            hook: None,
            series: None,
            tui: true,
            prompts: self.prompts,
            asker: None,
            view: None,
        };
        (output, captured)
    }

    /// Output bound to the given writers, with progress indicators disabled.
    #[cfg(test)]
    pub(crate) fn from_writers(
        out: impl Write + Send + 'static,
        err: impl Write + Send + 'static,
    ) -> Self {
        Self {
            out: Arc::new(Mutex::new(out)),
            err: Arc::new(Mutex::new(err)),
            progress: false,
            hook: None,
            series: None,
            tui: false,
            prompts: true,
            asker: None,
            view: None,
        }
    }

    /// True when animated progress indicators may draw on the terminal.
    pub fn progress_enabled(&self) -> bool {
        self.progress
    }

    /// This output for a server: a question that needs an answer (for
    /// example which token a symbol means) is an error, never a prompt.
    pub fn without_prompts(mut self) -> Self {
        self.prompts = false;
        self
    }

    /// This output with a slot through which a command asks the TUI a
    /// question (the pane's list).
    pub fn with_asker(mut self, asker: Asker) -> Self {
        self.asker = Some(asker);
        self
    }

    /// The ask slot, when a TUI shows this output.
    pub fn asker(&self) -> Option<Asker> {
        self.asker.clone()
    }

    /// True when a command may ask the user on the terminal.
    pub fn prompts_allowed(&self) -> bool {
        self.prompts
    }

    /// True when this output belongs to the TUI. Error text then gives TUI
    /// next steps, not CLI flags.
    pub fn is_tui(&self) -> bool {
        self.tui
    }

    /// The progress record of this run, when the output has one. The TUI's
    /// output has one; the plain CLI output does not.
    pub fn progress_hook(&self) -> Option<ProgressHook> {
        self.hook.clone()
    }

    /// The series of a repeating run, when the output has one: the TUI's
    /// pane output has one; the plain CLI output does not.
    pub fn series_hook(&self) -> Option<SeriesHook> {
        self.series.clone()
    }

    /// The view slot, when a screen draws the command's data (the TUI).
    pub fn view_hook(&self) -> Option<ViewHook> {
        self.view.clone()
    }

    /// Hands the command's data to the screen. Returns false when no screen
    /// draws it (the CLI); the printed report is then all there is.
    pub fn record_view(&self, view: SummaryView) -> bool {
        let Some(slot) = &self.view else {
            return false;
        };
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(view);
        true
    }

    /// Hands one run to the screen's series. Returns false when there is no
    /// series (the CLI), so the caller prints the run instead.
    pub fn record_run(&self, metrics: &[Metric], record: RunRecord) -> bool {
        let Some(series) = &self.series else {
            return false;
        };
        let mut series = lock(series);
        if series.metrics.is_empty() {
            series.metrics = metrics.to_vec();
        }
        series.runs.push_back(record);
        while series.runs.len() > MAX_SERIES_RUNS {
            series.runs.pop_front();
            series.dropped += 1;
        }
        true
    }

    /// Changes the progress record. Does nothing when there is no hook.
    pub fn update_progress(&self, change: impl FnOnce(&mut RunProgress)) {
        if let Some(hook) = &self.hook {
            change(&mut lock(hook));
        }
    }

    /// Writes formatted data to the data channel. Use [`outln!`](crate::outln) / [`out!`](crate::out).
    ///
    /// # Errors
    ///
    /// Returns the writer's error, for example `BrokenPipe` when a reader
    /// closes the pipe early or a storage error when stdout is a full disk.
    pub fn write_out(&self, args: fmt::Arguments<'_>) -> io::Result<()> {
        emit(&self.out, args)
    }

    /// Writes formatted text to the diagnostic channel. Use [`errln!`](crate::errln) / [`err!`](crate::err).
    ///
    /// # Errors
    ///
    /// Returns the writer's error.
    pub fn write_err(&self, args: fmt::Arguments<'_>) -> io::Result<()> {
        emit(&self.err, args)
    }

    /// Writes a status line to the diagnostic channel. Use
    /// [`note!`](crate::note). A failed write is dropped: see the module
    /// policy.
    pub fn note(&self, args: fmt::Arguments<'_>) {
        let _ = emit(&self.err, args);
    }

    /// The one "saved to" line: `<what> saved to: <where>`, as a note.
    pub fn note_saved(&self, what: &str, path: impl fmt::Display) {
        crate::note!(self, "{what} saved to: {path}");
    }
}

/// Reads back what an [`Output::capture`] output received.
#[derive(Clone)]
pub struct Captured {
    /// Only the data channel. Same buffer as `out` unless merged.
    data: Arc<Mutex<Vec<u8>>>,
    out: Arc<Mutex<Vec<u8>>>,
    err: Arc<Mutex<Vec<u8>>>,
    /// Set when a bounded copy ([`Output::teed`]) dropped text.
    trimmed: Arc<AtomicBool>,
}

impl Captured {
    /// True when the copy dropped the start of the text to stay in its
    /// limit. Only an [`Output::teed`] copy has a limit.
    pub fn trimmed(&self) -> bool {
        self.trimmed.load(Ordering::Relaxed)
    }

    /// Everything written to the data channel so far.
    pub fn out(&self) -> String {
        String::from_utf8_lossy(&lock(&self.out)).into_owned()
    }

    /// Only what was written to the data channel, without the diagnostics
    /// that a merged capture mixes in.
    pub fn data(&self) -> String {
        String::from_utf8_lossy(&lock(&self.data)).into_owned()
    }

    /// Everything written to the diagnostic channel so far.
    pub fn err(&self) -> String {
        String::from_utf8_lossy(&lock(&self.err)).into_owned()
    }

    /// The buffer behind the data channel, as a writer. The TUI gives it to
    /// [`scope::diag::redirect`] so library warnings join the same stream.
    pub fn out_sink(&self) -> scope::diag::Sink {
        self.out.clone()
    }
}

/// The data channel of a merged capture: writes to the merged stream and
/// to a data-only buffer.
struct Tee {
    merged: Arc<Mutex<Vec<u8>>>,
    data: Arc<Mutex<Vec<u8>>>,
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        lock(&self.merged).extend_from_slice(buf);
        lock(&self.data).extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// One channel of an [`Output::teed`] output: keeps a bounded copy, then
/// writes to the real writer.
struct TeeSink {
    real: Sink,
    merged: Arc<Mutex<Vec<u8>>>,
    /// The data-only copy; `None` on the diagnostic channel.
    data: Option<Arc<Mutex<Vec<u8>>>>,
    limit: usize,
    trimmed: Arc<AtomicBool>,
}

impl Write for TeeSink {
    /// Copies first, so the TUI still shows text that the terminal could
    /// not take; then returns the real writer's result.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for copy in std::iter::once(&self.merged).chain(self.data.as_ref()) {
            let mut copy = lock(copy);
            copy.extend_from_slice(buf);
            if keep_tail(&mut copy, self.limit) {
                self.trimmed.store(true, Ordering::Relaxed);
            }
        }
        lock(&self.real).write_all(buf)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        lock(&self.real).flush()
    }
}

/// Drops text from the start of `buf` when it is longer than `limit`,
/// down to about half of it, so a long run trims rarely. It cuts after a
/// newline in the half it keeps; with no newline there, it keeps the end
/// of the line, cut where no character or escape sequence is split.
/// Returns true when it dropped text.
fn keep_tail(buf: &mut Vec<u8>, limit: usize) -> bool {
    if buf.len() <= limit {
        return false;
    }
    let cut = buf.len() - limit / 2;
    let start = buf[cut..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|i| cut + i + 1)
        .filter(|&start| start < buf.len())
        .unwrap_or_else(|| whole_cut(buf, cut));
    buf.drain(..start);
    true
}

/// The first index at or after `cut` that is inside neither an escape
/// sequence ([`crate::cli::ansi`]) nor a UTF-8 character. It walks from
/// the start, so it knows where each sequence begins.
fn whole_cut(buf: &[u8], cut: usize) -> usize {
    let mut i = 0;
    while i < cut {
        i = crate::cli::ansi::escape_end(buf, i).unwrap_or(i + 1);
    }
    let continuation = |b: u8| b & 0b1100_0000 == 0b1000_0000;
    buf[i.min(buf.len())..]
        .iter()
        .position(|&b| !continuation(b))
        .map_or(buf.len(), |n| i + n)
}

/// Locks a mutex, recovering the data if a panicking writer poisoned it.
/// Output must keep working after an unrelated panic on another thread.
fn lock<T: ?Sized>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Writes one formatted message to a sink and returns the write error.
/// Data and error reports pass it on, so a command whose output did not
/// arrive (closed pipe, full disk) does not report success; a note drops
/// it (see the module policy).
fn emit(sink: &Mutex<dyn Write + Send>, args: fmt::Arguments<'_>) -> io::Result<()> {
    lock(sink).write_fmt(args)
}

/// Prints to the data channel of an [`Output`], with a newline.
#[macro_export]
macro_rules! outln {
    ($o:expr) => {
        $o.write_out(format_args!("\n"))
    };
    ($o:expr, $($arg:tt)*) => {
        $o.write_out(format_args!("{}\n", format_args!($($arg)*)))
    };
}

/// Prints to the data channel of an [`Output`], without a newline.
#[macro_export]
macro_rules! out {
    ($o:expr, $($arg:tt)*) => {
        $o.write_out(format_args!($($arg)*))
    };
}

/// Prints to the diagnostic channel of an [`Output`], with a newline.
#[macro_export]
macro_rules! errln {
    ($o:expr) => {
        $o.write_err(format_args!("\n"))
    };
    ($o:expr, $($arg:tt)*) => {
        $o.write_err(format_args!("{}\n", format_args!($($arg)*)))
    };
}

/// Writes a status line (a note) to the diagnostic channel of an
/// [`Output`], with a newline. Best effort: it returns nothing.
#[macro_export]
macro_rules! note {
    ($o:expr) => {
        $o.note(format_args!("\n"))
    };
    ($o:expr, $($arg:tt)*) => {
        $o.note(format_args!("{}\n", format_args!($($arg)*)))
    };
}

/// Prints to the diagnostic channel of an [`Output`], without a newline.
#[macro_export]
macro_rules! err {
    ($o:expr, $($arg:tt)*) => {
        $o.write_err(format_args!($($arg)*))
    };
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn test_capture_separates_data_from_diagnostics() {
        // Piped JSON must not contain warnings: the two channels stay apart.
        let (o, cap) = Output::capture();
        outln!(o, "{{\"ok\":{}}}", true).unwrap();
        errln!(o, "Warning: {}", "partial data").unwrap();
        assert_eq!(cap.out(), "{\"ok\":true}\n");
        assert_eq!(cap.err(), "Warning: partial data\n");
    }

    #[test]
    fn test_a_note_goes_to_the_diagnostic_channel_and_never_fails() {
        // A status line must not end a command whose real work went well,
        // and must stay out of data.
        let (o, cap) = Output::capture();
        note!(o, "Fetching {}...", "USDC");
        assert_eq!(cap.err(), "Fetching USDC...\n");
        assert_eq!(cap.out(), "");
        let closed = Output::from_writers(Vec::new(), ClosedPipe);
        note!(closed, "dropped");
    }

    #[test]
    fn test_note_saved_is_one_line_on_the_diagnostic_channel() {
        let (o, cap) = Output::capture();
        o.note_saved("Report", "r.md");
        assert_eq!(cap.err(), "Report saved to: r.md\n");
        assert_eq!(cap.out(), "");
    }

    #[test]
    fn test_clones_share_writers() {
        // Helpers receive clones; their output must land in the same buffer.
        let (o, cap) = Output::capture();
        let o2 = o.clone();
        out!(o, "a").unwrap();
        out!(o2, "b").unwrap();
        outln!(o).unwrap();
        assert_eq!(cap.out(), "ab\n");
    }

    /// A writer that fails like stdout does after `| head -1` closes the pipe.
    pub(crate) struct ClosedPipe;

    impl Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_merged_capture_keeps_data_apart_from_diagnostics() {
        let (o, cap) = Output::capture_merged();
        outln!(o, "data").unwrap();
        errln!(o, "warn").unwrap();
        assert_eq!(cap.data(), "data\n");
        assert_eq!(cap.out(), "data\nwarn\n");
    }

    #[test]
    fn test_failed_write_is_returned_not_dropped() {
        // A command whose output did not arrive must not report success.
        let o = Output::from_writers(ClosedPipe, ClosedPipe);
        let e = outln!(o, "data").unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(
            errln!(o, "warn").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn test_only_the_tui_capture_is_tui() {
        assert!(Output::capture_merged().0.is_tui());
        assert!(!Output::capture().0.is_tui());
        assert!(!Output::stdio().is_tui());
        assert!(Output::stdio().teed(1).0.is_tui());
    }

    #[test]
    fn test_capture_merged_keeps_write_order() {
        // The TUI pane must show a warning where it happened, between the
        // data lines, not after all of them.
        let (o, cap) = Output::capture_merged();
        outln!(o, "row 1").unwrap();
        errln!(o, "warning").unwrap();
        outln!(o, "row 2").unwrap();
        assert_eq!(cap.out(), "row 1\nwarning\nrow 2\n");
    }

    #[test]
    fn test_teed_output_reaches_the_terminal_and_keeps_a_copy() {
        // Outside the TUI the user reads the text live; the TUI shows the
        // copy when it returns, and Ctrl-Y gets only the data.
        let (terminal, screen) = Output::capture();
        let (o, copy) = terminal.teed(1024);
        outln!(o, "row").unwrap();
        errln!(o, "warn").unwrap();
        assert_eq!(screen.out(), "row\n");
        assert_eq!(screen.err(), "warn\n");
        assert_eq!(copy.out(), "row\nwarn\n");
        assert_eq!(copy.data(), "row\n");
        assert!(!copy.trimmed());
        // The copy is read in the TUI: TUI wording for hints. Progress
        // bars still draw on the real terminal as before.
        assert!(o.is_tui());
        assert_eq!(o.progress_enabled(), terminal.progress_enabled());
        assert!(Output::stdio().teed(1).0.progress_enabled());
        assert!(o.progress_hook().is_none());
    }

    #[test]
    fn test_teed_copy_keeps_whole_lines_from_the_end_within_its_limit() {
        // A long crawl must not grow the copy without bound.
        let (terminal, screen) = Output::capture();
        let (o, copy) = terminal.teed(100);
        for i in 0..1000 {
            outln!(o, "line {i:04}").unwrap();
        }
        assert!(copy.trimmed());
        let kept = copy.out();
        assert!(kept.len() <= 100, "{}", kept.len());
        assert!(kept.starts_with("line "), "whole lines: {kept:?}");
        assert!(kept.ends_with("line 0999\n"), "{kept:?}");
        assert!(copy.data().ends_with("line 0999\n"));
        assert_eq!(screen.out().lines().count(), 1000, "the terminal got all");
    }

    #[test]
    fn test_teed_copy_keeps_the_tail_of_one_long_line() {
        // A progress line or a JSON blob can be one write with no newline
        // in its end half. The copy must keep its end, not drop it all.
        let (terminal, _screen) = Output::capture();
        let (o, copy) = terminal.teed(100);
        let blob: String = (0..300)
            .map(|i| char::from(b'a' + (i % 26) as u8))
            .collect();
        out!(o, "{blob}").unwrap();
        assert!(copy.trimmed());
        let kept = copy.out();
        assert!(!kept.is_empty() && kept.len() <= 100, "{}", kept.len());
        assert!(blob.ends_with(&kept), "the end of the line: {kept:?}");
    }

    #[test]
    fn test_trim_of_one_line_splits_no_character_and_no_escape() {
        // The cut lands in the middle of a two-byte character, then in
        // the middle of a color sequence. Either would show as garbage.
        let mut chars = "é".repeat(60).into_bytes(); // 120 bytes
        assert!(keep_tail(&mut chars, 102)); // cuts at 120 - 51 = 69, inside an é
        assert!(String::from_utf8(chars.clone()).is_ok(), "{chars:?}");
        assert_eq!(chars.len(), 50);
        let mut colored = b"x".repeat(60);
        colored.extend_from_slice(b"\x1b[38;5;196mred");
        // 74 bytes; limit 20 cuts at 64, inside the sequence at 60..71.
        assert!(keep_tail(&mut colored, 20));
        assert_eq!(colored, b"red");
        let mut link = b"y".repeat(60);
        link.extend_from_slice(b"\x1b]8;;https://example.com/a/long/path\x1b\\text");
        // The OSC string runs from 60 to 98; a cut at 80 is inside it.
        let cut_at = link.len() - 40 / 2;
        assert!(cut_at > 60 && cut_at < 98, "{cut_at}");
        assert!(keep_tail(&mut link, 40));
        assert_eq!(link, b"text");
    }

    #[test]
    fn test_teed_copy_keeps_text_the_terminal_refused() {
        // A broken terminal must not lose the text the TUI can still show.
        let (o, copy) = Output::from_writers(ClosedPipe, ClosedPipe).teed(64);
        assert!(outln!(o, "data").is_err());
        assert_eq!(copy.out(), "data\n");
    }

    #[test]
    fn test_capture_disables_progress() {
        // Animated spinners would draw over a TUI screen.
        let (o, _) = Output::capture();
        assert!(!o.progress_enabled());
        assert!(Output::stdio().progress_enabled());
    }
}
