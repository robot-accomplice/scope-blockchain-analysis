# TUI Release Fixes (0.7.0): Design

- **Status:** draft for review
- **Date:** 2026-10-12
- **Source:** the TUI usability audit of `develop` @ `321d131` (local report `scope-tui-usability-audit.html`; findings A–E). Checklist numbers in brackets.
- **Decision (Jon, 2026-10-12):** fix every High and Medium finding before 0.7.0. Low findings are tracked in #66, #67 and #68. The color theme follows in the same release, as a separate design.

## 1. Goal

`scope tui` never leaves the user's terminal broken, never loses their work without warning, and behaves like a well-made TUI at its edges: signals, suspend, paste, copy, small terminals, long output and failures.

## 2. Requirements

Each requirement has an ID, the required behavior, and an acceptance check. "Harness" means a scenario in `tests/tui/scenarios/`. "Unit" means a test in the crate. "Manual" means a check the harness cannot express, recorded in the PR.

### Tier 1: High

**R1 Signals restore and save [53, 59].**
On SIGTERM, SIGHUP, SIGINT and SIGQUIT (outside a running pane command), scope restores the terminal (raw mode off, alternate screen off, cursor visible), saves history and the session, writes a journal record with outcome `signal:<NAME>`, and exits with 128+N.
- Acceptance: unit tests for the save-and-exit path. Manual: `kill -TERM`, then `stty -a` shows `icanon isig echo`, and tmux shows `alternate_on=0`, for each of the four signals.

**R2 Suspend and resume [54].**
Ctrl-Z, and an external SIGTSTP, restore the terminal and stop the process. On SIGCONT, scope re-enters raw mode and the alternate screen, redraws completely, and keeps the draft.
- Acceptance: Manual (tmux plus `bash -i`): Ctrl-Z, then the shell prompt is clean; `fg`, then a full redraw with the draft intact.

**R3 Cancel commands that run outside the TUI [9, 20, 25, M1].**
Ctrl-C during a suspend-route command (`crawl`, `monitor`, `setup`) cancels the command and returns to the TUI. The journal records `cancelled`. History is saved.
- Before scope leaves the TUI, it prints a one-line notice: `Running outside the TUI; it returns when the command ends (Ctrl-C cancels).`
- After the command ends, scope returns to the TUI without asking for Enter.
- The scrollback line shows the outcome and the duration: `Ran outside the TUI: crawl USDC (cancelled, 3.2s).`
- Acceptance: harness: start `crawl` against a replay fixture that never ends, send Ctrl-C, and expect the TUI back with a journal record of `cancelled`.

**R4 Terminal required [46, 57].**
`scope tui` checks that stdin and stdout are terminals before it starts. If not, it prints `scope tui needs an interactive terminal; run the command directly, e.g. scope market summary …` to stderr and exits with status 2. There is no panic.
- When `TERM=dumb`, the TUI refuses the same way, because a full-screen UI cannot work there.
- One function, `color_enabled()`, decides color for the TUI, the monitor and the CLI: off when `NO_COLOR` is set, when `TERM=dumb`, or when stdout is not a terminal.
- Acceptance: unit tests for `color_enabled`. Manual: `scope tui </dev/null`, `echo x | scope tui`, `scope tui >f` and `TERM=dumb scope tui` all exit 2 with the message.

**R5 Bracketed paste [29].**
Bracketed paste is on. A paste is inserted into the input as one edit and is never run.
- Newlines in a paste become spaces, and a note says `pasted 3 lines as one line`.
- The input scrolls horizontally, so the cursor stays visible on long lines.
- Acceptance: harness, with a bracketed-paste key sequence: the paste is not run, the input holds the joined text, and the journal has no new record.

**R6 Copy and save output [37, 40].**
- `Ctrl-Y` (yank) copies the last command's raw output, exactly as the command wrote it (no borders, no wraps, no footer), to the clipboard through OSC 52. A note says `copied N lines`.
- `save <file>` writes the same raw text to a file under the current directory, or to an absolute path.
- The output pane has no left or right border, so a mouse selection has no `│` characters.
- Help says that Shift-drag selects text.
- Acceptance: harness: run `venues list --format json`, then `save out.json`, then `expect_file` holds valid JSON. Unit: OSC 52 encoding. Manual: paste after Ctrl-Y in iTerm2 and Terminal.app.

R6 uses Ctrl-Y. Ctrl-Y as "yank" (paste from the kill ring) belongs to readline. The TUI has no kill ring, and #67 can choose another key if one is ever added.

