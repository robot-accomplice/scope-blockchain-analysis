# TUI Watchlists and Learnability: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the approved design in five sub-projects. This file gives the full task plan for the first one that can start now (learnability: completion, hints, palette), and the schedule for the other four.

**Architecture:** Learnability adds three pure modules to the phase 2 TUI (`crates/scope-cli/src/cli/tui/`). Each reads the clap definition (`Cli::command()`) and a snapshot of the user's stores (`Vocab`). `TuiState` gets one overlay field for the completion menu and the palette. No handler changes.

**Tech Stack:** Rust 2024, clap 4 (derive + introspection), ratatui 0.30, crossterm 0.29. No new dependencies.

**Spec:** `docs/design/2026-10-10-tui-watchlist-design.md` (approved 2026-10-10).

## Global Constraints

- One source of truth: completion, hints and the palette read `Cli::command()`. No hand-written list of CLI commands or flags.
- No new crates.
- Commit identity: `robot-accomplice <robot@accomplice.ch>`. No Claude or Anthropic attribution in commits or PRs.
- Commit messages and PR text: Simplified Technical English.
- Gates before each push: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`, and the pre-push coverage hook (no drop below `.coverage-last`).
- Tests never read or write the user's real files. Pass a `Vocab` built in the test.
- Tests that call `TuiState::start` hold `crate::cli::tui::DIAG_LOCK`.

## Review Focus

1. **Tab with the cursor not at the end of the line.** Expected: nothing changes; no panic. Test in Task 3.
2. **Tab inside an open quote** (`address-book add 0x1 --label "cold wal`). Expected: no candidates; no panic. Test in Task 2.
3. **Missing or corrupt stores** (no address book, a bad aliases file, no venue directory). Expected: completion still offers commands and flags; no error overlay. Test in Task 1.
4. **Long candidate lists** (all venues, 50 history entries) and a tiny terminal. Expected: the menu and the palette have a bounded height and scroll; no panic at 10x5. Tests in Tasks 3 and 5.
5. **Tab or Ctrl-K while a command runs.** Expected: both work on the input line; Enter stays blocked until the command ends. Test in Task 3.

---

## Sub-project schedule

| Plan | Sub-project | Spec section | Write its plan when | Why then |
|---|---|---|---|---|
| **L** (this file) | Completion, hints, palette | 5.1–5.4 | now | Depends only on phase 2, which is merged. |
| E | Watchlist engine | 2 | #47 phase 5 is merged | Its fetchers and runner consume the phase 5 shared book fetch and runner. Their signatures do not exist yet. |
| H | Headless `scope watch` | 4 | plan E is merged | It consumes `scope::watch::{Watchlist, Snapshot, Event}` from E. |
| U | Grid, drill-down, wizard | 3 | plan E is merged | Same reason as H. It also closes #47 phase 6. |
| F | Guided first run | 5.5 | plans U and L are merged | The welcome flow opens the wizard (U) and uses the palette and completion (L). |

Before plan E: #47 phase 3 (monitor-path defects), #37 (synthetic AMM book, `n/a`), #47 phase 5 (shared market core). Their plans are in #47 and #37.

---

## File structure (plan L)

| File | Responsibility |
|---|---|
| Create `crates/scope-cli/src/cli/tui/vocab.rs` | `Vocab`: the dynamic values (address-book labels, token aliases, venue IDs, chains). Loads them once; a failed store gives an empty list. |
| Create `crates/scope-cli/src/cli/tui/complete.rs` | `complete(line, vocab) -> Completion`. Pure. |
| Create `crates/scope-cli/src/cli/tui/hint.rs` | `hint(line) -> Option<Hint>`. Pure. |
| Create `crates/scope-cli/src/cli/tui/palette.rs` | `PaletteItem`, `items(history)`, `score(query, text)`, `PaletteState`. Pure except for rendering. |
| Modify `crates/scope-cli/src/cli/tui/exec.rs` | Make the chain list and the session-command list public in the crate, and build `write_help` from them. |
| Modify `crates/scope-cli/src/cli/tui/app.rs` | `Overlay` state, key routing for Tab and Ctrl-K, rendering of the menu, the hint row and the palette. |
| Modify `crates/scope-cli/src/cli/tui/mod.rs` | Declare the modules. Load `Vocab` once at start. |

---

### Task 1: `Vocab` and shared session-command list

**Files:**
- Create: `crates/scope-cli/src/cli/tui/vocab.rs`
- Modify: `crates/scope-cli/src/cli/tui/exec.rs` (the `CHAINS` const, `write_help`)
- Modify: `crates/scope-cli/src/cli/tui/mod.rs` (`pub mod vocab;`)

**Interfaces:**
- Produces: `pub struct Vocab { pub labels: Vec<String>, pub aliases: Vec<String>, pub venues: Vec<String>, pub chains: Vec<String> }`, `Vocab::load(config: &Config) -> Vocab`, `Vocab::default()`.
- Produces: `pub(crate) const CHAINS: &[&str]` and `pub(crate) const SESSION_COMMANDS: &[(&str, &str, &str)]` in `exec.rs`. The tuple is (word, usage, description).

