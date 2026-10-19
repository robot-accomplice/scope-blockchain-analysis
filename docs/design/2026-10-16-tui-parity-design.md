# TUI Parity with the CLI (#47): Design

- **Status:** approved by Jon on 2026-10-16, with the proposed answers to Q1–Q7 (section 9).
- **Date:** 2026-10-16
- **Issue:** #47 (TUI: full functional parity with the CLI).
- **Builds on:** `2026-10-10-tui-watchlist-design.md` (approved 2026-10-10). That design replaced the "monitor wizard" of #47 with watchlists. This design does not replace it. It records what is done, closes the gaps that design does not cover, and changes it in three places (section 4.2).
- **Standard:** Clean Code and Clean Architecture. Each concern has one source, the adapters are thin, and dependencies point inward.

## 1. Goal

Close #47. Its acceptance:

1. Every row of the coverage table shows full coverage.
2. The parity test runs in CI.
3. `scope market summary USDN --venue weex --every 30s --duration 1h` has a documented TUI path through the monitor wizard, and that path gives the same health results.
4. `scope monitor` and `scope interactive` open the same TUI.

## 2. Gap audit (develop at 2d89206, 2026-10-16)

The coverage table in #47 dates from 2026-10-09. Since then, the TUI (`scope tui`, alias of `scope interactive`) parses every line with the CLI's clap definition and calls `dispatch_with`, so each command uses the same handler and the same flags. `tui::exec::route` decides where each command runs. Its match has no wildcard arm, and the parity test in `tui/exec.rs` fails for a visible subcommand that has no route.

### 2.1 Where each command runs