**R7 Enter during startup (#63).**
Investigate the cause first (log the crossterm key events during startup). Then make Enter work in that window, either by enabling raw mode before anything reads input, or by treating `\n` as Enter.
- Acceptance: the harness option to wait for the first paint can be turned off for one scenario, and that scenario passes on a cold binary.

### Tier 2: Medium

**R8 Failed is not Done [19, 63].**
A command that ends with an error ends with `Failed in N.Ns.` (red, and marked `✗`, so it does not rely on color). A cancelled command ends with `Cancelled after N.Ns.`. A command that succeeds ends with `Done in N.Ns.` (dim).
- Acceptance: harness, with an `expect_screen` check for each of the three outcomes.

**R9 `help <command>` [61].**
`help market summary` prints that command's clap help, the same as `scope help market summary`.
- Acceptance: harness.

**R10 Scrolling [13, 38].**
- The scroll offset is clamped in the key handler.
- PageUp and PageDown move by the pane height minus one line.
- Ctrl-Home jumps to the top, and Ctrl-End jumps to the bottom. End on an empty input also jumps to the bottom.
- Any printable key snaps the view to the bottom.
- The scroll indicator stays visible while a command runs.
- Acceptance: unit tests for scroll state. Harness: PageUp ×50, PageDown ×1, and the view moves.

**R11 Wrapping and width [35, 36].**
- During a TUI run, the display layer uses the pane's inner width as the terminal width, so tables and rules fit the pane.
- Lines that still wrap keep their leading gutter and indent (hanging indent).
- Acceptance: harness: `venues list` at 40 columns, with a snapshot that has no rule wrapped onto a second row.

**R12 Terminal too small [42, M4].**
Below 40×10 (TUI) or 60×16 (monitor), scope draws one centered line, `Terminal too small: need at least 40×10`, and keeps the draft. When the terminal grows again, the UI comes back.
- Acceptance: harness: resize to 30×8, see the message; resize back and see the draft.

**R13 Status bar priority [34].**
When the status bar is too narrow, drop parts in this order: limit, format, chain, then the command text. Always keep `running Ns · Esc cancels`. Show `…` where text was cut.
- Acceptance: harness at 40 columns while a command runs.

**R14 Liveness and repeat status [17].**
- While a pane command runs, the pane shows a spinner line, `⠋ running market summary… 4s` (with an ASCII spinner when color is off).
- An `--every` command shows `run 2 of 4 · next in 3s` between runs, and the start banner that the CLI prints.
- Acceptance: harness: an `--every` run against a replay fixture, with `wait_for` on `run 2 of`.
- The full repeating view (charts) stays in #58.

**R15 Error wording [23].**
- Venue errors are labelled `Venue error (<venue>):`, not `Chain client error:`.
- In the TUI, the `use -v` and clap `--help` tips are replaced by TUI-specific next steps (`run 'help <command>'`, `set 'format json' for raw output`).
- Acceptance: unit tests for the label, and a harness check of the tip text.

**R16 Monitor fixes [M2, M3, M5].**
- The footer always keeps `q Quit` and the PAUSED state, even when an error is set; the error moves into the header line.
- With `--pair`, the monitor does not call DexScreener.
- `--layout` is applied at start.
- The Order Book and Recent Trades panel titles name their source: `(synthetic, from DEX pairs)` or `(<venue>)`.
- Acceptance: unit tests on the footer and title builders. Harness: a monitor run with `--pair` against replay fixtures, with no DexScreener request (a replay miss would fail the run).

**R17 Unicode cursor [30].**
The cursor column uses display width (`unicode-width`), not the character count.
- Acceptance: unit test with CJK and emoji.

**R18 Search [39, 28].**
- Ctrl-F, or `/` on an empty input, opens a search in the scrollback: n and N move between matches, matches are highlighted, and Esc closes the search.
- Ctrl-R searches history incrementally.
- Acceptance: harness for both.

**R19 Busy note [10].**
Ctrl-D or `exit` while a command runs shows `A command is running; Esc or Ctrl-C cancels it first.`
- Acceptance: harness.

**R20 Panic hook [56].**
A panic hook restores the terminal, writes `crash-<time>.log` (the message, location, backtrace and the last journal records) to the scope data directory, and prints the path.
- Acceptance: unit test of the log writer. Manual: force a panic in a debug build.

**R21 Contrast [48, 50].**
- Secondary text uses the default foreground with dim, or a named ANSI color that is readable on both dark and light themes.
- `Color::DarkGray` (`38;5;8`) and `Color::Gray` are not used for text.
- No meaning is carried by dim alone.
- The color theme design may refine this, but these rules hold.
- Acceptance: a guard test that greps the TUI and monitor sources for `DarkGray` and `Color::Gray` on text styles.

## 3. Build order

Each group is one PR into `develop`, with harness scenarios added for its R items.

| Group | Requirements | Why together |
|---|---|---|
| G1 Terminal lifecycle | R1, R2, R4, R20 | Same code: startup, exit, signals, panic |
| G2 Outside-the-TUI route | R3 | `run_suspended` |
| G3 Input | R5, R7, R17, R19 | Input handling and the event loop |
| G4 Output pane | R6, R8, R11, R13, R14 | Pane rendering and run footers |
| G5 Navigation and help | R9, R10, R12, R18 | Scroll, search, help and size |
| G6 Errors and contrast | R15, R21 | Display wording and styles |
| G7 Monitor | R16 | Monitor crate area |

Then:
1. The color theme design.
2. The release-candidate harness run.
3. The demo recordings.
4. The release.

## 4. Risks

- **Signal handling and suspend differ between macOS and Linux.** Test both before G1 merges.
- **OSC 52 support differs between terminals.** `save` is the reliable path, and the Ctrl-Y note says when the terminal did not confirm the copy.
- **Pane-width override (R11).** The display layer reads the terminal width in about 25 places. The override must be scoped to a TUI run, so the CLI is unaffected.
- **Removing the side borders (R6) changes the look.** The color theme work restyles the pane anyway.
