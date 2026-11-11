# Headless `scope watch`: three follow-ups

- **Status:** accepted on 2026-11-11. The owner chose all three items for 0.8.8. The owner accepted the recommended option of every open question on 2026-11-11. Section 9 lists the decisions.
- **Scope:** three changes to `scope watch` and the engine it shares with the TUI grid.
  1. A target on the command line (`scope watch USDC --venue weex`).
  2. Memory between `--once` runs (a state file).
  3. Concurrent venue reads in one tick.
- **Follows:** `2026-11-10-headless-watch-design.md`. Its section 10 left these three items out and gave reasons. This document answers each reason.
- **Based on:** `origin/develop` at 57d7367.
- **No code.** This document is a design. File names and function names are proposals unless the text says "today".

## 1. Summary

| Item | One-line answer | Main risk |
|---|---|---|
| 1. Target on the command line | Share one set of target flags between `scope monitor` and `scope watch`. Build the same `Watchlist` that `watchlist_from_args` builds today. A target flag in the command line means "inline target". No target flag means "saved name or path". | A new flag that the "is it inline" check does not count. |
| 2. Memory between runs | One JSON file for each watchlist in `<home>/state/watch/`. It holds the memory of the two since-last-tick metrics for each market. An entry older than 3 intervals is dropped. A short file lock and a merge on write protect two cron runs. | A `--once` run can now breach on `volume_change_pct`. Before, it always read `n/a`. |
| 3. Concurrent reads | Split the read from the judging. Run the reads with a bound, one at a time for each venue, each with a timeout. Judge in the given order, with no shared mutable state. | More requests at once can trigger venue rate limits. |

### 1.1 Order of work

The three items touch the same files. Use three pull requests in this order.

1. **Item 3.** It splits `read_target` into a read step and a judge step. Item 2 uses the judge step.
2. **Item 2.** It adds the load and save of `History` and a new key.
3. **Item 1.** It is nearly independent. It touches the flags and the loader only.

Item 1 can go first or last. Items 3 and 2 must keep this order, to avoid two edits of `read_target` at the same time.

## 2. Item 1: a target on the command line

### 2.1 What the code does today

1. `scope watch` has one positional argument, `list`. It is a path when it ends in `.yaml` or has a path separator. Otherwise it is a saved name. (`is_watchlist_path`, `watchlist_path` in `crates/scope-cli/src/cli/watch/mod.rs`.)
2. `headless::run` (`headless.rs`) reads the file with `file::load(path, Some(&ctx.venues))`. It rejects a list with no targets. It applies `--every`, `--duration`/`--once`, `--csv` and `--report` over the file (`apply_flags`).
3. `scope monitor` already turns its arguments into a `Watchlist`. `watchlist_from_args` (`watch/monitor.rs`) does this. The TUI wizard uses it (`tui/mod.rs`, near line 714). The rules are:
   - With `--venue`: a `Pair` target. Its id is `--pair`, else the positional token. It gets the `market health` preset unless `--preset none`.
   - Without `--venue`: a `Token` target. Its id is `--token`, else the positional token. It gets two rules: `price_usd > 0` and `volume_24h >= 0`.
   - `--preset market-health` without a venue is an error.
   - `--chain` is set on the target when it differs from the default chain.
   - The health flags become `target.health`.
   - The config `monitor.alerts` become extra rules (price bounds; whale and volume spike when a venue is set).
4. The flags of `scope monitor` live in two structs in `monitor/mod.rs`: `MonitorArgs` (the positional token, `--token`, `--chain`, `--refresh`, `--export`, `--venue`, `--pair`) and `MonitorWatchArgs` (`--every`, `--duration`, `--csv`, `--report`, `--preset`, `--save`, `--yes` and the health flags).
5. `WatchArgs` defines its own `--every`, `--duration`, `--csv` and `--report`. Only the parsers are shared (`parse_interval`, `parse_run_duration`).
6. The `--chain` flag has a default value (`DEFAULT_CHAIN`). A flag with a default cannot tell "the user typed it" from "the default".
7. The TUI session command `watch <name>` reads the word as a name. `list`, `show`, `stop` and `open` are reserved words there. This does not change.

### 2.2 Proposal

#### 2.2.1 Command forms

```text
scope watch USDC --venue weex --once
scope watch USDN --venue weex --pair USDN_USDT --once
scope watch 0xA0b8...eB48 --chain ethereum --once
scope watch @dai-token --once --chain ethereum
scope watch 0xWallet... --kind address --rule "native_balance > 1" --once
scope watch pegs --once                     # a saved name, as today
```

The positional argument has a new meaning: a saved name, a path, or a token, pair or address. Section 2.2.3 says how the command decides.

#### 2.2.2 Mapping to the existing target types

The command uses the rules of `watchlist_from_args`. It does not add a second set of rules.

| Command | Target kind | `id` | `venues` | Checks |
|---|---|---|---|---|
| `USDC --venue weex` | `pair` | `USDC` | `[weex]` | preset `market health` |
| `USDC --venue weex,binance` | `pair` | `USDC` | `[weex, binance]` | preset `market health` (one row for each venue) |
| `USDN --venue weex --pair USDN_USDT` | `pair` | `USDN_USDT` | `[weex]` | preset `market health` |
| `0xA0b8... --chain ethereum` | `token` | the address | none | `price_usd > 0`, `volume_24h >= 0` |
| `@dai-token` | `token` | the label (`prepare` resolves it) | none | the same two rules |
| `0xWallet... --kind address --rule "..."` | `address` | the address | none | the `--rule` values only |

- A `--rule` is added after the preset and after the default rules. It uses `Rule::parse`, the parser of the file. The error text is the same as a file error.
- `--kind` is new. Its default is the rule above: `pair` with a venue, `token` without one. `--kind address` is the only value a user needs to type. A wallet has no market and no token data, so the user must give a rule.
- The target is checked like a file target: a rule must be valid for the kind (`TargetKind::accepts`), and every venue must be in `venue_ids()`.
- `prepare` runs after this step. It resolves `@label` and token symbols as it does for a file.
- The result is a `Watchlist` with one target. It goes through the same `apply_flags`, `prepare`, CSV, report and tick code as a file.

#### 2.2.3 Which flags it reuses

Both commands flatten one struct. The flag definitions exist once.

`TargetArgs` (new, in `cli/monitor/` or a neutral module) holds:

| Flag | Today in | Note |
|---|---|---|
| `--token CONTRACT` | `MonitorArgs` | unchanged |
| `--chain`, `-c` | `MonitorArgs` | becomes `Option<String>`. The default text moves into the help. Monitor reads `unwrap_or(DEFAULT_CHAIN)`. |
| `--venue` | `MonitorArgs` | unchanged |
| `--pair` | `MonitorArgs` | unchanged |
| `--preset` | `MonitorWatchArgs` | moves |
| health flags (`HealthArgs`) | `MonitorWatchArgs` | moves |
| `--rule` (repeatable) | none | new. Question Q3. |
| `--kind` | none | new. Question Q2. |

