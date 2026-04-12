//! Integration tests for the domain layer: address book CRUD, label resolution,
//! and configuration loading.

use scope::config::{AddressBookConfig, Config};
use scope::domain::address_book::{AddressBook, WatchedAddress};
use std::io::Write;
use std::path::PathBuf;
use tempfile::TempDir;

// ============================================================================
// AddressBook CRUD
// ============================================================================

#[test]
fn address_book_create_add_find_remove() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().to_path_buf();

    let mut ab = AddressBook::default();
    assert!(ab.addresses.is_empty());

    // Add two addresses
    ab.add_address(WatchedAddress {
        address: "0xAAAABBBBCCCCDDDDEEEEFFFF0000111122223333".to_string(),
        label: Some("alice".to_string()),
        chain: "ethereum".to_string(),
        tags: vec!["personal".to_string()],
        added_at: 1000,
    })
    .unwrap();

    ab.add_address(WatchedAddress {
        address: "DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy".to_string(),
        label: Some("bob-sol".to_string()),
        chain: "solana".to_string(),
        tags: vec![],
        added_at: 2000,
    })
    .unwrap();

    assert_eq!(ab.addresses.len(), 2);

    // Find by address (case-insensitive)
    // The find should be case-insensitive on the hex chars
    let found = ab.find_address("0xaaaabbbbccccddddeeeeffff0000111122223333");
    assert!(found.is_some());
    assert_eq!(found.unwrap().label.as_deref(), Some("alice"));

    // Find by label
    let by_label = ab.find_by_label("bob-sol");
    assert!(by_label.is_some());
    assert_eq!(by_label.unwrap().chain, "solana");

    // Find by label is case-insensitive
    let by_label_upper = ab.find_by_label("BOB-SOL");
    assert!(by_label_upper.is_some());

    // Remove address
    let removed = ab
        .remove_address("DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy")
        .unwrap();
    assert!(removed);
    assert_eq!(ab.addresses.len(), 1);

    // Remove nonexistent returns false
    let not_removed = ab.remove_address("0xdeadbeef").unwrap();
    assert!(!not_removed);

    // Save and reload
    ab.save(&data_dir).unwrap();
    let loaded = AddressBook::load(tmp.path()).unwrap();
    assert_eq!(loaded.addresses.len(), 1);
    assert_eq!(loaded.addresses[0].label.as_deref(), Some("alice"));
}

#[test]
fn address_book_persistence_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().to_path_buf();

    let mut ab = AddressBook::default();
    for i in 0..5 {
        ab.add_address(WatchedAddress {
            address: format!("0x{:040x}", i),
            label: Some(format!("wallet-{}", i)),
            chain: "ethereum".to_string(),
            tags: vec!["batch".to_string()],
            added_at: 1000 + i,
        })
        .unwrap();
    }

    ab.save(&data_dir).unwrap();

    let loaded = AddressBook::load(tmp.path()).unwrap();
    assert_eq!(loaded.addresses.len(), 5);

    // Verify order and content preserved
    for i in 0..5 {
        assert_eq!(
            loaded.addresses[i].label.as_deref(),
            Some(format!("wallet-{}", i).as_str())
        );
        assert_eq!(loaded.addresses[i].added_at, 1000 + i as u64);
    }
}

#[test]
fn address_book_duplicate_rejected() {
    let mut ab = AddressBook::default();
    ab.add_address(WatchedAddress {
        address: "0xABCDEF".to_string(),
        label: Some("first".to_string()),
        chain: "ethereum".to_string(),
        tags: vec![],
        added_at: 0,
    })
    .unwrap();

    // Same address different case should be rejected
    let err = ab
        .add_address(WatchedAddress {
            address: "0xabcdef".to_string(),
            label: Some("second".to_string()),
            chain: "ethereum".to_string(),
            tags: vec![],
            added_at: 1,
        })
        .unwrap_err();

    assert!(err.to_string().contains("already in address book"));
}

#[test]
fn address_book_load_nonexistent_dir_returns_empty() {
    let tmp = TempDir::new().unwrap();
    // Don't create any file — should return default empty book
    let ab = AddressBook::load(tmp.path()).unwrap();
    assert!(ab.addresses.is_empty());
}

// ============================================================================
// resolve_address_book_input
// ============================================================================

