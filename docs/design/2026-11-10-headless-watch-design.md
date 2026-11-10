# Headless `scope watch`: Design

- **Status:** proposed on 2026-11-10. Waits for the decisions in section 11. Issue #123.
- **Scope:** `scope watch <name|path>` runs a saved watchlist without the TUI. It prints each tick, writes the CSV trail and the report, and ends with an exit code that a script can read.
- **Follows:** section 4 of `2026-10-10-tui-watchlist-design.md` (the approved design), decision Q4 of `2026-10-16-tui-parity-design.md` (headless mode ships after #47), and the watchlist engine `scope::watch`.
- **Out of scope:** section 10.
- **Built on the code after the removal of `scope monitor --dashboard`.** This design does not use `run_direct` or `args.watch.dashboard`.

## 1. Goal

A user wants to run a watchlist from cron, CI or a script. Today only the TUI grid runs a watchlist. The grid needs a terminal, and it has no exit code.

The new command runs the same engine and the same runner as the grid. It has no screen. The exit code tells the script what the last tick found.

## 2. What exists today

1. `scope::watch::file::load(path, Some(venue_ids))` reads and validates a watchlist file. It reports every error with its line.
2. `cli::watch::prepare` resolves `@label` targets and token symbols once, before the first tick. A symbol on many chains asks the user.
3. `cli::watch::run` runs the ticks on the watchlist schedule with `repeat::run`. It writes the CSV trail and the report. A callback gets each tick and its events.
4. A tick reads every row of every target. A row is one target on one venue. A failed read does not stop the tick: it makes the cells of that source `Error`, with the exact error text.
5. Each row has a status: `Breached` (a cell failed), `Warned`, `Unknown` (a cell is `Error`, no cell failed) or `Passed`.
6. `dispatch` returns `Result<i32>`. Only the TUI returns a code other than 0 today. Any `Err` becomes exit code 1 in `main`.
7. `repeat::run` does not handle signals. The TUI stops a run by dropping the future.
8. `watch::run` writes the report only when the loop ends. A dropped future writes no report.

## 3. Command

```text
scope watch <NAME|PATH> [--once] [--every D] [--duration D]
                        [--format text|json|csv]
                        [--csv PATH] [--append] [--report PATH]
```

| Flag | Meaning |
|---|---|
| `<NAME\|PATH>` | A saved watchlist name, or a file path. Section 4. |
| `--once` | Run one tick and stop. This is the mode for cron and CI. It conflicts with `--every` and `--duration`. |
| `--every D` | Time from one tick start to the next. Overrides `interval:` in the file. |
| `--duration D` | How long the run lasts. `forever` runs until stopped. Overrides `duration:` in the file. |
| `--format` | What goes to stdout. Default `text`. Section 6. |
| `--csv PATH` | Write one row for each cell and tick. Overrides `outputs.csv`. |
| `--append` | Keep the rows already in the `--csv` file. Open question 10. |
| `--report PATH` | Write the report when the run ends or stops. Overrides `outputs.report`. |

- A flag overrides the file. The file overrides the default (`60s`). `D` uses the one parser of the watchlist file (`30s`, `5m`, `1h`, `2d`, `forever`).
- The timing code of `scope monitor` (`--every`, `--duration`) and the new command share one function. There is no second parser.
- The command never reads the config `output.format` and ignores the global `--ai` flag. It has its own `--format`.
- The command never prompts. A question that needs an answer is an error (section 5, step 5).
- The command is not interactive in the sense of `Commands::is_interactive`: it gets the version line on stderr.
- `--help` lists the five exit codes, as `market summary --venue` does.

### 3.1 In the TUI

`watch` has a TUI route, so the parity test passes. The session command `watch <name|path>` opens the grid, the same as `watch open <name|path>`. The shell pane never runs a headless loop. Open question 8 decides which flags it accepts.

The TUI already handles the word `watch` before clap sees it. The new `route` arm is reached only by the parity test. It must compile, and its text must point to `watch open`.

## 4. Reading a saved watchlist

1. The argument is a path when it ends in `.yaml` or contains a path separator. Otherwise it is a name in `scope::paths::watchlists_dir()`. The grid session command uses this rule today (`tui/exec.rs`). The rule moves to one shared function, and both callers use it.
2. The file loads with `file::load(path, Some(&venue_ids()))`. The venue check needs the venue catalog. The headless branch of `dispatch` calls the same catalog step as `dispatch_with` does for `needs_venues` commands.
3. A list with no targets is not valid. `file::parse` does not check this today. The headless loader checks it.
4. A file error prints to stderr as `<path>:<line>: <message>`, one line for each error. A missing name also lists the saved names (`file::list`).
5. Relative paths in `outputs:` resolve against the working directory of the process. A cron job has its own working directory. The note at the start of the run prints the paths it uses.

## 5. Order of a run

1. Parse the flags. clap reports a usage error. See question 11.
2. Resolve the argument to a path (section 4).
3. Load and validate the file. On error: print it, exit 3. No request has been made.
4. Apply the flags over the file. A bad `--every` or `--duration` is exit 3.
5. `prepare`: resolve `@label` targets and token symbols. The output has prompts off. A symbol on many chains is an error that tells the user to put the contract in the file. On error: exit 3. See question 7.
6. Open the CSV file and write the header. On error: exit 4. No request has been made for a tick.
7. Print the `start` record (section 6).
8. Run the ticks (section 7). For each tick: fetch, diff, append to the CSV, print the tick.
9. Stop (section 7). Write the report. On error: exit 4.
10. Print the `end` record. Return the exit code.

The handler always returns `Ok(code)`. It never returns `Err`. An `Err` would reach `main` and exit with 1, and 1 means "breach" here. A panic exits with 101. The doc of the command says so.

## 6. What it prints

Data goes to stdout. Notes go to stderr. This is the split of `Output`. The `--- Run #N ---` lines of the repeat runner, the version line and the stop note are notes. Code writes only through `Output`, so `tests/one_output_path.rs` needs no new allowed entry.

The command flushes stdout after each tick. A pipe reader then sees each tick when it ends.

### 6.1 `--format text` (default)

```text
12:00:05 run 1 · 3 rows · 1 breached · 0 warned · 1 unknown · 1 passed
  BREACH   USDN@weex: Spread: 3.4%, max 3%
  ERROR    USDN@binance: market health: HTTP 503 from binance
12:01:05 run 2 · 3 rows · 0 breached · 0 warned · 1 unknown · 2 passed
  CLEARED  USDN@weex: Spread passes again
  ERROR    USDN@binance: market health: HTTP 503 from binance
```

- One summary line for each tick. Then one line for each event of the tick (`BREACH`, `CLEARED`, `WARN`, `WARN CLEARED`). The text of an event is `Event::banner`.
- `diff` makes events for failures and warnings only. An `Error` cell is not an event. So the text mode prints one `ERROR` line for each `Error` cell on every tick. Without it, exit code 2 has no visible reason.
- The first tick compares with an empty snapshot. Every failure at the start is a `BREACH` line.
- A row has one line for each venue. A target on two venues counts as two rows.
- There are no colors and no cursor codes.

### 6.2 `--format json` (JSON Lines)

One JSON object on each line. The field `type` says which. Every line parses alone.

```json
{"type":"start","ts":"2026-11-10T12:00:00Z","scope_version":"0.8.6","name":"pegs","path":"/home/u/.scope-bca/watchlists/pegs.yaml","interval_secs":60,"duration_secs":3600,"once":false,"rows":["USDN@weex","USDN@binance"],"csv":null,"report":"./pegs.md"}
{"type":"tick","run":1,"ts":"2026-11-10T12:00:05Z","counts":{"rows":2,"breached":1,"warned":0,"unknown":0,"passed":1},"rows":[{"target":"USDN@weex","venue":"weex","status":"breached","cells":[{"metric":"Spread","op":null,"threshold":null,"value":3.4,"status":"fail","source":"preset","error":null,"detail":"Spread: 3.4%, max 3%"}]}]}
{"type":"event","run":1,"ts":"2026-11-10T12:00:05Z","event":"breach_started","target":"USDN@weex","cell":"Spread","message":"Spread: 3.4%, max 3%"}
{"type":"end","ts":"2026-11-10T13:00:05Z","runs":61,"reason":"duration","exit_code":1,"report_written":true}
```

- `tick` holds every cell of every row. A cell has the same fields and the same values as a CSV row (`metric`, `value`, `op`, `threshold`, `status`, `source`, `error`, `detail`). An empty CSV field is `null`. A number is a number.
- The CSV rows and the JSON cells come from one function that builds a list of cell records. This keeps the two outputs equal. It replaces the field code that `csv_rows` holds now. The CSV bytes do not change.
- `event` follows its `tick`. The `event` values are `breach_started`, `breach_cleared`, `warn_started` and `warn_cleared`.
- `error` holds the exact error text, not a summary.
- `reason` in `end` is `once`, `duration`, `signal`, `output_error` or `stdout_closed`.
- `start` and `end` are added for the record of the run. Open question 9.

### 6.3 `--format csv`

Stdout gets `csv_header()` once and `csv_rows(...)` after each tick. The bytes are the bytes of the `--csv` file. Events and counts are not in the output. The exit code carries the verdict. Open question 5 covers this format.

### 6.4 Files

- `--csv PATH` works with every `--format`. It uses the same function as the grid run.
- `--report PATH` uses `report_text`, the report of the grid run.

## 7. Stop conditions

| Condition | What happens | `reason` |
|---|---|---|
| `--once` | One tick. The schedule is `Schedule::new(every, Duration::ZERO)`: it has one run. | `once` |
| `--duration D` (or `duration:` in the file) | The last tick starts when `D` ends. A run of `1h` every `30s` makes 121 ticks. This is the rule of the repeat runner. | `duration` |
| No duration | The run goes on until a signal. A file with no `duration:` means this. A `stderr` note says so at the start. Open question 2. | `signal` |
| Ctrl-C or SIGTERM | The run stops. Section 7.1. | `signal` |
| CSV write error | The run stops at once. The report is still tried. Exit 4. | `output_error` |
| stdout closed (a reader quits) | The run stops like a signal. A note goes to stderr. The code is the usual code. | `stdout_closed` |

- A tick that takes longer than the interval does not overlap the next one. The runner moves the schedule grid. This is today's rule for the grid.
- A tick in progress when the duration ends is not cut.
- `--count N` is not in this design. Open question 3.

### 7.1 Signals

1. The handler races the run against a stop future. On Unix the future ends on SIGINT or SIGTERM. On Windows it ends on Ctrl-C and Ctrl-Break (`tokio::signal::ctrl_c`, `ctrl_break`). SIGTERM does not exist there.
2. The handler keeps the last finished tick in a variable. The tick callback fills it. The callback runs after the CSV append, so the CSV and the variable agree.
3. On a signal the handler drops the run, writes the report from the last tick, prints the `end` record and returns the code of the last tick.
4. A signal during a tick drops that tick. The CSV has no row of it. The CSV append of a tick is one write, so the file never holds half a tick. Open question 6.
5. A signal before the first tick ends: no tick exists. Open question 4.
6. The stop future is a parameter of the handler. A test passes its own future. No test needs a real signal, except one Unix process test (section 12).

### 7.2 What the code needs

`watch::run` does two jobs: the loop, and the report at the end. Its error return loses the last tick when the report fails. It writes no report when dropped.

- Split it into the loop (`run_ticks`) and the report call. `run` keeps calling both. The grid is unchanged.
- The headless handler calls `run_ticks`, keeps the last tick itself, and calls `write_report` once, for the normal end and for a signal.
- Any error from `run_ticks` is a CSV error (the header or an append). `Schedule::new` cannot fail: the interval parser rejects zero.

## 8. Exit codes

| Code | Meaning |
|---|---|
| 0 | No row breached and no row is unknown on the last tick. Warnings count as a pass. |
| 1 | At least one row breached on the last tick. |
| 2 | No row breached, but at least one row is unknown (a cell is `Error`). Health is unknown. |
| 3 | The run could not start: the watchlist or a flag is not valid, or `prepare` failed. No tick ran. |
| 4 | The CSV file or the report could not be written. |

### 8.1 How the codes combine

Take the first rule that matches:

1. A write error (CSV or report) gives 4. The record of the run is lost, so this code wins over 0, 1 and 2.
2. The run did not start gives 3. This happens before any tick, so it cannot come with a tick result.
3. A row breached on the last tick gives 1.
4. A row is unknown on the last tick gives 2.
5. Otherwise 0.

A breach wins over an unknown row. A breach is a sure finding. An unknown row is a missing reading. A script that needs both reads the JSON output or the CSV file.

The code comes from one pure function over the last snapshot. The numbers are named constants.

### 8.2 The three causes the issue names

| Cause | Where it shows | Code |
|---|---|---|
| A rule hit (a rule or a preset check fails) | A `Fail` cell. The row is `Breached`. | 1 |
| An unhealthy market | The `market health` preset makes a `Fail` cell for a failed check. So it is a rule hit. A `Warn` check does not change the code. | 1 |
| A venue error (HTTP error, timeout, unknown pair) | An `Error` cell on the rows of that venue. The row is `Unknown`. | 2 |

An unknown venue name in the file is not a venue error. It fails the validation. The code is 3.

### 8.3 Two conflicts to know

1. `scope market summary` returns 0 for an unhealthy market. The verdict is in its output. `scope watch` returns 1, because the user wrote the checks to find this. The two commands differ on purpose.
2. `scope market summary --venue a,b` returns 1 when any venue failed. `scope watch` returns 2 for the same event. In `watch`, 1 means "breach". The issue fixes these codes.

### 8.4 The code is the last tick's

The code reflects the last tick only. A breach on tick 3 that clears on tick 5 gives 0 if tick 5 is the last. The JSON output and the CSV file keep the history. Open question 1.

## 9. Several venues

A target with `venue: [weex, binance]` makes one row for each venue. The row label is `USDN@weex`.

1. Each venue is read on its own. A failed venue makes `Error` cells in its row only. The other venues are read and judged as usual.
2. The tick reads the venues one after the other (today's behaviour of `read_target`). The grid does the same. Section 10 lists concurrent reads.
3. The exit code is the worst over all rows (section 8.1). One failed venue gives 2, unless a row breached: then 1. Open question 12 asks whether one failed venue should give 2.
4. Text lines and JSON rows name the row and its venue. The `venue` field is `null` for a row without a venue (a token or an address).
5. The CSV file has the row label in `target`. There is no `venue` column. This is the current format of the trail. The label holds the venue.
6. The since-last-tick metrics (`largest_trade`, `volume_change_pct`) keep one memory for each row. A row that fails one tick keeps its memory.

## 10. Out of scope

| Item | Why it waits |
|---|---|
| A target given on the command line (`scope watch USDC --venue weex`) | The issue asks for a saved watchlist. A target flag would copy the flags of `scope monitor`. A user can save a list with `scope monitor ... --save NAME`. Add it if users ask. |
| Memory from one `--once` run to the next | The since-last-tick metrics need a state file. A state file needs a place, a format and a rule for stale data. A `--once` run reports `volume_change_pct` as `n/a`, which does not count against the verdict. The docs say so. |
| Notifications and webhooks | Decision D4 of the watchlist design left them out. The JSON Lines and the exit code let a user connect a notifier. |
| Concurrent venue reads in a tick | The engine is shared with the grid. A change needs its own timing tests, and the exit codes do not depend on it. |
| `--count N` | See open question 3. `--once` covers the cron case. |
| A flag to count warnings as failures | See open question 13. No user asked for it yet. |
| An exit code that is a bit mask of all states | A mask is hard to read in a shell. A script that needs more reads the JSON. |
| `--quiet` for the `--- Run #N ---` notes | The notes come from the shared repeat runner. A flag there changes every repeating command. A user can redirect stderr. |
| A lock against two runs at the same time | Two cron runs with one `--csv` file write to one file. The scheduler can prevent this (`flock`). A lock file needs a policy for stale locks. |
| Reload of the file during a run | Not requested. The run reads the file once. |

## 11. Open questions

Each question has options and a recommendation. The recommendation is not a decision.

### 1. Which tick sets the exit code?

- **A. The last tick.** This is what the issue and the approved design say. A cleared breach exits 0.
- **B. The worst tick of the run.** A breach at any tick gives 1. A `--duration 1h` run then reports a breach that cleared at once.
- **C. The last tick by default, and a flag `--fail-on-any` for B.**

**Recommendation: A.** It matches the issue and the one-line rule in the docs. The JSON output and the CSV file hold the history. C is a small later addition if users ask.

### 2. What does a run do when no duration is given anywhere?

A file with no `duration:` means "until stopped". Without `--once`, a cron job on such a file never ends.

- **A. Run until stopped.** The same rule as the grid and the file format. Print a stderr note at the start: `No duration: runs until stopped. Use --once or --duration.`
- **B. Run one tick.** Safe for cron. It differs from the grid and from the file format.
- **C. Refuse with exit 3** and tell the user to give `--once` or `--duration`.

**Recommendation: A.** One meaning of "no duration" in all places. The note warns the user.

### 3. Does the command have `--count N`?

`--count N` equals `--duration (N-1)*every`, so it needs no runner change.

- **A. Add `--count N`.** `--once` is `--count 1`.
- **B. Only `--once`.**

**Recommendation: B.** The issue asks for `--once` only. Add `--count` when someone needs three ticks and not a duration.

### 4. What is the exit code when a signal comes before the first tick ends?

No tick exists, so "the last tick's code" has no value.

- **A. Exit 2.** Health is unknown.
- **B. Exit 128 plus the signal number** (130 for Ctrl-C, 143 for SIGTERM). This is the shell convention.
- **C. Exit 0.**

**Recommendation: A.** The verdict was not read. A script must not treat it as a pass. It keeps the five codes of the issue.

### 5. Does `--format csv` exist?

The issue lists `text` and `json`. The CSV file option `--csv PATH` exists already.

- **A. Add `--format csv`.** Stdout gets the same bytes as the file. The cost is small: the header and row functions exist.
- **B. Only `text` and `json`, as in the issue.** A user who wants CSV on stdout uses `--csv /dev/stdout`.

**Recommendation: A.** It costs a few lines and fits pipes. It is the one addition beyond the issue text, so say no if you want the issue as written.

### 6. What happens to a tick in progress when a signal comes?

- **A. Cut it at once.** The tick has no row in the CSV file. The report uses the tick before.
- **B. Finish the tick, then stop.** A second signal cuts it. The last tick is complete. A slow venue can delay the stop. A container gets a short time before SIGKILL.

**Recommendation: A.** A stop should be quick. The CSV file stays consistent. The cost is one lost tick.

### 7. What is the exit code when `prepare` fails?

`prepare` resolves symbols and the address book before the first tick. It can fail on a name that is not valid, or on a network error.

- **A. Exit 3 for every `prepare` error.** One rule: the run could not start.
- **B. Exit 3 for errors the user fixes in the file, exit 2 for network errors.** This needs the error type to tell them apart.

**Recommendation: A.** One rule is easy to document. The stderr text has the exact cause. The cost: a network error at the start looks like a bad file to a script.

### 8. Which flags does `watch <name>` accept in the TUI?

- **A. None.** `watch <name|path>` is an alias of `watch open`. A flag gives a one-line error that says to edit the list in the wizard.
- **B. `--every`, `--duration`, `--csv` and `--report`** override the list before the grid opens. `--once` and `--format` give a one-line refusal.

**Recommendation: A.** It is the smaller change. It makes the route and the parity test pass. The grid already has the wizard for edits.

### 9. Does the JSON output have `start` and `end` records?

The issue says one object for each event and each tick.

- **A. Add `start` and `end`.** `start` holds the effective settings after the flags. `end` holds the number of runs, the stop reason and the exit code. These records let a person find the cause of a failed job later.
- **B. Only `tick` and `event`.**

**Recommendation: A.** A job that ran at night leaves its full record in the log. A consumer that reads only `tick` and `event` is not affected.

### 10. What happens to an existing `--csv` file when the run starts?

Today the grid run truncates the file and writes a header. A cron job with `--once` then keeps only the last run.

- **A. Truncate.** The same as the grid. A user writes dated file names in the shell.
- **B. Always append in headless mode.** The header is written only when the file is new or empty.
- **C. Truncate by default. `--append` keeps the rows.**

**Recommendation: C.** It keeps the grid behaviour. It lets a cron job build one trail. The explicit flag prevents a surprise in both directions.

### 11. The exit code of a clap usage error

clap exits with 2 on a bad flag. Exit 2 also means "health unknown".

- **A. Accept the overlap.** A usage error shows on the first run. The stderr text is clear. The docs list it.
- **B. Map usage errors of `watch` to 3.** This needs `try_parse_from` in `main`, with a special case for this one command.
- **C. Use another code for "health unknown".** This changes the codes of the issue and the approved design.

**Recommendation: A.** B puts command-specific code in `main` for a case that shows once. The risk: a flag that breaks after an upgrade looks like "unknown" to a script. State it in `--help`.

### 12. Does one failed venue on a multi-venue target give exit 2?

- **A. Yes.** Any `Unknown` row gives 2 (if no row breached). A venue in the list that cannot be read is a gap in the watch.
- **B. Only when every venue of the target fails.** One failed venue is still shown in the output, but the code does not change.

**Recommendation: A.** The code never hides a gap in the watch. A script that wants to ignore one venue can remove it from the list.

### 13. Do warnings change the exit code?

A `Warn` preset check does not fail the row today.

- **A. No.** A warning gives 0. The text and JSON outputs show it.
- **B. Add a flag `--strict` so that a warning gives 1.**

**Recommendation: A.** The issue has five codes and no warning code. Add `--strict` when a user needs it.

## 12. Tests

All tests use mock venues and mock chain clients (`MockClientFactory`, `mockito` with the real venue mapping). No test uses the network. Each test sets the scope home to a fresh folder.

Tests encode the reason for each rule: a script depends on the code, so a wrong code is a wrong action in a pipeline.

**Exit codes** (one test for each, end to end through the handler, with a mock factory):

1. **0:** all rules pass; a `Warn` check also gives 0.
2. **1:** a rule fails; a failed preset check fails (an unhealthy market).
3. **2:** a venue answers 503 and no rule fails.
4. **3:** a YAML error; an unknown metric; a metric not valid for the kind; an unknown venue; a list with no targets; a name that does not exist; a bad `--every`. Each one proves that no request was made (the mock server counts zero calls).
5. **4:** the CSV path is in a folder that does not exist (no request is made); the report path cannot be written (the run was complete, the code is still 4).
6. **Combination:** a breach and an unknown row give 1. A breach and a write error give 4. An unknown row and a write error give 4. The pure function has a table test.
7. **Last tick:** a breach on tick 1 that clears on tick 2 gives 0. A pass on tick 1 and a breach on tick 2 give 1.
8. **No panic path:** the handler returns `Ok(code)` for every case above, never `Err`.

**Several venues:**

9. A target on `weex` and `binance`, where `binance` fails: two rows, one `Unknown`; the other venue still runs; the code is 2. With a breach on `weex`, the code is 1.
10. Both venues appear in the text lines, the JSON rows and the CSV `target` column.

**Output:**

11. Every line of `--format json` parses as JSON and has a known `type`. The `tick` cells equal the CSV rows of the same tick, field by field (this keeps the two outputs equal).
12. The `--format csv` stdout is byte-equal to the `--csv` file.
13. Text mode prints an `ERROR` line for an `Error` cell on every tick, and a `BREACH` line on the first tick for a failure that exists at the start.
14. stdout holds only data. The version line and the runner notes are on stderr (capture both streams).
15. `--csv` with `--append` keeps the old rows and does not write a second header. Without `--append`, the file is truncated.

**Stop conditions:**

16. `--once` makes exactly one tick and the `end` record says `once`. `--once` with `--duration` is a usage error.
17. `--duration 2s --every 1s` (paused clock, `tokio::test(start_paused)`) makes three ticks.
18. The stop future resolves after tick 2: the report file exists, it holds tick 2, the exit code is the code of tick 2, and the `end` record says `signal`.
19. The stop future resolves before tick 1 ends: the exit code follows question 4, and the report is written from an empty tick (no rows).
20. A closed stdout (a writer that fails with `BrokenPipe`) stops the run, writes the report and gives the usual code.
21. On Unix only: a process test starts the binary against a local mock venue and sends SIGTERM. It checks the exit code and the report file. (This test needs a user venue descriptor that points to the mock server; confirm this works when building the test.)

**Prompts and routing:**

22. A symbol on many chains in the file does not prompt. The run ends with exit 3 and a message that names the contract field.
23. The parity test: `watch` has a TUI route. `watch <name>` in the TUI opens the grid, as `watch open <name>` does.
24. The shared path rule (name or path) gives the same result for the TUI and the CLI.
25. The shared timing function gives the same result for `scope monitor` and `scope watch` (`30s`, `1h`, `forever`, a bad value).
26. `scope watch --help` lists the five exit codes and the `--once` example.
27. The existing tests for `watch::run`, the grid and `csv_rows` pass unchanged. They cover the split of `run` and the shared cell records.

## 13. What changes in the code

| Area | Change |
|---|---|
| `cli/mod.rs` | New `Commands::Watch(WatchArgs)`. |
| `cli/watch/headless.rs` (new) | Flags, loader, run loop, output, exit-code function, stop future. |
| `cli/watch/run.rs` | Split `run` into the loop and the report. Build CSV and JSON cells from one function. |
| `cli/watch/monitor.rs` | Move the `--every` and `--duration` code to one shared function. |
| `cli/dispatch.rs` | A branch for `Watch` that returns the code, next to the TUI branch. It runs the catalog step and turns the Ghola offer off. |
| `cli/tui/exec.rs` | A `route` arm. The `watch <name|path>` session command. The shared name-or-path function. |
| `tests/one_output_path.rs` | No change expected: all output goes through `Output`. |

No code in `scope-core` changes, except that a helper for the "no targets" check may live there.

## 14. Risks

1. **The `Err` to exit 1 path.** Any `Err` that escapes the handler reads as a breach. The test in section 12 (item 8) guards this.
2. **A run with no duration never ends.** The stderr note and the docs warn the user. Question 2.
3. **A relative output path in cron.** The `start` record and the stderr note print the full path that the run uses.
4. **Cut ticks.** A signal drops the tick in progress. The `end` record says `signal`, so a reader knows the last row is not the last intended tick.
5. **Windows.** There is no SIGTERM. The Windows stop future uses Ctrl-C and Ctrl-Break only.
