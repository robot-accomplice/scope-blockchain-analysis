//! Market venues: the DEX venues (books built from DexScreener pools) and
//! the CEX venues of the [`VenueRegistry`]. One parser decides which is which,
//! and one list holds every venue id.

use super::exchange::ExchangeClient;
use super::registry::{VenueRegistry, unknown_venue};
use crate::chains::chain_metadata;
use crate::error::{Result, ScopeError};
use serde::Serialize;

/// Help for every `--venue` that takes DEX venues (market summary,
/// token-health). One text, so no surface lists the DEX venues by hand.
pub const VENUE_HELP: &str = "Market venue: a CEX id (run `scope venues list`), or a DEX: \
    ethereum (eth) or solana (sol), whose book is built from the token's most liquid pool.";

/// The venue a market request falls back to when none is given.
pub const DEFAULT_VENUE: &str = "binance";

/// A DEX venue. Its book is synthesized from the most liquid pool of the
/// token on one chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DexVenue {
    /// Ethereum DEX pools.
    Ethereum,
    /// Solana DEX pools.
    Solana,
}

impl DexVenue {
    /// Every DEX venue, in display order.
    pub const ALL: [DexVenue; 2] = [DexVenue::Ethereum, DexVenue::Solana];

    /// Parses a venue or chain name with the chain registry
    /// ([`chain_metadata`]), so every chain alias it knows (`eth`, `sol`, any
    /// case) works here too. A chain without a DEX venue is `None`.
    pub fn parse(input: &str) -> Option<Self> {
        let chain = chain_metadata(input.trim())?.chain_id;
        Self::ALL.into_iter().find(|venue| venue.chain() == chain)
    }

    /// Canonical id: the canonical chain id (`ethereum`, `solana`).
    pub fn id(self) -> &'static str {
        self.chain()
    }

    /// The canonical id of the chain whose pools this venue reads.
    pub fn chain(self) -> &'static str {
        match self {
            Self::Ethereum => "ethereum",
            Self::Solana => "solana",
        }
    }

    /// Display name, e.g. `Ethereum DEX`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ethereum => "Ethereum DEX",
            Self::Solana => "Solana DEX",
        }
    }

    /// Checks that `chain` names this venue's chain (any alias, any case).
    /// Another chain is [`ScopeError::InvalidInput`]: the request asks for a
    /// book from pools this venue does not read.
    pub fn require_chain(self, chain: &str) -> Result<()> {
        if Self::parse(chain) == Some(self) {
            return Ok(());
        }
        Err(ScopeError::InvalidInput(format!(
            "DEX venue '{}' reads chain {}, not {}",
            self.id(),
            self.chain(),
            chain
        )))
    }
}

/// Where a market summary comes from: DEX pools or a CEX order book.
pub enum MarketVenue {
    /// Synthetic book from DEX pools.
    Dex(DexVenue),
    /// Order book of a registry venue.
    Cex(Box<ExchangeClient>),
}

/// The kind of a listed venue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VenueKind {
    /// A registry venue with its own order book API.
    Cex,
    /// A DEX venue with a synthetic book.
    Dex,
}

/// One row of the venue list, as `scope venues list` and `/api/venues` show it.
#[derive(Debug, Clone, Serialize)]
pub struct VenueListing {
    /// Venue id to pass as `--venue`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// CEX or DEX.
    pub kind: VenueKind,
    /// Capabilities (`order_book`, `ticker`, ...). A DEX venue has a
    /// synthetic order book only.
    pub capabilities: Vec<&'static str>,
    /// API base URL (CEX only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Chain whose pools build the book (DEX only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain: Option<&'static str>,
}

impl MarketVenue {
    /// Resolves a venue name. DEX names win; any other name must be a
    /// registry venue. An unknown name is [`ScopeError::InvalidInput`] that
    /// lists every market venue, DEX venues included.
    pub fn resolve(input: &str, registry: &VenueRegistry) -> Result<Self> {
        if let Some(dex) = DexVenue::parse(input) {
            return Ok(Self::Dex(dex));
        }
        if registry.contains(input) {
            return Ok(Self::Cex(Box::new(registry.create_exchange_client(input)?)));
        }
        Err(unknown_venue(input, &Self::ids(registry)))
    }

    /// [`MarketVenue::resolve`] against the loaded registry (built-in and
    /// user venues).
    pub fn from_name(input: &str) -> Result<Self> {
        Self::resolve(input, &VenueRegistry::load()?)
    }

    /// Checks an explicit chain against the venue. A CEX venue reads no
    /// chain, so any chain passes.
    pub fn require_chain(&self, chain: &str) -> Result<()> {
        match self {
            Self::Dex(dex) => dex.require_chain(chain),
            Self::Cex(_) => Ok(()),
        }
    }

    /// Every market venue: the registry venues (sorted), then the DEX venues.
    pub fn listings(registry: &VenueRegistry) -> Vec<VenueListing> {
        let cex = registry.list().into_iter().filter_map(|id| {
            registry.get(id).map(|desc| VenueListing {
                id: desc.id.clone(),
                name: desc.name.clone(),
                kind: VenueKind::Cex,
                capabilities: desc.capability_names(),
                base_url: Some(desc.base_url.clone()),
                chain: None,
            })
        });
        let dex = DexVenue::ALL.into_iter().map(|venue| VenueListing {
            id: venue.id().to_string(),
            name: venue.name().to_string(),
            kind: VenueKind::Dex,
            capabilities: vec!["order_book"],
            base_url: None,
            chain: Some(venue.chain()),
        });
        cex.chain(dex).collect()
    }

