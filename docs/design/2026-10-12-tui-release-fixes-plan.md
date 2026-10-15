# TUI Release Fixes (0.7.0): Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix every High and Medium finding of the 0.7.0 TUI usability audit (R1–R21) so `scope tui` never breaks the user's terminal or loses their work and behaves well at its edges.

**Architecture:** Changes are concentrated in `crates/scope-cli/src/cli/tui/` (`mod.rs` = lifecycle/event loop, `app.rs` = state/keys/render, `exec.rs` = line parsing/help, `journal.rs`), the monitor (`crates/scope-cli/src/cli/monitor/`), the shared display layer (`crates/scope-core/src/display/terminal.rs`), error display (`crates/scope-cli/src/cli/errors.rs`, `crates/scope-core/src/error.rs`) and the harness (`crates/scope-tui-harness/`). One task per requirement; harness support first so later tasks can add scenarios.

**Tech Stack:** Rust 2024, ratatui 0.30, crossterm 0.29 (`event-stream`, bracketed paste), tokio (`signal::unix`), ansi-to-tui 8, base64 (OSC 52), unicode-width 0.2 (new direct dep), portable-pty/vt100 (harness), libc (harness, unix).

**Spec:** `docs/design/2026-10-12-tui-release-fixes-design.md`. **Code map (read the section for your task before coding):** `/private/tmp/scope-audit/codemap.md` (`T/` = `crates/scope-cli/src/cli/tui`, `M/` = `.../monitor`, `C/` = `crates/scope-cli/src/cli`).

## Global Constraints

- Commit as `robot-accomplice <robot@accomplice.ch>` (`git -c user.name=robot-accomplice -c user.email=robot@accomplice.ch commit …`). No Claude/Anthropic attribution or Co-Authored-By.
- Gates before every commit: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. Tasks that add scenarios also run `just tui-test`.
- Never mutate a process-global env var in an in-process test. Tests that touch the diag sink hold `DIAG_LOCK` (see code map, Test conventions).
- Exact user-visible strings from the spec are used verbatim: `Terminal too small: need at least 40×10`; `A command is running; Esc or Ctrl-C cancels it first.`; `Running outside the TUI; it returns when the command ends (Ctrl-C cancels).`; `Ran outside the TUI: <line> (<outcome>, <N.N>s).`; `Failed in N.Ns.` / `Cancelled after N.Ns.` / `Done in N.Ns.`; `copied N lines`; `pasted N lines as one line`; `scope tui needs an interactive terminal; run the command directly, e.g. scope market summary …`; `Venue error (<venue>):`.
- Minimum sizes: TUI 40×10, monitor 60×16. Exit code 2 when the TUI needs a terminal and has none; 128+N after a fatal signal.
- No meaning by color alone; every new state has a glyph or word. Respect `color_enabled()` (R4) everywhere new color is added.
- Prose (comments, commits, PR text, help text): plain, short, active sentences.
- Branches (stacked, merged in order): `feature/tui-fix-g0-harness` (T0) → `g1-lifecycle` (T1–T4) → `g2-outside` (T5) → `g3-input` (T6–T9) → `g4-output` (T10–T14) → `g5-nav` (T15–T18) → `g6-errors-contrast` (T19–T20) → `g7-monitor` (T21). One PR per branch into `develop`, opened by the controller.

## Spec amendments (rulings made while planning; the spec is updated in T0)

- **A1 (R16 titles):** the monitor never fetches a real venue order book or trades (code map G7). Titles: `(synthetic, from DEX pairs)` when built from DEX pairs; in `--pair` (venue-only) mode the panels read `Order Book (not available in venue-only mode)` / `Recent Trades (not available in venue-only mode)`. A real venue book in the monitor is out of scope (follow-up issue).
- **A2 (R11 width):** a width override alone does not fix the fixed 50-column rules. `section_header`, `section_footer`, `separator` and table rules use `min(their width, terminal_width())`; `terminal_width()` honors a process-global override that the TUI sets to the pane's inner width.
- **A3 (R4 acceptance):** the harness gains `env`, `args` and `wait_for_paint` scenario fields so `TERM=dumb` and the exit-2 path are harness-tested; stdin/stdout-not-a-tty stays a unit test plus manual check.
- **A4 (R1 acceptance):** the harness gains a `signal` step (unix `kill`), so R1/R2 are harness-tested instead of manual.
- **A5 (R7):** cause first. If the investigation (T8 step 1) shows the Enter is lost before raw mode, the fix is to enter raw mode first in `tui::run` (before `SessionContext::load`/`Vocab::load`) and accept `\n` as Enter; the harness `wait_for_paint: false` scenario is the acceptance.

## Review Focus

1. **A signal arrives while a pane command runs** (R1): expect the command to be cancelled, the journal to record `signal:TERM`, the terminal restored, exit 143. T1 pins it.
2. **Ctrl-Z while a pane command runs** (R2): expect the command to keep its state; on `fg` the TUI redraws with the running footer. T2 pins it.
3. **Paste during a running command** (R5): the paste goes into the input (not run, not lost). T6 pins it.
4. **`save` with a relative path that escapes the cwd (`../x`) or an existing file**: write only inside cwd or to an absolute path; refuse to overwrite without `--force`. T10 pins it.
5. **Width override leaks to the CLI** (R11): a CLI run after the TUI exits must use the real width. T12 pins it (override reset on exit and on suspend).

---

## T0: Harness support and spec amendments (branch `feature/tui-fix-g0-harness`)

