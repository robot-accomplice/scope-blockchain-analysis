# Token disambiguation, plan 2 of 2: the picker in the TUI

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A command that runs in the TUI pane asks which token a symbol means in a list inside the TUI. Today it stops with `AmbiguousToken`.

**Architecture:** The pane's `Output` gets an *ask slot* (`Asker`). `TuiPicker`, a new `Picker`, puts a question there: a prompt, the option lines and a one-shot reply. Then it awaits the reply. The pane command's future runs inside the TUI event loop's `select!`, so the loop keeps handling keys while the command waits. On each pass the loop moves a waiting question into a new `Overlay::Choice`. `↑`/`↓` move, `Enter` replies with the index, and `Esc` replies with none, which makes the picker fail with "cancelled". The save question uses the same list with the options `No` and `Yes`.

**Tech Stack:** Rust, ratatui (`TestBackend` for tests), `tokio::sync::oneshot`.

**Spec:** `docs/design/2026-10-15-token-disambiguation-design.md` §4.2 (`TuiPicker`).

## Global Constraints

- In the pane, the picker never reads stdin. `prompts: false` stays on the pane output, and `picker_for` chooses `TuiPicker` only when the output has an ask slot.
- One question at a time. A second question that comes while one is open waits in the slot.
- `Esc` on the choice list answers the question (cancelled). It does not cancel the command. A second `Esc` with no list open cancels the command, as it does today.
- Plan 1's rules do not change: `--format json` and `--yes` still never ask. They stop with `AmbiguousToken`.
- Commits: robot-accomplice <robot@accomplice.ch>. Branch from `develop`. Compare a snapshot of the real scope folders before and after each full test run.

## Review Focus

1. The command is cancelled (Ctrl-C) while the list is open. Expected: the list closes, and no reply is sent to a dropped receiver (no panic). Pinned in Task 2, `test_cancelling_the_command_closes_the_choice`.
2. A question has more options than the list shows (20 candidates). Expected: the list scrolls with the selection, as the completion menu does. Pinned in Task 2, `test_a_long_choice_scrolls_with_the_selection`.
3. The terminal is too small for the list. Expected: the list is cut to the space there is and does not panic. Pinned in Task 2, `test_the_choice_draws_in_a_tiny_terminal`.

---

### Task 1: The ask slot and `TuiPicker`

**Files:**
- Modify: `crates/scope-cli/src/cli/output.rs` (`Asker`, `Ask`, `Output::with_asker`, `Output::asker`)
- Modify: `crates/scope-cli/src/cli/resolve.rs` (`TuiPicker`, `picker_for`)

**Interfaces:**
- Produces:
  - `pub struct Ask { pub prompt: String, pub options: Vec<String>, pub reply: tokio::sync::oneshot::Sender<Option<usize>> }`
  - `pub type Asker = Arc<Mutex<Option<Ask>>>`
  - `Output::with_asker(self, asker: Asker) -> Self`; `Output::asker(&self) -> Option<Asker>`
  - `pub struct TuiPicker { pub asker: Asker }`, with `impl Picker`
  - `picker_for` returns `TuiPicker` when `out.asker()` is `Some`. The checks for `json` and `--yes` stay first.

- [ ] **Step 1: Failing tests** (in `resolve.rs` `mod tests`)

```rust
    /// Answers the next question in `asker` with `answer`.
    fn answer(asker: &crate::cli::output::Asker, answer: Option<usize>) {
        let asker = asker.clone();
        tokio::spawn(async move {
            loop {
                if let Some(ask) = asker.lock().unwrap().take() {
                    let _ = ask.reply.send(answer);
                    return;
                }
                tokio::task::yield_now().await;
            }
        });
    }

    #[tokio::test]
    async fn test_the_tui_picker_asks_through_the_slot() {
        let asker = crate::cli::output::Asker::default();
        let p = TuiPicker { asker: asker.clone() };
        answer(&asker, Some(1));
        assert_eq!(p.pick("USDN", &[c("ethereum", "0xA"), c("bsc", "0xB")]).await.unwrap(), 1);
        answer(&asker, Some(1));
        assert!(p.confirm("Save so …? [y/N]: ").await.unwrap(), "option 1 is Yes");
    }

    #[tokio::test]
    async fn test_escape_on_the_choice_is_a_cancel() {
        let asker = crate::cli::output::Asker::default();
        let p = TuiPicker { asker: asker.clone() };
        answer(&asker, None);
        let err = p.pick("USDN", &[c("ethereum", "0xA"), c("bsc", "0xB")]).await.unwrap_err();
        assert!(err.to_string().contains("cancelled"), "{err}");
        answer(&asker, None);
        assert!(!p.confirm("Save?").await.unwrap(), "Esc on the save question is No");
    }

    #[tokio::test]
    async fn test_a_pane_output_with_an_asker_gets_the_tui_picker() {
        let (o, _cap) = Output::capture_merged();
        let asker = crate::cli::output::Asker::default();
        let o = o.with_asker(asker.clone());
        let p = picker_for(&o, false);
        answer(&asker, Some(0));
        assert_eq!(p.pick("USDN", &[c("ethereum", "0xA"), c("bsc", "0xB")]).await.unwrap(), 0);
        // JSON still never asks.
        let p = picker_for(&o, true);
        assert!(p.pick("USDN", &[c("ethereum", "0xA"), c("bsc", "0xB")]).await.is_err());
    }
```

