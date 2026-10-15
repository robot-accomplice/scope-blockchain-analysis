# Token disambiguation: one resolver for every command

Status: design for review. Date: 2026-10-15. Depends on: #105 (`~/.scope-bca`).

## 1. Goal

A symbol can name more than one token. For example, `USDN` is a different token on different chains, and a venue can list one `USDN` on several networks. Scope must never report on a token that the user did not mean.

When a symbol is ambiguous, scope shows the candidates and asks the user to choose one. Then it asks whether to save the choice. After that, `USDNUSDT` always means the same contract on that chain.

## 2. Agreed behavior (Jon, 2026-10-15)

1. All commands that take a token symbol use this, and so does the TUI.
2. More than one candidate: show the list and ask. One candidate: use it without a question, and show its contract.
3. After a choice: ask to save it. A saved binding is symbol + chain → contract.
4. A saved binding that no longer matches the venue's contract: stop with an error that shows both contracts.
5. No keyboard (pipe, `--format json`, scripts, CI): stop with an error that lists the candidates and tells how to choose. Never choose silently.
6. Storage: `tokens.yaml` in the scope data folder (`~/.scope-bca/data/tokens.yaml`).
7. On a CEX venue, a candidate is one network that the venue lists for the symbol, with its contract.

## 3. Current state

- `crawl::resolve_token_input` and `monitor::resolve_token_address` are two copies of the same logic: an address, then a saved alias, then a DEX search, then a numbered list (`token_pick::pick_token`), then the question "Save this token for future use?". `token-health`, `insights` and `market` on a DEX venue reach the first copy through `crawl::fetch_analytics_for_input`.
- `market` on a CEX venue sends the symbol to the venue without a check. Since #101 the report shows the venue's contract (the `assets` capability), but nothing compares it with the token the user means.
- In the TUI, the list question runs outside the TUI (suspend mode).
- `TokenAliases` (`scope-core/src/tokens.rs`) maps symbol → chain → `TokenInfo { address, symbol, name, chain, last_used }`.

## 4. Design

### 4.1 Resolver

A new module `crates/scope-cli/src/cli/resolve.rs` has one entry point:

```rust
pub async fn resolve(
    input: &str,
    ctx: &ResolveContext,          // chain filter, venue (DEX or CEX), --token
    sources: &dyn CandidateSource, // DEX search or venue coin list
    picker: &dyn Picker,
    store: &mut dyn BindingStore,  // TokenAliases behind a trait
    out: &Output,
) -> Result<Resolved>
```

`Resolved { symbol, chain, address, name, source: Source }`, where `Source` is one of `Flag`, `Address`, `Binding`, `Single` or `Picked { saved: bool }`.

The steps, in this order:

1. `--token <contract>` is set: use it. It is the only source of truth, and nothing else runs.
2. The input is an address: use it.
3. A saved binding for (symbol, chain) exists:
   - DEX venue: use it.
   - CEX venue with the `assets` capability: get the venue's contract for that symbol and network. If it is different, stop with `ScopeError::TokenMismatch`. The message shows the saved contract, the venue's contract and how to fix it (`--token`, or remove the binding with `tokens remove <SYMBOL>` in the TUI).
   - CEX venue without `assets`: use the binding, and note that the venue cannot be checked.
4. Get the candidates from the source:
   - DEX: search by symbol, filtered to the chain when one is set. Exact symbol matches only (case-insensitive).
   - CEX: the venue's coin list (#101). Each network of the symbol that has a contract is one candidate. A native coin (no contract) is one candidate with no contract.
5. 0 candidates: stop with "not found" and the source that was searched.
   1 candidate: use it, and say which one (symbol, chain, contract).
   2 or more: call the picker.
6. After a pick: ask "Save so <SYMBOL> always means <contract> on <chain>? [y/N]". `--yes` answers no without a question. `--save` (crawl has it today) answers yes. Save means `store.add(...)`.