- The positional argument stays in each command, because its meaning differs. In `scope monitor` it is a token. In `scope watch` it is a name, a path or a token.
- The body of `watchlist_from_args` splits in two. One function builds the `Target` and its notes from the positional text, `TargetArgs` and the config. `watchlist_from_args` calls it and adds the monitor-only parts (`--refresh`, `--export`, `--save`). The headless loader calls it too.
- `scope monitor` does not use `--rule` or `--kind` in its wizard yet. With the shared struct, the wizard can fill them. This costs one line each.
- `MonitorArgs` is used in three places outside `monitor/mod.rs`: `interactive.rs`, `cli/mod.rs` and `watch/monitor.rs`. The move is small.
- Not reused, with reasons:
  - `--refresh`: it is `--every` in seconds. `watch` already has `--every`.
  - `--export`: it is the old name of `--csv`.
  - `--save`: it writes a file in the user's home from a cron command. See section 7.
  - `--yes`: it skips the wizard. `watch` has no wizard.
- The `--every`, `--duration`, `--csv` and `--report` flags are defined twice today (in `MonitorWatchArgs` and `WatchArgs`). This is not in scope. See section 7.

#### 2.2.4 How the command tells a name from a target

A token symbol can also be the name of a saved list (`USDC`). The command must not guess from the file system. If it did, saving a list named `USDC` would change what a cron job does.

The rule has one input: the command line.

1. If any flag of `TargetArgs` is set, the positional argument is an **inline target**.
2. Otherwise it is a **saved name or a path**, as today.

`TargetArgs::is_set()` is one pure function. A test lists every flag of the struct and checks that each one makes `is_set()` true (section 2.3, test 3).

Consequences:

- `scope watch USDC` is a saved name. If it does not exist, the error lists the saved names and adds: `To watch a token, give a venue or a chain: scope watch USDC --venue weex`.
- `scope watch USDC --venue weex` is an inline target, even when a saved list `USDC` exists. The command prints one note: `A saved watchlist named USDC exists. This run ignores it. Drop the target flags to run it.`
- `scope watch pegs.yaml --venue weex` and `scope watch ./pegs --chain base` are errors (exit 3). A path names a file. A file already holds its targets.
- The TUI session command is not affected. Its `watch <name>` still opens a saved list.

#### 2.2.5 Names, outputs and timing

- The inline list name is `cli-<id>` (for example `cli-USDN`). It appears in the `start` record, the report title and the log.
- `--once`, `--every`, `--duration`, `--csv`, `--append`, `--report` and `--format` work as for a file.
- With no `--once` and no `--duration`, the run lasts 1 hour. This is the default of `Watchlist::new` and of `scope monitor`. The start note prints it. Question Q5.
- The state identity of an inline list is a hash of its definition. Section 3.2.2.

#### 2.2.6 Exit codes

The five codes of the first design do not change. Each inline case maps to an existing code.

| Case | Code |
|---|---|
| All checks pass; warnings only | 0 |
| A check fails on the last tick | 1 |
| A venue error and no failed check | 2 |
| Unknown venue; unknown preset; `--preset market-health` with no venue; `--pair` with no venue; a bad `--rule`; a rule not valid for the kind; `--kind address` with no rule; a path with target flags; a saved name that does not exist and no target flag; `--chain` that is not a known chain; a bad `--every` | 3 |
| CSV or report cannot be written | 4 |

Every code 3 case makes no request. A usage error from clap still exits 2 (decision Q11 of the first design). This does not change.

### 2.3 Tests

Each test says why it exists. All tests use mock venues. None uses the network.

1. **Same target as `scope monitor`.** For each row of the table in 2.2.2, `scope watch <args>` and `scope monitor <args>` build equal `Watchlist` values (the name is ignored). *Why: a user who tries a target in the wizard must get the same watch in cron.*
2. **Mapping table.** One test for each form: kind, id, chain, venues, preset, rules. *Why: the table is the contract; a change must show in a test.*
3. **`is_set` counts every target flag.** The test reads the arguments of `TargetArgs` through `clap::CommandFactory`. It sets each one and checks `is_set()`. *Why: a new flag that `is_set` does not count would turn an inline target into a saved name with no error.*
4. **Name and target.**
   - `USDC` with no flag loads the saved list `USDC`.
   - `USDC --venue weex` runs inline when a saved `USDC` exists, and prints the note.
   - `pegs.yaml --venue weex` exits 3.
   - A missing name exits 3 and prints the hint.
   *Why: the cron job must mean one thing, whatever files exist.*
5. **Exit codes, end to end.** One test for each of 0, 1, 2, 3, 4 with an inline target. Each code 3 case in 2.2.6 has a test that counts zero requests on the mock server. The handler returns `Ok(code)` for each. *Why: the codes are the interface of the command.*
6. **Shared flags.** `MonitorArgs` and `WatchArgs` both contain the argument ids of `TargetArgs`. The help text of each shared flag is the same in both. *Why: this is the "share, do not copy" rule of the owner; a copy would drift.*
7. **Config alerts.** With `monitor.alerts.price_min` set, the inline target has a `mid >= min` rule, and the `start` record lists it. *Why: a rule the user did not type must be visible.* Question Q4.
8. **Round trip.** The canonical text in the `start` record (`file::to_text`) parses back to the same `Watchlist` with `file::parse`. *Why: a person can paste it into a file and get the same run.*
9. **Chain default.** `scope monitor USDC` (no `--chain`) behaves as before. `scope watch USDC --chain ethereum` is inline. *Why: `--chain` loses its clap default, so both cases need a test.*
10. The existing tests for `watchlist_from_args`, the wizard fill and the parity test pass unchanged.

### 2.4 RCA instrumentation

A cron job leaves no screen. When a job breaches at 03:00, the record must say what it watched. The `start` record (JSON) and the `tracing` event get these fields:

- `source`: `file` or `cli`.
- `argv`: the exact argument vector, as typed.
- `definition`: for `cli`, the exact canonical YAML of the effective watchlist (`file::to_text`). It includes the rules from the config alerts. It is text, not a summary.
- `notes`: the notes that `watchlist_from_args` returned (for example "alerts need a venue").
- `shadowed_saved_list`: the name, when a saved list of the same name exists.
- `config_alerts_applied`: the list of rules that came from `monitor.alerts`.

The text format prints the same facts on stderr in one `Watching ...` note.

### 2.5 Open questions

**Q1. How does the command tell a target from a saved name?**