#[test]
fn resolve_label_returns_address_and_chain() {
    use scope::domain::address_book::resolve_address_book_input;

    let tmp = TempDir::new().unwrap();
    let mut ab = AddressBook::default();
    ab.add_address(WatchedAddress {
        address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
        label: Some("main-wallet".to_string()),
        chain: "polygon".to_string(),
        tags: vec![],
        added_at: 0,
    })
    .unwrap();
    ab.save(&tmp.path().to_path_buf()).unwrap();

    let config = Config {
        address_book: AddressBookConfig {
            data_dir: Some(tmp.path().to_path_buf()),
        },
        ..Default::default()
    };

    let result = resolve_address_book_input("@main-wallet", &config).unwrap();
    assert!(result.is_some());
    let (addr, chain) = result.unwrap();
    assert_eq!(addr, "0x1234567890abcdef1234567890abcdef12345678");
    assert_eq!(chain, "polygon");
}

#[test]
fn resolve_direct_address_passthrough() {
    use scope::domain::address_book::resolve_address_book_input;

    let tmp = TempDir::new().unwrap();
    let config = Config {
        address_book: AddressBookConfig {
            data_dir: Some(tmp.path().to_path_buf()),
        },
        ..Default::default()
    };

    // Raw address without @ should return None (no match in empty book)
    let result = resolve_address_book_input("0xSomeRandomAddress", &config).unwrap();
    assert!(result.is_none());
}

#[test]
fn resolve_nonexistent_label_returns_error() {
    use scope::domain::address_book::resolve_address_book_input;

    let tmp = TempDir::new().unwrap();
    let config = Config {
        address_book: AddressBookConfig {
            data_dir: Some(tmp.path().to_path_buf()),
        },
        ..Default::default()
    };

    let err = resolve_address_book_input("@does-not-exist", &config).unwrap_err();
    assert!(err.to_string().contains("@does-not-exist"));
}

// ============================================================================
// Config loading
// ============================================================================

#[test]
fn config_load_partial_yaml_fills_defaults() {
    let yaml = r#"
chains:
  ethereum_rpc: "https://test.example.com"
output:
  format: json
"#;
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(yaml.as_bytes()).unwrap();

    let config = Config::load(Some(file.path())).unwrap();

    assert_eq!(
        config.chains.ethereum_rpc,
        Some("https://test.example.com".into())
    );
    assert_eq!(config.output.format, scope::config::OutputFormat::Json);
    // Defaults should be filled in
    assert!(config.chains.solana_rpc.is_none());
    assert!(config.chains.api_keys.is_empty());
    assert!(config.output.color); // default is true
}

#[test]
fn config_web_defaults() {
    let config = Config::default();
    assert!(config.web.auth_token.is_none());
}

#[test]
fn config_web_from_yaml() {
    let yaml = r#"
web:
  auth_token: "my-secret-token"
"#;
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(yaml.as_bytes()).unwrap();

    let config = Config::load(Some(file.path())).unwrap();
    assert_eq!(config.web.auth_token.as_deref(), Some("my-secret-token"));
}

#[test]
fn config_ghola_defaults() {
    let config = Config::default();
    assert!(!config.ghola.enabled);
    assert!(!config.ghola.stealth);
    assert_eq!(config.ghola.buffer_size, 4096);
}

#[test]
fn config_ghola_partial_yaml() {
    let yaml = r#"
ghola:
  enabled: true
"#;
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(yaml.as_bytes()).unwrap();

    let config = Config::load(Some(file.path())).unwrap();
    assert!(config.ghola.enabled);
    assert!(!config.ghola.stealth); // default false
    assert_eq!(config.ghola.buffer_size, 4096); // default
}

#[test]
fn config_missing_file_returns_defaults() {
    let config = Config::load(Some(std::path::Path::new(
        "/tmp/nonexistent-scope-config.yaml",
    )))
    .unwrap();
    assert_eq!(config, Config::default());
}

#[test]
fn config_data_dir_override() {
    let config = Config {
        address_book: AddressBookConfig {
            data_dir: Some(PathBuf::from("/custom/data")),
        },
        ..Default::default()
    };
    assert_eq!(config.data_dir(), PathBuf::from("/custom/data"));
}

#[test]
fn config_data_dir_default_fallback() {
    let config = Config::default();
    let data_dir = config.data_dir();
    // Should contain "scope" somewhere in the path
    assert!(data_dir.to_string_lossy().contains("scope"));
}
