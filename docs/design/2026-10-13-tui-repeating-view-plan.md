# TUI Repeating View (#58) Implementation Plan

> For agentic workers: use superpowers:subagent-driven-development. One implementer per task, a task review after each, then a whole-branch review.

**Goal:** a command that runs with `--every` shows one in-place view: the latest snapshot and one chart per key value.
**Spec:** `docs/design/2026-10-13-tui-repeating-view-design.md` (approved by Jon on 2026-10-13).
**Standard:** Clean Code and Clean Architecture. Each concern has one source, adapters are thin, and dependencies point inward. Colors come only from `scope::display::theme` roles.

## Global constraints
- Commit as `robot-accomplice <robot@accomplice.ch>`. No AI attribution.
- Gates: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `just tui-test`.
- Coverage stays at or above the gate in `.coverage-last`.
- CLI plain output stays byte-identical, except the appended CSV columns.
- Write tests first and record a failing run (RED) for each behavior change.

## Review focus
1. A run that fails in the middle of a repeat. The view keeps the earlier runs, the strip shows `✗`, and the footer is `✗ Failed in`.
2. A metric with no value (`None`), for example DEX `n/a` values. The chart shows a gap, not a zero.
3. A resize during a repeat. The charts re-fit with no overlap.
4. More than 500 runs. The ring drops the oldest runs, and the charts and the summary stay correct.
5. Ctrl-Y or `save` during a run. They use the latest snapshot and the series so far.

## Tasks

### RV1: key values in the domain (`scope::market::series`)
- **Files:** new `crates/scope-core/src/market/series.rs`; `market/mod.rs` exports it.
- **Produces:**
  - `MetricId` (`Mid`, `PegDeviationPct`, `SpreadPct`, `BidDepth`, `AskDepth`, `BidTop3`, `AskTop3`, `BidTop10`, `AskTop10`, `Healthy`).
  - `Threshold { Max(f64), Min(f64), Target(f64) }`.
  - `Metric { id, label: &'static str, unit, threshold: Option<Threshold>, csv_column: &'static str }`.
  - `summary_metrics(&HealthThresholds) -> Vec<Metric>`.
  - `Sample { values: Vec<Option<f64>>, healthy: bool, failed_checks: Vec<String> }`.
  - `sample(&MarketSummary, &[Metric]) -> Sample`.
- **CSV order:** the first columns repeat today's header order exactly (`best_bid`, `best_ask`, `mid_price`, `spread`, `bid_depth`, `ask_depth`, `healthy`). New columns are appended after them. The CSV can use a separate ordering function, but both lists are built from the one metric table.
- **Tests:** every `MetricId` is covered; thresholds come from `HealthThresholds`; `None` fields stay `None`; failed checks are listed.

### RV2: one repeat runner and the series sink
- **Files:**
  - new `crates/scope-cli/src/cli/repeat.rs`;
  - `cli/output.rs`: series hook `record_run(RunRecord)` next to the progress hook;
  - `cli/market.rs`: `run_summary` becomes `repeat::run`;
  - the CSV writer is built from `series`.
- **Produces:**
  - `Schedule { every, duration }`.
  - `repeat::run(schedule, out, step) -> Result<()>`, where `step: FnMut(run) -> Future<Result<RunOutput>>`.
  - `RunOutput { sample, snapshot: String }`.
  - `RunRecord { run, at, sample, snapshot }`.
  - `Output::record_run`.
  - `Output::series_hook()`. It is `Some` only for the TUI capture, and it keeps a ring of at most 500 runs.
- **Behavior:**
  - In the CLI, each run still prints its report, and the CSV row is written from `sample`.
  - In the TUI, the run's report goes into the record's `snapshot`, not into the pane stream.
  - Progress (`run N of M · next in Ns`) comes from the runner only.
- **Tests:** run count and the last-run rule (moved from market.rs tests); an error ends the repeat; the CSV header and row match today's for the old columns, with new columns appended; the TUI capture gets records and no repeated reports.

### RV3: one chart component
- **Files:** new `crates/scope-cli/src/cli/charts.rs`. The monitor's sparkline code moves into it, and `monitor/widgets.rs` uses it.
- **Produces:**
  - `charts::Sparkline { values: &[Option<f64>], threshold: Option<Threshold>, role }`. It renders gaps for `None` and a threshold line.
  - `charts::HealthStrip { runs: &[bool] }`.
- **Tests:** a gap stays a gap; the threshold line row; width fitting (the last N values); the monitor renders as before (golden of one monitor frame).

### RV4: the repeating view in the TUI
- **Files:** new `crates/scope-cli/src/cli/tui/repeat_view.rs`; `tui/app.rs` (render switch, keys after the stop, Ctrl-Y and `save` sources); `tui/exec.rs` (help).
- **Behavior:** per spec section 3.4.
  - Left: the snapshot. Right: the trends.
  - The health strip shows the last failure under it.
  - At 80×24: as many charts as fit, with no overlap.
  - Below that: the trends move under the snapshot.
  - After Esc or Ctrl-C, or the end of the run: the view stays with its footer. The next key leaves it and pushes the final snapshot plus `N runs · K failed (run R)` into the scrollback.
  - `save x.csv` writes the series; `save x.txt` writes the snapshot. Ctrl-Y copies the snapshot.
- **Tests:** the view renders at 120×36 and 80×24 (no overlap, fewer charts at 80×24); the view stays after Esc; leaving pushes one copy; the review focus items 1–5.

### RV5: harness scenario
- **Files:** `tests/tui/scenarios/repeating-view.yaml`, plus recorded Binance fixtures (reviewed for secrets).
- **Steps:** `market summary USDC --venue binance --every 2s --duration 8s` at 120×36. Wait for `run 3 of`. Check the trends labels (`Mid price`, `Spread %`, `Bid depth`, `Health`). Press Esc and check that the view stays. Press any key and check the scrollback summary. Repeat at 80×24 and check that no chart label overlaps.

## Order
- RV1 can start at once. It is core only, with no file overlap with the theme step B-2.
- RV2 and RV3 start after B-2 lands: they touch `output.rs` and `monitor/widgets.rs`.
- RV4 depends on RV2 and RV3. RV5 depends on RV4.
