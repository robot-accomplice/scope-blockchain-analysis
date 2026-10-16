# One home for scope (`~/.scope-bca`): implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep every scope user file under one folder (`$SCOPE_BCA_HOME`, else `~/.scope-bca`), and copy the user's existing files there once without changing the old files.

**Architecture:** `scope::paths` is the only module that knows the layout. It gets named accessors (`home`, `config_file`, `venues_dir`, `data_dir`, `state_dir`, `user_home`) that return `PathBuf`. The return type changes from `Option<PathBuf>`, so the compiler finds every old caller. A new module `scope::paths::migration` copies the old files and writes an audit log. The `scope` binary calls it before it loads the config.

**Tech Stack:** Rust 2024 workspace (`scope-core` = `scope-bca-core`, `scope-cli`, `scope-web` holds the binary, `scope-tui-harness`), `dirs`, `serde_json`, `chrono`, `tempfile` for tests.

**Spec:** `docs/design/2026-10-15-scope-bca-home-design.md`

## Global Constraints

- The root is `$SCOPE_BCA_HOME` when it is set and not empty, else `~/.scope-bca`.
- Layout: `config.yaml`, `venues/`, `data/` (`address_book.yaml`, `tokens.yaml`), `state/` (`session.yaml`, `history.txt`, `tui-events.jsonl`, `migration.jsonl`, `scope-web.pid`, `scope-web.log`).
- Config file order: `--config <PATH>`, then `$SCOPE_CONFIG`, then `<root>/config.yaml`.
- `address_book.data_dir` in the config still overrides `<root>/data` for the address book.
- In test builds (`cfg(test)` or feature `test-support`), every path is in the sandbox from #98 (`SCOPE_TEST_HOME`, else `<temp>/scope-test-home-<pid>`). `SCOPE_BCA_HOME` is ignored in test builds.
- The migration never deletes or changes an old file and never writes over a new file.
- `portfolio.yaml`, `scope-web.pid` and `scope-web.log` are not migrated.
- `clippy.toml` keeps the ban on `dirs::*`. Only `scope::paths` calls `dirs` (with `#[allow(clippy::disallowed_methods)]`).
- Commits: identity robot-accomplice <robot@accomplice.ch>, never jon-outrider. Branch from `develop`, PR into `develop`.
- Safety: never run the test suite of a checkout older than PR #98 on this machine. Before and after every full test run, compare a snapshot of `~/.config/scope`, `~/Library/Application Support/scope` and `~/.scope-bca`. A test run must write none of them.

## Review Focus

1. The old config is only in the platform config folder (`~/Library/Application Support/scope/config.yaml`), not in `~/.config/scope`. Expected: the migration copies it. Pinned in Task 2, `test_config_comes_from_the_platform_folder_when_xdg_has_none`.
2. Both the old and the new file exist. Expected: the new file stays, and the log says `kept-existing`. Pinned in Task 2, `test_an_existing_new_file_is_never_overwritten`.
3. A copy fails partway (one source cannot be read). Expected: an error that names the file; the files copied before it stay; the next run copies the rest. Pinned in Task 2, `test_a_failed_copy_stops_names_the_file_and_a_rerun_finishes`.
4. `SCOPE_BCA_HOME` is set to an empty string. Expected: it is ignored, and the root is `~/.scope-bca`. Pinned in Task 1, `test_home_from_ignores_an_empty_override`.
5. The old `venues/` folder has non-YAML files (for example `README.txt` or a `.bak`). Expected: only `*.yaml` and `*.yml` are copied. Pinned in Task 2, `test_only_yaml_venue_files_are_copied`.

---

### Task 1: The layout in `scope::paths`, and every caller on it

