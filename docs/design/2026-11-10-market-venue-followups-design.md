# Market Venue Follow-ups: Design

- **Status:** decided on 2026-11-10 (section 9). PR 1 of 2 (`comparison`) is in progress.
- **Scope:** the items that `2026-11-09-market-venue-list-design.md` (section 8) left out:
  1. a `comparison` array in the JSON of `market summary` on 2 or more venues;
  2. `--venue` as a list on `token-health`;
  3. `--venue` as a list on `insights`.
- **Base:** `develop` after PR #188. The 2-or-more-venue shapes of #188 were on `develop` only when this was written. The last release then was v0.8.6, and it did not have them. v0.8.7 (2026-11-10) shipped them.

## 1. Goal

1. A script that compares venues reads one array of numbers. Today it reads the per-venue items and builds the table itself.
2. `token-health` and `insights` show the stablecoin market on more than one venue in one run. Today the user runs the command once for each venue.

## 2. What the code does today

| Item | Today |
|---|---|
| Text comparison table | `comparison_table` in `market_venues.rs`. One `Row` per venue, built by `row()`. Cells are text: `format_price`, `{:.3}%`, `format_large_number`, `HEALTHY`/`UNHEALTHY`/`ERROR`, bot signs. The last cell is cut at 60 columns. |
| The same rows in other places | `comparison_markdown` (the `--report` file) and `VenueRun.comparison` (a text string for the repeat view). All three call `row()`. |
| `summary --format json`, 1 venue | One object (`MarketSummary::to_json` plus `run`, `venue`, `tokens`). |
| `summary --format json`, 2+ venues | A list of those objects, or `{run, venue, error}` for a failed venue. |
| `token-health --venue` | One `String`. `--venue` only has an effect with `--with-market`. A market failure is a warning (`market data unavailable`); the DEX analytics still print; the exit code is 0. |
| `token-health --format` | `table`, `json`, `markdown`, `csv`. `csv` prints the same table as `table`. There is no CSV writer. |
| `token-health --format json` | `{"analytics": ..., "market": <MarketSummary JSON or null>}`. |
| `insights --venue` | None. For a stablecoin token, `insights` picks the default venue (or the first registered one) and adds one `Market (venue pair)` line. A failure there is silent. |
| `insights --format` | None. The output is Markdown. |

The numbers in the text table are already computed. The new JSON array must not format them again, so it does not copy the strings.

## 3. `comparison` in the summary JSON

### 3.1 Row shape

One row for each venue, in the order given. The row holds raw numbers, not text. Units:

| Field | Type | Unit | Missing value |
|---|---|---|---|
| `venue` | string | as given | never missing |
| `status` | string | `"healthy"`, `"unhealthy"` or `"error"` | never missing |
| `mid` | number | quote currency | `null` |
| `spread_pct` | number | percent (`0.020` is 0.020%) | `null` |
| `bid_depth` | number | USDT | `null` |
| `ask_depth` | number | USDT | `null` |
| `volume_24h` | number | quote currency | `null` |
| `peg_deviation_bps` | number | basis points from the peg | `null` |
| `bot_signs` | list of strings | `Finding::label()` names | `null` |
| `bot_signs_state` | string | `"found"`, `"none"`, `"none_no_tape"` or `"not_run"` | never missing |
| `error` | string | one-line reason | absent unless `status` is `"error"` |

Notes:

- `status` replaces the two text values `HEALTHY`/`UNHEALTHY` and the `ERROR` row. The text table keeps its upper-case words. JSON uses lower case.
- `bot_signs` and `bot_signs_state` hold what `bot_signs()` joins into one text cell today. `none`, `none (no trade tape)` and `-` become `bot_signs_state`, so a script does not parse text. `bot_signs` is `[]` for `none` and `none_no_tape`, and `null` for `not_run`.
- `peg_deviation_bps` is new. The text table has no such column, but the value is cheap (`MarketSummary::peg_deviation_bps`) and it is the number a peg comparison needs. See Q2.
- A failed venue has `status: "error"`, `error`, and `null` for every number. The 60-column cut of the text table does not apply: `error` holds the whole one-line reason.
- Number cells come from the fields of `MarketSummary`, never from the text of `row()`. A shared function reads the summary and gives a typed `ComparisonRow`. `row()` formats that struct for text and markdown. The JSON serializes it. One source feeds all three, so they cannot drift (CLAUDE.md rule 3).