**Files:** Modify `crates/scope-tui-harness/src/{scenario.rs,session.rs,run.rs}`, `crates/scope-tui-harness/Cargo.toml`; Modify `docs/design/2026-10-12-tui-release-fixes-design.md` (append "Amendments A1–A5" verbatim from this plan).

**Interfaces — Produces:**
- `scenario::Scenario` new optional fields: `env: BTreeMap<String,String>` (default empty; merged into the child env after the defaults), `args: Vec<String>` (default `["tui","--no-banner"]`; replaces the spawn args), `wait_for_paint: bool` (default `true`).
- `scenario::Step::Signal(String)` (`TERM|HUP|INT|QUIT|TSTP|CONT`), sent with `libc::kill(pid, sig)`.
- `key_bytes` new names: `Home` `\x1b[H`, `End` `\x1b[F`, `Delete` `\x1b[3~`, `Ctrl-Home` `\x1b[1;5H`, `Ctrl-End` `\x1b[1;5F`.
- `Session::start(&StartOpts)` with `StartOpts { …, args: Vec<String>, wait_for_paint: bool }`; `Session::pid() -> Option<u32>`.

- [ ] **Step 1: failing tests** in `scenario.rs`:

```rust
#[test]
fn test_new_fields_parse_with_defaults() {
    let s = parse("name: x\nsteps:\n  - expect_exit: 0\n").unwrap();
    assert!(s.env.is_empty());
    assert_eq!(s.args, vec!["tui".to_string(), "--no-banner".to_string()]);
    assert!(s.wait_for_paint);
}

#[test]
fn test_signal_step_and_new_keys() {
    let s = parse("name: x\nenv: {TERM: dumb}\nargs: [tui]\nwait_for_paint: false\nsteps:\n  - signal: TERM\n  - expect_exit: 143\n").unwrap();
    assert_eq!(s.env.get("TERM").map(String::as_str), Some("dumb"));
    assert!(matches!(&s.steps[0], Step::Signal(n) if n == "TERM"));
    assert_eq!(key_bytes("Ctrl-End").unwrap(), b"\x1b[1;5F");
    assert_eq!(key_bytes("Delete").unwrap(), b"\x1b[3~");
    assert!(parse("name: x\nsteps:\n  - signal: BOGUS\n  - expect_exit: 0\n").is_err());
}
```

- [ ] **Step 2:** run `cargo test -p scope-tui-harness scenario` → FAIL (unknown fields).
- [ ] **Step 3: implement.** Add the fields with serde defaults (`fn default_args()`, `fn yes() -> bool { true }`); validate signal names in `parse`; add key names; add `libc = "0.2"` under `[target.'cfg(unix)'.dependencies]`; in `session.rs` use `opts.args` for the spawn, apply `opts.env` after the fixed env, skip `wait_for_first_paint` when `!wait_for_paint`, add `pid()` from `child.process_id()`; in `run.rs` pass the scenario fields into `StartOpts` and execute `Step::Signal` with `unsafe { libc::kill(pid as i32, sig) }` (map names to `libc::SIGTERM` etc.), then `pump(SETTLE)`; a missing pid fails the step. An `expect_exit` after a signal must accept the exit code (already supported).
- [ ] **Step 4:** add an ignored PTY test `test_signal_step_ends_process` (`args: [tui, --no-banner]`, `signal: TERM`, `expect_exit` any non-zero — assert only that the process exits within 5 s; R1 later tightens to 143). Run `cargo build --bin scope && cargo test -p scope-tui-harness -- --include-ignored`.
- [ ] **Step 5:** append Amendments A1–A5 to the spec. Gates, commit (`feat(harness): env, args, wait_for_paint and signal steps`).

---

## G1 — Terminal lifecycle (branch `feature/tui-fix-g1-lifecycle`, from T0)

### T1: R1 signals restore and save

**Files:** `T/mod.rs` (`run`, `event_loop`, `Host`, `RealHost`, `TestHost`), `T/journal.rs` (`Event.outcome` → `Cow<'static, str>` or `String`), `crates/scope-web/src/main.rs` (exit code path), `tests/tui/scenarios/signal-term.yaml`.

**Interfaces — Produces:** `Host::signals(&mut self) -> &mut (impl Stream<Item = Signal> + Unpin)` (or an mpsc in TestHost); `enum Signal { Term, Hup, Int, Quit, Tstp, Cont }`; `pub struct TuiExit { pub code: i32 }` returned by `tui::run` via `Result<TuiExit>` (0 normally, 128+N after a signal); `main.rs` calls `std::process::exit(code)` when non-zero.

- [ ] **Step 1: failing tests** (`T/mod.rs` tests, TestHost gains a signal channel):

```rust
#[tokio::test]
async fn test_term_signal_saves_restores_and_journals() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::at(dir.path().join("h"), dir.path().join("e.jsonl"));
    // Type a line so history has content, then deliver SIGTERM.
    let (res, state, host, _) = drive_with(journal, script(&["ctx"]), vec![Signal::Term]).await;
    assert_eq!(res.unwrap().code, 143);
    assert_eq!(host.restores, 1, "terminal restored once");
    let ev = std::fs::read_to_string(dir.path().join("e.jsonl")).unwrap();
    assert!(ev.contains("\"outcome\":\"signal:TERM\""), "{ev}");
    assert!(std::fs::read_to_string(dir.path().join("h")).unwrap().contains("ctx"));
    let _ = state;
}

#[tokio::test]
async fn test_term_signal_cancels_running_pane_command() {
    // Start a repeat command, then SIGTERM: the command is dropped, code 143.
    let (res, state, _, _) = drive_signal_after(&["market summary USDC --venue eth --every 30s --duration 1h"], Signal::Term).await;
    assert_eq!(res.unwrap().code, 143);
    assert!(state.scrollback_text().contains("Cancelled"));
}
```

