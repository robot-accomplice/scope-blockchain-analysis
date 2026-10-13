//! # Screen
//!
//! The one owner of the full-screen terminal for the TUI and the monitor.
//! [`enter`] takes it: raw mode, the alternate screen, bracketed paste and,
//! for the monitor, mouse capture. [`restore`] gives it back. The first
//! [`enter`] installs one panic hook that restores the terminal; later
//! calls (a TUI resume, a second monitor run) install no more.
//!
//! No other module calls `ratatui::init` or `ratatui::restore`: each
//! `ratatui::init` adds a panic hook, so a resume per suspend would stack
//! them. A source guard in the tests keeps it so.

use std::io::{self, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Whether the screen reports mouse events. Only the monitor uses them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mouse {
    Off,
    Capture,
}

/// True while this module holds the screen. [`restore`] acts only then,
/// so a second restore (a panic after a normal exit, `cleanup` then
/// `Drop`) does not move the shell's cursor.
static HELD: AtomicBool = AtomicBool::new(false);

/// How many restore hooks this module installed. The tests read it.
static HOOK_INSTALLS: AtomicUsize = AtomicUsize::new(0);

/// Takes the terminal and returns a ratatui terminal on stdout.
///
/// # Panics
///
/// Panics when the terminal cannot be set up, as `ratatui::init` does.
pub(crate) fn enter(mouse: Mouse) -> ratatui::DefaultTerminal {
    take(
        &mut io::stdout(),
        crossterm::terminal::enable_raw_mode,
        mouse,
    )
    .and_then(|()| ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout())))
    .expect("failed to initialize terminal")
}

/// Gives the terminal back: mouse capture and bracketed paste off, raw
/// mode off, alternate screen off, cursor shown. Does nothing when the
/// screen is not held.
pub(crate) fn restore() {
    if HELD.swap(false, Ordering::SeqCst) {
        let mut out = io::stdout();
        let _ = crossterm::execute!(out, crossterm::event::DisableMouseCapture);
        paste_mode(&mut out, false);
        ratatui::restore();
    }
}

/// Bracketed paste on or off, as the escape bytes a terminal reads.
pub(crate) fn paste_mode(w: &mut impl Write, on: bool) {
    let _ = if on {
        crossterm::execute!(w, crossterm::event::EnableBracketedPaste)
    } else {
        crossterm::execute!(w, crossterm::event::DisableBracketedPaste)
    };
}

/// Installs the restore hook once, turns raw mode on with `raw_mode`, then
/// writes the sequences that take the screen to `w`.
fn take(
    w: &mut impl Write,
    raw_mode: impl FnOnce() -> io::Result<()>,
    mouse: Mouse,
) -> io::Result<()> {
    install_restore_hook();
    raw_mode()?;
    HELD.store(true, Ordering::SeqCst);
    crossterm::execute!(w, crossterm::terminal::EnterAlternateScreen)?;
    paste_mode(w, true);
    if mouse == Mouse::Capture {
        crossterm::execute!(w, crossterm::event::EnableMouseCapture)?;
    }
    Ok(())
}

/// Installs the panic hook that restores the terminal, then calls the
/// previous hook. Only the first call has an effect.
fn install_restore_hook() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        HOOK_INSTALLS.fetch_add(1, Ordering::SeqCst);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            previous(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // panic, and stack without end in a long TUI session.
        for mouse in [Mouse::Off, Mouse::Capture, Mouse::Off, Mouse::Capture] {
            let mut screen = Vec::new();
            take(&mut screen, || Ok(()), mouse).unwrap();
            assert!(screen.starts_with(b"\x1b[?1049h\x1b[?2004h"), "{screen:?}");
            assert_eq!(
                screen.ends_with(b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h"),
                mouse == Mouse::Capture,
                "mouse capture only for the monitor"
            );
        }
        assert_eq!(HOOK_INSTALLS.load(Ordering::SeqCst), 1);
        // The tests do not hold the real screen.
        HELD.store(false, Ordering::SeqCst);
    }

    #[test]
    fn test_raw_mode_failure_takes_nothing() {
        let mut screen = Vec::new();
        let e = take(&mut screen, || Err(io::Error::other("no tty")), Mouse::Off).unwrap_err();
        assert_eq!(e.to_string(), "no tty");
        assert!(screen.is_empty());
    }

    /// The non-test source of a file: everything before its test module.
    fn code_of(file: &str) -> &str {
        file.split("#[cfg(test)]\nmod tests").next().unwrap()
    }

    fn rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                rs_files(&p, out);
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p);
            }
        }
    }

    #[test]
    fn test_only_this_module_takes_or_gives_back_the_screen() {
        let cli = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli");
        let mut files = Vec::new();
        rs_files(&cli, &mut files);
        let calls = [
            "ratatui::init",
            "ratatui::try_init",
            "ratatui::restore",
            "ratatui::try_restore",
            "enable_raw_mode",
            "disable_raw_mode",
            "EnterAlternateScreen",
            "LeaveAlternateScreen",
            "EnableMouseCapture",
            "DisableMouseCapture",
            "panic::set_hook",
        ];
        let mut found = Vec::new();
        for path in files {
            let name = path
                .strip_prefix(&cli)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if name == "screen.rs" || name == "tui/crash.rs" {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap();
            for call in calls {
                if code_of(&src).contains(call) {
                    found.push(format!("{name}: {call}"));
                }
            }
        }
        assert!(found.is_empty(), "use cli::screen:\n{}", found.join("\n"));
    }
}
