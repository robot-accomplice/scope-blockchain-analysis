//! # Diagnostics Sink
//!
//! Library code has no injected writer of its own, but it still
//! writes warnings for the user (for example "Holder data requires a Pro
//! API key"). Those warnings go through this module instead of `eprintln!`.
//!
//! By default they go to stderr. A full-screen TUI calls [`redirect`](crate::diag::redirect) so
//! they appear in its output pane instead of over the screen. The binary
//! also gives [`Stderr`](crate::diag::Stderr) to its tracing subscriber, so log lines follow the
//! same route.

use std::fmt;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, RwLock};

/// A shared writer that receives diagnostics while a redirect is active.
pub type Sink = Arc<Mutex<dyn Write + Send>>;

/// The active redirect. `None` means the process stderr.
static SINK: RwLock<Option<Sink>> = RwLock::new(None);

/// Writes one diagnostic line to the active sink.
///
/// A failed write is ignored: diagnostics are best-effort, and the library
/// call that produced the warning must not fail because stderr is closed.
pub fn notice(args: fmt::Arguments<'_>) {
    let _ = Stderr.write_fmt(format_args!("{}\n", args));
}

/// Sends diagnostics to `sink` until the returned guard is dropped.
///
/// Nested redirects are allowed. Dropping a guard restores the sink that
/// was active when that guard was created.
pub fn redirect(sink: Sink) -> RedirectGuard {
    let mut slot = SINK.write().unwrap_or_else(|p| p.into_inner());
    let previous = slot.replace(sink);
    RedirectGuard { previous }
}

/// Restores the previous diagnostics sink when dropped.
#[must_use = "diagnostics return to stderr as soon as the guard is dropped"]
pub struct RedirectGuard {
    previous: Option<Sink>,
}

impl Drop for RedirectGuard {
    fn drop(&mut self) {
        let mut slot = SINK.write().unwrap_or_else(|p| p.into_inner());
        *slot = self.previous.take();
    }
}

/// A writer that forwards to the active diagnostics sink.
///
/// Use it where an API wants a writer for diagnostics, for example
/// `tracing_subscriber::fmt().with_writer(|| scope::diag::Stderr)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stderr;

impl Write for Stderr {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let slot = SINK.read().unwrap_or_else(|p| p.into_inner());
        match slot.as_ref() {
            Some(sink) => sink.lock().unwrap_or_else(|p| p.into_inner()).write(buf),
            None => io::stderr().write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let slot = SINK.read().unwrap_or_else(|p| p.into_inner());
        match slot.as_ref() {
            Some(sink) => sink.lock().unwrap_or_else(|p| p.into_inner()).flush(),
            None => io::stderr().flush(),
        }
    }
}

/// Writes a formatted diagnostic line. See [`notice`].
#[macro_export]
macro_rules! notice {
    ($($arg:tt)*) => {
        $crate::diag::notice(format_args!($($arg)*))
    };
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stderr_writer_flushes_with_and_without_redirect() {
        // tracing calls flush on its writer; both routes must accept it.
        let mut w = Stderr;
        assert!(w.flush().is_ok());
        let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let _g = redirect(buf.clone());
        assert!(w.flush().is_ok());
    }

    #[test]
    fn test_redirect_captures_and_restores() {
        // While the TUI owns the screen, library warnings must land in its
        // pane; after it exits they must reach stderr again.
        let outer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let inner: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let _g1 = redirect(outer.clone());
            notice(format_args!("one"));
            {
                let _g2 = redirect(inner.clone());
                crate::notice!("two {}", 2);
            }
            notice(format_args!("three"));
        }
        // Other tests may emit notices concurrently into the same global
        // sink, so check for our lines and their order, not for equality.
        let outer = String::from_utf8_lossy(&outer.lock().unwrap()).into_owned();
        let inner = String::from_utf8_lossy(&inner.lock().unwrap()).into_owned();
        let (one, three) = (outer.find("one\n").unwrap(), outer.find("three\n").unwrap());
        assert!(one < three);
        assert!(!outer.contains("two 2"));
        assert!(inner.contains("two 2\n"));
        assert!(!inner.contains("one") && !inner.contains("three"));
    }
}
