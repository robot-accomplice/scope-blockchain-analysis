//! # Screen
//!
//! The one owner of the full-screen terminal for the TUI and the monitor.
//! [`enter`] takes it: raw mode, the alternate screen, bracketed paste and,
//! for the monitor, mouse capture. [`restore`] gives it back. The first
//! [`enter`] installs one panic hook that restores the terminal; later
//! calls (a TUI resume, a second monitor run) install no more.
//!
//! No other module takes the screen or sets a restore hook. A guard test
//! (`tests/one_screen_owner.rs`) keeps it so.

use scope::error::{Result, ScopeError};
use std::io::{self, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the screen reports mouse events. Only the monitor uses them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mouse {
    Off,
    Capture,
}

/// The state of the screen: whether it is held, and whether the restore
/// hook is in. One process-wide value ([`SCREEN`]); tests make their own.
struct Lifecycle {
    /// True while the screen is held. Giving it back acts only then, so a
    /// second restore (a panic after a normal exit, `cleanup` then `Drop`)
    /// does not move the shell's cursor.
    held: AtomicBool,
    /// Set once the restore hook is in.
    hook: OnceLock<()>,
}

static SCREEN: Lifecycle = Lifecycle::new();

impl Lifecycle {
    const fn new() -> Self {
        Self {
            held: AtomicBool::new(false),
            hook: OnceLock::new(),
        }
    }

    /// Runs `install_hook` the first time only, turns raw mode on with
    /// `raw_mode`, then writes the sequences that take the screen to `w`.
    fn take(
        &self,
        w: &mut impl Write,
        raw_mode: impl FnOnce() -> io::Result<()>,
        mouse: Mouse,
        install_hook: impl FnOnce(),
    ) -> io::Result<()> {
        self.hook.get_or_init(install_hook);
        raw_mode()?;
        self.held.store(true, Ordering::SeqCst);
        crossterm::execute!(w, crossterm::terminal::EnterAlternateScreen)?;
        paste_mode(w, true);
        if mouse == Mouse::Capture {
            crossterm::execute!(w, crossterm::event::EnableMouseCapture)?;
        }
        Ok(())
    }

    /// When the screen is held: writes the sequences that turn mouse
    /// capture and bracketed paste off to `w`, then runs `leave` (raw mode
    /// off, alternate screen off). Otherwise does nothing.
    fn give_back(&self, w: &mut impl Write, leave: impl FnOnce()) {
        if self.held.swap(false, Ordering::SeqCst) {
            let _ = crossterm::execute!(w, crossterm::event::DisableMouseCapture);
            paste_mode(w, false);
            leave();
        }
    }
}

/// Takes the terminal and returns a ratatui terminal on stdout.
///
/// # Errors
///
/// [`not_taken`] with the terminal error (no tty, mouse capture refused).
/// The screen is given back first, so the caller does not have to.
pub(crate) fn enter(mouse: Mouse) -> Result<ratatui::DefaultTerminal> {
    take_screen(
        &SCREEN,
        &mut io::stdout(),
        crossterm::terminal::enable_raw_mode,
        mouse,
    )
    .and_then(|()| {
        ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))
            .map_err(not_taken)
    })
    .inspect_err(|_| restore())
}

/// The one error for a terminal the screen cannot take, for the TUI and
/// the monitor alike.
pub(crate) fn not_taken(e: io::Error) -> ScopeError {
    ScopeError::io_context(e, "cannot take the terminal")
}

/// [`Lifecycle::take`] with the restore hook, as a command error.
fn take_screen(
    lifecycle: &Lifecycle,
    w: &mut impl Write,
    raw_mode: impl FnOnce() -> io::Result<()>,
    mouse: Mouse,
) -> Result<()> {
    lifecycle
        .take(w, raw_mode, mouse, install_restore_hook)
        .map_err(not_taken)
}

/// Gives the terminal back: mouse capture and bracketed paste off, raw
/// mode off, alternate screen off, cursor shown. Does nothing when the
/// screen is not held.
pub(crate) fn restore() {
    SCREEN.give_back(&mut io::stdout(), ratatui::restore);
}

/// Bracketed paste on or off, as the escape bytes a terminal reads.
fn paste_mode(w: &mut impl Write, on: bool) {
    let _ = if on {
        crossterm::execute!(w, crossterm::event::EnableBracketedPaste)
    } else {
        crossterm::execute!(w, crossterm::event::DisableBracketedPaste)
    };
}