- A. A target flag present means inline. No target flag means saved name or path. Print a note when a saved list of the same name exists.
- B. An explicit marker, for example `--target USDC`. The positional argument is always a name.
- C. Look for a saved list first. If none exists, treat the word as a token.

**Recommended: A.** It matches the example the owner gave (`scope watch USDC --venue weex`). The command line alone decides, so the meaning of a cron job cannot change when someone saves a file. C has the opposite flaw: it changes the meaning of a job when a file appears. B costs a flag for a case that A resolves.

**Q2. How does a user ask for a wallet (the address form)?**

- A. The default kind follows `watchlist_from_args` (pair with a venue, token without). `--kind address` selects a wallet.
- B. Infer the kind from the metrics of the `--rule` values. A rule on `native_balance` or `risk_score` means a wallet.
- C. No address form in this release.

**Recommended: A.** It mirrors the `kind:` field of the file, so there is one idea in both places. B makes the kind depend on the rule text, and a mixed rule set then needs its own error. C does not meet the request (pair and address forms).

**Q3. Does the shared flag set include `--rule`?**

- A. Yes. `scope monitor` gets `--rule` too (the wizard can fill it).
- B. No. A wallet has no checks, so the address form cannot work; token and pair forms use the defaults only.

**Recommended: A.** Without a rule flag, the address form is useless, and a rule is the point of a watch. One shared definition costs less than two. B is smaller but drops half of the request.

**Q4. Do the config `monitor.alerts` apply to an inline target?**

- A. Yes, as in `scope monitor`. The `start` record lists them.
- B. No. An inline target has only the flags on the command line.

**Recommended: A.** `watchlist_from_args` is the one definition of what `USDC --venue weex` watches. B would make `monitor` and `watch` disagree on the same words. The cost: a config edit changes a cron job. The `start` record shows the rule, so the cause is visible.

**Q5. What is the length of an inline run with no `--once` and no `--duration`?**

- A. 1 hour, as in `scope monitor` and `Watchlist::new`.
- B. One tick.
- C. Refuse with exit 3.

**Recommended: A.** One default for the same words in both commands. The start note says `for 1h`. The first design (Q2) already chose to warn and not to refuse.

## 3. Item 2: memory between `--once` runs

### 3.1 What the code does today

1. Two metrics compare a tick with the tick before: `largest_trade` and `volume_change_pct` (`MetricId::since_last_tick`, `crates/scope-core/src/metrics.rs`).
2. `History` (`crates/scope-core/src/watch/history.rs`) holds the memory. For each market it keeps two values:
   - `last_trade_ms`: the newest trade time seen on the tape.
   - `volume_24h`: the 24h volume of the last tick that had one.
3. The key is a string: the row label, for example `USDN@weex` (`Target::row_label`). `History::read` creates the entry with `or_default()`.
4. `run_ticks` (`watch/run.rs`) creates `History::default()` at the start of the run. It lives in a `RefCell` for the run. It is lost when the run ends. The grid and the headless command both use `run_ticks`.
5. A first tick has no memory:
   - `volume_change_pct` returns `NotApplicable("needs a second reading")`. This cell does not count against the verdict.
   - `largest_trade` reads the tape over the last `window_ms` before `now_ms`. `window_ms` is the watchlist interval (`Clock::now` in `fetch.rs`).
6. `--once` is one tick, so it is always a first tick. `volume_change_pct` is always `n/a` there, and `largest_trade` looks back only one interval. `--once` conflicts with `--every`, so the interval is the file value (60 s by default). A cron job that runs every 5 minutes reads only the last 60 seconds of trades.
7. `paths.rs` is the only place that knows the folder layout. `state_dir()` is `<home>/state`, described as "session, history and logs; safe to delete". Other files there: `session.yaml`, `resolutions.jsonl`, `tui-events.jsonl`.
8. The workspace has `serde_json`, `sha2` and `hex`. The toolchain is 1.99.1, so `std::fs::File::lock` and `try_lock` are available (stable since 1.89). No new crate is needed.

**Found while reading, not yet reproduced.** Two targets in one list with the same id and venue share one `History` entry, because the key is the row label. In one tick, the second target reads the memory that the first one has just updated. Its `volume_change_pct` would be 0 and its `largest_trade` would be 0. The file parser does not reject duplicate targets. The first test of this item must reproduce this on the current code before any fix. Question Q11.

### 3.2 Proposal

#### 3.2.1 What is stored, and where

- **Folder:** `<home>/state/watch/`. A new function `paths::watch_state_dir()` returns it, so `paths.rs` stays the only place that knows the layout.
- **One file for each watchlist:** `<slug>-<hash8>.json`. The lock file is `<slug>-<hash8>.lock` in the same folder.
  - `slug` is the list name with every character outside `a-z A-Z 0-9 - _` replaced by `-`. It is for people.
  - `hash8` is the first 8 hex characters of SHA-256 of the identity string. It makes the name unique.
- **Format:** JSON, one object.

```json
{
  "version": 1,
  "identity": "saved:pegs",
  "scope_version": "0.8.8",
  "written_at_ms": 1789000000000,
  "markets": [
    {
      "id": "USDN",
      "chain": null,
      "venue": "weex",
      "ordinal": 1,
      "last_trade_ms": 1788999990000,
      "volume_24h": 1234567.0,
      "updated_ms": 1789000000000
    }
  ]
}
```

- **Key** of a market: the watchlist (the file) + the target (`id`, `chain`, `ordinal`) + the venue. The `ordinal` is 1 for the first target with this id, chain and venue, 2 for the second, and so on. Question Q11.
- **Why JSON:** it matches the other state files, a person can read it, and `serde_json` is already a dependency. The file is small: two numbers for each market.
- **Why not one file for everything:** one file for each list means two different lists never write the same file. The lock then guards one list only.

#### 3.2.2 The identity of a watchlist

The identity string says which list a state file belongs to.

| Run | Identity | Reason |
|---|---|---|
| Saved name `pegs` | `saved:pegs` | The same list from any working directory. |
| Path `./pegs.yaml` | `path:<absolute path>` | A cron job has its own working directory. `std::path::absolute` gives a stable value (the headless code already uses it for `full_path`). |
| Inline target | `inline:<canonical YAML>` | Two inline jobs with different rules or markets do not share memory. |

A saved list and a path to the same file have different identities. This is on purpose: the code cannot know that they are one file without a file system lookup that may fail. The cost is one first tick when a user changes how they name the list.

A shared memory between two different jobs on the same market would be wrong. A job that runs every 5 minutes and a job that runs every hour would take turns writing `last_trade_ms`. The hourly job would then see only 5 minutes of trades.

#### 3.2.3 The run

1. After `prepare` and before the first tick, the command loads the file. It builds a `History` from the entries (section 3.2.4).
2. `run_ticks` takes the starting `History` as an argument. The grid passes `History::default()`. The grid does not use the state file. See section 7.
3. After each tick, the headless callback saves the state (section 3.2.6). It gets the `History` as one new argument of the callback.
4. The `start` record (JSON) and a `Watching` note say what was loaded (section 3.4).

