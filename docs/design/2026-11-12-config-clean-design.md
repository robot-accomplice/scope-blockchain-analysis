# Design: `scope setup --clean`, a command that removes dead keys from the config file

Date: 2026-11-12
Status: proposed. The owner has not yet answered Q1 to Q11 (section 11).
Based on: `origin/develop` at c676c11 (v0.8.8).
No code. This document is a design. File names and function names are proposals unless the text says "today".

## 1. Problem

Version 0.8.7 removed the old monitor dashboard. A config written by an older `scope setup` can still have six keys of that dashboard under `monitor:`: `layout`, `widgets`, `scale`, `color_scheme`, `export` and `auto_pause_on_input`.

Such a config still loads. But every command prints this on stderr:

```
scope: ignoring config keys of the removed monitor dashboard under `monitor:`: ... Delete them from the config file.
```

The user must edit the file by hand. The owner chose a command that removes the keys. The command makes a backup and shows a diff. A load must never rewrite the file without being asked.

## 2. Decisions in short

1. The command is a flag of the setup command: `scope setup --clean`. The alias gives `scope config --clean`.
2. The default is a dry run. It prints the diff and changes nothing. `--write` applies the change.
3. The command removes only the six known keys, and only under the top-level `monitor:`. It never removes an unknown key.
4. The edit is line-based, so comments, order and line endings stay as they are. A check with the YAML parser must pass before anything is written.
5. `--write` first copies the file to a backup beside it. It then writes a temporary file and renames it over the config.
6. Exit code 0: done, dry run, or nothing to clean. Exit code 1: any error. No new code.
7. The load notice names the command.
8. Every `--write` run prints, and logs, the backup path and the removed lines with their line numbers.

## 3. What the code does today

### 3.1 The notice and the list of dead keys

| Fact | Where |
|---|---|
| One constant holds the six keys: `DROPPED_MONITOR_KEYS`. It is private. | `crates/scope-core/src/config.rs:458` |
| `dropped_monitor_keys(contents)` parses the text with a small probe struct. It returns the keys found under `monitor:`, in the order of the constant. It returns an empty list when the text does not parse. | `config.rs:470` |
| `report_dropped_monitor_keys` writes the one line. It writes only once for each process (`DROPPED_KEYS_REPORTED`). The line goes to the diagnostics writer, never to stdout. | `config.rs:495`, text at `config.rs:505` |
| `Config::load` calls both after a successful parse. | `config.rs:330` |
| `MonitorConfig` has no field for the six keys. The doc comment says they are ignored. | `config.rs:510` |
| No struct has `deny_unknown_fields`. Every other unknown key, at any level, is ignored with no message. | `grep` of `crates`: no match |

So the list of dead keys is one constant. The notice is the only place that handles ignored keys. A typo such as `monitor.refresh_secs` is silent today.

### 3.2 How the config is read and written

| Fact | Where |
|---|---|
| The path rule is: `--config` argument, else `SCOPE_CONFIG`, else `<home>/config.yaml`. The rule is written inline in `Config::load`. | `config.rs:301` to `305` |
| `<home>` is `$SCOPE_BCA_HOME`, else `~/.scope-bca`. An empty variable is no override. | `crates/scope-core/src/paths.rs:35`, `:44` |
| In tests (`cfg(test)` or feature `test-support`) `paths::home()` is a sandbox folder in the system temp directory. It is never the real home. | `paths.rs:117` |
| `--config PATH` is a global flag. | `crates/scope-cli/src/cli/mod.rs:150` |
| The binary loads the config at start. A load error is a note ("Could not load config, using defaults"). The command still runs. | `crates/scope-web/src/main.rs:171` |
| `setup` is excluded from the first-run prompt. | `main.rs:218` |
| `Commands::Setup(args)` calls `setup::run(args, config, out)`. | `crates/scope-cli/src/cli/dispatch.rs:202` |
| `setup::run` re-reads the file with `Config::load(None)`. It ignores `--config`. `Config::config_path()` is always `<home>/config.yaml`. | `setup.rs:75`, `config.rs:347` |
| The only YAML writer is `save_config_to_path`. It serializes the whole `Config` with `serde_saphyr::to_string`. It adds the header `# Scope Configuration`. It uses `std::fs::write`. There is no temporary file and no backup. | `setup.rs:805` |

