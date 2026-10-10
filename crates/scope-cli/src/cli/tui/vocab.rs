//! # Completion Vocabulary
//!
//! The values that completion offers besides commands and flags: address-book
//! labels, saved token aliases, venue IDs and chains. They load once when the
//! TUI starts. A store that fails to load gives an empty list, so completion
//! of commands and flags keeps working.

use super::exec::CHAINS;
use scope::config::Config;
use scope::domain::address_book::AddressBook;
use scope::market::VenueRegistry;
use scope::tokens::TokenAliases;

/// Dynamic completion values.
#[derive(Debug, Clone, Default)]
pub struct Vocab {
    /// Address-book labels, without the `@`.
    pub labels: Vec<String>,
    /// Saved token alias symbols.
    pub aliases: Vec<String>,
    /// Venue IDs from the registry.
    pub venues: Vec<String>,
    /// Chain names.
    pub chains: Vec<String>,
}

impl Vocab {
    /// Loads the vocabulary from the user's stores.
    pub fn load(config: &Config) -> Self {
        let mut labels: Vec<String> = AddressBook::load(&config.data_dir())
            .map(|b| b.addresses.into_iter().filter_map(|a| a.label).collect())
            .unwrap_or_default();
        labels.sort();
        labels.dedup();
        let mut aliases: Vec<String> = TokenAliases::load()
            .list()
            .into_iter()
            .map(|t| t.symbol.clone())
            .collect();
        aliases.sort();
        aliases.dedup();
        let venues = VenueRegistry::load()
            .map(|r| r.list().into_iter().map(str::to_string).collect())
            .unwrap_or_default();
        Self {
            labels,
            aliases,
            venues,
            chains: CHAINS.iter().map(|c| c.to_string()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_with_missing_stores_still_has_chains() {
        // Review Focus 3: no address book, no data dir. Completion of
        // chains must still work.
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.address_book.data_dir = Some(dir.path().join("nothing-here"));
        let v = Vocab::load(&config);
        assert!(v.labels.is_empty());
        assert!(v.chains.iter().any(|c| c == "ethereum"));
    }

    #[test]
    fn test_load_with_corrupt_address_book_gives_empty_labels() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("address_book.yaml"), "addresses: [unclosed").unwrap();
        let mut config = Config::default();
        config.address_book.data_dir = Some(dir.path().to_path_buf());
        let v = Vocab::load(&config);
        assert!(v.labels.is_empty());
    }

    #[test]
    fn test_labels_come_from_the_address_book() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("address_book.yaml"),
            "addresses:\n  - address: '0x1'\n    label: cold\n    chain: ethereum\n    tags: []\n    added_at: 0\n",
        )
        .unwrap();
        let mut config = Config::default();
        config.address_book.data_dir = Some(dir.path().to_path_buf());
        assert_eq!(Vocab::load(&config).labels, vec!["cold"]);
    }
}
