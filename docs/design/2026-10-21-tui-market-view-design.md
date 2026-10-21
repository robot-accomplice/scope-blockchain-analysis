# TUI Market View: Design

- **Status:** draft for Jon's review.
- **Date:** 2026-10-21
- **Scope:** the repeating view of `market summary --every` (#58), and the market summary snapshot it shows. The grid's detail pane (#47) uses the same parts later.
- **Standard:** Clean Code and Clean Architecture. Each concern has one source, the adapters are thin, and dependencies point inward.

## 1. Problems (Jon, 2026-10-21)

Captured with the TUI harness on recorded Binance replies, at 120×36 and 80×24.

| # | Problem | Where it comes from |
|---|---|---|
| J1 | Poor information density. The bid and ask rows repeat their values: `1.000960  2629525.00 USDC  2632049.34 USDT`. For a stable pair, the base amount and the quote value are the same number twice. | The snapshot is the CLI text report (`MarketSummary::format_text_with_tokens`). |
| J2 | The panes are not sized to their data. The text side grows with the window and shows whitespace; the sparkline side could use that room for more history. | `repeat_view::draw` splits the body 50/50, whatever the content. |
| J3 | The numbers column of the trends must keep a constant width, so the numbers are never cut; the sparkline is what grows. | The label and value share one row, and the chart is a separate row under them. |
| J4 | There is no separator between the numbers and the sparkline. It should be a vertical line, or a T bar with a long horizontal line that the sparkline fills over time. | The chart row starts at column 18, with no separator. |
| J5 | The status lines at the bottom (spinner, `run 3 of 4 · next in 1s`) crowd the view above them: no separator, no blank row. | `draw` puts the status rows right under the body. |

Found in the same captures:

- The verdict (`HEALTHY`) is the last line of the snapshot; at 120×36 it is below the fold.
- One screen has three number formats: `38.54M USDT`, `38544484 USDT` and `2629525.00 USDC`.
- At 80 columns the snapshot is cut (`exchange order book│`), a wrapped line loses its indent, and the two titles are one row apart.

## 2. Design

### 2.1 Layout: content first, the rest to the trends (J2)

The body has two columns when the terminal is wide enough, else two rows.

- **Snapshot column:** as wide as its content (the widest line of the compact snapshot, section 2.2), and as tall as its content. It does not grow with the window. Free rows under it stay empty only in the stacked layout.
- **Trends column:** everything else. More width shows more history; more height makes each chart taller (section 2.3).
- **Two columns** when `body width ≥ snapshot width + 1 + TRENDS_MIN_WIDTH` (`TRENDS_MIN_WIDTH` = the numbers column plus 20 chart cells). Else the snapshot goes on top at its content height, and the trends get the rows under it.
- A single vertical rule `│` separates the two columns. There are no box titles on separate rows.

### 2.2 The compact snapshot (J1)

A TUI renderer of `MarketSummary` that writes `Vec<Line>`: a pure function in the TUI adapter. The CLI keeps its text report as it is.

```text
 USDC/USDT · binance                    ✓ HEALTHY
 Mid     1.000965  +0.096%     Spread   0.001%
 Bid     1.000960  +0.096%     Ask    1.000970
 Depth   38.54M bid · 17.43M ask      Vol 3.71B
 Exec 10K  ~0.05 bps buy · ~0.05 bps sell
 Book · USDT ─────────────────────────────────
  1.001010    3.96K ▏
  1.001000  767.88K ███
  1.000990    2.99M ███████████
  1.000980    1.16M ████
  1.000970  225.28K █
 ─ mid 1.000965 · spread 0.001% ─────────────
  1.000960    2.63M ██████████
  1.000950    6.09M ███████████████████████
  1.000940    2.18M ████████
  1.000930  938.62K ███
  1.000920  587.68K ██
  +89 asks · +95 bids · 6 outliers left out
 Checks ── 11 pass · 1 n/a ───────────────────
 ✓ no sells below peg   ✓ bid/ask 2.21x
 ✓ spread ≤ 3.0%        ✓ levels 100 / 94
 ✓ depth ≥ 3K           ✓ top-3 ≥ 300
 ✓ top-10 ≥ 2K          – depth/vol n/a
```

- **The verdict is on the first line**, with the failed count when it fails (`✗ UNHEALTHY · 2 failed`).
- **Key values in two columns:** mid with peg deviation, spread, best bid and ask, depth per side, 24h volume, execution cost.
- **The book is one ladder:** asks above (highest first), a mid line, bids below (best first). Each level has a price, one size in the quote asset, and a bar. There is no base amount: for a stable pair it repeats the quote value. The ladder shows N levels per side (5 at 120 columns, 3 at 80). A last line counts the levels left out and the outliers.
- **Checks in a grid of two columns,** with the count in the title. A failed or `n/a` check sorts first and shows its whole message on its own line under the grid.
- **One number format:** `scope::display::format::format_large_number` for sizes and depths (`2.63M`), 6 decimals for prices, 3 for percents.
- **Width:** about 46 columns. It never wraps. Below that width, the ladder drops its bars first, then its sizes.

### 2.3 The trends (J3, J4)

```text
 Trends · run 3 of 4 · every 2s
 Health                     ┤✓✓✓───────────────────────────────────────
 Mid price         1.000965 ┤███───────────────────────────────────────
 Peg deviation      +0.096% ┤▄▄▄───────────────────────────────────────
 Spread %   ≤3.0%    0.001% ┤▁▁▁───────────────────────────────────────
 Bid depth  ≥3K      38.54M ┤███───────────────────────────────────────
 Ask depth  ≥3K      17.43M ┤███───────────────────────────────────────
```

- **One row per metric:** label, limit, value, then the chart. The text part has a constant width (`NUMBERS_COLS`). The value is right-aligned, so the digits line up and are never cut.
- **The T bar `┤`** joins the numbers to a track `─` that runs to the right edge. The chart fills the track from the left, one cell per run. When the track is full, it scrolls: the newest run stays at the right edge. So the track shows how much history fits and how much has come.
- **The limit is in its own column** (`≤3.0%`, `≥3K`). A run outside its limit draws its bar in the error color. The dim threshold line across the chart goes, because it took the whole row in a one-row chart.
- **Taller charts when there is room.** Rows left over after one row per metric are shared out: each chart gets 2 or 3 rows (more resolution). The label and the value stay on the chart's last row, next to the T bar (`┤` on that row, `│` on the rows above).
- **Health** is the first row, with `✓`/`✗` per run on the same track. The last failure goes in the footer (section 2.4).

### 2.4 The footer (J5)

```text
───────────────────────────────────────────────────────────────────────────────────────────────────────────────────
 ⠙ run 3 of 4 · next in 1s · last failure: none                      Esc stops · market summary USDC --venue binance
```

- A dim rule `─` across the pane separates the view from the footer.
- One line, not two: the spinner, the progress, the last failure, and (right-aligned) the key and the command.
- After the run ends: the same line with `✓ Done in 8.2s` or `Cancelled after 4.1s`, and the leave hint.

### 2.5 Mockups

**120×36, side by side:**

```text
 scope v0.7.0 ────────────────────────────────────────────────────────────────────────────────────────────── running
 USDC/USDT · binance       ✓ HEALTHY   │ Trends · run 3 of 4 · every 2s
 Mid  1.000965 +0.096%   Spread 0.001% │ Health                      ┤✓✓✓──────────────────────────────────────
 Bid  1.000960     Ask 1.000970        │                             │
 Depth 38.54M / 17.43M   Vol 3.71B     │ Mid price          1.000965 ┤███──────────────────────────────────────
 Exec 10K ~0.05 bps buy · ~0.05 sell     │                             │
 Book · USDT ─────────────────────────── │ Peg deviation       +0.096% ┤▄▄▄──────────────────────────────────────
  1.001010    3.96K ▏                    │                             │
  1.001000  767.88K ███                  │ Spread %    ≤3.0%    0.001% ┤▁▁▁──────────────────────────────────────
  1.000990    2.99M ███████████          │                             │
  1.000980    1.16M ████                 │ Bid depth   ≥3K      38.54M ┤███──────────────────────────────────────
  1.000970  225.28K █                    │                             │
 ─ mid 1.000965 · spread 0.001% ──────── │ Ask depth   ≥3K      17.43M ┤███──────────────────────────────────────
  1.000960    2.63M ██████████           │                             │
  1.000950    6.09M ███████████████████  │ Bid top-3   ≥300     10.90M ┤███──────────────────────────────────────
  1.000940    2.18M ████████             │                             │
  1.000930  938.62K ███                  │ Ask top-3   ≥300      4.37M ┤███──────────────────────────────────────
  1.000920  587.68K ██                   │                             │
  +89 asks · +95 bids · 6 outliers out   │ Bid top-10  ≥2K      25.01M ┤███──────────────────────────────────────
 Checks ── 11 pass · 1 n/a ───────────── │                             │
 ✓ no sells below peg  ✓ bid/ask 2.21x   │ Ask top-10  ≥2K       5.15M ┤███──────────────────────────────────────
 ✓ spread ≤ 3.0%       ✓ levels 100/94   │
 ✓ depth ≥ 3K          ✓ top-3 ≥ 300     │
 ✓ top-10 ≥ 2K         – depth/vol n/a   │
 – depth/vol: the book ends before ±2%   │
                                         │
──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
 ⠙ run 3 of 4 · next in 1s · last failure: none                       Esc stops · market summary USDC --venue binance
┌scope:auto> ──────────────────────────────────────────────────────────────────────────────────────────────────────────┐
```

(Here each chart is 2 rows high: the rows left over after one row per metric are shared out.)

**80×24, stacked** (`80 < 46 + 1 + TRENDS_MIN_WIDTH`):

```text
 scope v0.7.0 ──────────────────────────────────────────────────────── running
 USDC/USDT · binance                    ✓ HEALTHY
 Mid     1.000965  +0.096%     Spread   0.001%
 Depth   38.54M bid · 17.43M ask      Vol 3.71B
 Book · USDT ── 1.000990 2.99M · 1.000980 1.16M · 1.000970 225K
            ── 1.000960 2.63M · 1.000950 6.09M · 1.000940 2.18M
 Checks ── 11 pass · 1 n/a (depth/vol: the book ends before ±2%)
──────────────────────────────────────────────────────────────────────────────
 Health                     ┤✓✓✓──────────────────────────────────────────────
 Mid price         1.000965 ┤███──────────────────────────────────────────────
 Spread %   ≤3.0%    0.001% ┤▁▁▁──────────────────────────────────────────────
 Bid depth  ≥3K      38.54M ┤███──────────────────────────────────────────────
 Ask depth  ≥3K      17.43M ┤███──────────────────────────────────────────────
──────────────────────────────────────────────────────────────────────────────
 ⠙ run 3 of 4 · next in 1s                                            Esc stops
┌scope:auto> ──────────────────────────────────────────────────────────────────┐
```

At 80×24 the snapshot is its compact form (no ladder bars; the checks in one line when they all pass), and the trends get the rows left over.

## 3. Architecture

- **`scope::market` (domain):** no change. `MarketSummary` already has every value.
- **`cli::tui::market_view` (new TUI adapter):** `fn snapshot_lines(&MarketSummary, venue, width) -> Vec<Line>` and `fn snapshot_width(...)`. They are pure, with no I/O, and their tests are table-driven.
- **The data reaches the TUI as data, not as ANSI text.** `RunRecord` gets the run's `MarketSummary` (an `Option`, so other commands still repeat without one). The CLI keeps printing `snapshot` text; the TUI draws `summary` when it has one, else the text as today.
- **`cli::charts::Sparkline`** gets a "track" mode: it fills from the left, draws the `─` track in the empty cells, and colors a bar by its limit. The monitor dashboard keeps the current mode until it goes (#47, Q1).
- **`repeat_view::draw`** computes the layout from the snapshot's width and height, not from percentages.

## 4. Tests

- **`market_view`:** the first line holds the verdict; the ladder has N levels per side and no base amounts; one number format; the width never goes past `snapshot_width` (property test over books of 0 to 200 levels).
- **Layout:** at 120×36 the two columns are side by side and the snapshot column is its content width; at 80×24 they are stacked; at 160×48 the trends get the extra width and taller charts; no row overlaps (rendered buffers).
- **Trends:** the numbers column has the same width for every metric and every value (`0.001%`, `38.54M`, `n/a`); the track fills from the left and scrolls when full; a bar outside its limit has the error style.
- **Footer:** a rule row, then one status line; after the run, the footer and the leave hint.
- **Harness:** `repeating-view.yaml` gets snapshots at 120×36, 80×24 and 160×48.

## 5. Out of scope

- The one-shot `market summary` in the pane (not repeating) still prints the CLI text. Open question Q4.
- `crawl`, `token-health` and the grid's detail pane. They get their own pass on these parts later.

## 6. Open questions for Jon

| # | Question | Proposal |
|---|---|---|
| Q1 | Book bars: each level's size, or the cumulative depth? | Each level's size: walls are what a reader looks for. |
| Q2 | Ladder levels per side? | 5 at 120 columns and up, a one-line book at 80. |
| Q3 | The track fills from the left, then scrolls (proposed), or the newest run is always at the right edge (today)? | Fill from the left. |
| Q4 | Should the one-shot `market summary` in the TUI show the compact snapshot too? | Yes: the same renderer. Ctrl-Y and `save` keep the CLI text. |
| Q5 | Extra height: taller charts (proposed), or more metrics (best bid and ask, absolute spread)? | Taller charts; the extra metrics repeat values already shown. |
