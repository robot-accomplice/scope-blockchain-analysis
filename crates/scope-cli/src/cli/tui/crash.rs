//! # Crash log
//!
//! A panic in the TUI writes a crash log and tells the user where it is.
//! The hook first gives the terminal back ([`screen::restore`], which does
//! nothing when the screen is already back), so the notice is readable,
//! then calls the previous hook.

use super::journal::Journal;
use crate::cli::screen;
use regex::Regex;
use scope::http::replay::SECRET_PARAMS;
use std::backtrace::Backtrace;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// How many journal lines the crash log keeps.
const JOURNAL_TAIL_LINES: usize = 20;

const REDACTED: &str = "<redacted>";

/// Replaces secret values in one journal line: URL query values for the
/// replay `SECRET_PARAMS`, and the argument after `--key`, `--token` or
/// `--api-key` (plain or inside a JSON argv array).
fn redact(line: &str) -> String {
    let names = SECRET_PARAMS.join("|");
    let query = Regex::new(&format!(r#"(?i)\b({names})=[^&\s",]+"#)).expect("valid regex");
    let flag = Regex::new(r#"(?i)(--(?:key|token|api-key|api_key|apikey|access-token|access_token))("\s*,\s*"|\s+|=)[^\s",&]+"#)
        .expect("valid regex");
    let line = query.replace_all(line, format!("$1={REDACTED}").as_str());
    flag.replace_all(&line, format!("$1$2{REDACTED}").as_str())
        .into_owned()
}

/// Writes `crash-<timestamp>.log` in `dir`: the panic message and
/// location, a backtrace, and the redacted journal tail. The name has
/// milliseconds and the process id, so two crashes do not overwrite each
/// other. Returns the file path.
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
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S%.3f");
    let path = dir.join(format!("crash-{stamp}-{}.log", std::process::id()));
    let mut text = format!(
        "{info_text}\n\nBacktrace:\n{}\n",
        Backtrace::force_capture()
    );
    text.push_str(&format!(
        "\nInputs below are redacted; review before sharing.\nLast {JOURNAL_TAIL_LINES} journal lines:\n"
    ));
    for line in journal_tail {
        text.push_str(&redact(line));
        text.push('\n');
    }
    fs::write(&path, text)?;
    Ok(path)
}

/// Writes the crash log and returns the line that tells the user where it
/// is, or why it could not be written. `dir` is `None` when the platform
/// has no data directory.
fn crash_notice(dir: Option<&Path>, info_text: &str, journal_tail: &[String]) -> String {
    match dir
        .ok_or_else(|| io::Error::other("no data directory"))
        .and_then(|dir| write_crash_log(dir, info_text, journal_tail))
    {
        Ok(path) => format!("scope crashed; details: {}", path.display()),
        Err(e) => format!("scope crashed; the crash log failed: {e}"),
    }
}

/// Installs the crash-log panic hook. Only the first call has an effect.
pub(crate) fn install_panic_hook() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            screen::restore();
            let tail = Journal::in_data_dir().tail(JOURNAL_TAIL_LINES);
            let dir = dirs::data_dir().map(|d| d.join("scope"));
            let notice = crash_notice(dir.as_deref(), &info.to_string(), &tail);
            // `writeln!`, not `eprintln!`: a closed stderr must not panic
            // inside the hook, which would skip the previous hook.
            let _ = writeln!(io::stderr(), "{notice}");
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
        assert!(text.contains("Inputs below are redacted; review before sharing."));
    }

    #[test]
    fn test_crash_log_redacts_secrets_in_the_journal_tail() {
        let dir = tempfile::tempdir().unwrap();
        let tail = [
            r#"{"input":"setup --key sk-123 --access-token tokZ7","argv":["scope","setup","--key","sk-456"]}"#
                .to_string(),
            r#"{"error":"failed: https://x.io/api?chain=eth&apikey=sk3cr3tQ&Token=tok9"}"#.to_string(),
        ];
        let p = write_crash_log(dir.path(), "boom", &tail).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        for secret in ["sk-123", "sk-456", "sk3cr3tQ", "tok9", "tokZ7"] {
            assert!(!text.contains(secret), "{secret} leaked");
        }
        assert!(text.contains("chain=eth") && text.contains("<redacted>"));
    }

    #[test]
    fn test_crash_logs_written_back_to_back_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_crash_log(dir.path(), "a", &[]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = write_crash_log(dir.path(), "b", &[]).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn test_crash_notice_names_the_log_file_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        let notice = crash_notice(Some(dir.path()), "boom", &[]);
        let path = notice
            .strip_prefix("scope crashed; details: ")
            .expect(&notice);
        assert!(std::fs::read_to_string(path).unwrap().contains("boom"));
    }

    #[test]
    fn test_crash_notice_says_why_no_log_was_written() {
        // The user must learn that no log exists, and why.
        let none = crash_notice(None, "boom", &[]);
        assert_eq!(
            none,
            "scope crashed; the crash log failed: no data directory"
        );
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-dir");
        std::fs::write(&file, "x").unwrap();
        let blocked = crash_notice(Some(&file), "boom", &[]);
        assert!(
            blocked.starts_with("scope crashed; the crash log failed: "),
            "{blocked}"
        );
    }
}