State is on by default for every headless run, not only `--once`. A long run also resumes after a crash. `--no-state` turns it off for a run. Questions Q8 and Q9.

#### 3.2.4 The rule for stale state

An entry is **stale** when:

- `now_ms - updated_ms > STALE_AFTER_INTERVALS * interval`, or
- `updated_ms > now_ms` (the clock moved back).

`STALE_AFTER_INTERVALS` is a named constant, `3`. A stale entry is dropped at load. The market then has a first tick: `volume_change_pct` reads `n/a` and `largest_trade` looks back one interval. Question Q6.

The `interval` is the one that the run uses after the flags. This creates a problem. A cron job that runs every 5 minutes, with `interval` at its default of 60 s, has a stale limit of 3 minutes. Every entry is stale at the next run, and the memory never works.

Today `--once` conflicts with `--every`. This design allows `--every` with `--once`. With `--once`, `--every` does not sleep, because the run has one tick. It states **how far apart the runs are**:

- It sets the stale limit (3 times the value).
- It sets the look-back window of a first tick (`window_ms`).

So the cron line is `scope watch pegs --once --every 5m`. A user can set `interval: 5m` in the file instead. Question Q7.

Entries are kept for each market and not for the list. A venue that fails for many ticks keeps its old entry (the first design, section 9, point 6). The entry ages out by `updated_ms`. When the venue comes back after the limit, its first tick has no old reading.

#### 3.2.5 What `History` needs (scope-core, pure)

- `Memory` gets `updated_ms`. `History::read` sets it to `now_ms` when it changes the entry.
- `History::restore(entries, now_ms, max_age_ms)` returns the `History` and a list that says, for each entry, `loaded` or `stale` and its age.
- `History::entries()` returns the entries for saving.
- The key becomes a small struct (`id`, `chain`, `venue`, `ordinal`) with a stable text form. `read_target` builds it with `Target::market_key(venue, ordinal)`. The row label stays for display.
- The file code (read, write, lock, merge) lives in scope-cli, in a new module `watch/state.rs`. scope-core stays free of I/O, as `watch/mod.rs` says.

#### 3.2.6 Writing and two cron runs

Two runs of one list can overlap when a tick takes longer than the cron period.

The write is:

1. Take the exclusive lock on `<key>.lock` with `File::lock`. The critical section is short (a read, a merge and a rename). The OS drops the lock when the process ends, so a crash leaves no stale lock.
2. Read the file on disk. If it is not valid, treat it as empty.
3. **Merge** with the memory of this run, for each market key:
   - Take the entry with the larger `updated_ms`.
   - Set `last_trade_ms` to the larger of the two values.
4. Drop the entries for markets that the list no longer has, or that have no since-last-tick rule (section 3.2.7).
5. Write the result to `<key>.json.tmp` in the same folder, then `rename` it over `<key>.json`. A reader never sees half a file.
6. Release the lock.

What this gives:

- No corrupt file. The rename is atomic.
- No step back. A slow run that finishes late cannot replace newer memory with older memory, because the merge keeps the larger `updated_ms` and `last_trade_ms`.
- No blocking for the length of a tick. Only the write holds the lock.
- Two overlapping runs that both loaded the same old state report the same trades once each. This is a true reading by both. It is not a duplicate of an alert for one run.

What it does not give: a lock for the whole run. Two runs still both read the venue, and they still both append to one `--csv` file. Section 7 explains why this stays out. Question Q10.

A network file system may not support the lock. The code then reports the error as a write failure of the state (section 3.2.8). The atomic rename and the merge still protect the file.

#### 3.2.7 When the watchlist changes

| Change | Effect |
|---|---|
| A target is added | It has no entry. First tick for it. |
| A target or a venue is removed | Its entry is dropped at the next write. |
| A rule on `largest_trade` or `volume_change_pct` is removed from a target | Its entries are dropped at the next write. `History::read` is not called without such a rule. |
| A rule is added or its threshold changes | No effect on the memory. The memory holds market data, not the rule. |
| `interval` changes | The stale limit uses the new value at the next load. |
| `chain` of a target changes | The key has `chain`, so the old entry is not used. |
| The `id` changes (for example the pair) | New key. First tick. |
| The list is renamed | New identity. First tick. The old state file stays in the folder. It is safe to delete. |
| The file is replaced by a different list with the same name | Same identity. The entries apply by market key, which is correct, because the memory is about the market. |

No code removes an unused state file. A file is 1 KB or less. Question about cleanup: section 7.

#### 3.2.8 Failure cases

| Case | What happens |
|---|---|
| No state file | First tick. Not an error. The `start` record says `none`. |
| File cannot be parsed, or `version` is not 1 | The command moves the file to `<key>.json.bad` (one copy, the older copy is replaced). It starts with no state and prints a note. The `start` record says `discarded` and gives the reason. Question Q12. |
| State cannot be written (read-only folder, lock error) | A note on stderr, a field in the `end` record, a `tracing` event. The exit code does not change. Question Q9. |
| `--no-state` | No load, no write. The `start` record says `off`. |

### 3.3 Tests

All tests use a fresh scope home (the sandbox of `paths.rs`) and a controllable clock. The pure parts take `now_ms` as an argument, so they need no clock at all. No test sleeps.

**Pure (scope-core):**

1. **A restored `History` reads like the one that was saved.** Save after tick 1, restore, read tick 2: the result equals the result of two ticks in one `History`. *Why: the state file must not change the numbers.*
2. **Stale rule, at the edge.** An entry that is exactly 3 intervals old is kept. One millisecond more is dropped. An entry with `updated_ms` in the future is dropped. *Why: the constant is a contract; the clock-back case must not give a negative age.*
3. **A dropped entry gives a first tick.** `volume_change_pct` reads `NotApplicable`. `largest_trade` uses the window. *Why: a stale reading must never produce a fake percentage.*
4. **Duplicate targets.** First reproduce, on the current code, that two targets with one label share memory in one tick (expected: the second reads 0). Then show that with ordinals each has its own. *Why: Rule 17. The claim comes from reading the code; a test must prove it before a fix.*

**File (scope-cli):**