### 3.2 Where the array goes

Three ways to hold it. Q1 asks Jon to choose.

**A. Envelope for 2 or more venues.**

```json
{
  "pair": "USDC/USDT",
  "venues": [
    {"venue": "weex", "run": null, "pair": "USDC/USDT", "healthy": true, "...": "..."},
    {"venue": "nowhere", "run": null, "error": "Unknown venue \"nowhere\""}
  ],
  "comparison": [
    {"venue": "weex", "status": "healthy", "mid": 1.0, "spread_pct": 0.02, "bid_depth": 42000.0,
     "ask_depth": 45000.0, "volume_24h": 125000.0, "peg_deviation_bps": 0.0,
     "bot_signs": ["Repeated size", "Metronome"], "bot_signs_state": "found"},
    {"venue": "nowhere", "status": "error", "mid": null, "spread_pct": null, "bid_depth": null,
     "ask_depth": null, "volume_24h": null, "peg_deviation_bps": null, "bot_signs": null,
     "bot_signs_state": "not_run", "error": "Unknown venue \"nowhere\""}
  ]
}
```

- This is the one change to the 2-or-more shape: a list becomes an object. Nobody has released that list, so no released script breaks.
- The one-venue output stays a bare object. A script still chooses the shape by the number of venues it names.
- `ohlc` and `trades` keep their lists. They have no comparison.

**B. Keep the list. Put one `comparison` row inside each item.**

```json
[
  {"venue": "weex", "...": "...", "comparison": {"status": "healthy", "mid": 1.0, "...": "..."}},
  {"venue": "nowhere", "error": "...", "comparison": {"status": "error", "...": "..."}}
]
```

- No shape breaks, ever. Old readers ignore the new key.
- It is a row in each item, not one array. A script that wants the table maps over the items.
- It repeats the values that the item already holds (`mid_price`, `healthy`, and so on).

**C. Keep the list. Add a flag (`--comparison`) that turns the envelope on.**

- Backward compatible by construction, but it adds a flag for a shape question and two shapes to test and document.

**One-venue output (all three).** It does not change. There is no `comparison` key, no flag and no envelope. One venue has nothing to compare, and the design of #188 pins this (section 2, decision 1).

### 3.3 Other places the rows appear

- **`--every` with JSON.** Each run prints its output. With A, each run prints one envelope with the `run` number in every `venues` item and in the envelope (`"run": 3`). With B, each item holds its own comparison row. In both, the last run decides the exit code, as today.
- **`--report`.** The markdown comparison table is unchanged. It now reads the typed rows, so its text does not change. A test pins that.
- **Repeat view (TUI).** It shows `VenueRun.comparison` text. It is unchanged.

### 3.4 CSV

- `market summary --csv` is the time series of `--every`. Its columns are metrics (`timestamp,run,venue,...`). It already has one row for each venue and run. The comparison is a view of one run. It adds no column and no file.
- A separate comparison CSV (`--comparison-csv`) is not built. The time-series CSV holds every number of the row that the series charts (mid, spread, depth, volume, health). Q2 asks whether `peg_deviation_bps` becomes a series metric.
- No change to the CSV header of the one-venue or the many-venue case.

## 4. `token-health` on a venue list

### 4.1 Flag

