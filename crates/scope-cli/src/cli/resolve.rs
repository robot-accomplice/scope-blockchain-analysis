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

/// One token a symbol could mean.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub symbol: String,
    pub name: String,
    /// The chain, or the venue's network name for a CEX candidate.
    pub chain: String,
    /// The contract; `None` for a native coin or an unknown contract.
    pub address: Option<String>,
    pub price_usd: Option<f64>,
    pub liquidity_usd: Option<f64>,
}

/// Where the command reads its market data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VenueKind {
    Dex,
    /// A CEX venue, by id.
    Cex(String),
}

/// What the command knows before the lookup.
#[derive(Debug, Clone)]
pub struct ResolveContext {
    /// The chain filter, when the user set one.
    pub chain: Option<String>,
    pub venue: VenueKind,
    /// `--token <contract>`.
    pub token_flag: Option<String>,
    /// `--yes`: no save question; an ambiguous symbol stops.
    pub yes: bool,
    /// `--save`: save the choice without a question.
    pub save: bool,
}

/// How the token was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Flag,
    Address,
    Binding,
    Single,
    Picked { saved: bool },
}

/// The token a command uses.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub symbol: String,
    pub name: String,
    pub chain: String,
    pub address: Option<String>,
    pub source: Source,
}

/// Where candidates come from: a DEX search or a venue's coin list.
#[async_trait::async_trait]
pub trait CandidateSource: Send + Sync {
    async fn candidates(&self, symbol: &str, chain: Option<&str>) -> Result<Vec<Candidate>>;
    /// What was searched, for a "not found" error.
    fn searched(&self, chain: Option<&str>) -> String;
}

/// Asks the user to choose.
#[async_trait::async_trait]
pub trait Picker: Send + Sync {
    /// The index of the chosen candidate.
    async fn pick(&self, input: &str, candidates: &[Candidate]) -> Result<usize>;
    /// The answer to a yes/no question.
    async fn confirm(&self, question: &str) -> Result<bool>;
}

/// The saved bindings.
pub trait BindingStore {
    fn get(&self, symbol: &str, chain: Option<&str>) -> Option<TokenInfo>;
    fn save(&mut self, candidate: &Candidate, venue: Option<&str>) -> Result<()>;
}

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

/// A DexScreener search result as a candidate. An empty address (the CEX
/// ticker fallback) is a token without a contract.
pub(crate) fn dex_candidate(r: &scope::chains::dex::TokenSearchResult) -> Candidate {
    Candidate {
        symbol: r.symbol.clone(),
        name: r.name.clone(),
        chain: r.chain.clone(),
        address: (!r.address.is_empty()).then(|| r.address.clone()),
        price_usd: r.price_usd,
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
                price_usd: None,
                liquidity_usd: None,
            })
        })
        .collect()
}

/// Candidates from a DexScreener search, with the CEX ticker fallback when
/// `fallback` is set and the search finds nothing.
pub struct DexCandidates<'a> {
    pub dex: &'a dyn scope::chains::DexDataSource,
    pub out: &'a Output,
    pub fallback: bool,
}

#[async_trait::async_trait]
impl CandidateSource for DexCandidates<'_> {
    async fn candidates(&self, symbol: &str, chain: Option<&str>) -> Result<Vec<Candidate>> {
        let mut found = self.dex.search_tokens(symbol, chain).await?;
        if found.is_empty()
            && self.fallback
            && let Some(f) = crate::cli::token_pick::cex_fallback(
                symbol,
                chain.unwrap_or("ethereum"),
                None,
                self.out,
            )
            .await
        {
            found.push(f);
        }
        Ok(found.iter().map(dex_candidate).collect())
    }

    fn searched(&self, chain: Option<&str>) -> String {
        format!(
            "DexScreener on {}{}",
            chain.unwrap_or("all chains"),
            if self.fallback {
                " and the CEX ticker fallback"
            } else {
                ""
            }
        )
    }
}

/// Candidates from a CEX venue's coin list (the `assets` capability).
pub struct VenueCandidates<'a> {
    pub client: &'a scope::market::ExchangeClient,
}