5. **Round trip.** Save, load, compare. The JSON keys are stable (a golden file for version 1). *Why: a change of format must bump `version`, and the golden file makes that visible.*
6. **Atomic write.** A failure after the temp file exists leaves the old file whole. *Why: a crash must not lose the memory.*
7. **Merge.** Two writers with different `updated_ms` leave the newer entry, and `last_trade_ms` never goes down. Run both orders. *Why: a slow run that ends late must not step the memory back.*
8. **Two real processes' worth of work.** Two tasks write at once under the lock. The file is valid, and every market has the larger `last_trade_ms`. *Why: the lock and the merge are what the cron case needs.*
9. **Corrupt file.** Garbage, a truncated file, `version: 2`. Each moves the file aside as `.bad` with the exact bytes, runs as a first tick, and reports `discarded`. *Why: a bad cache must not stop a watch, and the bytes are the evidence.*
10. **Unwritable state folder.** The run ends with the exit code of the checks, a note, and `state_written: false` in the `end` record. *Why: the state is a cache. Exit 4 must mean the CSV or report.*
11. **Identity.** Saved name, path (two working directories give one identity) and inline definition. Two inline lists with different rules have different files. *Why: shared memory between two jobs would give wrong readings.*
12. **Watchlist change.** Each row of the table in 3.2.7 has a test on the entries that remain.

**End to end (mock venue, two `--once` runs):**

13. **The reason for the feature.** Run 1 reads volume 1000. Run 2 reads volume 1500. The `volume_change_pct` cell of run 2 is `50`, and a rule `volume_change_pct <= 40` breaches (exit 1). Without the state file, the same run 2 reads `n/a` and exits 0. *Why: this is the cron case that the first design left out.*
14. **`largest_trade` covers the gap.** Between two runs, a large trade occurs 4 minutes before run 2, and the interval is 60 s. With the state, run 2 sees it. With `--no-state`, it does not. *Why: the look-back window of 60 s is too short for a cron period of 5 minutes.*
15. **Stale.** A run with an entry older than 3 intervals acts as a first tick. The `start` record says `stale` with the age.
16. **`--no-state`** reads and writes nothing.
17. **`--once --every 5m`** parses. `--once --duration 1h` is still a usage error. The look-back window and the stale limit use 5 minutes.
18. The existing tests for the grid, `run_ticks` and `csv_rows` pass. The grid passes `History::default()` and writes no file.

### 3.4 RCA instrumentation

When a cron job reports `n/a` or a surprising `volume_change_pct`, the record must say which memory the run used. Each item below is recorded:

- **In the state file:** `identity`, `scope_version`, `written_at_ms`, and `updated_ms` for each market. A person can read the file and see what the next run will use.
- **In the `start` record and the `tracing` event:** `state.path`, `state.mode` (`on` or `off`), and one record for each market: `row`, `basis` (`loaded`, `stale`, `none`), `age_ms`, `stale_limit_ms`, `last_trade_ms` and `volume_24h` as loaded. `state.discarded` holds the reason and the path of the `.bad` file when the file was bad.
- **In the `end` record:** `state_written` (true or false), `state_error` (the exact error text), and the number of entries written and dropped.
- **For `largest_trade`:** a `tracing` field says whether the tape covers the gap. The tape holds `TRADE_SAMPLE` (500) trades. On a busy market, a gap of 5 minutes can hold more trades than that. The oldest trade on the tape is then newer than `last_trade_ms`, and the reading is a lower bound. The field is `tape_covers_gap`. The metric itself does not change. See section 7.
- **The bad file itself**, kept as `<key>.json.bad` with its exact bytes.

### 3.5 Open questions

**Q6. How old may an entry be?**

- A. 3 intervals. A named constant.
- B. 2 intervals.
- C. A flag (`--state-max-age`).

**Recommended: A.** One missed cron run (two intervals of gap) still works. Two missed runs reset the memory. A longer limit would compare two readings that are too far apart to mean "since the last tick". C adds a flag before any user needs it. The constant is easy to change.

**Q7. Does `--once` accept `--every`?**

- A. Yes. With `--once`, `--every` sets the gap between runs: the stale limit and the look-back window.
- B. No. The interval comes from the file only (`interval: 5m`).

**Recommended: A.** A cron user then writes the whole job on one line. Without A, the file must carry a value that is really a property of the cron table. The risk is a user who thinks `--every` makes `--once` sleep. The help text says what it does.

**Q8. Which runs use the state?**

- A. Every headless run, with `--no-state` to turn it off.
- B. Only `--once` runs.
- C. Only when the user asks (`--state`).

**Recommended: A.** One rule. A long run that crashes also resumes. The feature works from cron with no extra flag. The cost: a `--once` run that always passed (because `n/a` does not count) can now breach. The release notes must say so. C would leave the cron case without its fix unless the user knows the flag.

**Q9. Does a state write failure change the exit code?**

- A. No. Print a note, record it in the `end` record, keep the code.
- B. Yes, exit 4.

**Recommended: A.** `state/` is "safe to delete" in `paths.rs`. Exit 4 means "the record of the run is lost", and the state is not the record. A cron alarm for a cache is noise. The failure is not silent: the note, the `end` record and the log all show it.

**Q10. How do two overlapping runs of one list behave?**

- A. A short lock around the write, plus a merge. No lock for the whole run.
- B. A lock for the whole run. A second run exits with code 3 ("already running").
- C. No lock. The last writer wins.

**Recommended: A.** It cannot corrupt the file and it cannot step the memory back. It never blocks a run. B also protects the `--csv` file, but it adds a skipped-run policy that no one asked for: a skipped cron run is a missing reading. C is the smallest, but the late writer can erase newer memory.

**Q11. How should the state tell two targets with the same id and venue apart?**

- A. Add an `ordinal` to the key (1 for the first, 2 for the second). `History` uses the same key, so the grid is fixed too.
- B. Reject a list with duplicate targets (exit 3, and a file error).
- C. Leave the key as the row label.

**Recommended: A.** A duplicate is valid today, so B can break saved lists. C keeps the problem in section 3.1. A is one number in the key. The grid and the headless command share the engine, so the fix applies to both. It is a separate defect fix, and it comes with test 4.

**Q12. What happens to a state file that cannot be read?**

- A. Move it to `<key>.json.bad`, run as a first tick, print a note.
- B. Delete it and run as a first tick.
- C. Stop with exit 3.

**Recommended: A.** The bytes stay as evidence (Rule 13). The watch goes on. C stops a cron job because of a cache. B loses the evidence.

## 4. Item 3: concurrent venue reads in one tick

### 4.1 What the code does today

1. `tick` (`watch/fetch.rs`) loops over the targets in order. For each target it calls `read_target` and awaits it. The tick stays on one task. Nothing runs at the same time.
2. `read_target` reads in this order, all with `.await` one after the other:
   1. The token source (`read_token`), if a rule needs it.
   2. The address source (`read_address`), if a rule needs it.
   3. For each row of the target, the market (`read_market`). One row is one venue.
