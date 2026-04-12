//! Address book domain logic.
//!
//! Core data types (`AddressBook`, `WatchedAddress`) and business logic
//! (load, save, CRUD, label resolution) shared by the CLI and web layers.

use crate::config::Config;
use crate::error::{Result, ScopeError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A watched address in the address book.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchedAddress {
    /// The blockchain address.
    pub address: String,

    /// Human-readable label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// Blockchain network.
    pub chain: String,

    /// Tags for categorization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// When the address was added (Unix timestamp).
    pub added_at: u64,
}

/// AddressBook data storage.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AddressBook {
    /// All watched addresses.
    pub addresses: Vec<WatchedAddress>,
}

/// AddressBook summary report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressBookSummary {
    /// Total number of addresses.
    pub address_count: usize,

    /// Balances by chain.
    pub balances_by_chain: HashMap<String, ChainBalance>,

    /// Total address book value in USD (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_usd: Option<f64>,

    /// Individual address summaries.
    pub addresses: Vec<AddressSummary>,
}

/// Balance summary for a chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainBalance {
    /// Native token balance.
    pub native_balance: String,

    /// Native token symbol.
    pub symbol: String,

    /// USD value (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usd: Option<f64>,
}

/// Summary for a single address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressSummary {
    /// The address.
    pub address: String,

    /// Label (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// Chain.
    pub chain: String,

    /// Native balance.
    pub balance: String,

    /// USD value (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usd: Option<f64>,

    /// Token balances (for chains that support SPL/ERC20 tokens).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tokens: Vec<TokenSummary>,
}

/// Summary for a token balance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSummary {
    /// Token contract/mint address.
    pub contract_address: String,
    /// Token balance (human-readable).
    pub balance: String,
    /// Token decimals.
    pub decimals: u8,
    /// Token symbol (if known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
}

impl AddressBook {
    /// Loads the address book from the data directory.
    pub fn load(data_dir: &std::path::Path) -> Result<Self> {
        let path = data_dir.join("address_book.yaml");

        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(&path)?;
        let address_book: AddressBook = serde_yaml::from_str(&contents)
            .map_err(|e| ScopeError::Config(crate::error::ConfigError::Parse { source: e }))?;

        Ok(address_book)
    }

    /// Saves the address book to the data directory.
    pub fn save(&self, data_dir: &PathBuf) -> Result<()> {
        std::fs::create_dir_all(data_dir)?;

        let path = data_dir.join("address_book.yaml");
        let contents = serde_yaml::to_string(self)
            .map_err(|e| ScopeError::Export(format!("Failed to serialize address book: {}", e)))?;

        std::fs::write(&path, contents)?;
        Ok(())
    }

    /// Adds an address to the address book.
    pub fn add_address(&mut self, watched: WatchedAddress) -> Result<()> {
        // Check for duplicates
        if self
            .addresses
            .iter()
            .any(|a| a.address.to_lowercase() == watched.address.to_lowercase())
        {
            return Err(ScopeError::Chain(format!(
                "Address already in address book: {}",
                watched.address
            )));
        }

        self.addresses.push(watched);
        Ok(())
    }

    /// Removes an address from the address book.
    pub fn remove_address(&mut self, address: &str) -> Result<bool> {
        let original_len = self.addresses.len();
        self.addresses
            .retain(|a| a.address.to_lowercase() != address.to_lowercase());

        Ok(self.addresses.len() < original_len)
    }

    /// Finds an address in the address book by address string.
    pub fn find_address(&self, address: &str) -> Option<&WatchedAddress> {
        self.addresses
            .iter()
            .find(|a| a.address.to_lowercase() == address.to_lowercase())
    }

    /// Finds an address in the address book by its label (case-insensitive).
    ///
    /// Returns the first matching entry. Labels are compared after
    /// lowercasing and trimming whitespace.
    pub fn find_by_label(&self, label: &str) -> Option<&WatchedAddress> {
        let needle = label.trim().to_lowercase();
        self.addresses.iter().find(|a| {
            a.label
                .as_ref()
                .is_some_and(|l| l.trim().to_lowercase() == needle)
        })
    }
}

