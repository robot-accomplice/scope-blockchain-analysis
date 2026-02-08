//! # Token Alias Storage
//!
//! This module provides storage and retrieval of token aliases,
//! allowing users to reference tokens by friendly names instead
//! of full contract addresses.
//!
//! ## Storage Location
//!
//! Token aliases are stored in `~/.local/share/scope/tokens.yaml`
//!
//! ## Usage
//!
//! ```rust,no_run
//! use scope::tokens::TokenAliases;
//!
//! // Load existing aliases
//! let mut aliases = TokenAliases::load();
//!
//! // Add an alias
//! aliases.add("USDC", "ethereum", "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", "USD Coin");
//!
//! // Look up an alias
//! if let Some(info) = aliases.get("USDC", Some("ethereum")) {
//!     println!("USDC address: {}", info.address);
//! }
//!
//! // Save aliases
//! aliases.save().unwrap();
//! ```

use crate::error::{Result, ScopeError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A saved token alias with its address and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    /// Token contract address.
    pub address: String,

    /// Token symbol.
    pub symbol: String,

    /// Token name.
    pub name: String,

    /// Blockchain network.
    pub chain: String,

    /// When this alias was last used.
    #[serde(default)]
    pub last_used: Option<i64>,
}

/// Collection of token aliases.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenAliases {
    /// Map of alias -> chain -> token info.
    /// Using nested maps to support same symbol on different chains.
    #[serde(default)]
    aliases: HashMap<String, HashMap<String, TokenInfo>>,

    /// Recent tokens for quick access.
    #[serde(default)]
    recent: Vec<TokenInfo>,
}

impl TokenAliases {
    /// Returns the path to the token aliases file.
    pub fn aliases_path() -> Option<PathBuf> {
        dirs::data_dir().map(|p| p.join("scope").join("tokens.yaml"))
    }

    /// Loads token aliases from disk.
    pub fn load() -> Self {
        Self::aliases_path()
            .and_then(|path| std::fs::read_to_string(&path).ok())
            .and_then(|contents| serde_yaml::from_str(&contents).ok())
            .unwrap_or_default()
    }