**Files:**
- Modify: `crates/scope-core/src/paths.rs` (whole file)
- Modify: `crates/scope-core/src/config.rs:318-372` (`default_path`, `config_path`, `default_data_dir`), and its tests near lines 861-900 and 1014
- Modify: `crates/scope-core/src/market/registry.rs:139-145` (`user_venues_dir`)
- Modify: `crates/scope-core/src/tokens.rs:91-94` (`aliases_path`)
- Modify: `crates/scope-cli/src/cli/interactive.rs:127-130` (`context_path`)
- Modify: `crates/scope-cli/src/cli/tui/journal.rs:54-61` (`in_data_dir`) and the test at lines 220-227
- Modify: `crates/scope-cli/src/cli/tui/crash.rs:88`
- Modify: `crates/scope-cli/src/cli/tui/app.rs:1242`
- Modify: `crates/scope-web/src/lib.rs:143-150` (`pid_file_path`, `log_file_path`)

**Interfaces:**
- Produces (all in `scope::paths`, all `pub`):
  - `pub const HOME_VAR: &str = "SCOPE_BCA_HOME";`
  - `pub const HOME_DIR_NAME: &str = ".scope-bca";`
  - `pub fn home() -> PathBuf`
  - `pub fn user_home() -> Option<PathBuf>`
  - `pub fn config_file() -> PathBuf` = `home()/config.yaml`
  - `pub fn venues_dir() -> PathBuf` = `home()/venues`
  - `pub fn data_dir() -> PathBuf` = `home()/data`
  - `pub fn state_dir() -> PathBuf` = `home()/state`
  - `pub fn home_from(override_root: Option<std::ffi::OsString>, user_home: Option<PathBuf>) -> PathBuf`
  - `pub(crate) fn legacy_config_dir() -> Option<PathBuf>` (platform config folder, or the sandbox `legacy-config` in tests)
  - `pub(crate) fn legacy_data_dir() -> Option<PathBuf>` (platform data folder, or the sandbox `legacy-data` in tests)
- Removes: `pub fn home_dir()`, `pub fn config_dir()`, and the old `pub fn data_dir() -> Option<PathBuf>`.

- [ ] **Step 1: Write the failing tests in `crates/scope-core/src/paths.rs`**

Replace the `#[cfg(test)] mod tests` block at the end of the file with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_test_builds_never_resolve_the_real_home() {
        // The guard against tests writing the user's files: every
        // directory is in the sandbox under the temp directory.
        let temp = std::env::temp_dir();
        let mut all = vec![home(), config_file(), venues_dir(), data_dir(), state_dir()];
        all.extend(user_home());
        all.extend(legacy_config_dir());
        all.extend(legacy_data_dir());
        for dir in all {
            assert!(dir.starts_with(&temp), "{} is outside the sandbox", dir.display());
        }
    }

    #[test]
    fn test_the_layout_is_under_one_home() {
        let root = home();
        assert_eq!(config_file(), root.join("config.yaml"));
        assert_eq!(venues_dir(), root.join("venues"));
        assert_eq!(data_dir(), root.join("data"));
        assert_eq!(state_dir(), root.join("state"));
    }

    #[test]
    fn test_home_from_uses_the_override_else_the_user_home() {
        let user = Some(PathBuf::from("/u/jon"));
        assert_eq!(home_from(None, user.clone()), PathBuf::from("/u/jon/.scope-bca"));
        assert_eq!(
            home_from(Some(OsString::from("/profiles/two")), user),
            PathBuf::from("/profiles/two")
        );
    }

    #[test]
    fn test_home_from_ignores_an_empty_override() {
        // Review focus 4: `SCOPE_BCA_HOME=` must not make the root ".".
        let user = Some(PathBuf::from("/u/jon"));
        assert_eq!(home_from(Some(OsString::new()), user), PathBuf::from("/u/jon/.scope-bca"));
    }

    #[test]
    fn test_home_from_without_a_user_home_is_relative() {
        assert_eq!(home_from(None, None), PathBuf::from(HOME_DIR_NAME));
    }
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -p scope-bca-core --lib paths::`
Expected: compile errors (`home`, `config_file`, `venues_dir`, `state_dir`, `user_home`, `home_from`, `legacy_config_dir` and `legacy_data_dir` are not defined).

- [ ] **Step 3: Write `crates/scope-core/src/paths.rs` above the tests**

Replace everything above `#[cfg(test)]` with:

```rust
//! Where scope keeps the user's files: one folder, `$SCOPE_BCA_HOME`, else
//! `~/.scope-bca`.
//!
//! ```text
//! <home>/config.yaml   venues/   data/ (address book, tokens)
//!        state/ (session, history, tui-events, migration log, daemon files)
//! ```
//!
//! This module is the only place that knows the layout. Code calls these
//! functions instead of `dirs::*` (clippy's `disallowed-methods` enforces
//! this).
//!
//! In test builds (`cfg(test)` or the `test-support` feature, which every
//! test crate turns on) every path is under a sandbox for each process,
//! never the real home, and `SCOPE_BCA_HOME` is ignored. Tests that wrote
//! the real folders replaced a user's API keys, address book and token
//! aliases with test data (#98).

pub mod migration;

use std::ffi::OsString;
use std::path::PathBuf;

/// In test builds, the environment variable that moves the sandbox, for a
/// test that needs a home it controls (for example one that cannot be
/// written).
pub const TEST_HOME_VAR: &str = "SCOPE_TEST_HOME";

/// The environment variable that moves the whole scope folder.
pub const HOME_VAR: &str = "SCOPE_BCA_HOME";

/// The scope folder's name in the user's home.
pub const HOME_DIR_NAME: &str = ".scope-bca";

/// The scope folder: `$SCOPE_BCA_HOME`, else `~/.scope-bca`.
pub fn home() -> PathBuf {
    #[cfg(any(test, feature = "test-support"))]
    return sandbox().join(HOME_DIR_NAME);
    #[cfg(not(any(test, feature = "test-support")))]
    home_from(std::env::var_os(HOME_VAR), user_home())
}

/// The root for an override and a user home. An empty override is no
/// override; with no user home the folder is relative.
pub fn home_from(override_root: Option<OsString>, user_home: Option<PathBuf>) -> PathBuf {
    match override_root.filter(|r| !r.is_empty()) {
        Some(root) => PathBuf::from(root),
        None => user_home
            .map(|h| h.join(HOME_DIR_NAME))
            .unwrap_or_else(|| PathBuf::from(HOME_DIR_NAME)),
    }
}

/// `<home>/config.yaml`.
pub fn config_file() -> PathBuf {
    home().join("config.yaml")
}

/// `<home>/venues`: the user's venue descriptors.
pub fn venues_dir() -> PathBuf {
    home().join("venues")
}

/// `<home>/data`: what scope keeps for the user.
pub fn data_dir() -> PathBuf {
    home().join("data")
}

/// `<home>/state`: session, history and logs; safe to delete.
pub fn state_dir() -> PathBuf {
    home().join("state")
}

/// The user's home folder (for `~` in paths the user types).
#[allow(clippy::disallowed_methods)]
pub fn user_home() -> Option<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    return Some(sandbox());
    #[cfg(not(any(test, feature = "test-support")))]
    dirs::home_dir()
}

/// The platform's config folder, where old versions could keep the config.
/// Only the migration uses it.
#[allow(clippy::disallowed_methods)]
pub(crate) fn legacy_config_dir() -> Option<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    return Some(sandbox().join("legacy-config"));
    #[cfg(not(any(test, feature = "test-support")))]
    dirs::config_dir()
}

/// The platform's data folder, where old versions kept tokens and TUI
/// state. Only the migration uses it.
#[allow(clippy::disallowed_methods)]
pub(crate) fn legacy_data_dir() -> Option<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    return Some(sandbox().join("legacy-data"));
    #[cfg(not(any(test, feature = "test-support")))]
    dirs::data_dir()
}