Run `cargo test -q -p scope-bca-cli --lib resolve::`. Expected: compile errors.

- [ ] **Step 2: Implement**

`output.rs`: add the `Ask` struct and the `Asker` type. Add the field `asker: Option<Asker>` to `Output`, set to `None` in every constructor and copied in `teed`. Add:

```rust
    /// This output with a slot through which a command asks the TUI a
    /// question (the pane's list).
    pub fn with_asker(mut self, asker: Asker) -> Self {
        self.asker = Some(asker);
        self
    }

    /// The ask slot, when a TUI shows this output.
    pub fn asker(&self) -> Option<Asker> {
        self.asker.clone()
    }
```

`resolve.rs`:

```rust
/// Asks in the TUI: puts the question in the pane's ask slot and waits for
/// the list's answer.
pub struct TuiPicker {
    pub asker: crate::cli::output::Asker,
}

impl TuiPicker {
    async fn ask(&self, prompt: String, options: Vec<String>) -> Option<usize> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        let ask = crate::cli::output::Ask { prompt, options, reply };
        // One question at a time: wait for the slot to be free.
        let mut ask = Some(ask);
        while let Some(a) = ask.take() {
            let mut slot = self.asker.lock().unwrap_or_else(|e| e.into_inner());
            if slot.is_none() {
                *slot = Some(a);
            } else {
                ask = Some(a);
                drop(slot);
                tokio::task::yield_now().await;
            }
        }
        answer.await.ok().flatten()
    }
}

#[async_trait::async_trait]
impl Picker for TuiPicker {
    async fn pick(&self, input: &str, cs: &[Candidate]) -> Result<usize> {
        let prompt = format!("'{input}' matches more than one token. Choose one:");
        self.ask(prompt, cs.iter().map(candidate_line).collect())
            .await
            .ok_or_else(|| ScopeError::Other(format!("Choosing a token for '{input}' was cancelled.")))
    }

    async fn confirm(&self, question: &str) -> Result<bool> {
        let options = vec!["No".to_string(), "Yes".to_string()];
        Ok(self.ask(question.trim_end_matches(&[' ', ':'][..]).to_string(), options).await == Some(1))
    }
}
```

In `picker_for`, before the `prompts_allowed` check and after `json`:

```rust
    if json {
        return Box::new(NoTerminalPicker);
    }
    if let Some(asker) = out.asker() {
        return Box::new(TuiPicker { asker });
    }
```

Run the tests: they pass. Commit: `feat(resolve): a picker that asks through the TUI`.

---

### Task 2: The choice list in the TUI

**Files:**
- Modify: `crates/scope-cli/src/cli/tui/app.rs` (`Overlay::Choice`, `ChoiceState`, `TuiState::start` sets the asker, `poll_ask`, key handling, drawing, `EscAction`)
- Modify: `crates/scope-cli/src/cli/tui/mod.rs` (call `state.poll_ask()` once per event-loop pass)

**Interfaces:**
- Consumes: Task 1 `Ask`, `Asker`, `Output::with_asker`.
- Produces: `pub struct ChoiceState { prompt: String, options: Vec<String>, selected: usize, reply: Option<oneshot::Sender<Option<usize>>> }`. `TuiState::poll_ask(&mut self)` moves a waiting `Ask` into `Overlay::Choice` when no choice is open.

- [ ] **Step 1: Failing tests** (in `app.rs` tests, following the existing `TuiState` key-handling tests)