(Write `drive_with`/`drive_signal_after` as small extensions of the existing `drive`, taking a `Journal` and a list of signals delivered after the scripted keys.)

- [ ] **Step 2:** run the two tests → FAIL (no signals in `Host`).
- [ ] **Step 3: implement.**
  - `RealHost` owns a `SignalSet` built from `tokio::signal::unix::signal(SignalKind::{terminate,hangup,interrupt,quit})` merged into one stream (`futures::stream::select_all` over `SignalStream` mapped to `Signal`). Create it at the start of `run` (unix only; on other targets use `futures::stream::pending()`).
  - In `event_loop`'s `select!` (biased), add a signal arm BEFORE the events arm: on `Term|Hup|Int|Quit`, cancel any `pending` (`state.finish(Ending::Cancelled)` if running), record `Event { route: "session", outcome: "signal:<NAME>", … }` through a new `TuiState::record_signal(name)`, and return `Ok(TuiExit { code: 128 + signo })`.
  - `run` already restores and saves after `event_loop` returns; keep that order and return the code.
  - `journal::Event.outcome` becomes `Cow<'static, str>`; update all writers.
  - `main.rs`: `dispatch` maps `Commands::Interactive` to the exit code (add an `Outcome`/exit-code return from `scope_cli::cli::dispatch::dispatch` or a dedicated `interactive::run` path), and exits with it.
- [ ] **Step 4:** scenario `tests/tui/scenarios/signal-term.yaml`:

```yaml
name: SIGTERM restores the terminal and records the signal
tiers: [offline]
steps:
  - type: "ctx"
  - key: Enter
  - wait_for: { text: "Current Context" }
  - signal: TERM
  - expect_exit: 143
  - expect_journal: { argv_prefix: [], outcome: "signal:TERM" }
```

- [ ] **Step 5:** gates + `just tui-test`; manual: in tmux run scope, `kill -TERM`, then `stty -a` shows `icanon isig echo` and `#{alternate_on}` is 0 — record output in the report. Commit `feat(tui): restore, save and journal on SIGTERM/HUP/INT/QUIT`.

### T2: R2 suspend and resume

**Files:** `T/app.rs` (`handle_key`: Ctrl-Z → `Action::Suspend`), `T/mod.rs` (loop: `Action::Suspend` and `Signal::Tstp` → `host.suspend(); raise SIGTSTP; on resume host.resume(); terminal.clear()`), `tests/tui/scenarios/ctrl-z.yaml`.

- [ ] **Step 1: failing tests:** a TestHost test that sends Ctrl-Z and asserts `host.suspends == 1 && host.resumes == 1`, the draft text survives (`state.input == "draf"` typed before Ctrl-Z), and `host.cleared == 1` (TestHost counts `clear()` calls). In TestHost, "raise SIGTSTP" is a no-op hook (`Host::stop_self()`), so the test does not stop the test process.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: implement.** `Host::stop_self()` on RealHost: `libc::raise(libc::SIGTSTP)` is not available in scope-cli; use `nix`? Not a dep — use `tokio::process`? Ruling: add `libc = "0.2"` to scope-cli `[target.'cfg(unix)'.dependencies]` and call `libc::kill(libc::getpid(), libc::SIGSTOP)` after restore (SIGSTOP cannot be caught, so the process really stops; the shell sees a stopped job); on return from the call the process was continued → `resume()`, `terminal.clear()`. External `SIGTSTP` (from `Signal::Tstp` arm) takes the same path. A pending pane command is kept (not cancelled); its future simply is not polled while stopped.
- [ ] **Step 4:** scenario `ctrl-z.yaml`: `args: [tui, --no-banner]` cannot use job control without a shell; use `signal: TSTP` then `signal: CONT`, then `type: "x"`, `wait_for: { text: "x" }` (input still works after resume), `expect_screen: { contains: ["scope v"] }` (redrawn), then Ctrl-D, `expect_exit: 0`.
- [ ] **Step 5:** gates + `just tui-test`; manual job-control check in tmux (`bash -i`, Ctrl-Z, `fg`) recorded in the report. Commit `feat(tui): suspend on Ctrl-Z and SIGTSTP, redraw on resume`.

### T3: R4 terminal required and `color_enabled()`

**Files:** `crates/scope-core/src/display/terminal.rs` (`color_enabled()`, replace `is_tty()` in the styled wrappers), `C/errors.rs` (`is_tty_stderr` → `color_enabled_stderr()` using the same env rules), `C/progress.rs` (`atty_stderr`), `T/mod.rs` `run` (terminal check before anything else), `crates/scope-web/src/main.rs` (exit 2; check BEFORE `prompt_for_setup` for `Commands::Interactive`), `C/mod.rs` (`--no-color` flag → sets the override), monitor (`M/input.rs` `MonitorApp::new` and ratatui styles: when `!color_enabled()` use `Style::reset()` colors via a `palette_for(color_enabled)`).

