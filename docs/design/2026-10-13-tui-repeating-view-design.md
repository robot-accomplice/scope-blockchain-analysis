# TUI Repeating View (#58): Design

- **Status:** draft for Jon's review. The mockup (v2) is approved visually.
- **Release:** 0.7.0 (Jon, 2026-10-13: hold the release for this feature).
- **Standard:** Clean Code and Clean Architecture. Each concern has one source, the adapters are thin, and dependencies point inward.

## 1. Goal

A command that runs with `--every` shows one screen that updates in place:
- the latest snapshot;
- one chart per key value over the runs so far.

It no longer prints one full report per run into the scrollback.

## 2. Today

Only `market summary` takes `--every`. Its loop is inline in `cli/market.rs::run_summary`. It does three things:
- it prints every run's full report into the output;
- it writes a CSV row from a hand-made list of fields;
- it updates the progress hook (`run N of M`).

## 3. Design

### 3.1 Key values: one list per command (domain, scope-core)
- New module `scope::market::series`.
- `MetricId`: `Mid`, `PegDeviationPct`, `SpreadPct`, `BidDepth`, `AskDepth`, `BidTop3`, `AskTop3`, `BidTop10`, `AskTop10`, `Healthy`.
- `struct Metric { id, label, unit, threshold: Option<Threshold> }`. `Threshold` is `Max(f64)`, `Min(f64)` or `Target(f64)`. Thresholds come from `HealthThresholds`: the peg target, `max_spread_pct` and `min_depth`.
- `fn summary_metrics(&HealthThresholds) -> Vec<Metric>` defines the list once.
- `fn sample(&MarketSummary) -> Sample { values: Vec<Option<f64>>, healthy: bool, failed_checks: Vec<String> }` reads one run.
- This module has no presentation types.

### 3.2 One repeat runner (application, scope-cli)
- New `cli::repeat::run(schedule, out, step)`. It owns the interval, the duration, the run count, the progress hook, the final-run rule (`repeat_total`) and the rule that an error ends the run.
- Each command supplies `step` (one run). `step` returns its `Sample`, plus the snapshot text it printed.
- `run_summary` becomes `repeat::run(...)` over `run_summary_once`. No second loop exists anywhere.

### 3.3 One sink for samples (boundary)
`Output` gets a series hook next to the progress hook: `out.record_run(RunRecord { run, at, sample, snapshot })`.
- **CLI (non-TUI):** the hook appends the CSV row. The CSV columns come from `summary_metrics`. The existing columns stay first, in their order, and new columns are appended after them. The run report prints as today.
- **TUI:** the hook keeps the records in a ring of at most 500 runs. The pane does not print one report per run.

### 3.4 Repeating view (TUI adapter)
- When a pane command has run records, the pane draws `RepeatingView` instead of the scrolling output.
  - **Left:** the latest snapshot text.
  - **Right:** the trends. Each metric gets a label, a sparkline over the last N runs (N = the column width), a threshold line under it when the metric has a threshold, and the latest value.
  - **Health:** a `✓`/`✗` strip, with the last failure under it (`run 16: spread 3.4%`).
- **Chart component:** one shared widget, `cli::charts::Sparkline`. The monitor and the repeating view both use it; the monitor's own sparkline code moves into it. Colors come only from the theme roles.
- **Stop:** Esc or Ctrl-C stops the run. The view stays, with the final snapshot and charts and the footer `Cancelled after N.Ns` or `Done in N.Ns`. The next key press leaves the view and pushes one copy of the final snapshot plus a one-line summary (`42 runs · 1 failed (run 16)`) into the scrollback.
- **Ctrl-Y and `save`:** they take the final snapshot (stdout only) and the CSV-shaped series. `save run.csv` writes the series.
- **Size:** at 80×24 the view shows the snapshot plus as many charts as fit, without overlap. Below that, the trends column moves under the snapshot. Below 40×10, the too-small message shows (existing).

### 3.5 What does not change
- `--report` and `--csv` behave the same. CSV columns are only appended.
- The CLI's plain output is byte-identical, except for the CSV's new trailing columns.

## 4. Tests
- **scope-core:** `summary_metrics` and `sample` cover every `MetricId` and every threshold.
- **Repeat runner:** a mock step checks the run count, the last-run rule and that an error stops the run.
- **CSV:** the existing columns are unchanged, and the new columns come last.
- **TUI unit tests:** the view renders at 120×36 and at 80×24 (charts never overlap, and fewer charts show at 80×24); the threshold line; the failure strip; the view stays after Esc and leaves on the next key; one scrollback copy after leaving.
- **Harness:** `repeating-view.yaml` replays recorded Binance fixtures at 120×36 and at 80×24. It waits for `run 3 of`, checks the trends labels and the view after Esc, presses a key, and checks the scrollback summary.

## 5. Out of scope
Commands other than `market summary` do not take `--every` today. When one gains it, it declares its metrics in `series` and uses `repeat::run`. No other change is needed.

## 6. Implementation notes (2026-10-14)
- The key that leaves a finished view also does its normal job. A user who types the next command at once loses no character, and Ctrl-D still exits.
- On leaving, the scrollback gets the final snapshot first, then the command's notes (`Completed N run(s)`, saved files), the summary (`N runs · K failed (run R)`) and the footer. The closing lines then stay in view together.
- Below 80 columns the trends sit under the snapshot. Charts are added only while two rows still fit, so they never overlap.
- The ring keeps 500 runs; the summary counts all runs and says `in the last 500` when older runs were dropped.
