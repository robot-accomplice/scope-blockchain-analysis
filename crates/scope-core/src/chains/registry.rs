//! # Chain registry (L2)
//!
//! The one table of chains: the canonical id, the aliases users type, the
//! EVM chain id, the explorer, the native token, the DexScreener id and the
//! wrapped native token. Chain metadata, the DEX lookups, the contract
//! source and Sourcify checks, the EVM client and the CLI's lists of
//! supported chains all read it.

/// How scope talks to a chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainKind {
    /// An EVM chain behind the Etherscan v2 API (or a JSON-RPC node).
    Evm,
    Solana,
    Tron,
    /// Known to DexScreener only: prices, no chain client.
    DexOnly,
}

/// One chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chain {
    /// Canonical id (`ethereum`).
    pub id: &'static str,
    /// Other names users type (`eth`), lower case.
    pub aliases: &'static [&'static str],
    pub kind: ChainKind,
    /// The EVM chain id, for Etherscan v2 and Sourcify (`"1"`).
    pub evm_chain_id: Option<&'static str>,
    /// The config key of the chain's explorer API key.
    pub api_key_name: Option<&'static str>,
    pub native_symbol: &'static str,
    pub native_decimals: u8,
    /// Explorer base for token pages.
    pub explorer_token_base: Option<&'static str>,
    /// Explorer base for address pages.
    pub explorer_address_base: Option<&'static str>,
    /// The chain's id at DexScreener.
    pub dexscreener_id: Option<&'static str>,
    /// The wrapped native token, which prices the native token on a DEX.
    pub wrapped_native: Option<&'static str>,
}

const fn evm(
    id: &'static str,
    aliases: &'static [&'static str],
    chain_id: &'static str,
    key: &'static str,
    symbol: &'static str,
    explorer: (&'static str, &'static str),
    wrapped: &'static str,
) -> Chain {
    Chain {
        id,
        aliases,
        kind: ChainKind::Evm,
        evm_chain_id: Some(chain_id),
        api_key_name: Some(key),
        native_symbol: symbol,
        native_decimals: 18,
        explorer_token_base: Some(explorer.0),
        explorer_address_base: Some(explorer.1),
        dexscreener_id: Some(id),
        wrapped_native: Some(wrapped),
    }
}

/// Every chain scope knows, in the order lists show them.
pub const CHAINS: &[Chain] = &[
    evm(
        "ethereum",
        &["eth"],
        "1",
        "etherscan",
        "ETH",
        ("https://etherscan.io/token", "https://etherscan.io/address"),
        "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
    ),
    evm(
        "polygon",
        &["matic"],
        "137",
        "polygonscan",
        "MATIC",
        (
            "https://polygonscan.com/token",
            "https://polygonscan.com/address",
        ),
        "0x0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270",
    ),
    evm(
        "arbitrum",
        &["arb"],
        "42161",
        "arbiscan",
        "ETH",
        ("https://arbiscan.io/token", "https://arbiscan.io/address"),
        "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1",
    ),
    evm(
        "optimism",
        &["op"],
        "10",
        "optimism",
        "ETH",
        (
            "https://optimistic.etherscan.io/token",
            "https://optimistic.etherscan.io/address",
        ),
        "0x4200000000000000000000000000000000000006",
    ),
    evm(
        "base",
        &[],
        "8453",
        "basescan",
        "ETH",
        ("https://basescan.org/token", "https://basescan.org/address"),
        "0x4200000000000000000000000000000000000006",
    ),
    evm(
        "bsc",
        &["bnb"],
        "56",
        "bscscan",
        "BNB",
        ("https://bscscan.com/token", "https://bscscan.com/address"),
        "0xbb4CdB9CBd36B01bD1cBaEBF2De08d9173bc095c",
    ),
    Chain {
        id: "aegis",
        aliases: &[],
        kind: ChainKind::Evm,
        // A JSON-RPC node (config `aegis_rpc`), not Etherscan.
        evm_chain_id: None,
        api_key_name: None,
        native_symbol: "WRAITH",
        native_decimals: 18,
        explorer_token_base: None,
        explorer_address_base: None,
        dexscreener_id: None,
        wrapped_native: None,
    },
    Chain {
        id: "solana",
        aliases: &["sol"],
        kind: ChainKind::Solana,
        evm_chain_id: None,
        api_key_name: None,
        native_symbol: "SOL",
        native_decimals: 9,
        explorer_token_base: Some("https://solscan.io/token"),
        explorer_address_base: Some("https://solscan.io/account"),
        dexscreener_id: Some("solana"),
        wrapped_native: Some("So11111111111111111111111111111111111111112"),
    },
    Chain {
        id: "tron",
        aliases: &["trx"],
        kind: ChainKind::Tron,
        evm_chain_id: None,
        api_key_name: None,
        native_symbol: "TRX",
        native_decimals: 6,
        explorer_token_base: Some("https://tronscan.org/#/token20"),
        explorer_address_base: Some("https://tronscan.org/#/address"),
        dexscreener_id: Some("tron"),
        // The wrapped TRX token differs by DEX: no one address.
        wrapped_native: None,
    },
    Chain {
        id: "avalanche",
        aliases: &["avax"],
        kind: ChainKind::DexOnly,
        evm_chain_id: None,
        api_key_name: None,
        native_symbol: "AVAX",
        native_decimals: 18,
        explorer_token_base: None,
        explorer_address_base: None,
        dexscreener_id: Some("avalanche"),
        wrapped_native: None,
    },
];

