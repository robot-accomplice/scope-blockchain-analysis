# TUI Test Harness: Design

- **Status:** draft for review
- **Date:** 2026-10-12
- **Issue:** #47 (TUI parity), requirement 5 (a CLI command cannot ship without a TUI path).
- **Depends on:** nothing. Step 1 (one HTTP transport) is a prerequisite inside this design.

## 1. Goal

Validate the TUI on the local machine before each release. The harness runs the real `scope` binary in a real terminal, sends keys, and checks the screen, the event log and the output files.

### First use

The first use is the prospective TUI release (the `[Unreleased]` changes on `develop`, planned as 0.7.0):

1. Run the harness against the release candidate.
2. Fix the defects it finds, or record them as issues.
3. Then do the final usability audit. The harness report is an input to that audit: it shows which commands and features work, with evidence for each failure.

The 0.7.0 release waits for these three steps. The harness checks function. It does not judge usability; the audit does.

### Decisions (agreed 2026-10-12)

| # | Decision |
|---|---|
| D1 | Two tiers. **Offline:** every command and TUI feature against recorded API responses. Deterministic. This tier is the release gate. **Live:** the same scenarios against the real APIs. Reported, not gating, because an API outage is not a scope defect. |
| D2 | Offline data comes from a **record/replay transport** inside scope. All HTTP goes through `HttpClient` first. |
| D3 | A **Rust runner** drives the binary in a PTY. Scenarios are **YAML** files, so a new scenario needs no Rust. |
| D4 | The gate is **local** for now: `just tui-test`, required by `docs/RELEASE.md`. A gate on every PR comes later, with a faster solution (section 8). |

### Out of scope

- **Windows.** The TUI does not support Windows today.
- **Pixel or color checks.** The harness checks text and cell layout from the `vt100` screen model. Colors are parsed but not asserted.
- **Load or performance tests.** The harness checks behavior, not speed. A scenario timeout only stops hangs.

## 2. One HTTP transport (prerequisite)

Two paths bypass the configured transport (`NativeHttpClient` or `GholaHttpClient`):

- `scope-core/src/market/configurable_client.rs` holds a raw `reqwest::Client` (line 48). Every exchange venue call skips the transport, including Ghola.
- `scope-core/src/chains/ethereum.rs` creates its own `NativeHttpClient` (lines 241, 271, 288).

### Change

1. `ConfigurableExchangeClient` takes an `Arc<dyn HttpClient>`. Status-code handling (`format_http_api_error`) uses `Response::status_code`.
2. `ethereum.rs` uses the client from the factory.
3. The client factory builds the transport one time from config and gives it to every client.
4. A test checks that no file outside `scope-core/src/http/` uses `reqwest::Client` or `NativeHttpClient::new` outside `#[cfg(test)]` code. A new bypass then fails the build.

This change also fixes a real defect: venue calls ignore the Ghola transport today.

## 3. Replay transport

### 3.1 Placement and selection

`scope-core/src/http/replay.rs`: `ReplayHttpClient`, a third `HttpClient` implementation.

The factory selects it from environment variables, not from `config.yaml`, so a normal user cannot turn it on by accident:

| Variable | Values | Effect |
|---|---|---|
| `SCOPE_HTTP_MODE` | `replay` | Answer each request from the fixture directory. Never use the network. |
| | `record` | Send each request through the configured transport, then write the response as a fixture. |
| | unset | Normal operation. |
| `SCOPE_HTTP_FIXTURES` | a directory | Where fixtures are read or written. Required when `SCOPE_HTTP_MODE` is set. |
| `SCOPE_FAKE_NOW` | RFC 3339 time | Only in `replay` mode: the time that scope shows and uses. Snapshots then do not drift. Ignored in other modes. |

### 3.2 Fixture key and file

- **Key:** the method, the URL with secret query parameters removed, and the SHA-256 of the request body. Query parameters are sorted, so their order does not matter.
- **File:** `<fixtures>/<host>/<sha256-of-key>.json`:

```json
{
  "recorded": "2026-10-12T09:30:00Z",
  "request": { "method": "GET", "url": "https://api.binance.com/api/v3/depth?limit=100&symbol=USDCUSDT", "body_sha256": "e3b0…" },
  "response": { "status": 200, "headers": { "content-type": "application/json" }, "body": "{…}" }
}
```

- **Repeated requests:** when a scenario sends the same request more than once (repeat mode), record mode keeps each response in order (`responses: [...]`). Replay serves them in order and repeats the last one when the list runs out.

### 3.3 Errors

- **Replay miss:** a request with no fixture returns an error that names the method and the URL: `no fixture for GET https://…; run just tui-record <scenario>`. It never falls back to the network.
- **Record failure:** a network error is passed through unchanged and no fixture is written.

### 3.4 Secrets

Before writing a fixture, record mode:

