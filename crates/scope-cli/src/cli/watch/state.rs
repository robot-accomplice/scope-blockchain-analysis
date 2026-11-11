//! The memory of `scope watch` between runs (design 2026-11-11, section 3).
//!
//! Two metrics compare a tick with the one before: `largest_trade` and
//! `volume_change_pct`. [`History`] holds what they need. A run that ends
//! loses it, so a `--once` run from cron was always a first tick. This
//! module saves the memory to a small JSON file for each watchlist and loads
//! it at the start of the next run.
//!
//! The file is a cache, not the record of the run. A bad file is moved
//! aside, a write failure is a note and a field of the `end` record, and
//! neither changes the exit code. The pure part (which entry is stale) is in
//! scope-core ([`History::restore`]); this module does the I/O.

use super::is_watchlist_path;
use scope::watch::{Entry, History, MarketKey, Restored};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

/// The `version` of the file. A change of the format bumps it.
pub const STATE_VERSION: u32 = 1;

/// An entry is stale when it is older than this many intervals (Q6). The
/// interval is the gap between runs: with `--once`, the `--every` value.
pub const STALE_AFTER_INTERVALS: u64 = 3;

/// How many hex characters of the SHA-256 of the identity name the file.
const HASH_CHARS: usize = 8;

/// The extension of a state file and of the files beside it.
const JSON_EXT: &str = "json";
const LOCK_EXT: &str = "lock";
const BAD_EXT: &str = "json.bad";
const TMP_EXT: &str = "json.tmp";

/// What the file holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StateFile {
    version: u32,
    identity: String,
    scope_version: String,
    written_at_ms: u64,
    markets: Vec<Entry>,
}

/// The identity of a watchlist: which list a state file belongs to. A
/// saved name is the same from any working directory. A path is made
/// absolute, so a cron job with its own working directory has a stable
/// value. A saved name and a path to the same file are two identities, on
/// purpose: the code cannot tell without a file system lookup that may
/// fail. The cost is one first tick when a user changes how they name the
/// list.
pub fn identity(arg: &str, path: &Path) -> String {
    if is_watchlist_path(arg) {
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        format!("path:{}", absolute.display())
    } else {
        format!("saved:{arg}")
    }
}

/// The identity of a target from the command line: its canonical
/// definition (`file::to_text` of the effective list, after the flags).
/// Two jobs with different rules, markets or timing do not share memory; the
/// same words always do.
pub fn inline_identity(definition: &str) -> String {
    format!("inline:{definition}")
}

/// The files of one watchlist's state.
#[derive(Debug, Clone, PartialEq)]
pub struct StateFiles {
    pub json: PathBuf,
    pub lock: PathBuf,
    pub bad: PathBuf,
    tmp: PathBuf,
}

/// The files for `identity` in `dir`: `<slug>-<hash8>.json`, where the slug
/// is `name` with every character outside `a-z A-Z 0-9 - _` replaced by `-`
/// (for people) and the hash is the first 8 hex characters of SHA-256 of the
/// identity (for uniqueness).
pub fn files(dir: &Path, name: &str, identity: &str) -> StateFiles {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let hash = hex::encode(Sha256::digest(identity.as_bytes()));
    let stem = format!("{slug}-{}", &hash[..HASH_CHARS]);
    let with = |ext: &str| dir.join(format!("{stem}.{ext}"));
    StateFiles {
        json: with(JSON_EXT),
        lock: with(LOCK_EXT),
        bad: with(BAD_EXT),
        tmp: with(TMP_EXT),
    }
}

/// A state file that could not be used.
#[derive(Debug, Clone, PartialEq)]
pub struct Discarded {
    /// Why, in words.
    pub reason: String,
    /// Where the bytes went; `None` when they could not be moved.
    pub bad: Option<PathBuf>,
    /// Why they could not be moved.
    pub move_error: Option<String>,
}

/// What loading the state gave.
#[derive(Debug)]
pub struct Loaded {
    /// The memory the run starts with.
    pub history: History,
    /// What was done with each saved entry.
    pub restored: Vec<Restored>,
    /// A file was there and was valid.
    pub found: bool,
    /// A file was there and was not valid.
    pub discarded: Option<Discarded>,
}