3. In the same loop, `read_target` also judges: it builds the cells, and it calls `history.read(...)` for the since-last-tick metrics. `history` is `&mut History`. So the read and the judge are mixed in one function.
4. `read_market` builds the venue (`MarketVenue::from_name_with_http`) and calls `fetch_summary_with_trades` (scope-core). That makes several requests: the order book and the volume (`fetch_book_and_volume`), then the trades (`fetch_pattern_trades`).
5. There is no timeout around a read. The only limits are per request: the venue descriptor has `timeout_secs` (default 15 s in `configurable_client.rs`), and the native transport has 30 s. A tick takes at most the sum of all reads.
6. `Clock::now(ready)` is taken once at the start of the tick. All rows share `now_ms`.
7. A failed read does not stop the tick. It makes the cells of that source `Error`, with the exact error text. The row is then `Unknown` (exit 2).
8. The repeat runner (`cli/repeat.rs`) uses `tokio::time::Instant` for its clock and `tokio::time::sleep_until`. A tick that takes longer than the interval moves the schedule grid. Ticks never overlap.
9. `Tick.summaries` is aligned with the rows of `snapshot.targets`. `report_text` zips them.
10. **A precedent exists.** `market summary --venue a,b` (`cli/market_venues.rs`, `read_all`) reads venues together with `futures::future::join_all` and keeps the order of the legs. It has no bound. Its test `venues_are_read_at_the_same_time` uses real TCP stubs that hold each connection until the other venue connects, with a 5 s limit on the wall clock. The `futures` crate is already a dependency of scope-cli.
11. The tests of `tick` use mock chain clients and a mock HTTP server. The transport is a trait (`HttpClient::send`) that `ChainClientFactory::http()` returns. A test can give its own implementation.
12. The first design (section 10) left this out for one reason: "a change needs its own timing tests". This section gives them.

### 4.2 Proposal

#### 4.2.1 Two steps: read, then judge

Split `read_target` in two. Each step has one job.

1. **Read** (async, no shared mutable state). It takes the target, the contract and the sources. It returns the raw result of each source: token result, address result, and one market result for each venue (`Result<(MarketSummary, trades), String>`). It does not touch `History`.
2. **Judge** (sync, in the given order). It takes the raw results and `&mut History`. It builds the cells, calls `history.read`, and returns the `TargetReading`. This is the code of `read_target` after the awaits, unchanged.

`tick` becomes:

1. List the read jobs, in the given order: for each target, in order, one token job, one address job, and one market job for each venue. A job has an index.
2. Run the jobs with a bound and a timeout (section 4.2.2).
3. Put the results back in index order.
4. Judge target by target, in the given order, with the one `&mut History`.

The signature `tick(ready, src, out, &mut History) -> Tick` does not change. The grid and the existing tests call it as before. A second function, `tick_with(..., limits)`, takes the limits. `tick` calls it with the defaults. Tests call it with small values.

Because the judge step is sequential and in order, `History` needs no lock and no `Arc`. The results do not depend on which read finished first. This is what keeps the output equal to the sequential output.

#### 4.2.2 The bound

- A named constant, `MAX_CONCURRENT_READS = 8`, in `fetch.rs`. It limits the jobs that run at the same time in one tick.
- **One read at a time for each venue.** The jobs for one venue run one after the other, in the given order, as they do today. Different venues run at the same time. A job group key is:
  - `venue:<id>` for a market job,
  - `analytics` for token jobs (all go to the same data source),
  - `address:<chain>` for address jobs.
- Why the per-venue rule: ten targets on one venue would otherwise send ten requests at the same moment to one host. Exchanges limit requests by IP and answer with an error. A rate-limit error is an `Error` cell, which makes exit 2. Concurrency would then turn a slow watch into a wrong watch. With one read at a time for each venue, the request pattern to each host stays the same as today. Only the waiting between hosts goes away. Question Q14.
- A DEX venue and the token source may use the same data host. The bound of 8 is the limit for that case. See the risks.

#### 4.2.3 How results keep the given order

- Each job carries its index. The runner uses `futures::stream::iter(groups).map(run_group).buffer_unordered(MAX_CONCURRENT_READS)`. Each group returns `(index, result)` pairs. The runner writes each pair into a vector at its index.
- After the stream ends, the vector holds every result in the given order. The judge step reads it by index.
- `buffer_unordered` plus the index is chosen over `buffered`. `buffered` returns results in order, but a completed result waits in the queue until the earlier ones finish, and it keeps its slot. One slow venue at the head would hold the slots. With the index, a finished group frees its slot at once.
- The order is the same for every run: targets in list order, rows in `Target::rows()` order. `snapshot.targets`, `summaries`, the CSV rows and the JSON rows keep the order they have today.

#### 4.2.4 Timeouts

- A named constant, `READ_TIMEOUT = 60 s`. Each job runs under `tokio::time::timeout(READ_TIMEOUT, ...)`.
- On timeout the job future is dropped, which cancels its requests. The result is `Err("read timed out after 60s")`. The judge step turns it into `Error` cells like any other read error. The row is `Unknown`. The exit code is 2, as for any venue error (design 1, section 8.2).
- Why 60 s: one venue read makes several requests, and each can wait up to 15 s (descriptor default). 60 s is longer than the common case and shorter than a hung connection. The timeout is a backstop for a read that never answers. The request timeouts stay the first limit.
- The timeout does not depend on the interval. A short interval does not cut a slow but working venue. A tick that is longer than the interval already moves the schedule grid (point 8 above). Question Q15.
- A timed-out job does not cancel the other jobs.
- No retry. The next tick is the retry. A retry inside a tick would change the length of the tick and hide the error.

#### 4.2.5 What else changes

- **The grid gets it too.** The engine is shared. This is the point of one engine. The grid shows a tick sooner when it watches many venues.
- **Output from a read.** `CrawlDexSource` and `fetch_analytics_for_input` receive `&Output`. Two reads at the same time may write two notes. `Output` takes `&self`. Check when building that a note is written as whole lines, so two notes never mix inside a line. The tests of design 1 (item 14: stdout holds only data) apply as they are.
- **No prompt in a read.** `prepare` resolves symbols before the first tick, so a read does not ask. Check when building that no read path can prompt (`CrawlDexSource` with a pair target and no contract calls `resolve_dex_token`). Two prompts at once would be a defect. If a path can prompt, the read must fail with an error.
- **Cancellation.** The stop future of the headless command drops the whole `tick`. With the jobs inside it, all reads drop with it. No task is spawned, so nothing runs on after the drop. This is why the design uses a stream in the same task, and not `tokio::spawn`.

### 4.3 Tests

**No test uses the wall clock.** Two tools make this possible:

- `#[tokio::test(start_paused = true)]`. Time moves only when every task waits. `tokio::time::sleep` then advances the virtual clock at once. The repeat runner already uses it (`repeat.rs`, `market.rs`).
- A test transport that implements `HttpClient::send`. It answers a request after `tokio::time::sleep(latency)`, with a latency set for each venue. It keeps an atomic counter of requests in flight and the highest value it saw. The test gives it to `tick_with` through the `ChainClientFactory::http()` of the test factory. The real `read_market` and the real venue mapping run. This proves that the real path is concurrent, and not a mock of it. (Confirm when building that the transport can serve the order book, ticker and trades responses of the built-in venues. The mockito fixtures of `fetch.rs` tests show the shapes.)

