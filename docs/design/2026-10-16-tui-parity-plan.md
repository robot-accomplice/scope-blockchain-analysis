# TUI Parity with the CLI (#47): Implementation Plan

> **For agentic workers:** use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. One implementer per task, a task review after each, then a whole-branch review. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** close #47. `scope monitor` becomes the monitor wizard in the one TUI, its live view gives the same health results as `market summary`, and a metric set can be saved and loaded.

**Spec:** `docs/design/2026-10-16-tui-parity-design.md` (draft, needs Jon's approval) and `docs/design/2026-10-10-tui-watchlist-design.md` (approved 2026-10-10), with the changes P1–P3 of the parity design.

**Architecture:** the engine (`scope::watch`) is pure and lives in scope-core: types, rule evaluation, snapshot comparison, file format. scope-cli holds the fetchers, which call compute functions that return data, and the TUI screens (wizard, grid, detail), which are pure state plus a render function, like `TuiState`. The tick loop is `cli::repeat::run`. The metric catalog is `scope::market::series`, extended.

**Tech stack:** Rust 2024, clap 4, ratatui 0.30, crossterm 0.29, tokio. No new crates, except the YAML crate that #118 picks.

**This plan replaces watchlist plans E and U.** Plans H (headless `scope watch`) and F (guided first run) are not in it (parity design, section 3).

## Global Constraints

- Gates before each push: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`, `just tui-test`, and the pre-push coverage hook (no drop below `.coverage-last`).
- Commit messages and PR text: Simplified Technical English, the repo's commit identity.
- Write the test first and record a failing run (RED) for each behavior change.
- Tests never read or write the user's real files. Watchlist files go in a temp dir; `scope::paths` is overridden as the existing tests do.
- Tests that call `TuiState::start` hold `crate::cli::tui::DIAG_LOCK`.
- Network only through mocks: `MockClientFactory`, and mockito venues through `ExchangeClient::from_descriptor`.
- CLI plain output of every command other than `monitor` stays byte-identical.
- One PR per phase into `develop`. Each PR leaves the CLI and the TUI working.

## Review Focus

1. **Same results as `market summary` (P1).** For one book, the grid's preset cells, their order and the row status equal `MarketSummary::checks` and `healthy`, including `Warn` and `n/a` on a synthetic AMM book. Test in Task E4 and Task U8.
2. **A fetch that fails for one target.** Its cells become `Error(text)`; other rows keep updating; the row is "unknown", never "passed". Test in Task E5.
3. **A watchlist file that is not valid** (bad YAML, unknown metric, metric not valid for the kind, unknown venue). The error names the line and the reason; the wizard opens on the first error; nothing is fetched. Test in Task E3 and Task U3.
4. **`scope monitor` arguments into the wizard.** Every row of the parity design's table 5.3 maps; ignored flags give exactly one note. Test in Task U6.
5. **Esc and Ctrl-C at each step** (wizard with changes, grid running, a token question open in the wizard). No panic, no lost report when a report path is set, no stuck asker slot. Tests in Tasks U2, U4 and U5.
6. **Small terminals.** Wizard and grid at 80×24 and 40×10: no overlap, no panic; below 40×10 the too-small message. Tests in Tasks U2 and U4.

---

## Phase order

| Phase | Tasks | PR | Depends on |
|---|---|---|---|
| R | R1 | `fix(tui): crawl with a symbol runs in the pane` | none |
| E | E1–E6 | `feat(watch): watchlist engine` | #118 picks its YAML crate (E3 only) |
| U | U1–U8 | `feat(tui): monitor wizard and watchlist grid` | E |
| M | M1–M2 | `refactor(monitor): retire the old dashboard` | U, and Q1 (now or one release later) |
| D | D1 | `docs: TUI parity` | U |

R can start at once. E1, E2, E4 and E5 do not need the YAML crate; E3 waits for #118 or uses its pick.

---

## Phase R

### Task R1: `crawl` with a symbol routes to the pane (G5)

**Files:** `crates/scope-cli/src/cli/tui/exec.rs` (`route`, `test_routes_for_prompting_commands`), `tests/tui/scenarios/outside-crawl.yaml` (rename to `pane-crawl.yaml`).

- [ ] Change the test: `crawl USDC` routes to `Route::Pane`. Run it: it fails (RED).
- [ ] In `route`, `Commands::Crawl(_) => Route::Pane`. Remove the comment about the prompt.
- [ ] Add a test with `MockClientFactory` and a DEX source that returns two candidates: `crawl USDC` in the pane puts an `Ask` with two options in the asker slot; the answer `1` gives the second token's report.
- [ ] Add a test: the "Save this token?" question is a yes/no `Ask` in the pane, and "no" saves nothing.
- [ ] Change the harness scenario: it waits for the token list in the pane, presses Down and Enter, and checks the report. No suspend marker.
- [ ] Run the gates.

---

## Phase E: watchlist engine

### Task E1: token and address metrics in the catalog (P3)

**Files:** `crates/scope-core/src/market/series.rs` (or `scope::metrics`, after Q6), `crates/scope-core/src/market/mod.rs`.

- [ ] Tests first: `MetricId::ALL` contains `PriceUsd`, `PriceChange24hPct`, `Volume24h`, `LiquidityUsd`, `Holders`, `NativeBalance`, `RiskScore`; each has a label, a unit and a `MetricGroup` (`Market`, `Token`, `Address`).
- [ ] Add `MetricGroup` and `fn group(MetricId) -> MetricGroup`. `describe` covers the new ids; they have no default threshold.
- [ ] Add `fn applies_to(MetricId, TargetKind) -> bool`: market metrics to token and pair, token metrics to token, address metrics to address.
- [ ] Check that `summary_metrics` and the CSV column order of `market summary --csv` do not change (existing tests stay green).
- [ ] If Q6 is "yes": move the module to `scope::metrics` and keep `pub use` at `scope::market::series`.

### Task E2: types and rule evaluation (`scope::watch`)

**Files:** new `crates/scope-core/src/watch/mod.rs`, `crates/scope-core/src/watch/rule.rs`; `crates/scope-core/src/lib.rs`.

- [ ] Tests first, table-driven: every `Op` (`Lt`, `Le`, `Gt`, `Ge`, `Eq`, `Ne`) against a value below, at and above the limit; `None` value gives `Error("no value")`.
- [ ] Types from the watchlist design 2.2, with P2: `RuleStatus { Pass, Warn(String), Fail(String), NotApplicable(String), Error(String) }`.
- [ ] `Target { kind, id, chain, venue, preset: Option<Preset>, rules }`; `Preset::MarketHealth { overrides: HealthOverrides }`.
- [ ] `fn target_status(&[RuleStatus]) -> TargetStatus { Passed, Warned, Unknown, Breached }`. Tests: any `Fail` is breached; `Error` without `Fail` is unknown; `Warn` without `Fail` and `Error` is warned; only `NotApplicable` is passed.
- [ ] No I/O in this module (a test-free check: `scope::watch` imports no `std::fs`, `reqwest` or `tokio`).

### Task E3: watchlist file (requirement 6)

**Files:** new `crates/scope-core/src/watch/file.rs`; `crates/scope-core/src/paths.rs` (`watchlists_dir()`).

- [ ] Tests first: the example file of the watchlist design 2.4 loads; `save` then `load` gives an equal `Watchlist`; the rule text `holders >= 1000` parses; `preset: market health` with `health:` overrides parses.
- [ ] Errors, one test each, every one with the line and the reason: bad YAML; unknown metric; metric not valid for the kind (names the valid metrics); unknown venue (names the known venue IDs); `interval: 0s`; a pair with no venue.
- [ ] `load(path) -> Result<Watchlist, Vec<FileError>>` reports every error, not only the first.
- [ ] `pub fn load` and `pub fn save` are the only functions that touch the YAML crate (N7).
- [ ] `paths::watchlists_dir()` is `<config dir>/watchlists`. `list()` returns the names, sorted; a missing folder gives an empty list.

### Task E4: the "market health" preset as one check set (P1)

**Files:** new `crates/scope-core/src/watch/preset.rs`.

- [ ] Test first: for a fixed `OrderBook`, `preset_statuses(&summary)` gives one `RuleStatus` per `HealthCheck` in `MarketSummary::checks`, in order: `Pass` to `Pass`, `Warn` to `Warn`, `Fail` to `Fail`, `NotApplicable` to `NotApplicable`; each keeps the check's message.
- [ ] Test: on a synthetic AMM book (`order_book_from_analytics`), the `n/a` checks stay `NotApplicable` and do not make the row breached.
- [ ] Test: `HealthOverrides` resolve in the same order as `market summary`: watchlist value, then config `market.health`, then the default. Reuse the function `HealthArgs` uses; do not copy it.
- [ ] The preset never expands into `Rule`s.

### Task E5: fetchers and one tick (scope-cli)

**Files:** new `crates/scope-cli/src/cli/watch/mod.rs`, `crates/scope-cli/src/cli/watch/fetch.rs`.

- [ ] Tests first with `MockClientFactory` and a mockito venue: a pair target with the preset gets its book once per tick through `scope::market::fetch_book_and_volume`, and its statuses through E4.
- [ ] A token target's rules read `crawl::fetch_analytics_for_input`; an address target's read `address::analyze_address` and `RiskEngine::assess_address`. None of them prints: the fetchers pass an `Output` that captures, and the test checks it is empty of report text.
- [ ] Each source is fetched once per target per tick, even with many rules on it (count the mock calls).
- [ ] A failed fetch: that target's rules become `Error(<exact error text>)`; the other targets in the same tick are evaluated. Review focus 2.
- [ ] `fn tick(&Watchlist, …) -> Snapshot`.

### Task E6: snapshot comparison and the run

**Files:** `crates/scope-core/src/watch/diff.rs`; `crates/scope-cli/src/cli/watch/run.rs`.

- [ ] Tests first: `diff(prev, cur)` gives `BreachStarted` and `BreachCleared` (and `WarnStarted`, `WarnCleared` for P2); no event when nothing changes; the first tick compares with an empty snapshot.
- [ ] `watch::run` calls `cli::repeat::run` with the watchlist's `Schedule`. Each run's `Sample` is one value per rule; the CSV row is the RCA trail of the watchlist design 4.5 (`ts, target, metric, value, op, threshold, status, source, error`).
- [ ] With a report path, the end of the run writes the report: one `market summary --report` section per preset target (reuse `market_summary_to_markdown`), then a table of every rule's last value. Test: the preset section equals the `market summary --report` text for the same book.
- [ ] A CSV or report write error is returned, never dropped.

---

## Phase U: monitor wizard and grid

### Task U1: `Route::Wizard` and the screen switch

**Files:** `crates/scope-cli/src/cli/tui/exec.rs`, `crates/scope-cli/src/cli/tui/app.rs`, `crates/scope-cli/src/cli/tui/mod.rs`.

- [ ] Test first: `route(monitor USDC)` is `Route::Wizard`; the parity test `test_every_cli_command_parses_and_has_a_tui_route` accepts it.
- [ ] `TuiState` gets `screen: Screen { Shell, Wizard(WizardState), Grid(GridState) }`. `Tab` switches between Shell and a running Grid.
- [ ] `write_help` lists `monitor` as "opens the monitor wizard" and the session commands `watch list`, `watch open <name>`, `watch edit <name>`, `watch stop`.
- [ ] Add the watchlist names to `Vocab` and to the palette (watchlist plan U note).

### Task U2: wizard steps 3 and 4 (timing, outputs)

**Files:** new `crates/scope-cli/src/cli/tui/wizard.rs`.

- [ ] Tests first with scripted keys and `TestBackend`: `Tab`/`Shift-Tab` move between steps; an interval of `0s` shows an error and blocks "Start"; the fetch count updates when the interval changes.
- [ ] Defaults from Q7.
- [ ] `Esc` with changes asks "Leave without saving? [y/N]"; without changes it leaves at once.
- [ ] Renders at 120×36, 80×24 and 40×10 with no overlap. Review focus 6.

### Task U3: wizard steps 1 and 2 (targets, metrics)

**Files:** `crates/scope-cli/src/cli/tui/wizard.rs`.

- [ ] Tests first: `a`, then `USDC`, then Enter adds a target through `resolve` with `TuiPicker`; a symbol on two chains shows the list; Esc in the list adds nothing and frees the asker slot. Review focus 5.
- [ ] The venue list is the `VenueRegistry` IDs. A pair without a venue cannot be saved.
- [ ] Step 2 lists only the metrics where `applies_to` is true. The preset is on by default for a pair or a token with a venue. A threshold's initial value comes from the E4 resolve order.
- [ ] `watch edit <name>` on a file that is not valid opens the wizard on the first error, with the line and the reason. Review focus 3.
- [ ] Save writes through `watch::file::save`. Test: the file loads back equal.

### Task U4: the grid

**Files:** new `crates/scope-cli/src/cli/tui/grid.rs`.

- [ ] Tests first with a scripted `Snapshot` stream: one row per target; a preset row has one cell per health check; rows sort breached, warned, unknown, passed; the footer shows `run N of M · next in Ns`.
- [ ] On `BreachStarted`: the banner names the target and the check; it stays until a key; the bell rings once.
- [ ] A row that clears is dim for one tick.
- [ ] The grid keeps refreshing while the user is in the Shell screen; the status line shows the breach count.
- [ ] `Esc` stops the run; with a report path, the report is written and the note names the file. Review focus 5.
- [ ] Renders at 120×36, 80×24 and 40×10. Review focus 6.

### Task U5: the detail pane and drill-down keys

**Files:** new `crates/scope-cli/src/cli/tui/detail.rs`.

- [ ] Tests first: `Enter` opens the detail; each rule shows its value against its threshold and a `cli::charts::Sparkline` of the last runs; with a price or volume metric, the price and volume charts show (N3).
- [ ] Keys `b`, `t`, `h`, `r`, `x` put the matching CLI command in the input line and run it in the shell pane (watchlist design 3.3). Test: `b` on USDN@weex runs `market summary USDN --venue weex`.
- [ ] `Esc` returns to the grid.

### Task U6: `scope monitor` opens the TUI with the wizard filled (N1, N4)

**Files:** `crates/scope-cli/src/cli/monitor/mod.rs` (`MonitorArgs`), `crates/scope-cli/src/cli/dispatch.rs`, `crates/scope-cli/src/cli/interactive.rs`.

- [ ] Tests first, one per row of parity design table 5.3: `MonitorArgs` to `Watchlist` (`fn watchlist_from_args`). `--every 30s` and `--refresh 30` give the same interval. `--layout`, `--scale` and `--color-scheme` give one note each, once.
- [ ] Add the flags `--every`, `--duration`, `--csv` (with `--export` as a hidden alias), `--report`, `HealthArgs` (flatten), `--preset`, `--save`, `--yes`.
- [ ] `dispatch` sends `Commands::Monitor` to `interactive::run` with a start action `OpenWizard(Watchlist)`; with `--yes`, `StartGrid(Watchlist)`. Acceptance 4: `scope monitor` and `scope tui` call the same entry.
- [ ] The old dashboard stays reachable as `scope monitor --dashboard` until phase M (Q1).

### Task U7: monitor alerts to rules (N5)

**Files:** `crates/scope-cli/src/cli/watch/mod.rs`.

- [ ] Tests first: `monitor.alerts.price_min` and `price_max` become the rules `price_usd >= min` and `price_usd <= max`; whale and volume-spike settings give one note, on the first load only.
- [ ] Follow Q2 if Jon picks the new metrics instead.

### Task U8: equivalence and harness scenarios

**Files:** new `crates/scope-cli/tests/monitor_equivalence.rs` (or a test module in `cli/watch`), `tests/tui/scenarios/monitor-wizard.yaml`, `tests/tui/scenarios/monitor-pair.yaml` (change).

- [ ] The parity design section 7 test: one recorded WEEX book through `market summary … --format json` and through a watchlist with the preset; per tick, the statuses and the order are equal. Review focus 1.
- [ ] Harness `monitor-wizard.yaml` at 120×36: `monitor USDN --venue weex --every 2s --duration 6s` with recorded fixtures (reviewed for secrets); Enter starts the grid; wait for `run 3 of`; check the check names; Esc; check the report note.
- [ ] Change `monitor-pair.yaml` to the wizard flow.

---

## Phase M: retire the old dashboard

### Task M1: remove `--dashboard` and `run_direct`

Only after Q1 says when.

**Files:** `crates/scope-cli/src/cli/monitor/` (most of it), `crates/scope-core/src/config.rs` (`MonitorConfig`: keep `refresh_seconds`, `alerts`, `venue`; drop `layout`, `widgets`, `scale`, `color_scheme` with a load warning), `CHANGELOG.md`.

- [ ] Before the removal: a list of each monitor feature and its replacement (grid, detail charts, `b`, `t`), in the PR text and the CHANGELOG.
- [ ] Remove the code and its tests. Keep `MonitorArgs` (now the wizard's arguments).
- [ ] A config with the dropped keys still loads, with one warning. Test.
- [ ] Reset `.coverage-last` in this PR, with the before and after numbers and the reason.

### Task M2: `scope interactive` and `scope monitor` share help text

**Files:** `crates/scope-cli/src/cli/mod.rs`.

- [ ] Change the doc comment of `Commands::Monitor`: "Open the monitor wizard in the TUI." Test: `scope monitor --help` names the wizard and the trigger-case example.

---

## Phase D

### Task D1: docs and the issue

**Files:** `README.md`, `docs/monitor-layout-architecture.md` (replace or remove), `CHANGELOG.md`.

- [ ] README: the monitor section shows the wizard, the watchlist file, and the trigger case as `scope monitor USDN --venue weex --every 30s --duration 1h`.
- [ ] Post the new coverage table on #47 (every row "pane", "wizard" or "outside: setup"), and the acceptance check, before closing it.