### 3.3 Does `scope setup` already fix the file?

This is the "is the change needed" test.

Yes, as a side effect. `Config` has no field for the dead keys, so `save_config_to_path` writes none of them. After a full wizard run the notice is gone.

But the wizard is not a good fix:

1. It asks questions the user did not come to answer.
2. It replaces the whole file. Comments are lost. The key order becomes the order of the struct. Unknown keys are lost.
3. It makes no backup.

The command in this document does the same removal, and nothing else. The change is needed.

### 3.4 The CLI shape today

`Commands::Setup(SetupArgs)` has `#[command(visible_alias = "config")]` (`mod.rs:299`). So `scope config` is `scope setup`. `SetupArgs` is a flat struct with three flags: `--status`, `--key`, `--reset` (`setup.rs:37`). It has no subcommands. `scope config clean` as a subcommand would turn `SetupArgs` into a subcommand group. `scope config` alone must still start the wizard.

`Commands::is_interactive` lists `Setup`. So `setup` prints no version header (`mod.rs:321`). That also suits `--clean`.

`crates/scope-web/src/main.rs:229` builds a `SetupArgs` with a struct literal. A new field breaks that line. The change must update it.

### 3.5 How other commands write user files

| Command | Method | Where |
|---|---|---|
| Venue override (#211 and before) | Move the old files into a new folder `<home>/venues-backup-<stamp>`. The stamp is `chrono::Local::now().format("%Y-%m-%d-%H%M%S")`. No collision rule. | `venues.rs:382`, `:525` |
| Watch state (#209) | Write a temporary file, then `rename` it over the target. A failed rename removes the temporary file. The rename is injected as a closure so a test can make it fail. | `watch/state.rs:252` to `:298` |
| Home migration | Copy to `<name>.migrating`, then `rename`. A stale temporary file is replaced. A log line in JSON for each copy. | `paths/migration.rs:234` to `:241` |
| `setup` wizard | `std::fs::write` of the whole file. | `setup.rs:816` |

No code sets file permissions on a config or a state file. `std::fs::write` uses the default mode.

The new command copies the temporary-file pattern from the watch state, the stamp from the venue backup, and the injected closure for tests.

## 4. Edit method: line-based or serde

| | A. Line-based edit | B. Serde round trip |
|---|---|---|
| Comments | Kept | Lost |
| Key order, spacing, quotes | Kept | Rewritten |
| Unknown keys | Kept | Lost (`Config` has no place for them) |
| CRLF, BOM | Kept (whole lines are removed, other bytes do not change) | Lost |
| Code to write | A small scanner for the block under `monitor:` | One call (`save_config_to_path`) |
| Main risk | The scanner may misread a YAML shape it does not know | A user loses comments and keys they wrote |

There is a third path: add a YAML crate that keeps the syntax tree. The project does not use one. A new dependency for one command is not justified yet.

**Recommended: A, with a check by the parser.** The scanner handles one shape: the block form of the top-level `monitor:` mapping. That is the shape every `scope setup` wrote. For any other shape the command stops and writes nothing (section 5.5). The check in section 5.6 proves the result before the file is touched. B fails the rule in the task: it would remove keys the command must not remove.

## 5. Proposal

### 5.1 CLI shape and help text

```
scope setup --clean [--write]
scope config --clean [--write]            (alias)
scope --config PATH setup --clean         (a file you name)
```

New fields of `SetupArgs`:

```rust
/// Remove config keys that no code reads (shows a diff; writes nothing).
#[arg(long, conflicts_with_all = ["status", "key", "reset"])]
pub clean: bool,

/// With --clean: apply the change. Saves a backup of the file first.
#[arg(long, requires = "clean")]
pub write: bool,
```

Help text of the flags (STE):

```
--clean   Remove dead keys from the config file. Shows what it would
          remove. Changes nothing unless you add --write.
--write   With --clean: save the change. The command first copies the
          file to config.yaml.backup-<date-time> in the same folder.
```

New example lines for `after_help`:

```
  scope setup --clean
  scope setup --clean --write
```

The `setup` doc comment gets one more sentence: "Use --clean to remove config keys of the removed monitor dashboard."

`--config PATH` and `SCOPE_CONFIG` work as for every other command (section 5.4).

### 5.2 Output

The diff goes to stdout. It is data, and a user can redirect it. The prose goes to stderr through `note!`, as for other commands. Colour follows the existing rules: none when the stream is not a terminal.

The diff is in unified form, with only deletions. There are no context lines (see risk 8).

Dry run:

```
scope: config clean (dry run): C:\Users\me\.scope-bca\config.yaml
--- config.yaml
+++ config.yaml (cleaned)
@@ -41,3 +40,0 @@
-  layout: default
-  widgets:
-    - price
@@ -47,1 +43,0 @@
-  scale: 2
scope: would remove 3 keys (layout, widgets, scale) in 4 lines. Nothing was changed.
scope: run `scope setup --clean --write` to apply.
```

The first number of a hunk is the line in the original file. A hunk for a pure deletion has `+start,0`, where `start` is the line before the deletion in the new file. This is the format of `diff -u`, so `patch` accepts it.

After `--write`:

```
scope: config clean: C:\Users\me\.scope-bca\config.yaml
<the same diff>
scope: backup: C:\Users\me\.scope-bca\config.yaml.backup-2026-11-12-143005
scope: removed 3 keys (layout, widgets, scale), 4 lines. File: 1180 bytes to 1096 bytes.
```

A removed line that holds a value is shown as it is, with its value. The user sees `scale: 2` or `export:` with its path.

### 5.3 What the command removes

1. Only the keys in `DROPPED_MONITOR_KEYS`. The constant becomes `pub` in `scope-core`. The notice, the detection and the cleaner all read it. There is one list.
2. Only keys that are direct children of the top-level `monitor:`. A key `layout:` elsewhere (for example under `output:`) is never touched.
3. For each such key: the key line and every following line that is deeper in indent. This covers a block map, a list, a block scalar (`|`) and a flow value that spans lines.
4. A key with a value other than the old default is removed too. The keys have no field now, so any value has no effect. The command does not try to tell "default" from "changed": the old defaults are not in the code any more. The diff shows every removed line, and the backup keeps the file. Question Q7.
5. Comment lines and blank lines are not removed. A comment above a removed key stays (Q4). A blank or comment line between two lines of one removed value is removed with it. A comment at the key's indent or lower, after the value, stays.
6. If no live key is left in the `monitor:` block, the `monitor:` line is removed too. A bare `monitor:` would load as a null value, and the check in 5.6 proves what the parser does with the result. Question Q10.

Unknown keys under `monitor:` (for example `monitor.refresh_secs`) stay. The command may print them as a note: "kept 1 key that scope does not read: refresh_secs". Reason: a typo is a likely cause, and the user is already looking at this section. This note is optional and not in the first release (section 9).

### 5.4 The config path

1. The path rule moves into one function, `Config::resolve_path(arg: Option<&Path>, env: Option<OsString>) -> PathBuf`. `Config::load` calls it. The cleaner calls it. The rule stays in one place, and the env value is a parameter so tests need no process-wide variable.
2. `dispatch` passes `source.path` to the cleaner. The cleaner does not use `Config::config_path()`, because that ignores `--config`. See section 9 for the related fault in `setup`.
3. The first line of the output names the file.
4. If the path is a symbolic link, the command writes to the target and keeps the link. It uses `canonicalize` once, before the read.

### 5.5 Steps of a run

1. Resolve the path. If the file does not exist: print `no config file at <path>`, exit 1. Nothing is created (Q5).
2. Read the file as bytes. If it is not valid UTF-8: error, exit 1.
3. Parse the text with the YAML parser as `Config`. If it fails: print the parse error and "fix the YAML first; nothing was changed", exit 1.
4. Call `dropped_monitor_keys(text)`. If it is empty: print `no dead keys in <path>; nothing to do`, exit 0. No backup. No write. This holds with `--write` too, so a second run changes nothing.
5. Plan: scan the lines (section 5.5.1). The result is the list of removed line ranges and the new bytes.
6. Check the plan (section 5.6). If a check fails: print which one, exit 1. Nothing was written.
7. Dry run: print the diff and the hint, exit 0.
8. `--write`:
   1. Create the backup with `create_new` (section 5.7). Copy the original bytes into it. Read it back and compare. If it differs: remove it, error, exit 1.
   2. Write the new bytes to `<name>.cleaning` in the same folder (a stale file of that name is replaced). Copy the permissions of the original to it. Call `sync_all`.
   3. Read the original again. If its bytes differ from step 2 of this list, the user or another program changed it. Remove the temporary file, print "the file changed while scope was working; nothing was written", exit 1.
   4. `rename` the temporary file over the original. On failure: remove the temporary file, print the error and the backup path, exit 1. The original is intact.
   5. Read the new file. Compare it with the planned bytes. If it differs: print the exact restore command with the backup path, exit 1.
   6. Print the diff and the summary. Write the log line (section 6). Exit 0.

#### 5.5.1 The scanner

Input: the lines of the file, each with its own terminator (`\n` or `\r\n`). A BOM at the start of line 1 is skipped for matching only.

1. Find the line `monitor:` at indent 0. Allowed after the colon: spaces and a comment. Any other form of the key (`monitor: {...}`, `"monitor":`, a second `monitor:` key, a line with `\t` in the indent) is "unsupported shape".
2. The block is the lines after it, until the next line that is not blank, not a comment, and has indent 0. A line `---` or `...` also ends it.
3. The child indent is the indent of the first live line in the block.
4. A dead key line: indent equals the child indent, and the text starts with a name from the list, then `:`, then a space, a comment or the end of the line.
5. The extent of the key: the key line, plus the following lines that are deeper than the child indent. Blank and comment lines that sit between two such lines belong to the extent. Those after the last deeper line do not.
6. If `dropped_monitor_keys` found a key that the scanner did not remove, the shape is unsupported. Print: "cannot remove `monitor.<key>` automatically (the layout is not the block form). Delete it by hand: line N." Exit 1. Nothing is written. The scanner never guesses.

### 5.6 The check before any write

All five must hold. The code for the check is the same for a dry run and for `--write`, so a dry run proves a `--write` would pass.

1. The new text parses as `Config`.
2. The `Config` of the new text equals the `Config` of the old text. `Config` derives `PartialEq` (`config.rs:83`). Both ignore the dead keys, so a difference means the scanner removed a live line.
3. A generic tree of the old text, minus the dead keys, equals the generic tree of the new text. (A `serde_json::Value`, read through the same YAML parser.) This covers the keys `Config` ignores, such as unknown keys.
4. `dropped_monitor_keys(new)` is empty.
5. The keys of the removed lines are exactly the keys of `dropped_monitor_keys(old)`.

Check 2 does not see an unknown key. Check 3 does. Together they prove that only dead keys left.

### 5.7 The backup

- Name: `<config file name>.backup-<stamp>` in the folder of the config. Example: `config.yaml.backup-2026-11-12-143005`. The stamp is the local time in the format of the venue backup: `%Y-%m-%d-%H%M%S`. Question Q9.
- Creation: `OpenOptions::create_new(true)`. It never overwrites.
- Collision: if the name exists, add `-2`, `-3`, and so on, up to `-99`. Then stop with an error. The existing backups are never touched.
- Content: the exact bytes that were read in step 2. Not a re-serialized copy.
- Permissions: the same as the original (see 5.8). The backup holds the same API keys as the config.
- The command never deletes a backup.

### 5.8 Line endings, BOM, permissions

| Item | Rule |
|---|---|
| CRLF or LF | Only whole lines (with their own terminator) are removed. The other bytes do not change. A file with mixed endings stays mixed. |
| BOM | Not matched, not removed. It stays on line 1. |
| No final newline | If the last line of the file is removed, the new last line keeps its own terminator. The file may now end with a newline that it did not have. This is harmless and the YAML is equal (check 3). |
| Unix mode | Copied from the original to the temporary file and to the backup (`set_permissions`). A config with mode 0600 stays 0600. |
| Windows | The temporary file and the backup get the access rules of the folder, as a new file does. `rename` over an existing file replaces it. If another program holds the config open, the rename fails and step 8.4 applies. |
| Owner | Not changed. If the file belongs to another user, the rename may fail. Step 8.4 applies. |

### 5.9 Exit codes

| Case | Code | Output |
|---|---|---|
| Dead keys found, dry run | 0 | diff and hint |
| Dead keys removed with `--write` | 0 | diff, backup path, summary |
| No dead keys | 0 | "nothing to do" |
| File does not exist (Q5) | 1 | path |
| File is not UTF-8 | 1 | path |
| YAML does not parse | 1 | the parse error |
| Unsupported shape | 1 | key and line number |
| A check in 5.6 fails | 1 | the check |
| Backup, write, rename or file-changed failure | 1 | the step and the backup path |
| Bad flag combination (`--write` without `--clean`, `--clean --reset`) | 2 | from clap, as everywhere |

The binary maps every `Err` to exit 1 (`main.rs:317`) and clap usage errors to 2. The command adds no code. A dry run with dead keys exits 0 (Q6).

### 5.10 The load notice

The text changes to:

```
scope: ignoring config keys of the removed monitor dashboard under `monitor:`: layout, scale. Run `scope setup --clean` to remove them.
```

When the path came from `--config`, the hint is `scope --config "<path>" setup --clean`. `Config::load` has the argument, so it can build the text. When the path came from `SCOPE_CONFIG` or the default, the short hint is right in the same shell.

The notice is still printed once for each process. The `setup --clean` run itself prints it first, because the binary loads the config before the command runs. This is noise, not a fault (Q11).

The doc comments at `config.rs:456` and `config.rs:510`, and the README note at `README.md:1077`, name the command.

## 6. RCA instrumentation

A bad rewrite must be traceable after the fact. The command records:

1. **On the terminal (every run).** The config path. The mode (`dry run` or `write`). Every removed line with its original line number, in the diff. The key names. Byte counts before and after. With `--write`: the backup path.
2. **A log line (`--write` only).** One JSON line appended to `<home>/state/config-clean.log`:

```json
{"at":"2026-11-12T14:30:05+01:00","config":"...\\config.yaml","backup":"...\\config.yaml.backup-2026-11-12-143005",
 "keys":["layout","widgets","scale"],"removed":[{"line":41,"text":"  layout: default"}],
 "bytes_before":1180,"bytes_after":1096,"sha256_before":"...","sha256_after":"...","result":"ok"}
```

   The line is written for a failed run too, with `result` set to the step that failed (`backup`, `write`, `file_changed`, `rename`, `verify_after`). `sha2` is already a dependency of both crates. The hash proves that the backup is the file that was there. A failure to write the log is a note and does not change the exit code. `state/` is the folder for logs and is safe to delete (`paths.rs`).
3. **Tracing events.** `tracing::info!` for each step with the same fields, and `tracing::warn!` for a refused shape, with the line number.
4. **No secret.** The log holds only dead-key lines. It never holds a context line (risk 8).

The backup file is the full record of the input. The log is the record of the decision.

## 7. Tests

All tests use a temporary folder, or the `paths` sandbox. None reads or writes the real `~/.scope-bca`. No test sets a process-wide variable: the path rule takes the env value as a parameter.

Pure tests (`scope-core`, on text):

1. **Only the six keys go.** A config with all six keys, `monitor.refresh_seconds`, `monitor.venue`, an unknown `monitor.refresh_secs`, a top-level `layout:` and an `output.layout:`. Only the six keys under `monitor:` are gone. *Why: the owner said "never unknown keys"; a key of the same name elsewhere is user data.*
2. **Whole values.** The six keys with a block map (`export:` with children), a list (`widgets:`), a block scalar and a multi-line flow list. No orphan line stays. *Why: a half-removed value is invalid YAML or a changed one.*
3. **Everything else is identical.** Every line outside the removed ranges is byte-equal, in the same order, with the comments. *Why: the reason for the line-based method.*
4. **Endings.** The same fixture with LF, with CRLF, with mixed endings, with a BOM, and with no final newline. The output keeps the endings and the BOM. *Why: a Windows config must not turn into a mixed file.*
5. **Empty block.** All children dead: the `monitor:` line goes and the `Config` is equal. *Why: a bare `monitor:` is a null; the check must prove the parser accepts the result.*
6. **Unsupported shapes refuse.** `monitor: {layout: x}`, `"monitor":`, a tab in the indent, a second `monitor:` key. Each returns the "cannot remove automatically" error with the key and a line number, and no new text. *Why: the scanner must not guess at a shape it does not know.*
7. **Idempotent.** Cleaning the output finds nothing. *Why: a second run must not change a file or make a backup.*
8. **The check can fail.** A test hook gives the check a plan that also drops a live line, then one that drops an unknown key. Check 2 fails the first, check 3 the second. *Why: a check that has never failed is not proven to work.*
9. **One list.** The test builds a config from every entry of `DROPPED_MONITOR_KEYS`, then asserts that detection, the notice and the cleaner each name all of them. *Why: a key added to the constant must reach all three; a second list would drift.*
10. **Detector and scanner agree.** For a table of fixtures, the keys the scanner removes equal `dropped_monitor_keys`. *Why: they are two readers of one fact.*

Command tests (`scope-cli`, on a temporary file):

11. **A dry run changes nothing.** Bytes and modified time of the file are the same. No backup and no `.cleaning` file exist. Exit 0. *Why: the default must be safe.*
12. **`--write` result.** The backup equals the original byte for byte. The new file equals the plan. Stdout holds the backup path and the line numbers. Exit 0. *Why: this is the contract of the command.*
13. **Backup collision.** A file with the stamp name exists. The new backup is `-2`. The old one is unchanged. A test passes the stamp in. *Why: a backup must never overwrite an older backup.*
14. **Failed rename.** Inject a rename that fails (the pattern of `save_with` in `watch/state.rs`). The original is intact, the temporary file is gone, the backup exists, the message names it, exit 1. *Why: a failed write must never cost the config.*
15. **File changed during the run.** Inject a change between the backup and the rename. Nothing is written. Exit 1. *Why: the user's edit made in an editor must win.*
16. **Missing, bad YAML, nothing to clean.** Missing: exit 1, nothing created. Bad YAML: exit 1, file unchanged, no backup. Nothing to clean with `--write`: exit 0, no backup. *Why: each case has its own code and must not leave files.*
17. **No secret in output.** A config with an API key on the line before and the line after `monitor:`. Neither stdout, stderr nor the log holds the key, in dry run and in `--write`. *Why: the rule is "never print an API key"; a diff with context lines would break it.*
18. **Path rule.** `--config` beats `SCOPE_CONFIG`, which beats the home path. An empty env value counts as unset. The command with `--config` cleans that file and not the home file. *Why: `setup` ignores `--config` today; the new command must not copy that fault.*
19. **The notice.** The text names `scope setup --clean`. With a `--config` path it names `scope --config "<path>" setup --clean`. It is written once for each process. The existing notice tests (`config.rs`, near line 613) change to the new text. *Why: the notice is the only way users find the command.*
20. **Clap.** `--clean` conflicts with `--status`, `--key` and `--reset`. `--write` needs `--clean`. `config --clean` parses as `setup --clean`. *Why: `--write` alone must not look like it did something.*
21. **Unix mode (`cfg(unix)`).** A 0600 config gives a 0600 backup and a 0600 result. A symbolic link stays a link and the target changes. *Why: the file holds an API key.*
22. **The log.** A `--write` run adds one JSON line to the log in the sandbox `state/`. A failed run adds a line with the failed step. A log that cannot be written does not change the exit code. *Why: the trail is the RCA record.*
23. **Existing tests.** The tests for `SetupArgs` (`setup.rs`), for the load of a config with dead keys (`config.rs:585`) and the parity of the help text pass, with the struct literal in `main.rs` updated. *Why: the change must not break the wizard.*

## 8. Risks

1. **The scanner reads YAML as lines.** It may misread an unusual shape. The shape rule in 5.5.1 and the check in 5.6 turn a misread into a refusal, not into a wrong file. Test 8 proves the check can fail.
2. **A change made while the command runs.** The editor of the user may save the file. Step 8.3 compares the bytes again, but a window of a few microseconds remains between the compare and the rename. A lock would not stop an editor. The backup covers the window.
3. **Backups pile up and hold API keys.** Each `--write` run makes one only when keys were found, so the number is small. The backup has the mode of the config. The summary line tells the user to delete it when the result is good. The command never deletes it.
4. **Windows locks.** An editor or antivirus can hold the config open. The rename then fails. The user sees the error, the original is intact, and the backup exists.
5. **Orphan comments.** A comment above a removed key stays and may now describe nothing. This is harmless. The alternative (remove it) can delete text that is not about the key (Q4).
6. **A running TUI.** The TUI reads the config at start and again after `setup`. A `--clean --write` run inside the TUI is a normal command. The user gets the new file at the next load. No prompt is shown: `--write` is the explicit yes.
7. **A second tool writes the config.** `scope web` and the wizard also write the file. A clean run and a wizard run at the same time can lose one change. This is true for the wizard alone today. Out of scope (section 9).
8. **A secret in the diff.** A diff with context lines would print the neighbours of `monitor:`. The config has API keys in `chains:`. The diff therefore has no context lines. Test 17 guards it. A dead key line can hold only a dashboard setting.
9. **The notice names a command that does not exist in an older install.** The notice comes from the same binary as the command. No mismatch can occur.
10. **Docs.** The CHANGELOG needs an entry under `Added` (`[Unreleased]`). The README note at `README.md:1077` and the help text of `setup` change. The release notes should say that the notice names the command.

## 9. Out of scope, with reasons

| Item | Why it is not in this design |
|---|---|
| Rewriting the file on load, without being asked | The owner chose a command. A load runs in read-only commands, in `scope watch` under cron, and in several processes at once. A write there is a risk with no consent. |
| Removing unknown keys, or a typo such as `refresh_secs` | Scope ignores unknown keys everywhere and has no list of them. A key may be a typo, or a key of a later version. Removal can destroy user data. A note that lists them is possible later; it needs a list of all known keys, which the code does not have. |
| A table of deprecated keys for all sections | Only one section has dead keys today. A general table is a design for the second case, not the first. Revisit when a second key is removed. |
| `--check` (exit 1 when there is something to clean) | No user asked. A dry run already shows the result. It is one flag to add later, and it does not change this design. Q6 keeps the exit code of the dry run simple. |
| `--format json` output | No consumer exists. The log line holds the same facts in JSON. |
| A confirmation prompt with `--write` | `--write` is the confirmation. A prompt breaks scripts, pipes and the TUI, and the first run also has the backup. |
| Deleting or limiting old backups | It needs a policy (age, count, size). The venue backups have none either. |
| `--restore` | The user can copy the backup over the config. The summary prints the path. Add it if users ask. |
| A YAML crate that keeps comments | A new dependency. Not needed while the scanner covers the shape that `scope setup` wrote. Revisit if test 6 shows real configs that it refuses. |
| `setup` ignoring `--config` (`setup.rs:75`, `:326`, `:503`) | This is an existing fault: `scope --config X setup` writes the default file. The fix changes the behaviour of the wizard, a separate change. The owner should decide. The new command does not share the fault (test 18). |
| A wizard that keeps comments and unknown keys | It is the larger fix for the same loss (section 3.3). It needs the same line-based method for every key the wizard sets. Too large for this change. |
| Other files (venues, watchlists, state) | No dead keys are known there. |
| Hiding the notice in the `--clean` run | The load runs before the command is known. The change would add a rule to `Config::load` for one command. Q11. |

## 10. Order of work

One pull request.

1. `scope-core`: make `DROPPED_MONITOR_KEYS` `pub`. Add `Config::resolve_path`. Add the pure planner and the check (new module, for example `config_clean.rs`) with tests 1 to 10.
2. `scope-cli`: the two flags, the command (`cli/config_clean.rs`), the dispatch line, the `main.rs` struct literal, the log. Tests 11 to 18 and 20 to 23.
3. The notice text and its tests (test 19). The README, the CHANGELOG and the help text.

## 11. All open questions

| # | Question | Options | Recommended |
|---|---|---|---|
| Q1 | What is the CLI shape? | A. Flag: `scope setup --clean [--write]`. B. Subcommand: `scope config clean`. C. A new top-level command. | A |
| Q2 | How does it edit the file? | A. Line-based, with a parser check. B. Serde round trip. C. A new YAML crate. | A |
| Q3 | What is the default mode? | A. Dry run, `--write` applies. B. Write, `--dry-run` previews. C. Ask a question. | A |
| Q4 | What happens to a comment above a removed key? | A. It stays. B. It is removed when it sits directly above, at the same indent. | A |
| Q5 | What is the exit code when the file does not exist? | A. 1 with the path. B. 0 with "nothing to do". | A |
| Q6 | What is the exit code of a dry run that finds dead keys? | A. 0. B. 1, as a check. | A |
| Q7 | What about a key with a value that is not the old default? | A. Remove it, show it in the diff, keep it in the backup. B. Ask for `--force`. | A |
| Q8 | Does `--write` write a log line? | A. Yes, in `<home>/state/config-clean.log`. B. Tracing events only. | A |
| Q9 | Where does the backup go? | A. Beside the config: `config.yaml.backup-<stamp>`. B. In a folder of the scope home. | A |
| Q10 | What happens when all children of `monitor:` are dead? | A. The `monitor:` line is removed too. B. It becomes `monitor: {}`. | A |
| Q11 | Does the `--clean` run hide the load notice? | A. No. B. Yes. | A |

Reasons for the recommendations:

- **Q1.** `--status`, `--key` and `--reset` are flags. A flag adds no subcommand layer, and the `config` alias gives `scope config --clean`. B makes `SetupArgs` a subcommand group and must keep `scope config` as the wizard. C adds a command for one task.
- **Q2.** It is the only option that keeps comments, order and unknown keys, and the check makes it safe. B fails the rule "never unknown keys". C adds a dependency before it is shown to be needed.
- **Q3.** The owner asked for a diff and not a silent rewrite. A dry run by default cannot harm a file.
- **Q4.** Text that is not the key's value may not be about the key. An orphan comment costs nothing. A removed comment may cost the user text.
- **Q5.** A missing file is a failure to do the task. A wrong `--config` path must not look like success. The default path missing is a first run, and the message says so.
- **Q6.** A dry run succeeded in its job. A check mode is a later flag (section 9).
- **Q7.** The keys have no effect, so the value cannot change behaviour. The diff and the backup protect the user. A `--force` would block a script for no gain.
- **Q8.** Rule: a new failure surface ships with its record. The backup holds the input. The log holds the decision and survives a deleted backup.
- **Q9.** The same folder gives the same permissions and the same disk. `--config` may name a file anywhere. The name follows the venue backup stamp.
- **Q10.** The check proves the result loads to an equal `Config`. `monitor: {}` is safe but it leaves a line with no meaning.
- **Q11.** The binary loads the config before it knows the command. A special case in `Config::load` costs more than one repeated line.