/// Sets the panic hook that restores the terminal, then calls the
/// previous hook. [`Lifecycle::take`] runs it once.
fn install_restore_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const ALT_SCREEN_AND_PASTE: &[u8] = b"\x1b[?1049h\x1b[?2004h";
    const MOUSE_ON: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h";

    #[test]
    fn test_paste_mode_writes_the_enable_and_disable_sequences() {
        let mut on = Vec::new();
        paste_mode(&mut on, true);
        assert_eq!(on, b"\x1b[?2004h");
        let mut off = Vec::new();
        paste_mode(&mut off, false);
        assert_eq!(off, b"\x1b[?2004l");
    }

    #[test]
    fn test_tui_resumes_and_monitor_runs_install_one_restore_hook() {
        // Each TUI resume and each monitor run takes the screen again. A
        // hook per take would restore the terminal once per take on a
        // panic, and stack without end in a long TUI session. The test
        // uses its own lifecycle and a counting installer: no real hook.
        let screen = Lifecycle::new();
        let installs = Cell::new(0);
        for mouse in [Mouse::Off, Mouse::Capture, Mouse::Off, Mouse::Capture] {
            let mut out = Vec::new();
            screen
                .take(
                    &mut out,
                    || Ok(()),
                    mouse,
                    || installs.set(installs.get() + 1),
                )
                .unwrap();
            assert!(out.starts_with(ALT_SCREEN_AND_PASTE), "{out:?}");
            assert_eq!(
                out.ends_with(MOUSE_ON),
                mouse == Mouse::Capture,
                "mouse capture only for the monitor"
            );
            screen.give_back(&mut Vec::new(), || {});
        }
        assert_eq!(installs.get(), 1);
    }

    #[test]
    fn test_give_back_acts_once_per_take() {
        let screen = Lifecycle::new();
        let leaves = Cell::new(0);
        screen
            .take(&mut Vec::new(), || Ok(()), Mouse::Capture, || {})
            .unwrap();
        let mut first = Vec::new();
        screen.give_back(&mut first, || leaves.set(leaves.get() + 1));
        assert!(
            first.ends_with(b"\x1b[?2004l"),
            "mouse and paste off: {first:?}"
        );
        let mut second = Vec::new();
        screen.give_back(&mut second, || leaves.set(leaves.get() + 1));
        assert!(second.is_empty(), "a second restore moves the cursor");
        assert_eq!(leaves.get(), 1);
    }

    #[test]
    fn test_a_failed_take_is_the_one_terminal_error() {
        // The TUI and the monitor show the same words for a terminal
        // they cannot take.
        let screen = Lifecycle::new();
        let e = take_screen(
            &screen,
            &mut Vec::new(),
            || Err(io::Error::other("no tty")),
            Mouse::Capture,
        )
        .unwrap_err();
        assert!(matches!(&e, ScopeError::IoError(io) if io.kind() == io::ErrorKind::Other));
        assert_eq!(e.to_string(), "I/O error: cannot take the terminal: no tty");
    }

    #[test]
    fn test_raw_mode_failure_takes_nothing() {
        let screen = Lifecycle::new();
        let mut out = Vec::new();
        let e = screen
            .take(
                &mut out,
                || Err(io::Error::other("no tty")),
                Mouse::Off,
                || {},
            )
            .unwrap_err();
        assert_eq!(e.to_string(), "no tty");
        assert!(out.is_empty());
        assert!(!screen.held.load(Ordering::SeqCst));
    }

    /// A terminal that refuses everything after raw mode is on.
    struct Refusing;

    impl Write for Refusing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("refused"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_a_failed_take_is_an_error_and_can_be_given_back() {
        // The monitor returns this error to the user; `enter` gives the
        // screen back before it does.
        let screen = Lifecycle::new();
        let e = screen
            .take(&mut Refusing, || Ok(()), Mouse::Capture, || {})
            .unwrap_err();
        assert_eq!(e.to_string(), "refused");
        let left = Cell::new(false);
        screen.give_back(&mut Vec::new(), || left.set(true));
        assert!(left.get(), "raw mode was on, so it is turned off");
    }
}