**Interfaces — Produces:** `pub fn color_enabled() -> bool` and `pub fn color_enabled_with(env: &dyn Fn(&str)->Option<String>, stdout_is_tty: bool) -> bool` in `scope::display::terminal`; `pub fn set_color_override(Option<bool>)` (process-global `AtomicU8`, used by `--no-color`); `pub fn tui_terminal_ok(stdin_tty: bool, stdout_tty: bool, term: Option<&str>) -> Result<(), String>` in `T/mod.rs`.

- [ ] **Step 1: failing tests:**

```rust
#[test]
fn test_color_enabled_rules() {
    let env = |m: &'static [(&'static str, &'static str)]| move |k: &str| m.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
    assert!(color_enabled_with(&env(&[]), true));
    assert!(!color_enabled_with(&env(&[("NO_COLOR", "1")]), true));
    // no-color.org: only a non-empty NO_COLOR disables color.
    assert!(color_enabled_with(&env(&[("NO_COLOR", "")]), true));
    assert!(!color_enabled_with(&env(&[("TERM", "dumb")]), true));
    assert!(!color_enabled_with(&env(&[]), false));
}

#[test]
fn test_tui_requires_a_terminal() {
    assert!(tui_terminal_ok(true, true, Some("xterm-256color")).is_ok());
    for (i, o, t) in [(false, true, Some("xterm")), (true, false, Some("xterm")), (true, true, Some("dumb"))] {
        let e = tui_terminal_ok(i, o, t).unwrap_err();
        assert!(e.starts_with("scope tui needs an interactive terminal"), "{e}");
    }
}
```

- [ ] **Step 2:** FAIL.
- [ ] **Step 3: implement.** `color_enabled_with`: false if `NO_COLOR` is set and non-empty, false if `TERM == "dumb"`, false if stdout is not a tty, else true; an override (from `--no-color`) wins. Replace each `xxx_styled(.., is_tty())` call with `xxx_styled(.., color_enabled())`. In `main.rs`, for `Commands::Interactive`, call `tui_terminal_ok(stdin().is_terminal(), stdout().is_terminal(), env TERM)` before config load/setup prompt; on Err print the message to stderr and `exit(2)`. Monitor: under `!color_enabled()` build the palette with `Color::Reset` for all fields and drop `fg` colors (bold/reverse stay).
- [ ] **Step 4:** scenario `term-dumb.yaml`: `env: {TERM: dumb}`, `wait_for_paint: false`, `steps: [ { expect_exit: 2 } ]` plus an `expect_screen` on the message (screen before exit is captured by the PTY). Manual: `scope tui </dev/null`, `echo x | scope tui`, `scope tui >f` each exit 2 with the message (record).
- [ ] **Step 5:** gates + `just tui-test`. Commit `feat: color_enabled() for NO_COLOR, TERM=dumb and non-TTY; tui requires a terminal`.

### T4: R20 panic hook with crash log

**Files:** `T/mod.rs` (install once, after the first `ratatui::init`, guarded by a `OnceLock`), new `T/crash.rs` (`write_crash_log(dir, info_text, journal_tail) -> io::Result<PathBuf>`), `T/journal.rs` (`tail(n) -> Vec<String>`).

- [ ] **Step 1: failing test** in `crash.rs`:

```rust
#[test]
fn test_crash_log_has_message_location_and_journal_tail() {
    let dir = tempfile::tempdir().unwrap();
    let p = write_crash_log(dir.path(), "boom at src/x.rs:1:2", &["{\"input\":\"ctx\"}".to_string()]).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(p.file_name().unwrap().to_string_lossy().starts_with("crash-"));
    assert!(text.contains("boom at src/x.rs:1:2") && text.contains("\"input\":\"ctx\""));
}
```

- [ ] **Step 2:** FAIL. **Step 3:** implement; the hook (chained once with `std::panic::take_hook`) restores the terminal (`ratatui::restore()`, disable bracketed paste), captures `std::backtrace::Backtrace::force_capture()`, writes the log to `dirs::data_dir()/scope/`, prints `scope crashed; details: <path>` to stderr, then calls the previous hook. Re-init on resume must not stack hooks (`OnceLock`).
- [ ] **Step 4:** manual: a debug-only hidden session command `__panic` (cfg(debug_assertions)) triggers a panic; record the printed path and the log head. Gates. Commit `feat(tui): crash log on panic`.

---

## G2 — Outside-the-TUI route (branch `feature/tui-fix-g2-outside`, from G1)

### T5: R3 cancel and auto-return

**Files:** `T/mod.rs` (`run_suspended`), `T/app.rs` (`record_suspended` gains outcome `cancelled` and the new note text), tests in `T/mod.rs`, `tests/tui/scenarios/outside-cancel.yaml` + fixture.

