//! # Common Test Utilities
//!
//! Shared utilities for integration tests.

use std::path::PathBuf;
use tempfile::TempDir;

/// Creates a temporary directory for test data.
pub fn create_temp_dir() -> TempDir {
    tempfile::tempdir().expect("Failed to create temp directory")
}

/// Creates a test configuration file.
pub fn create_test_config(dir: &TempDir, content: &str) -> PathBuf {
    let config_dir = dir.path().join(".config").join("scope");
    std::fs::create_dir_all(&config_dir).expect("Failed to create config directory");
    
    let config_path = config_dir.join("config.yaml");
    std::fs::write(&config_path, content).expect("Failed to write config file");
    
    config_path
}

/// Creates a test address book file.
pub fn create_test_address_book(dir: &TempDir, content: &str) -> PathBuf {
    let data_dir = dir.path().join(".local").join("share").join("scope");
    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");
    
    let address_book_path = data_dir.join("address_book.yaml");
    std::fs::write(&address_book_path, content).expect("Failed to write address book file");
    
    address_book_path
}

/// Valid Ethereum address for testing.
pub const TEST_ADDRESS: &str = "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2";

/// Valid transaction hash for testing.
pub const TEST_TX_HASH: &str = "0xabc123def456789012345678901234567890123456789012345678901234abcd";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_temp_dir() {
        let temp_dir = create_temp_dir();
        assert!(temp_dir.path().exists());
    }

    #[test]
    fn test_create_test_config() {
        let temp_dir = create_temp_dir();
        let config_path = create_test_config(&temp_dir, "test: value");
        
        assert!(config_path.exists());
        let content = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(content, "test: value");
    }

    #[test]
    fn test_create_test_address_book() {
        let temp_dir = create_temp_dir();
        let address_book_path = create_test_address_book(&temp_dir, "addresses: []");
        
        assert!(address_book_path.exists());
        let content = std::fs::read_to_string(&address_book_path).unwrap();
        assert_eq!(content, "addresses: []");
    }

    #[test]
    fn test_constants() {
        assert!(TEST_ADDRESS.starts_with("0x"));
        assert_eq!(TEST_ADDRESS.len(), 42);
        
        assert!(TEST_TX_HASH.starts_with("0x"));
        assert_eq!(TEST_TX_HASH.len(), 66);
    }
}
