//! # Crash log
//!
//! A panic in the TUI must not leave the terminal in raw mode on the
//! alternate screen. The panic hook gives the terminal back, writes a crash
//! log, tells the user where it is, and then calls the previous hook.

use super::journal::Journal;
use std::backtrace::Backtrace;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// How many journal lines the crash log keeps.
const JOURNAL_TAIL_LINES: usize = 20;

/// Gives the terminal back: raw mode off, alternate screen off, cursor
/// shown. Every restore site calls this one function, so a new terminal
/// mode (bracketed paste) needs one change here.
pub(crate) fn restore_terminal() {
    ratatui::restore();
}

/// Writes `crash-<timestamp>.log` in `dir`: the panic message and
/// location, a backtrace, and the journal tail. Returns the file path.
///
/// # Errors
///
/// Returns the file system error.
pub fn write_crash_log(
    dir: &Path,
    info_text: &str,
    journal_tail: &[String],
) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("crash-{stamp}.log"));
    let mut text = format!(
        "{info_text}\n\nBacktrace:\n{}\n",
        Backtrace::force_capture()
    );
    text.push_str(&format!("\nLast {JOURNAL_TAIL_LINES} journal lines:\n"));
    for line in journal_tail {
        text.push_str(line);
        text.push('\n');
    }
    fs::write(&path, text)?;
    Ok(path)
}

/// Installs the crash-log panic hook. Only the first call has an effect,
/// so a resume that re-initializes the terminal does not stack hooks.
pub(crate) fn install_panic_hook() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal();
            let tail = Journal::in_data_dir().tail(JOURNAL_TAIL_LINES);
            let logged = dirs::data_dir()
                .map(|d| d.join("scope"))
                .ok_or_else(|| io::Error::other("no data directory"))
                .and_then(|dir| write_crash_log(&dir, &info.to_string(), &tail));
            match logged {
                Ok(path) => eprintln!("scope crashed; details: {}", path.display()),
                Err(e) => eprintln!("scope crashed; the crash log failed: {e}"),
            }
            previous(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crash_log_has_message_location_and_journal_tail() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_crash_log(
            dir.path(),
            "boom at src/x.rs:1:2",
            &["{\"input\":\"ctx\"}".to_string()],
        )
        .unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("crash-")
        );
        assert!(text.contains("boom at src/x.rs:1:2") && text.contains("\"input\":\"ctx\""));
        assert!(text.contains("Backtrace:"));
    }
}
