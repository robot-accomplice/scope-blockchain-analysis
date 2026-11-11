# Design: a CSV writer for `token-health --format csv`

Date: 2026-11-11
Status: proposed. The open questions in section 9 need an answer before code.
Follows: `2026-11-10-market-venue-followups-design.md`, sections 4.4 and 8.

## 1. Problem

`scope token-health --format csv` prints the same table as `--format table`. The output has box-drawing characters and colour codes. It is not CSV. The README says `--format csv` gives "CSV, spreadsheet import". The follow-up design (section 8) deferred a real writer. It named the open point: a CSV needs one column set, with the analytics fields and the market fields in one row, or two files.

This document picks that column set. It adds no flag and no new command.

## 2. Decisions in short

1. One file, one header.
2. One row for each venue, in the order given. The DEX analytics repeat in every row.
3. The same header for one venue, many venues, and no `--with-market`.
4. Raw numbers, never the table strings.
5. A failed venue has a row. The row has `status` = `error` and the whole reason.
6. The quoting function of `scope watch` moves to a shared place. No second CSV helper is written.
7. The change goes in the CHANGELOG as a fix. Section 7 gives the text.

## 3. What exists today

There is no shared CSV helper. There are three writers:

| Writer | Where | Quoting | Missing value | Flags |
|---|---|---|---|---|
| Time series of `market summary --csv` | `scope-core/src/metrics.rs` (`csv_header_venue`, `csv_row_venue`) | none | `-` | `true` / `false` |
| Watch trail | `scope-cli/src/cli/watch/run.rs` (`csv_rows`, private `csv_field`) | RFC 4180 | empty | not used |
| `crawl --format csv`, `discover`, `address` | one `outln!` each | none (`discover` swaps commas for `;`) | field left out | not used |

`crawl --format csv` uses a `metric,value` layout. That layout cannot hold a row for each venue, so this design does not copy it.

The typed row `ComparisonRow` (`market_venues.rs`) already holds the market numbers of one venue as raw values. The text table, the markdown table and the JSON `comparison` array come from it. This design makes the CSV a fourth view of the same row. The four then cannot differ.

The header of the time series is frozen by a test (`the_time_series_csv_header_does_not_change`). This design does not touch it.

## 4. Row shape

Three shapes were weighed.

| Shape | For | Against |
|---|---|---|
| A. One row for each venue, analytics repeated | One header. Every row is complete. A spreadsheet filter, a pivot table and `pandas.read_csv` work with no code. Same shape for 1 and N venues. | The analytics repeat. There are 9 analytics cells, so the cost is small. |
| B. One analytics row, then one row for each venue, with a `kind` column | No repeat. | Half the cells of each row are empty. A reader must filter on `kind` first. Sorting breaks the file. |
| C. Two files | No repeat, no empty cells. | `--format csv` writes to stdout. It cannot write two files. It needs a new `--output` flag, and the pipe use case is lost. |

Decision: shape A. See Q1 for the options.

Rows by case:

| Case | Rows |
|---|---|
| No `--with-market` | 1 row. The market cells are empty, including `venue` and `status`. |
| `--with-market`, 1 venue, answered | 1 row. |
| `--with-market`, 1 venue, failed | 1 row, `status` = `error`. See Q6. |
| `--with-market`, N venues | N rows, in the order given. A failed venue keeps its place. |

The file always has the header and at least one data row. Progress text and warnings go to stderr (the `Output` rule). Stdout holds only CSV.

## 5. Header

The header is a constant in the source. The order is fixed. A new column goes at the end.

```
symbol,chain,contract,price_usd,price_change_24h,volume_24h,liquidity_usd,market_cap,top_10_concentration,venue,status,mid,spread_pct,bid_depth,ask_depth,market_volume_24h,peg_deviation_bps,bot_signs,bot_signs_state,error
```

20 columns. Columns 1 to 9 come from `TokenAnalytics`. Columns 10 to 20 come from `ComparisonRow`.

