# Grid Peg Deviation Chart and Venue Editor: Design

- **Status:** decided on 2026-11-11 (section 0 has the decisions). Item 1 (the chart) is built. Item 2 (the editor) is not built yet.
- **Scope:** the two items that `2026-11-10-market-venue-followups-design.md` (section 8) deferred. The owner chose both for 0.8.8:
  1. a per-venue peg deviation chart in the repeat view of `market summary --every` and in the watch grid;
  2. a venue editor in the watch grid: change the venues of a target without delete and add.
- **Base:** `develop` at 57d7367.
- **Reads:** `2026-11-04-watch-venue-list-design.md` (Q8, and the venue editor note in section 2.7), `2026-11-10-market-venue-followups-design.md` (section 8).
- **Delivery:** two PRs, one for each item (Q-P in section 5). The chart goes first. It is read-only and has no state.

## 0. Decisions

The owner decided on 2026-11-11. The owner chose the recommended option of every question except Q-D7.

| Question | Chosen option | Date |
|---|---|---|
| Q-D1 reference of the deviation | a. The peg only | 2026-11-11 |
| Q-D2 position of the chart | b. First in the grid; old slot in the repeat view | 2026-11-11 |
| Q-D3 scale | a. Always include the on-peg band | 2026-11-11 |
| Q-D4 isolated view | b. A braille line with the band | 2026-11-11 |
| Q-D5 user rule `peg_deviation_pct` in the grid | a. Show both charts | 2026-11-11 |
| Q-D6 DEX tag | a. Tag `usd` | 2026-11-11 |
| **Q-D7 watch CSV row** | **b. Yes: a `peg_deviation_bps` row, with source `chart`** (not the recommended option) | 2026-11-11 |
| Q-E1 how an edit takes effect | b. Between ticks, in the same run | 2026-11-11 |
| Q-E2 key | a. `v` | 2026-11-11 |
| Q-E3 input line | a. Prefilled with the current venues | 2026-11-11 |
| Q-E4 write the saved watchlist | a. No; `Ctrl-S` saves | 2026-11-11 |
| Q-E5 what `Ctrl-S` writes | a. The whole list, with a guard | 2026-11-11 |
| Q-E6 undo | a. One level | 2026-11-11 |
| Q-E7 shared venue check | a. In `wizard.rs` | 2026-11-11 |
| Q-P number of PRs | a. Two | 2026-11-11 |

Where the text below still shows a recommendation, the table above is the decision.

## 1. Why these two were deferred, and what changed

Both items were left out on 2026-11-10 for one reason each:

| Item | Reason then | What changed |
|---|---|---|
| Deviation chart | Nobody had said the venue overlay is hard to read. | The owner chose it for 0.8.8. The data now exists: `comparison` rows have `peg_deviation_bps` (PR #192, shipped in v0.8.7). |
| Venue editor | Nobody had said delete-and-add is a problem. | The owner chose it for 0.8.8. |

This document does not re-argue those reasons. It designs the two items so that they are small and safe.

## 2. Terms

- **Peg deviation:** `(mid - peg) / peg`, in basis points (bps). One bps is 0.01%. The one definition is `MarketSummary::peg_deviation_bps` (`scope-core/src/market/health.rs`).
- **On-peg band:** the peg plus or minus `peg_range`. The default range is 0.001, so the band is plus or minus 10 bps. `PegBands::classify` returns `OnPeg` inside it.
- **Outlier band:** `peg_range` times `PEG_OUTLIER_FACTOR` (5). The default is plus or minus 50 bps. `PegBands::classify` returns `Slight` inside it and `Off` outside it.
- **Venue chart set:** the charts of `tui/venue_charts.rs`. One `VenueChart` for each metric. One line for each venue. One legend. The grid and the repeat view both call `venue_charts::draw`.

---

# 3. Item 1: the peg deviation chart

## 3.1 What the code does today

| Part | Today |
|---|---|
| Chart set, shared code | `venue_charts.rs`: `VenueChart { label, threshold, unit, series }`, `Series { venue, values, status }`, `draw`, `shortfall`, `Selection::apply` (keys `←` `→` Space Tab PgUp PgDn). The grid (`grid.rs`, `GridState::venue_charts`) and the repeat view (`repeat_view.rs`, `venue_charts_of`) both build a `VenueCharts` and call `draw`. |
| Repeat view | `venue_charts_of` makes one chart for each metric of `Series.metrics` that `charted()` keeps. `charted()` drops only `BestBid`, `BestAsk`, `Spread` and `Healthy`. The metric list is `summary_metrics`. It contains `PegDeviationPct`. **So a "Peg deviation %" chart already exists in the repeat view of 2 or more venues.** It is in percent, it has no limit line, and its scale fits the data only. `min_span` for `Unit::Percent` is 0.05 points (5 bps). |
| Grid | The charts are the cells of the selected market row: the checks of the preset and the market rules. History is `GridState.history`, keyed by (row label, cell label), 120 values (`HISTORY`). A cell has a value only when a preset check message has a number (`preset::check_value`) or a rule reads a metric. **There is no peg deviation cell.** A user can add the rule `peg_deviation_pct < 0.1`. Then the grid has a percent chart with a limit line. |
| Where the grid can get the mid | `Tick.summaries[i]` is the `MarketSummary` of row `i` of the snapshot (`watch/fetch.rs`). The grid receives the whole `Tick` in `GridMsg::Tick` and then keeps only `c.value` of each cell. The mid and the bands are in the summary, and the grid does not use them. |
| Scale rules | In the all-venues view, the scale fits the venue values and `min_span`. A limit line shows only when it is on the scale (design 2026-11-04, section 2.9, point 3). The isolated view is the bar `Sparkline`, with one threshold line. |
| Venue colors | `series_style(i)` picks `SERIES_TONES[i % 4]`: blue, magenta, cyan, plain. A test (`test_series_styles_are_no_health_color_and_repeat_after_four` in `theme.rs`) forbids the health colors. The fifth venue and later use dots. |
| Colour off | `plain::strip_color` removes colors. The legend and the value box name each venue with its value. Test: `with_colour_off_every_venue_still_has_its_name_and_value` in `grid.rs`. |
| Window too small | `venue_charts::shortfall` and `enlarge_hint` give "enlarge it N columns wider and M rows taller to see the venue charts". Test: `test_a_window_too_small_for_the_venue_charts_says_how_much_to_enlarge_it`. |
| DEX venue | `order_book_from_analytics` makes a synthetic AMM book. Its prices are the pool USD price (`market/analytics.rs`). The mid, and so the deviation, exist. There is no trade tape. |

Two findings change the task:

1. The repeat view already charts deviation, but in percent and with no band. The work there is a change, not an addition.
2. In the grid the deviation is not a cell. The grid must carry its own series. Section 3.2 says how.

## 3.2 Proposal

### Data

- One chart, `Peg deviation (bps)`, in the venue chart set. It has one line for each venue. The value is `MarketSummary::peg_deviation_bps(mid)` for that venue and tick.
- **One helper.** Add `MarketSummary::mid_peg_deviation_bps()` in `scope-core`: `mid_price.and_then(|m| self.peg_deviation_bps(m))`. The expression exists now in `ComparisonRow::from_report` (`market_venues.rs`) and in `insights.rs` (`peg_line`). The chart is the third caller. All three call the helper. This is the rule 3 fix: the chart and the `comparison` JSON cannot disagree.
- **Grid.** `GridState` gets `deviation: HashMap<String, VecDeque<Option<f64>>>`, keyed by row label, with the same `HISTORY` limit. `apply(GridMsg::Tick)` fills it from `tick.summaries` zipped with `tick.snapshot.targets` (the two vectors line up by index; `fetch.rs` documents it). A failed venue or a row with no mid gives `None`, which is a gap, not zero.
- **Band, grid.** `PegBands` comes from the summary (`summary.peg_bands()`). All venues of one target use the same thresholds (`preset::thresholds(target, base)`), so one band serves the chart. The grid takes it from the newest summary of any venue row of the target.
- **Repeat view.** The series already holds the `PegDeviationPct` value of each venue and run (in percent). The chart multiplies by `BPS_PER_PERCENT`, the same factor that `peg_deviation_bps` uses. The bands are not in the series today. `output::Series` gets one field, `peg: Option<PegBands>`. The repeat runner sets it next to `metrics`, from the thresholds that `--peg` and `--peg-range` and the config give. A repeat run on one venue does not use the chart.
- **No new `MetricId`, no new cell.** A cell would change the preset contract ("the grid shows `MarketSummary::checks` one to one", `watch/preset.rs`). It would add a column to every market row. The chart is a view. It does not need to be a cell. The record in the watch CSV is a row of its own, with source `chart` (Q-D7 b, section 3.4).

### Where it fits: one more chart in the set

The deviation chart is one more `VenueChart` in the existing set. It is not a separate view.

Reasons:

1. All keys work with no change: `←` `→` select a venue, Space isolates, Tab moves to the next chart, PgUp and PgDn page. `Selection::apply` is one function for the grid and the repeat view.
2. The legend, the value box, the paging, the "window too small" hint and the 40-column minimum are reused.
3. A separate view needs a new key, a new layout and a new "too small" rule, for a chart of the same shape.

Position (Q-D2):

- **Grid:** first. The first page then shows the peg picture. A preset has about 10 checks, so a last position is on page 2 at 80x24.
- **Repeat view:** in the place of the existing "Peg deviation %" chart, which is after "Mid price". The repeat view starts with the price, and it already shows deviation in the same slot. The chart count does not change, so the paging does not change.

The grid and the repeat view then differ in order. Each keeps its own first chart. Q-D2 asks if the owner wants one order.

### Scale and band

```
┌ Peg deviation (bps) ─ weex +2.1  binance -3.4  eth +14.0 ┐┌ venues: Peg deviation (bps) ┐
│+15┤ ·········································   ⣀⣀⠤⠤ ││  all  -3.4 – +14.0          │
│   │                              ⣀⣀⠤⠤⠒⠒⠉⠉⠉⠉⠒⠒⠤⠤⣀⣀     ││  band ±10 bps               │
│ 0 ┤ ─ ─ ─ ⠤⠤⠒⠒⠉⠉⠉⠒⠒⠤⠤⣀⣀⣀⠤⠤⠒⠒                  ││> ■ weex      +2.1 ✓         │
│-10┤ ·········································    ││  ■ binance   -3.4 ✓         │
│   │ -2h              -1h                  now     ││  ■ eth usd  +14.0 !         │
└───────────────────────────────────────────────────┘└─────────────────────────────┘
```

The sketch shows the shape only. The braille lines are not real data.

- Unit: bps, signed. Labels: `+2.1`, `-3.4`. The `all` row shows the range of the last values across venues.
- **The scale always includes the on-peg band** (zero plus or minus `peg_range`), then the data, then `min_span`. The picture then answers the question "is each venue inside the band?". This differs from the rule of section 2.9 (a far limit is left out), because here the band is the point. The band is near by definition (10 bps), so it cannot squash the lines far (Q-D3).
- The band is two dotted lines (`Marker::Dot`, `SECONDARY`) at plus and minus `peg_range`. A third, dashed line marks zero (the peg). The outlier band (plus or minus 50 bps) shows only when it is on the scale.
- The legend gets one row under `all`: `band ±10 bps`. It is text, so it also works with colour off.
- Each venue's glyph comes from `PegBands::classify(mid)`: `OnPeg` is `✓`, `Slight` is `!`, `Off` is `✗`, no mid is `-`. These glyphs keep their health colors as today. Only the lines use the venue colors.
- A peg target of 0 or less gives no deviation (the helper returns `None`). The existing filter then drops the chart. See section 3.5 for the absence note.

### Isolated view (Space)

Today the isolated view is bars with one threshold line. A bar chart cannot draw two band lines. A bar also hides the sign of the deviation, because bars are drawn from the bottom of the scale. For this chart, the isolated view stays a braille line with the band (Q-D4). The selected venue is the only line, and the other venues are not drawn. This is a small exception in `draw_chart`: one branch on a chart flag `signed_band`. The #110 finding (bars for noisy series) is about price-like series. A deviation in bps changes by a few units, and the band is what the reader needs.

### Colors

- Lines use `series_style(i)`: the venue colors, with `i` the place in the venue list. No health color. The test of `theme.rs` already pins this.
- The band lines use `SECONDARY`. They are dots, so they differ from the lines by glyph and not only by color.
- The status glyph uses `OK`, `WARN`, `ERR` as today. It is a glyph, not a line.

### Colour off

- All cells have the default color (`strip_color`). The lines are then told apart by position only. This is already true for every venue chart.
- What remains readable: the value box in the top border (`weex +2.1  binance -3.4`), the legend rows (`> ■ weex +2.1 ✓`), the `band ±10 bps` row, and the dotted band lines (a dot glyph, not a color).
- The unit and sign are in the text, so the chart does not depend on color.

### A window too small

- The deviation chart is one chart of the set. `shortfall` and `enlarge_hint` apply with no change (40 columns plus the legend width, 5 rows).
- At the lowest pane the plot has 2 rows under the x-axis labels. The band lines would sit on the top and the bottom row and would hide the data. **Rule:** when the plot is under 4 rows, `draw_chart` draws no band lines. The legend row `band ±10 bps` still names the band. Test at 80x24 and at the minimum size.
- The legend is wider by the `band` row only in height, not in width.

### A DEX venue

- A DEX venue (for example `eth`) has a mid from the pool USD price. It gets a line like any venue.
- The quote differs. A CEX pair `USDN/USDT` has a mid in USDT. A DEX pool has a mid in USD. If USDT is itself off the peg, the two lines differ for that reason. The legend must say this. A DEX venue name gets the tag `usd` in the legend and in the value box (`eth usd +14.0`). `DexVenue::parse(name).is_some()` decides it (Q-D6).
- There is no trade tape. This does not matter for this chart.
- A DEX pool price is slow (the data comes from DexScreener). Its line can be flat for several ticks. A flat line is not an error.
- A failed venue gives gaps. The existing code splits a line at a gap.

## 3.3 Tests (each says why)

Unit tests (`venue_charts.rs`, `grid.rs`, `repeat_view.rs`, `scope-core`):

1. **Helper equals the comparison.** `mid_peg_deviation_bps` equals `ComparisonRow.peg_deviation_bps` for the same summary, to the last bit. *Why:* the chart and `comparison` must show one number. This fails if the chart converts by its own constant.
2. **Unit.** A mid of 1.0012 on a peg of 1.0 gives `+12.0` bps, not `0.12` (percent) or `0.0012`. *Why:* the repeat view starts from percent values. A missing conversion shows a 100 times smaller line.
3. **Grid series.** After three ticks with known mids, `GridState.deviation` holds the right values for each row label. A failed venue gives `None`. *Why:* the series must line up with rows by index. A shifted index puts one venue's line on another venue.
4. **Band from the summary.** A target with `--peg-range 0.002` in its `health` overrides gives a band of 20 bps. *Why:* the band must follow the thresholds the verdicts use. A hard-coded 10 bps fails this.
5. **Scale includes the band.** Data inside plus or minus 2 bps still gives a scale of at least plus or minus 10 bps with two band lines. Data at 80 bps widens the scale and keeps the band lines. *Why:* the chart exists to show "inside or outside".
6. **Position.** The grid has the chart first. The repeat view has it in the old slot, and the chart count is unchanged. *Why:* it pins Q-D2 and the paging.
7. **No health color.** Each line style of the chart is a `series_style`. The band style is `SECONDARY`. *Why:* the owner's rule, and `theme.rs` pins it for one chart only.
8. **Colour off.** After `strip_color`, every cell has the default color, and the value box, the legend and `band ±10 bps` still hold the values. *Why:* section 3.2.
9. **Too small.** At the minimum pane, no band line is drawn and the legend names the band. Below it, the enlarge hint shows. *Why:* the band lines must not hide the data.
10. **Isolated.** Space shows one braille line with the band, and bars do not show. *Why:* it pins Q-D4.
11. **DEX tag.** A target on `eth` and `binance` shows `eth usd`. *Why:* the quote differs.
12. **Absence.** A peg of 0 gives no chart and the note of section 3.4. All venues without a mid give the same note. *Why:* a silent missing chart cannot be diagnosed.
13. **A rule `peg_deviation_pct` in the grid.** Both charts show. *Why:* it pins Q-D5.
14. **Repeat view test update.** `test_a_repeat_on_two_venues_draws_the_venue_charts_of_the_watch_grid` expects the legend ` venues: Mid price `. It stays true, because the old slot is after Mid. Add a check for the deviation chart in this test.
14a. **Watch CSV row (Q-D7 b).** A tick of a target on 2 or more venues writes one `peg_deviation_bps` row for each market row, with source `chart`, after the cell rows. The value equals the chart value for that tick. A target on one venue writes none. A row with no mid has an empty value, status `n/a` and the reason in `detail`. *Why:* a grid chart must be rebuildable from the record, and a gap in the chart must differ from a missing row.
14b. **The CSV contract.** The tests that pin the cell rows keep their count: the cell rows are the same, and the chart rows are counted apart. The `--format csv` output on stdout is still the bytes of the file. *Why:* the new row must not change what a reader of the cell rows sees.
14c. **Same rule for glyph and CSV.** The CSV status of a row equals the legend glyph of the same venue (`pass` is `✓`, `warn` is `!`, `fail` is `✗`, `n/a` is `-`). *Why:* both call one function, and the test fails if one of them gets its own rule.

TUI harness scenarios (`tests/tui/scenarios`, offline tier):

15. `watch-deviation-chart.yaml`: open a two-venue watchlist (a recorded fixture for each of two venues), Enter on the row, expect `Peg deviation (bps)` and `band ±10 bps`. Resize to 79x24 and expect the chart still present. Resize to a very small size and expect `Window too small`. *Why:* the unit tests use a test backend. The scenario runs the real binary.
16. `repeat-deviation-chart.yaml`: `market summary USDC --venue binance,weex --every 2s --duration 8s`, expect the chart title and the legend rows. *Why:* the repeat view has its own path (`venue_charts_of`).
17. The same scenario with `env: { NO_COLOR: "1" }`: expect the same text. *Why:* the harness checks text, so it proves the text half of colour off. The color half is test 8.

The harness needs a recorded second venue. The existing fixtures hold `api.binance.com` only (`tests/tui/fixtures/*`). The work includes one new fixture folder for the second venue. If a recording is not possible offline, the scenario moves to the live tier and the unit tests carry the proof. The plan must say which.

Built: the recording was possible. The fixtures `watch-deviation-chart`, `repeat-deviation-chart` and `repeat-deviation-chart-nocolor` hold `api.binance.com` and `api-spot.weex.com`, recorded from the live APIs with `SCOPE_HTTP_MODE=record`. All three scenarios run in the offline tier. In the repeat scenario, the chart keys are used after Esc stops the run, because the keys apply to the finished view.

## 3.4 RCA instrumentation (rule 13)

The chart is a view. It makes no decision. The failure it can show is a wrong or a missing line. The record for that failure:

1. **Source numbers.** The repeat view's CSV already holds `peg_deviation_pct` for each venue and run (`market_venues.rs` header). Test 1 proves that the chart equals the `comparison` rows. A wrong line can be reproduced from the CSV or from `market summary --format json`.
2. **The watch CSV row (Q-D7 b).** The watch CSV had no mid and no deviation. It has one row for each cell, so a grid chart could not be rebuilt from it. The owner chose to add a row. Each tick, for each market row of a target on 2 or more venues, the CSV gets one row after the cell rows: `target` is the row label (`USDN@weex`), `metric` is `peg_deviation_bps`, `value` is the deviation in bps, `source` is `chart`, and `op` and `threshold` are empty. `status` is the legend glyph in words (`pass`, `warn`, `fail`, or `n/a` with no value). `detail` holds the verdict and the band (`on the peg (band ±10.0 bps)`) or the reason there is no value. The row is written when the value is missing, so a gap in the chart is not a missing row. A row with source `chart` is a reading, not a check: it is never a breach, and the breach count does not include it. The value comes from `MarketSummary::mid_peg_deviation_bps`, the call that the chart uses.
3. **Absence is shown on screen, with a reason.** The legend (and, with no legend, the footer) says why the chart is missing: `peg deviation: peg target is not positive`, or `peg deviation: no venue has a mid price yet`. The reason is a typed value (`DeviationGap::NoPeg`, `DeviationGap::NoPrice`), so a test can assert it.
4. **No new log line.** Nothing here runs in the background.

## 3.5 Left out, with reasons

| Item | Why |
|---|---|
| Deviation from the cross-venue median (Q8 of the 2026-11-04 design) | The owner asked for deviation in bps, and `comparison` holds it against the peg. A median line shows how far venues differ from each other. A peg line shows how far each is from the peg. When all venues drift together, the median line stays flat and hides the drift. See Q-D1. |
| A deviation chart for a target on one venue | The venue charts exist only for 2 or more venues (design 2026-11-04, section 2.9). One venue keeps its detail pane. Showing one line is a different feature. |
| A deviation cell or column in the watch CSV | Q-D7 chose a row, not a cell or a column. The CSV keeps its 10 columns. |
| The deviation in the `--format json` output of `scope watch` | Not asked. The JSON cells equal the CSV cell rows. The chart rows are in the CSV only. Add it if a reader of the JSON needs it. |
| A new `MetricId` for bps | The percent metric exists. One factor converts it. A second metric would be a second definition of the same value. |
| The web monitor | `scope-web/src/monitor.rs` does not draw the venue charts. |
| A shaded band | A terminal chart has no fill in braille. Two dotted lines and a legend row are enough. |

## 3.6 Open questions, item 1

**Q-D1. What is the reference of the deviation?**
- a. The peg only. Matches the request and `comparison.peg_deviation_bps`. **(recommended)**
- b. The cross-venue median only (the Q8 wording).
- c. Both, as a toggle key.

Reason for a: the owner's request says "peg deviation in bps". The median hides a de-peg that all venues share. Add c later if the owner needs to see the venue spread alone. The `all` row of the legend already gives the spread between venues.

**Q-D2. Where is the chart in the set?**
- a. First in both views.
- b. First in the grid. In the repeat view, in the place of the existing percent chart. **(recommended)**
- c. Last in both.

Reason for b: each view keeps its own first chart (the repeat view starts with the price). The repeat view's chart count and paging do not change. c puts the chart on page 2 at 80x24 in the grid.

**Q-D3. Does the scale always include the on-peg band?**
- a. Yes. **(recommended)**
- b. No. Follow the rule of section 2.9: a limit shows only when it is on the scale.

Reason for a: the band is 10 bps by default. It cannot squash the lines far. The picture then shows "in or out" at a glance. With b, a tight cluster of venues hides the band.

**Q-D4. How does the isolated view draw this chart?**
- a. Bars with a zero line (today's component, no band).
- b. A braille line with the band. **(recommended)**
- c. Extend `Sparkline` to draw two threshold lines.

Reason for b: a bar chart cannot show a sign or two band lines. c changes the shared component for all charts, for one chart's need.

**Q-D5. Grid: when the user has a rule `peg_deviation_pct`, show both charts?**
- a. Show both. **(recommended)**
- b. Hide the bps chart.

Reason for a: the rule chart is the user's own, with its own limit. The cost is one more chart. b hides a chart the user asked for or hides the new one.

**Q-D6. Tag a DEX venue in the legend?**
- a. Yes: `eth usd`. **(recommended)**
- b. No.

Reason for a: the DEX mid is in USD and a CEX mid is in the quote. The reader must know why two lines differ.

**Q-D7. Add a `peg_deviation_bps` row to the watch CSV for each venue row and tick?**
- a. No. (the first recommendation)
- b. Yes, for targets on 2 or more venues, with source `chart`. **(decided by the owner on 2026-11-11)**

Decision: b. The owner chose the complete trail. The first recommendation was a, because the CSV contract is one row for each cell and a new row changes the row counts for every reader. The cost of b is that visible CSV change. The CHANGELOG says it. The rows keep the 10 columns and sit after the cell rows of the tick, so a reader that selects by `source` or by `metric` sees no change.

---

# 4. Item 2: the venue editor

## 4.1 What the code does today

| Part | Today |
|---|---|
| Venue list on a target | `Target.venues: Vec<String>` (`scope-core/src/watch/mod.rs`). `Target::rows()` gives the rows: no venue is the id row; one venue is one row with all cells; 2 or more are one market row for each venue, plus an id row when token or address rules need one. `row_label(venue)` is `USDN@weex`. |
| Change venues now | Delete the target (`d`) and add it again (`a`) in the wizard (`wizard.rs`, `targets_key`). The wizard is closed when the run starts (`Action::StartWatch` sets `wizard = None`, `mod.rs`). **The grid has no edit key.** The grid's keys: `↑` `↓` Enter Esc, and in the detail pane `b` `t` `h` `r` `x`, and the venue keys `←` `→` Space Tab PgUp PgDn. |
| Venue parsing, wizard | `WizardState::add_target` (`wizard.rs`). The loop over the extra words checks: the word is a known venue (case-insensitive, against `self.venues`), not listed twice, and for a pair there is at least one. The messages are `venue `x` is listed twice`, `unknown venue `x`; the venues are …`, `a pair needs a venue`. The parser is inline in `add_target`. It is not a function. |
| Venue parsing, file | `watch/file.rs` `parse`, lines 208 to 234, has the same two checks (duplicate, unknown) with line numbers. Lines 283 to 295 have "market metrics need a venue" and "a pair needs a venue". **So two copies exist now.** |
| Known venues | `watch::venue_ids()` (`VenueRegistry`). The TUI keeps them in `vocab.venues`, and the wizard receives them in `WizardState::new`. |
| The run | `begin_watch` → `run_watch` → `prepare(&list)` → `Ready { list, contracts }` → `watch::run(&ready, …)`. `run_ticks` borrows `ready` for the whole run. `tick(ready, …)` reads `ready.list.targets` each tick. The list cannot change in a run. The grid receives `GridMsg::Ready(ready)` once. |
| Memory | `scope::watch::History` (`watch/history.rs`) is a map from market label (`USDN@weex`) to `Memory { last_trade_ms, volume_24h }`. `History::read` creates the entry on first use. **Nothing removes an entry.** It gives `largest_trade` and `volume_change_pct`. First tick: `largest_trade` is over one interval back (`window_ms`) and `volume_change_pct` waits for a second reading. |
| Grid memory | `GridState.history` (120 values for each (row label, cell label)), `cleared`, `selected` (an index into the sorted rows), `venues: Selection`. |
| CSV | `run()` calls `start_csv(path, false)` at the start. That **truncates** the file and writes the header. `run_ticks` then appends `csv_rows(at, snapshot)` each tick: 10 columns, `target` is the row label. `start_csv(path, true)` appends (the headless run uses it). |
| Report | `write_report(path, w, &last)` at the end, from `ready.list`. |
| Save | `scope::watch::file::save(path, &list)` writes `to_text` (a full file, generated; comments are not kept). The wizard saves on Ctrl-S and asks "Leave without saving?" on Esc when dirty. `settings_reloaded` sets `GridState.stale` when `setup` saved new settings. The run keeps its old ones. The grid has no knowledge of the file of its list. `watch open <name or path>` loads the file in `exec.rs`, and `WatchCommand::Open(list)` carries only the list. |
| Journal | `tui/journal.rs` writes `tui-events.jsonl`: one record for each submitted line, and one for a signal. A grid key is not a line, so it leaves no record. |

## 4.2 Proposal

### The key and the input flow

- **Key: `v`** (venues). It works in the row view and in the detail pane. It is free: the detail pane uses `b t h r x`, and the venue charts use `← → Space Tab PgUp PgDn`. A Ctrl key is not needed. (Q-E2)
- **Targets that accept it.** The selected row's target must be a pair, or a token with a market metric (`Target::needs_market`). An address target has no venues. A token with no market metric would read nothing from a venue. For both, the grid says why in the footer and does not open the input.
- **The run must be running.** After the run ends there is no loop to apply the change. The footer says: `the run ended: change the venues in the wizard (monitor) and start again`.
- **The input line.** One line in the footer, prefilled with the current venues, space separated:

```
 USDN: venues  weex binance okx█
 venues: binance, okx, weex, mexc · Enter apply · Esc cancel
```

  Type to edit. Backspace deletes. Enter checks and applies. Esc cancels. While the line is open, **all keys type into it**. `b`, `t` and the other evidence keys must not run a command. A test pins this.
- **Prefill (Q-E3).** The user edits the current list: remove a name, add a name, or change the order. This changes the venues with one key and one line. The alternative `+okx -weex` syntax is a new grammar and is left out.
- **One parser.** The check of the venue words is **extracted** from `WizardState::add_target` into one function, and `add_target` calls it. The editor calls it with the same known-venue list. The function takes the words and the known venues. It returns the venue list or the same messages as today. It keeps the user's spelling of a name that matches an existing venue ignoring case (see "Memory" below). Nothing new is parsed in the editor. (Q-E7)
- **Rules after parsing** (the same rules as the wizard and the file):
  - at least one venue for a pair, and for any target that `needs_market`;
  - no duplicate, case-insensitive;
  - an unknown venue is an error with the list of known venues.
  An error keeps the line open with the message. The list does not change.
- **The known venues** come from `vocab.venues`. `GridState` receives them at construction, like the wizard does.

### How an edit takes effect: at the next tick (Q-E1)

The edit applies **between ticks, in the same run**. The run is not restarted.

Mechanism, in words:

1. The grid puts a `VenueChange { target_index, venues }` in a shared queue (an `Arc<Mutex<Vec<…>>>`, the same pattern as `ProgressHook`). A second edit of the same target before the tick replaces the first.
2. The run takes the queue at the start of a tick. It changes the venues of its own copy of the list. It keeps `contracts` as they are (the target count does not change, so they still line up).
3. For each removed row label it calls the new `History::forget(label)`.
4. It sends `GridMsg::Ready(ready.clone())`. The grid already replaces its `ready` on this message, and `selected_target()` reads `ready.list` first. No new message type.
5. The tick reads the new rows. The report at the end uses the changed list.

A tick that is already running finishes with the old venues. The grid shows the edit as pending until the next `Ready` arrives.

```
 pegs · run 14 of 60 · next in 12s                                   health ✓✓✓✓✓✗✓✓
 USDN@weex       ✓ ✓ ✓ ✓ ✓ ✓
 USDN@binance    ✓ ✓ ✓ ✓ ✓ ✗
 USDC@binance    ✓ ✓ ✓ ✓ ✓ ✓
 ─────────────────────────────────────────────────────────────────────────────────────
 USDN: weex binance → weex binance okx · applies at the next tick · not saved
 ↑↓ select · Enter detail · v venues · u undo · Ctrl-S save · Ctrl-G shell · Esc stop the run
```

Why not a restart (option a of Q-E1): a restart builds a new `GridState` and a new `History`. It loses the 120-value chart history. It loses the memory of every venue that the owner did not touch. `run()` truncates the CSV unless the caller appends. A restart does what the owner asked the design to avoid: "what happens to each venue's memory and the CSV".

### What happens to each venue's memory

| Case | `History` (`largest_trade`, `volume_change_pct`) | Grid chart history | CSV |
|---|---|---|---|
| Venue kept | Kept. The entry has the same label. The next tick compares with the last tick, as if nothing happened. | Kept. | Rows continue under the same `target` label. |
| Venue kept, case changed (`Weex` for `weex`) | The existing spelling stays, so the label and the memory stay. | Kept. | No change. |
| Venues reordered | Kept (the key is the label). Venue colors change, because color is the place in the list. | Kept. | No change. |
| Venue removed | `History::forget(label)`. The entry is dropped. | Series with that label are dropped, and the deviation series too. | Rows for that label stop. No marker row. |
| Venue added | No entry. First-tick rules apply: `largest_trade` over one interval back; `volume_change_pct` shows n/a until the second reading. | New series. Its line starts at the tick of the change. | Rows start under the new label. |
| Venue removed, then added again | Fresh, as an added venue. Without `forget`, the new `volume_change_pct` would compare with a volume from before the gap and show a false jump. | Fresh. | New rows. |
| 1 venue → 2 or more (token rules present) | The market cells move to `USDN@weex` and the token cells move to a new `USDN` row (`Target::rows`). The memory of the market rows stays. | Series of the token cells for the old label are dropped. | The token cells appear under `USDN`. |
| 2 or more → 1 | The token cells move back to the single row. The id row's series are dropped. | As left. | As left. |

The CSV header is written once, at the start. A change never rewrites or truncates the file. The `target` column is the row label, and it already carries the venue. So the CSV needs no new column. A reader sees the labels start and stop. A venue that fails keeps writing `error` rows, so a stop of rows means removal, not failure.

### Save behavior for a saved watchlist (Q-E4, Q-E5)

- **An edit changes the run only.** It does not write the file. Reason: an edit in a live run is often a trial. The saved list is the owner's definition.
- **`Ctrl-S` saves.** It is the wizard's key. It writes the run's list with `scope::watch::file::save`, to the file the list came from. The grid learns the file from the opener: `watch open <name or path>`, the wizard's Save, and `monitor --yes --save`. A list that has no file saves to `path_in(watchlists_dir, name)`, as the wizard does.
- **Guard.** The grid keeps the text it loaded. Before it writes, it reads the file again. If the text differs from what was loaded, it does not write. The footer says `watchlist.yaml changed on disk since the run started: not saved`. This protects hand edits made in an editor during the run. A list with no file has no guard.
- **What a save loses.** `to_text` regenerates the file, so YAML comments are lost. The wizard has the same effect. This is a known property of `file::save`, not a new one. The footer text says `saved (comments in the file are not kept)` only when the old file held a comment line.
- **Unsaved changes at close.** Esc stops the run, and Esc again closes the grid. The closing summary (`grid::summary`) adds one line when venues changed and were not saved: `venues of USDN changed in the run and not saved`. There is no modal, because Esc must always stop a run fast.
- **The title shows state.** The footer shows `not saved` after an edit and `saved` after Ctrl-S.

### Undo and cancel

- **Cancel:** Esc in the input line. Nothing is queued. Nothing is journaled except the cancel.
- **Undo:** `u` restores the venues from before the last edit, of any target (one level, Q-E6). It is a new edit of the old list. It goes through the same checks and the same queue. If the edit is still pending, `u` removes it and nothing reaches the run.
- **Undo does not restore memory.** A venue that an edit removed and `u` brought back is fresh (the table above). The footer says so: `undone: okx removed again` or `undone: weex is fresh (its history was dropped)`.

### Where it lives in the code (named, not written)

- `grid.rs`: `GridState` gets `edit: Option<…>` (the line), `pending`, `undo`, `known_venues`, `file`. `key()` sends keys to the line first when it is open. `GridAction` gets one variant for a change, so the app can write the journal.
- `wizard.rs`: the venue check moves out of `add_target` into a function that both call. The one-line text editing (type, Backspace, Enter, Esc) is the same in the wizard (`WizardState::key`, the `edit.take()` block) and the grid. It becomes one small helper, so there is no second copy.
- `watch/run.rs`, `watch/mod.rs`: `run_ticks` reads a queue each tick and works on a copy of `Ready` it owns. `run()` uses the final list for the report.
- `scope-core/src/watch/history.rs`: `History::forget`.
- `tui/journal.rs`: one more record kind (see 4.4).

## 4.3 Tests (each says why)

Core and CLI unit tests:

1. **One parser.** The wizard's add line and the editor give the same result and the same message for: an unknown venue, a duplicate with other case, an empty list for a pair. *Why:* two parsers drift. A test that calls both with the same inputs fails when one is changed alone.
2. **Spelling.** Editing `weex` to `Weex` keeps the label `USDN@weex`. *Why:* the label is the key of the memory, the chart history and the CSV. A new spelling would start a "new" venue by mistake.
3. **Refusals.** The editor refuses an empty list for a pair and for a token with a market rule. It refuses an address target and a token with no market metric. It refuses when the run ended. Each refusal gives its message and leaves the list unchanged. *Why:* each of these would otherwise produce a target that cannot read anything (`file.rs` rejects the same cases).
4. **Cancel.** Esc leaves the list, the queue and the file unchanged. *Why:* the user must be able to back out.
5. **Keys captured.** With the line open, `b` types the letter `b`, and no `market summary` command runs. *Why:* the evidence keys are live in the detail pane. This is the most likely bug.
6. **Apply timing.** After Enter, the run's list is unchanged until the next tick starts. Two edits before the tick give one change (the last). *Why:* a change in the middle of a tick would mix two venue sets in one snapshot.
7. **Kept venue.** (`watch/history.rs`) Two ticks, a change that keeps `weex`, a third tick: `weex` has a `volume_change_pct` against tick 2, and its `largest_trade` window continues. *Why:* this is what a restart would lose.
8. **Added venue.** The first tick of `okx` has `largest_trade` over one interval and `volume_change_pct` n/a. *Why:* it pins the first-tick rule for a venue added in the middle of a run.
9. **Removed, then added again.** `volume_change_pct` of the new `okx` is n/a at its first tick, not a change against the old volume. *Why:* without `History::forget`, this shows a false jump. The test fails if `forget` is missing.
10. **CSV continuity.** A run of 4 ticks with a change after tick 2: the file has one header, the rows of the removed label stop, the rows of the added label start at tick 3, and no row is lost. *Why:* a restart truncates the file.
11. **Report.** The report written at the end lists the venues of the final list. *Why:* the run's list changed, and the report reads it.
12. **Grid follows `Ready`.** After the new `Ready` and a tick, the rows, the legend and the venue charts show the new venues. The removed label's chart history is gone. The selected row stays on the same target. *Why:* `selected` is an index into sorted rows, so it can point at another row after the set changes.
13. **1 ↔ 2 venues.** A token with market and holder rules: the token cells move to the id row and back. *Why:* `Target::rows` changes shape.
14. **Undo.** `u` restores the list. A restored venue is fresh. A pending edit is removed with no change in the run. *Why:* it pins what undo promises and what it does not.
15. **Save.** After an edit, the file is unchanged. After Ctrl-S the file is exactly `to_text(list)`. If the file changed on disk after the run started, nothing is written and the footer says so. A list with no file saves to `path_in`. *Why:* a trial edit must not rewrite a saved watchlist, and a save must not erase a hand edit.
16. **Summary line.** Closing a grid with unsaved venue changes adds the line. *Why:* the user must learn it.
17. **Narrow window.** The input line and the legend do not wrap at 79 columns. A long venue list is cut with an ellipsis. *Why:* the repo has this rule for other panes (`test_venues_list_rules_never_wrap_in_a_narrow_pane`).

TUI harness scenarios (offline tier):

18. `watch-venue-editor.yaml`: wizard → Ctrl-R on `USDC` with one venue → wait for run 2 → `v` → expect the prompt with the venue → Esc → expect no pending text → `v`, type a second venue, Enter → expect `applies at the next tick` → expect the new row label after the next run → `u` → expect the row gone → Ctrl-S → `expect_file` on the saved list and on the CSV (`contains` the header and both labels). *Why:* only the real binary shows the keys, the queue and the files together.
19. The same scenario at 79x24, and once with `NO_COLOR`.
20. `watch-venue-editor-refusals.yaml`: `v` on an address row, `v` after the run ended. *Why:* the messages are user-facing.

The scenarios need a recorded second venue (see section 3.3).

## 4.4 RCA instrumentation (rule 13)

The editor changes a running system by hand. After the fact, the owner must be able to say which venues a run read at each tick and why.

1. **Journal record for every outcome.** `tui-events.jsonl` gets one more record kind, like the existing signal record. Fields: time, watchlist name, run number at the time, target, `from`, `to`, the exact line typed (bytes, not a summary), and `outcome`: `applied`, `cancelled`, `refused`, `undone`, `saved`, `save_refused`. A refusal and a refused save carry the reason text. The grid returns a `GridAction` and the app writes the record, since the app owns the journal.
2. **The tick that applied it.** The run reports the run number at which it applied each change. The record for `applied` is written when the run confirms (the `Ready` message), not when the user presses Enter. The two times differ by up to one tick. Both are recorded: `queued_at` and `applied_at_run`.
3. **The CSV carries the effect.** Rows start and stop with the labels (section 4.2). No new column.
4. **Save guard record.** A refused save logs the two hashes (loaded text, text on disk).
5. **No silent skip.** A change the run cannot apply (for example, the run ended between Enter and the tick) is journaled as `refused: run ended` and shown in the footer.

## 4.5 Left out, with reasons

| Item | Why |
|---|---|
| Restart the run to apply a change | It loses chart history, the memory of untouched venues, and it truncates the CSV. See Q-E1. |
| Adding and removing whole targets in the grid | The request is about venues. `a` and `d` stay in the wizard. A target editor needs the metric and rule editing of the wizard (a larger feature). |
| Editing rules, metrics, interval or outputs in the grid | Same reason. Each has its own validation and its own effect on the schedule or the files. |
| `+okx -weex` syntax | A new grammar next to the one the wizard uses. Prefill and edit does the same work with the existing parser. |
| Undo stack | One level covers "I mistyped". A stack needs a visible history to be understood. See Q-E6. |
| A marker row in the CSV for a venue change | The `target` label already shows it. A marker row would change the row format for every reader. The journal holds the exact time and run. |
| A "venue changes" section in the report | The report is the last tick plus the final list. The journal is the trail. Add it if the owner wants one file for a run. |
| Headless `scope watch` and the web monitor | They have no interactive surface. The venue list is edited in the file. |
| Merging the copies of the venue checks in `file.rs` | See Q-E7. `file.rs` messages carry line numbers. Changing them is a separate risk. |
| Per-row (not per-target) editing | The venue list belongs to the target. The rows are derived from it. |

## 4.6 Risks and what the design does about them

| Risk | Handling |
|---|---|
| A change applies in the middle of a tick and mixes two venue sets | Changes apply only at the start of a tick (test 6). |
| `History` grows with removed venues, or reuses stale volume | `History::forget` (test 9). |
| `selected` points at another row after the rows change | The grid re-selects the same target (test 12). |
| A hand edit of the file is overwritten by Ctrl-S | The guard (test 15). |
| An evidence key runs a command while the user types | Keys captured (test 5). |
| `Ready` and the list drift apart | One list in the run; the grid takes it from `GridMsg::Ready` (test 12). |

## 4.7 Open questions, item 2

**Q-E1. How does an edit take effect?**
- a. Restart the run with the new list.
- b. Apply between ticks in the same run. **(recommended)**

Reason for b: a restart loses the chart history and the memory of every venue, and it truncates the CSV unless the caller changes to append. b keeps all of them. The cost is a change in `run_ticks` (a queue and a list the run owns) and `History::forget`.

**Q-E2. Which key opens the editor?**
- a. `v`. **(recommended)**
- b. `e`.

Reason for a: `v` is for venues, and it is free in the row view and the detail pane. `e` suggests "edit the target", but only the venues can change.

**Q-E3. What does the input line start with?**
- a. The current venues. The user edits the list. **(recommended)**
- b. An empty line. The user types the full new list.
- c. A delta syntax: `+okx -weex`.

Reason for a: it is one key and one line for add, remove and reorder, with the wizard's parser. b loses the current list from view. c needs a new grammar.

**Q-E4. Does an edit write the saved watchlist?**
- a. No. `Ctrl-S` saves. **(recommended)**
- b. Yes, at once.
- c. Ask after each edit.

Reason for a: an edit in a live run is often a trial. The wizard also saves only on Ctrl-S. b changes the owner's file without a decision. c adds a prompt for every edit.

**Q-E5. What does `Ctrl-S` write?**
- a. The run's whole list, if the file did not change on disk since the run started. **(recommended)**
- b. Only the venues of the targets, patched into a fresh read of the file.

Reason for a: it is the wizard's behavior and needs no matching of targets. The guard stops the one real danger, a lost hand edit. b keeps other file changes, but it must match targets by position or name, and it can fail when the file has changed shape. If the owner wants b, the guard becomes a merge and the test list grows.

**Q-E6. How deep is undo?**
- a. One level (`u`). **(recommended)**
- b. A stack.
- c. No undo. The user edits again.

Reason for a: it covers a mistake. The memory rule (a restored venue is fresh) is simple to say for one step. b needs a visible history, and memory makes each step harder to explain. With c, the previous list is gone from view.

**Q-E7. Where does the shared venue check live?**
- a. In `wizard.rs`, as a function that `add_target` and the editor both call. `file.rs` keeps its copy. **(recommended)**
- b. In `scope-core`, and `file.rs` calls it too.

Reason for a: the request is to reuse the wizard's parser. It removes the new copy and leaves `file.rs` messages (with line numbers) as they are. Two copies remain (the file and the wizard), as today. b is the clean end state (rule 3: DRY). It changes the file loader and its error tests. I surface the existing duplicate for the owner to decide, and I do not decide it in silence.

---

# 5. Delivery

**Q-P. How many PRs?**
- a. Two PRs to `develop`: the chart, then the editor. **(recommended)**
- b. One PR.

Reason for a: the chart is read-only and has no state. The editor changes the run loop, `History` and the files. A defect in the editor then cannot hold back the chart. Each has its own tests and review. Both target release 0.8.8. Do not cut 0.8.8 before both are merged, or the owner moves one to 0.8.9.

Order of work, chart: (1) the helper in `scope-core`; (2) the `Series.peg` field; (3) the chart in the repeat view; (4) the grid series and chart; (5) tests and scenarios.

Order of work, editor: (1) `History::forget`; (2) the shared venue check and line helper; (3) the run queue; (4) the grid editor; (5) save, undo and the journal; (6) tests and scenarios.

## 6. Summary of recommendations

| Question | Recommended |
|---|---|
| Q-D1 reference | a. Peg only |
| Q-D2 position | b. First in the grid; old slot in the repeat view |
| Q-D3 scale | a. Always include the band |
| Q-D4 isolated view | b. Braille line with the band |
| Q-D5 user rule chart | a. Show both |
| Q-D6 DEX tag | a. Tag `usd` |
| Q-D7 watch CSV row | b. Yes (the owner's choice; the first recommendation was a) |
| Q-E1 apply | b. Between ticks |
| Q-E2 key | a. `v` |
| Q-E3 input | a. Prefilled list |
| Q-E4 write the file | a. No; Ctrl-S |
| Q-E5 what Ctrl-S writes | a. Whole list with a guard |
| Q-E6 undo | a. One level |
| Q-E7 shared check | a. In `wizard.rs` |
| Q-P PRs | a. Two |