- [ ] **Step 1: Write the failing tests** in `vocab.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_with_missing_stores_still_has_chains() {
        // Review Focus 3: no address book, no data dir. Completion of
        // chains must still work.
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.address_book.data_dir = Some(dir.path().join("nothing-here"));
        let v = Vocab::load(&config);
        assert!(v.labels.is_empty());
        assert!(v.chains.iter().any(|c| c == "ethereum"));
    }

    #[test]
    fn test_load_with_corrupt_address_book_gives_empty_labels() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("address_book.yaml"), "addresses: [unclosed").unwrap();
        let mut config = Config::default();
        config.address_book.data_dir = Some(dir.path().to_path_buf());
        let v = Vocab::load(&config);
        assert!(v.labels.is_empty());
    }

    #[test]
    fn test_labels_come_from_the_address_book() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("address_book.yaml"),
            "addresses:\n  - address: '0x1'\n    label: cold\n    chain: ethereum\n    tags: []\n    added_at: 0\n",
        )
        .unwrap();
        let mut config = Config::default();
        config.address_book.data_dir = Some(dir.path().to_path_buf());
        assert_eq!(Vocab::load(&config).labels, vec!["cold"]);
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p scope-bca-cli --lib -- cli::tui::vocab`
Expected: FAIL to compile, "cannot find type `Vocab`".

- [ ] **Step 3: Implement `vocab.rs`**

```rust
//! # Completion Vocabulary
//!
//! The values that completion offers besides commands and flags: address-book
//! labels, saved token aliases, venue IDs and chains. They load once when the
//! TUI starts. A store that fails to load gives an empty list, so completion
//! of commands and flags keeps working.

use super::exec::CHAINS;
use scope::config::Config;
use scope::domain::address_book::AddressBook;
use scope::market::VenueRegistry;
use scope::tokens::TokenAliases;

/// Dynamic completion values.
#[derive(Debug, Clone, Default)]
pub struct Vocab {
    /// Address-book labels, without the `@`.
    pub labels: Vec<String>,
    /// Saved token alias symbols.
    pub aliases: Vec<String>,
    /// Venue IDs from the registry.
    pub venues: Vec<String>,
    /// Chain names.
    pub chains: Vec<String>,
}

impl Vocab {
    /// Loads the vocabulary from the user's stores.
    pub fn load(config: &Config) -> Self {
        let mut labels: Vec<String> = AddressBook::load(&config.data_dir())
            .map(|b| b.addresses.into_iter().filter_map(|a| a.label).collect())
            .unwrap_or_default();
        labels.sort();
        labels.dedup();
        let mut aliases: Vec<String> = TokenAliases::load()
            .list()
            .into_iter()
            .map(|t| t.symbol.clone())
            .collect();
        aliases.sort();
        aliases.dedup();
        let venues = VenueRegistry::load()
            .map(|r| r.list().into_iter().map(str::to_string).collect())
            .unwrap_or_default();
        Self {
            labels,
            aliases,
            venues,
            chains: CHAINS.iter().map(|c| c.to_string()).collect(),
        }
    }
}
```

- [ ] **Step 4: Share the session-command list in `exec.rs`**

Change `const CHAINS` to `pub(crate) const CHAINS`. Add above `write_help`:

```rust
/// The session commands: (word, usage, description). Help, completion and
/// the palette read this list.
pub(crate) const SESSION_COMMANDS: &[(&str, &str, &str)] = &[
    ("chain", "chain [name|auto]", "Show, pin or unpin the chain"),
    ("format", "format [table|json|csv|markdown]", "Show or set the output format"),
    ("+tokens", "+tokens", "Toggle token balances for address"),
    ("+txs", "+txs", "Toggle transactions for address"),
    ("trace", "trace", "Toggle trace for tx"),
    ("decode", "decode", "Toggle decode for tx"),
    ("limit", "limit [n]", "Show or set the transaction limit for address"),
    ("tokens", "tokens [list|recent|add|remove]", "Manage saved token aliases"),
    ("ctx", "ctx", "Show the session context"),
    ("clear", "clear", "Reset the session context"),
    ("help", "help", "Show this help"),
    ("exit", "exit", "Leave the TUI (also Ctrl-D)"),
];
```

In `write_help`, replace the hard-coded session list with:

```rust
    outln!(out, "Session commands:")?;
    for (_, usage, what) in SESSION_COMMANDS {
        outln!(out, "  {:<34} {}", usage, what)?;
    }
```

and change the keys line to:

```rust
    outln!(
        out,
        "Keys: Enter run · Tab complete · Ctrl-K palette · ↑/↓ history · PgUp/PgDn scroll · Esc or Ctrl-C cancel"
    )
```

