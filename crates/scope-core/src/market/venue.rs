//! Market venues: the DEX venues (books built from DexScreener pools) and
//! the CEX venues of the [`VenueRegistry`]. One parser decides which is which.

use super::exchange::ExchangeClient;
use super::registry::VenueRegistry;
use crate::error::{Result, ScopeError};

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

    /// Parses a venue or chain name, ignoring case. Accepts the chain name
    /// and its short form (`ethereum`/`eth`, `solana`/`sol`). Any other value
    /// is not a DEX venue.
    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "ethereum" | "eth" => Some(Self::Ethereum),
            "solana" | "sol" => Some(Self::Solana),
            _ => None,
        }
    }

    /// Canonical id, also the chain name (`ethereum`, `solana`).
    pub fn id(self) -> &'static str {
        match self {
            Self::Ethereum => "ethereum",
            Self::Solana => "solana",
        }
    }

    /// The chain whose pools this venue reads.
    pub fn chain(self) -> &'static str {
        self.id()
    }

    /// Display name, e.g. `Ethereum DEX`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ethereum => "Ethereum DEX",
            Self::Solana => "Solana DEX",
        }
    }

    /// Checks that `chain` names this venue's chain (any accepted spelling,
    /// any case). A book from another chain's pools is an error.
    pub fn require_chain(self, chain: &str) -> Result<()> {
        if Self::parse(chain) == Some(self) {
            return Ok(());
        }
        Err(ScopeError::Chain(format!(
            "DEX venue '{}' reads chain {}, not {}",
            self.id(),
            self.chain(),
            chain
        )))
    }

    /// The canonical ids, comma-separated, for help and error text.
    pub fn id_list() -> String {
        Self::ALL.map(Self::id).join(", ")
    }
}

/// Where a market summary comes from: DEX pools or a CEX order book.
pub enum MarketVenue {
    /// Synthetic book from DEX pools.
    Dex(DexVenue),
    /// Order book of a registry venue.
    Cex(Box<ExchangeClient>),
}

impl MarketVenue {
    /// Resolves a venue name. DEX names win; any other name must be a
    /// registry venue, or the result is the registry's unknown-venue error.
    pub fn resolve(input: &str, registry: &VenueRegistry) -> Result<Self> {
        match DexVenue::parse(input) {
            Some(dex) => Ok(Self::Dex(dex)),
            None => Ok(Self::Cex(Box::new(registry.create_exchange_client(input)?))),
        }
    }

    /// Checks an explicit chain against the venue. A CEX venue reads no
    /// chain, so any chain passes.
    pub fn require_chain(&self, chain: &str) -> Result<()> {
        match self {
            Self::Dex(dex) => dex.require_chain(chain),
            Self::Cex(_) => Ok(()),
        }
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
        assert!(msg.contains("DEX venues: ethereum, solana"), "{msg}");
    }

    #[test]
    fn a_cex_venue_ignores_the_chain() {
        let registry = VenueRegistry::load().unwrap();
        let venue = MarketVenue::resolve("binance", &registry).unwrap();
        assert!(venue.require_chain("solana").is_ok());
    }
}