1. Removes secret query parameters (`apikey`, `api_key`, `key`, `token`, `access_token`) and headers (`authorization`, `x-api-key`, `cookie`, `set-cookie`).
2. Scans the whole file for any secret value from the active config (`chains.api_keys`, `web.auth_token`, RPC URL credentials). If it finds one, it does not write the file and returns an error.

## 4. Harness crate

`crates/scope-tui-harness`, with `publish = false`. Binary `scope-tui-harness`. It depends on `scope-bca-cli` only to list the commands (section 6.1).

### 4.1 Units

| Unit | Purpose |
|---|---|
| `scenario` | Parse and validate a YAML scenario. |
| `pty` | Start `scope tui --no-banner` in a PTY (`portable-pty`) at a given size, send bytes, resize, wait for exit. |
| `screen` | Feed PTY output into a `vt100::Parser` and give the screen as text. |
| `steps` | Run the steps and their checks. |
| `evidence` | Save the evidence of a failed scenario. |
| `coverage` | Compare the commands that ran with the command list. |
| `report` | Print one line per scenario and a summary, and set the exit code. |

### 4.2 Isolation for each scenario

- A new temporary `HOME`. `XDG_CONFIG_HOME`, `XDG_DATA_HOME` and `XDG_CACHE_HOME` are removed from the environment, so the default paths resolve under that `HOME`.
- The scenario `config` is written to `$HOME/.config/scope/config.yaml`. With no `config`, the file is `{}`.
- `TZ=UTC`, `LANG=C.UTF-8`, `TERM=xterm-256color`.
- Offline tier: `SCOPE_HTTP_MODE=replay`, `SCOPE_HTTP_FIXTURES=tests/tui/fixtures/<scenario>`, and `SCOPE_FAKE_NOW` when the scenario sets `now`.
- Scenarios run in parallel (`--jobs N`, default: the CPU count). They share only the built binary.

## 5. Scenarios

### 5.1 File

`tests/tui/scenarios/<name>.yaml`:

```yaml
name: market summary repeat mode
tiers: [offline, live]            # default: both
terminal: { cols: 120, rows: 36 } # default: 120 x 36
now: "2026-10-12T09:30:00Z"       # optional; offline tier only
timeout: 120s                     # whole scenario; default 120s
config: |
  market:
    health: { min_levels: 10 }
steps:
  - type: "market summary USDC --venue binance --every 1s --duration 3s"
  - key: Enter
  - wait_for: { text: "Run #3", timeout: 15s }
  - expect_screen: { contains: ["Health Checks", "HEALTHY|UNHEALTHY"] }
  - expect_journal: { argv_prefix: ["market", "summary"], outcome: ok }
  - key: Ctrl-K
  - type: "venues"
  - expect_screen: { contains: ["venues list"] }
  - key: Esc
  - resize: { cols: 80, rows: 24 }
  - expect_screen: { not_contains: ["panicked"] }
  - snapshot: market-80x24
  - key: Ctrl-D
  - expect_exit: 0
```

### 5.2 Steps

| Step | Effect |
|---|---|
| `type: <text>` | Send the text as typed characters. |
| `key: <name>` | Send one key or chord: `Enter`, `Tab`, `Esc`, `Up`, `Down`, `Left`, `Right`, `Backspace`, `PageUp`, `PageDown`, `Ctrl-<letter>`, `Alt-<letter>`, `F1`–`F12`. |
| `wait_for: {text, timeout}` | Read the screen again and again until the regex `text` matches. Fail at `timeout` (default 10s). There are no fixed sleeps. |
| `expect_screen: {contains, not_contains}` | Each item is a regex. All `contains` items must match the screen, and no `not_contains` item may match. |
| `expect_journal: {argv_prefix, outcome, error}` | At least one `tui-events.jsonl` record written since the previous `expect_journal` must match. `outcome` is `ok`, `error`, `cancelled` or `session`. `error` is a regex. |
| `expect_file: {path, contains}` | A file under `$HOME` exists, and its text matches each regex. |
| `resize: {cols, rows}` | Resize the PTY. |
| `snapshot: <name>` | Compare the masked screen with `tests/tui/snapshots/<scenario>/<name>.txt`. If the file is missing, write it and fail, so a new snapshot is reviewed before it counts. |
| `expect_exit: <code>` | Wait for the process to exit (within the step timeout) and check the exit code. |

### 5.3 Validation

A scenario is rejected before it runs when:

- it has no `expect_*` or `snapshot` step (a scenario that checks nothing cannot pass),
- a key name or step field is unknown,
- `now` is set and the scenario includes the live tier only.

### 5.4 Snapshot masks

Before comparing, the harness replaces text that changes from run to run: RFC 3339 and `HH:MM:SS` times, durations (`123ms`, `1.2s`), and run times. A scenario can add masks (`masks: ["Block \\d+"]`). Trailing spaces are removed from each line.

## 6. Coverage

### 6.1 Commands

`coverage` walks `scope_cli::cli::Cli::command()` and lists each leaf subcommand path (for example `market summary`, `compliance trace`).