| Column | Source | Unit |
|---|---|---|
| `symbol`, `chain`, `contract` | `token.symbol`, `chain`, `token.contract_address` | text |
| `price_usd` | `price_usd` | USD |
| `price_change_24h` | `price_change_24h` | percent (`-0.01` is -0.01%) |
| `volume_24h` | DEX `volume_24h` | USD |
| `liquidity_usd` | `liquidity_usd` | USD |
| `market_cap` | `market_cap` | USD, empty if unknown |
| `top_10_concentration` | `top_10_concentration` | percent, empty if unknown |
| `venue` | `ComparisonRow.venue` | the name as given |
| `status` | `ComparisonRow.status` | `healthy`, `unhealthy`, `error`; empty with no market |
| `mid` | `mid` | quote currency |
| `spread_pct` | `spread_pct` | percent |
| `bid_depth`, `ask_depth` | same | quote currency |
| `market_volume_24h` | `ComparisonRow.volume_24h` | quote currency |
| `peg_deviation_bps` | same | basis points from the peg |
| `bot_signs` | `bot_signs` | names joined with `;` |
| `bot_signs_state` | same | `found`, `none`, `none_no_tape`, `not_run` |
| `error` | `error` | the whole one-line reason |

Naming rules:

- Analytics names follow the `crawl --format csv` names (`price_usd`, `volume_24h`, `liquidity_usd`, `market_cap`, `top_10_concentration`). One name for one meaning across the two commands.
- Market names follow the keys of the JSON `comparison` rows.
- Two `volume_24h` values exist: the DEX volume in USD and the venue volume in the quote currency. They are not the same number. The market one is `market_volume_24h` in the CSV, so no column name repeats. This is the one place where a CSV name differs from the JSON key. The JSON keeps `volume_24h` because it nests the two in separate objects.

Columns left out, with the reason: `name` (free text from a DEX, no use as a key, `symbol` and `contract` identify the token), `volume_7d`, `fdv`, `total_holders` (not in the table or the comparison; add at the end on request), best bid, best ask, peg target, the 10k slippage values and the health checks (see Q2).

## 6. Values, quoting, text safety

Numbers:

- A number is the Rust `f64` text (`to_string`). It is the shortest text that reads back to the same value. It never uses an exponent. The decimal mark is always `.`, whatever the locale.
- No rounding, no thousands separator, no `$`, no `%`, no unit suffix. The unit is in section 5, and in the `--help` text.
- This is the same text as the JSON and the time series. A value such as `0.19999999999998908` can show the float noise that the JSON shows. A reader who wants fewer digits rounds in the tool that reads the file.
- A value that is missing is an empty cell (Q3). A value that is `NaN` or infinite is also an empty cell. A test pins this.

Quoting:

- RFC 4180: a field is quoted when it holds a comma, a double quote, CR or LF. A quote inside a field is doubled.
- The line end is LF, as in the other writers and in the fixtures.
- The fields that can hold such characters: `symbol`, `bot_signs`, `error`. The reason of a venue error often holds a comma.

Text from outside (Q4):

- `symbol` comes from a DEX API. The `error` text comes from a venue. A spreadsheet runs a cell that starts with `=`, `+`, `-` or `@` as a formula. The README sells this output as "spreadsheet import", so this is a real risk.
- Decision: in the text columns only (`symbol`, `bot_signs`, `error`), a value that starts with `=`, `+`, `-`, `@`, TAB or CR gets a leading `'`. Numeric columns are not touched, because a negative number is correct data.

## 7. Reuse and the breaking change

### Reuse

- Rows come from `ComparisonRow`, built with `comparison_rows`. For one venue, the code builds the `VenueReport` that the many-venue path already uses, so there is one path for 1 and N venues.
- `csv_field` moves out of `watch/run.rs` to a small shared module (Q5). The watch trail calls it from there. Its tests move with it. The text of the function does not change, so the trail does not change.
- The writer adds one thing: a function that joins the cells of a row, and the neutralising step of section 6. Both sit next to `csv_field`. No second quoting function exists.
- The series writer in `metrics.rs` stays as it is. It uses `-` for a missing value and does not quote. These two differences are real, and Q3 records them. The header of the series is frozen, so no change is made there.