```rust
    fn asked(s: &mut TuiState, options: &[&str]) -> tokio::sync::oneshot::Receiver<Option<usize>> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        *s.asker.lock().unwrap() = Some(crate::cli::output::Ask {
            prompt: "Choose one:".into(),
            options: options.iter().map(|o| o.to_string()).collect(),
            reply,
        });
        s.poll_ask();
        rx
    }

    #[test]
    fn test_the_choice_list_answers_with_the_selected_row() {
        let mut s = state();
        let mut rx = asked(&mut s, &["ethereum 0xA USDN", "bsc 0xB USDN"]);
        assert!(matches!(s.overlay, Some(Overlay::Choice(_))));
        s.on_key(press(KeyCode::Down));
        s.on_key(press(KeyCode::Enter));
        assert_eq!(rx.try_recv().unwrap(), Some(1));
        assert!(s.overlay.is_none());
    }

    #[test]
    fn test_escape_answers_none_and_does_not_cancel_the_command() {
        let mut s = state();
        let mut rx = asked(&mut s, &["a", "b"]);
        assert!(matches!(s.on_key(press(KeyCode::Esc)), Action::None));
        assert_eq!(rx.try_recv().unwrap(), None);
    }

    #[test]
    fn test_cancelling_the_command_closes_the_choice() {
        // Review focus 1.
        let mut s = state();
        let rx = asked(&mut s, &["a", "b"]);
        drop(rx); // the command future is gone
        s.finish(Ending::Cancelled);
        assert!(s.overlay.is_none());
    }

    #[test]
    fn test_a_long_choice_scrolls_with_the_selection() {
        // Review focus 2: 20 options; the 15th must be on screen.
        let mut s = state();
        let options: Vec<String> = (1..=20).map(|i| format!("chain{i} 0x{i} T")).collect();
        let refs: Vec<&str> = options.iter().map(String::as_str).collect();
        let _rx = asked(&mut s, &refs);
        for _ in 0..14 {
            s.on_key(press(KeyCode::Down));
        }
        let screen = render(&s, 100, 30); // the existing TestBackend helper
        assert!(screen.contains("chain15 0x15 T"), "{screen}");
    }

    #[test]
    fn test_the_choice_draws_in_a_tiny_terminal() {
        // Review focus 3.
        let mut s = state();
        let _rx = asked(&mut s, &["a", "b", "c"]);
        let _ = render(&s, 20, 6);
    }
```

Use the module's existing helpers for a test state, a key press and a TestBackend render (`grep -n 'fn state()\|fn press\|fn render' crates/scope-cli/src/cli/tui/app.rs`). Run them: compile errors.

- [ ] **Step 2: Implement**

1. `Overlay::Choice(ChoiceState)`. `EscAction`: an open choice comes first, as `AnswerChoice`.
2. `TuiState` gets `asker: Asker` (created in `new`). `start` returns `Output::capture_merged()` followed by `.with_asker(self.asker.clone())`.
3. `poll_ask`: if `self.overlay` is not `Choice` and `self.asker` holds an `Ask`, take it and open `Overlay::Choice` with `selected: 0`.
4. Keys while a choice is open:
   - `Up`/`Down` move the selection, with wrap.
   - `Enter` sends `Some(selected)`.
   - `Esc` sends `None`.
   - Both close the list.
   - Other keys do nothing.
5. `finish(...)` (the command ended or was cancelled) closes an open choice without sending, because the receiver is gone, and clears the slot.
6. Drawing: the same layout as `Overlay::Menu`, but above the input line. The prompt is the block title, there is one row per option, and the rows scroll so that `selected` is visible (copy the menu's `first` rule). Height and width are capped to the frame.
7. `mod.rs` event loop: call `state.poll_ask()` at the top of each pass, before drawing. The 100 ms live tick wakes the loop while a command runs, so a question shows within one tick.

Run: `cargo test -q -p scope-bca-cli --lib tui::`. Pass. Commit: `feat(tui): commands ask which token in a list inside the TUI`.

---

### Task 3: Harness scenario, docs, end to end

- [ ] **Step 1:** Add `tests/tui/scenarios/pane-token-choice.yaml`. It runs `token-health USDC` in the pane against the existing DexScreener fixtures for `USDC`: wait for the choice title `matches more than one token`, press `Down`, `Enter`, then `Enter` on `No` for the save question, and wait for the token-health report. If the `USDC` search fixture from `outside-crawl` cannot be reused for `token-health` (different requests), record one with `just tui-record pane-token-choice` and review the fixture diff.
- [ ] **Step 2:** README "Choosing a token": replace "or a command in the TUI pane" in the no-terminal list with "In the TUI, the choice is a list: ↑/↓ and Enter; Esc cancels." CHANGELOG `[Unreleased]` → Added: "In the TUI, a command asks which token in a list."
- [ ] **Step 3:** Gates: fmt, clippy, `cargo test --workspace`, `just tui-test`, and the real-folder snapshot check. Then push and open a PR into `develop`.
