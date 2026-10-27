//! Copy the user's files from the old locations into the scope home, once.
//!
//! Old versions kept the config, venues and address book in
//! `~/.config/scope` (or the platform config folder), and tokens and TUI
//! state in the platform data folder. A start copies every old file whose
//! new copy does not exist. Nothing old is deleted or changed, and no new
//! file is written over. Each copy, and each failed copy, gets one line in
//! `state/migration.jsonl` (the audit record).
//!
//! It happens once: scope has one home. When a run that copied files
//! finishes, the log gets a `complete` line, and no later start reads the
//! old folders again, so a copy the user deleted stays deleted and a file
//! added to an old folder is ignored. A log from before that line existed
//! still stops a file it shows as copied from being copied back. A start
//! with nothing to copy writes nothing, so a read-only home works.

use crate::error::{Result, ScopeError};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// One old file and its new place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// What happened to one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Copied,
    KeptExisting,
    NoSource,
    /// Copied by an earlier start: the user may have deleted the copy.
    CopiedBefore,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Copied => "copied",
            Outcome::KeptExisting => "kept-existing",
            Outcome::NoSource => "no-source",
            Outcome::CopiedBefore => "copied-before",
        }
    }
}

/// The result of a migration run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub copied: usize,
    /// The old folders the copied files came from, each once, in order.
    pub from: Vec<PathBuf>,
}

/// Files in the old platform data folder (`<data>/scope/`) and where they go.
const DATA_FILES: [(&str, &str); 4] = [
    ("tokens.yaml", "data/tokens.yaml"),
    ("session.yaml", "state/session.yaml"),
    ("history.txt", "state/history.txt"),
    ("tui-events.jsonl", "state/tui-events.jsonl"),
];

/// The real old-to-new list for this machine.
pub fn legacy_items() -> Vec<Item> {
    let xdg = super::user_home().map(|h| h.join(".config").join("scope"));
    items_for(
        xdg.as_deref(),
        super::legacy_config_dir().as_deref(),
        super::legacy_data_dir().as_deref(),
        &super::home(),
    )
}

/// The old-to-new list for the given old roots and new home.
/// `xdg_root` is `~/.config/scope`; the platform roots are the folders
/// above their `scope` subfolder.
pub fn items_for(
    xdg_root: Option<&Path>,
    platform_config: Option<&Path>,
    platform_data: Option<&Path>,
    home: &Path,
) -> Vec<Item> {
    let mut items = Vec::new();
    // The config: ~/.config/scope first, then the platform folder, as old
    // versions read it.
    let config_from = [
        xdg_root.map(|r| r.join("config.yaml")),
        platform_config.map(|r| r.join("scope").join("config.yaml")),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.exists())
    .or_else(|| xdg_root.map(|r| r.join("config.yaml")));
    if let Some(from) = config_from {
        items.push(Item {
            from,
            to: home.join("config.yaml"),
        });
    }
    if let Some(xdg) = xdg_root {
        items.push(Item {
            from: xdg.join("data").join("address_book.yaml"),
            to: home.join("data").join("address_book.yaml"),
        });
        if let Ok(entries) = std::fs::read_dir(xdg.join("venues")) {
            let mut venues: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "yaml" || e == "yml"))
                .collect();
            venues.sort();
            for from in venues {
                let to = home
                    .join("venues")
                    .join(from.file_name().unwrap_or_default());
                items.push(Item { from, to });
            }
        }
    }
    if let Some(data) = platform_data {
        for (old, new) in DATA_FILES {
            items.push(Item {
                from: data.join("scope").join(old),
                to: home.join(new),
            });
        }
    }
    items
}