#[async_trait::async_trait]
impl CandidateSource for VenueCandidates<'_> {
    async fn candidates(&self, symbol: &str, _chain: Option<&str>) -> Result<Vec<Candidate>> {
        match self.client.fetch_assets().await {
            Some(assets) => Ok(venue_candidates_from(symbol, &assets?)),
            // No `assets` capability: the venue symbol is all that is known.
            None => Ok(vec![Candidate {
                symbol: symbol.to_uppercase(),
                name: symbol.to_uppercase(),
                chain: String::new(),
                address: None,
                price_usd: None,
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
        let address = c.address.as_deref().unwrap_or("");
        self.add_binding(&c.symbol, &c.chain, address, &c.name, venue);
        TokenAliases::save(self)
    }
}

/// Asks on the terminal: the numbered token list, then yes/no.
pub struct TerminalPicker<'a> {
    pub out: &'a Output,
}

#[async_trait::async_trait]
impl Picker for TerminalPicker<'_> {
    async fn pick(&self, _input: &str, cs: &[Candidate]) -> Result<usize> {
        let table = crate::cli::token_pick::candidate_table(cs);
        let mut input = crate::cli::prompt::stdin();
        crate::cli::prompt::pick(&mut input, self.out, &table, "token", cs.len())
    }

    async fn confirm(&self, question: &str) -> Result<bool> {
        let mut input = crate::cli::prompt::stdin();
        let answer = crate::cli::prompt::ask(&mut input, self.out, question)?;
        Ok(matches!(
            answer.trim().to_ascii_lowercase().as_str(),
            "y" | "yes"
        ))
    }
}

/// Never asks: a choice is an error that lists the candidates.
pub struct NoTerminalPicker;

#[async_trait::async_trait]
impl Picker for NoTerminalPicker {
    async fn pick(&self, input: &str, cs: &[Candidate]) -> Result<usize> {
        Err(ScopeError::AmbiguousToken {
            input: input.to_string(),
            candidates: cs.iter().map(candidate_line).collect(),
        })
    }

    async fn confirm(&self, _question: &str) -> Result<bool> {
        Ok(false)
    }
}

/// The picker for this run: no prompt in the TUI pane (plan 2 adds an
/// overlay), for JSON output, or without a terminal.
pub fn picker_for<'a>(out: &'a Output, json: bool) -> Box<dyn Picker + 'a> {
    use std::io::IsTerminal as _;
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if out.is_tui() || json || !terminal {
        Box::new(NoTerminalPicker)
    } else {
        Box::new(TerminalPicker { out })
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
    // 3. a saved binding, checked against a CEX venue's coin list
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
                Err(e) => note!(
                    out,
                    "{symbol}: {venue} could not be checked ({e}); using your saved token."
                ),
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
    // 5. none, one, or a choice
    let chosen = match candidates.len() {
        0 => {
            return Err(ScopeError::TokenNotFound {
                input: input.to_string(),
                searched: source.searched(ctx.chain.as_deref()),
            });
        }
        1 => {
            note!(out, "Using {}", candidate_line(&candidates[0]));
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
        || picker
            .confirm(&format!(
                "Save so {symbol} always means {contract} on {chain}? [y/N]: "
            ))
            .await?;
    if save {
        let mut binding = chosen.clone();
        binding.chain = chain.clone();
        let venue = match &ctx.venue {
            VenueKind::Cex(v) => Some(v.as_str()),
            VenueKind::Dex => None,
        };
        store.save(&binding, venue)?;
    }
    Ok(resolved_from(chosen, Source::Picked { saved: save }))
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
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)?;
    writeln!(f, "{line}")
}

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
            price_usd: None,
            liquidity_usd: None,
        }
    }

    struct Fake(std::result::Result<Vec<Candidate>, String>);
    #[async_trait::async_trait]
    impl CandidateSource for Fake {
        async fn candidates(&self, _s: &str, _c: Option<&str>) -> Result<Vec<Candidate>> {
            match &self.0 {
                Ok(v) => Ok(v.clone()),
                Err(e) => Err(ScopeError::Network(e.clone())),
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
            Self {
                pick,
                confirm,
                asked: Mutex::new(Vec::new()),
            }
        }
        fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }
    #[async_trait::async_trait]
    impl Picker for Scripted {
        async fn pick(&self, input: &str, cs: &[Candidate]) -> Result<usize> {
            self.asked
                .lock()
                .unwrap()
                .push(format!("pick {input} {}", cs.len()));
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
            self.0
                .push(binding(&c.chain, c.address.as_deref().unwrap_or(""), venue));
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
        ResolveContext {
            chain: Some("ethereum".into()),
            venue,
            token_flag: None,
            yes: false,
            save: false,
        }
    }

    fn cex() -> VenueKind {
        VenueKind::Cex("weex".into())
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
        let r = resolve("USDN", &cx, &Fake(Ok(vec![])), &picker, &mut store, &out())
            .await
            .unwrap();
        assert_eq!(r.address.as_deref(), Some("0xFLAG"));
        assert!(matches!(r.source, Source::Flag));
        assert!(picker.asked().is_empty());
    }

    #[tokio::test]
    async fn test_an_address_passes_through() {
        let addr = "0xeC747DAf41A4Ca4ebE72356592F8301924132907";
        let picker = Scripted::new(None, false);
        let r = resolve(
            addr,
            &ctx(VenueKind::Dex),
            &Fake(Ok(vec![])),
            &picker,
            &mut Store::default(),
            &out(),
        )
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
        let r = resolve(
            "usdn",
            &ctx(VenueKind::Dex),
            &two,
            &picker,
            &mut store,
            &out(),
        )
        .await
        .unwrap();
        assert_eq!(r.address.as_deref(), Some("0xSAVED"));
        assert!(matches!(r.source, Source::Binding));
        assert!(picker.asked().is_empty());
    }

    #[tokio::test]
    async fn test_a_binding_the_venue_contradicts_stops_with_both_contracts() {
        let mut store = Store(vec![binding("ethereum", "0xAAA", Some("weex"))]);
        let venue = Fake(Ok(vec![c("ETH", "0xBBB")]));
        let err = resolve(
            "USDN",
            &ctx(cex()),
            &venue,
            &Scripted::new(None, false),
            &mut store,
            &out(),
        )
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
        let r = resolve(
            "USDN",
            &ctx(cex()),
            &venue,
            &Scripted::new(None, false),
            &mut store,
            &out(),
        )
        .await
        .unwrap();
        assert_eq!(r.address.as_deref(), Some("0xAAA"));
    }

    #[tokio::test]
    async fn test_evm_contracts_compare_without_case() {
        // Review focus 3.
        let mut store = Store(vec![binding(
            "ethereum",
            "0xeC747DAf41A4Ca4ebE72356592F8301924132907",
            None,
        )]);
        let venue = Fake(Ok(vec![c(
            "ETH",
            "0xec747daf41a4ca4ebe72356592f8301924132907",
        )]));
        assert!(
            resolve(
                "USDN",
                &ctx(cex()),
                &venue,
                &Scripted::new(None, false),
                &mut store,
                &out()
            )
            .await
            .is_ok()
        );
    }

    #[tokio::test]
    async fn test_a_binding_is_used_when_the_venue_check_fails() {
        // Review focus 4: a network error must not block a saved binding.
        let (o, cap) = Output::capture();
        let mut store = Store(vec![binding("ethereum", "0xAAA", None)]);
        let venue = Fake(Err("down".into()));
        let r = resolve(
            "USDN",
            &ctx(cex()),
            &venue,
            &Scripted::new(None, false),
            &mut store,
            &o,
        )
        .await
        .unwrap();
        assert_eq!(r.address.as_deref(), Some("0xAAA"));
        assert!(cap.err().contains("could not be checked"), "{}", cap.err());
    }

    #[tokio::test]
    async fn test_no_candidates_stops_and_names_the_source() {
        let err = resolve(
            "USDN",
            &ctx(VenueKind::Dex),
            &Fake(Ok(vec![])),
            &Scripted::new(None, false),
            &mut Store::default(),
            &out(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ScopeError::TokenNotFound { .. }));
        assert!(err.to_string().contains("the fake source"));
    }

    #[tokio::test]
    async fn test_one_candidate_is_used_without_a_question() {
        let picker = Scripted::new(None, false);
        let r = resolve(
            "USDN",
            &ctx(VenueKind::Dex),
            &Fake(Ok(vec![c("ethereum", "0xA")])),
            &picker,
            &mut Store::default(),
            &out(),
        )
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
        let r = resolve(
            "USDN",
            &ctx(VenueKind::Dex),
            &Fake(Ok(vec![other, c("ethereum", "0xA")])),
            &picker,
            &mut Store::default(),
            &out(),
        )
        .await
        .unwrap();
        assert_eq!(r.address.as_deref(), Some("0xA"));
        assert!(picker.asked().is_empty(), "one exact match: no question");
    }

    #[tokio::test]
    async fn test_two_candidates_ask_then_save_on_yes() {
        let mut store = Store::default();
        let picker = Scripted::new(Some(1), true);
        let r = resolve(
            "USDN",
            &ctx(cex()),
            &Fake(Ok(vec![c("ETH", "0xA"), c("BSC", "0xB")])),
            &picker,
            &mut store,
            &out(),
        )
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
    async fn test_no_saves_nothing_and_the_save_flag_skips_the_question() {
        let two = || Fake(Ok(vec![c("ethereum", "0xA"), c("bsc", "0xB")]));
        let mut store = Store::default();
        let picker = Scripted::new(Some(0), false);
        resolve(
            "USDN",
            &ctx(VenueKind::Dex),
            &two(),
            &picker,
            &mut store,
            &out(),
        )
        .await
        .unwrap();
        assert!(store.0.is_empty());

        // --save answers yes without a question.
        let mut cx = ctx(VenueKind::Dex);
        cx.save = true;
        let picker = Scripted::new(Some(0), false);
        resolve("USDN", &cx, &two(), &picker, &mut store, &out())
            .await
            .unwrap();
        assert_eq!(store.0.len(), 1);
        assert!(!picker.asked().iter().any(|a| a.starts_with("confirm")));
    }

    #[tokio::test]
    async fn test_yes_with_an_ambiguous_symbol_stops_with_the_candidates() {
        // Jon, 2026-10-15: --yes never picks the first match.
        let mut cx = ctx(VenueKind::Dex);
        cx.yes = true;
        let err = resolve(
            "USDN",
            &cx,
            &Fake(Ok(vec![c("ethereum", "0xA"), c("bsc", "0xB")])),
            &Scripted::new(Some(0), true),
            &mut Store::default(),
            &out(),
        )
        .await
        .unwrap_err();
        let text = err.to_string();
        assert!(matches!(err, ScopeError::AmbiguousToken { .. }), "{text}");
        assert!(
            text.contains("ethereum 0xA") && text.contains("bsc 0xB"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn test_no_terminal_picker_lists_every_candidate_and_the_hint() {
        let err = NoTerminalPicker
            .pick("USDN", &[c("ethereum", "0xA"), c("bsc", "0xB")])
            .await
            .unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("ethereum 0xA") && text.contains("bsc 0xB"),
            "{text}"
        );
        assert!(text.contains("--token"), "{text}");
        assert!(!NoTerminalPicker.confirm("save?").await.unwrap());
    }

    #[tokio::test]
    async fn test_picker_for_json_never_prompts() {
        // A JSON run must not wait for stdin.
        let (o, _cap) = Output::capture();
        let p = picker_for(&o, true);
        let r = p
            .pick("USDN", &[c("ethereum", "0xA"), c("bsc", "0xB")])
            .await;
        assert!(matches!(r, Err(ScopeError::AmbiguousToken { .. })));
    }

    #[test]
    fn test_venue_candidates_are_one_per_network() {
        use scope::market::assets::{AssetInfo, AssetNetwork};
        let assets = vec![
            AssetInfo {
                symbol: "USDN".into(),
                networks: vec![
                    AssetNetwork {
                        network: "ETH".into(),
                        contract: Some("0xA".into()),
                    },
                    AssetNetwork {
                        network: "BSC".into(),
                        contract: Some("0xB".into()),
                    },
                ],
            },
            AssetInfo {
                symbol: "BTC".into(),
                networks: vec![],
            },
        ];
        let cs = venue_candidates_from("usdn", &assets);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].address.as_deref(), Some("0xA"));
        assert_eq!(cs[0].chain, "ETH", "the venue's name; resolve maps it");
    }

    #[test]
    fn test_dex_results_become_candidates_with_liquidity() {
        let r = scope::chains::dex::TokenSearchResult {
            address: "0xA".into(),
            symbol: "USDN".into(),
            name: "USDN".into(),
            chain: "ethereum".into(),
            price_usd: None,
            volume_24h: 0.0,
            liquidity_usd: 5.0,
            market_cap: None,
        };
        assert_eq!(dex_candidate(&r).liquidity_usd, Some(5.0));
        // The CEX ticker fallback has no address: no contract.
        let mut f = r.clone();
        f.address = String::new();
        assert_eq!(dex_candidate(&f).address, None);
    }

    #[test]
    fn test_the_alias_file_is_a_binding_store() {
        let mut aliases = TokenAliases::default();
        let store: &mut dyn BindingStore = &mut aliases;
        store.save(&c("bsc", "0xB"), Some("weex")).unwrap();
        let got = store.get("usdn", Some("bsc")).unwrap();
        assert_eq!(
            (got.address.as_str(), got.venue.as_deref()),
            ("0xB", Some("weex"))
        );
    }

    #[tokio::test]
    async fn test_each_resolution_writes_one_log_line() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("resolutions.jsonl");
        let picker = Scripted::new(None, false);
        resolve_logged(
            "USDN",
            &ctx(VenueKind::Dex),
            &Fake(Ok(vec![c("ethereum", "0xA")])),
            &picker,
            &mut Store::default(),
            &out(),
            &log,
        )
        .await
        .unwrap();
        let _ = resolve_logged(
            "NOPE",
            &ctx(VenueKind::Dex),
            &Fake(Ok(vec![])),
            &picker,
            &mut Store::default(),
            &out(),
            &log,
        )
        .await;
        let lines: Vec<serde_json::Value> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["source"], "single");
        assert_eq!(lines[0]["chosen"], "0xA");
        assert!(lines[1]["error"].as_str().unwrap().contains("NOPE"));
    }
}
