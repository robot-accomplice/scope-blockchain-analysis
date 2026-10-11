# TUI Watchlists and Learnability: Design

- **Status:** draft for review
- **Date:** 2026-10-10
- **Issue:** #47 (TUI parity). This design replaces the "monitor wizard" part of #47 (phase 6) and adds new scope.
- **Depends on:** #37 (synthetic AMM book, `n/a` status) and #47 phase 5 (shared market core and repeat runner).

## 1. Goal

scope is a general-purpose blockchain-analysis tool. This design makes the TUI easier to use and more useful.

1. **Monitoring first.** The user watches a set of targets (tokens, pairs, addresses) against rules and sees breaches at once.
2. **Investigation one key away.** From any breach, the user opens the evidence with one key.
3. **Learnable.** A new user finds commands and flags without the README.

### Decisions (agreed 2026-10-10)

| # | Decision |
|---|---|
| D1 | The main job is monitoring. Investigation commands stay one key away. First-run experience is the second priority. |
| D2 | A watchlist is the saved output of the #47 monitor wizard. The wizard edits a watchlist. The grid is its live view. |
| D3 | A rule is a metric threshold: `metric op value`. A target passes when all its applicable rules pass. |
| D4 | Notifications: terminal bell and banner in the TUI, and a headless `scope watch` command with exit codes. |
| D5 | Learnability: tab completion, inline hints, a command palette, and a guided first run. |
| D6 | Build order: engine first (approach A). See section 7. |

### Out of scope

- **Desktop notifications and webhooks.** Not chosen for the first version (D4). Headless mode with exit codes and JSON Lines lets a user connect any notifier.
- **Rule durations** (`… for 5m`) and **boolean expressions.** Not chosen (D3). Thresholds cover the stated needs. A duration needs state for each rule and is a separate change.
- **Exact v3 AMM depth.** Out of scope in #37 (decision C there).

## 2. Watchlist engine

### 2.1 Placement

- `scope::watch` in scope-core holds the types, rule evaluation, snapshot comparison and the file format. It does no I/O.
- scope-cli holds the fetchers. A fetcher calls compute functions that return data. It never calls a printing handler.

| Source | Compute function |
|---|---|
| Market book | shared book fetch from #47 phase 5, then `MarketSummary::from_order_book` |
| Token analytics | `crawl::fetch_analytics_for_input` |
| Address | `address::analyze_address` |
| Risk | `RiskEngine::assess_address` |

### 2.2 Types

```rust
pub enum TargetKind { Token, Pair, Address }

pub struct Target {
    pub kind: TargetKind,
    pub id: String,             // symbol, contract address, or wallet address
    pub chain: Option<String>,  // inferred when absent, as the CLI does
    pub venue: Option<String>,  // required for market metrics on a CEX
    pub rules: Vec<Rule>,
}

pub enum Op { Lt, Le, Gt, Ge, Eq, Ne }

pub struct Rule { pub metric: MetricId, pub op: Op, pub value: f64 }

pub enum RuleStatus {
    Pass,
    Fail,
    NotApplicable(String), // from #37, for example spread on a synthetic book
    Error(String),         // fetch failed: health unknown, never a pass
}
```

A target **passes** when every rule that is not `NotApplicable` is `Pass`. A target with any `Error` and no `Fail` is **unknown**. A target with any `Fail` is **breached**.

### 2.3 Metric catalog

The catalog is a fixed list. Each metric has an ID, a unit, the target kinds it applies to, and the source that supplies it.

| Group | Metrics | Kinds | Source |
|---|---|---|---|
| market | `mid_price`, `spread_pct`, `levels`, `bid_depth`, `ask_depth`, `top3_depth`, `top10_depth`, `bid_ask_ratio`, `peg_deviation_pct` | token, pair | market book |
| token | `price_usd`, `price_change_24h_pct`, `volume_24h`, `liquidity_usd`, `holders` | token | token analytics |
| address | `native_balance`, `token_balance:<symbol>`, `risk_score` | address | address, risk |

When a watchlist loads, a metric that does not apply to its target kind is an error.

