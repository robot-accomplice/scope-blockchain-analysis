//! Which token a venue's symbol is: the asset's networks and contracts,
//! from the venue's coin list (the `assets` capability).
//!
//! A venue symbol such as `USDN` names one asset on that venue, but not
//! which token it is. The report shows the contract, so a reader can check
//! it against the issuer's.

use serde::Serialize;

/// One network of an asset and the token contract on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AssetNetwork {
    /// The venue's network name (e.g. `ETH`).
    pub network: String,
    /// The token contract, or `None` for the network's native coin.
    pub contract: Option<String>,
}

/// An asset in the venue's coin list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AssetInfo {
    pub symbol: String,
    pub networks: Vec<AssetNetwork>,
}

/// The line the report shows for one token of the pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenAddress {
    pub symbol: String,
    pub network: String,
    pub contract: Option<String>,
    /// Networks the venue lists for the asset that this line does not show.
    pub other_networks: usize,
}

/// The shortest string read as a contract. Venues put a placeholder such
/// as `btc` in the contract field of a native coin.
const MIN_CONTRACT_LEN: usize = 20;

/// The contract from a venue's contract field, or `None` for a native coin.
pub fn contract_of(raw: &str) -> Option<String> {
    let raw = raw.trim();
    (raw.len() >= MIN_CONTRACT_LEN).then(|| raw.to_string())
}

/// The lines for the pair's tokens. The base shows every network the venue
/// lists. The quote (often listed on many networks) shows the network it
/// shares with the base, else its first, and counts the others. A symbol
/// missing from the coin list has no line.
pub fn token_addresses(base: &str, quote: &str, assets: &[AssetInfo]) -> Vec<TokenAddress> {
    let find = |s: &str| assets.iter().find(|a| a.symbol.eq_ignore_ascii_case(s));
    let mut lines = Vec::new();
    let mut base_networks = Vec::new();
    if let Some(b) = find(base) {
        for n in &b.networks {
            base_networks.push(n.network.clone());
            lines.push(TokenAddress {
                symbol: b.symbol.clone(),
                network: n.network.clone(),
                contract: n.contract.clone(),
                other_networks: 0,
            });
        }
    }
    if let Some(q) = find(quote)
        && let Some(n) = q
            .networks
            .iter()
            .find(|n| base_networks.contains(&n.network))
            .or_else(|| q.networks.first())
    {
        lines.push(TokenAddress {
            symbol: q.symbol.clone(),
            network: n.network.clone(),
            contract: n.contract.clone(),
            other_networks: q.networks.len() - 1,
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(symbol: &str, networks: &[(&str, &str)]) -> AssetInfo {
        AssetInfo {
            symbol: symbol.to_string(),
            networks: networks
                .iter()
                .map(|(n, c)| AssetNetwork {
                    network: n.to_string(),
                    contract: contract_of(c),
                })
                .collect(),
        }
    }

    const USDN: &str = "0xeC747DAf41A4Ca4ebE72356592F8301924132907";
    const USDT_ETH: &str = "0xdAC17F958D2ee523a2206206994597C13D831ec7";

    #[test]
    fn test_the_quote_shows_the_network_it_shares_with_the_base() {
        // USDT is on many networks; the line a reader needs is the one on
        // the base token's network, with the rest counted.
        let assets = [
            asset("USDN", &[("ETH", USDN)]),
            asset(
                "USDT",
                &[
                    ("TRX", "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"),
                    ("ETH", USDT_ETH),
                    ("BSC", "0x55d398326f99059fF775485246999027B3197955"),
                ],
            ),
        ];
        let lines = token_addresses("usdn", "USDT", &assets);
        assert_eq!(lines.len(), 2);
        assert_eq!(
            (lines[0].symbol.as_str(), lines[0].network.as_str()),
            ("USDN", "ETH")
        );
        assert_eq!(lines[0].contract.as_deref(), Some(USDN));
        assert_eq!(lines[1].network, "ETH");
        assert_eq!(lines[1].contract.as_deref(), Some(USDT_ETH));
        assert_eq!(lines[1].other_networks, 2);
    }

    #[test]
    fn test_unlisted_symbols_and_native_coins() {
        let assets = [
            asset("BTC", &[("BTC", "btc")]),
            asset("USDT", &[("TRX", "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t")]),
        ];
        let lines = token_addresses("BTC", "USDT", &assets);
        // A placeholder such as "btc" is a native coin, not a contract.
        assert_eq!(lines[0].contract, None);
        // No shared network: the quote's first network.
        assert_eq!(lines[1].network, "TRX");
        assert!(token_addresses("NOPE", "ALSO", &assets).is_empty());
    }
}