- [ ] **Step 1: failing tests:** update `test_loop_suspend_mode_hands_over_the_terminal_and_records`: expect `Running outside the TUI; it returns when the command ends (Ctrl-C cancels).` in `suspended.out()`, NO `Press Enter to return`, and a scrollback note matching `^Ran outside the TUI: setup --key nosuchkey \(ok, \d+\.\ds\)\.$`. Add `test_suspended_command_is_cancelled_by_sigint`: a TestHost whose signal channel delivers `Signal::Int` while a never-ending suspended invocation runs (use a test-only route override or a `crawl` with a mock dex that never resolves); assert scrollback `(cancelled, …s)`, journal outcome `cancelled`, and the loop continues (a following `ctx` line runs).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: implement.** In `run_suspended`: print the notice line; `tokio::select! { r = run_invocation(..) => r, _ = host.interrupt() => Err(Cancelled) }` where `Host::interrupt()` on RealHost awaits `tokio::signal::ctrl_c()` (cooked mode → real SIGINT) and on TestHost awaits the signal channel's `Int`; remove the Enter wait (delete `wait_to_return` from `Host`); `record_suspended` takes an outcome enum (`Ok|Error|Cancelled`) and writes the note with `{:.1}s`. Note: a blocking `read_line` inside `crawl`/`setup` prompts cannot be interrupted by the select (code map G2); in that state Ctrl-C still raises SIGINT, so the handler installed for `ctrl_c()` keeps the process alive and the cancel completes when the read returns — document this limitation in the PR.
- [ ] **Step 4:** scenario `outside-cancel.yaml` (offline): `crawl USDC` with a fixture that makes DexScreener search hang is not possible with replay (replay answers instantly). Instead use `monitor USDC --venue binance --pair USDC_USDT` with a replay fixture for the ticker, wait for the monitor screen, send `Ctrl-C` (monitor treats it as quit → outcome `ok`), then assert the TUI is back without pressing Enter (`wait_for: { text: "Ran outside the TUI: monitor" }`). Record the binance ticker fixture with `SCOPE_HTTP_MODE=record` into `tests/tui/fixtures/outside-cancel/` (review it for secrets; none expected for public endpoints). The SIGINT cancel path is covered by the unit test.
- [ ] **Step 5:** gates + `just tui-test`. Commit `feat(tui): commands outside the TUI cancel with Ctrl-C and return automatically`.

---

## G3 — Input (branch `feature/tui-fix-g3-input`, from G2)

### T6: R5 bracketed paste and horizontal input scroll

**Files:** `T/mod.rs` (enable after every `ratatui::init`/`resume`, disable before every `restore`/`suspend`; loop arm `Event::Paste(s) => state.paste(&s)`), `T/app.rs` (`paste`, input render with `scroll_x`), `crates/scope-cli/Cargo.toml` (`unicode-width = "0.2"`), `tests/tui/scenarios/paste.yaml`.

- [ ] **Step 1: failing tests** (app.rs):

```rust
#[test]
fn test_paste_joins_lines_and_never_runs() {
    let mut s = state();
    s.paste("venues list\nmarket summary USDC\n");
    assert_eq!(s.input, "venues list market summary USDC");
    assert!(s.scrollback_text().contains("pasted 2 lines as one line"));
}

#[test]
fn test_long_input_keeps_cursor_visible() {
    let mut s = state();
    s.paste(&"x".repeat(300));
    let sc = screen(&s, 60, 12);
    let input_row = sc.lines().find(|l| l.contains("xxxx")).unwrap();
    assert!(input_row.trim_end().ends_with('│') || input_row.len() <= 60);
    // The visible input ends at the cursor: the last visible char is 'x', not cut mid-border.
}
```

(Tighten the second test to assert the rendered cursor x position equals the inner right edge minus 1 using `Frame` cursor from `TestBackend::get_cursor_position`.)
- [ ] **Step 2:** FAIL. **Step 3:** implement `paste`: replace `\r\n`/`\n`/`\r` with spaces, trim one trailing separator, insert at cursor, note when line count > 1; paste during a run inserts into the input too. Input render: compute the cursor display column with `UnicodeWidthStr::width(&input[..byte_index(cursor)])`; `scroll_x = col.saturating_sub(inner_width - 1)`; `Paragraph::scroll((0, scroll_x))`; cursor x = `inner.x + col - scroll_x`.
- [ ] **Step 4:** scenario `paste.yaml`: `type: "\e[200~venues list\nhelp\e[201~"`; `wait_for: { text: "pasted 2 lines as one line" }`; `expect_screen: { contains: ["venues list help"], not_contains: ["Available Venues"] }` (the pasted `venues list` did not run); `key: Ctrl-U`; `key: Ctrl-D`; `expect_exit: 0`. Gates + `just tui-test`. Commit `feat(tui): bracketed paste and scrolling input line`.

### T7: R17 unicode cursor

**Files:** `T/app.rs` (all width math via unicode-width: cursor, menu width at :621-627).