/// The chain named `name` (its id or an alias, any case).
pub fn chain(name: &str) -> Option<&'static Chain> {
    let name = name.trim().to_lowercase();
    CHAINS
        .iter()
        .find(|c| c.id == name || c.aliases.contains(&name.as_str()))
}

/// The ids of the chains with a chain client (address, tx, contract):
/// everything but the DEX-only chains.
pub fn client_chain_ids() -> Vec<&'static str> {
    CHAINS
        .iter()
        .filter(|c| c.kind != ChainKind::DexOnly)
        .map(|c| c.id)
        .collect()
}

/// [`client_chain_ids`] joined for help and error text.
pub fn client_chain_list() -> String {
    client_chain_ids().join(", ")
}

/// The EVM chains with an Etherscan-compatible API, joined for help text.
pub fn explorer_api_chain_list() -> String {
    CHAINS
        .iter()
        .filter(|c| c.evm_chain_id.is_some())
        .map(|c| c.id)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The `--chain` help of commands that take any chain with a client.
pub fn chain_help() -> String {
    format!("Target blockchain network: {}", client_chain_list())
}

/// True when `name` has a chain client and a block explorer (token info and
/// holders): not a JSON-RPC-only chain, not a DEX-only one.
pub fn has_explorer(name: &str) -> bool {
    chain(name).is_some_and(|c| c.kind != ChainKind::DexOnly && c.explorer_token_base.is_some())
}

/// True when `name` is an EVM chain with a client.
pub fn is_evm(name: &str) -> bool {
    chain(name).is_some_and(|c| c.kind == ChainKind::Evm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ids_and_aliases_resolve_in_any_case() {
        assert_eq!(chain("ETH").unwrap().id, "ethereum");
        assert_eq!(chain("matic").unwrap().id, "polygon");
        assert_eq!(chain(" Sol ").unwrap().id, "solana");
        assert!(chain("nosuch").is_none());
    }

    #[test]
    fn test_every_alias_names_one_chain_only() {
        let mut names: Vec<&str> = CHAINS
            .iter()
            .flat_map(|c| std::iter::once(c.id).chain(c.aliases.iter().copied()))
            .collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "a name is listed twice");
    }

    #[test]
    fn test_the_client_list_has_aegis_and_not_the_dex_only_chains() {
        let ids = client_chain_ids();
        assert!(ids.contains(&"aegis"));
        assert!(!ids.contains(&"avalanche"));
        assert!(is_evm("arb") && !is_evm("solana"));
    }
}
