# Token disambiguation, plan 1 of 2: the resolver and the commands

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every command that takes a token symbol resolves it through one resolver. When the symbol is ambiguous, the resolver asks the user to choose in a terminal and stops with the candidates when there is no terminal. It offers to save the choice as a binding, and it checks saved bindings against a CEX venue's coin list.

**Architecture:** A new `crates/scope-cli/src/cli/resolve.rs` holds the steps from spec §4.1. It works through three traits (`CandidateSource`, `Picker` and `BindingStore`), so the tests use fakes. The DEX search and the venue coin list are the two `CandidateSource`s. `TerminalPicker` and `NoTerminalPicker` are the two pickers in this plan; plan 2 adds the TUI overlay picker. `crawl`, `monitor`, `token-health`, `insights` and `market` call the resolver. The two old copies of the logic are removed.

**Tech Stack:** Rust 2024, `async-trait`, `serde`, `serde_json`, `chrono`. Test fakes are in-process; there is no network in tests (the #39 loopback guard).

**Spec:** `docs/design/2026-10-15-token-disambiguation-design.md`

## Global Constraints

- Steps in this order: `--token` → address → saved binding (checked on a CEX that has `assets`) → candidates → 0 stops / 1 is used / 2 or more go to the picker → save question.
- Only exact symbols are candidates (case-insensitive). On a CEX, one candidate is one network that the venue lists for the symbol.
- Never choose silently. With no terminal, `--format json` or `--yes`, an ambiguous symbol stops with `ScopeError::AmbiguousToken`. That error lists every candidate and the hint `Pass --token <contract>`.
- `--yes` skips only the save question (it answers no). `--save` answers yes.
- A binding that a CEX venue contradicts stops with `ScopeError::TokenMismatch`, which shows both contracts.
- Bindings are stored in `scope::paths::data_dir()/tokens.yaml` (`TokenAliases`). The new field `venue: Option<String>` has a serde default.
- RCA: one JSON line per resolution in `scope::paths::state_dir()/resolutions.jsonl`. A failure to write the log is a `tracing::warn!` and does not stop the command.
- Inside the TUI, until plan 2: a command that runs in the TUI pane (`Output::is_tui()`) uses `NoTerminalPicker`. A command that runs outside the TUI (suspend mode, for example `crawl`) uses `TerminalPicker`.
- Commits: robot-accomplice <robot@accomplice.ch>. Branch from `develop`. Never run tests of a checkout older than #98. Compare a snapshot of the real scope folders before and after each full test run.

## Review Focus

1. A DEX search returns tokens whose symbol only contains the input (`USDNX` for `USDN`). Expected: they are not candidates. Pinned in Task 2, `test_only_exact_symbols_are_candidates`.
2. A binding saved with chain `ethereum` meets a venue that names the network `ERC20` or `ETH`. Expected: they are the same chain, and the contracts are compared. Pinned in Task 1, `test_venue_network_codes_map_to_scope_chains`, and in Task 2, `test_a_binding_matches_a_venue_network_by_chain`.
3. Contract case differs (`0xeC74…` saved, `0xec74…` from the venue). Expected: EVM contracts compare case-insensitively. A Solana or Tron contract compares exactly. Pinned in Task 2, `test_evm_contracts_compare_without_case`.
4. The venue's coin list cannot be fetched (network error) while a binding exists. Expected: the binding is used, with a note that the venue could not be checked. The command does not stop. Pinned in Task 2, `test_a_binding_is_used_when_the_venue_check_fails`.
5. `token-health` runs inside the TUI with an ambiguous symbol. Expected: it stops with the candidate list. It does not hang on a stdin read inside the TUI. Pinned in Task 4, `test_pane_commands_never_use_the_terminal_picker`.

---

### Task 1: Core types: errors, the binding venue, and network names

**Files:**
- Modify: `crates/scope-core/src/error.rs` (add 3 variants to `ScopeError`)
- Modify: `crates/scope-core/src/tokens.rs` (`TokenInfo.venue`, `TokenAliases::add_binding`)
- Modify: `crates/scope-core/src/market/assets.rs` (`chain_for_network`)

**Interfaces:**
- Produces:
  - `ScopeError::TokenNotFound { input: String, searched: String }`
  - `ScopeError::AmbiguousToken { input: String, candidates: Vec<String> }`. Each string is one line: `"<chain> <contract-or-'native'> <name>"`.
  - `ScopeError::TokenMismatch { symbol: String, chain: String, saved: String, venue_contract: String, venue: String }`
  - `TokenInfo { …, #[serde(default)] pub venue: Option<String> }`
  - `pub fn TokenAliases::add_binding(&mut self, alias: &str, chain: &str, address: &str, name: &str, venue: Option<&str>)`. The existing `add` calls it with `None`.
  - `pub fn scope::market::assets::chain_for_network(network: &str) -> String`

- [ ] **Step 1: Write the failing tests**

In `crates/scope-core/src/market/assets.rs`, in `mod tests`:

```rust
    #[test]
    fn test_venue_network_codes_map_to_scope_chains() {
        // Review focus 2: a binding saved as `ethereum` must meet a venue's
        // `ETH` or `ERC20` as the same chain.
        for (code, chain) in [
            ("ETH", "ethereum"),
            ("ERC20", "ethereum"),
            ("ethereum", "ethereum"),
            ("BSC", "bsc"),
            ("BEP20", "bsc"),
            ("TRX", "tron"),
            ("TRC20", "tron"),
            ("SOL", "solana"),
        ] {
            assert_eq!(chain_for_network(code), chain, "{code}");
        }
        assert_eq!(chain_for_network("SOMECHAIN"), "somechain");
    }
```

In `crates/scope-core/src/tokens.rs`, in `mod tests`:

```rust
    #[test]
    fn test_a_binding_keeps_its_venue_and_old_files_load() {
        let mut aliases = TokenAliases::default();
        aliases.add_binding("USDN", "ethereum", "0xeC74", "USDN", Some("weex"));
        let info = aliases.get("usdn", Some("ethereum")).unwrap();
        assert_eq!(info.venue.as_deref(), Some("weex"));
        // A tokens.yaml written before this change has no `venue`.
        let old = "aliases:\n  USDN:\n    ethereum:\n      address: '0x1'\n      symbol: USDN\n      name: USDN\n      chain: ethereum\n";
        let loaded: TokenAliases = serde_yaml::from_str(old).unwrap();
        assert_eq!(loaded.get("USDN", Some("ethereum")).unwrap().venue, None);
    }
```

In `crates/scope-core/src/error.rs`, in `mod tests` (create the module at the end of the file if it does not exist):

```rust
    #[test]
    fn test_token_errors_say_what_to_do() {
        let e = ScopeError::AmbiguousToken {
            input: "USDN".into(),
            candidates: vec!["ethereum 0xeC74 USDN".into(), "bsc 0x55d3 USDN".into()],
        };
        let text = e.to_string();
        assert!(text.contains("ethereum 0xeC74 USDN") && text.contains("bsc 0x55d3 USDN"), "{text}");
        assert!(text.contains("--token"), "{text}");

        let e = ScopeError::TokenMismatch {
            symbol: "USDN".into(),
            chain: "ethereum".into(),
            saved: "0xAAA".into(),
            venue_contract: "0xBBB".into(),
            venue: "weex".into(),
        };
        let text = e.to_string();
        assert!(text.contains("0xAAA") && text.contains("0xBBB") && text.contains("weex"), "{text}");
    }
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -q -p scope-bca-core --lib -- assets:: tokens:: error::`
Expected: compile errors (`chain_for_network`, `add_binding`, `venue` and the three variants do not exist).

- [ ] **Step 3: Implement**

`error.rs`: add to `ScopeError`, next to `NotFound`:

```rust
    /// No token matches a symbol in the place that was searched.
    #[error("No token matches '{input}' ({searched})")]
    TokenNotFound {
        /// What the user typed.
        input: String,
        /// Where scope looked, for example "DexScreener on ethereum".
        searched: String,
    },

    /// A symbol names more than one token and scope cannot ask which one.
    #[error(
        "'{input}' matches more than one token:\n  {}\nPass --token <contract>, or run the command in a terminal to choose and save one.",
        candidates.join("\n  ")
    )]
    AmbiguousToken {
        /// What the user typed.
        input: String,
        /// One line for each candidate: chain, contract, name.
        candidates: Vec<String>,
    },

    /// A saved binding names a different contract than the venue lists.
    #[error(
        "{symbol} on {chain}: your saved token is {saved}, but {venue} lists {venue_contract}. \
         Check which one you mean, then pass --token <contract> or remove the binding (tokens remove {symbol} in the TUI)."
    )]
    TokenMismatch {
        symbol: String,
        chain: String,
        saved: String,
        venue_contract: String,
        venue: String,
    },
```

`tokens.rs`: add to `TokenInfo`, after `last_used`:

```rust
    /// The venue whose coin list this binding was checked against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<String>,
```

Rename the body of `add` to `add_binding`, with the extra argument, and keep `add` as a wrapper:

```rust
    pub fn add(&mut self, alias: &str, chain: &str, address: &str, name: &str) {
        self.add_binding(alias, chain, address, name, None);
    }

    /// Adds or updates a binding, with the venue that it was checked
    /// against.
    pub fn add_binding(
        &mut self,
        alias: &str,
        chain: &str,
        address: &str,
        name: &str,
        venue: Option<&str>,
    ) {
        // (the former body of `add`, with `venue: venue.map(str::to_string),`
        // added to the `TokenInfo { … }` literal)
    }
```

Fix the other `TokenInfo { … }` literals (`git grep -n 'TokenInfo {' crates`) with `venue: None`.

`market/assets.rs`:

```rust
/// Venue network codes that are not chain names or aliases in
/// `chains::registry`.
const NETWORK_CODES: [(&str, &str); 6] = [
    ("eth", "ethereum"),
    ("erc20", "ethereum"),
    ("bep20", "bsc"),
    ("trc20", "tron"),
    ("matic", "polygon"),
    ("arb", "arbitrum"),
];

/// The scope chain for a venue's network name (`ETH`, `ERC20`, `BSC`,
/// `TRX`, …). An unknown network is its own name in lower case.
pub fn chain_for_network(network: &str) -> String {
    let n = network.trim().to_ascii_lowercase();
    if let Some(chain) = crate::chains::registry::chain(&n) {
        return chain.id.to_string();
    }
    NETWORK_CODES
        .iter()
        .find(|(code, _)| *code == n)
        .map(|(_, chain)| chain.to_string())
        .unwrap_or(n)
}
```

Check that `chains::registry::chain` takes `&str` and returns a `Chain` with the field `id` (see `crates/scope-core/src/chains/registry.rs:187`). If "eth" is already an alias there, remove it from `NETWORK_CODES`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -q -p scope-bca-core --lib -- assets:: tokens:: error::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A crates/scope-core
git commit -m "feat(core): token errors, a binding's venue, and venue network names

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: The resolver

**Files:**
- Create: `crates/scope-cli/src/cli/resolve.rs`
- Modify: `crates/scope-cli/src/cli/mod.rs` (add `pub mod resolve;`)

**Interfaces:**
- Consumes: Task 1 (`ScopeError::{TokenNotFound, AmbiguousToken, TokenMismatch}`, `TokenAliases::add_binding`, `chain_for_network`).
- Produces (all `pub` in `crate::cli::resolve`):

```rust
pub struct Candidate { pub symbol: String, pub name: String, pub chain: String,
                       pub address: Option<String>, pub liquidity_usd: Option<f64> }
pub enum VenueKind { Dex, Cex(String) }          // Cex(venue id)
pub struct ResolveContext { pub chain: Option<String>, pub venue: VenueKind,
                            pub token_flag: Option<String>, pub yes: bool, pub save: bool }
pub enum Source { Flag, Address, Binding, Single, Picked { saved: bool } }
pub struct Resolved { pub symbol: String, pub name: String, pub chain: String,
                      pub address: Option<String>, pub source: Source }
#[async_trait] pub trait CandidateSource: Send + Sync {
    async fn candidates(&self, symbol: &str, chain: Option<&str>) -> Result<Vec<Candidate>>;
    fn searched(&self, chain: Option<&str>) -> String;   // for TokenNotFound
}
#[async_trait] pub trait Picker: Send + Sync {
    async fn pick(&self, input: &str, candidates: &[Candidate]) -> Result<usize>;
    async fn confirm(&self, question: &str) -> Result<bool>;
}
pub trait BindingStore { fn get(&self, symbol: &str, chain: Option<&str>) -> Option<TokenInfo>;
                         fn save(&mut self, c: &Candidate, venue: Option<&str>) -> Result<()>; }
pub async fn resolve(input: &str, ctx: &ResolveContext, source: &dyn CandidateSource,
                     picker: &dyn Picker, store: &mut dyn BindingStore, out: &Output) -> Result<Resolved>
pub fn candidate_line(c: &Candidate) -> String     // "<chain> <address|native> <name>"
```

- [ ] **Step 1: Write the failing tests**

Create `resolve.rs` with only `use` lines and this test module. The fakes are part of the tests.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn c(chain: &str, address: &str) -> Candidate {
        Candidate {
            symbol: "USDN".into(),
            name: "USDN".into(),
            chain: chain.into(),
            address: Some(address.into()),
            liquidity_usd: None,
        }
    }

    struct Fake(Result<Vec<Candidate>>);
    #[async_trait::async_trait]
    impl CandidateSource for Fake {
        async fn candidates(&self, _s: &str, _c: Option<&str>) -> Result<Vec<Candidate>> {
            match &self.0 {
                Ok(v) => Ok(v.clone()),
                Err(e) => Err(ScopeError::Network(e.to_string())),
            }
        }
        fn searched(&self, _c: Option<&str>) -> String {
            "the fake source".into()
        }
    }

    /// Answers with `pick` and `confirm`, and records what it was asked.
    struct Scripted {
        pick: Option<usize>,
        confirm: bool,
        asked: Mutex<Vec<String>>,
    }
    impl Scripted {
        fn new(pick: Option<usize>, confirm: bool) -> Self {
            Self { pick, confirm, asked: Mutex::new(Vec::new()) }
        }
        fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }
    #[async_trait::async_trait]
    impl Picker for Scripted {
        async fn pick(&self, input: &str, cs: &[Candidate]) -> Result<usize> {
            self.asked.lock().unwrap().push(format!("pick {input} {}", cs.len()));
            self.pick.ok_or_else(|| ScopeError::AmbiguousToken {
                input: input.into(),
                candidates: cs.iter().map(candidate_line).collect(),
            })
        }
        async fn confirm(&self, q: &str) -> Result<bool> {
            self.asked.lock().unwrap().push(format!("confirm {q}"));
            Ok(self.confirm)
        }
    }

    #[derive(Default)]
    struct Store(Vec<TokenInfo>);
    impl BindingStore for Store {
        fn get(&self, s: &str, chain: Option<&str>) -> Option<TokenInfo> {
            self.0
                .iter()
                .find(|t| t.symbol.eq_ignore_ascii_case(s) && chain.is_none_or(|ch| t.chain == ch))
                .cloned()
        }
        fn save(&mut self, c: &Candidate, venue: Option<&str>) -> Result<()> {
            self.0.push(binding(&c.chain, c.address.as_deref().unwrap_or(""), venue));
            Ok(())
        }
    }

    fn binding(chain: &str, address: &str, venue: Option<&str>) -> TokenInfo {
        TokenInfo {
            address: address.into(),
            symbol: "USDN".into(),
            name: "USDN".into(),
            chain: chain.into(),
            last_used: None,
            venue: venue.map(str::to_string),
        }
    }

    fn ctx(venue: VenueKind) -> ResolveContext {
        ResolveContext { chain: Some("ethereum".into()), venue, token_flag: None, yes: false, save: false }
    }

    fn out() -> Output {
        Output::capture().0
    }

    #[tokio::test]
    async fn test_the_token_flag_wins_over_everything() {
        let mut store = Store(vec![binding("ethereum", "0xSAVED", None)]);
        let mut cx = ctx(VenueKind::Dex);
        cx.token_flag = Some("0xFLAG".into());
        let picker = Scripted::new(None, false);
        let r = resolve("USDN", &cx, &Fake(Ok(vec![])), &picker, &mut store, &out()).await.unwrap();
        assert_eq!(r.address.as_deref(), Some("0xFLAG"));
        assert!(matches!(r.source, Source::Flag));
        assert!(picker.asked().is_empty());
    }

    #[tokio::test]
    async fn test_an_address_passes_through() {
        let addr = "0xeC747DAf41A4Ca4ebE72356592F8301924132907";
        let picker = Scripted::new(None, false);
        let r = resolve(addr, &ctx(VenueKind::Dex), &Fake(Ok(vec![])), &picker, &mut Store::default(), &out())
            .await
            .unwrap();
        assert_eq!(r.address.as_deref(), Some(addr));
        assert!(matches!(r.source, Source::Address));
    }

    #[tokio::test]
    async fn test_a_saved_binding_is_used_without_a_question() {
        let mut store = Store(vec![binding("ethereum", "0xSAVED", None)]);
        let picker = Scripted::new(None, false);
        let two = Fake(Ok(vec![c("ethereum", "0xA"), c("ethereum", "0xB")]));
        let r = resolve("usdn", &ctx(VenueKind::Dex), &two, &picker, &mut store, &out()).await.unwrap();
        assert_eq!(r.address.as_deref(), Some("0xSAVED"));
        assert!(matches!(r.source, Source::Binding));
        assert!(picker.asked().is_empty());
    }

    #[tokio::test]
    async fn test_a_binding_the_venue_contradicts_stops_with_both_contracts() {
        let mut store = Store(vec![binding("ethereum", "0xAAA", Some("weex"))]);
        let venue = Fake(Ok(vec![c("ETH", "0xBBB")]));
        let err = resolve("USDN", &ctx(VenueKind::Cex("weex".into())), &venue,
                          &Scripted::new(None, false), &mut store, &out())
            .await
            .unwrap_err();
        let text = err.to_string();
        assert!(matches!(err, ScopeError::TokenMismatch { .. }), "{text}");
        assert!(text.contains("0xAAA") && text.contains("0xBBB"), "{text}");
    }

    #[tokio::test]
    async fn test_a_binding_matches_a_venue_network_by_chain() {
        // Review focus 2: the venue calls it ERC20; the binding says ethereum.
        let mut store = Store(vec![binding("ethereum", "0xAAA", Some("weex"))]);
        let venue = Fake(Ok(vec![c("BEP20", "0xOTHER"), c("ERC20", "0xAAA")]));
        let r = resolve("USDN", &ctx(VenueKind::Cex("weex".into())), &venue,
                        &Scripted::new(None, false), &mut store, &out())
            .await
            .unwrap();
        assert_eq!(r.address.as_deref(), Some("0xAAA"));
    }

    #[tokio::test]
    async fn test_evm_contracts_compare_without_case() {
        // Review focus 3.
        let mut store = Store(vec![binding("ethereum", "0xeC747DAf41A4Ca4ebE72356592F8301924132907", None)]);
        let venue = Fake(Ok(vec![c("ETH", "0xec747daf41a4ca4ebe72356592f8301924132907")]));
        assert!(resolve("USDN", &ctx(VenueKind::Cex("weex".into())), &venue,
                        &Scripted::new(None, false), &mut store, &out())
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_a_binding_is_used_when_the_venue_check_fails() {
        // Review focus 4: a network error must not block a saved binding.
        let (o, cap) = Output::capture();
        let mut store = Store(vec![binding("ethereum", "0xAAA", None)]);
        let venue = Fake(Err(ScopeError::Network("down".into())));
        let r = resolve("USDN", &ctx(VenueKind::Cex("weex".into())), &venue,
                        &Scripted::new(None, false), &mut store, &o)
            .await
            .unwrap();
        assert_eq!(r.address.as_deref(), Some("0xAAA"));
        assert!(cap.err().contains("could not be checked"), "{}", cap.err());
    }

    #[tokio::test]
    async fn test_no_candidates_stops_and_names_the_source() {
        let err = resolve("USDN", &ctx(VenueKind::Dex), &Fake(Ok(vec![])),
                          &Scripted::new(None, false), &mut Store::default(), &out())
            .await
            .unwrap_err();
        assert!(matches!(err, ScopeError::TokenNotFound { .. }));
        assert!(err.to_string().contains("the fake source"));
    }

    #[tokio::test]
    async fn test_one_candidate_is_used_without_a_question() {
        let picker = Scripted::new(None, false);
        let r = resolve("USDN", &ctx(VenueKind::Dex), &Fake(Ok(vec![c("ethereum", "0xA")])),
                        &picker, &mut Store::default(), &out())
            .await
            .unwrap();
        assert_eq!(r.address.as_deref(), Some("0xA"));
        assert!(matches!(r.source, Source::Single));
        assert!(picker.asked().is_empty());
    }

    #[tokio::test]
    async fn test_only_exact_symbols_are_candidates() {
        // Review focus 1: USDNX is not USDN.
        let mut other = c("ethereum", "0xX");
        other.symbol = "USDNX".into();
        let picker = Scripted::new(None, false);
        let r = resolve("USDN", &ctx(VenueKind::Dex), &Fake(Ok(vec![other, c("ethereum", "0xA")])),
                        &picker, &mut Store::default(), &out())
            .await
            .unwrap();
        assert_eq!(r.address.as_deref(), Some("0xA"));
        assert!(picker.asked().is_empty(), "one exact match: no question");
    }

    #[tokio::test]
    async fn test_two_candidates_ask_then_save_on_yes() {
        let mut store = Store::default();
        let picker = Scripted::new(Some(1), true);
        let r = resolve("USDN", &ctx(VenueKind::Cex("weex".into())),
                        &Fake(Ok(vec![c("ETH", "0xA"), c("BSC", "0xB")])), &picker, &mut store, &out())
            .await
            .unwrap();
        assert_eq!(r.address.as_deref(), Some("0xB"));
        assert!(matches!(r.source, Source::Picked { saved: true }));
        assert_eq!(store.0.len(), 1);
        assert_eq!(store.0[0].chain, "bsc", "saved under the scope chain name");
        assert_eq!(store.0[0].venue.as_deref(), Some("weex"));
        let asked = picker.asked();
        assert!(asked[1].contains("always means 0xB on bsc"), "{asked:?}");
    }

    #[tokio::test]
    async fn test_no_saves_nothing_and_yes_flag_skips_the_question() {
        let two = || Fake(Ok(vec![c("ethereum", "0xA"), c("bsc", "0xB")]));
        let mut store = Store::default();
        let picker = Scripted::new(Some(0), false);
        resolve("USDN", &ctx(VenueKind::Dex), &two(), &picker, &mut store, &out()).await.unwrap();
        assert!(store.0.is_empty());

        // --save answers yes without a question.
        let mut cx = ctx(VenueKind::Dex);
        cx.save = true;
        let picker = Scripted::new(Some(0), false);
        resolve("USDN", &cx, &two(), &picker, &mut store, &out()).await.unwrap();
        assert_eq!(store.0.len(), 1);
        assert!(!picker.asked().iter().any(|a| a.starts_with("confirm")));
    }

    #[tokio::test]
    async fn test_yes_with_an_ambiguous_symbol_stops_with_the_candidates() {
        // Jon, 2026-10-15: --yes never picks the first match.
        let mut cx = ctx(VenueKind::Dex);
        cx.yes = true;
        let err = resolve("USDN", &cx, &Fake(Ok(vec![c("ethereum", "0xA"), c("bsc", "0xB")])),
                          &Scripted::new(Some(0), true), &mut Store::default(), &out())
            .await
            .unwrap_err();
        let text = err.to_string();
        assert!(matches!(err, ScopeError::AmbiguousToken { .. }), "{text}");
        assert!(text.contains("ethereum 0xA") && text.contains("bsc 0xB"), "{text}");
    }

    #[tokio::test]
    async fn test_each_resolution_writes_one_log_line() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("resolutions.jsonl");
        let picker = Scripted::new(None, false);
        resolve_logged("USDN", &ctx(VenueKind::Dex), &Fake(Ok(vec![c("ethereum", "0xA")])),
                       &picker, &mut Store::default(), &out(), &log)
            .await
            .unwrap();
        let _ = resolve_logged("NOPE", &ctx(VenueKind::Dex), &Fake(Ok(vec![])),
                               &picker, &mut Store::default(), &out(), &log)
            .await;
        let lines: Vec<serde_json::Value> = std::fs::read_to_string(&log).unwrap()
            .lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["source"], "single");
        assert_eq!(lines[0]["chosen"], "0xA");
        assert!(lines[1]["error"].as_str().unwrap().contains("NOPE"));
    }
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -q -p scope-bca-cli --lib resolve::`
Expected: compile errors (the types and `resolve` are not defined).

- [ ] **Step 3: Implement `resolve.rs` above the tests**

```rust
//! One resolver for every command that takes a token symbol
//! (docs/design/2026-10-15-token-disambiguation-design.md).
//!
//! Steps: `--token` → an address → a saved binding (checked against a CEX
//! venue's coin list) → the candidates → 0 stops, 1 is used, more go to
//! the picker → the save question. It never chooses silently.

use crate::cli::output::Output;
use crate::note;
use scope::error::{Result, ScopeError};
use scope::market::assets::chain_for_network;
use scope::tokens::{TokenAliases, TokenInfo};
use std::path::Path;

/// (Types and traits exactly as in this task's Interfaces block, each with
/// a one-line doc comment, `#[derive(Debug, Clone, PartialEq)]` on the data
/// types.)

/// One line for a candidate in lists and errors: chain, contract, name.
pub fn candidate_line(c: &Candidate) -> String {
    format!(
        "{} {} {}",
        c.chain,
        c.address.as_deref().unwrap_or("native"),
        c.name
    )
}

/// Two contracts are the same token: EVM hex compares without case.
fn same_contract(a: &str, b: &str) -> bool {
    if a.starts_with("0x") && b.starts_with("0x") {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// [`resolve_logged`] with the log in the scope state folder.
pub async fn resolve(
    input: &str,
    ctx: &ResolveContext,
    source: &dyn CandidateSource,
    picker: &dyn Picker,
    store: &mut dyn BindingStore,
    out: &Output,
) -> Result<Resolved> {
    let log = scope::paths::state_dir().join("resolutions.jsonl");
    resolve_logged(input, ctx, source, picker, store, out, &log).await
}

/// Resolves `input`, then appends one line to `log` (the RCA trail). A
/// log failure is a warning.
pub async fn resolve_logged(
    input: &str,
    ctx: &ResolveContext,
    source: &dyn CandidateSource,
    picker: &dyn Picker,
    store: &mut dyn BindingStore,
    out: &Output,
    log: &Path,
) -> Result<Resolved> {
    let mut seen = Vec::new();
    let result = steps(input, ctx, source, picker, store, out, &mut seen).await;
    if let Err(e) = write_log(log, input, ctx, &seen, &result) {
        tracing::warn!("cannot write {}: {e}", log.display());
    }
    result
}

async fn steps(
    input: &str,
    ctx: &ResolveContext,
    source: &dyn CandidateSource,
    picker: &dyn Picker,
    store: &mut dyn BindingStore,
    out: &Output,
    seen: &mut Vec<Candidate>,
) -> Result<Resolved> {
    let symbol = input.trim().to_uppercase();
    let direct = |address: &str, source: Source| Resolved {
        symbol: symbol.clone(),
        name: symbol.clone(),
        chain: ctx.chain.clone().unwrap_or_default(),
        address: Some(address.to_string()),
        source,
    };
    // 1. --token, 2. an address
    if let Some(flag) = &ctx.token_flag {
        return Ok(direct(flag, Source::Flag));
    }
    if TokenAliases::is_address(input.trim()) {
        return Ok(direct(input.trim(), Source::Address));
    }
    // 3. a saved binding
    if let Some(saved) = store.get(&symbol, ctx.chain.as_deref()) {
        if let VenueKind::Cex(venue) = &ctx.venue {
            match source.candidates(&symbol, None).await {
                Ok(listed) => {
                    *seen = listed.clone();
                    let on_chain = listed
                        .iter()
                        .find(|c| chain_for_network(&c.chain) == saved.chain);
                    if let Some(c) = on_chain
                        && let Some(venue_contract) = &c.address
                        && !same_contract(venue_contract, &saved.address)
                    {
                        return Err(ScopeError::TokenMismatch {
                            symbol,
                            chain: saved.chain,
                            saved: saved.address,
                            venue_contract: venue_contract.clone(),
                            venue: venue.clone(),
                        });
                    }
                }
                Err(e) => note!(out, "{symbol}: {venue} could not be checked ({e}); using your saved token."),
            }
        }
        return Ok(Resolved {
            symbol,
            name: saved.name,
            chain: saved.chain,
            address: Some(saved.address),
            source: Source::Binding,
        });
    }
    // 4. the candidates: exact symbols only
    let candidates: Vec<Candidate> = source
        .candidates(&symbol, ctx.chain.as_deref())
        .await?
        .into_iter()
        .filter(|c| c.symbol.eq_ignore_ascii_case(&symbol))
        .collect();
    *seen = candidates.clone();
    // 5. 0 / 1 / more
    let chosen = match candidates.len() {
        0 => {
            return Err(ScopeError::TokenNotFound {
                input: input.to_string(),
                searched: source.searched(ctx.chain.as_deref()),
            });
        }
        1 => {
            note!(out, "Using {} ({})", candidate_line(&candidates[0]), symbol);
            return Ok(resolved_from(&candidates[0], Source::Single));
        }
        _ if ctx.yes => {
            return Err(ScopeError::AmbiguousToken {
                input: input.to_string(),
                candidates: candidates.iter().map(candidate_line).collect(),
            });
        }
        _ => &candidates[picker.pick(input, &candidates).await?],
    };
    // 6. the save question
    let chain = chain_for_network(&chosen.chain);
    let contract = chosen.address.as_deref().unwrap_or("its native coin");
    let save = ctx.save
        || (!ctx.yes
            && picker
                .confirm(&format!(
                    "Save so {symbol} always means {contract} on {chain}? [y/N]: "
                ))
                .await?);
    if save {
        let mut binding = chosen.clone();
        binding.chain = chain.clone();
        let venue = match &ctx.venue {
            VenueKind::Cex(v) => Some(v.as_str()),
            VenueKind::Dex => None,
        };
        store.save(&binding, venue)?;
    }
    let mut r = resolved_from(chosen, Source::Picked { saved: save });
    r.chain = chain;
    Ok(r)
}

fn resolved_from(c: &Candidate, source: Source) -> Resolved {
    Resolved {
        symbol: c.symbol.to_uppercase(),
        name: c.name.clone(),
        chain: chain_for_network(&c.chain),
        address: c.address.clone(),
        source,
    }
}

fn write_log(
    log: &Path,
    input: &str,
    ctx: &ResolveContext,
    seen: &[Candidate],
    result: &Result<Resolved>,
) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let venue = match &ctx.venue {
        VenueKind::Dex => "dex".to_string(),
        VenueKind::Cex(v) => v.clone(),
    };
    let mut line = serde_json::json!({
        "at": chrono::Utc::now().to_rfc3339(),
        "input": input,
        "chain": ctx.chain,
        "venue": venue,
        "candidates": seen.iter().map(candidate_line).collect::<Vec<_>>(),
    });
    match result {
        Ok(r) => {
            line["chosen"] = serde_json::json!(r.address);
            line["source"] = serde_json::json!(match r.source {
                Source::Flag => "flag",
                Source::Address => "address",
                Source::Binding => "binding",
                Source::Single => "single",
                Source::Picked { .. } => "picked",
            });
            line["saved"] = serde_json::json!(matches!(r.source, Source::Picked { saved: true }));
        }
        Err(e) => line["error"] = serde_json::json!(e.to_string()),
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(log)?;
    writeln!(f, "{line}")
}
```

`TokenNotFound`'s message holds `input`, so the log's `error` text contains it. The last test checks this.

Add to `crates/scope-cli/src/cli/mod.rs`, next to the other `pub mod` lines: `pub mod resolve;`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -q -p scope-bca-cli --lib resolve::`
Expected: all 15 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A crates/scope-cli
git commit -m "feat(resolve): one token resolver with bindings, venue check and an RCA log

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: The sources, the store and the two pickers

**Files:**
- Modify: `crates/scope-cli/src/cli/resolve.rs` (add the implementations below the core code)
- Modify: `crates/scope-cli/src/cli/token_pick.rs` (add `pub(crate) fn candidate_table(&[Candidate]) -> String`)

**Interfaces:**
- Consumes: Task 2 traits. `DexDataSource::search_tokens(&self, query: &str, chain: Option<&str>) -> Result<Vec<TokenSearchResult>>`. `token_pick::cex_fallback(symbol, chain, None, out) -> Option<TokenSearchResult>`. `ExchangeClient::fetch_assets` through `ConfigurableExchangeClient` (#101): add `pub async fn ExchangeClient::fetch_assets(&self) -> Option<Result<Vec<AssetInfo>>>` in `crates/scope-core/src/market/exchange.rs`, which mirrors `fetch_token_addresses`.
- Produces:
  - `pub struct DexCandidates<'a> { pub dex: &'a dyn DexDataSource, pub out: &'a Output }`
  - `pub struct VenueCandidates<'a> { pub client: &'a ExchangeClient }`
  - `impl BindingStore for TokenAliases`. `save` calls `add_binding(...)` then `TokenAliases::save()`.
  - `pub struct TerminalPicker<'a> { pub out: &'a Output }`
  - `pub struct NoTerminalPicker`
  - `pub fn picker_for<'a>(out: &'a Output, json: bool) -> Box<dyn Picker + 'a>`: a `NoTerminalPicker` when `out.is_tui()`, or `json`, or stdin/stdout is not a terminal; else a `TerminalPicker`.

- [ ] **Step 1: Write the failing tests** (in `resolve.rs` `mod tests`)

```rust
    #[tokio::test]
    async fn test_no_terminal_picker_lists_every_candidate_and_the_hint() {
        let err = NoTerminalPicker
            .pick("USDN", &[c("ethereum", "0xA"), c("bsc", "0xB")])
            .await
            .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("ethereum 0xA") && text.contains("bsc 0xB"), "{text}");
        assert!(text.contains("--token"), "{text}");
        assert!(!NoTerminalPicker.confirm("save?").await.unwrap());
    }

    #[test]
    fn test_picker_for_json_or_tui_never_prompts() {
        // Review focus 5 (the TUI part is in Task 4): a JSON run must not wait for stdin.
        let (o, _cap) = Output::capture();
        let p = picker_for(&o, true);
        let err = futures::executor::block_on(p.pick("USDN", &[c("ethereum", "0xA"), c("bsc", "0xB")]));
        assert!(matches!(err, Err(ScopeError::AmbiguousToken { .. })));
    }

    #[tokio::test]
    async fn test_venue_candidates_are_one_per_network_with_scope_chain_names() {
        let assets = vec![scope::market::assets::AssetInfo {
            symbol: "USDN".into(),
            networks: vec![
                scope::market::assets::AssetNetwork { network: "ETH".into(), contract: Some("0xA".into()) },
                scope::market::assets::AssetNetwork { network: "BSC".into(), contract: Some("0xB".into()) },
            ],
        }];
        let cs = venue_candidates_from("usdn", &assets);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].address.as_deref(), Some("0xA"));
        assert_eq!(cs[0].chain, "ETH", "the venue's name; resolve maps it");
    }

    #[test]
    fn test_dex_results_become_candidates_with_liquidity() {
        let r = scope::chains::dex::TokenSearchResult {
            address: "0xA".into(), symbol: "USDN".into(), name: "USDN".into(), chain: "ethereum".into(),
            price_usd: None, volume_24h: 0.0, liquidity_usd: 5.0, market_cap: None,
        };
        let c = dex_candidate(&r);
        assert_eq!(c.liquidity_usd, Some(5.0));
        // A CEX fallback result has no address: it is a native-style candidate.
        let mut f = r.clone();
        f.address = String::new();
        assert_eq!(dex_candidate(&f).address, None);
    }
```

(If the crate has no `futures` dev-dependency, use `#[tokio::test]` and `.await` for the second test instead. Check with `grep -n futures crates/scope-cli/Cargo.toml`.)

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -q -p scope-bca-cli --lib resolve::`
Expected: compile errors (`NoTerminalPicker`, `picker_for`, `venue_candidates_from` and `dex_candidate` are not defined).

- [ ] **Step 3: Implement**

```rust
/// A DexScreener search result as a candidate. An empty address (the CEX
/// ticker fallback) is a token without a contract.
pub(crate) fn dex_candidate(r: &scope::chains::dex::TokenSearchResult) -> Candidate {
    Candidate {
        symbol: r.symbol.clone(),
        name: r.name.clone(),
        chain: r.chain.clone(),
        address: (!r.address.is_empty()).then(|| r.address.clone()),
        liquidity_usd: Some(r.liquidity_usd),
    }
}

/// The venue's networks for `symbol`, one candidate each.
pub(crate) fn venue_candidates_from(
    symbol: &str,
    assets: &[scope::market::assets::AssetInfo],
) -> Vec<Candidate> {
    assets
        .iter()
        .filter(|a| a.symbol.eq_ignore_ascii_case(symbol))
        .flat_map(|a| {
            a.networks.iter().map(|n| Candidate {
                symbol: a.symbol.clone(),
                name: a.symbol.clone(),
                chain: n.network.clone(),
                address: n.contract.clone(),
                liquidity_usd: None,
            })
        })
        .collect()
}

pub struct DexCandidates<'a> {
    pub dex: &'a dyn scope::chains::DexDataSource,
    pub out: &'a Output,
}

#[async_trait::async_trait]
impl CandidateSource for DexCandidates<'_> {
    async fn candidates(&self, symbol: &str, chain: Option<&str>) -> Result<Vec<Candidate>> {
        let mut found = self.dex.search_tokens(symbol, chain).await?;
        if found.is_empty()
            && let Some(f) = crate::cli::token_pick::cex_fallback(
                symbol, chain.unwrap_or("ethereum"), None, self.out).await
        {
            found.push(f);
        }
        Ok(found.iter().map(dex_candidate).collect())
    }
    fn searched(&self, chain: Option<&str>) -> String {
        format!("DexScreener on {} and the CEX ticker fallback", chain.unwrap_or("all chains"))
    }
}

pub struct VenueCandidates<'a> {
    pub client: &'a scope::market::ExchangeClient,
}

#[async_trait::async_trait]
impl CandidateSource for VenueCandidates<'_> {
    async fn candidates(&self, symbol: &str, _chain: Option<&str>) -> Result<Vec<Candidate>> {
        match self.client.fetch_assets().await {
            Some(assets) => Ok(venue_candidates_from(symbol, &assets?)),
            // No `assets` capability: the venue symbol is the only thing known.
            None => Ok(vec![Candidate {
                symbol: symbol.to_uppercase(),
                name: symbol.to_uppercase(),
                chain: String::new(),
                address: None,
                liquidity_usd: None,
            }]),
        }
    }
    fn searched(&self, _chain: Option<&str>) -> String {
        format!("the {} coin list", self.client.venue_id())
    }
}