- `--venue` becomes `Vec<String>` with `value_delimiter = ','`. It is the attribute of `market summary` and `monitor`. `checked_venues()` checks it (empty name and duplicate are errors, before any request).
- The default stays `DEFAULT_VENUE`. `--venue` still has no effect without `--with-market`. With 2 or more venues and no `--with-market`, the command fails with `--venue needs --with-market` for a list. One venue keeps today's silent rule, so nothing old breaks (Q4 asks whether to fail for one venue too).
- The DEX analytics (`crawl::fetch_analytics_for_input`) run once. The venue loop reuses the `analytics` value. The market reads for all venues run at the same time (`futures::join_all`), as `market summary` does. They do not prompt: the token is already resolved by the analytics step.

### 4.2 What it prints for each venue

- **One venue:** unchanged, in every format.
- **2 or more venues, table and csv:** the DEX Analytics block once. Then one `Market / Order Book (venue)` block for each venue, in the order given. Each block is today's block: peg target, best bid, best ask, mid, depths, health checks, status line. A failed venue has a block with one warning row: `No market data: <reason>`. Then one comparison table (the table of section 2, from the same `comparison_table` function). The table is the last block.
- **2 or more venues, markdown:** the crawl report once. Then `## Market / Order Book` with one `###` section for each venue (`MarketSummary::format_markdown(Some(venue))`), then `### Comparison` with `comparison_markdown`.
- **2 or more venues, json:**

```json
{
  "analytics": {"...": "..."},
  "market": null,
  "markets": [ {"venue": "weex", "summary": {"...": "..."}}, {"venue": "nowhere", "error": "..."} ],
  "comparison": [ "..." ]
}
```

  `market` stays in the output and is `null` for 2 or more venues. A script that reads `market` for one venue gets a clear `null`, not a changed type. Q3 asks Jon to choose between this and other shapes.
- The `csv` format keeps its meaning: it prints the table. No CSV writer is added (section 4.4).

### 4.3 Exit codes

Today a failed market read is a warning and the exit code is 0, because the main result (the DEX analytics) is there. Two rules fit. Q5 asks Jon.

| Case | Rule X (keep the warning rule) | Rule Y (same as `market summary`) |
|---|---|---|
| Analytics fail | 1 | 1 |
| 1 venue, market fails | 0 (warning, as today) | 0 (warning, as today) |
| 2+ venues, all answer (healthy or not) | 0 | 0 |
| 2+ venues, some fail | 0, warnings and error rows | 1, after all output is printed: `1 of 3 venues failed: nowhere` |
| 2+ venues, all fail | 0 | 1 |

An unhealthy verdict never changes the code, as in `market summary`.

### 4.4 CSV impact

`token-health --format csv` has no writer today. It prints the table. This design adds none. The comparison is the machine-readable form in JSON. A real CSV (one row for each venue) is deferred (section 8).

## 5. `insights` on a venue list

### 5.1 Flag

- Add `--venue`, a `Vec<String>` with `value_delimiter = ','`, to `InsightsArgs`. It has no default value. Without it, the code is unchanged: the default venue with the registry fallback, one `Market` line, no error shown.
- `--venue` applies to a token target that is a stablecoin symbol (`is_stablecoin_symbol`). For an address, a transaction, or a non-stablecoin token, `insights` does not read a market. If `--venue` is given there, `insights` prints one note: `--venue ignored: <target type> has no market check`. It does not fail (Q6 asks).
- `checked_venues()` checks the list.
- `--format` does not exist on `insights`. This design adds none.

### 5.2 What it prints for each venue