/// Copies each item whose source exists, whose destination does not, and
/// that `log` does not show as copied already, logging each copy and each
/// failure to `log`. Stops at the first failed copy with an error that
/// names the file; the files copied before it stay.
pub fn run(items: &[Item], log: &Path) -> Result<Report> {
    let entries = log_entries(log);
    if entries.iter().any(|v| v["result"] == COMPLETE) {
        return Ok(Report {
            copied: 0,
            from: Vec::new(),
        });
    }
    let done = copied_before(&entries);
    let mut copied = 0;
    let mut from: Vec<PathBuf> = Vec::new();
    for item in items {
        let result = if done.contains(&(item.from.clone(), item.to.clone())) {
            Ok(Outcome::CopiedBefore)
        } else {
            copy_one(item)
        };
        match &result {
            Ok(Outcome::Copied) => append_log(log, item, Outcome::Copied.as_str())?,
            Ok(_) => {}
            Err(e) => append_log(log, item, &format!("failed: {e}"))?,
        }
        match result {
            Ok(Outcome::Copied) => {
                copied += 1;
                let dir = item
                    .from
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default();
                if !from.contains(&dir) {
                    from.push(dir);
                }
            }
            Ok(_) => {}
            Err(e) => {
                return Err(ScopeError::io_context(
                    e,
                    format!(
                        "cannot copy {} to {}",
                        item.from.display(),
                        item.to.display()
                    ),
                ));
            }
        }
    }
    // Done once anything was ever copied: the old folders are not read
    // again. With nothing copied, nothing is written (a read-only home).
    if copied > 0 || !done.is_empty() {
        let end = serde_json::json!({
            "at": chrono::Utc::now().to_rfc3339(),
            "result": COMPLETE,
        });
        append_line(log, &end)?;
    }
    Ok(Report { copied, from })
}

/// The result of the log line that ends a migration.
const COMPLETE: &str = "complete";

/// The log's entries; an unreadable log or line has none.
fn log_entries(log: &Path) -> Vec<serde_json::Value> {
    let Ok(text) = std::fs::read_to_string(log) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect()
}

/// The (from, to) pairs `entries` record as copied. The worst case of a
/// lost log is the old behaviour, a copy made again.
fn copied_before(entries: &[serde_json::Value]) -> std::collections::HashSet<(PathBuf, PathBuf)> {
    entries
        .iter()
        .filter(|v| v["result"] == Outcome::Copied.as_str())
        .filter_map(|v| {
            Some((
                PathBuf::from(v["from"].as_str()?),
                PathBuf::from(v["to"].as_str()?),
            ))
        })
        .collect()
}

fn copy_one(item: &Item) -> std::io::Result<Outcome> {
    if !item.from.exists() {
        return Ok(Outcome::NoSource);
    }
    if item.to.exists() {
        return Ok(Outcome::KeptExisting);
    }
    if let Some(parent) = item.to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Copy to a temporary file and rename it: a copy that stops partway
    // (disk full, a kill) must not become the new file, which later starts
    // would keep. A temporary file left by such a copy is replaced.
    let mut name = item.to.file_name().unwrap_or_default().to_os_string();
    name.push(".migrating");
    let temp = item.to.with_file_name(name);
    std::fs::copy(&item.from, &temp)?;
    std::fs::rename(&temp, &item.to)?;
    Ok(Outcome::Copied)
}

fn append_log(log: &Path, item: &Item, result: &str) -> Result<()> {
    let line = serde_json::json!({
        "at": chrono::Utc::now().to_rfc3339(),
        "from": item.from.display().to_string(),
        "to": item.to.display().to_string(),
        "result": result,
    });
    append_line(log, &line)
}

fn append_line(log: &Path, line: &serde_json::Value) -> Result<()> {
    let context = || format!("cannot write the migration log {}", log.display());
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ScopeError::io_context(e, context()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|e| ScopeError::io_context(e, context()))?;
    writeln!(file, "{line}").map_err(|e| ScopeError::io_context(e, context()))?;
    Ok(())
}