impl BindingStore for TokenAliases {
    fn get(&self, symbol: &str, chain: Option<&str>) -> Option<TokenInfo> {
        TokenAliases::get(self, symbol, chain).cloned()
    }
    fn save(&mut self, c: &Candidate, venue: Option<&str>) -> Result<()> {
        self.add_binding(&c.symbol, &c.chain, c.address.as_deref().unwrap_or(""), &c.name, venue);
        TokenAliases::save(self)
    }
}

pub struct TerminalPicker<'a> {
    pub out: &'a Output,
}

#[async_trait::async_trait]
impl Picker for TerminalPicker<'_> {
    async fn pick(&self, _input: &str, cs: &[Candidate]) -> Result<usize> {
        let table = crate::cli::token_pick::candidate_table(cs);
        crate::cli::prompt::pick(&mut crate::cli::prompt::stdin(), self.out, &table, "token", cs.len())
    }
    async fn confirm(&self, q: &str) -> Result<bool> {
        let a = crate::cli::prompt::ask(&mut crate::cli::prompt::stdin(), self.out, q)?;
        Ok(matches!(a.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
    }
}

pub struct NoTerminalPicker;

#[async_trait::async_trait]
impl Picker for NoTerminalPicker {
    async fn pick(&self, input: &str, cs: &[Candidate]) -> Result<usize> {
        Err(ScopeError::AmbiguousToken {
            input: input.to_string(),
            candidates: cs.iter().map(candidate_line).collect(),
        })
    }
    async fn confirm(&self, _q: &str) -> Result<bool> {
        Ok(false)
    }
}

/// The picker for this run: never a prompt in the TUI pane, for JSON
/// output, or without a terminal.
pub fn picker_for<'a>(out: &'a Output, json: bool) -> Box<dyn Picker + 'a> {
    use std::io::IsTerminal as _;
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if out.is_tui() || json || !terminal {
        Box::new(NoTerminalPicker)
    } else {
        Box::new(TerminalPicker { out })
    }
}
```

`token_pick.rs`: add `candidate_table(cs: &[Candidate]) -> String`, which builds the same numbered table as `table(&[TokenSearchResult])` (columns: #, symbol, name, chain, address, liquidity). Make `table` call it after a `dex_candidate` mapping, so there is one table function.

`exchange.rs`: add, next to `fetch_token_addresses`:

```rust
    /// The venue's coin list, or `None` when the venue has no `assets`
    /// capability.
    pub async fn fetch_assets(&self) -> Option<Result<Vec<super::assets::AssetInfo>>> {
        Some(self.assets.as_ref()?.fetch_assets().await)
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -q -p scope-bca-cli --lib -- resolve:: token_pick::` and `cargo test -q -p scope-bca-core --lib market::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A crates
git commit -m "feat(resolve): DEX and venue candidates, the alias store, terminal and no-terminal pickers

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Every command on the resolver

**Files:**
- Modify: `crates/scope-cli/src/cli/crawl.rs` (`resolve_token_input`, `select_token*`, `prompt_save_alias*`, `fetch_analytics_for_input`, the `--yes` help, and a new `--token` arg)
- Modify: `crates/scope-cli/src/cli/monitor/mod.rs` (`resolve_token_address`, `select_token_*`, and a new `--token` arg)
- Modify: `crates/scope-cli/src/cli/market.rs` (CEX `summary`, `ohlc` and `trades` resolve the base symbol first; `--token` on `SummaryArgs`)
- Modify: `crates/scope-cli/src/cli/token_health.rs`, `crates/scope-cli/src/cli/insights.rs` (pass `--token` through to `fetch_analytics_for_input`)

**Interfaces:**
- Consumes: Task 2 `resolve`, `ResolveContext`, `VenueKind`; Task 3 `DexCandidates`, `VenueCandidates`, `picker_for`, `impl BindingStore for TokenAliases`.
- Produces: `crawl::resolve_token_input(args, aliases, dex, spinner, out) -> Result<ResolvedToken>` keeps its signature but calls `resolve`. `fetch_analytics_for_input` gets a new last-but-one argument, `token_flag: Option<&str>`.

- [ ] **Step 1: Write the failing tests**

In `crawl.rs` `mod tests`, add:

```rust
    #[tokio::test]
    async fn test_crawl_yes_with_two_matches_stops_with_the_candidates() {
        // Jon, 2026-10-15: --yes no longer picks the first match.
        let dex = TwoUsdn; // a DexDataSource fake: search_tokens returns two exact USDN tokens
        let mut aliases = TokenAliases::default();
        let mut args = test_args("USDN");
        args.yes = true;
        let err = resolve_token_input(&args, &mut aliases, &dex, None, &Output::capture().0)
            .await
            .unwrap_err();
        assert!(matches!(err, ScopeError::AmbiguousToken { .. }), "{err}");
    }
```

Write the `TwoUsdn` fake next to the existing `DexDataSource` fakes in the `crawl.rs` tests (`grep -n 'impl DexDataSource for' crates/scope-cli/src/cli/crawl.rs`). Copy one, and make its `search_tokens` return two `TokenSearchResult`s with symbol `USDN` on `ethereum` and `bsc`. Use the module's existing `CrawlArgs` test builder, or add `fn test_args(token: &str) -> CrawlArgs` with the defaults that the other tests use.

In `market.rs` `mod tests`, add a CEX test with a mock venue that has `assets` (reuse `scope::market::testing::venue_at` and add an `assets` capability to its descriptor):

```rust
    #[tokio::test]
    async fn test_market_stops_when_a_binding_disagrees_with_the_venue() {
        // A saved USDN (0xAAA) on ethereum; the venue lists USDN on ETH as 0xBBB.
        // The summary must not run on the venue's token.
        // (Mock server: /coins returns [{"coin":"USDN","networkList":[{"network":"ETH","contractAddress":"0xBBB"}]}].)
        // Set the binding in the sandbox store: TokenAliases::load(), add_binding("USDN","ethereum","0xAAA","USDN",Some("mock")), save().
        // Run summary_run(...) for pair "USDN" on that venue.
        // assert!(matches!(err, ScopeError::TokenMismatch { .. }))
    }
```

Write this test fully, following the existing mockito venue tests in `market.rs` (`grep -n 'venue_at\|mockito' crates/scope-cli/src/cli/market.rs`). The sandbox (#98) keeps `TokenAliases::save()` in the temp folder.

Add a test for review focus 5 in `crates/scope-cli/src/cli/resolve.rs`:

```rust
    #[test]
    fn test_pane_commands_never_use_the_terminal_picker() {
        // Review focus 5: in the TUI pane a prompt would read the TUI's stdin.
        let (o, _cap) = Output::capture_merged(); // a TUI-style output (series hook)
        assert!(o.is_tui());
        let p = picker_for(&o, false);
        let r = futures::executor::block_on(p.pick("USDN", &[c("ethereum", "0xA"), c("bsc", "0xB")]));
        assert!(matches!(r, Err(ScopeError::AmbiguousToken { .. })));
    }
```

Check how `Output::is_tui()` is decided (`grep -n 'fn is_tui' -A5 crates/scope-cli/src/cli/output.rs`). Build the TUI-style output the same way the TUI does, if `capture_merged` is not TUI-style.

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -q -p scope-bca-cli --lib -- crawl::tests::test_crawl_yes market::tests::test_market_stops resolve::tests::test_pane`
Expected: the crawl test fails, because `--yes` picks the first match today. The market test fails, because the summary runs. The pane test passes or fails depending on `is_tui` (record which one).

- [ ] **Step 3: Wire the commands**

1. `crawl::resolve_token_input`: replace the body after the address-book step with a call to `resolve`:

   ```rust
   let ctx = ResolveContext {
       chain: (args.chain != "ethereum").then(|| args.chain.clone()),
       venue: VenueKind::Dex,
       token_flag: args.token_contract.clone(),
       yes: args.yes,
       save: args.save,
   };
   let source = DexCandidates { dex: dex_client, out };
   let picker = picker_for(out, matches!(args.format, OutputFormat::Json));
   let r = resolve(&args.token, &ctx, &source, picker.as_ref(), aliases, out).await?;
   Ok(ResolvedToken {
       address: r.address.unwrap_or_default(),
       chain: if r.chain.is_empty() { args.chain.clone() } else { r.chain },
       alias_info: Some((r.symbol, r.name)),
   })
   ```

   Keep the address-chain inference: the `is_address` branch with `infer_chain_from_address` stays before the call. Delete `select_token`, `select_token_impl`, `prompt_save_alias` and `prompt_save_alias_impl`, and the tests that only covered them. Their behavior is now in `resolve.rs` tests.

2. `CrawlArgs`: add

   ```rust
   /// The token's contract: use it and skip the symbol lookup.
   #[arg(long = "token", value_name = "CONTRACT")]
   pub token_contract: Option<String>,
   ```

   Change the `yes` help to `/// Skip the save question (an ambiguous symbol still stops; use --token).`

3. `fetch_analytics_for_input`: add the argument `token_flag: Option<&str>`, and set `yes: false` (it was `true`, which picked silently). Build `CrawlArgs { token_contract: token_flag.map(str::to_string), … }`. Update the callers in `token_health.rs`, `insights.rs` and `market.rs` (`CrawlDexSource`), and add `--token` to `TokenHealthArgs` and `InsightsArgs` in the same way as in `CrawlArgs`.

4. `monitor::resolve_token_address`: replace the body with the same `resolve` call (`VenueKind::Dex`, the `allow_cex_fallback` flag decides between `DexCandidates` and a `DexCandidates` without the fallback; add a `fallback: bool` field to `DexCandidates` for this). Delete `select_token_interactive` and `select_token_impl`. Add `--token` to `MonitorArgs`.

5. `market` CEX path: in `summary_run` (and in the `ohlc` and `trades` runners), when `MarketVenue::from_name(&args.venue)` is `Cex(client)`, first resolve `Pair::parse(&args.pair).base` with `VenueCandidates { client: &client }`, `VenueKind::Cex(client.venue_id().into())`, and `ctx.chain = args.chain.clone()`. Use the result only to stop on a mismatch or an ambiguity. The venue symbol stays as it is. Do it once per command (before the repeat loop), as `token_addresses` does.

6. TUI routing: `crates/scope-cli/src/cli/tui/exec.rs` already routes `crawl` with a symbol to suspend mode, so it gets the terminal picker. `token-health`, `insights`, `market` and `monitor` keep their routes. In the pane they get `NoTerminalPicker` (plan 2 replaces it with the overlay).

- [ ] **Step 4: Run the tests**

Run:
```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
just tui-test
```
Expected: all pass. The snapshot check shows that no real file was written. If a harness scenario resolved a symbol by first match (`offline outside-crawl`), it now prompts or stops. Update the scenario's input to pick by number, and record that as a ruling.

- [ ] **Step 5: Commit**

```bash
git add -A crates
git commit -m "feat: every command resolves tokens through one resolver

crawl, monitor, token-health, insights and market (DEX and CEX) call
resolve. --token <contract> skips the lookup. --yes skips only the save
question; an ambiguous symbol stops with the candidates. A CEX binding
that the venue's coin list contradicts stops with both contracts.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Docs, CHANGELOG and an end-to-end check

**Files:**
- Modify: `README.md` (a "Choosing a token" section), `CHANGELOG.md` (`[Unreleased]`)

- [ ] **Step 1: README**

Add under the crawl and market sections:

```markdown
### Choosing a token

A symbol such as `USDN` can name more than one token. When it does, scope lists the tokens (chain, contract, name) and asks you to choose one. Then it asks whether to save the choice: "Save so USDN always means 0x… on ethereum?". A saved choice is used from then on without a question. On an exchange venue that lists its coins' contracts, scope also checks the saved contract against the venue's, and stops if they differ.

Without a terminal (a pipe, `--format json` or a script), scope does not ask. It stops with the list. Pass `--token <contract>` to choose. `--yes` skips only the save question.

Saved choices are in `~/.scope-bca/data/tokens.yaml`. In the TUI, `tokens list` shows them and `tokens remove <SYMBOL>` removes one. Each lookup is recorded in `~/.scope-bca/state/resolutions.jsonl`.
```

- [ ] **Step 2: CHANGELOG `[Unreleased]`**

```markdown
### Changed (breaking)
- `crawl --yes` no longer selects the first match for an ambiguous symbol. It stops with the candidates; pass `--token <contract>`. `--yes` now skips only the save question.
- `token-health`, `insights` and `market` on a DEX venue no longer pick the first search match. An ambiguous symbol asks, or stops with the candidates when there is no terminal.

### Added
- One token resolver for every command that takes a symbol, with saved choices, `--token <contract>`, and a check of saved choices against a venue's coin list.
```

- [ ] **Step 3: End to end (reads real data; writes only to a temporary home)**

```bash
cargo build --release
tmp=$(mktemp -d)
SCOPE_BCA_HOME=$tmp ./target/release/scope market summary USDN --venue weex --format json | head -5
SCOPE_BCA_HOME=$tmp ./target/release/scope crawl USDN --format json | head -5
tail -3 $tmp/state/resolutions.jsonl
rm -rf $tmp
```

Expected:
1. `market … --venue weex`: one WEEX USDN network, so there is no question, and a `single` log line.
2. `crawl USDN --format json`: if DexScreener has more than one exact USDN, the run stops with `AmbiguousToken` and lists them; else it reports on the only one.
3. The log lines match.

- [ ] **Step 4: Gates, commit and PR**

Run the gates from Task 4, Step 4. Then commit (`docs: choosing a token`), push, and open a PR into `develop` with the end-to-end output.