A command counts as covered only when a **passing offline** scenario ran it: the `argv` of a `tui-events.jsonl` record starts with the path and the record's `outcome` is `ok` or `error` (an error scenario also tests the command). A scenario cannot claim coverage that it does not exercise.

The run fails when a command is not covered. An allow-list in `tests/tui/coverage-exempt.yaml` excludes commands that do not belong in the TUI. Each entry needs a reason:

```yaml
- command: completions
  reason: Writes a shell script to stdout. It has no TUI use.
```

### 6.2 TUI features

Each feature has at least one scenario:

- Tab completion, inline hints, the `Ctrl-K` palette.
- `Esc` and `Ctrl-C` stop a running `--every` loop; the journal records `cancelled`.
- Session commands: `chain`, `format`, `limit`, `+tokens`, `+txs`, `trace`, `decode`, `tokens`, `ctx`, `clear`.
- History: a second session in the same `HOME` recalls the first session's lines with `Up`.
- Resize from 120×36 to 80×24 and to 200×60, with no panic text and a correct snapshot.
- The monitor: start, show data, quit back to the TUI.
- The repeating view (#58), when it ships: a `--every` run updates one screen in place; check the snapshot panel and each key-value chart after N runs, at 120×36 and at 80×24.
- Errors: an unknown venue, a replay miss (offline only), a network error chain (offline, from a recorded error).

## 7. Fixtures and the live tier

### 7.1 Recording

`just tui-record <scenario>` runs one scenario in `record` mode against the live APIs and writes `tests/tui/fixtures/<scenario>/`. The change is reviewed as a normal git diff. Large array responses are trimmed only when the scenario sets `record_trim: {path: <json-path>, keep: N}`.

### 7.2 Live tier

`just tui-smoke` runs the scenarios that include `live`, with no `SCOPE_HTTP_MODE`.

- Assertions in live scenarios must check structure, not values (for example `HEALTHY|UNHEALTHY`, not `HEALTHY`). Exact values belong in the offline tier.
- A failed live scenario runs again one time. The report shows the retry.
- **API drift:** the live run records responses to a temporary directory and compares the JSON key paths of each response with the stored fixture for the same key. A difference is reported as `API drift: <host> <path>: added …, removed …`. Drift does not fail the run. It tells you that a fixture, and maybe a parser, needs an update.

## 8. Evidence, reports and release use

### 8.1 Evidence

When a scenario fails, the harness saves to `target/tui-harness/<run-id>/<scenario>/`:

- `screen.txt`: the screen at the failure.
- `pty.raw`: all PTY output, with escape codes.
- `tui-events.jsonl` and the scope stderr log.
- `steps.jsonl`: each step with its start time, duration and result.
- `failure.txt`: the failed step, what it expected, and what it found.

`--keep` also keeps the temporary `HOME`.

### 8.2 Report and exit code

One line per scenario (`PASS`, `FAIL`, `SKIP`, with the duration), then a summary, the coverage result and any API drift. The exit code is `1` when an offline scenario fails or coverage fails. Live results never change the exit code.

### 8.3 Release

`docs/RELEASE.md`, "Pre-Release", gets two steps:

1. Run `just tui-test` on the release commit. It must pass. Paste the summary into the release PR.
2. Run `just tui-smoke`. Attach the report, including API drift. It does not block the release.

### 8.4 Later: a gate on every PR

Deferred. **Why:** PTY scenarios add minutes to every CI build, and the cost needs a better solution first, for example an in-process mode that drives the TUI app without a PTY, or a shared runner. A follow-up issue records this. A CI job is an infrastructure change: review first whether it is needed, then what it can break, before it is applied.

## 9. Build order

Each step is one PR into `develop`. Each PR leaves the CLI and the TUI working.

| Step | Work | Depends on |
|---|---|---|
| 1 | One HTTP transport (section 2), with the bypass test | none |
| 2 | Replay transport, secret scan, `SCOPE_FAKE_NOW`, unit tests (section 3) | 1 |
| 3 | Harness crate: PTY, screen, steps, evidence, report (sections 4, 5, 8.1, 8.2) | none (can run beside 1 and 2) |
| 4 | Coverage check and the first scenarios (section 6) | 2, 3 |
| 5 | Fixtures, `just` recipes, live tier with drift, `RELEASE.md` steps (sections 7, 8.3) | 4 |
| 6 | Run against the 0.7.0 candidate; triage results for the usability audit (section 1, "First use") | 5 |

## 10. Risks

- **Fixture size.** Recorded bodies can be large. Trimming (7.1) is explicit per scenario, so no data is lost by accident.
- **PTY behavior differs between macOS and Linux.** `portable-pty` supports both. The harness runs on both before step 5 merges.
- **Replay hides API changes.** Replay proves the TUI with the responses we recorded. The live tier and the drift report cover the gap.
- **Fake time misses time bugs.** `SCOPE_FAKE_NOW` only applies in replay mode. Live scenarios use the real clock.
- **Secret leak through fixtures.** Section 3.4 removes known secret fields and blocks any file that contains a configured secret value.
