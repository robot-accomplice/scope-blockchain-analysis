//! # TUI Journal
//!
//! Two files under the scope data directory:
//!
//! - `history.txt`: the input history, one line per entry. The rustyline
//!   REPL wrote the same file, so old history carries over.
//! - `tui-events.jsonl`: one JSON object per submitted line, with the
//!   exact input, the argument vector clap parsed, where the command ran,
//!   how long it took and how it ended. Use it to find out after the fact
//!   what a TUI session did.

use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

/// The most history entries kept on disk.
pub const HISTORY_LIMIT: usize = 1000;

/// The header line that rustyline writes at the top of its history file.
const RUSTYLINE_HEADER: &str = "#V2";

/// Where the journal files live. `None` paths turn the journal off.
#[derive(Debug, Clone, Default)]
pub struct Journal {
    history: Option<PathBuf>,
    events: Option<PathBuf>,
}

/// One record in `tui-events.jsonl`.
#[derive(Debug, Serialize)]
pub struct Event<'a> {
    /// When the line was submitted (RFC 3339, UTC).
    pub started: String,
    /// Wall time until the command ended, in milliseconds.
    pub duration_ms: u128,
    /// The line exactly as the user typed it.
    pub input: &'a str,
    /// The argument vector clap parsed, after context injection. Empty for
    /// session commands and parse errors.
    pub argv: &'a [String],
    /// `pane`, `suspend` or `session`.
    pub route: &'static str,
    /// `ok`, `error`, `cancelled` or `session`.
    pub outcome: &'static str,
    /// The error text, for `error`.
    pub error: Option<String>,
}

impl Journal {
    /// The journal in the scope data directory.
    pub fn in_data_dir() -> Self {
        let dir = dirs::data_dir().map(|d| d.join("scope"));
        Self {
            history: dir.as_ref().map(|d| d.join("history.txt")),
            events: dir.map(|d| d.join("tui-events.jsonl")),
        }
    }

    /// A journal at explicit paths.
    pub fn at(history: PathBuf, events: PathBuf) -> Self {
        Self {
            history: Some(history),
            events: Some(events),
        }
    }

    /// Loads the history. A missing file is an empty history.
    pub fn load_history(&self) -> Vec<String> {
        let Some(path) = &self.history else {
            return Vec::new();
        };
        fs::read_to_string(path)
            .map(|text| {
                text.lines()
                    .filter(|l| !l.is_empty() && *l != RUSTYLINE_HEADER)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Saves the newest [`HISTORY_LIMIT`] entries.
    ///
    /// # Errors
    ///
    /// Returns the file system error.
    pub fn save_history(&self, history: &[String]) -> io::Result<()> {
        let Some(path) = &self.history else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let start = history.len().saturating_sub(HISTORY_LIMIT);
        let mut text = history[start..].join("\n");
        text.push('\n');
        fs::write(path, text)
    }

    /// Appends one event record.
    ///
    /// # Errors
    ///
    /// Returns the file system or serialization error.
    pub fn record(&self, event: &Event<'_>) -> io::Result<()> {
        let Some(path) = &self.events else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut line = serde_json::to_string(event)?;
        line.push('\n');
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?
            .write_all(line.as_bytes())
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn journal(dir: &tempfile::TempDir) -> Journal {
        Journal::at(dir.path().join("h.txt"), dir.path().join("e.jsonl"))
    }

    #[test]
    fn test_history_round_trip_keeps_newest_entries() {
        let dir = tempfile::tempdir().unwrap();
        let j = journal(&dir);
        let many: Vec<String> = (0..HISTORY_LIMIT + 5)
            .map(|i| format!("cmd {}", i))
            .collect();
        j.save_history(&many).unwrap();
        let loaded = j.load_history();
        assert_eq!(loaded.len(), HISTORY_LIMIT);
        assert_eq!(loaded.first().unwrap(), "cmd 5");
        assert_eq!(
            loaded.last().unwrap(),
            &format!("cmd {}", HISTORY_LIMIT + 4)
        );
    }

    #[test]
    fn test_rustyline_history_carries_over() {
        // Users upgrading from the REPL keep their history.
        let dir = tempfile::tempdir().unwrap();
        let j = journal(&dir);
        fs::write(dir.path().join("h.txt"), "#V2\naddress 0x1\n\ntx 0x2\n").unwrap();
        assert_eq!(j.load_history(), vec!["address 0x1", "tx 0x2"]);
    }

    #[test]
    fn test_events_append_one_json_object_per_line() {
        let dir = tempfile::tempdir().unwrap();
        let j = journal(&dir);
        let argv = vec![
            "scope".to_string(),
            "venues".to_string(),
            "list".to_string(),
        ];
        for outcome in ["ok", "error"] {
            j.record(&Event {
                started: "2026-10-10T12:00:00Z".into(),
                duration_ms: 12,
                input: "venues list",
                argv: &argv,
                route: "pane",
                outcome,
                error: (outcome == "error").then(|| "boom".to_string()),
            })
            .unwrap();
        }
        let text = fs::read_to_string(dir.path().join("e.jsonl")).unwrap();
        let rows: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["argv"][2], "list");
        assert_eq!(rows[1]["outcome"], "error");
        assert_eq!(rows[1]["error"], "boom");
    }

    #[test]
    fn test_disabled_journal_is_a_no_op() {
        let j = Journal::default();
        assert!(j.load_history().is_empty());
        j.save_history(&["x".into()]).unwrap();
    }
}