/// Migrates this machine's old files into the scope home.
pub fn migrate() -> Result<Report> {
    run(&legacy_items(), &super::state_dir().join("migration.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Old folders and a new home under one temp folder.
    struct Fixture {
        _tmp: tempfile::TempDir,
        xdg: PathBuf,
        platform_config: PathBuf,
        platform_data: PathBuf,
        home: PathBuf,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().to_path_buf();
        Fixture {
            xdg: p.join("xdg/scope"),
            platform_config: p.join("platform-config"),
            platform_data: p.join("platform-data"),
            home: p.join("home/.scope-bca"),
            _tmp: tmp,
        }
    }

    fn put(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn items(f: &Fixture) -> Vec<Item> {
        items_for(
            Some(&f.xdg),
            Some(&f.platform_config),
            Some(&f.platform_data),
            &f.home,
        )
    }

    fn log_lines(f: &Fixture) -> Vec<serde_json::Value> {
        fs::read_to_string(f.home.join("state/migration.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn run_fixture(f: &Fixture) -> crate::error::Result<Report> {
        run(&items(f), &f.home.join("state/migration.jsonl"))
    }

    #[test]
    fn test_copies_every_old_file_to_its_new_place_and_leaves_the_old() {
        let f = fixture();
        put(&f.xdg.join("config.yaml"), "chains: {}\n");
        put(&f.xdg.join("venues/weex.yaml"), "id: weex\n");
        put(&f.xdg.join("data/address_book.yaml"), "addresses: []\n");
        put(&f.platform_data.join("scope/tokens.yaml"), "aliases: {}\n");
        put(&f.platform_data.join("scope/session.yaml"), "chain: auto\n");
        put(&f.platform_data.join("scope/history.txt"), "address 0x1\n");
        put(&f.platform_data.join("scope/tui-events.jsonl"), "{}\n");

        let report = run_fixture(&f).unwrap();

        assert_eq!(report.copied, 7);
        for (new, body) in [
            ("config.yaml", "chains: {}\n"),
            ("venues/weex.yaml", "id: weex\n"),
            ("data/address_book.yaml", "addresses: []\n"),
            ("data/tokens.yaml", "aliases: {}\n"),
            ("state/session.yaml", "chain: auto\n"),
            ("state/history.txt", "address 0x1\n"),
            ("state/tui-events.jsonl", "{}\n"),
        ] {
            assert_eq!(fs::read_to_string(f.home.join(new)).unwrap(), body, "{new}");
        }
        // The old files are unchanged.
        assert_eq!(
            fs::read_to_string(f.xdg.join("config.yaml")).unwrap(),
            "chains: {}\n"
        );
        assert!(f.platform_data.join("scope/tokens.yaml").exists());
    }

    #[test]
    fn test_config_comes_from_the_platform_folder_when_xdg_has_none() {
        // Review focus 1: old versions read the platform config folder too.
        let f = fixture();
        put(&f.platform_config.join("scope/config.yaml"), "output: {}\n");
        run_fixture(&f).unwrap();
        assert_eq!(
            fs::read_to_string(f.home.join("config.yaml")).unwrap(),
            "output: {}\n"
        );
    }

    #[test]
    fn test_an_existing_new_file_is_never_overwritten() {
        // Review focus 2.
        let f = fixture();
        put(&f.xdg.join("config.yaml"), "old\n");
        put(&f.home.join("config.yaml"), "new\n");
        let report = run_fixture(&f).unwrap();
        assert_eq!(report.copied, 0);
        assert_eq!(
            fs::read_to_string(f.home.join("config.yaml")).unwrap(),
            "new\n"
        );
    }

    #[test]
    fn test_a_second_run_copies_nothing() {
        let f = fixture();
        put(&f.xdg.join("config.yaml"), "chains: {}\n");
        assert_eq!(run_fixture(&f).unwrap().copied, 1);
        assert_eq!(run_fixture(&f).unwrap().copied, 0);
    }

    #[test]
    fn test_it_happens_once_so_the_old_folders_never_compete() {
        let f = fixture();
        put(
            &f.xdg.join("venues/weex.yaml"),
            "id: weex
",
        );
        let report = run_fixture(&f).unwrap();
        assert_eq!(report.copied, 1);
        // The report names where the copies came from.
        assert_eq!(report.from, [f.xdg.join("venues")]);
        // A deleted copy stays deleted, and a file added to the old
        // folder later is not read.
        fs::remove_file(f.home.join("venues/weex.yaml")).unwrap();
        put(
            &f.xdg.join("venues/kucoin.yaml"),
            "id: kucoin
",
        );
        assert_eq!(run_fixture(&f).unwrap().copied, 0);
        assert!(!f.home.join("venues/weex.yaml").exists());
        assert!(!f.home.join("venues/kucoin.yaml").exists());
    }

    #[test]
    fn test_a_log_from_before_complete_stops_copies_coming_back_and_ends() {
        let f = fixture();
        put(
            &f.xdg.join("venues/weex.yaml"),
            "id: weex
",
        );
        let item = Item {
            from: f.xdg.join("venues/weex.yaml"),
            to: f.home.join("venues/weex.yaml"),
        };
        // 0.8.4 logged the copy but no end; the user then deleted it.
        append_log(&f.home.join("state/migration.jsonl"), &item, "copied").unwrap();
        assert_eq!(run_fixture(&f).unwrap().copied, 0);
        assert!(!item.to.exists());
        let last = log_lines(&f).pop().unwrap();
        assert_eq!(last["result"], COMPLETE);
    }

    #[test]
    fn test_the_log_records_each_copy_and_nothing_else() {
        // The audit record: one line per copied (or failed) file.
        let f = fixture();
        put(&f.xdg.join("config.yaml"), "chains: {}\n");
        put(&f.platform_data.join("scope/tokens.yaml"), "aliases: {}\n");
        run_fixture(&f).unwrap();
        let mut lines = log_lines(&f);
        assert_eq!(lines.len(), 3, "{lines:?}");
        // The last line ends the migration.
        assert_eq!(lines.pop().unwrap()["result"], COMPLETE);
        for line in &lines {
            assert!(line["at"].is_string() && line["from"].is_string() && line["to"].is_string());
            assert_eq!(line["result"], "copied");
        }
        run_fixture(&f).unwrap();
        assert_eq!(log_lines(&f).len(), 3, "a later start adds no line");
    }

    #[cfg(unix)]
    #[test]
    fn test_nothing_to_copy_writes_nothing_so_a_read_only_home_works() {
        // Review C1/I2: with nothing to copy, a start must not write at
        // all (a read-only home ran every command before this change) and
        // must not grow the log.
        use std::os::unix::fs::PermissionsExt;
        let f = fixture();
        put(&f.home.join("config.yaml"), "chains: {}\n");
        fs::set_permissions(&f.home, fs::Permissions::from_mode(0o555)).unwrap();
        let result = run_fixture(&f);
        fs::set_permissions(&f.home, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(result.unwrap().copied, 0);
        assert!(
            !f.home.join("state").exists(),
            "no log when nothing happened"
        );
    }

    #[test]
    fn test_a_copy_left_by_a_crash_is_replaced_not_kept() {
        // Review I1: a copy that stopped partway must not become the new
        // file. Copies go through a temporary file and a rename, and a
        // temporary file left by a crash is replaced.
        let f = fixture();
        put(&f.xdg.join("config.yaml"), "chains: {}\n");
        let leftover = f.home.join("config.yaml.migrating");
        put(&leftover, "chai");
        assert_eq!(run_fixture(&f).unwrap().copied, 1);
        assert_eq!(
            fs::read_to_string(f.home.join("config.yaml")).unwrap(),
            "chains: {}\n"
        );
        assert!(!leftover.exists(), "the temporary file is gone");
    }

    #[test]
    fn test_a_failed_copy_stops_names_the_file_and_a_rerun_finishes() {
        // Review focus 3: a source that cannot be read. A folder in its
        // place cannot be copied, also as root, where mode 000 does not
        // stop a read (#106), and on Windows.
        let f = fixture();
        put(&f.xdg.join("config.yaml"), "chains: {}\n");
        let tokens = f.platform_data.join("scope/tokens.yaml");
        fs::create_dir_all(&tokens).unwrap();

        let err = run_fixture(&f).unwrap_err().to_string();
        assert!(err.contains("tokens.yaml"), "{err}");
        // The file copied before the failure stays.
        assert!(f.home.join("config.yaml").exists());
        assert!(
            log_lines(&f)
                .iter()
                .any(|l| l["result"].as_str().unwrap().starts_with("failed:"))
        );

        fs::remove_dir(&tokens).unwrap();
        put(&tokens, "aliases: {}\n");
        let report = run_fixture(&f).unwrap();
        assert_eq!(report.copied, 1, "the rerun copies only what is left");
        assert!(f.home.join("data/tokens.yaml").exists());
    }

    #[test]
    fn test_only_yaml_venue_files_are_copied() {
        // Review focus 5.
        let f = fixture();
        put(&f.xdg.join("venues/weex.yaml"), "id: weex\n");
        put(&f.xdg.join("venues/kraken.yml"), "id: kraken\n");
        put(&f.xdg.join("venues/README.txt"), "notes\n");
        put(&f.xdg.join("venues/weex.yaml.bak"), "old\n");
        run_fixture(&f).unwrap();
        assert!(f.home.join("venues/weex.yaml").exists());
        assert!(f.home.join("venues/kraken.yml").exists());
        assert!(!f.home.join("venues/README.txt").exists());
        assert!(!f.home.join("venues/weex.yaml.bak").exists());
    }

    #[test]
    fn test_portfolio_and_daemon_files_are_not_migrated() {
        let f = fixture();
        for name in ["portfolio.yaml", "scope-web.pid", "scope-web.log"] {
            put(&f.platform_data.join("scope").join(name), "x\n");
        }
        assert_eq!(run_fixture(&f).unwrap().copied, 0);
    }

    #[test]
    fn test_real_items_point_into_the_sandbox_in_tests() {
        // legacy_items() must never name a real user file in a test build.
        let temp = std::env::temp_dir();
        for item in legacy_items() {
            assert!(item.from.starts_with(&temp), "{}", item.from.display());
            assert!(item.to.starts_with(&temp), "{}", item.to.display());
        }
    }
}