- [ ] **Step 1: failing test:** type `"日本 ok"`, `End`; assert the cursor x from `TestBackend::get_cursor_position()` equals `input.x + 1 + 7` (日本 = 4 columns, space, ok = 2). Repeat with `"😀x"` (emoji = 2 columns).
- [ ] **Step 2:** FAIL. **Step 3:** implement (shares T6's display-column helper; if T6 already fixed it, this task only adds the menu width and tests). **Step 4:** gates. Commit `fix(tui): cursor and menu widths use display width`.

### T8: R7 Enter during startup (#63)

**Files:** `T/mod.rs` (`run`: order of init), `tests/tui/scenarios/startup-enter.yaml`.

- [ ] **Step 1: investigate (record evidence before changing code).** Add a temporary `tracing::debug!` of every crossterm event in the loop; run a cold copy of the binary in tmux with `-vv` piped to a log, send `ctx` + Enter immediately after spawn; record which event (if any) the Enter produced. Remove the temporary logging.
- [ ] **Step 2: failing scenario** `startup-enter.yaml`: `wait_for_paint: false`, `type: "ctx\r"` as the first step, `wait_for: { text: "Current Context", timeout: 15s }`, Ctrl-D, `expect_exit: 0`. Run against a freshly copied binary (`SCOPE_BIN=$(mktemp)…` per the Task-12 reproduction) → FAIL.
- [ ] **Step 3: implement per amendment A5:** call `ratatui::init()` (and the paste enable) first in `tui::run`, then load session/vocab and push the welcome; additionally map a key event of `KeyCode::Char('\n')` / `Ctrl-J` to Enter in `handle_key` (covers bytes that arrived in cooked mode). If step 1 shows a different cause, stop and report (NEEDS_CONTEXT) instead of applying A5.
- [ ] **Step 4:** scenario passes 3/3 on fresh copies; then remove the harness's need for `wait_for_paint` nowhere else (keep default `true`). Gates + `just tui-test`. Commit `fix(tui): keep Enter typed during startup (#63)`.

### T9: R19 busy note

**Files:** `T/app.rs` `handle_key` (Ctrl-D and Enter-with-`exit|quit|q` while `running.is_some()`).

- [ ] **Step 1: failing test:** start a pane invocation (hold `DIAG_LOCK`), press Ctrl-D → scrollback contains `A command is running; Esc or Ctrl-C cancels it first.` and `handle_key` returned `Action::None`; type `exit`, Enter → same note, input kept.
- [ ] **Step 2–3:** FAIL, implement (note once per key press; do not duplicate if the last scrollback line is already the note). **Step 4:** scenario lines added to `paste.yaml` are not needed; add `busy-note.yaml`: `market summary USDC --venue binance --every 5s --duration 1m` with a recorded binance fixture, Ctrl-D, `wait_for` the note, Esc, Ctrl-D, `expect_exit: 0`. Gates + `just tui-test`. Commit `feat(tui): explain why exit waits while a command runs`.

---

## G4 — Output pane (branch `feature/tui-fix-g4-output`, from G3)

### T10: R6 copy and save

**Files:** `T/app.rs` (`last_output: Option<String>` set in `finish()` and in session-command output; Ctrl-Y; pane block `Borders::TOP | Borders::BOTTOM`), `T/exec.rs` (`save <file> [--force]` session command; help text: `Ctrl-Y copy last output · Shift-drag selects text`), new `T/clip.rs` (`osc52(text) -> String`, `strip_ansi(&str) -> String`), `tests/tui/scenarios/save-json.yaml`.

- [ ] **Step 1: failing tests:**

```rust
#[test]
fn test_osc52_encodes_base64_payload() {
    assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
}

#[test]
fn test_strip_ansi_removes_sgr() {
    assert_eq!(strip_ansi("\x1b[1;36mA\x1b[0m b"), "A b");
}

#[test]
fn test_save_writes_last_output_and_refuses_escape_and_overwrite() {
    // run `venues list --format json` as a pane command with the mock factory, then:
    // `save out.json` -> file is valid JSON (serde_json::from_str::<Value>)
    // `save ../x.json` -> refused ("save writes inside the current directory or to an absolute path")
    // second `save out.json` -> refused without --force; with --force -> written
}
```

- [ ] **Step 2:** FAIL. **Step 3:** implement: `last_output` = `strip_ansi(captured.out())` for successful runs (exclude the error display and the footer); Ctrl-Y writes `osc52(last_output)` to the terminal through `Host` (new `Host::write_raw(&str)`), notes `copied N lines`; when there is no output, notes `nothing to copy yet`. `save`: resolve relative paths against `std::env::current_dir()`, reject any path whose canonical parent is outside cwd unless absolute; refuse to overwrite without `--force`.
- [ ] **Step 4:** scenario `save-json.yaml`: `venues list --format json` (no network), `wait_for: Done in`, `save out.json`, `wait_for: saved`, `expect_file: { path: "out.json", contains: ["\"binance\""] }`. (The harness child cwd is the temp HOME, so `out.json` is under HOME.) Gates + `just tui-test`. Commit `feat(tui): Ctrl-Y copies and save writes the last output; pane has no side borders`.

### T11: R8 Failed / Cancelled / Done footers

**Files:** `T/app.rs` `finish()`; update existing assertions (`Done in`, `Cancelled.`) in `T/mod.rs` and `T/app.rs` tests; `C/output.rs` untouched.

- [ ] **Step 1: failing tests:** ok → last scrollback line matches `^Done in \d+\.\ds\.$`; error → `^✗ Failed in \d+\.\ds\.$` and the line's style fg is red; cancel → `^Cancelled after \d+\.\ds\.$`.
- [ ] **Step 2–3:** FAIL, implement (`push_styled_note(text, style)`; glyph `✗` always present). **Step 4:** scenario `footers.yaml`: `venues list` → `Done in`; `market summary USDC --venue nosuch` → `Failed in`; Gates + `just tui-test`. Commit `feat(tui): failed and cancelled runs end with their own footer`.

### T12: R11 width-aware output

**Files:** `crates/scope-core/src/display/terminal.rs` (`static WIDTH_OVERRIDE: AtomicUsize`, `set_width_override(Option<usize>)`, `terminal_width()` honors it; `section_header_styled`, `section_footer`, `separator`, `table_header_styled` rules use `min(fixed, terminal_width())`; `wrap_lines` measures with display width), `T/app.rs` (`render` stores the pane inner width and calls `set_width_override(Some(inner.width))` when it changes; `run`/`suspend` reset to `None`), hanging-indent pre-wrap of pane lines.

- [ ] **Step 1: failing tests:** (core) with override 40, `section_footer()` plain text width ≤ 40 and `section_header_styled("Available Venues", false)` width ≤ 40; (core) `wrap_lines` on CJK counts display width; (tui) `screen(&s, 40, 20)` after a `venues list` run has no line where a `─` run continues onto the next row (assert no row that starts with `─` after a row that ends with `─`); (tui) after `TuiState` drop / explicit reset, `terminal_width()` returns the real width (Review Focus 5).
- [ ] **Step 2–3:** FAIL, implement; hanging indent: before building the `Paragraph`, split each `Line` whose display width exceeds `inner.width` into continuation lines prefixed with the source line's leading whitespace and box gutter characters (`│ `), preserving span styles; disable ratatui wrap for pane lines that were pre-wrapped.
- [ ] **Step 4:** scenario `narrow.yaml`: terminal 40×20, `venues list`, `snapshot: venues-40` (review the written snapshot). Gates + `just tui-test`. Commit `feat: output fits the pane width; wrapped lines keep their gutter`.

### T13: R13 status bar priority

**Files:** `T/app.rs` `status_line()` → `fn status_parts(&self) -> StatusParts` + `fn fit_status(parts, width) -> Line`.

- [ ] **Step 1: failing tests:** at widths 120, 60, 40, 30 while running: always contains `running ` and `Esc cancels`; at 40 does not contain `limit` or `format`; contains `…` when something was cut; idle at 30 still contains `help`.
- [ ] **Step 2–4:** FAIL, implement (drop order: limit, format, chain, command text; display-width math), gates. Commit `fix(tui): status bar keeps the running state and cancel hint`.

### T14: R14 liveness and repeat status

**Files:** `C/output.rs` (`Output::progress_hook() -> Option<ProgressHook>`; `ProgressHook = Arc<Mutex<RunProgress>>`, `RunProgress { run: u64, total: Option<u64>, next_at: Option<Instant>, banner: Option<String> }`), `C/market.rs` (`run_summary` updates the hook: banner text at start, run number, total = `duration/every + 1`, next run instant), `T/app.rs` (spinner line rendered after the running output: `⠋ running <cmd>… Ns`, ASCII `-\|/` when `!color_enabled()`; `run N of M · next in Ns` when progress has a run), `tests/tui/scenarios/repeat-status.yaml` + recorded binance fixture.

- [ ] **Step 1: failing tests:** (market) with a capture output that has a hook, `run_summary` with `--every 1s --duration 2s` against a mock exchange sets `total = Some(3)` and run increments; (tui) rendering a running state whose hook says `run 2, total 4, next in 3s` shows `run 2 of 4 · next in 3s`; spinner glyph present.
- [ ] **Step 2–4:** FAIL, implement, scenario: `market summary USDC --venue binance --every 2s --duration 5s`, `wait_for: { text: "run 2 of" }`, `wait_for: Done in`. Gates + `just tui-test`. Commit `feat(tui): spinner and run status for repeating commands`.

---

## G5 — Navigation and help (branch `feature/tui-fix-g5-nav`, from G4)

### T15: R9 `help <command>`

**Files:** `T/exec.rs` (`"help"` arm: if words remain, re-plan as `[words…, "--help"]`).

- [ ] **Step 1: failing test:** `plan_line("help market summary")` output contains `Usage:` and `--venue`, and does not contain `Session commands`.
- [ ] **Step 2–4:** FAIL, implement, scenario line in `help.yaml` (`help market summary`, `wait_for: "--venue"`). Gates. Commit `feat(tui): help <command> shows that command's help`.

### T16: R10 scrolling

**Files:** `T/app.rs` (`last_max_top: Cell<usize>`, `last_pane_height: Cell<usize>` written by `render`; clamp in PageUp; `PAGE` replaced by `pane_height-1`; Ctrl-Home/Ctrl-End; End on empty input; printable key → `scroll = 0`; status shows scroll indicator during runs too), update `test_render_shows_latest_output_prompt_and_status`.

- [ ] **Step 1: failing tests:** 50×PageUp on a short scrollback then 1×PageDown changes the visible top line; Ctrl-End returns to bottom; typing `a` while scrolled sets scroll 0; while running and scrolled, status contains both `running` and `scrolled up`.
- [ ] **Step 2–4:** FAIL, implement, scenario `scroll.yaml` (help ×3, PageUp ×20, PageDown, `expect_screen` changed vs a snapshot is brittle → assert the status text `scrolled up` decreases: wait_for regex). Gates + `just tui-test`. Commit `feat(tui): bounded scrolling with page, top and bottom keys`.

### T17: R12 too-small message (TUI)

**Files:** `T/app.rs` `render` guard (`MIN_W=40`, `MIN_H=10` constants).

- [ ] **Step 1: failing test:** `screen(&s, 30, 8)` contains `Terminal too small: need at least 40×10`; `s.input` unchanged; `screen(&s, 40, 10)` does not contain it.
- [ ] **Step 2–4:** FAIL, implement (keep existing tiny-terminal no-panic tests), scenario `too-small.yaml` (type `draft`, resize 30×8, `wait_for` message, resize 120×36, `expect_screen: draft`). Gates + `just tui-test`. Commit `feat(tui): too-small terminal message`. (Monitor guard is in T21.)

### T18: R18 search

**Files:** `T/app.rs` (`Overlay::Search { query, matches: Vec<usize>, idx }` for scrollback; `Overlay::HistorySearch { query, idx }`; Ctrl-F and `/` on empty input open search; Ctrl-R opens history search; n/N/Enter/Esc; matched rows highlighted with reverse + `▶` marker in the gutter), help Keys line updated.

- [ ] **Step 1: failing tests:** scrollback with lines `alpha`, `beta`, `alpha2`: Ctrl-F, type `alpha` → status shows `1/2`, view scrolled so the match is visible; `n` → `2/2`; Esc closes; history `["venues list","ctx","market summary USDC"]`: Ctrl-R, type `ven` → input becomes `venues list`, Enter accepts (does not run), Esc restores the draft.
- [ ] **Step 2–4:** FAIL, implement, scenario `search.yaml`. Gates + `just tui-test`. Commit `feat(tui): search the scrollback (Ctrl-F, /) and history (Ctrl-R)`.

---

## G6 — Errors and contrast (branch `feature/tui-fix-g6-errors-contrast`, from G5)

### T19: R15 error wording

**Files:** `crates/scope-core/src/error.rs` (`ScopeError::Venue { venue: String, message: String }` with `#[error("Venue error ({venue}): {message}")]`), venue sites in `scope-core/src/market/{configurable_client.rs,exchange.rs,registry.rs}` and `C/venues.rs` migrated; tests asserting `Chain client error` for venue failures updated (`C/monitor/mod.rs:3703-3718`); `C/errors.rs` (`display_error_styled` gains `tui: bool` via `Output::is_tui()` flag set by the TUI capture; in TUI mode the `-v` suggestion becomes `run 'help <command>' for usage; set 'format json' for raw output`), `T/exec.rs` `write_clap_error` (strip clap's `For more information, try '--help'.` and append `run 'help <command>' for its options`).

- [ ] **Step 1: failing tests:** a venue HTTP 400 error displays `Venue error (binance):`; in TUI mode a `Network` error's suggestion does not contain `-v`; `plan_line("market summary --bogus")` output does not contain `try '--help'` and contains `help market summary`.
- [ ] **Step 2–4:** FAIL, implement (non-venue `Chain` uses unchanged), gates. Commit `fix: venue errors name the venue; TUI errors give TUI next steps`.

### T20: R21 contrast

**Files:** `T/app.rs` (note, hint, palette description, status-left styles → `Style::new().add_modifier(Modifier::DIM)` on default fg, never DarkGray), `M/widgets.rs` + `M/config.rs` (text styles: replace `Color::DarkGray`/`Color::Gray`/`.gray()`/`.dark_gray()` on text with default fg + DIM or a named readable color; borders may keep DarkGray; meaningful dim-only rows get a glyph/word), new guard test `crates/scope-cli/tests/tui_contrast.rs`.

- [ ] **Step 1: failing guard test:** scan `crates/scope-cli/src/cli/tui/*.rs` and `crates/scope-cli/src/cli/monitor/{widgets,config}.rs` for `DarkGray`, `Color::Gray`, `.gray()`, `.dark_gray()` on lines that are not border styles (a line counts as border when it also contains `border` or `Borders`); allow-list none. It fails on today's code.
- [ ] **Step 2–4:** FAIL, implement, update tests that assert old colors (e.g. `first.style.fg == Some(Color::Red)` stays), gates + `just tui-test`. Commit `fix(tui,monitor): readable secondary text on dark and light themes`.

---

## G7 — Monitor (branch `feature/tui-fix-g7-monitor`, from G6)

### T21: R16 monitor fixes + too-small guard

**Files:** `M/widgets.rs` (`fn footer_spans(state) -> Vec<Span>` with `Q Quit` first and PAUSED independent of `error_message`; error moved to the header title; panel titles via `fn order_book_title(state)` / `fn trades_title(state)` per amendment A1; size guard at the top of `ui()` with `60×16`), `M/state.rs` (`venue_only: bool`; `auto_layout` false when the user chose a layout), `M/input.rs` (`fetch_data` skips `dex_client.get_token_data` when `venue_only`), `M/mod.rs` (`monitor_config_from_args` reports whether `--layout` was given; `MonitorApp` struct literals in tests gain new fields).

- [ ] **Step 1: failing tests:** `footer_spans` with `error_message=Some(..)` and `paused=true` contains `Q`/`Quit` first and `PAUSED`; header title contains the error; `venue_only` state: `fetch_data` with a `MockDexDataSource` that counts calls → 0 DEX calls over 3 refreshes; titles: DEX-built book → `(synthetic, from DEX pairs)`, venue-only → `(not available in venue-only mode)`; `--layout exchange` → first `ui()` frame keeps `Exchange`; `ui()` at 50×12 shows `Terminal too small: need at least 60×16`.
- [ ] **Step 2–4:** FAIL, implement, gates. Scenario `monitor-pair.yaml` (offline, recorded binance ticker fixture; a DexScreener request would be a replay miss and show in the header): `monitor USDC --venue binance --pair USDC_USDT --layout exchange`, `wait_for: "Order Book \\(not available in venue-only mode\\)"`, `expect_screen: { not_contains: ["API Error", "no fixture"] }`, `key: q`, `wait_for: "Ran outside the TUI: monitor"`. Gates + `just tui-test`. Commit `fix(monitor): quit hint and pause always visible; venue-only mode; layout honored; size guard`.

---

## After G7

Controller: open the follow-up issue for amendment A1 (real venue order book/trades in the monitor), run the final whole-branch review across G0–G7, then hand over to the color-theme design.