impl Loaded {
    fn empty() -> Self {
        Self {
            history: History::default(),
            restored: Vec::new(),
            found: false,
            discarded: None,
        }
    }
}

/// Loads the state of a list. No file is a first tick, not an error. A file
/// that cannot be read or parsed, or whose `version` is not
/// [`STATE_VERSION`], is moved to `<name>.json.bad` (the bytes stay as
/// evidence; an older `.bad` is replaced) and the run starts with no state.
pub fn load(files: &StateFiles, now_ms: u64, max_age_ms: u64) -> Loaded {
    let bytes = match std::fs::read(&files.json) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Loaded::empty(),
        Err(e) => return discard(files, format!("cannot read: {e}")),
    };
    match parse(&bytes) {
        Ok(file) => {
            let (history, restored) = History::restore(file.markets, now_ms, max_age_ms);
            Loaded {
                history,
                restored,
                found: true,
                discarded: None,
            }
        }
        Err(reason) => discard(files, reason),
    }
}

fn parse(bytes: &[u8]) -> Result<StateFile, String> {
    let file: StateFile =
        serde_json::from_slice(bytes).map_err(|e| format!("not a valid state file: {e}"))?;
    if file.version == STATE_VERSION {
        Ok(file)
    } else {
        Err(format!("version {} is not {STATE_VERSION}", file.version))
    }
}

fn discard(files: &StateFiles, reason: String) -> Loaded {
    let moved = std::fs::rename(&files.json, &files.bad);
    let (bad, move_error) = match moved {
        Ok(()) => (Some(files.bad.clone()), None),
        Err(e) => (None, Some(e.to_string())),
    };
    Loaded {
        discarded: Some(Discarded {
            reason,
            bad,
            move_error,
        }),
        ..Loaded::empty()
    }
}

/// What a save wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Saved {
    /// The entries in the file now.
    pub written: usize,
    /// The entries left out: markets that the list no longer has, or that
    /// have no rule on a since-last-tick metric.
    pub dropped: usize,
}

/// The entries of `disk` and `mine` for each market, merged: the entry with
/// the larger `updated_ms` wins, and `last_trade_ms` is the larger of the
/// two. A slow run that ends late cannot step the memory back.
fn merge(disk: Vec<Entry>, mine: Vec<Entry>) -> Vec<Entry> {
    let mut merged: HashMap<MarketKey, Entry> = HashMap::new();
    for entry in disk.into_iter().chain(mine) {
        match merged.remove(&entry.key) {
            None => {
                merged.insert(entry.key.clone(), entry);
            }
            Some(old) => {
                let last_trade_ms = old.last_trade_ms.max(entry.last_trade_ms);
                // On a tie, the later argument (this run) wins.
                let mut winner = if entry.updated_ms >= old.updated_ms {
                    entry
                } else {
                    old
                };
                winner.last_trade_ms = last_trade_ms;
                merged.insert(winner.key.clone(), winner);
            }
        }
    }
    let mut entries: Vec<Entry> = merged.into_values().collect();
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    entries
}

/// Saves the memory of this run: under the lock of the list, reads the file
/// on disk (an invalid one counts as empty), merges, drops the entries of
/// markets not in `keep`, writes a temporary file and renames it over the
/// state file. The lock is held for this short step only, and the OS drops
/// it when the process ends, so a crash leaves no stale lock.
///
/// # Errors
///
/// The exact text of the failure (folder, lock, write or rename).
pub fn save(
    files: &StateFiles,
    identity: &str,
    history: &History,
    keep: &[MarketKey],
    now_ms: u64,
) -> Result<Saved, String> {
    save_with(files, identity, history, keep, now_ms, &|from, to| {
        std::fs::rename(from, to)
    })
}