| CLI command | TUI route | Notes |
|---|---|---|
| `address`, `tx`, `insights`, `contract` | pane | Session context fills `--chain`, `--include-txs`, `--limit`, `--trace`, `--decode`. |
| `crawl` | pane with an address or `--yes`; **outside** with a name or symbol | **Stale.** The symbol search now uses `resolve` with `TuiPicker` (#109), which asks in the pane. The outside route is no longer needed. |
| `token-health` | pane | Uses the same resolver and the in-pane token list. |
| `discover` | pane | |
| `market summary` | pane | All flags, `HealthArgs` included. `--every`/`--duration` show the repeating view (#58). `--report`, `--csv` work. `save x.csv` writes the series. |
| `market ohlc`, `market trades` | pane | One-shot only: no `--every`. |
| `venues list/schema/init/validate` | pane | |
| `compliance risk/trace/analyze/compliance-report` | pane | |
| `address-book add/remove/list/summary` | pane | |
| `export` | pane | |
| `report batch` | pane | |
| `setup --status` | pane | |
| `setup` (wizard, `--key`, `--reset`) | **outside** | Reads stdin. The watchlist design (5.5, plan F) moves key entry into a TUI form. |
| `monitor` | **outside** | Draws its own dashboard (`monitor::run_direct`). This is the main gap. |
| `interactive`, `web`, `completions` | refused (`TUI_REFUSED`) | `web --stop` and `web --status` are refused too, but they do not run a server. |

### 2.2 The trigger case today

`market summary USDN --venue weex --every 30s --duration 1h` runs in the pane with the repeating view. That covers requirement 3 for this command. The `monitor` path is still different from it:

| Difference listed in #47 | Today |
|---|---|
| Synthetic order book | **Fixed** (#78). With `--venue`, the monitor shows the venue's real book and trades. Without a venue, the DEX book is the core AMM book from #37 (`order_book_from_analytics`). |
| No health checks | **Open.** The monitor reads `market.health` only to build the DEX book. It shows no checks and no pass/fail state. `HealthArgs` flags are not on `monitor`. |
| No `--duration` | **Open.** The monitor runs until the user quits. |
| `--export` writes price data only | **Open.** The CSV header is `timestamp,price_usd,volume_24h,liquidity_usd,buys_24h,sells_24h,market_cap`. No `--report`. |

### 2.3 What exists to build on

| Part | Where | State |
|---|---|---|
| Shared book fetch | `scope::market::fetch_book_and_volume` | Done. `market summary` uses it; the monitor does not (it calls `ExchangeClient` directly). |
| Repeat runner | `cli::repeat::run`, `Schedule` (interval, optional duration) | Done (#58). Only `market summary` uses it. |
| Key values | `scope::market::series` (`MetricId`, `Metric`, `Threshold`, `Sample`, `summary_metrics`) | Done for the order book only. |
| Charts | `cli::charts::{Sparkline, HealthStrip}` | Done. The monitor and the repeating view share them. |
| In-pane questions | `Output::asker`, `TuiPicker` | Done (#109). Pick from a list and yes/no. |
| Completion, hints, palette | `tui/{vocab,complete,hint,palette}.rs` | Done (watchlist plan L). |
| Watchlist engine, `scope watch`, grid, wizard, first run | — | Not started (watchlist plans E, H, U, F). |
| Saved metric set (#47 requirement 6) | — | Not started. |

### 2.4 Gaps

| # | Gap | Closes |
|---|---|---|
| G1 | `scope monitor` opens a separate dashboard, not the TUI. | Acceptance 4 |
| G2 | No monitor wizard; no live view with thresholds and pass/fail for many targets. | Acceptance 3 |
| G3 | A metric set cannot be saved or loaded. | Requirement 6 |
| G4 | The monitor has no health checks, no `--duration`, no `--report`, and no `HealthArgs`. | Trigger case |
| G5 | `crawl <symbol>` leaves the TUI with no need. | Table row `crawl` |
| G6 | `setup` (not `--status`) leaves the TUI. | Table row `setup` |
| G7 | The metric catalog covers only the order book. Token and address metrics have no `MetricId`. | Requirement 3 for other metrics |
| G8 | The watchlist file format is YAML, and #118 moves off `serde_yaml`. | Risk, not a row |

## 3. Scope

### In

- G1–G5 and G7, through watchlist plans E (engine) and U (grid, drill-down, wizard), with the changes in section 4.2.
- G5 as its own small change: `crawl` routes to the pane.
- `scope monitor <args>` opens the TUI with the wizard filled from the arguments (section 5.3).
- The equivalence test for the trigger case (section 7).
- The README and the issue table updated to the new coverage.

### Out

- **Headless `scope watch` (plan H).** It is in the approved watchlist design, but #47 does not need it. It ships after #47 closes. See Q4 (section 9).
- **Guided first run and the setup form (plan F).** G6 stays: `setup` runs outside the TUI and returns to it, and the config reloads after it (#114). #47 counts a suspend-mode command as "runs in the TUI": the user starts it from the TUI and comes back to it. See Q3 (section 9).
- **`web` and `completions` in the TUI.** They stay refused, for the reasons in `TUI_REFUSED`.
- **`--every` on more CLI commands** (`crawl`, `token-health`, `market trades`). A user who wants token metrics over time uses a watchlist. See Q5 (section 9).
- **Desktop notifications, webhooks, rule durations.** Out in the watchlist design, and still out.

## 4. Decisions

### 4.1 Decisions this design keeps from the watchlist design

| # | Decision |
|---|---|
| W-D2 | A watchlist is the saved output of the monitor wizard. The wizard edits a watchlist. The grid is its live view. |
| W-D3 | A rule is a metric threshold: `metric op value`. |
| — | Watchlist file at `<scope config folder>/watchlists/<name>.yaml` (the folder from `scope::paths`, `~/.scope-bca` since #106). It is the saved metric set of requirement 6. |
| — | Drill-down keys run real CLI commands in the shell pane. |

### 4.2 Changes to the watchlist design

| # | Change | Why |
|---|---|---|
| P1 | **The "market health" preset is one check set, not a list of rules.** A target with the preset gets one cell group from `MarketSummary::checks`: each `HealthCheck` is a cell, and the group's status is the summary's own status. The preset does not expand into `metric op value` rules. | The summary's checks include rules that are not one metric against one value: "no sells below peg", the bid/ask ratio band, depth against 24h volume, `Warn` levels, and `n/a` for a synthetic book. Expanded rules would give other results than `market summary`, and #47 acceptance asks for the same results. With P1 the grid calls `MarketSummary::from_order_book` with the same thresholds, so the results are the same by construction. |
| P2 | **`RuleStatus` gets `Warn`.** A target with a `Warn` and no `Fail` is **warned**, shown between passed and breached. | `HealthCheck` has `Warn`. Without it, P1 must map a warn to pass or fail, and both differ from `market summary`. |
| P3 | **One metric catalog: `scope::market::series` grows, `scope::watch` reuses it.** Token and address metrics become new `MetricId`s with the same `Metric { id, label, unit, threshold }` shape. The engine does not define a second `MetricId`. | The repeating view, the CSV, the grid and the detail charts then read one table. A metric added once appears in all of them. |

### 4.3 New decisions

| # | Decision |
|---|---|
| N1 | `scope monitor <args>` starts the TUI (the same entry as `scope tui`) and opens the wizard, filled from the arguments. The user confirms with Enter, and the grid starts. With `--yes` (new), the grid starts at once. |
| N2 | `monitor` inside the TUI does the same: it opens the wizard in the TUI. It no longer routes outside. `route` gives `Commands::Monitor` a new route, `Route::Wizard`. |
| N3 | The old dashboard (`monitor::run_direct` and its five layouts) is retired: the grid replaces it when plan U ships, and the dashboard stays behind `--dashboard` for one release and the code goes in the release after (Q1). Its price and volume charts move into the grid's detail pane, through `cli::charts`. The order book panel and the trades feed become drill-down keys (`b`, `t`), which already run `market summary` and `market trades`. See Q1 (section 9). |
| N4 | `monitor` flags map onto the watchlist. New flags: `--duration`, `--report`, `--csv`, the `HealthArgs` overrides, `--preset`, and `--save <name>`. `--export` stays as a hidden alias of `--csv`, with a note. Section 5.3 has the table. |
| N5 | The monitor's alert config (`monitor.alerts`: price min/max, whale size, volume spike) maps to rules where it can: `price_usd < min`, `price_usd > max`. Whale and volume-spike alerts have no metric. They are dropped, with a note on first load. See Q2 (section 9). |
| N6 | `crawl` with a symbol routes to the pane (G5). The `TuiPicker` asks there. |
| N7 | The watchlist file waits for the YAML choice of #118, or uses the crate #118 picks. The loader is behind one function, `scope::watch::file::{load, save}`, so a crate change touches one file. |

## 5. The monitor wizard

### 5.1 Flow

The wizard is one screen with four steps. `Tab` and `Shift-Tab` move between steps. `Esc` asks "Leave without saving? [y/N]" when anything changed.

1. **Targets.** A list of targets. `a` adds one: the user types a token, a pair, an address or an `@label`. The same resolver as the CLI runs (`resolve` with `TuiPicker`), so a symbol on many chains asks in a list. For a pair or a token with market metrics, the user picks a venue from the `VenueRegistry` list (the same list as `--venue <Tab>`). `d` deletes the selected target.
2. **Metrics.** For the selected target: the "market health" preset (on by default for a pair or a token with a venue), and the catalog metrics that apply to its kind (section 6). Each metric has an operator and a value. The initial value is the threshold from `HealthThresholds` or config, the same order as `market summary`: wizard value, then config `market.health`, then the built-in default.
3. **Timing.** Interval and duration. The defaults are 60s and 1h, the same as `market summary` (Q7). The step shows the number of fetches for each tick and for the whole run, so a user sees the cost before starting (watchlist risk "fetch cost").
4. **Outputs.** CSV path, report path, and the watchlist name. "Save" writes the watchlist file. "Start" starts the grid; it saves first when a name is set.

### 5.2 Live view

The grid from watchlist design 3.2, with P1 and P2:

- One row for each target. A preset target has one cell for each health check, in the summary's order.
- Row status: breached, warned, unknown, passed (sort order).
- The footer shows `run N of M · next in Ns`, from `cli::repeat`, the same as the repeating view.
- `Enter` opens the detail pane: each rule's value against its threshold, a `cli::charts::Sparkline` per metric, and the price and volume charts (N3).
- `Esc` from the grid stops the run and asks to write the report when one is set. The final report has the same sections as `market summary --report` for each preset target, plus a table of every rule's last value.

### 5.3 From `scope monitor` arguments to the wizard

| `monitor` argument | Wizard field |
|---|---|
| `<TOKEN>`, `--token`, `--chain` | Target 1, resolved as today. |
| `--venue`, `--pair` | Target 1's venue and pair. The kind is pair. |
| `--refresh <secs>` | Interval. |
| `--duration` (new) | Duration. |
| `--export` / `--csv` (new) | CSV path. |
| `--report` (new) | Report path. |
| `HealthArgs` (new) | Preset threshold overrides. |
| `--preset <name>` (new, default `market health` with a venue, none without) | Metrics. |
| `--save <name>` (new) | Watchlist name. |
| `--layout`, `--scale`, `--color-scheme` | Ignored, with one note: the grid has no layouts. Removed one release later. |

So the trigger case is:

```text
scope monitor USDN --venue weex --every 30s --duration 1h
```

`--every` is added as an alias of `--refresh` that takes a duration, so the two commands read the same. In the TUI the line `monitor USDN --venue weex --every 30s --duration 1h` does the same.

## 6. Metric catalog (P3)

`scope::market::series` today has the order-book `MetricId`s. The catalog adds:

| Group | New `MetricId`s | Kinds | Source (compute function, no printing) |
|---|---|---|---|
| token | `PriceUsd`, `PriceChange24hPct`, `Volume24h`, `LiquidityUsd`, `Holders` | token | `crawl::fetch_analytics_for_input` |
| address | `NativeBalance`, `RiskScore` | address | `address::analyze_address`, `RiskEngine::assess_address` |

`token_balance:<symbol>` from the watchlist design is left for later: it is a metric with a parameter, and the catalog has none yet.

The market group stays as it is. The preset uses `MarketSummary::checks` (P1); the market metrics stay available as single rules for users who want one number, for example `spread_pct < 0.5`.

The module may move from `scope::market::series` to `scope::metrics` when token and address metrics join it. See Q6 (section 9).

## 7. Equivalence for the trigger case

A test feeds one recorded WEEX book (mockito, through `ExchangeClient::from_descriptor`) to both paths:

1. `market summary USDN --venue weex --every 1s --duration 2s --format json`;
2. a watchlist with USDN on WEEX and the "market health" preset, run for the same ticks.

For each tick, the check list and the status of path 2 equal the `checks` and `healthy` fields of path 1. The test lives in scope-cli and runs in CI.

## 8. Risks

| Risk | Mitigation |
|---|---|
| **Users of the old dashboard lose its layouts** (chart focus, feed, exchange). | Q1. The detail pane keeps the charts. The release notes name the replacement for each layout. |
| **Removing `run_direct` deletes about 10,600 lines (`cli/monitor/`)**, with their tests, and the coverage gate drops in the raw count. | Remove it in its own PR after the grid ships, and reset `.coverage-last` in that PR with the reason. |
| **The YAML crate changes under the file format** (#118). | N7: one loader function. The format is plain YAML that every crate reads the same. |
| **Fetch cost** of many targets at a short interval. | The wizard shows the fetch count (5.1 step 3). Each tick fetches each source once for each target. |
| **`Route::Wizard` is a new route kind.** The parity test and `write_help` read the routes. | The parity test gets a case for it. The help lists `monitor` as "opens the monitor wizard". |
| **`TuiPicker` inside the wizard.** The wizard is a screen, not a pane command; the asker slot belongs to a running command. | The wizard runs target resolution as a short pane-style task with its own `Output::with_asker`, the same as `TuiState::start`. A test drives a symbol with two chains. |
| **Terminal bell and flash** differ between terminals. | The banner is the reliable signal (watchlist design). |
| **Web UI and `Warn` (P2).** The web API returns check status. | P2 adds `Warn` to the grid only. The JSON of `market summary` does not change. |

## 9. Decisions on the open questions (Jon, 2026-10-16)

| # | Question | Decision |
|---|---|---|
| Q1 | Remove the old dashboard when the grid ships? | Keep it one release behind `scope monitor --dashboard`, with a deprecation note. Phase M removes it in the release after. |
| Q2 | Whale and volume-spike alerts have no metric. | Drop them in #47, with a note on load. #122 tracks the `LargestTrade` and `VolumeChangePct` metrics. |
| Q3 | Does `setup` in suspend mode count as "runs in the TUI"? | Yes. Plan F's setup form stays a separate step. |
| Q4 | Is headless `scope watch` part of #47? | No. It ships after #47, on the same engine (#123). |
| Q5 | `--every` on more commands? | No. Repeats of other metrics go through watchlists only. |
| Q6 | Move `scope::market::series` to `scope::metrics`? | Yes, with a re-export from the old path for one release. |
| Q7 | Wizard defaults for interval and duration? | One set for the CLI and the wizard: `market summary`'s (60s, 1h). |

## 10. Implementation notes

- **Ctrl-G, not Tab, switches between the grid and the shell.** Tab completes in the shell (watchlist plan L), so it cannot also switch screens. The status line shows `watch <name>: N breached · Ctrl-G grid` while the grid is hidden.
- **The run is a second future in the event loop**, next to the pane command. Shell commands run while the grid ticks. Ticks reach the grid through a channel that the loop drains before each draw.
- **`watch [list | open <name|file.yaml> | stop]`** is the session command for saved watchlists. The wizard (`monitor …`) starts the same run.