- In the token section, one `Market (venue pair)` line for each venue, in the order given. Each is today's `peg_line`.
- A failed venue has the line `- **Market (venue):** unavailable: <reason>`. Today's silent skip applies to the no-flag path only. With `--venue` the user asked for the venue, so the failure is shown.
- The reads run at the same time. The summary is read once for each venue with the same `analytics` and `thresholds`.
- The synthesis and the key takeaway use one peg status for the token. With several venues this is the worst status. The order, worst first, is `Deviation`, `SlightDeviation`, `BookUnhealthy`, `NoPrice`, `OnPeg`. It is defined once, next to `PegStatus` (a method `severity()`), not in `insights.rs`. A venue that fails is left out of the worst-status choice. If every venue fails, `peg_status` is `None`, as today when no venue answers. (Q7 asks.)
- When the venues disagree (one `OnPeg`, one `Deviation`), the synthesis adds one sentence: `Venues disagree on the peg: weex Peg healthy, binance Peg deviation.` This is the reason to ask for more than one venue, so it is part of the output. The sentence is built from the `peg_words` labels, so the wording stays in one place.
- A comparison table is not added to `insights`. It is a prose report; the lines are short.

### 5.3 Exit codes

- Today `insights` returns 0 unless the command itself fails. The market line is a side result.
- With `--venue`: Q5 applies as well. One rule for both commands means a user learns one rule.

### 5.4 CSV impact

None. `insights` has no CSV or JSON output.

## 6. Order of work in a run

1. Check `--venue` first (empty name, duplicate, `token-health` list without `--with-market`). A failure here happens before any request.
2. Resolve the main result (DEX analytics, or the `insights` target).
3. Resolve each venue (`MarketVenue::from_name_with_http`). A failure becomes the error of that venue.
4. Read the summaries of all venues at the same time.
5. Print in the order given. Return the code.

## 7. Tests

Every test uses mocked venues (`mockito` with the real venue mappings, or the mock DEX factory). None uses the network. The tests set `SCOPE_BCA_HOME` to a fresh folder.

Each test below states why it matters, so it fails if the logic changes.

**Comparison in the summary JSON**

1. **Rows hold raw numbers.** A venue with a known book gives `mid`, `spread_pct`, `bid_depth`, `ask_depth` and `volume_24h` as JSON numbers equal to the `MarketSummary` fields. This fails if the code copies the formatted text (`"42.00K"`).
2. **Units.** `spread_pct` of 0.020% is `0.02`, not `0.0002`. `peg_deviation_bps` equals `MarketSummary::peg_deviation_bps`.
3. **A failed venue.** `status` is `"error"`, every number is `null`, `error` holds the whole reason (not the 60-column cut). The healthy venue is unaffected.
4. **Bot signs.** The four states (`found`, `none`, `none_no_tape`, `not_run`) map to the right `bot_signs`. This fails if "no tape" turns into "none".
5. **Order.** The rows keep the order given, with 3 venues in a non-alphabetical order.
6. **One venue is unchanged.** One-venue JSON is the same bytes as before (a golden test). It has no `comparison` key and no envelope. This pins the compatibility promise.
7. **Many venues, the chosen shape (A, B or C).** The `venues` items are byte-identical to the items of #188 (A), or the list is identical apart from the added key (B).
8. **One source.** The text table, the markdown table and the JSON rows come from one `ComparisonRow` list. A test builds one set of reports and checks that the three outputs agree on every venue's status and mid.
9. **`--every`, JSON.** Each run prints a valid document with the right `run`.
10. **`--report` is unchanged.** The markdown comparison text is identical to the text of before the refactor.
11. **CSV.** The header of the time series is unchanged for one venue and for many venues. If Q2 chooses (c), one new column is added at the end, and a test checks its position.

**`token-health`**

12. **Flag.** `--venue a --venue b` and `--venue a,b` give the same list. A duplicate and an empty name are errors. A list without `--with-market` fails; one venue without it does not.
13. **One venue is unchanged** in table, markdown, json and csv: golden output, including `market` as an object.
14. **Two venues, each format.** The DEX block shows once. A market block shows for each venue, in order. The comparison table is last. In JSON, `markets` has one item per venue, `market` is `null`.
15. **A failing venue does not hide the others.** One unknown venue and one answering venue give an error block and a normal block. The exit code follows the answer to Q5.
16. **Concurrency.** Two venues that each take 400 ms are read in under 780 ms, without a clock ratio (use the technique of `bbdfc98`).
17. **The analytics are fetched once.** A mock counts the crawl calls: 1 for any number of venues.
18. **An unhealthy venue does not change the exit code.**
19. **Help.** The help names `--venue weex,binance` and the exit rule.

