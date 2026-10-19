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
//!
//! [`scoped`](crate::diag::scoped) gives one future its own sink, over the redirect. The TUI
//! polls a pane command and a watchlist run in one task; each keeps its own
//! warnings (#47).

use std::fmt;
use std::future::Future;
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

tokio::task_local! {
    /// The sink of the future [`scoped`] runs, while that future is polled.
    static SCOPED: Sink;
}

/// Runs `fut` with its diagnostics in `sink`, whatever the redirect is.
///
/// The sink holds only while `fut` is polled. So two futures that one task
/// polls in turn (a TUI pane command and a watchlist run) keep their
/// warnings apart: the other one's go to the redirect.
pub fn scoped<F: Future>(sink: Sink, fut: F) -> impl Future<Output = F::Output> {
    SCOPED.scope(sink, fut)
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
        if let Ok(written) =
            SCOPED.try_with(|sink| sink.lock().unwrap_or_else(|p| p.into_inner()).write(buf))
        {
            return written;
        }
        let slot = SINK.read().unwrap_or_else(|p| p.into_inner());
        match slot.as_ref() {
            Some(sink) => sink.lock().unwrap_or_else(|p| p.into_inner()).write(buf),
            None => io::stderr().write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Ok(flushed) =
            SCOPED.try_with(|sink| sink.lock().unwrap_or_else(|p| p.into_inner()).flush())
        {
            return flushed;
        }
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

    /// Serializes the tests that redirect the process-global sink. Without
    /// it, one test's redirect captures the other's notices (34 of 60 runs
    /// failed with 8 test threads).
    static REDIRECT_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_stderr_writer_flushes_with_and_without_redirect() {
        let _lock = REDIRECT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // tracing calls flush on its writer; both routes must accept it.
        let mut w = Stderr;
        assert!(w.flush().is_ok());
        let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let _g = redirect(buf.clone());
        assert!(w.flush().is_ok());
    }

    #[test]
    fn test_a_scoped_future_keeps_its_notices_from_the_redirect() {
        let _lock = REDIRECT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // The lock is held outside the runtime, never across an await.
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        // One task polls both futures in turn, as the TUI polls a pane
        // command and a watchlist run.
        let pane: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let watch: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let _g = redirect(pane.clone());
        let watching = scoped(watch.clone(), async {
            for i in 0..3 {
                notice(format_args!("watch {i}"));
                tokio::task::yield_now().await;
            }
        });
        let command = async {
            for i in 0..3 {
                notice(format_args!("pane {i}"));
                tokio::task::yield_now().await;
            }
        };
        rt.block_on(async { tokio::join!(watching, command) });
        let pane = String::from_utf8_lossy(&pane.lock().unwrap()).into_owned();
        let watch = String::from_utf8_lossy(&watch.lock().unwrap()).into_owned();
        assert!(
            watch.contains(
                "watch 0
watch 1
watch 2
"
            ),
            "{watch}"
        );
        assert!(!watch.contains("pane"), "{watch}");
        assert!(
            pane.contains(
                "pane 0
"
            ) && pane.contains(
                "pane 2
"
            ),
            "{pane}"
        );
        assert!(!pane.contains("watch"), "{pane}");
    }

    #[test]
    fn test_redirect_captures_and_restores() {
        let _lock = REDIRECT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
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