Update `test_help_lists_session_and_cli_commands`: replace `"chain [name|auto]"` with `"+tokens"`, so the test checks a row that only `SESSION_COMMANDS` supplies.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p scope-bca-cli --lib -- cli::tui`
Expected: PASS, including the 3 new `vocab` tests and the help test.

- [ ] **Step 6: Commit**

```bash
git add crates/scope-cli/src/cli/tui/vocab.rs crates/scope-cli/src/cli/tui/exec.rs crates/scope-cli/src/cli/tui/mod.rs
git commit -m "feat(tui): add the completion vocabulary and share the session-command list"
```

---

### Task 2: `complete()`

**Files:**
- Create: `crates/scope-cli/src/cli/tui/complete.rs`
- Modify: `crates/scope-cli/src/cli/tui/mod.rs` (`pub mod complete;`)

**Interfaces:**
- Consumes: `Vocab` (Task 1), `SESSION_COMMANDS` (Task 1), `crate::cli::Cli`.
- Produces: `pub struct Completion { pub start: usize, pub candidates: Vec<String> }` and `pub fn complete(line: &str, vocab: &Vocab) -> Completion`. `start` is the byte index where the word being completed begins. Each candidate replaces `line[start..]`.

- [ ] **Step 1: Write the failing tests** in `complete.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn vocab() -> Vocab {
        Vocab {
            labels: vec!["cold".into(), "hot".into()],
            aliases: vec!["USDC".into(), "USDT".into()],
            venues: vec!["binance".into(), "bitget".into(), "okx".into()],
            chains: vec!["ethereum".into(), "solana".into()],
        }
    }

    fn cands(line: &str) -> Vec<String> {
        complete(line, &vocab()).candidates
    }

    #[test]
    fn test_root_offers_cli_and_session_commands() {
        let c = cands("ad");
        assert!(c.contains(&"address".to_string()));
        assert!(c.contains(&"address-book".to_string()));
        assert_eq!(cands("cha"), vec!["chain"]);
    }

    #[test]
    fn test_subcommands_flags_and_enum_values() {
        assert_eq!(cands("market su"), vec!["summary"]);
        assert!(cands("market summary USDC --ve").contains(&"--venue".to_string()));
        assert_eq!(cands("address 0x1 --format c"), vec!["csv"]);
    }

    #[test]
    fn test_dynamic_values() {
        assert_eq!(cands("market summary USDC --venue bi"), vec!["binance", "bitget"]);
        assert_eq!(cands("address 0x1 --chain so"), vec!["solana"]);
        assert_eq!(cands("address @co"), vec!["@cold"]);
        assert_eq!(cands("crawl US"), vec!["USDC", "USDT"]);
    }

    #[test]
    fn test_start_points_at_the_word() {
        let c = complete("market su", &vocab());
        assert_eq!(c.start, "market ".len());
    }

    #[test]
    fn test_inside_open_quote_offers_nothing() {
        // Review Focus 2.
        assert!(cands("address-book add 0x1 --label \"cold wal").is_empty());
    }

    #[test]
    fn test_empty_line_offers_commands_and_unknown_command_offers_nothing() {
        assert!(cands("").contains(&"address".to_string()));
        assert!(cands("nosuchcommand x").is_empty());
    }

    #[test]
    fn test_every_visible_subcommand_is_offered_at_the_root() {
        // One source of truth: a new CLI command is completable with no
        // extra code.
        let all = cands("");
        for sub in crate::cli::Cli::command().get_subcommands() {
            if !sub.is_hide_set() {
                assert!(all.contains(&sub.get_name().to_string()), "{}", sub.get_name());
            }
        }
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p scope-bca-cli --lib -- cli::tui::complete`
Expected: FAIL to compile, "cannot find function `complete`".

- [ ] **Step 3: Implement `complete.rs`**

```rust
//! # Tab Completion
//!
//! Completes the last word of the input line from the clap definition and
//! the [`Vocab`]. Commands, flags and enum values come from `Cli::command()`,
//! so a new command or flag completes with no extra code.

use super::exec::SESSION_COMMANDS;
use super::vocab::Vocab;
use crate::cli::Cli;
use clap::{Arg, Command, CommandFactory};

/// The candidates for the word that ends the line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Completion {
    /// Byte index where the word starts. A candidate replaces `line[start..]`.
    pub start: usize,
    /// Sorted, unique candidates that start with the word.
    pub candidates: Vec<String>,
}

/// Completes the last word of `line`.
pub fn complete(line: &str, vocab: &Vocab) -> Completion {
    let start = line.rfind(' ').map_or(0, |i| i + 1);
    let word = &line[start..];
    // Inside an open quote the word boundaries are not known.
    if line[..start].matches('"').count() % 2 == 1 || word.starts_with('"') {
        return Completion { start, candidates: Vec::new() };
    }
    let before: Vec<&str> = line[..start].split_whitespace().collect();

    let root = Cli::command();
    let mut cmd: &Command = &root;
    let mut positional_seen = false;
    for w in &before {
        if w.starts_with('-') {
            continue;
        }
        match (positional_seen, cmd.find_subcommand(w)) {
            (false, Some(sub)) => cmd = sub,
            _ => positional_seen = true,
        }
    }
    let at_root = std::ptr::eq(cmd, &root);
    if at_root && positional_seen {
        // The first word is not a known command.
        return Completion { start, candidates: Vec::new() };
    }

    let pool: Vec<String> = if let Some(arg) = before
        .last()
        .filter(|p| p.starts_with('-'))
        .and_then(|p| find_arg(cmd, p))
        .filter(|a| a.get_action().takes_values())
    {
        values_for(arg, vocab)
    } else if word.starts_with('@') {
        vocab.labels.iter().map(|l| format!("@{}", l)).collect()
    } else if word.starts_with('-') {
        cmd.get_arguments()
            .filter(|a| !a.is_hide_set())
            .filter_map(|a| a.get_long().map(|l| format!("--{}", l)))
            .chain(std::iter::once("--help".to_string()))
            .collect()
    } else if cmd.has_subcommands() && !positional_seen {
        let mut names: Vec<String> = cmd
            .get_subcommands()
            .filter(|s| !s.is_hide_set())
            .map(|s| s.get_name().to_string())
            .collect();
        if at_root {
            names.extend(SESSION_COMMANDS.iter().map(|(w, _, _)| w.to_string()));
        }
        names
    } else {
        vocab.aliases.clone()
    };

    let mut candidates: Vec<String> = pool.into_iter().filter(|c| c.starts_with(word)).collect();
    candidates.sort();
    candidates.dedup();
    Completion { start, candidates }
}

/// Finds the argument that a flag word names (`--venue`, `--venue=x`, `-c`).
fn find_arg<'a>(cmd: &'a Command, flag: &str) -> Option<&'a Arg> {
    if let Some(long) = flag.strip_prefix("--") {
        let long = long.split('=').next().unwrap_or(long);
        cmd.get_arguments().find(|a| a.get_long() == Some(long))
    } else {
        let short = flag.strip_prefix('-')?.chars().next()?;
        cmd.get_arguments().find(|a| a.get_short() == Some(short))
    }
}

