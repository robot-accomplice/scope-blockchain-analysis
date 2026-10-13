//! # Command Output
//!
//! The writers that command handlers print through. The CLI binds them to
//! stdout and stderr. The TUI and the tests bind them to in-memory buffers,
//! so the same handler can run in both places without printing over the
//! terminal UI.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use scope_cli::cli::output::Output;
//! use scope_cli::{errln, outln};
//!
//! let out = Output::stdio();
//! outln!(out, "Balance: {}", 42)?;   // data → stdout
//! errln!(out, "Warning: partial")?; // diagnostics → stderr
//! ```
//!
//! Every macro returns `std::io::Result<()>`. Propagate it with `?`, so a
//! command whose output failed (closed pipe, full disk) does not succeed.
//!
//! Data goes to `out`. Warnings, progress and status lines go to `err`, so
//! piped JSON and CSV stay clean.

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

/// The output channels for one command run.
///
/// Cloning is cheap: clones share the same writers.
#[derive(Clone)]
pub struct Output {
    out: Sink,
    err: Sink,
    progress: bool,
    hook: Option<ProgressHook>,
    tui: bool,
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
            tui: false,
        }
    }

    /// Output bound to in-memory buffers, with progress indicators disabled.
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
            tui: false,
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
            tui: true,
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
            tui: true,
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
            tui: false,
        }
    }

    /// True when animated progress indicators may draw on the terminal.
    pub fn progress_enabled(&self) -> bool {
        self.progress
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

    /// Changes the progress record. Does nothing when there is no hook.
    pub fn update_progress(&self, change: impl FnOnce(&mut RunProgress)) {
        if let Some(hook) = &self.hook {
            change(&mut lock(hook));
        }
    }

    /// The data channel as a writer, for code that writes through
    /// [`Write`] (the prompts, the setup wizard). What it writes takes the
    /// same path as [`outln!`](crate::outln), so a TUI run keeps a copy.
    pub fn data_writer(&self) -> DataWriter {
        DataWriter(self.out.clone())
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
}

/// The data channel of an [`Output`] as a writer. See [`Output::data_writer`].
pub struct DataWriter(Sink);

impl Write for DataWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        lock(&self.0).write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        lock(&self.0).flush()
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
/// of the line. Returns true when it dropped text.
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
        .unwrap_or(cut);
    buf.drain(..start);
    true
}

/// Locks a mutex, recovering the data if a panicking writer poisoned it.
/// Output must keep working after an unrelated panic on another thread.
fn lock<T: ?Sized>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Writes one formatted message to a sink.
///
/// A failed write is returned to the caller, never dropped: a command whose
/// output did not arrive (closed pipe, full disk) must not report success.
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
    fn test_data_writer_writes_to_the_data_channel() {
        // The setup wizard writes through a writer; a TUI run must keep
        // its text like any other data.
        let (o, cap) = Output::capture();
        let mut w = o.data_writer();
        write!(w, "Step 1: API Keys").unwrap();
        w.flush().unwrap();
        assert_eq!(cap.out(), "Step 1: API Keys");
        assert_eq!(cap.err(), "");
        let mut closed = Output::from_writers(ClosedPipe, Vec::new()).data_writer();
        assert_eq!(
            closed.write(b"x").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
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