Elapsed time is measured with `tokio::time::Instant`, so it is exact. The tests compare equal numbers, not ratios.

1. **Reads run at the same time.** 4 venues, each with latency L, bound 8. The tick takes exactly L of virtual time. *Why: this is the feature. Sequential reads take 4L.*
2. **The bound holds.** 6 venues, bound 2, latency L. The tick takes exactly 3L. The transport saw at most 2 requests at once (and saw 2). *Why: the bound is the protection against too many requests. The test must fail if the bound is 1 (3L becomes 6L) or 6 (L).*
3. **One read at a time for each venue.** 3 targets on one venue and 1 on another. The highest in-flight count for the first venue is 1. The tick takes 3L. *Why: this keeps the request pattern per host as it is today, so rate limits do not change.*
4. **Order does not depend on speed.** Latencies in reverse (the first venue is the slowest). `snapshot.targets` and `summaries` are in list order. *Why: the report and the CSV zip rows by position.*
5. **Equal to the sequential result.** Run the same list with bound 1 and with bound 8. The two `Tick` values are equal, cell by cell. Repeat for two ticks with `volume_change_pct` and `largest_trade` rules, so the `History` is part of the proof. *Why: concurrency must change the time of a tick, never its result.*
6. **Timeout.** One venue never answers. At exactly `READ_TIMEOUT` the cell is `Error("read timed out after 60s")`. The other venues are normal. The tick takes `READ_TIMEOUT`, not forever. The headless exit code is 2. *Why: a hung venue must not stop a cron job. The error text is evidence.*
7. **A slow or failed read does not stop the others.** One venue fails at once, one is slow, one is fast. All three results are present. *Why: this is the rule "a failed read does not stop the tick".*
8. **No overlap of ticks.** A tick that takes longer than the interval does not start the next tick before it ends. The grid moves, as it does today. *Why: concurrency inside a tick must not become concurrency between ticks.*
9. **Cancel.** The stop future resolves in the middle of a tick. The in-flight counter goes to 0, and no request is sent after that. *Why: a signal must stop the process quickly, with no read left behind.*
10. **Different `READ_TIMEOUT` and bound in `tick_with`.** Tests pass small limits. A test checks that the defaults are the named constants. *Why: tests must not wait 60 virtual seconds to prove a rule about small limits, and the defaults must not drift.*
11. **The existing tests** for `tick`, the grid, `csv_rows`, `report_text` and the headless exit codes pass unchanged.

### 4.4 RCA instrumentation

When a tick is slow, or a row shows a timeout, the record must say which read was slow. Each read job records:

- the row label (or `token`, `address`) and its group key,
- `queued_ms` (offset from the start of the tick when it was ready to run) and `started_ms`,
- `took_ms`,
- `outcome`: `ok`, `error` or `timeout`, and the exact error text.

These go to `tracing` events (visible with `-v`). The JSON `tick` record also gets one field, `timing`:

```json
"timing": {
  "wall_ms": 1210,
  "sum_ms": 3470,
  "bound": 8,
  "reads": [
    {"row": "USDN@weex", "group": "venue:weex", "started_ms": 0, "took_ms": 1210, "outcome": "ok"},
    {"row": "USDN@binance", "group": "venue:binance", "started_ms": 0, "took_ms": 60000, "outcome": "timeout"}
  ]
}
```

`wall_ms` against `sum_ms` shows how much time the concurrency saved. The field is new and additive. A reader of the old `tick` fields is not affected. The CSV bytes do not change. Question Q16.

### 4.5 Open questions

**Q13. What is the bound, and can the user change it?**

- A. A constant, 8. No flag.
- B. A config key.
- C. A flag on `scope watch`.

**Recommended: A.** No user has asked to change it. A watchlist with more than 8 venues is rare. A flag would be a new public setting to keep. The constant is named, so a later change is one line. The grid would also need the setting for B.

**Q14. Does one venue get one read at a time?**

- A. Yes. Different venues run at the same time; one venue's jobs run one after the other.
- B. Only the global bound.

**Recommended: A.** The exit code depends on venue errors. An exchange rate limit is a venue error. B can turn a working watch into exit 2 when a list has many targets on one venue. A keeps today's request pattern for each host. The gain is smaller for a list with one venue, but this case was not slow because of waiting between venues.

**Q15. How long is the timeout of one read?**

- A. A constant, 60 s.
- B. Derived from the interval (for example, the interval).
- C. None. Rely on the request timeouts.

**Recommended: A.** B would cut a working but slow read when the user picks a short interval, and would give a different result for the same venue. C leaves the tick open to a hung read that the request timeout does not catch (several requests each of 15 s can add up, and a stuck connection may not honour the timeout). 60 s is above the normal case.

**Q16. Does the JSON `tick` record carry the timing of the reads?**

- A. Yes, in a `timing` field. It is also in `tracing`.
- B. Only in `tracing`.

**Recommended: A.** A cron job usually saves the JSON log and not the `-v` output. A person who looks for a slow venue needs the number in that log. The field is additive.

## 5. How the three items work together

- A `--once` inline run (item 1) with the state file (item 2) has an identity from its definition. The first run has no state. The second run has it. The `start` record shows both facts.
- The state is saved after the judge step (item 3), in the given order. The concurrency has no effect on what is saved.
- The `start` record gets new fields from items 1 and 2: `source`, `argv`, `definition`, `state`. The `tick` record gets `timing` from item 3. The `end` record gets `state_written` and `state_error`. All are additive. The stdout contract of design 1 holds: stdout has data only. Notes go to stderr.
- The exit codes do not change. Items 1 and 3 map their new failures to codes 2 and 3. Item 2 adds no code.

## 6. Risks

1. **A `--once` run can now breach.** A rule on `volume_change_pct` was always `n/a` in a `--once` run. With the state file it can fail. A cron job that passed can exit 1. The release notes must say this. `--no-state` restores the old result. Question Q8.
2. **More requests at the same time.** The bound and the per-venue rule limit this. A list with 8 different venues sends 8 requests at the same time, which is a new pattern for the user's network. Question Q13.
3. **A shared data host.** A DEX venue market read and the token source may both use DexScreener. They are in different groups and can overlap. The global bound of 8 limits it. If rate limits appear, group them by host.
4. **`--chain` loses its clap default.** Monitor help no longer shows the default through clap. The help text must say it. Test 9 of item 1 covers the behaviour.
5. **A wrong guess at "inline".** A new target flag that `is_set` does not count turns a target into a saved name. Test 3 of item 1 guards it.
6. **A partial `largest_trade`.** The tape holds 500 trades. A long gap on a busy market can hold more. The reading is then a lower bound. This also happens in a long run with a long interval today. The `tape_covers_gap` field records it. Section 7 explains why it stays out.
7. **Clock changes.** The state uses wall time (`now_ms`). A clock that moves back drops the entry (test 2 of item 2). A clock that moves forward by hours drops the entry by the stale rule. Both give a first tick, which is the safe result.
8. **A state file on a network file system.** The lock may not work. The atomic rename and the merge still protect the file.