/// Resolves a user-supplied input string against the address book.
///
/// **Label lookup (requires `@` prefix):** If the input starts with `@`, the remainder
/// is looked up as a label. Example: `@main-wallet` resolves to the address with
/// label "main-wallet". This convention distinguishes label lookups from raw addresses.
///
/// **Address match (no `@`):** If the input does not start with `@`, only direct
/// address matching is attempted (to inject chain info from the address book).
/// Raw addresses and token identifiers are not treated as labels.
///
/// Returns `Ok(Some((address, chain)))` when resolved, `Ok(None)` when no `@` prefix
/// and no address match, or `Err` when the `@` prefix was used but the label wasn't found.
pub fn resolve_address_book_input(
    input: &str,
    config: &Config,
) -> crate::error::Result<Option<(String, String)>> {
    let data_dir = config.data_dir();
    let address_book = match AddressBook::load(&data_dir) {
        Ok(ab) => ab,
        Err(_) => return Ok(None),
    };

    // If input starts with @, strip it and look up remainder as label
    if let Some(label) = input.strip_prefix('@') {
        if let Some(watched) = address_book.find_by_label(label) {
            let label_display = watched.label.as_deref().unwrap_or(label);
            eprintln!(
                "  Using '{}' → {} ({})",
                label_display, watched.address, watched.chain
            );
            return Ok(Some((watched.address.clone(), watched.chain.clone())));
        }
        // List available labels to help the user
        let available: Vec<String> = address_book
            .addresses
            .iter()
            .filter_map(|a| a.label.clone())
            .collect();
        let suggestion = if available.is_empty() {
            "Your address book is empty. Add entries with `scope address-book add`.".to_string()
        } else {
            format!(
                "Available labels: {}",
                available
                    .iter()
                    .map(|l| format!("@{}", l))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        return Err(crate::error::ScopeError::NotFound(format!(
            "No address book entry matching '@{}'.\n      {}",
            label, suggestion
        )));
    }

    // No @ prefix: only try address match (inject chain info from address book)
    if let Some(watched) = address_book.find_address(input) {
        if let Some(ref label) = watched.label {
            tracing::debug!(
                "Address book match by address for '{}' ({})",
                label,
                watched.chain
            );
        }
        return Ok(Some((watched.address.clone(), watched.chain.clone())));
    }

    Ok(None)
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_address_book() -> AddressBook {
        AddressBook {
            addresses: vec![
                WatchedAddress {
                    address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
                    label: Some("Main Wallet".to_string()),
                    chain: "ethereum".to_string(),
                    tags: vec!["personal".to_string()],
                    added_at: 1700000000,
                },
                WatchedAddress {
                    address: "0xABCdef1234567890abcdef1234567890ABCDEF12".to_string(),
                    label: None,
                    chain: "polygon".to_string(),
                    tags: vec![],
                    added_at: 1700000001,
                },
            ],
        }
    }

    #[test]
    fn test_address_book_default() {
        let address_book = AddressBook::default();
        assert!(address_book.addresses.is_empty());
    }

    #[test]
    fn test_address_book_add_address() {
        let mut address_book = AddressBook::default();

        let watched = WatchedAddress {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            label: Some("Test".to_string()),
            chain: "ethereum".to_string(),
            tags: vec![],
            added_at: 0,
        };

        let result = address_book.add_address(watched);
        assert!(result.is_ok());
        assert_eq!(address_book.addresses.len(), 1);
    }

    #[test]
    fn test_address_book_add_duplicate_fails() {
        let mut address_book = AddressBook::default();

        let watched1 = WatchedAddress {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            label: Some("First".to_string()),
            chain: "ethereum".to_string(),
            tags: vec![],
            added_at: 0,
        };

        let watched2 = WatchedAddress {
            address: "0x742d35CC6634C0532925A3b844Bc9e7595f1b3c2".to_string(), // Same address, different case
            label: Some("Second".to_string()),
            chain: "ethereum".to_string(),
            tags: vec![],
            added_at: 0,
        };

        address_book.add_address(watched1).unwrap();
        let result = address_book.add_address(watched2);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("already in address book")
        );
    }

    #[test]
    fn test_address_book_remove_address() {
        let mut address_book = create_test_address_book();
        let original_len = address_book.addresses.len();

        let removed = address_book
            .remove_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2")
            .unwrap();

        assert!(removed);
        assert_eq!(address_book.addresses.len(), original_len - 1);
    }

    #[test]
    fn test_address_book_remove_nonexistent() {
        let mut address_book = create_test_address_book();
        let original_len = address_book.addresses.len();

        let removed = address_book.remove_address("0xnonexistent").unwrap();

        assert!(!removed);
        assert_eq!(address_book.addresses.len(), original_len);
    }

    #[test]
    fn test_address_book_find_address() {
        let address_book = create_test_address_book();

        let found = address_book.find_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2");
        assert!(found.is_some());
        assert_eq!(found.unwrap().label, Some("Main Wallet".to_string()));

        let not_found = address_book.find_address("0xnonexistent");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_address_book_find_address_case_insensitive() {
        let address_book = create_test_address_book();

        let found = address_book.find_address("0x742D35CC6634C0532925A3B844BC9E7595F1B3C2");
        assert!(found.is_some());
    }

    #[test]
    fn test_address_book_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();

        let address_book = create_test_address_book();
        address_book.save(&data_dir).unwrap();

        let loaded = AddressBook::load(&data_dir).unwrap();
        assert_eq!(loaded.addresses.len(), address_book.addresses.len());
        assert_eq!(
            loaded.addresses[0].address,
            address_book.addresses[0].address
        );
    }

    #[test]
    fn test_address_book_load_nonexistent_returns_default() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();

        let address_book = AddressBook::load(&data_dir).unwrap();
        assert!(address_book.addresses.is_empty());
    }

    #[test]
    fn test_watched_address_serialization() {
        let watched = WatchedAddress {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            label: Some("Test".to_string()),
            chain: "ethereum".to_string(),
            tags: vec!["tag1".to_string(), "tag2".to_string()],
            added_at: 1700000000,
        };

        let json = serde_json::to_string(&watched).unwrap();
        assert!(json.contains("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"));
        assert!(json.contains("Test"));
        assert!(json.contains("tag1"));

        let deserialized: WatchedAddress = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.address, watched.address);
        assert_eq!(deserialized.tags.len(), 2);
    }

    #[test]
    fn test_address_book_summary_serialization() {
        let summary = AddressBookSummary {
            address_count: 2,
            balances_by_chain: HashMap::new(),
            total_usd: Some(10000.0),
            addresses: vec![AddressSummary {
                address: "0x123".to_string(),
                label: Some("Test".to_string()),
                chain: "ethereum".to_string(),
                balance: "1.5".to_string(),
                usd: Some(5000.0),
                tokens: vec![],
            }],
        };

        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("10000"));
        assert!(json.contains("0x123"));
    }

    #[test]
    fn test_chain_balance_serialization() {
        let balance = ChainBalance {
            native_balance: "10.5".to_string(),
            symbol: "ETH".to_string(),
            usd: Some(35000.0),
        };

        let json = serde_json::to_string(&balance).unwrap();
        assert!(json.contains("10.5"));
        assert!(json.contains("ETH"));
        assert!(json.contains("35000"));
    }
}