/// The values a flag accepts: its enum values, or a store from the vocab.
fn values_for(arg: &Arg, vocab: &Vocab) -> Vec<String> {
    let enums: Vec<String> = arg
        .get_possible_values()
        .iter()
        .filter(|p| !p.is_hide_set())
        .map(|p| p.get_name().to_string())
        .collect();
    if !enums.is_empty() {
        return enums;
    }
    match arg.get_long() {
        Some("venue") => vocab.venues.clone(),
        Some("chain") => vocab.chains.clone(),
        _ => Vec::new(),
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p scope-bca-cli --lib -- cli::tui::complete`
Expected: PASS (7 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/scope-cli/src/cli/tui/complete.rs crates/scope-cli/src/cli/tui/mod.rs
git commit -m "feat(tui): complete commands, flags, enum values, labels, aliases and venues"
```

---

### Task 3: Tab completion in the input line

**Files:**
- Modify: `crates/scope-cli/src/cli/tui/app.rs`
- Modify: `crates/scope-cli/src/cli/tui/mod.rs` (`TuiState::new` gets the vocab)

**Interfaces:**
- Consumes: `complete`, `Completion` (Task 2), `Vocab` (Task 1).
- Produces: `pub enum Overlay { Menu(Menu), Palette(PaletteState) }` (the `Palette` variant is added in Task 5), `pub struct Menu { pub start: usize, pub candidates: Vec<String>, pub selected: usize }`, `TuiState::new(ctx: SessionContext, journal: Journal, vocab: Vocab) -> TuiState`, and the field `pub overlay: Option<Overlay>`.

Behavior:
- `Tab` with the cursor at the end of the line: no candidate → nothing; one → replace the word and add a space; more than one → fill the longest common prefix and open the menu.
- `Tab` with the cursor not at the end: nothing (Review Focus 1).
- With the menu open: `Tab`/`Down` select the next candidate, `BackTab`/`Up` the previous one, `Enter` accepts the selection (it does not submit the line), `Esc` closes the menu, and any other key closes the menu and is handled as usual.
- The menu shows at most 8 rows above the input, scrolled to keep the selection visible.

- [ ] **Step 1: Change the constructor**

In `app.rs`, add `vocab: Vocab` to `TuiState` and `overlay: Option<Overlay>`. Change `TuiState::new(ctx, journal)` to `TuiState::new(ctx, journal, vocab)`. Update every caller:
- `mod.rs` `run`: `TuiState::new(SessionContext::load(), Journal::in_data_dir(), Vocab::load(config))`
- tests: `TuiState::new(SessionContext::default(), Journal::default(), Vocab::default())` (in `app.rs` `state()`, `test_finish_with_error_shows_cli_error_text_and_logs`, and `mod.rs` `via_tui`, `drive`, `test_library_warning_lands_in_the_running_command_output`).

Run: `cargo test -p scope-bca-cli --lib -- cli::tui`
Expected: PASS (no behavior change yet).

- [ ] **Step 2: Write the failing tests** in `app.rs` tests

```rust
    fn vocab_state() -> TuiState {
        TuiState::new(
            SessionContext::default(),
            Journal::default(),
            Vocab {
                venues: vec!["binance".into(), "bitget".into(), "okx".into()],
                ..Vocab::default()
            },
        )
    }

    #[test]
    fn test_tab_single_candidate_completes_and_adds_space() {
        let mut s = vocab_state();
        type_str(&mut s, "market su");
        s.handle_key(key(KeyCode::Tab));
        assert_eq!(s.input, "market summary ");
        assert!(s.overlay.is_none());
    }

    #[test]
    fn test_tab_many_candidates_fills_prefix_and_opens_menu() {
        let mut s = vocab_state();
        type_str(&mut s, "market summary USDC --venue b");
        s.handle_key(key(KeyCode::Tab));
        assert_eq!(s.input, "market summary USDC --venue bi");
        let Some(Overlay::Menu(m)) = &s.overlay else { panic!("menu expected") };
        assert_eq!(m.candidates, vec!["binance", "bitget"]);
        s.handle_key(key(KeyCode::Tab));
        s.handle_key(key(KeyCode::Enter));
        assert_eq!(s.input, "market summary USDC --venue bitget ");
        assert!(s.overlay.is_none(), "Enter in the menu accepts; it does not submit");
    }

    #[test]
    fn test_tab_mid_line_does_nothing() {
        // Review Focus 1.
        let mut s = vocab_state();
        type_str(&mut s, "market su");
        s.handle_key(key(KeyCode::Left));
        s.handle_key(key(KeyCode::Tab));
        assert_eq!(s.input, "market su");
        assert!(s.overlay.is_none());
    }

    #[test]
    fn test_menu_escape_and_other_keys_close_it() {
        let mut s = vocab_state();
        type_str(&mut s, "ad");
        s.handle_key(key(KeyCode::Tab));
        assert!(matches!(s.overlay, Some(Overlay::Menu(_))));
        s.handle_key(key(KeyCode::Esc));
        assert!(s.overlay.is_none());
        assert_eq!(s.input, "address", "Esc closes the menu, not the line");
        s.handle_key(key(KeyCode::Tab));
        type_str(&mut s, "-");
        assert!(s.overlay.is_none());
        assert_eq!(s.input, "address-");
    }

    #[test]
    fn test_tab_works_while_a_command_runs() {
        // Review Focus 5.
        let _lock = super::super::DIAG_LOCK.blocking_lock();
        let mut s = vocab_state();
        let inv = Invocation { command: venues_list(), route: Route::Pane, ai: false, argv: vec![] };
        s.start("venues list", &inv);
        type_str(&mut s, "market su");
        s.handle_key(key(KeyCode::Tab));
        assert_eq!(s.input, "market summary ");
        assert_eq!(s.handle_key(key(KeyCode::Enter)), Action::None);
        s.finish(Ending::Cancelled);
    }

    #[test]
    fn test_menu_renders_bounded_and_tiny_terminal_is_safe() {
        // Review Focus 4.
        let mut s = TuiState::new(
            SessionContext::default(),
            Journal::default(),
            Vocab { venues: (0..40).map(|i| format!("v{:02}", i)).collect(), ..Vocab::default() },
        );
        type_str(&mut s, "market summary X --venue v");
        s.handle_key(key(KeyCode::Tab));
        for _ in 0..20 {
            s.handle_key(key(KeyCode::Tab));
        }
        let text = screen(&s, 60, 20);
        assert!(text.contains("v20"), "the selection stays visible: {}", text);
        assert!(!text.contains("v00"), "at most 8 rows: {}", text);
        screen(&s, 10, 5);
    }
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test -p scope-bca-cli --lib -- cli::tui::app`
Expected: FAIL to compile, "cannot find type `Overlay`".

- [ ] **Step 4: Implement the menu**

Add to `app.rs`:

```rust
/// The most completion candidates shown at once.
const MENU_ROWS: usize = 8;

/// A floating list over the screen.
pub enum Overlay {
    /// Completion candidates.
    Menu(Menu),
}

/// The open completion menu.
pub struct Menu {
    /// Byte index where the completed word starts.
    pub start: usize,
    /// The candidates.
    pub candidates: Vec<String>,
    /// The selected candidate.
    pub selected: usize,
}

fn common_prefix(items: &[String]) -> String {
    let first = &items[0];
    let mut end = first.len();
    for s in &items[1..] {
        end = end.min(first.bytes().zip(s.bytes()).take_while(|(a, b)| a == b).count());
    }
    while !first.is_char_boundary(end) {
        end -= 1;
    }
    first[..end].to_string()
}
```

At the start of `handle_key`, before the main `match`:

```rust
        if let Some(Overlay::Menu(menu)) = &mut self.overlay {
            match key.code {
                KeyCode::Tab | KeyCode::Down => {
                    menu.selected = (menu.selected + 1) % menu.candidates.len();
                    return Action::None;
                }
                KeyCode::BackTab | KeyCode::Up => {
                    menu.selected = (menu.selected + menu.candidates.len() - 1) % menu.candidates.len();
                    return Action::None;
                }
                KeyCode::Enter => {
                    let (start, pick) = (menu.start, menu.candidates[menu.selected].clone());
                    self.overlay = None;
                    self.replace_word(start, &pick, true);
                    return Action::None;
                }
                KeyCode::Esc => {
                    self.overlay = None;
                    return Action::None;
                }
                _ => self.overlay = None,
            }
        }
```

In the main `match`, add:

```rust
            KeyCode::Tab => self.tab_complete(),
```

And the helpers:

```rust
    fn tab_complete(&mut self) {
        if self.cursor != self.char_len() {
            return;
        }
        let c = super::complete::complete(&self.input, &self.vocab);
        match c.candidates.len() {
            0 => {}
            1 => self.replace_word(c.start, &c.candidates[0], true),
            _ => {
                let prefix = common_prefix(&c.candidates);
                self.replace_word(c.start, &prefix, false);
                self.overlay = Some(Overlay::Menu(Menu {
                    start: c.start,
                    candidates: c.candidates,
                    selected: 0,
                }));
            }
        }
    }

    /// Replaces `input[start..]` with `word`, optionally followed by a space.
    fn replace_word(&mut self, start: usize, word: &str, space: bool) {
        self.input.truncate(start);
        self.input.push_str(word);
        if space {
            self.input.push(' ');
        }
        self.cursor = self.char_len();
    }
```

In `render`, after the input box is drawn:

```rust
        if let Some(Overlay::Menu(m)) = &self.overlay {
            let rows = m.candidates.len().min(MENU_ROWS);
            let first = m.selected.saturating_sub(rows - 1);
            let height = (rows as u16 + 2).min(input.y);
            let width = m.candidates.iter().map(|c| c.chars().count()).max().unwrap_or(0) as u16 + 4;
            let area = ratatui::layout::Rect {
                x: input.x,
                y: input.y.saturating_sub(height),
                width: width.min(input.width),
                height,
            };
            let lines: Vec<Line> = m.candidates[first..first + rows]
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let style = if first + i == m.selected {
                        Style::new().add_modifier(Modifier::REVERSED)
                    } else {
                        Style::new()
                    };
                    Line::styled(c.clone(), style)
                })
                .collect();
            f.render_widget(ratatui::widgets::Clear, area);
            f.render_widget(Paragraph::new(lines).block(Block::bordered()), area);
        }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p scope-bca-cli --lib -- cli::tui`
Expected: PASS, including the 6 new tests.

- [ ] **Step 6: Commit**

```bash
git add crates/scope-cli/src/cli/tui/app.rs crates/scope-cli/src/cli/tui/mod.rs
git commit -m "feat(tui): Tab completes the input line, with a menu for several matches"
```

---

### Task 4: Inline hints

**Files:**
- Create: `crates/scope-cli/src/cli/tui/hint.rs`
- Modify: `crates/scope-cli/src/cli/tui/app.rs` (layout row and render)
- Modify: `crates/scope-cli/src/cli/tui/mod.rs` (`pub mod hint;`)

**Interfaces:**
- Consumes: `SESSION_COMMANDS` (Task 1), `Cli`.
- Produces: `pub struct Hint { pub text: String, pub next: Option<String> }` and `pub fn hint(line: &str) -> Option<Hint>`.

- [ ] **Step 1: Write the failing tests** in `hint.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_usage_of_the_matched_command() {
        let h = hint("market summary").unwrap();
        assert!(h.text.starts_with("scope market summary"), "{}", h.text);
        assert_eq!(h.next.as_deref(), Some("<SYMBOL>"));
    }

    #[test]
    fn test_next_moves_past_typed_positionals() {
        let h = hint("address 0x1 ").unwrap();
        assert_eq!(h.next, None, "address has one positional");
    }

    #[test]
    fn test_session_command_usage() {
        assert_eq!(hint("chain").unwrap().text, "chain [name|auto]");
    }

    #[test]
    fn test_unknown_command_suggests_like_clap() {
        let h = hint("adress 0x1").unwrap();
        assert!(h.text.contains("address"), "{}", h.text);
    }

    #[test]
    fn test_empty_line_has_no_hint() {
        assert!(hint("").is_none());
        assert!(hint("   ").is_none());
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p scope-bca-cli --lib -- cli::tui::hint`
Expected: FAIL to compile, "cannot find function `hint`".

- [ ] **Step 3: Implement `hint.rs`**

```rust
//! # Inline Hints
//!
//! The dim line under the input: the usage of the command being typed, with
//! the next expected argument, or clap's suggestion for an unknown command.

use super::exec::SESSION_COMMANDS;
use crate::cli::Cli;
use clap::{Command, CommandFactory, Parser};

/// One hint line.
#[derive(Debug, PartialEq, Eq)]
pub struct Hint {
    /// The usage text or the suggestion.
    pub text: String,
    /// The next positional argument the command expects, as `<NAME>`.
    pub next: Option<String>,
}

/// The hint for `line`, or `None` for an empty line.
pub fn hint(line: &str) -> Option<Hint> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let first = *words.first()?;
    if let Some((_, usage, _)) = SESSION_COMMANDS.iter().find(|(w, _, _)| *w == first) {
        return Some(Hint { text: usage.to_string(), next: None });
    }

    let mut root = Cli::command();
    root.build();
    let mut path = vec!["scope".to_string()];
    let mut cmd: &Command = &root;
    let mut rest = 0;
    for (i, w) in words.iter().enumerate() {
        match cmd.find_subcommand(w) {
            Some(sub) if !w.starts_with('-') => {
                path.push(sub.get_name().to_string());
                cmd = sub;
                rest = i + 1;
            }
            _ => break,
        }
    }
    if path.len() == 1 {
        let err = Cli::try_parse_from(["scope", first]).err()?;
        let tip = err
            .render()
            .to_string()
            .lines()
            .find(|l| l.contains("similar"))
            .map(|l| l.trim().trim_start_matches("tip:").trim().to_string())
            .unwrap_or_else(|| format!("unknown command `{}`; type `help`", first));
        return Some(Hint { text: tip, next: None });
    }

    let usage = cmd
        .clone()
        .bin_name(path.join(" "))
        .render_usage()
        .to_string();
    let text = usage.trim_start_matches("Usage:").trim().to_string();
    let typed = words[rest..].iter().filter(|w| !w.starts_with('-')).count();
    let next = cmd
        .get_positionals()
        .nth(typed)
        .map(|a| {
            let name = a
                .get_value_names()
                .and_then(|v| v.first())
                .map(|n| n.to_string())
                .unwrap_or_else(|| a.get_id().to_string().to_uppercase());
            format!("<{}>", name)
        });
    Some(Hint { text, next })
}
```

Note: the `typed` count treats a flag's value as a positional (`--venue binance`). Keep it: the hint then shows no `next`, which never misleads. A later change can make it exact.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p scope-bca-cli --lib -- cli::tui::hint`
Expected: PASS (5 tests). `SummaryArgs::pair` has `value_name = "SYMBOL"` (`market.rs`), so `next` is `<SYMBOL>`.

- [ ] **Step 5: Render the hint row**

In `app.rs` `render`, change the layout to four rows:

```rust
        let [pane, input, hint_row, status] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(f.area());
```

and after the input box:

```rust
        if let Some(h) = super::hint::hint(&self.input) {
            let mut spans = vec![Span::styled(format!(" {}", h.text), Style::new().fg(Color::DarkGray))];
            if let Some(next) = h.next {
                spans.push(Span::styled(format!("  next: {}", next), Style::new().fg(Color::Yellow)));
            }
            f.render_widget(Paragraph::new(Line::from(spans)), hint_row);
        }
```

Update `test_newest_line_is_on_the_last_pane_row`: the pane now ends one row higher. The pane rows are 0..=9 (border at 0 and 9), so assert `rows[8]` holds `line 39` and `rows[9]` starts with `└`. Add:

```rust
    #[test]
    fn test_hint_row_shows_usage_while_typing() {
        let mut s = state();
        type_str(&mut s, "market summary");
        let text = screen(&s, 100, 15);
        assert!(text.contains("scope market summary"), "{}", text);
        assert!(text.contains("next: <SYMBOL>"), "{}", text);
    }
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p scope-bca-cli --lib -- cli::tui`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/scope-cli/src/cli/tui/hint.rs crates/scope-cli/src/cli/tui/app.rs crates/scope-cli/src/cli/tui/mod.rs
git commit -m "feat(tui): show the usage and the next argument under the input line"
```

---

### Task 5: Command palette

**Files:**
- Create: `crates/scope-cli/src/cli/tui/palette.rs`
- Modify: `crates/scope-cli/src/cli/tui/app.rs` (`Overlay::Palette`, keys, render)
- Modify: `crates/scope-cli/src/cli/tui/mod.rs` (`pub mod palette;`)

**Interfaces:**
- Consumes: `SESSION_COMMANDS` (Task 1), `Cli`, `TuiState::history`.
- Produces: `pub struct PaletteItem { pub label: String, pub description: String, pub fill: String }`, `pub fn items(history: &[String]) -> Vec<PaletteItem>`, `pub fn score(query: &str, text: &str) -> Option<i32>`, `pub struct PaletteState { pub query: String, pub items: Vec<PaletteItem>, pub selected: usize }` with `PaletteState::new(items)`, `matches(&self) -> Vec<usize>`.

- [ ] **Step 1: Write the failing tests** in `palette.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_every_visible_leaf_command_is_an_item() {
        // The same guard as the parity test: a new command shows up here.
        let labels: Vec<String> = items(&[]).into_iter().map(|i| i.label).collect();
        for sub in Cli::command().get_subcommands().filter(|s| !s.is_hide_set()) {
            if sub.has_subcommands() {
                for leaf in sub.get_subcommands().filter(|s| !s.is_hide_set()) {
                    let l = format!("{} {}", sub.get_name(), leaf.get_name());
                    assert!(labels.contains(&l), "{}", l);
                }
            } else {
                assert!(labels.contains(&sub.get_name().to_string()), "{}", sub.get_name());
            }
        }
        assert!(labels.contains(&"chain".to_string()));
    }

    #[test]
    fn test_history_comes_first_newest_first_and_unique() {
        let h = vec!["venues list".to_string(), "tx 0x1".to_string(), "venues list".to_string()];
        let it = items(&h);
        assert_eq!(it[0].label, "venues list");
        assert_eq!(it[1].label, "tx 0x1");
        assert_eq!(it.iter().filter(|i| i.label == "venues list").count(), 2, "history + command");
    }

    #[test]
    fn test_score_prefers_prefix_and_consecutive_letters() {
        assert!(score("msum", "market summary").is_some());
        assert!(score("xyz", "market summary").is_none());
        assert!(score("mark", "market summary") > score("mark", "compliance risk mark"));
        assert!(score("addr", "address") > score("addr", "address-book add"));
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn test_matches_filters_and_ranks() {
        let mut p = PaletteState::new(items(&[]));
        p.query = "msum".into();
        let m = p.matches();
        assert_eq!(p.items[m[0]].label, "market summary");
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p scope-bca-cli --lib -- cli::tui::palette`
Expected: FAIL to compile, "cannot find function `items`".

- [ ] **Step 3: Implement `palette.rs`**

```rust
//! # Command Palette
//!
//! `Ctrl-K` opens a fuzzy list of every command (from the clap definition),
//! the session commands, and recent history. `Enter` puts the selection in
//! the input line; it does not run it.

use super::exec::SESSION_COMMANDS;
use crate::cli::Cli;
use clap::CommandFactory;

/// The most history entries offered.
const HISTORY_ITEMS: usize = 50;

/// One palette entry.
#[derive(Debug, Clone)]
pub struct PaletteItem {
    /// What the list shows and the query matches.
    pub label: String,
    /// The one-line description.
    pub description: String,
    /// What goes in the input line.
    pub fill: String,
}

/// All palette entries: history (newest first), then CLI commands, then
/// session commands.
pub fn items(history: &[String]) -> Vec<PaletteItem> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in history.iter().rev() {
        if seen.insert(line.clone()) && seen.len() <= HISTORY_ITEMS {
            out.push(PaletteItem {
                label: line.clone(),
                description: "history".into(),
                fill: line.clone(),
            });
        }
    }
    for sub in Cli::command().get_subcommands().filter(|s| !s.is_hide_set()) {
        let leaves: Vec<(String, String)> = if sub.has_subcommands() {
            sub.get_subcommands()
                .filter(|s| !s.is_hide_set())
                .map(|l| {
                    let about = l.get_about().map(|a| a.to_string()).unwrap_or_default();
                    (format!("{} {}", sub.get_name(), l.get_name()), about)
                })
                .collect()
        } else {
            let about = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
            vec![(sub.get_name().to_string(), about)]
        };
        for (label, description) in leaves {
            out.push(PaletteItem { fill: format!("{} ", label), label, description });
        }
    }
    for (word, usage, what) in SESSION_COMMANDS {
        out.push(PaletteItem {
            label: word.to_string(),
            description: format!("{} — {}", usage, what),
            fill: format!("{} ", word),
        });
    }
    out
}

/// Fuzzy score of `query` against `text`, case-insensitive. `None` when the
/// letters of `query` are not all in `text` in order. Higher is better.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut total = 0;
    let mut pos = 0;
    let mut prev: Option<usize> = None;
    for q in query.to_lowercase().chars() {
        let found = (pos..text.len()).find(|&i| text[i] == q)?;
        if found == 0 {
            total += 20;
        } else if matches!(text[found - 1], ' ' | '-') {
            total += 5;
        }
        match prev {
            Some(p) if p + 1 == found => total += 10,
            Some(p) => total -= (found - p - 1) as i32,
            None => total -= found as i32,
        }
        prev = Some(found);
        pos = found + 1;
    }
    Some(total - (text.len() as i32 / 8))
}

/// The open palette.
pub struct PaletteState {
    /// The typed query.
    pub query: String,
    /// All entries.
    pub items: Vec<PaletteItem>,
    /// Index into `matches()`.
    pub selected: usize,
}

impl PaletteState {
    /// A palette over `items` with an empty query.
    pub fn new(items: Vec<PaletteItem>) -> Self {
        Self { query: String::new(), items, selected: 0 }
    }

    /// Indexes of the matching items, best first. An empty query keeps the
    /// original order.
    pub fn matches(&self) -> Vec<usize> {
        let mut scored: Vec<(i32, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| score(&self.query, &it.label).map(|s| (s, i)))
            .collect();
        if !self.query.is_empty() {
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        scored.into_iter().map(|(_, i)| i).collect()
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p scope-bca-cli --lib -- cli::tui::palette`
Expected: PASS (4 tests). If a ranking assertion fails, change the weights in `score`, not the test: the tests state the wanted order.

- [ ] **Step 5: Write the failing app tests** in `app.rs`

```rust
    fn ctrl_k() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL)
    }

    #[test]
    fn test_ctrl_k_opens_palette_and_enter_fills_input_without_running() {
        let mut s = state();
        s.handle_key(ctrl_k());
        assert!(matches!(s.overlay, Some(Overlay::Palette(_))));
        type_str(&mut s, "msum");
        assert_eq!(s.handle_key(key(KeyCode::Enter)), Action::None);
        assert_eq!(s.input, "market summary ");
        assert!(s.overlay.is_none());
    }

    #[test]
    fn test_palette_escape_keeps_the_input() {
        let mut s = state();
        type_str(&mut s, "draft");
        s.handle_key(ctrl_k());
        type_str(&mut s, "zzz");
        s.handle_key(key(KeyCode::Esc));
        assert!(s.overlay.is_none());
        assert_eq!(s.input, "draft");
    }

    #[test]
    fn test_palette_with_long_history_renders_bounded() {
        // Review Focus 4.
        let mut s = state();
        for i in 0..60 {
            type_str(&mut s, &format!("venues list {}", i));
            s.handle_key(key(KeyCode::Enter));
        }
        s.handle_key(ctrl_k());
        for _ in 0..30 {
            s.handle_key(key(KeyCode::Down));
        }
        screen(&s, 80, 20);
        screen(&s, 10, 5);
    }
```

- [ ] **Step 6: Implement the palette in `app.rs`**

Add the variant `Palette(super::palette::PaletteState)` to `Overlay`. Before the menu block in `handle_key`:

```rust
        if let Some(Overlay::Palette(p)) = &mut self.overlay {
            let count = p.matches().len();
            match key.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Enter => {
                    let fill = p.matches().get(p.selected).map(|&i| p.items[i].fill.clone());
                    self.overlay = None;
                    if let Some(fill) = fill {
                        self.set_input(fill);
                    }
                }
                KeyCode::Down | KeyCode::Tab if count > 0 => p.selected = (p.selected + 1) % count,
                KeyCode::Up | KeyCode::BackTab if count > 0 => p.selected = (p.selected + count - 1) % count,
                KeyCode::Backspace => {
                    p.query.pop();
                    p.selected = 0;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    p.query.push(c);
                    p.selected = 0;
                }
                _ => {}
            }
            return Action::None;
        }
```

In the main `match`:

```rust
            KeyCode::Char('k') if ctrl => {
                self.overlay = Some(Overlay::Palette(super::palette::PaletteState::new(
                    super::palette::items(&self.history),
                )));
            }
```

In `render`, after the menu block:

```rust
        if let Some(Overlay::Palette(p)) = &self.overlay {
            let area = f.area();
            let width = area.width.saturating_sub(4).min(80);
            let height = area.height.saturating_sub(4).min(16);
            let rect = ratatui::layout::Rect {
                x: area.x + (area.width.saturating_sub(width)) / 2,
                y: area.y + 1,
                width,
                height,
            };
            let rows = height.saturating_sub(3) as usize;
            let matches = p.matches();
            let first = p.selected.saturating_sub(rows.saturating_sub(1));
            let mut lines = vec![Line::from(format!("> {}", p.query))];
            for (n, &i) in matches.iter().enumerate().skip(first).take(rows) {
                let it = &p.items[i];
                let style = if n == p.selected {
                    Style::new().add_modifier(Modifier::REVERSED)
                } else {
                    Style::new()
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("{:<28}", it.label), style),
                    Span::styled(format!(" {}", it.description), Style::new().fg(Color::DarkGray)),
                ]));
            }
            f.render_widget(ratatui::widgets::Clear, rect);
            f.render_widget(
                Paragraph::new(lines).block(Block::bordered().title(" Commands (Ctrl-K) ")),
                rect,
            );
        }