/// The test sandbox: `SCOPE_TEST_HOME` when set, else a directory for this
/// process under the system temp directory.
#[cfg(any(test, feature = "test-support"))]
fn sandbox() -> PathBuf {
    std::env::var_os(TEST_HOME_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("scope-test-home-{}", std::process::id()))
        })
}
```

Create `crates/scope-core/src/paths/migration.rs` with one line now (Task 2 fills it in), and move `paths.rs` to `paths/mod.rs` only if the compiler requires it. In Rust 2024, `paths.rs` with `pub mod migration;` uses `paths/migration.rs`, so no move is needed:

```rust
//! Copy the user's files from the old locations (filled in by Task 2).
```

- [ ] **Step 4: Run the paths tests**

Run: `cargo test -p scope-bca-core --lib paths::`
Expected: PASS for the 5 tests. The rest of the workspace does not compile yet. That is expected, and Step 5 fixes it.

- [ ] **Step 5: Move every caller to the new functions**

`crates/scope-core/src/config.rs` (replace lines 318-363, the three functions):

```rust
    /// Returns the default configuration file path: `<home>/config.yaml`
    /// (see [`crate::paths`]).
    pub fn default_path() -> PathBuf {
        crate::paths::config_file()
    }

    /// Returns the configuration file path that `setup` and `scope web`
    /// write: `<home>/config.yaml`.
    pub fn config_path() -> Option<PathBuf> {
        Some(crate::paths::config_file())
    }

    /// Returns the default data directory for address book storage:
    /// `<home>/data`.
    pub fn default_data_dir() -> PathBuf {
        crate::paths::data_dir()
    }
```

`crates/scope-core/src/market/registry.rs` (`user_venues_dir`, lines 139-145):

```rust
    /// Path to the user venues directory (`<home>/venues`).
    pub fn user_venues_dir() -> PathBuf {
        crate::paths::venues_dir()
    }
```

`crates/scope-core/src/tokens.rs` (`aliases_path`, lines 91-94):

```rust
    /// Returns the path to the token aliases file (`<home>/data/tokens.yaml`).
    pub fn aliases_path() -> Option<PathBuf> {
        Some(crate::paths::data_dir().join("tokens.yaml"))
    }
```

`crates/scope-cli/src/cli/interactive.rs` (`context_path`, lines 127-130):

```rust
    /// Returns the path to the session context file
    /// (`<home>/state/session.yaml`).
    fn context_path() -> Option<PathBuf> {
        Some(scope::paths::state_dir().join("session.yaml"))
    }
```

`crates/scope-cli/src/cli/tui/journal.rs` (`in_data_dir`, lines 54-61):

```rust
    /// The journal in the scope state directory.
    pub fn in_data_dir() -> Self {
        let dir = scope::paths::state_dir();
        Self {
            history: Some(dir.join("history.txt")),
            events: Some(dir.join("tui-events.jsonl")),
        }
    }
```

In the same file, in `test_data_dir_journal_uses_the_documented_file_names`, change the two asserts to:

```rust
            assert!(h.ends_with("state/history.txt"));
            assert!(e.ends_with("state/tui-events.jsonl"));
```

`crates/scope-cli/src/cli/tui/crash.rs:88`:

```rust
            let dir = Some(scope::paths::state_dir());
```

`crates/scope-cli/src/cli/tui/app.rs:1242`:

```rust
                let home = scope::paths::user_home();
```

`crates/scope-web/src/lib.rs` (lines 143-150):

```rust
/// Returns the path to the PID file for the daemon.
pub fn pid_file_path() -> std::path::PathBuf {
    scope::paths::state_dir().join("scope-web.pid")
}

/// Returns the path to the log file for the daemon.
pub fn log_file_path() -> std::path::PathBuf {
    scope::paths::state_dir().join("scope-web.log")
}
```

Then run `cargo check --workspace --all-targets`, and fix every remaining error the same way. Expected remaining spots: the `config.rs` tests near lines 861-900 and 1014. Each expected path must be `crate::paths::config_file()` or `crate::paths::data_dir()`. Example for the test at line 861:

```rust
        let path = Config::default_path();
        assert_eq!(path, crate::paths::config_file());
