# One home for scope: `~/.scope-bca`

Status: design for review. Date: 2026-10-15.

## 1. Goal

Scope keeps all user files in one folder: `~/.scope-bca`. Today the files are in two places, and the second place is different for each operating system. Users cannot find their files, and back-ups miss some of them.

The token disambiguation design comes after this one. It keeps its saved bindings in this folder.

## 2. Current state

| File | Who writes it | Location now |
|---|---|---|
| `config.yaml` | `setup`, `scope web` | `~/.config/scope/` (or the platform config folder, if a file is there) |
| `venues/*.yaml` | the user | `~/.config/scope/venues/` |
| `data/address_book.yaml` | `address-book` | `~/.config/scope/data/` (or `address_book.data_dir` in the config) |
| `tokens.yaml` | `crawl`, `tokens` | platform data folder `/scope/` |
| `session.yaml` | the TUI | platform data folder `/scope/` |
| `history.txt` | the TUI | platform data folder `/scope/` |
| `tui-events.jsonl` | the TUI | platform data folder `/scope/` |
| `portfolio.yaml` | no current code | platform data folder `/scope/` |

The platform data folder is `~/Library/Application Support` on macOS and `~/.local/share` on Linux.

Path code is in six files: `config.rs`, `market/registry.rs`, `tokens.rs`, `cli/interactive.rs`, `cli/tui/journal.rs` and `cli/tui/crash.rs`. Each one adds its own `scope` subfolder to a root from `scope::paths`. The TUI test harness (`scope-tui-harness/src/session.rs`) has a copy of the data folder rule.

## 3. Layout

```
~/.scope-bca/
  config.yaml          what the user edits
  venues/              user venue descriptors
  data/                what scope keeps for the user
    address_book.yaml
    tokens.yaml
  state/               safe to delete
    session.yaml
    history.txt
    tui-events.jsonl
    migration.jsonl
```

## 4. Resolution rules

1. The root is `$SCOPE_BCA_HOME` when it is set and not empty. Otherwise it is `~/.scope-bca`.
2. The config file is `--config <PATH>`, then `$SCOPE_CONFIG`, then `<root>/config.yaml`. The first two keep their current meaning.
3. `address_book.data_dir` in the config still overrides `<root>/data` for the address book.
4. In test builds, the root is the test sandbox from #98 (`SCOPE_TEST_HOME`, else a folder for each process in the temp folder). No test can resolve a real user folder.

## 5. Code

`scope::paths` is the only place that knows the layout. It gets these functions:

| Function | Returns |
|---|---|
| `home()` | the root (rule 1 and rule 4) |
| `config_file()` | `<root>/config.yaml` |
| `venues_dir()` | `<root>/venues` |
| `data_dir()` | `<root>/data` |
| `state_dir()` | `<root>/state` |

The current functions change as follows. The `clippy.toml` ban on `dirs::*` stays.

| Now | After |
|---|---|
| `home_dir()` | `user_home()`: the user's home folder (sandboxed in tests). The TUI `save ~/file` command uses it to expand `~`. |
| `config_dir()` | `legacy_config_dir()`, private: only the migration (section 6) uses it. |
| `data_dir()` | `legacy_data_dir()`, private: only the migration uses it. The name `data_dir()` now means `<root>/data`. |

The six files in section 2 call the new functions. No caller builds a path with `.config`, `scope` or a platform folder after this change. The harness gets the layout from one function and does not copy the rule.

## 6. Migration

The migration runs once at startup, before the config loads. It runs in the `scope` binary only, not in the library.

1. For each file in the table in section 2 (`portfolio.yaml` excluded), find the old path.
2. If the old file exists and the new file does not exist, copy the old file to the new path. Create the parent folder when necessary.
3. Never delete or change an old file. Never write over a new file.
4. For each file that it examines, write one line to `state/migration.jsonl`: the time, the source, the destination, and the result (`copied`, `kept-existing`, `no-source` or `failed: <error>`).
5. If it copied at least one file, show one note: `Moved your scope files to ~/.scope-bca (copied N files; the old files are unchanged).`
6. If a copy fails, stop with an error. The error names the file and the cause. The files that were copied before the failure stay. The next start tries the remaining files again.

`portfolio.yaml` is not copied, because no current code reads it.

The migration is quick on later starts: each file is one `exists` check.

## 7. Documents

Change the paths to `~/.scope-bca` in: `README.md`, `docs/QUICKSTART.md`, `docs/GHOLA_INTEGRATION.md`, `docs/architecture/c4-context.md`, `docs/architecture/dataflow-main.md`, `docs/monitor-config-schema.yaml`, `crates/scope-core/venues/SCHEMA.md`, and the help and note texts in `setup.rs`, `venues.rs` and `cli/mod.rs`. Add a CHANGELOG entry under "Changed (breaking)".

## 8. Tests

1. Paths: each function returns its folder under the root; `SCOPE_BCA_HOME` moves all of them; in test builds every path is in the sandbox.
2. Migration, with temporary old and new folders:
   - It copies a file that has no new copy, and the old file stays the same.
   - It does not write over a new file.
   - A second run copies nothing.
   - A failed copy (a source that cannot be read) stops with the file name, and the files copied before it stay.
   - `migration.jsonl` has one line for each file, with the correct result.
3. The test sandbox: a full `cargo test --workspace` writes no file in the real old folders or in `~/.scope-bca` (the same snapshot check as #98).
4. The TUI harness finds `tui-events.jsonl` in `state/`.

## 9. Out of scope

- Deletion of the old folders. The user deletes them when they want. The note in section 6 says that the old files are unchanged.
- The token disambiguation. It has its own design after this one.
- #99 (TUI config reload). It comes after the two designs.
