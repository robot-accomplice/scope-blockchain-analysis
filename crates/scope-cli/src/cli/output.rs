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
use std::sync::{Arc, Mutex};

/// A shared, thread-safe writer.
type Sink = Arc<Mutex<dyn Write + Send>>;

/// The output channels for one command run.
///
/// Cloning is cheap: clones share the same writers.
#[derive(Clone)]
pub struct Output {
    out: Sink,
    err: Sink,
    progress: bool,
}

impl Output {
    /// Output bound to the process stdout and stderr, with progress
    /// indicators enabled (they still hide themselves when stderr is not a TTY).
    pub fn stdio() -> Self {
        Self {
            out: Arc::new(Mutex::new(io::stdout())),
            err: Arc::new(Mutex::new(io::stderr())),
            progress: true,
        }
    }

    /// Output bound to in-memory buffers, with progress indicators disabled.
    ///
    /// Returns the output and a handle that reads what was written.
    pub fn capture() -> (Self, Captured) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let err = Arc::new(Mutex::new(Vec::new()));
        let captured = Captured {
            out: out.clone(),
            err: err.clone(),
        };
        let output = Self {
            out,
            err,
            progress: false,
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
        }
    }

    /// True when animated progress indicators may draw on the terminal.
    pub fn progress_enabled(&self) -> bool {
        self.progress
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

/// Reads back what an [`Output::capture`] output received.
#[derive(Clone)]
pub struct Captured {
    out: Arc<Mutex<Vec<u8>>>,
    err: Arc<Mutex<Vec<u8>>>,
}

impl Captured {
    /// Everything written to the data channel so far.
    pub fn out(&self) -> String {
        String::from_utf8_lossy(&lock(&self.out)).into_owned()
    }

    /// Everything written to the diagnostic channel so far.
    pub fn err(&self) -> String {
        String::from_utf8_lossy(&lock(&self.err)).into_owned()
    }
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
    fn test_capture_disables_progress() {
        // Animated spinners would draw over a TUI screen.
        let (o, _) = Output::capture();
        assert!(!o.progress_enabled());
        assert!(Output::stdio().progress_enabled());
    }
}