```

- [ ] **Step 6: Run the gates**

Run:
```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
Expected: clippy has no warnings. All tests pass. The snapshot check (Global Constraints) shows that no real file changed.

- [ ] **Step 7: Commit**

```bash
git add -A crates
git commit -m "refactor(paths): one home for every user file

scope::paths returns the layout under \$SCOPE_BCA_HOME or ~/.scope-bca:
config_file, venues_dir, data_dir, state_dir and user_home. Callers no
longer build .config/scope or platform-folder paths themselves.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: The migration, with its audit log, called at startup

**Files:**
- Modify: `crates/scope-core/src/paths/migration.rs` (whole file)
- Modify: `crates/scope-web/src/main.rs:118-121` (just before `// Load configuration`)

**Interfaces:**
- Consumes: `scope::paths::{home, config_file, venues_dir, data_dir, state_dir, user_home, legacy_config_dir, legacy_data_dir}` from Task 1.
- Produces:
  - `pub struct Item { pub from: PathBuf, pub to: PathBuf }`
  - `pub enum Outcome { Copied, KeptExisting, NoSource }`
  - `pub struct Report { pub copied: usize }`
  - `pub fn legacy_items() -> Vec<Item>` (the real old-to-new list)
  - `pub fn items_for(xdg_root: Option<&Path>, platform_config: Option<&Path>, platform_data: Option<&Path>, home: &Path) -> Vec<Item>`
  - `pub fn run(items: &[Item], log: &Path) -> crate::error::Result<Report>`
  - `pub fn migrate() -> crate::error::Result<Report>`: `run(&legacy_items(), &state_dir().join("migration.jsonl"))`

- [ ] **Step 1: Write the failing tests**

Put this test module at the end of `crates/scope-core/src/paths/migration.rs`:

```rust
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
        assert_eq!(fs::read_to_string(f.xdg.join("config.yaml")).unwrap(), "chains: {}\n");
        assert!(f.platform_data.join("scope/tokens.yaml").exists());
    }

    #[test]
    fn test_config_comes_from_the_platform_folder_when_xdg_has_none() {
        // Review focus 1: old versions read the platform config folder too.
        let f = fixture();
        put(&f.platform_config.join("scope/config.yaml"), "output: {}\n");
        run_fixture(&f).unwrap();
        assert_eq!(fs::read_to_string(f.home.join("config.yaml")).unwrap(), "output: {}\n");
    }

    #[test]
    fn test_an_existing_new_file_is_never_overwritten() {
        // Review focus 2.
        let f = fixture();
        put(&f.xdg.join("config.yaml"), "old\n");
        put(&f.home.join("config.yaml"), "new\n");
        let report = run_fixture(&f).unwrap();
        assert_eq!(report.copied, 0);
        assert_eq!(fs::read_to_string(f.home.join("config.yaml")).unwrap(), "new\n");
        let config = log_lines(&f)
            .into_iter()
            .find(|l| l["to"].as_str().unwrap().ends_with("config.yaml"))
            .unwrap();
        assert_eq!(config["result"], "kept-existing");
    }

    #[test]
    fn test_a_second_run_copies_nothing() {
        let f = fixture();
        put(&f.xdg.join("config.yaml"), "chains: {}\n");
        assert_eq!(run_fixture(&f).unwrap().copied, 1);
        assert_eq!(run_fixture(&f).unwrap().copied, 0);
    }

    #[test]
    fn test_the_log_has_one_line_per_file_with_its_result() {
        let f = fixture();
        put(&f.xdg.join("config.yaml"), "chains: {}\n");
        run_fixture(&f).unwrap();
        let lines = log_lines(&f);
        assert_eq!(lines.len(), items(&f).len());
        for line in &lines {
            assert!(line["at"].is_string() && line["from"].is_string() && line["to"].is_string());
        }
        let results: Vec<&str> = lines.iter().map(|l| l["result"].as_str().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| **r == "copied").count(), 1);
        assert!(results.iter().all(|r| *r == "copied" || *r == "no-source"), "{results:?}");
    }

    #[cfg(unix)]
    #[test]
    fn test_a_failed_copy_stops_names_the_file_and_a_rerun_finishes() {
        // Review focus 3: a source that cannot be read.
        use std::os::unix::fs::PermissionsExt;
        let f = fixture();
        put(&f.xdg.join("config.yaml"), "chains: {}\n");
        let tokens = f.platform_data.join("scope/tokens.yaml");
        put(&tokens, "aliases: {}\n");
        fs::set_permissions(&tokens, fs::Permissions::from_mode(0o000)).unwrap();

        let err = run_fixture(&f).unwrap_err().to_string();
        assert!(err.contains("tokens.yaml"), "{err}");
        // The file copied before the failure stays.
        assert!(f.home.join("config.yaml").exists());
        assert!(log_lines(&f).iter().any(|l| l["result"].as_str().unwrap().starts_with("failed:")));

        fs::set_permissions(&tokens, fs::Permissions::from_mode(0o644)).unwrap();
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
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -p scope-bca-core --lib paths::migration`
Expected: compile errors (`Item`, `items_for`, `run`, `Report` and `legacy_items` are not defined).