**`insights`**

20. **No flag: unchanged.** The existing tests pass unchanged. The default-venue path is not touched.
21. **A list gives a line for each venue**, in order. A failed venue gives the `unavailable` line, not silence.
22. **Worst status.** Venues `OnPeg` + `Deviation` give the deviation synthesis and takeaway. `severity()` has its own unit test over all five statuses.
23. **Disagreement sentence** appears when the statuses differ and does not appear when they match.
24. **Ignored flag.** For an address target and a non-stablecoin token, `--venue` prints the note and no market request is made (the mock sees zero calls).
25. **Help.**

## 8. Left out, with reasons

| Item | Stays deferred? | Why |
|---|---|---|
| Per-venue deviation chart in the repeat view and the watch grid (Q8 of the watch design) | Yes | The watch design left it until the overlay is seen on real data. Nobody has reported that the overlay is hard to read. A chart is also a TUI change with its own key and layout work, and this design changes no TUI. The new `peg_deviation_bps` field in `comparison` gives the data, so the chart can come later without a data change. |
| A venue editor in the watch grid (change the venues of a target without delete and add) | Yes | The watch design said to build it only if delete-and-add is a problem in use. No such report exists. It is a TUI input flow, a separate piece of work. |
| A real CSV writer for `token-health --format csv` | Yes | `csv` prints the table today and nobody asked for a file. The comparison in JSON is the machine form. A CSV needs a column set to agree on (analytics fields and market fields in one row, or two files). That is a design question that no user has raised. |
| A comparison table for `ohlc` and `trades` | Yes | A candle comparison needs a time grid and a rule for gaps (as the first design said). The request is for numbers per venue, which the blocks give. |
| A comparison section in `insights` | Yes | `insights` is a prose report with short lines. A table would be larger than the rest of the market section. The disagreement sentence gives the one fact the table would add. |
| Concurrent `ohlc` and `trades` reads | Yes | The token check can prompt the user, and the calls are short (first design, section 6). |
| A separate comparison CSV file | Yes | The time-series CSV already has one row per venue and run (section 3.4). |
| `--venue` list on `monitor` | Not needed | `monitor --venue` already takes a list. |

## 9. Decisions

Jon accepted the recommended option for all eight questions on 2026-11-10.

| Question | Chosen option | Date |
|---|---|---|
| Q1. Where does the `comparison` array go in the many-venue summary JSON? | a. Envelope (A): `{pair, venues, comparison}`. One-venue output is unchanged. | 2026-11-10 |
| Q2. Which fields does a comparison row hold? | a. The columns of the text table, plus `peg_deviation_bps`. The time-series CSV is unchanged. | 2026-11-10 |
| Q3. What is the `token-health --format json` shape for 2 or more venues? | a. Keep `market` (it is `null`), and add `markets` and `comparison` next to `analytics`. | 2026-11-10 |
| Q4. Does `--venue` without `--with-market` fail on `token-health`? | a. Fail only for a list (2 or more venues). One venue keeps today's silent rule. | 2026-11-10 |
| Q5. What exit code does a partial venue failure give on `token-health` and `insights`? | a. Rule X: 0 when the main result printed. Failed venues show as warnings and error rows. | 2026-11-10 |
| Q6. What does `insights --venue` do for a target with no market check? | a. Print one note (`--venue ignored: ...`) and go on. | 2026-11-10 |
| Q7. How does `insights` choose one peg status from several venues? | a. The worst status (`Deviation` first), plus a sentence when the venues disagree. | 2026-11-10 |
| Q8. One PR or two? | b. Two PRs: first `comparison` and the typed-row refactor, then `--venue` on `token-health` and `insights`. | 2026-11-10 |
