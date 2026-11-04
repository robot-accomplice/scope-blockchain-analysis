# Watchlist Venue List: Design

- **Status:** approved by Jon on 2026-11-04, with the proposed answers to Q1–Q9 (section 4). Section 2.9 (venue charts) was added on 2026-11-04 from Jon's request.
- **Delivery (Q9):** two PRs to `develop`, both in release 0.8.6. PR 1: the venue list (sections 2.1–2.8). PR 2: the venue charts (section 2.9). Do not cut 0.8.6 before PR 2 is merged.
- **Date:** 2026-11-04
- **Scope:** a watchlist target names a list of venues, not one venue. One target (for example USDN) is watched on several venues in one run. Follows #122 (PR #181).
- **Out of scope:** the old monitor dashboard (deprecated, single venue) and the web monitor (`scope-web/src/monitor.rs`). They do not read watchlists.

## 1. Problem

`scope::watch::Target` has `venue: Option<String>`. To watch USDN on weex and on binance, the user must write two targets with the same id, the same preset and the same rules. The two copies can drift apart.

## 2. Design

### 2.1 Model

```rust
pub struct Target {
    // ...
    /// The venues, for market metrics, in the user's order. Empty: none.
    pub venues: Vec<String>,
}
```

- The list has no duplicates (case-insensitive). The parser reports a duplicate as an error.
- `Target::new` gives an empty list.

### 2.2 File format (`watch/file.rs`)

The key stays `venue`. It takes one name or a list:

```yaml
targets:
  - kind: pair
    id: USDN
    venue: [weex, binance]     # new: a list
    preset: market health
  - kind: pair
    id: USDC
    venue: binance             # old: one name, loads as [binance]
```

- `RawTarget.venue` becomes a one-or-many type (`#[serde(untagged)] enum { One(String), Many(Vec<String>) }`).
- `to_text` writes one venue as a scalar and two or more as a list. A single-venue file therefore stays byte-equal after a load and a save, and an older scope binary still reads it.
- An older binary cannot read a file with a venue list. It reports a parse error; it does not lose data.
- `venue: []` is the same as no venue.
- The validation per venue stays: an unknown venue is an error on its line. "Market metrics need a venue" and "a pair needs a venue" now mean "at least one venue".

### 2.3 Rows: one row per venue

A target with N venues gives N grid rows, one per venue. Each row has the market cells (preset checks and market rules) for its venue.

| Target | Rows |
|---|---|
| USDN, `venue: [weex, binance]`, market health | `USDN@weex`, `USDN@binance` |
| USDN, `venue: weex`, market health + `holders >= 10` | `USDN@weex` (all cells, as today) |
| USDN, `venue: [weex, binance]`, market health + `holders >= 10` | `USDN@weex`, `USDN@binance` (market cells), `USDN` (token cells) |
| PEPE, no venue, `holders >= 10` | `PEPE` (as today) |

Why one row per venue, not one row with per-venue cells:

1. Everything downstream already keys by row label: `diff` events, the banner, `History` (#122), the CSV `target` column, the report table and the grid's evidence keys. `USDN@weex` and `USDN@binance` work in all of them with no change in format.
2. Each venue gets its own status. The grid sorts breached rows first, so the venue that breaks is at the top.
3. Per-venue cells need a new grid layout (a column per venue) and a new rule for which venue a breach belongs to.

Token and address cells do not depend on a venue. With two or more venues they go on one more row, labelled with the id alone. They are read once per tick, not once per venue, and they do not repeat in the CSV or in the events. With one venue, the row is the same as today, so existing CSV files and reports keep their labels.

### 2.4 Labels

- `TargetSnapshot` gets `venue: Option<String>`: the venue of a market row, `None` for the id row. The grid and the report read the venue from this field. They do not parse it out of the label.
- `Target::row_label(venue: Option<&str>)` gives `USDN@weex` or `USDN`. The fetcher, `History` and the grid use it.
- `Target::label()` stays for text that names the whole target (wizard messages): `USDN@weex`, `USDN@weex,binance`, or `USDN`.
- The grid finds the target of a selected row by id and venue, not by `label() ==`.

### 2.5 Fetcher (`watch/fetch.rs`)

- `read_target` returns `Vec<TargetReading>`, one per row, in the row order of section 2.3.
- One market read per venue, in list order. A failed read makes only that venue's cells `Error`. The other venues and targets go on.
- The since-last-tick metrics read `History` with the row label, so each venue has its own trade tape and volume.
- `Tick.summaries` stays aligned with `snapshot.targets`: one entry per row, not per target. Only market rows have a summary.
- `Watchlist::fetches_per_tick` counts one market fetch per venue.
- The token contract (`Ready.contracts`) stays per target. All venues of a target use the same contract.

### 2.6 CSV and report (`watch/run.rs`)

- CSV: no new column. The `target` column has the row label, as today. Two venues give two sets of rows.
- Report: one `market summary` section per market row, from the row's venue and summary. Today the report zips targets with summaries; it iterates rows.

### 2.7 Monitor wizard (`tui/wizard.rs`)

- `a` (add target) takes more words: `pair USDN weex binance`. Each word that is a known venue goes in the list. For a token, one word that is not a venue is the chain, as today. A pair with a word that is not a venue is still an error.
- The target line shows `pair USDN @weex,binance  [market health]`.
- To change the venues of a target, delete it (`d`) and add it again. No new key. This keeps the change small; a venue editor can come later if the delete-and-add step is a problem in use.

### 2.8 `scope monitor --venue`

- `--venue` becomes repeatable and takes a comma list: `--venue weex --venue binance` or `--venue weex,binance`. `MonitorArgs.venue` becomes `Vec<String>`.
- `watchlist_from_args` puts all the venues on the one target.
- The old dashboard (`--dashboard`) reads one venue. With two or more it stops with an error: "the dashboard reads one venue; remove --dashboard to watch several". It does not take the first venue in silence.

### 2.9 Venue charts (Jon, 2026-11-04: "take inspiration from bottom")

Today the grid's detail pane draws one bar `Sparkline` per cell, for the selected row only. With N venues, the user must open N rows to compare them. The venue charts put the venues of one target on one chart per metric.

#### Where

- The detail pane of a market row whose target has two or more venues shows the venue charts. A target with one venue keeps today's detail pane.
- The charts read the grid's existing history (`GridState.history`, 120 values per row and cell). It is keyed by row label, so each venue already has its own series. No new fetch or store.
- The repeat view of `market summary --every` is single-venue. It does not change.

#### Layout

```
┌ mid ─────── weex 1.0003 binance 0.9998 ┐┌ spread_pct ≤ 3 ─ weex 0.02 binance 0.04 ┐┌ venues: mid ───────┐
│1.0005┤ ⠒⠤⣀      ⣀⠤⠒⠉⠉⠒⠤⣀              ││3.00┤···································││  all  0.9998–1.0003│
│      │     ⠉⠒⠒⠉           ⠉⠒⠤⣀⣀       ││    │ ⣀⣀⠤⠤⠒⠒⠉⠉⠉⠒⠒⠤⠤⣀⣀⣀⠤⠤⠒⠒⠉⠉⠒⠒⠤⠤⣀⣀    ││> ■ weex    1.0003 ✓│
│0.9995┤                                 ││0.00┤                                   ││  ■ binance 0.9998 ✗│
│       -2h          -1h            now  ││     -2h          -1h            now    ││                    │
└────────────────────────────────────────┘└────────────────────────────────────────┘│                    │
┌ depth_bid_usd ≥ 100000 ─── weex 2.1M binance 9.8M ┐┌ largest_trade < 50000 ─ ... ┐  │                    │
```

The sketch shows the shape only; the plot rows are not real data.

1. **One chart per market cell** (preset checks and market rules), in cell order. Each chart is a pane with a titled border: the cell label, for example `spread_pct ≤ 3`.
2. **One line per venue** on a shared y-axis and a shared time axis. The y-axis fits the range of all venues, with the metric's minimum span (`min_span` of the repeat view), so a flat series does not fill the height. The time axis uses the repeat view's `time_axis` labels (`-2h … now`).
3. **The rule's limit** is a dotted line in `SECONDARY`, as in the bar sparkline.
4. **A value box** in the chart's top border shows each venue's last value in its venue colour (bottom's Memory and Network boxes). When the box does not fit in the border, the chart drops it; the legend still has the values of the selected chart.
5. **Panes in a grid.** The number of chart columns is the body width divided by a minimum chart width (`MIN_CHART_COLS`, 40). Each chart is at least `MIN_CHART_ROWS` tall (5: two border rows and three plot rows).
6. **One venue legend** on the right, for all the charts (bottom's CPU legend). Row one is `all`, with the range of the last values across venues (`0.9998–1.0003`): for a peg, the spread between venues is the number that matters. Then one row per venue: the colour, the name, the last value of the selected chart and the row's status glyph (`✓ ⚠ ✗`) in its health colour. The legend scrolls when the venues do not fit.

#### Drawing: overlaid lines, and the #110 finding

ratatui 0.30 `Chart` with `Dataset`, `Marker::Braille` and `GraphType::Line` draws the overlaid lines. This conflicts with the decision on PR #110: Jon kept the bars because braille "only helps on smooth series and series with gaps; on noisy data it loses vertical resolution and readability". Bars cannot overlay, so overlaid lines need braille or dots.

The proposal keeps both:

1. **All venues:** braille lines, because the point of this chart is to compare lines.
2. **One venue isolated:** that venue's series as today's bar `Sparkline`, at the full plot height, with the threshold row. This is the readable view of one noisy series.

Two more limits of braille overlay:

- ratatui gives a braille cell the colour of the last series drawn in it. Venue prices of a stablecoin are close, so lines touch. The selected venue is drawn last, so it is always on top.
- With colour off (`NO_COLOR`, `TERM=dumb`, `--no-color`), the lines have no colour, and colour must not carry meaning (theme rule 3). The legend and the value box still name each venue with its value, and isolating a venue shows only that venue. So no fact depends on the colour.

#### Venue colours

The theme uses only the 16 named ANSI colours (theme rule 2), and all colours stay in the theme module (rule 1, the contrast guard). The health colours are green, yellow and red; they are not venue colours.

- `theme::VENUE_SERIES`: blue, magenta, cyan, then the default foreground. These four read on dark and on light themes. The bright variants are not used: bright cyan and bright white are weak on a light background.
- A venue's colour is its position in the target's venue list, so it is the same on every chart and in the legend.
- From the fifth venue, the colours repeat, and the line uses `Marker::Dot` instead of braille so that venues one and five still differ.
- The contrast guard gets the four new colours.

#### Keys (detail pane with venue charts)

| Key | Does |
|---|---|
| `←` `→` | Select the previous or next venue in the legend. The first selection is the venue of the row you opened. |
| `Space` | Isolate the selected venue (bars), or show all venues again. |
| `Tab` | Select the next chart; the legend shows that chart's values. |
| `PgUp` `PgDn` | Show the previous or next page of charts, when not all fit. |
| `b` `t` `h` `r` `x` `Esc` | As today. `b` and `t` use the selected venue. |

`DETAIL_LEGEND` gets the new keys when the venue charts show.

#### Small windows

1. Charts that do not fit go on further pages (`PgDn`). The legend's title says `charts 1–4 of 9`.
2. When the body is narrower or lower than one chart plus the legend, the pane shows today's detail pane (one bar sparkline per cell) for the selected row, and the repeat view's too-small hint: `! Window too small: enlarge it 12 columns wider to see the venue charts`. `enlarge_hint` moves from `repeat_view.rs` to a shared place, so both views use one text.

## 3. Tests

Every test uses a sandboxed `SCOPE_BCA_HOME`, as the watch tests do today.

1. File: a single-venue file loads as a list of one and saves byte-equal (old files keep working). A list loads in order. A duplicate venue, an unknown venue in a list and `venue: []` on a pair are errors with their lines.
2. Rows: the four cases of the table in section 2.3, through `read_target` on recorded replies. One venue that fails and one that passes give one `Error` row and one good row.
3. History: two venues of one target keep separate memory (a trade on weex does not change `largest_trade` on binance).
4. CSV and report: two venues give two labelled row sets and two summary sections.
5. Grid: the evidence keys of a `USDN@binance` row run `market summary USDN --venue binance`.
6. Wizard: `pair USDN weex binance` adds one target with two venues; the target line shows both.
7. Monitor: `--venue weex --venue binance` and `--venue weex,binance` give the same watchlist; `--dashboard` with two venues is an error.
8. Venue charts, through the TUI harness at 120×36 and 80×24: one chart per market cell, one line per venue, the legend values and glyphs, and the selected venue on top where lines touch.
9. Keys: `←→` moves the legend selection and `b` uses it; `Space` shows the isolated venue as bars and back; `Tab` changes the legend's values; `PgDn` pages the charts.
10. Small window: below the minimum size, the pane is today's detail pane and has the too-small hint.
11. Colour off: the frame has no colour, and each venue's name and value are still on screen.
12. Contrast guard: the venue colours are in the theme module only, and none is a health colour.

## 4. Open questions

| # | Question | Proposed answer |
|---|---|---|
| Q1 | File key: keep `venue:` (one name or a list), or add a new `venues:` key? | Keep `venue:`. One key, no migration, old files unchanged. |
| Q2 | Grid: one row per venue, or one row with per-venue cells? | One row per venue (section 2.3). |
| Q3 | Token and address cells with two or more venues: on a separate id row, or on the first venue's row? | A separate id row. A `holders` breach is not a weex breach. |
| Q4 | Wizard: delete and add again to change venues, or a new key to edit them? | Delete and add again, for now. |
| Q5 | `--venue`: repeatable, comma list, or both? | Both. |
| Q6 | Overlaid braille lines conflict with the #110 decision to keep bars. Accept braille for the all-venues chart only, with bars when one venue is isolated? Or use small multiples instead: one bar sparkline per venue, stacked, on a shared y-axis (no overlay, no venue colours)? | Braille for all venues, bars for one (section 2.9). |
| Q7 | The legend's `all` row: the range across venues, or the average (as bottom does)? | The range. |
| Q8 | Also chart each venue's deviation from the cross-venue median, as a view toggle? It shows a peg break on one venue more clearly than overlaid prices. | Not now. Add it if the overlay is hard to read on real data. Jon: "maybe later". It is a possible follow-up, not part of PR 1 or PR 2: the overlay must first be seen on real data to know if it is needed. |
| Q9 | Ship the charts in the same PR as the venue list, or in a second PR? | A second PR. The venue list is useful alone, and the charts need their own harness review. |