- [ ] **Step 3: Write the migration above the tests**

```rust
//! Copy the user's files from the old locations into the scope home, once.
//!
//! Old versions kept the config, venues and address book in
//! `~/.config/scope` (or the platform config folder), and tokens and TUI
//! state in the platform data folder. On each start, every old file
//! whose new copy does not exist is copied. Nothing old is deleted or
//! changed, and no new file is written over. Every file examined gets one
//! line in `state/migration.jsonl` (the audit record).

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
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Copied => "copied",
            Outcome::KeptExisting => "kept-existing",
            Outcome::NoSource => "no-source",
        }
    }
}

/// The result of a migration run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub copied: usize,
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
        items.push(Item { from, to: home.join("config.yaml") });
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
                .filter(|p| {
                    p.extension()
                        .is_some_and(|e| e == "yaml" || e == "yml")
                })
                .collect();
            venues.sort();
            for from in venues {
                let to = home.join("venues").join(from.file_name().unwrap_or_default());
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

/// Copies each item whose source exists and whose destination does not,
/// logging every item to `log`. Stops at the first failed copy with an
/// error that names the file; the files copied before it stay.
pub fn run(items: &[Item], log: &Path) -> Result<Report> {
    let mut copied = 0;
    for item in items {
        let result = copy_one(item);
        let text = match &result {
            Ok(outcome) => outcome.as_str().to_string(),
            Err(e) => format!("failed: {e}"),
        };
        append_log(log, item, &text)?;
        match result {
            Ok(Outcome::Copied) => copied += 1,
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
    Ok(Report { copied })
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
    std::fs::copy(&item.from, &item.to)?;
    Ok(Outcome::Copied)
}

fn append_log(log: &Path, item: &Item, result: &str) -> Result<()> {
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::json!({
        "at": chrono::Utc::now().to_rfc3339(),
        "from": item.from.display().to_string(),
        "to": item.to.display().to_string(),
        "result": result,
    });
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// Migrates this machine's old files into the scope home.
pub fn migrate() -> Result<Report> {
    run(&legacy_items(), &super::state_dir().join("migration.jsonl"))
}
```