    /// The ids of [`MarketVenue::listings`]: what `--venue` accepts where
    /// DEX venues are valid.
    pub fn ids(registry: &VenueRegistry) -> Vec<String> {
        Self::listings(registry)
            .into_iter()
            .map(|listing| listing.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dex_names_parse_in_any_case() {
        for (input, venue) in [
            ("eth", DexVenue::Ethereum),
            ("ethereum", DexVenue::Ethereum),
            ("Ethereum", DexVenue::Ethereum),
            ("solana", DexVenue::Solana),
            ("Solana", DexVenue::Solana),
        ] {
            assert_eq!(DexVenue::parse(input), Some(venue), "{input}");
        }
    }

    /// `sol` is the chain map's short form, so it is a DEX venue too.
    #[test]
    fn sol_is_the_solana_dex() {
        assert_eq!(DexVenue::parse("sol"), Some(DexVenue::Solana));
        assert_eq!(DexVenue::parse("SOL"), Some(DexVenue::Solana));
        assert_eq!(DexVenue::Solana.chain(), "solana");
    }

    /// Before: an unknown name mapped to `ethereum` without an error.
    #[test]
    fn other_names_are_not_dex_venues() {
        for input in ["binance", "okx", "mexc", "kraken", "polygon", "unknown", ""] {
            assert_eq!(DexVenue::parse(input), None, "{input}");
        }
    }

    #[test]
    fn chain_check_accepts_any_spelling_of_the_venue_chain() {
        assert!(DexVenue::Ethereum.require_chain("ethereum").is_ok());
        assert!(DexVenue::Ethereum.require_chain("ETH").is_ok());
        assert!(DexVenue::Solana.require_chain("Sol").is_ok());
        let err = DexVenue::Solana.require_chain("ethereum").unwrap_err();
        assert!(err.to_string().contains("reads chain solana"), "{err}");
    }

    #[test]
    fn resolve_sends_dex_names_to_dex_and_others_to_the_registry() {
        let registry = VenueRegistry::load().unwrap();
        assert!(matches!(
            MarketVenue::resolve("sol", &registry).unwrap(),
            MarketVenue::Dex(DexVenue::Solana)
        ));
        assert!(matches!(
            MarketVenue::resolve("binance", &registry).unwrap(),
            MarketVenue::Cex(_)
        ));
    }

    #[test]
    fn an_unknown_venue_is_an_error_that_lists_the_dex_venues() {
        let registry = VenueRegistry::load().unwrap();
        let err = MarketVenue::resolve("polygon", &registry)
            .err()
            .expect("unknown venue must be an error");
        let msg = err.to_string();
        assert!(msg.contains("Unknown venue \"polygon\""), "{msg}");
        assert!(msg.contains(", ethereum, solana\n"), "{msg}");
        assert!(matches!(err, ScopeError::InvalidInput(_)), "{err:?}");
    }

    /// A lookup that cannot serve DEX venues (ohlc, trades, monitor) never
    /// suggests them; the unknown name is the one request-fault kind.
    #[test]
    fn the_registry_lookup_does_not_suggest_dex_venues() {
        let registry = VenueRegistry::load().unwrap();
        let err = registry.create_exchange_client("solana").err().unwrap();
        assert!(matches!(err, ScopeError::InvalidInput(_)), "{err:?}");
        let msg = err.to_string();
        assert!(msg.contains("Unknown venue \"solana\""), "{msg}");
        assert!(!msg.contains("ethereum"), "{msg}");
    }

    #[test]
    fn a_typo_of_a_dex_venue_is_suggested_where_dex_venues_are_valid() {
        let registry = VenueRegistry::load().unwrap();
        let err = MarketVenue::resolve("solanna", &registry).err().unwrap();
        assert!(err.to_string().contains("Did you mean \"solana\""), "{err}");
    }

    #[test]
    fn one_list_holds_the_registry_and_the_dex_venues() {
        let registry = VenueRegistry::load().unwrap();
        let ids = MarketVenue::ids(&registry);
        let cex: Vec<String> = registry.list().into_iter().map(str::to_string).collect();
        assert_eq!(ids[..cex.len()], cex[..]);
        assert_eq!(ids[cex.len()..], ["ethereum", "solana"]);
        let last = MarketVenue::listings(&registry).pop().unwrap();
        assert_eq!(last.kind, VenueKind::Dex);
        assert_eq!(last.chain, Some("solana"));
    }

    #[test]
    fn a_chain_mismatch_is_a_request_fault() {
        let err = DexVenue::Solana.require_chain("ethereum").unwrap_err();
        assert!(matches!(err, ScopeError::InvalidInput(_)), "{err:?}");
    }

    /// Aliases come from the chain registry: any alias it knows works here.
    #[test]
    fn dex_aliases_come_from_chain_metadata() {
        for alias in ["eth", "ETH", "sol", " Solana "] {
            let venue = DexVenue::parse(alias).unwrap();
            let chain = chain_metadata(alias.trim()).unwrap().chain_id;
            assert_eq!(chain, venue.chain());
        }
        assert_eq!(
            DexVenue::parse("polygon"),
            None,
            "a chain without a DEX venue"
        );
    }

    #[test]
    fn a_cex_venue_ignores_the_chain() {
        let registry = VenueRegistry::load().unwrap();
        let venue = MarketVenue::resolve("binance", &registry).unwrap();
        assert!(venue.require_chain("solana").is_ok());
    }
}