    /// Saves token aliases to disk.
    pub fn save(&self) -> Result<()> {
        if let Some(path) = Self::aliases_path() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    ScopeError::Io(format!("Failed to create token aliases directory: {}", e))
                })?;
            }
            let contents = serde_yaml::to_string(self)
                .map_err(|e| ScopeError::Export(format!("Failed to serialize aliases: {}", e)))?;
            std::fs::write(&path, contents)
                .map_err(|e| ScopeError::Io(format!("Failed to write token aliases: {}", e)))?;
        }
        Ok(())
    }

    /// Adds or updates a token alias.
    ///
    /// # Arguments
    ///
    /// * `alias` - The friendly name/symbol to use (case-insensitive)
    /// * `chain` - The blockchain network
    /// * `address` - The token contract address
    /// * `name` - The full token name
    pub fn add(&mut self, alias: &str, chain: &str, address: &str, name: &str) {
        let alias_key = alias.to_uppercase();
        let chain_key = chain.to_lowercase();

        let info = TokenInfo {
            address: address.to_string(),
            symbol: alias.to_uppercase(),
            name: name.to_string(),
            chain: chain_key.clone(),
            last_used: Some(chrono::Utc::now().timestamp()),
        };

        // Add to aliases map
        self.aliases
            .entry(alias_key)
            .or_default()
            .insert(chain_key, info.clone());

        // Add to recent (remove existing first, then add to front)
        self.recent
            .retain(|t| !(t.symbol == info.symbol && t.chain == info.chain));
        self.recent.insert(0, info);

        // Keep only last 20 recent
        self.recent.truncate(20);
    }

    /// Looks up a token alias.
    ///
    /// # Arguments
    ///
    /// * `alias` - The alias to look up (case-insensitive)
    /// * `chain` - Optional chain filter. If None, returns first match.
    ///
    /// # Returns
    ///
    /// Returns the token info if found.
    pub fn get(&self, alias: &str, chain: Option<&str>) -> Option<&TokenInfo> {
        let alias_key = alias.to_uppercase();

        if let Some(chain_map) = self.aliases.get(&alias_key) {
            if let Some(chain) = chain {
                let chain_key = chain.to_lowercase();
                chain_map.get(&chain_key)
            } else {
                // Return the first one (or prefer ethereum if available)
                chain_map
                    .get("ethereum")
                    .or_else(|| chain_map.values().next())
            }
        } else {
            None
        }
    }

    /// Gets all chains that have this alias defined.
    pub fn get_chains_for_alias(&self, alias: &str) -> Vec<&str> {
        let alias_key = alias.to_uppercase();
        self.aliases
            .get(&alias_key)
            .map(|chain_map| chain_map.keys().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Returns recent tokens.
    pub fn recent(&self) -> &[TokenInfo] {
        &self.recent
    }

    /// Removes an alias.
    pub fn remove(&mut self, alias: &str, chain: Option<&str>) {
        let alias_key = alias.to_uppercase();

        if let Some(chain) = chain {
            let chain_key = chain.to_lowercase();
            if let Some(chain_map) = self.aliases.get_mut(&alias_key) {
                chain_map.remove(&chain_key);
                if chain_map.is_empty() {
                    self.aliases.remove(&alias_key);
                }
            }
            self.recent
                .retain(|t| !(t.symbol == alias_key && t.chain == chain_key));
        } else {
            self.aliases.remove(&alias_key);
            self.recent.retain(|t| t.symbol != alias_key);
        }
    }

    /// Lists all saved aliases.
    pub fn list(&self) -> Vec<&TokenInfo> {
        self.aliases
            .values()
            .flat_map(|chain_map| chain_map.values())
            .collect()
    }

    /// Checks if the input looks like a token address or a name.
    pub fn is_address(input: &str) -> bool {
        // EVM address: 0x + 40 hex chars
        if input.starts_with("0x") && input.len() == 42 {
            return input[2..].chars().all(|c| c.is_ascii_hexdigit());
        }

        // Solana address: base58, 32-44 chars
        if input.len() >= 32
            && input.len() <= 44
            && let Ok(decoded) = bs58::decode(input).into_vec()
            && decoded.len() == 32
        {
            return true;
        }

        // Tron address: T + 33 chars
        if input.starts_with('T') && input.len() == 34 {
            return bs58::decode(input).into_vec().is_ok();
        }

        false
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_aliases_default() {
        let aliases = TokenAliases::default();
        assert!(aliases.aliases.is_empty());
        assert!(aliases.recent.is_empty());
    }

    #[test]
    fn test_add_and_get_alias() {
        let mut aliases = TokenAliases::default();
        aliases.add(
            "USDC",
            "ethereum",
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
            "USD Coin",
        );

        let info = aliases.get("USDC", Some("ethereum")).unwrap();
        assert_eq!(info.symbol, "USDC");
        assert_eq!(info.chain, "ethereum");
        assert!(info.address.starts_with("0x"));

        // Case insensitive lookup
        let info2 = aliases.get("usdc", Some("ethereum")).unwrap();
        assert_eq!(info2.symbol, "USDC");
    }

    #[test]
    fn test_get_without_chain() {
        let mut aliases = TokenAliases::default();
        aliases.add("USDC", "ethereum", "0xETH...", "USD Coin");
        aliases.add("USDC", "polygon", "0xPOLY...", "USD Coin");

        // Should prefer ethereum
        let info = aliases.get("USDC", None).unwrap();
        assert_eq!(info.chain, "ethereum");
    }

    #[test]
    fn test_remove_alias() {
        let mut aliases = TokenAliases::default();
        aliases.add("USDC", "ethereum", "0x...", "USD Coin");
        aliases.add("USDC", "polygon", "0x...", "USD Coin");

        // Remove specific chain
        aliases.remove("USDC", Some("ethereum"));
        assert!(aliases.get("USDC", Some("ethereum")).is_none());
        assert!(aliases.get("USDC", Some("polygon")).is_some());

        // Remove all chains
        aliases.remove("USDC", None);
        assert!(aliases.get("USDC", None).is_none());
    }

    #[test]
    fn test_is_address() {
        // EVM addresses
        assert!(TokenAliases::is_address(
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
        ));
        assert!(TokenAliases::is_address(
            "0x0000000000000000000000000000000000000000"
        ));

        // Not addresses
        assert!(!TokenAliases::is_address("USDC"));
        assert!(!TokenAliases::is_address("ethereum"));
        assert!(!TokenAliases::is_address("0x123")); // Too short
    }

    #[test]
    fn test_recent_tokens() {
        let mut aliases = TokenAliases::default();
        aliases.add("USDC", "ethereum", "0x1...", "USD Coin");
        aliases.add("WETH", "ethereum", "0x2...", "Wrapped Ether");

        assert_eq!(aliases.recent().len(), 2);
        // Most recent first
        assert_eq!(aliases.recent()[0].symbol, "WETH");
    }

    #[test]
    fn test_list_aliases() {
        let mut aliases = TokenAliases::default();
        aliases.add("USDC", "ethereum", "0x1...", "USD Coin");
        aliases.add("USDC", "polygon", "0x2...", "USD Coin");
        aliases.add("WETH", "ethereum", "0x3...", "Wrapped Ether");

        let list = aliases.list();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn test_get_chains_for_alias() {
        let mut aliases = TokenAliases::default();
        aliases.add("USDC", "ethereum", "0x1...", "USD Coin");
        aliases.add("USDC", "polygon", "0x2...", "USD Coin");

        let chains = aliases.get_chains_for_alias("USDC");
        assert_eq!(chains.len(), 2);
        assert!(chains.contains(&"ethereum"));
        assert!(chains.contains(&"polygon"));
    }

    #[test]
    fn test_get_chains_for_missing_alias() {
        let aliases = TokenAliases::default();
        let chains = aliases.get_chains_for_alias("NONEXISTENT");
        assert!(chains.is_empty());
    }

    #[test]
    fn test_is_address_solana() {
        // Valid Solana address (base58, 32-44 chars, decodes to 32 bytes)
        assert!(TokenAliases::is_address(
            "DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy"
        ));
        // System program address
        assert!(TokenAliases::is_address("11111111111111111111111111111111"));
    }

    #[test]
    fn test_is_address_tron() {
        // Valid Tron address (starts with T, 34 chars)
        assert!(TokenAliases::is_address(
            "TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf"
        ));
        assert!(TokenAliases::is_address(
            "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"
        ));
    }

    #[test]
    fn test_is_address_edge_cases() {
        assert!(!TokenAliases::is_address("")); // Empty
        assert!(!TokenAliases::is_address("0x")); // Incomplete EVM prefix
        assert!(!TokenAliases::is_address("T123")); // Too short for Tron
        assert!(!TokenAliases::is_address("hello world")); // Random text
    }

    #[test]
    fn test_remove_specific_chain() {
        let mut aliases = TokenAliases::default();
        aliases.add("USDC", "ethereum", "0x1...", "USD Coin");
        aliases.add("USDC", "polygon", "0x2...", "USD Coin");

        // Remove only polygon
        aliases.remove("USDC", Some("polygon"));
        assert!(aliases.get("USDC", Some("polygon")).is_none());
        assert!(aliases.get("USDC", Some("ethereum")).is_some());
    }

    #[test]
    fn test_remove_last_chain_cleans_up() {
        let mut aliases = TokenAliases::default();
        aliases.add("USDC", "ethereum", "0x1...", "USD Coin");

        // Removing the only chain should clean up the alias entirely
        aliases.remove("USDC", Some("ethereum"));
        assert!(aliases.get("USDC", None).is_none());
        let chains = aliases.get_chains_for_alias("USDC");
        assert!(chains.is_empty());
    }

    #[test]
    fn test_remove_cleans_recent() {
        let mut aliases = TokenAliases::default();
        aliases.add("USDC", "ethereum", "0x1...", "USD Coin");
        assert_eq!(aliases.recent().len(), 1);

        aliases.remove("USDC", None);
        assert!(aliases.recent().is_empty());
    }

    #[test]
    fn test_add_updates_existing() {
        let mut aliases = TokenAliases::default();
        aliases.add("USDC", "ethereum", "0x1...", "USD Coin");
        aliases.add("USDC", "ethereum", "0x2...", "USD Coin V2");

        let info = aliases.get("USDC", Some("ethereum")).unwrap();
        assert_eq!(info.address, "0x2...");
        assert_eq!(info.name, "USD Coin V2");
    }

    #[test]
    fn test_recent_truncation() {
        let mut aliases = TokenAliases::default();
        // Add 25 tokens, recent should be capped at 20
        for i in 0..25 {
            aliases.add(
                &format!("T{}", i),
                "ethereum",
                &format!("0x{}...", i),
                &format!("Token {}", i),
            );
        }
        assert_eq!(aliases.recent().len(), 20);
        // The most recent should be T24
        assert_eq!(aliases.recent()[0].symbol, "T24");
    }

    #[test]
    fn test_case_insensitive_operations() {
        let mut aliases = TokenAliases::default();
        aliases.add("usdc", "Ethereum", "0x1...", "USD Coin");

        // Alias stored uppercase, chain stored lowercase
        let info = aliases.get("USDC", Some("ethereum")).unwrap();
        assert_eq!(info.symbol, "USDC");
        assert_eq!(info.chain, "ethereum");
    }

    #[test]
    fn test_token_info_has_last_used() {
        let mut aliases = TokenAliases::default();
        aliases.add("USDC", "ethereum", "0x1...", "USD Coin");
        let info = aliases.get("USDC", Some("ethereum")).unwrap();
        assert!(info.last_used.is_some());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let mut aliases = TokenAliases::default();
        aliases.add("SAVE_TEST", "ethereum", "0xsave...", "Save Test Token");

        // save() writes to the standard location which should be writable in test env
        let result = aliases.save();
        assert!(result.is_ok());

        // Load it back
        let loaded = TokenAliases::load();
        let info = loaded.get("SAVE_TEST", Some("ethereum"));
        assert!(info.is_some());
        assert_eq!(info.unwrap().address, "0xsave...");

        // Cleanup: remove the test entry and save again
        aliases.remove("SAVE_TEST", None);
        let _ = aliases.save();
    }
}