`ScopeError::io_context(e, msg)` exists in `crates/scope-core/src/error.rs` (it is used in `crates/scope-cli/src/cli/repeat.rs`). If `?` on `std::io::Error` does not convert to `ScopeError`, map it with `ScopeError::io_context(e, "cannot write the migration log")`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p scope-bca-core --lib paths::migration`
Expected: all 9 tests pass. If `test_a_failed_copy_stops_names_the_file_and_a_rerun_finishes` fails because the test runs as root (root can read mode 000), mark only that test `#[ignore = "root can read mode 000"]` when `libc::geteuid() == 0`. Do not weaken it otherwise.

- [ ] **Step 5: Call the migration in the binary**

In `crates/scope-web/src/main.rs`, insert just before the line `// Load configuration`:

```rust
    // Copy files from the old locations into ~/.scope-bca, once. The old
    // files stay (spec: docs/design/2026-10-15-scope-bca-home-design.md).
    match scope::paths::migration::migrate() {
        Ok(report) if report.copied > 0 => note!(
            out,
            "Moved your scope files to {} (copied {} files; the old files are unchanged).",
            scope::paths::home().display(),
            report.copied
        ),
        Ok(_) => {}
        Err(e) => fail(&out, e.to_string(), 1),
    }
```

- [ ] **Step 6: Run the gates**

Run:
```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
Expected: all pass. The snapshot check shows no real file changed.

- [ ] **Step 7: Commit**

```bash
git add -A crates
git commit -m "feat(paths): copy old files into ~/.scope-bca once, with an audit log

At startup, each old file whose new copy does not exist is copied.
Nothing old is deleted or changed, and no new file is written over.
state/migration.jsonl records every file and its result. A failed copy
stops with the file name; the next start copies the rest.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: The TUI harness, the documents and the CHANGELOG

**Files:**
- Modify: `crates/scope-tui-harness/src/session.rs:34-41` (`data_dir`)
- Modify: `crates/scope-tui-harness/src/evidence.rs:12`, `crates/scope-tui-harness/src/run.rs:41` (callers, if renamed)
- Modify: `crates/scope-cli/src/cli/mod.rs:140`, `crates/scope-cli/src/cli/setup.rs:305,338,482`, `crates/scope-cli/src/cli/venues.rs:59,79,141`, `crates/scope-core/src/lib.rs:135`
- Modify: `README.md`, `docs/QUICKSTART.md`, `docs/GHOLA_INTEGRATION.md`, `docs/architecture/c4-context.md`, `docs/architecture/dataflow-main.md`, `docs/monitor-config-schema.yaml`, `crates/scope-core/venues/SCHEMA.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: the layout from Task 1. The harness runs the real binary with `HOME=<temp>` and no `SCOPE_BCA_HOME`, so the binary's home is `<temp>/.scope-bca`.
- Produces: `pub fn state_dir(home: &Path) -> PathBuf` in `scope-tui-harness/src/session.rs` (replaces `data_dir`).

- [ ] **Step 1: Change the harness to the new layout**

In `crates/scope-tui-harness/src/session.rs`, replace `data_dir`:

```rust
/// Where scope keeps its TUI journal under a given HOME: the binary runs
/// with `HOME` set and no `SCOPE_BCA_HOME`, so its home is
/// `<HOME>/.scope-bca` (docs/design/2026-10-15-scope-bca-home-design.md).
pub fn state_dir(home: &Path) -> PathBuf {
    home.join(".scope-bca").join("state")
}
```

In `evidence.rs:12` and `run.rs:41`, rename the calls `data_dir(home)` to `state_dir(home)`, and update the `use crate::session::...` imports.

- [ ] **Step 2: Run the harness tests**

Run: `cargo test -p scope-tui-harness`
Expected: PASS. Then run one recorded scenario end to end: `just tui-record smoke` (or the first scenario that `ls crates/scope-tui-harness/scenarios` lists). Check that it finds `tui-events.jsonl`. Expected: the harness reports the scenario as passed.

- [ ] **Step 3: Change the texts in code**

Replace `~/.config/scope/config.yaml` with `~/.scope-bca/config.yaml`, and `~/.config/scope/venues/` with `~/.scope-bca/venues/`, at:
- `crates/scope-cli/src/cli/mod.rs:140` (the `--config` help)
- `crates/scope-cli/src/cli/setup.rs:305`, `:338`, `:482`
- `crates/scope-cli/src/cli/venues.rs:59`, `:79`, `:141`
- `crates/scope-core/src/lib.rs:135`

Then check that none is left:

Run: `git grep -n '\.config/scope\|Application Support/scope' -- crates`
Expected: no line outside `crates/scope-core/src/paths/migration.rs` and its tests.

- [ ] **Step 4: Change the documents**

In `README.md`, `docs/QUICKSTART.md`, `docs/GHOLA_INTEGRATION.md`, `docs/architecture/c4-context.md`, `docs/architecture/dataflow-main.md`, `docs/monitor-config-schema.yaml` and `crates/scope-core/venues/SCHEMA.md`, replace each old path with its new path:

| Old | New |
|---|---|
| `~/.config/scope/config.yaml` | `~/.scope-bca/config.yaml` |
| `~/.config/scope/venues/` | `~/.scope-bca/venues/` |
| `~/.config/scope/data/` | `~/.scope-bca/data/` |
| `~/Library/Application Support/scope/` (tokens) | `~/.scope-bca/data/` |
| `~/Library/Application Support/scope/` (session, history, tui-events) | `~/.scope-bca/state/` |

In the README configuration section, add this paragraph:

```markdown
All scope files are in one folder, `~/.scope-bca` (set `SCOPE_BCA_HOME` to use another folder). On the first start after an upgrade, scope copies your files from `~/.config/scope` and the platform data folder into it. It does not change or delete the old files. `~/.scope-bca/state/migration.jsonl` lists each file and what happened to it. Delete the old folders when you no longer need them.
```

Run: `git grep -n '\.config/scope\|Application Support/scope' -- README.md docs crates/scope-core/venues`
Expected: only the migration paragraph above. Dated design documents in `docs/design/` keep their old text, because they record past decisions.

- [ ] **Step 5: Add the CHANGELOG entry**

Under `## [Unreleased]` in `CHANGELOG.md`, add (create the `### Changed (breaking)` heading if it is not there):

