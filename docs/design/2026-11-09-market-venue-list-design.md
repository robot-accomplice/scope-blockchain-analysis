# Market Commands on Several Venues: Design

- **Status:** decisions made by Jon on 2026-11-09. Implemented in one PR to `develop`.
- **Scope:** `scope market summary`, `market ohlc` and `market trades` take a list of venues. Follows the watchlist venue list and the venue charts (`2026-11-04-watch-venue-list-design.md`, PR #182 and #183).
- **Out of scope:** `token-health --venue`, `insights` and `monitor`. They are not in the request. `monitor --venue` already takes a list.

## 1. Goal

To compare one pair on several venues today, the user runs the command once for each venue and compares the screens by eye. One run on a list of venues shows the venues side by side.

## 2. Decisions

1. **`--venue` takes a list.** Repeat the flag, or give a comma list: `--venue weex,binance`. This is the clap attribute of `scope monitor --venue` (`value_delimiter = ','` on a `Vec<String>`). There is no second parser. A venue named twice (any case) is an error, as in a watchlist file. One venue, and the default, give the output of before in every format. The code of the one-venue path is unchanged; the tests pin its shapes.
2. **`market summary` on 2 or more venues:** one full summary for each venue, in the order given, each headed by its venue. Then one comparison table with one row for each venue: mid, spread, bid depth, ask depth, 24h volume, health verdict, and the bot-pattern signs (the count and the names). A venue that fails does not stop the others. Its section says why, and its row shows `ERROR` and the reason. The venues are read at the same time.
3. **`market summary --every` on 2 or more venues:** the repeat view reuses the venue charts of the watch grid (`tui/venue_charts.rs`): braille lines for all venues, bars for one isolated venue, and the `all` row of the legend with the range. The keys are the keys of the grid. The code of the key handling moved from `GridState` to `Selection::apply`, so the grid and the repeat view call one function.
4. **`market ohlc` and `market trades` on 2 or more venues:** one labelled block for each venue, in the order given. The shape of the machine formats is in section 4.
5. **Exit code:** section 5.

## 3. Output shapes

### 3.1 `market summary --venue weex,binance`

```text
┌─ USDC/USDT (weex) ─────────────────────
│ ... the full summary of one venue ...
└────────────────────────────────────────

┌─ USDCUSDT (nowhere) ───────────────────
│  ⚠ No summary: Unknown venue "nowhere"
└────────────────────────────────────────

┌─ Comparison — USDC/USDT ───────────────
│  Venue    Mid  Spread  Bid depth  Ask depth  Volume 24h  Health   Bot signs or error
│  weex  1.0000  0.020%     42.00K     45.00K     125.00K  HEALTHY  2: Repeated size, Metronome
│  nowhere   -        -          -          -           -  ERROR    Unknown venue "nowhere"
└────────────────────────────────────────
```

- A failed venue has a section, so the order of sections is the order given.
- The last column holds the bot-pattern signs, or the error for a failed venue. Signs read `none` when the tape showed nothing, `none (no trade tape)` when there was no tape, and `-` when the check did not run.
- The last column is cut at 60 columns. The section of the venue has the whole reason.
- The health verdict is the verdict of the single summary (`HEALTHY` or `UNHEALTHY`).
- The pair in the section of a failed venue is the pair as the user typed it. The venue gave no symbol.

### 3.2 `market summary --every`, in the TUI

```text
┌─ Comparison — USDC/USDT ─────  (the table of the last run)
│  ...
┌ Mid price ── weex 1.0000 binance 1.0002 ┐┌ venues: Mid price ──┐
│ 1.0003┤  braille line for each venue    ││  all  1.0000–1.0002 │
│ ...                                     ││> ■ weex    1.0000 ✓ │
└─────────────────────────────────────────┘│  ■ binance 1.0002 ✓ │
```

- One chart for each charted key value (the metrics of the single repeat view, less best bid, best ask, absolute spread and health).
- The glyph in the legend is the health of the venue in the last run: `✓` healthy, `✗` unhealthy, `?` no reading.
- A failed venue leaves a gap in its line. The repeat goes on. When every venue fails in one run, the repeat ends with an error, as one failing venue does today.
- Keys: `←` `→` venue, Space isolate (bars), Tab chart, PgUp and PgDn page. The keys work only when the input line is empty, so typing is not lost. They do not leave a finished view. Any other key leaves it, as before.
- A window too small for the charts shows the table and the footer hint `enlarge it N columns wider to see the venue charts`.
- In the CLI (no TUI), each run prints the sections and the table, as for one venue.

### 3.3 `market ohlc` and `market trades`

Text: one block for each venue, a blank line between. A failed venue has a block with `No data: <reason>`.

```text
┌─ OHLC — USDCUSDT (weex) ─────   (chart and summary, as for one venue)
└─────
┌─ OHLC — USDCUSDT (nowhere) ──
│  ⚠ No data: Unknown venue "nowhere"
└─────
```

## 4. Machine formats

| Command | One venue (unchanged) | Two or more venues |
|---|---|---|
| `summary --format json` | one object | list of objects |
| `ohlc --format json` | list of candles | list of `{venue, candles}` |
| `trades --format json` | list of trades | list of `{venue, trades}` |
| `summary --csv` (with `--every`) | one row for each run | one row for each venue and run, `venue` column after `run` |

```json
[
  {"venue": "weex", "run": null, "pair": "USDC/USDT", "healthy": true, "...": "..."},
  {"venue": "nowhere", "run": null, "error": "Unknown venue \"nowhere\""}
]
```

Why this shape:

1. **A list of venue items, in the order given.** The order is stable, so a script reads item 0 as the first venue. A script that wants a map builds one from `venue`.
2. **One item for each venue, also for a failed venue.** The item holds `error`. A script then sees which venue failed and why. A flat list of candles cannot hold an error, and it cannot say which venue a candle came from without a `venue` field on each candle (a larger output).
3. **A `summary` item is the object of one venue.** It already has `venue` and `run`. A script that reads one venue reads each item with the same code.
4. **The key (`candles`, `trades`) names the payload.** `ohlc` and `trades` have different row types, so each item names its own.
5. **The one-venue shape does not change.** Scripts that read one venue keep working. A script must choose the list shape on purpose, by naming 2 or more venues.
6. **The CSV adds a `venue` column and a row for each venue.** Rows for one run stay together, in the order of the venues. A failed venue has `-` in every value column, as a missing value does today. The column goes after `run` so the key of a row is `timestamp,run,venue`. The one-venue CSV has no `venue` column, so its header does not change.

Left out: a `comparison` array in the summary JSON. The list items hold every value of the table, so a script can build it. It would repeat data and add a second shape to keep. Add it if a script needs it.

## 5. Exit codes

Today, one-venue `market summary` returns 0 for a healthy or an unhealthy market (the verdict is in the output, not in the code), and 1 when the command fails (an error, such as an unknown venue or a network error). `ohlc` and `trades` return 0, or 1 on an error.

With 2 or more venues:

| Case | Exit code |
|---|---|
| Every venue answered (healthy or not) | 0 |
| Any venue failed | 1, after all output is printed |
| Every venue failed | 1 |

- This is the worst of the venues, with the order `answered` < `failed`. The unhealthy verdict does not count, as with one venue.
- The error line names the venues that failed: `1 of 3 venues failed: nowhere`. The reasons are in the sections.
- With `--every`, the code follows the last run. The repeat ends early with an error only when every venue fails in a run.
- `--report` and `--csv` are written before the code is returned.
- The `--help` text of the three commands says this.

Why 1 for a partial failure: a script that runs a comparison needs to know that a venue is missing from it. The output still holds the venues that answered.

## 6. Order of work in a run

1. `--venue` is checked first: an empty name or a duplicate fails before any request.
2. For each venue, one after the other: the venue is resolved, then the token check and the coin list. These steps can ask the user which token is meant (a symbol on several networks), so they do not run at the same time. A failure here becomes the error of that venue.
3. For `market summary`, the order books, tickers and trade tapes of all venues are read at the same time (`futures::join_all`). The reports keep the order given.
4. `ohlc` and `trades` read the venues one after the other. They are not slow, and the token check can ask the user.

## 7. What does not change

- The one-venue code path and output in every format, including the TUI compact view of a one-shot `market summary` and the repeat view of `--every`.
- With 2 or more venues in the TUI, a one-shot `market summary` shows the text report (the compact market view reads one venue).
- `scope monitor`, `token-health` and the watchlist. The watch grid keeps its behaviour; only the key code moved.
- The thresholds: one set (flags, config `market.health`, defaults) applies to every venue.
- `--token` applies to every venue. A DEX venue that needs another contract has the contract resolved on its own chain.

## 8. Left out, with reasons

| Item | Why |
|---|---|
| A comparison array in the summary JSON | See section 4. The items hold the data. |
| A comparison table in `ohlc` and `trades` | The request asks for blocks. A candle table across venues needs a time grid and a rule for gaps. |
| Concurrent `ohlc` and `trades` | The token check can ask the user, and the calls are short. |
| `--venue` on `token-health` and `insights` | Not in the request. |
| A cross-venue deviation chart | The watch design left it for later (Q8), until the overlay is seen on real data. |
| The per-venue `depth cycle` note in the repeat view | The note reads one venue's depth series. Each venue still gets its depth-cycle finding in its own section. |

## 9. Tests

Every test uses mocked venues (`mockito` with the real venue mappings, or the mock DEX factory). None uses the network. The test build puts the scope home in a sandbox; the test run also sets `SCOPE_BCA_HOME` to a fresh folder.

1. **Flag:** `--venue a --venue b` and `--venue a,b` give the same list on the three commands. The default is the default venue. A duplicate and an empty name are errors.
2. **One venue is unchanged:** one-venue `summary` has one section, no table, and a JSON object. One-venue `ohlc` JSON is a bare list. One-venue `trades` text is the exact table of before. A failed one-venue `summary` gives its own error, not `1 of 1 venues failed`.
3. **Summary on several venues:** sections in the order given, then the table with its columns; the verdict and the bot signs (count and names) for each venue; the report file; the JSON list.
4. **A failing venue does not hide the others:** an unknown venue and a venue that answers 400 give an error section and an error row, and the other venues print. The exit code is 1 and names the venue.
5. **Concurrency:** two venues that each take 400 ms are read in under 780 ms.
6. **An unhealthy venue does not fail the command.**
7. **Repeat on several venues:** a venue that fails does not end the repeat; the CSV has a row for each venue and run and a `venue` column; the series holds the readings of each venue; every venue failing ends the repeat with an error.
8. **Repeat view in the TUI (120×36 and 80×24):** the table and the charts show; the keys `←` `→`, Space and Tab work as in the grid; a finished view stays; another key leaves it and types; typing with text on the input line is not taken by the chart keys; a small window shows the hint; `save x.csv` has the venue column.
9. **`ohlc` and `trades`:** blocks in order, a failed venue's block, the JSON list items.
10. **Help:** each command's help names `--venue weex,binance` and the exit code.
11. **Watch grid:** the existing venue-chart tests pass unchanged. They cover the moved key code.