```

- [ ] **Step 7: Run all gates**

Run:
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```
Expected: all pass.

- [ ] **Step 8: Real-terminal check**

Run the TUI in tmux with an isolated `HOME`, the same way as phase 2:

```bash
S=$(mktemp -d); echo "{}" > $S/config.yaml
tmux new-session -d -s scopetui -x 110 -y 32 "HOME=$S target/debug/scope --config $S/config.yaml tui --no-banner"
tmux send-keys -t scopetui 'market su' Tab
tmux capture-pane -t scopetui -p | tail -6
tmux send-keys -t scopetui C-k 'vlist' Enter
tmux capture-pane -t scopetui -p | tail -6
tmux send-keys -t scopetui C-d
```
Expected: the input shows `market summary ` with the hint `next: <SYMBOL>`. After `Ctrl-K` and `vlist`, the input shows `venues list `.

- [ ] **Step 9: Commit, push, open the PR**

```bash
git add crates/scope-cli/src/cli/tui/palette.rs crates/scope-cli/src/cli/tui/app.rs crates/scope-cli/src/cli/tui/mod.rs
git commit -m "feat(tui): add the Ctrl-K command palette"
GH_TOKEN=$(gh auth token --user robot-accomplice) git push -u origin <branch>
```

Commit the updated `.coverage-last` that the hook writes. Open the PR into `develop` with the robot-accomplice token.