/// [`save`] with the last step given, so a test can fail it.
fn save_with(
    files: &StateFiles,
    identity: &str,
    history: &History,
    keep: &[MarketKey],
    now_ms: u64,
    rename: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> Result<Saved, String> {
    let fail = |what: &str, path: &Path, e: io::Error| format!("{what} {}: {e}", path.display());
    if let Some(dir) = files.json.parent() {
        std::fs::create_dir_all(dir).map_err(|e| fail("cannot create", dir, e))?;
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&files.lock)
        .map_err(|e| fail("cannot open", &files.lock, e))?;
    lock.lock()
        .map_err(|e| fail("cannot lock", &files.lock, e))?;

    let on_disk = std::fs::read(&files.json)
        .ok()
        .and_then(|bytes| parse(&bytes).ok())
        .map(|file| file.markets)
        .unwrap_or_default();
    let merged = merge(on_disk, history.entries());
    let total = merged.len();
    let markets: Vec<Entry> = merged
        .into_iter()
        .filter(|e| keep.contains(&e.key))
        .collect();
    let saved = Saved {
        written: markets.len(),
        dropped: total - markets.len(),
    };
    let file = StateFile {
        version: STATE_VERSION,
        identity: identity.to_string(),
        scope_version: scope::VERSION.to_string(),
        written_at_ms: now_ms,
        markets,
    };
    let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(&files.tmp, text).map_err(|e| fail("cannot write", &files.tmp, e))?;
    if let Err(e) = rename(&files.tmp, &files.json) {
        let _ = std::fs::remove_file(&files.tmp);
        return Err(fail("cannot replace", &files.json, e));
    }
    // The lock is released when `lock` drops.
    Ok(saved)
}

/// The state of one run: what was loaded at the start, and how the saves
/// went. It is the record that explains a first tick or a missing file
/// after the fact (design section 3.4).
pub struct RunState {
    enabled: bool,
    identity: String,
    files: StateFiles,
    /// The markets of the list that keep a memory, with their keys.
    keep: Vec<MarketKey>,
    stale_limit_ms: u64,
    loaded: Loaded,
    /// The last save: what it wrote, or the exact error.
    last: Option<Result<Saved, String>>,
    /// The error text last shown as a note, so a long run says it once.
    noted_error: Option<String>,
}

impl RunState {
    /// Loads the state for a run of `list` (after the flags), or does
    /// nothing when `enabled` is false (`--no-state`). Returns the state and
    /// the `History` the run starts with.
    pub fn start(
        enabled: bool,
        dir: &Path,
        identity: String,
        list: &scope::watch::Watchlist,
        now_ms: u64,
    ) -> (Self, History) {
        let files = files(dir, &list.name, &identity);
        let keep: Vec<MarketKey> = scope::watch::market_keys(&list.targets)
            .into_iter()
            .zip(&list.targets)
            .filter(|(_, t)| t.keeps_history())
            .flat_map(|(keys, _)| keys.into_iter().flatten())
            .collect();
        let interval_ms = u64::try_from(list.interval.as_millis()).unwrap_or(u64::MAX);
        let stale_limit_ms = STALE_AFTER_INTERVALS.saturating_mul(interval_ms);
        // A list with no rule on a since-last-tick metric has no memory to
        // keep: it neither reads nor writes a file.
        let loaded = if enabled && !keep.is_empty() {
            load(&files, now_ms, stale_limit_ms)
        } else {
            Loaded::empty()
        };
        let history = loaded.history.clone();
        let state = Self {
            enabled,
            identity,
            files,
            keep,
            stale_limit_ms,
            loaded,
            last: None,
            noted_error: None,
        };
        for line in state.market_records() {
            tracing::info!(state = %line, "watch state market");
        }
        (state, history)
    }

    fn active(&self) -> bool {
        self.enabled && !self.keep.is_empty()
    }

    /// One record for each market of the list that keeps a memory: what the
    /// run loaded for it.
    fn market_records(&self) -> Vec<serde_json::Value> {
        self.keep
            .iter()
            .map(|key| {
                let hit = self.loaded.restored.iter().find(|r| &r.entry.key == key);
                serde_json::json!({
                    "row": key.to_string(),
                    "basis": hit.map_or("none", |r| r.basis.name()),
                    "age_ms": hit.and_then(|r| r.age_ms),
                    "stale_limit_ms": self.stale_limit_ms,
                    "last_trade_ms": hit.and_then(|r| r.entry.last_trade_ms),
                    "volume_24h": hit.and_then(|r| r.entry.volume_24h),
                })
            })
            .collect()
    }

    /// The `state` field of the `start` record.
    pub fn start_record(&self) -> serde_json::Value {
        let file = if self.loaded.discarded.is_some() {
            "discarded"
        } else if self.loaded.found {
            "loaded"
        } else {
            "none"
        };
        serde_json::json!({
            "mode": if self.enabled { "on" } else { "off" },
            "path": self.active().then(|| self.files.json.display().to_string()),
            "identity": self.identity,
            "file": file,
            "discarded": self.loaded.discarded.as_ref().map(|d| serde_json::json!({
                "reason": d.reason,
                "bad_path": d.bad.as_ref().map(|p| p.display().to_string()),
                "move_error": d.move_error,
            })),
            "stale_limit_ms": self.stale_limit_ms,
            "markets": self.market_records(),
        })
    }

    /// The lines of the start note: what was loaded, and a bad file.
    pub fn start_notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if let Some(d) = &self.loaded.discarded {
            let moved = match (&d.bad, &d.move_error) {
                (Some(bad), _) => format!("Moved it to {}.", bad.display()),
                (None, Some(e)) => format!("It could not be moved: {e}."),
                (None, None) => String::new(),
            };
            notes.push(format!(
                "State file {} is not usable ({}). {moved} The run starts with no state.",
                self.files.json.display(),
                d.reason
            ));
        }
        if !self.enabled {
            notes.push("State: off (--no-state).".to_string());
        } else if self.keep.is_empty() {
            notes
                .push("State: not used (no rule compares a tick with the one before).".to_string());
        } else {
            let count = |b| self.loaded.restored.iter().filter(|r| r.basis == b).count();
            notes.push(format!(
                "State: {} ({} loaded, {} stale, {} new).",
                self.files.json.display(),
                count(scope::watch::Basis::Loaded),
                count(scope::watch::Basis::Stale),
                self.keep.len().saturating_sub(self.loaded.restored.len()),
            ));
        }
        notes
    }

    /// Saves the memory after a tick. A failure is kept for the `end`
    /// record and returned the first time its text appears, as the text
    /// of a note; it never changes the exit code (Q9).
    pub fn save(&mut self, history: &History, now_ms: u64) -> Option<String> {
        if !self.active() {
            return None;
        }
        let result = save(&self.files, &self.identity, history, &self.keep, now_ms);
        let note = match &result {
            Ok(saved) => {
                tracing::debug!(
                    written = saved.written,
                    dropped = saved.dropped,
                    "watch state saved"
                );
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, path = %self.files.json.display(), "watch state not saved");
                (self.noted_error.as_deref() != Some(e.as_str()))
                    .then(|| format!("State not saved ({e}). The exit code does not change."))
            }
        };
        if let Err(e) = &result {
            self.noted_error = Some(e.clone());
        }
        self.last = Some(result);
        note
    }

    /// The fields of the `end` record: whether the last save worked and,
    /// if not, the exact error; and how many entries it wrote and dropped.
    pub fn end_record(&self) -> serde_json::Value {
        let (written, error, entries, dropped) = match &self.last {
            Some(Ok(s)) => (true, None, Some(s.written), Some(s.dropped)),
            Some(Err(e)) => (false, Some(e.clone()), None, None),
            None => (false, None, None, None),
        };
        serde_json::json!({
            "state_written": written,
            "state_error": error,
            "state_entries_written": entries,
            "state_entries_dropped": dropped,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope::watch::Basis;

    const MIN: u64 = 60_000;

    fn key(id: &str, venue: &str) -> MarketKey {
        MarketKey {
            id: id.into(),
            chain: None,
            venue: venue.into(),
            ordinal: 1,
        }
    }

    fn entry(id: &str, last_trade_ms: u64, updated_ms: u64) -> Entry {
        Entry {
            key: key(id, "weex"),
            last_trade_ms: Some(last_trade_ms),
            volume_24h: Some(1_000.0),
            updated_ms,
        }
    }

    /// A history that holds `entries` (as a restored one does).
    fn history(entries: Vec<Entry>) -> History {
        History::restore(entries, u64::MAX, u64::MAX).0
    }

    fn setup() -> (tempfile::TempDir, StateFiles) {
        let dir = tempfile::tempdir().unwrap();
        let f = files(dir.path(), "pegs", "saved:pegs");
        (dir, f)
    }

    fn disk(f: &StateFiles) -> StateFile {
        serde_json::from_slice(&std::fs::read(&f.json).unwrap()).unwrap()
    }

    /// Why: a change of the format must bump `version`, and the golden text
    /// makes a change visible. Save, then load, gives the same entries.
    #[test]
    fn the_file_round_trips_and_its_keys_are_those_of_version_1() {
        let (_dir, f) = setup();
        let mut e = entry("USDN", 1_788_999_990_000, 1_789_000_000_000);
        e.key.chain = Some("base".into());
        let keep = vec![e.key.clone()];
        let saved = save(
            &f,
            "saved:pegs",
            &history(vec![e.clone()]),
            &keep,
            1_789_000_000_000,
        );
        assert_eq!(
            saved,
            Ok(Saved {
                written: 1,
                dropped: 0
            })
        );

        let text = std::fs::read_to_string(&f.json).unwrap();
        let golden = format!(
            r#"{{
  "version": 1,
  "identity": "saved:pegs",
  "scope_version": "{}",
  "written_at_ms": 1789000000000,
  "markets": [
    {{
      "id": "USDN",
      "chain": "base",
      "venue": "weex",
      "ordinal": 1,
      "last_trade_ms": 1788999990000,
      "volume_24h": 1000.0,
      "updated_ms": 1789000000000
    }}
  ]
}}"#,
            scope::VERSION
        );
        assert_eq!(text, golden);

        let loaded = load(&f, 1_789_000_000_000, 3 * MIN);
        assert!(loaded.found && loaded.discarded.is_none());
        assert_eq!(loaded.history.entries(), vec![e]);
        assert_eq!(loaded.restored[0].basis, Basis::Loaded);
    }

    /// Why: no file is a first tick and not an error.
    #[test]
    fn no_file_is_a_first_tick() {
        let (_dir, f) = setup();
        let loaded = load(&f, 0, MIN);
        assert!(!loaded.found && loaded.discarded.is_none());
        assert!(loaded.history.entries().is_empty());
    }

    /// Why: a crash must not lose the memory. A failure after the
    /// temporary file exists leaves the old file whole, and no temporary
    /// file behind.
    #[test]
    fn a_failed_replace_leaves_the_old_file_whole() {
        let (_dir, f) = setup();
        let keep = vec![key("USDN", "weex")];
        save(
            &f,
            "saved:pegs",
            &history(vec![entry("USDN", 1, 10)]),
            &keep,
            10,
        )
        .unwrap();
        let before = std::fs::read(&f.json).unwrap();

        let fail = |_: &Path, _: &Path| Err(io::Error::other("disk full"));
        let result = save_with(
            &f,
            "saved:pegs",
            &history(vec![entry("USDN", 2, 20)]),
            &keep,
            20,
            &fail,
        );
        let error = result.unwrap_err();
        assert!(error.contains("disk full"), "the exact cause: {error}");
        assert_eq!(std::fs::read(&f.json).unwrap(), before);
        assert!(!f.tmp.exists(), "the temporary file is removed");
    }

    /// Why: a slow run that ends late must not step the memory back. Both
    /// orders give the newer entry, and `last_trade_ms` never goes down.
    #[test]
    fn merging_keeps_the_newer_entry_and_never_lowers_last_trade_ms() {
        let newer = entry("USDN", 500, 200);
        let mut older = entry("USDN", 900, 100);
        older.volume_24h = Some(1.0);
        for (first, second) in [(&newer, &older), (&older, &newer)] {
            let (_dir, f) = setup();
            let keep = vec![key("USDN", "weex")];
            save(&f, "saved:pegs", &history(vec![first.clone()]), &keep, 300).unwrap();
            save(&f, "saved:pegs", &history(vec![second.clone()]), &keep, 301).unwrap();
            let [got] = disk(&f).markets.try_into().unwrap();
            assert_eq!(got.updated_ms, 200, "the newer entry wins");
            assert_eq!(got.volume_24h, Some(1_000.0));
            assert_eq!(got.last_trade_ms, Some(900), "the larger trade time stays");
        }
    }

    /// Why: the lock and the merge are what two overlapping cron runs need.
    /// Writers on separate threads (the lock is per open file, as it is per
    /// process) leave a valid file, and every market has its largest
    /// `last_trade_ms`.
    #[test]
    fn writers_at_the_same_time_leave_a_valid_file_with_the_largest_values() {
        let (_dir, f) = setup();
        let ids = ["A", "B", "C", "D"];
        let keep: Vec<MarketKey> = ids.iter().map(|id| key(id, "weex")).collect();
        std::thread::scope(|s| {
            for writer in 1..=8u64 {
                let (f, keep) = (&f, &keep);
                s.spawn(move || {
                    for round in 0..5u64 {
                        let entries = ids
                            .iter()
                            .map(|id| entry(id, writer * 1_000 + round, writer * 10 + round))
                            .collect();
                        save(f, "saved:pegs", &history(entries), keep, 1).unwrap();
                    }
                });
            }
        });
        let file = disk(&f);
        assert_eq!(file.markets.len(), ids.len());
        for m in &file.markets {
            assert_eq!(m.last_trade_ms, Some(8_004), "{m:?}");
            assert_eq!(m.updated_ms, 84, "{m:?}");
        }
    }

    /// Why: a bad cache must not stop a watch, and the bytes are the
    /// evidence. Garbage, a truncated file and another version each move
    /// aside with their exact bytes and run as a first tick.
    #[test]
    fn a_bad_file_is_moved_aside_with_its_bytes_and_the_run_starts_clean() {
        let good = {
            let (_dir, f) = setup();
            let keep = vec![key("USDN", "weex")];
            save(
                &f,
                "saved:pegs",
                &history(vec![entry("USDN", 1, 10)]),
                &keep,
                10,
            )
            .unwrap();
            std::fs::read_to_string(&f.json).unwrap()
        };
        let cases: Vec<(&str, Vec<u8>, &str)> = vec![
            (
                "garbage",
                b"\x00not json at all".to_vec(),
                "not a valid state file",
            ),
            (
                "truncated",
                good.as_bytes()[..good.len() / 2].to_vec(),
                "not a valid state file",
            ),
            (
                "version",
                good.replace("\"version\": 1", "\"version\": 2")
                    .into_bytes(),
                "version 2 is not 1",
            ),
        ];
        for (name, bytes, reason) in cases {
            let (_dir, f) = setup();
            std::fs::create_dir_all(f.json.parent().unwrap()).unwrap();
            // An older .bad copy is replaced.
            std::fs::write(&f.bad, b"older").unwrap();
            std::fs::write(&f.json, &bytes).unwrap();
            let loaded = load(&f, 10, MIN);
            let d = loaded
                .discarded
                .unwrap_or_else(|| panic!("{name}: not discarded"));
            assert!(d.reason.contains(reason), "{name}: {}", d.reason);
            assert_eq!(d.bad.as_deref(), Some(f.bad.as_path()), "{name}");
            assert_eq!(std::fs::read(&f.bad).unwrap(), bytes, "{name}: the bytes");
            assert!(!f.json.exists(), "{name}");
            assert!(
                !loaded.found && loaded.history.entries().is_empty(),
                "{name}"
            );
        }
    }

    /// Why: a state file that is bad must not lose a good write later. The
    /// next save treats it as empty and writes a valid file.
    #[test]
    fn a_save_over_an_invalid_file_writes_a_valid_one() {
        let (_dir, f) = setup();
        std::fs::create_dir_all(f.json.parent().unwrap()).unwrap();
        std::fs::write(&f.json, b"{").unwrap();
        let keep = vec![key("USDN", "weex")];
        save(
            &f,
            "saved:pegs",
            &history(vec![entry("USDN", 1, 10)]),
            &keep,
            10,
        )
        .unwrap();
        assert_eq!(disk(&f).markets.len(), 1);
    }

    /// Why: shared memory between two different jobs would give wrong
    /// readings (a job every 5 minutes and one every hour would take turns
    /// writing `last_trade_ms`). The saved name is the same from any
    /// directory, and a path is made absolute.
    #[test]
    fn the_identity_is_the_saved_name_or_the_absolute_path() {
        let path = Path::new("pegs");
        assert_eq!(identity("pegs", path), "saved:pegs");
        let here = std::path::absolute("./pegs.yaml").unwrap();
        assert_eq!(
            identity("./pegs.yaml", Path::new("./pegs.yaml")),
            format!("path:{}", here.display())
        );
        assert_eq!(
            identity("./pegs.yaml", &here),
            identity("./pegs.yaml", Path::new("./pegs.yaml")),
            "a relative and an absolute spelling of one file are one identity"
        );
        let dir = Path::new("state");
        let saved = files(dir, "pegs", &identity("pegs", path));
        let by_path = files(dir, "pegs", &identity("./pegs.yaml", &here));
        assert_ne!(saved.json, by_path.json);
        assert_ne!(
            files(dir, "pegs", "path:/a/pegs.yaml").json,
            files(dir, "pegs", "path:/b/pegs.yaml").json,
            "two lists with one name in two folders have two files"
        );
    }

    /// Why: shared memory between two inline jobs with different rules or
    /// markets would give wrong readings. The identity is the definition.
    #[test]
    fn an_inline_identity_is_its_definition() {
        let a = "name: cli-USDC\ntargets:\n- id: USDC\n  rules:\n  - mid_price > 0\n";
        let b = a.replace("mid_price > 0", "mid_price > 1");
        assert_eq!(inline_identity(a), inline_identity(a));
        assert_ne!(inline_identity(a), inline_identity(&b));
        assert!(inline_identity(a).starts_with("inline:"));
        let dir = Path::new("d");
        assert_ne!(
            files(dir, "cli-USDC", &inline_identity(a)).json,
            files(dir, "cli-USDC", &inline_identity(&b)).json
        );
    }

    #[test]
    fn the_file_name_has_a_slug_for_people_and_a_hash_for_uniqueness() {
        let f = files(Path::new("d"), "my pegs/1", "saved:my pegs/1");
        let name = f.json.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("my-pegs-1-"), "{name}");
        assert!(name.ends_with(".json"), "{name}");
        assert_eq!(name.len(), "my-pegs-1-".len() + HASH_CHARS + ".json".len());
        assert_eq!(f.lock.extension().unwrap(), "lock");
    }

    /// Why (design 3.2.7): the entries that remain after a change of the
    /// list. A market that the list no longer has, or that has no rule on a
    /// since-last-tick metric, is dropped at the next write. The memory
    /// holds market data, so a rule or a threshold change needs nothing.
    #[test]
    fn a_write_drops_the_markets_that_the_list_no_longer_has() {
        let (_dir, f) = setup();
        let all = vec![key("A", "weex"), key("B", "weex"), key("C", "weex")];
        let entries = vec![entry("A", 1, 10), entry("B", 1, 10), entry("C", 1, 10)];
        let saved = save(&f, "saved:pegs", &history(entries), &all, 10).unwrap();
        assert_eq!(
            saved,
            Saved {
                written: 3,
                dropped: 0
            }
        );
        // B was removed from the list. An empty history writes on.
        let keep = vec![key("A", "weex"), key("C", "weex")];
        let saved = save(&f, "saved:pegs", &History::default(), &keep, 20).unwrap();
        assert_eq!(
            saved,
            Saved {
                written: 2,
                dropped: 1
            }
        );
        let ids: Vec<_> = disk(&f).markets.into_iter().map(|m| m.key.id).collect();
        assert_eq!(ids, ["A", "C"]);
    }

    /// Why: the state is a cache; a failure to write it is an error text
    /// for the caller, never a panic.
    #[test]
    fn an_unwritable_folder_is_an_error_with_the_cause() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("state");
        std::fs::write(&blocker, b"a file where the folder should be").unwrap();
        let f = files(&blocker, "pegs", "saved:pegs");
        let error = save(&f, "saved:pegs", &History::default(), &[], 1).unwrap_err();
        assert!(error.contains("cannot create"), "{error}");
    }
}
