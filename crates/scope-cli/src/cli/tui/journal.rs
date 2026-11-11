//! # TUI Journal
//!
//! Two files under the scope data directory:
//!
//! - `history.txt`: the input history, one line per entry. The rustyline
//!   REPL wrote the same file, so old history carries over.
//! - `tui-events.jsonl`: one JSON object per submitted line, with the
//!   exact input, the argument vector clap parsed, where the command ran,
//!   how long it took and how it ended. One more object records a signal
//!   that ends the TUI. A third kind, `event: "watch_venues"`, records each
//!   outcome of the venue editor of the watch grid ([`VenueRecord`]). Use
//!   the file to find out after the fact what a TUI session did.

use serde::Serialize;
use std::borrow::Cow;
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
    /// `ok`, `error`, `cancelled`, `session`, or `signal:<NAME>` (for
    /// example `signal:TERM`) when a signal ended the TUI.
    pub outcome: Cow<'static, str>,
    /// The error text, for `error`.
    pub error: Option<String>,
}

/// The name of the `event` field of a [`VenueRecord`], to tell it from the
/// records of submitted lines.
pub const VENUE_EVENT: &str = "watch_venues";

/// One outcome of the venue editor of the watch grid (design 2026-11-11,
/// item 2, section 4.4). The grid makes it; the app writes it, because the
/// app owns the journal.
///
/// `outcome` is `queued` (Enter), `applied` (the run took the change at the
/// start of a tick), `cancelled`, `refused`, `superseded` (a later edit of
/// the same target replaced it before the tick), `undone`, `saved` or
/// `save_refused`. A record with a reason in `reason` says why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VenueRecord {
    /// Always [`VENUE_EVENT`].
    pub event: &'static str,
    /// When the record was made (RFC 3339, UTC).
    pub at: String,
    pub watchlist: String,
    /// The run number of the last tick when the record was made.
    pub run: u64,
    /// The target's id.
    pub target: Option<String>,
    /// The grid's number of the edit; the same in the records of one edit.
    pub seq: Option<u64>,
    /// The venues before the edit.
    pub from: Vec<String>,
    /// The venues the edit asks for.
    pub to: Vec<String>,
    /// The line exactly as the user typed it.
    pub line: Option<String>,
    pub outcome: &'static str,
    pub reason: Option<String>,
    /// When Enter queued the edit.
    pub queued_at: Option<String>,
    /// The run number of the tick that applied the edit.
    pub applied_at_run: Option<u64>,
    /// The file of a save.
    pub path: Option<String>,
    /// SHA-256 of the file text the run loaded, for a save that was refused.
    pub loaded_sha256: Option<String>,
    /// SHA-256 of the file text on disk then.
    pub disk_sha256: Option<String>,
}

impl VenueRecord {
    /// A record with an outcome and nothing else set.
    pub fn new(
        outcome: &'static str,
        watchlist: impl Into<String>,
        run: u64,
        at: impl Into<String>,
    ) -> Self {
        Self {
            event: VENUE_EVENT,
            at: at.into(),
            watchlist: watchlist.into(),
            run,
            target: None,
            seq: None,
            from: Vec::new(),
            to: Vec::new(),
            line: None,
            outcome,
            reason: None,
            queued_at: None,
            applied_at_run: None,
            path: None,
            loaded_sha256: None,
            disk_sha256: None,
        }
    }
}

impl Journal {
    /// The journal in the scope state directory.
    pub fn in_data_dir() -> Self {
        let dir = scope::paths::state_dir();
        Self {
            history: Some(dir.join("history.txt")),
            events: Some(dir.join("tui-events.jsonl")),
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

    /// The last `n` lines of the event log, oldest first. A missing or
    /// unreadable file gives no lines.
    pub fn tail(&self, n: usize) -> Vec<String> {
        let Some(path) = &self.events else {
            return Vec::new();
        };
        let text = fs::read_to_string(path).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        lines[lines.len().saturating_sub(n)..]
            .iter()
            .map(|l| (*l).to_string())
            .collect()
    }

    /// Appends one event record.
    ///
    /// # Errors
    ///
    /// Returns the file system or serialization error.
    pub fn record(&self, event: &Event<'_>) -> io::Result<()> {
        self.append(event)
    }

    /// Appends one record of the venue editor.
    ///
    /// # Errors
    ///
    /// Returns the file system or serialization error.
    pub fn record_venues(&self, record: &VenueRecord) -> io::Result<()> {
        self.append(record)
    }

    fn append(&self, record: &impl Serialize) -> io::Result<()> {
        let Some(path) = &self.events else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut line = serde_json::to_string(record)?;
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
                outcome: outcome.into(),
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
    fn test_tail_returns_the_newest_lines_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let j = journal(&dir);
        assert!(j.tail(2).is_empty());
        std::fs::write(dir.path().join("e.jsonl"), "a\nb\nc\n").unwrap();
        assert_eq!(j.tail(2), vec!["b", "c"]);
        assert_eq!(j.tail(10), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_data_dir_journal_uses_the_documented_file_names() {
        // Docs and RCA instructions name these files; keep them stable.
        let j = Journal::in_data_dir();
        if let (Some(h), Some(e)) = (&j.history, &j.events) {
            assert!(h.ends_with("state/history.txt"));
            assert!(e.ends_with("state/tui-events.jsonl"));
        }
    }

    #[test]
    fn test_disabled_journal_is_a_no_op() {
        let j = Journal::default();
        assert!(j.load_history().is_empty());
        j.save_history(&["x".into()]).unwrap();
    }
}