`crawl --yes` no longer selects the first match. It only skips the save question. With `--yes`, an ambiguous symbol stops in the same way as when there is no keyboard (`AmbiguousToken`, which lists the candidates and the `--token` hint). This is a breaking change for scripts that relied on the first match (Jon, 2026-10-15). The `--yes` help text and the CHANGELOG say so.

`crawl`, `monitor`, `token-health`, `insights` and `market summary|ohlc|trades` (DEX and CEX) call `resolve`. The two copies in `crawl` and `monitor` are removed.

On a CEX, `market` then uses the venue symbol as before. The resolver only proves which token the symbol is.

### 4.2 Picker

```rust
#[async_trait]
pub trait Picker {
    /// Index of the chosen candidate, or Err(NoChoice) when the user cancels.
    async fn pick(&self, prompt: &str, candidates: &[Candidate]) -> Result<usize>;
    /// The answer to a yes/no question.
    async fn confirm(&self, prompt: &str) -> Result<bool>;
}
```

There are three versions:

1. `TerminalPicker`: today's numbered list (`token_pick`), with chain, contract, liquidity and age for each candidate.
2. `TuiPicker`: a list overlay in the TUI (the same overlay system as the command palette). The command does not run outside the TUI for this question.
3. `NoTerminalPicker`: used when stdin or stdout is not a terminal, or when `--format json` is set. `pick` returns `ScopeError::AmbiguousToken`, which lists each candidate (chain, contract, name) and says: "Pass --token <contract>, or run the command in a terminal to choose and save one." `confirm` returns `false`.

### 4.3 Storage

`~/.scope-bca/data/tokens.yaml` keeps its format. `TokenInfo` gets one optional field, `venue: Option<String>`: the venue that the binding was checked against when it was saved. Files without the field load as before. `BindingStore` is a trait over `TokenAliases`, so tests use a store in memory.

### 4.4 RCA trail

Each resolution writes one line to `~/.scope-bca/state/resolutions.jsonl`:

```json
{"at": "...", "input": "USDNUSDT", "chain": "ethereum", "venue": "weex",
 "candidates": [{"chain": "ETH", "address": "0xeC74…"}],
 "chosen": "0xeC74…", "source": "picked", "saved": true}
```

A failure (mismatch, not found, ambiguous with no keyboard) writes a line with `"error"`. A log-write failure is a warning and does not stop the command.

### 4.5 Error types

`ScopeError` gets three variants, so a script can tell them apart by exit code and message:

- `TokenNotFound { input, searched }`
- `AmbiguousToken { input, candidates }`
- `TokenMismatch { symbol, chain, saved, venue_contract, venue }`

## 5. Tests

1. The resolver, with a fake source, picker and store, one test for each step in 4.1:
   - `--token` wins over a binding.
   - An address passes through.
   - A binding is used.
   - A binding that the CEX venue contradicts stops with both contracts.
   - A binding on a CEX without `assets` is used, with a note.
   - 0 candidates stops.
   - 1 candidate gets no question.
   - 2 candidates call the picker.
   - Save yes, save no, `--yes` and `--save`.
2. `NoTerminalPicker`: the error lists every candidate and the `--token` hint.
3. `TuiPicker`: an overlay test with the TUI test backend; Enter picks, Esc cancels.
4. The commands: a run of `crawl` and `market summary --venue <mock CEX>` through the resolver, with a binding mismatch that stops `market`.
5. `resolutions.jsonl`: one line for each resolution, with the right `source`.
6. Old `tokens.yaml` (no `venue` field) loads.

## 6. Out of scope

- Look-alike symbols (`USDN2`, `USDNX`). Only exact symbols are candidates (Jon's choice: network + contract).
- A CLI `tokens` command. Binding management stays in the TUI (`tokens add|remove|list`). If scripts need it, a CLI `tokens` command can come later.
- #99 (TUI config reload). It comes next.