## 7. Left out, and why

| Item | Why it is not in this design |
|---|---|
| `--save NAME` on `scope watch` | It writes a file in the user's home from a command made to run unattended. `scope monitor ... --save NAME` does it already, and the inline `start` record prints the canonical definition that a person can save. Add it if users ask. |
| State for the TUI grid | The grid is one long process. It keeps its `History` in memory. No one asked for memory between grid sessions, and a grid that resumes old readings would confuse a person who watches live. The grid passes `History::default()`. |
| A lock for the whole run, and for the `--csv` file | Question Q10. The old reason ("a lock file needs a policy for stale locks") does not hold for an OS lock: the OS frees it when the process ends. The remaining reason is a policy question: what a skipped run means. No one asked for it. Item 2 shows that the code for it is small, so a later change is cheap. |
| Removal of unused state files | A file is 1 KB or less, `state/` is safe to delete, and a rule for age or count would be a new policy. The `end` record counts the entries. Add a cleanup if the folder grows in use. |
| A fix for a partial `largest_trade` when the tape is too short | It needs a way to ask a venue for trades since a time. The venue trait has only "the last N". This is a change to every venue. The same limit exists in a long run today. The design records it (`tape_covers_gap`) so the cause is visible. |
| A retry after a timeout | A retry changes the length of the tick and hides the error. The next tick is the retry. |
| A bound or a timeout that the user sets | Questions Q13 and Q15. No user has asked. Constants are easy to expose later. |
| Grouping by host and not by venue | It needs a map from venue to host that the code does not have. The group keys in section 4.2.2 are a good approximation. Revisit if rate limits appear (risk 3). |
| One shared definition of the four flags `--every`, `--duration`, `--csv` and `--report` | `MonitorWatchArgs` and `WatchArgs` define them twice, with different help text and different conflicts (`--refresh` and `--once`). This is outside the three items. It is a cleanup, and the owner decides whether to do it. This design only reuses the target flags, as asked. |
| Notifications, `--count N`, `--strict`, `--quiet` | Unchanged from section 10 of the first design. Nothing here changes the reasons. |
| The session command `watch <name>` in the TUI taking a target | The TUI has the wizard (`scope monitor` fills it from the same flags). A second path would copy it. |

## 8. All open questions

| # | Question | Options | Recommended |
|---|---|---|---|
| Q1 | How does the command tell a target from a saved name? | A flags decide, with a note on a name clash. B explicit marker. C saved list first. | A |
| Q2 | How does a user ask for a wallet? | A `--kind address`. B infer from the rule. C no address form. | A |
| Q3 | Is `--rule` in the shared flags? | A yes, `monitor` gets it too. B no. | A |
| Q4 | Do config `monitor.alerts` apply to an inline target? | A yes, listed in `start`. B no. | A |
| Q5 | Length of an inline run with no `--once` and no `--duration`? | A 1 hour. B one tick. C refuse. | A |
| Q6 | How old may a state entry be? | A 3 intervals. B 2 intervals. C a flag. | A |
| Q7 | Does `--once` accept `--every`? | A yes, it sets the gap between runs. B no. | A |
| Q8 | Which runs use the state? | A every headless run, `--no-state` to turn off. B only `--once`. C opt in. | A |
| Q9 | Does a state write failure change the exit code? | A no. B exit 4. | A |
| Q10 | How do two overlapping runs behave? | A short lock and merge. B whole-run lock. C no lock. | A |
| Q11 | How are two targets with the same id and venue told apart? | A ordinal in the key. B reject duplicates. C leave it. | A |
| Q12 | What happens to an unreadable state file? | A move to `.bad`. B delete. C exit 3. | A |
| Q13 | What is the bound, and can the user change it? | A constant 8. B config key. C flag. | A |
| Q14 | One read at a time for each venue? | A yes. B global bound only. | A |
| Q15 | How long is the timeout of one read? | A constant 60 s. B from the interval. C none. | A |
| Q16 | Does the JSON `tick` carry read timing? | A yes. B tracing only. | A |

## 9. Decisions

The owner accepted the recommended option of every question on 2026-11-11. One decision (D1) was added on the same day.

| # | Question | Chosen option | Date |
|---|---|---|---|
| Q1 | How does the command tell a target from a saved name? | A. Flags decide, with a note on a name clash. | 2026-11-11 |
| Q2 | How does a user ask for a wallet? | A. `--kind address`. | 2026-11-11 |
| Q3 | Is `--rule` in the shared flags? | A. Yes, `monitor` gets it too. | 2026-11-11 |
| Q4 | Do config `monitor.alerts` apply to an inline target? | A. Yes, listed in `start`. | 2026-11-11 |
| Q5 | Length of an inline run with no `--once` and no `--duration`? | A. 1 hour. | 2026-11-11 |
| Q6 | How old may a state entry be? | A. 3 intervals. | 2026-11-11 |
| Q7 | Does `--once` accept `--every`? | A. Yes, it sets the gap between runs. | 2026-11-11 |
| Q8 | Which runs use the state? | A. Every headless run, `--no-state` to turn off. | 2026-11-11 |
| Q9 | Does a state write failure change the exit code? | A. No. | 2026-11-11 |
| Q10 | How do two overlapping runs behave? | A. Short lock and merge. | 2026-11-11 |
| Q11 | How are two targets with the same id and venue told apart? | A. Ordinal in the key. | 2026-11-11 |
| Q12 | What happens to an unreadable state file? | A. Move to `.bad`. | 2026-11-11 |
| Q13 | What is the bound, and can the user change it? | A. Constant 8. | 2026-11-11 |
| Q14 | One read at a time for each venue? | A. Yes. | 2026-11-11 |
| Q15 | How long is the timeout of one read? | A. Constant 60 s. | 2026-11-11 |
| Q16 | Does the JSON `tick` carry read timing? | A. Yes, in a `timing` field. | 2026-11-11 |
| D1 | Do `MonitorWatchArgs` and `WatchArgs` keep their own copies of `--every`, `--duration`, `--csv` and `--report`? (Section 7 left this to the owner.) | No. The four flags are merged into shared flags. The merge is part of the CLI-target PR (item 1). | 2026-11-11 |