### Is it a breaking change?

Technically yes, for a script that reads today's output. In practice no valid reader exists. Today's output is a box-drawn table with colour codes. No CSV parser reads it, and the README says `csv` gives CSV. The change makes the documented behaviour true.

It also ends a promise in the v0.8.7 notes: "One venue, and the default, give the output they gave before, in every format." That promise is false for `csv` from now on. The CHANGELOG must say so in plain words.

Proposed CHANGELOG entry, under `Fixed` (Q7):

> `token-health --format csv` now prints CSV. It printed the same box table as `--format table`, which no CSV reader could parse. The file has one header and one row for each venue, in the order given, with the DEX analytics repeated in each row. Numbers are raw (no `$`, `%` or `K`/`M`/`B` suffix). A venue that fails has a row with `status` `error` and the reason in `error`. Without `--with-market`, there is one row and the market cells are empty. Scripts that read the old table output of `--format csv` must change; use `--format table` for it. `table`, `json` and `markdown` do not change.

Related edits: the `--format` help of `token-health` (name the unit rule and point to the header), the README line for `--format csv`, and the "csv prints the table" lines in the follow-up design (a one-line note that this document replaces them).

## 8. Tests

Each test says why it exists.

1. `the_csv_header_is_stable`. Compares the first line with a literal string in the test. A script reads columns by name or by position. A change of order or name must fail a test and make the author read the CHANGELOG rule.
2. `the_header_is_the_same_for_one_venue_many_venues_and_no_market`. A reader that handles one case must handle the others.
3. `one_row_for_each_venue_in_the_order_given`. Uses a list in a non-alphabetical order. The order is a promise made for every other format of this command.
4. `no_market_gives_one_row_with_empty_market_cells`. The analytics remain useful without `--with-market`.
5. `a_failed_venue_has_a_row_with_status_error_and_the_reason`. Uses a reason with a comma and a double quote. The row has the right number of cells, the number cells are empty, and the other venues are not affected. The exit code is still 0.
6. `a_one_venue_failure_has_an_error_row` (Q6). Today only a warning shows. A pipe user must see the failure in the data.
7. `numbers_are_raw_not_formatted`. For every numeric column of the fixture, the cell parses as `f64` and has none of `$`, `%`, `,`, `K`, `M`, `B`. A formatted cell breaks a spreadsheet sum.
8. `missing_and_non_finite_numbers_are_empty_cells`. Q3.
9. `the_cells_equal_the_json_comparison_for_the_same_run`. The CSV and the JSON `comparison` array come from the same rows. This test fails if someone gives the CSV its own source. It is the same idea as the watch test that compares JSON lines with CSV rows.
10. `text_that_looks_like_a_formula_is_neutralised_and_numbers_are_not`. Q4. Uses `=HYPERLINK(...)` as a symbol and `-0.01` as a number.
11. `stdout_is_only_csv`. The whole stdout parses with the quoting rules. Every line has 20 fields. No box character and no colour code. Warnings are on stderr.
12. `csv_field_is_shared_and_the_watch_trail_is_unchanged`. The moved function keeps its tests, and the existing watch CSV test passes with no edit.
13. `markdown_override_still_wins`. With `--ai`, the command prints markdown, as before.
14. Fixture, byte for byte: `tests/fixtures/venue_list/token_health_one_venue.csv` is replaced. The expected text is typed by hand from the section 5 rules and the mock inputs. It is not captured from the new writer. The test masks nothing: the CSV holds no time or version. A second fixture `token_health_two_venues_one_failed.csv` pins the many-venue case.

Two existing tests must change, because they encode the old behaviour:

- `csv_prints_the_same_table_as_table_for_a_list` is replaced by tests 2 and 3.
- `one_venue_gives_exactly_the_old_output_in_every_format` keeps `table`, `markdown` and `json` against the `develop` captures. Its `csv` entry moves to test 14. The doc comment on `develop_output` says that `csv` printed the table; it needs one sentence that says the `csv` fixture is now written by hand.