```markdown
### Changed (breaking)
- All scope files are now in one folder, `~/.scope-bca` (`SCOPE_BCA_HOME` moves it): `config.yaml`, `venues/`, `data/` and `state/`. On the first start, scope copies the files from `~/.config/scope` and the platform data folder, and writes `state/migration.jsonl`. The old files are not changed.
```

- [ ] **Step 6: Run the gates and commit**

Run:
```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
Expected: all pass, and the snapshot check shows no real file changed.

```bash
git add -A crates docs README.md CHANGELOG.md
git commit -m "docs: scope files are in ~/.scope-bca

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: End-to-end check on a copy, then the PR

**Files:** none changed.

- [ ] **Step 1: Run the release binary against a temporary home**

This reads the real old files and writes only to a temporary folder:

```bash
cargo build --release
tmp=$(mktemp -d)
SCOPE_BCA_HOME=$tmp ./target/release/scope setup --status
```

Expected:
- the note `Moved your scope files to <tmp> (copied N files; the old files are unchanged).`, where N is the count of your old files;
- `setup --status` shows your Etherscan key as ✓ (it was read from `<tmp>/config.yaml`);
- `$tmp/state/migration.jsonl` has one line for each file;
- the old files have the same modification times as before the run (`stat` them before and after).

- [ ] **Step 2: Run it a second time**

```bash
SCOPE_BCA_HOME=$tmp ./target/release/scope setup --status
```

Expected: no "Moved" note, and the log lines for this run all say `kept-existing` or `no-source`.

- [ ] **Step 3: Push and open the PR into develop**

Push the branch (the pre-push hook runs fmt, clippy, audit and coverage). Open a PR into `develop`. Write its description in Simplified Technical English, and include the end-to-end output from Steps 1-2.