**Preset "market health".** The preset expands to the rules that `HealthThresholds` defines today. Values resolve in the same order as `market summary`: watchlist value, then config `market.health`, then the built-in default. So a grid row gives the same results as `scope market summary` for the same target (#47 acceptance).

### 2.4 Watchlist file

Path: `~/.config/scope/watchlists/<name>.yaml`. This file is the "saved metric set" of #47 requirement 6.

```yaml
name: stablecoins
interval: 30s
duration: 1h          # optional; absent = until stopped
outputs:
  csv: ./watch.csv    # optional
  report: ./watch.md  # optional, written at the end
targets:
  - kind: pair
    id: USDC
    venue: binance
    preset: market health
  - kind: token
    id: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
    chain: ethereum
    rules:
      - holders >= 1000
      - liquidity_usd > 1000000
```

The rule syntax in YAML is `<metric> <op> <value>`. The loader parses it into `Rule` and reports the line and the reason for every error.

### 2.5 Runner and events

- The runner is the #47 phase 5 repeat runner: interval, duration, a cancellation token, and a callback for each tick.
- Each tick fetches each source once for each target, then evaluates all rules into a `Snapshot`.
- `diff(previous, current)` gives events: `BreachStarted { target, rule, value }` and `BreachCleared { target, rule }`.
- The grid, the banner, the bell and headless mode all consume these events.

### 2.6 Tests

- Rule evaluation and snapshot comparison are pure: table-driven unit tests, including every `Op` and every status.
- Fetchers: tests against `MockClientFactory`, and against mockito venues through `ExchangeClient::from_descriptor`.
- An equivalence test: the "market health" preset gives the same checks as `market summary` for the same mock book.

## 3. Watchlist UI

### 3.1 Screens

The phase 2 TUI gets three screens over one state.

| Screen | Purpose | Enter |
|---|---|---|
| Grid | Live view of the open watchlist. The home screen when a default watchlist is set. | `Tab`, `watch open <name>` |
| Shell | The phase 2 command pane. | `Tab` |
| Wizard | Create or edit a watchlist. | `watch new`, `watch edit <name>`, `scope monitor …` |

A running watchlist keeps refreshing while the user is in the shell. The status line shows its breach count.

### 3.2 Grid

- One row for each target: name, chain or venue, one cell for each rule (pass, fail, n/a, error), the last update time, and a status badge.
- Breached rows sort first.
- When a breach starts, the row flashes, the bell rings once, and a banner names the rule: `USDC@binance: spread_pct 3.4 > 3.0`. The banner stays until the user presses a key.
- A row that clears stays dim for one tick.

### 3.3 Drill-down

- `Enter` on a row opens a detail pane. It shows each rule's value against its threshold, and the last N ticks.
- Evidence keys run the matching CLI command in the shell pane, with the target filled in:

| Key | Command |
|---|---|
| `b` | `market summary <id> --venue <venue>` (order book) |
| `t` | `market trades <id> --venue <venue>` |
| `h` | `crawl <id>` (holders) |
| `r` | `compliance risk <id>` |
| `x` | `address <id> --include-txs` |

- `Esc` returns to the grid.
- Every evidence view is a real CLI command. The user learns the CLI while they investigate.

### 3.4 Wizard

1. **Targets.** Add a token, pair or address. The wizard infers the chain where it can. The venue comes from a list of the `VenueRegistry` IDs.
2. **Rules.** For each target, pick metrics from the catalog, filtered by target kind. Start from the "market health" preset or from empty. Edit the thresholds.
3. **Timing.** Interval and duration.
4. **Outputs.** CSV and report paths. Then save.

`scope monitor <args>` opens the wizard with the values from its arguments (#47).

Session commands: `watch list`, `watch new`, `watch edit <name>`, `watch open <name>`, `watch default <name>`, `watch stop`.

### 3.5 What happens to the live monitor dashboard

The chart panels in `monitor/widgets.rs` stay (decision C of the #47 plan). When a target has a price or volume metric, the detail pane shows the price and volume charts next to the rules.

### 3.6 Tests

The grid, the detail pane and each wizard step are pure state plus a render function, like `TuiState`. Tests drive them with scripted keys through the phase 2 `Host` trait and check the `TestBackend` buffer.

## 4. Headless `scope watch`

### 4.1 Command

```text
scope watch <name|path> [--once] [--every D] [--duration D]
                        [--format text|json] [--csv PATH] [--report PATH]
```

Flags override the values in the watchlist file. The engine and the runner are the same as the grid's.

### 4.2 Output

- **text:** one summary line for each tick (`12:00:05 3 targets · 1 breach`) and one line for each event.
- **json:** JSON Lines. There is one object for each event and one for each tick snapshot.
- Data goes to stdout. Diagnostics go to stderr (the phase 1 `Output` split).

### 4.3 Exit codes

| Code | Meaning |
|---|---|
| 0 | Every target passed on the last tick. |
| 1 | At least one target breached on the last tick. |
| 2 | No breach, but at least one rule is `Error`: health is unknown. |
| 3 | The watchlist is not valid (YAML, unknown metric, metric not valid for the kind). The command checks this before it fetches. |
| 4 | The CSV or the report could not be written. This code wins over 0, 1 and 2, because the record of the run is lost. |

- `--once` runs one tick and exits. This is the cron and CI mode.
- Without `--once`, the command runs until the duration ends, Ctrl-C, or SIGTERM. A signal stops it cleanly: it writes the report and exits with the code for the last tick.

### 4.4 Parity and routing

`watch` is a new CLI command, so `tui::exec::route` does not compile until it has a route. In the TUI, `watch` opens the grid. It does not run in the shell pane.

### 4.5 RCA trail

- With `--csv`, each tick appends one row for each rule: `ts, target, metric, value, op, threshold, status, source, error`.
- With `--format json`, the event stream is the trail.
- A fetch error records the exact error text, not only "error".

### 4.6 Tests

- One test for each of the five exit codes, with mock fetchers.
- A test that every JSON line parses.
- A cancellation test: send a signal, then check the report file and the exit code.

## 5. Learnability

### 5.1 One source of truth

Completion, hints and the palette read `Cli::command()`, the same as the phase 2 parser and parity test. A new command or flag appears in all three with no extra code. Dynamic values come from existing stores:

| Value | Store |
|---|---|
| `@label` | address book |
| token argument | token aliases |
| `--venue` | `VenueRegistry` |
| `--chain` | chain list |
| watchlist name | `~/.config/scope/watchlists/` |

### 5.2 Tab completion

- Completes the command, the subcommand, the flags and the enum values (`--format <Tab>` gives `table json csv markdown`).
- One match fills in. More than one match opens a small menu above the input. `Tab` cycles. `Enter` accepts.

### 5.3 Inline hints

- A dim line under the input shows the usage of the matched command. The next expected argument is highlighted: `market summary <PAIR> [--venue <VENUE>] …`.
- An unknown command shows clap's "did you mean" text before the user presses Enter.

### 5.4 Command palette

- `Ctrl-K` opens a fuzzy search over every command (with its one-line description), recent history, and saved watchlists.
- `Enter` puts the command in the input line. It does not run it. The user sees and edits it first. A watchlist entry opens the grid.

### 5.5 Guided first run

Trigger: no config file, or `scope tui --welcome`.

1. A welcome screen with three choices: set API keys, start a watchlist, go to the shell.
2. **Set API keys** is a form in the TUI. It calls the setup functions that already take a reader and a writer (`configure_single_key_impl` and others), so setup does not need suspend mode.
3. **Start a watchlist** opens the wizard with a starter watchlist: the USDC and USDT pairs on the built-in `binance` and `coinbase` venues, with the "market health" preset. These are generic, well-known markets. The user edits the list or saves it.
4. For `scope tui`, the binary does not show its "Run setup now? [Y/n]" stdin prompt. One-shot CLI commands keep it.

### 5.6 Tests

- Completion, hints and palette ranking are pure functions of `Cli::command()` and a store snapshot. Table-driven tests cover them.
- A test checks that every visible subcommand appears in the palette (the same guard as the parity test).
- The welcome flow is tested with scripted keys through the `Host` trait.

## 6. Error handling

| Case | Behavior |
|---|---|
| Fetch fails for one source | Its rules become `Error(text)`. Other targets keep updating. The grid shows the error text in the detail pane. |
| Watchlist file is not valid | TUI: the wizard opens on the first error, with the line and the reason. Headless: exit code 3 before any fetch. |
| Venue not in the registry | A validation error, with the list of known venue IDs. |
| Metric not valid for a target kind | A validation error that names the metric, the kind and the valid metrics. |
| Write to CSV or report fails | TUI: a banner and a note in the shell pane. Headless: the error goes to stderr and the exit code is 4. Never silent (phase 1 option C). |

## 7. Build order (approach A)

| Step | Work | Depends on |
|---|---|---|
| 1 | #47 phase 3: monitor-path defects | none |
| 2 | #37: synthetic AMM book, `n/a` status | none |
| 3 | #47 phase 5: shared market core and runner | 2 |
| 4 | Watchlist engine (section 2) | 3 |
| 5 | Headless `scope watch` (section 4) | 4 |
| 6 | Watchlist UI: grid, drill-down, wizard (section 3). Closes #47 phase 6. | 4 |
| 7 | Guided first run (section 5.5) | 6, 8 |
| 8 | Completion, hints, palette (sections 5.1 to 5.4) | phase 2 (merged). Can run in parallel with steps 1 to 6. |
| 9 | #47 phase 7: README and dataflow docs | 6 |

Each step is one PR into `develop`. Each PR leaves the CLI and the TUI working.

## 8. Risks

- **JSON consumers and `n/a`.** #37 adds a third status. The web UI must handle it before step 4 shows it in the grid.
- **Fetch cost.** A large watchlist at a short interval calls many APIs. The runner fetches each source once for each target in a tick. The wizard shows the number of calls for each tick before it saves.
- **Terminal capability.** The bell and the flash depend on the terminal. The banner is the reliable signal, and it does not depend on them.