## 9. Open questions

**Q1. Row shape.**
a. One row for each venue, analytics repeated. (Recommended.)
b. One analytics row and one row for each venue, with a `kind` column.
c. Two files.

Recommend a. It is the only option that is a flat table, so every spreadsheet and data tool reads it with no step. The repeat costs 9 cells. Option c needs a new flag and loses the pipe. Option b makes every reader filter first.

**Q2. How many market columns?**
a. Only the `ComparisonRow` fields (the 8 of the comparison, plus `status` and `error`). (Recommended.)
b. Also `best_bid`, `best_ask`, `peg_target`, and the 10k buy and sell slippage.
c. Everything in `MarketSummary`, including the checks.

Recommend a. It keeps the CSV equal to the JSON `comparison` and gives test 9 a hard meaning. Option b needs new fields on `ComparisonRow`, which also change the JSON `comparison` and the tables. That is a wider change and needs its own design. Adding columns at the end later does not break a reader that reads by name. Note that the one-venue table shows best bid and best ask, so a user may miss them. If this matters, choose b and put the new fields in `ComparisonRow` first.

**Q3. Spelling of a missing value.**
a. An empty cell, as in the watch trail. (Recommended.)
b. `-`, as in the time series.

Recommend a. An empty cell is the null of `pandas`, R and spreadsheets, and a number column stays numeric. A `-` turns the column into text. This leaves two spellings in the product (time series `-`, others empty). That is a conflict. The frozen series header and its users are the reason to leave the series as it is; list it for a later cleanup, with a CHANGELOG line, if wanted.

**Q4. Formula injection.**
a. Neutralise the text columns only, with a leading `'`. (Recommended.)
b. Neutralise every cell that starts with `=`, `+`, `-`, `@`, including numbers.
c. Do nothing.

Recommend a. `symbol` and `error` come from third parties, and the README offers the file to spreadsheets. Option b breaks negative numbers. Option c leaves a known attack open. Option a changes the symbol text only when it starts with one of the six characters.

**Q5. Where does `csv_field` live?**
a. A new small module `cli/csv.rs`. (Recommended.)
b. Keep it in `watch/run.rs` and make it `pub(crate)`.
c. `scope-core`, next to the series writer.

Recommend a. It is a move with no change of text. Option b makes `token-health` depend on the `watch` module for a general function. Option c is useful if the series writer will quote later, but `scope-core` has no other need today.

**Q6. A single venue that fails.**
a. A row with `status` `error` and the reason, as for many venues. (Recommended.)
b. A row with empty market cells and the warning on stderr only, as for the table.

Recommend a. Stdout is the data channel. A script that reads only stdout must be able to tell "no market data" from "venue not asked". The cost: the single-venue path must keep the error text that it drops today (it prints a warning and goes on). The stderr warning stays.

**Q7. CHANGELOG class and version.**
a. `Fixed`, in the next patch release (0.8.8). (Recommended.)
b. `Changed`, in the next patch release, with the same text.
c. `Changed`, in a minor release (0.9.0).

Recommend a. The README always said `csv` gives CSV, and no reader could parse the old output. The entry names the break so that no one finds it by surprise. Choose c if you want the 0.x minor number to mark every change of output.

**Q8. A time column.**
a. None. (Recommended.)
b. A `fetched_at` column (UTC, ISO 8601), as in the time series.

Recommend a. The command is one reading, and the DEX fetch time is not the time of the venue book. Option b would need a mask in the byte-for-byte fixture test and adds a time that means two things. A user who appends runs to one file can add the time with the shell. If `--every` ever reaches `token-health`, the time series is the right tool.

## 10. Out of scope, with reasons

- A `--csv <file>` option for `token-health`: no user asked, and `--format csv > file` does the job.
- Holder, pair and social lists as extra CSV sections: `crawl --format csv` has a holders block. A second table in one file is not CSV. A user who needs holders uses `crawl`.
- A change of the series writer (quoting, empty cells): its header is frozen and its users exist. See Q3.
- Rounding: raw numbers are the request. Rounding hides data from the reader.
