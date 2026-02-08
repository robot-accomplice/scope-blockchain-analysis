//! # Configuration Management
//!
//! This module handles loading, merging, and validating configuration
//! from multiple sources with the following priority (highest to lowest):
//!
//! 1. CLI arguments
//! 2. Environment variables (`BCC_*` prefix)
//! 3. User config file (`~/.config/bcc/config.yaml`)
//! 4. Built-in defaults
//!
//! ## Configuration File Format
//!
//! ```yaml
//! chains:
//!   # EVM-compatible chains
//!   ethereum_rpc: "https://mainnet.infura.io/v3/YOUR_KEY"
//!   bsc_rpc: "https://bsc-dataseed.binance.org"
//!   aegis_rpc: "http://localhost:8545"
//!
//!   # Non-EVM chains
//!   solana_rpc: "https://api.mainnet-beta.solana.com"
//!   tron_api: "https://api.trongrid.io"
//!
//!   api_keys:
//!     etherscan: "YOUR_API_KEY"
//!     bscscan: "YOUR_API_KEY"
//!     solscan: "YOUR_API_KEY"
//!     tronscan: "YOUR_API_KEY"
//!
//! output:
//!   format: table  # table, json, csv
//!   color: true
//!
//! portfolio:
//!   data_dir: "~/.local/share/bcc"
//! ```
//!
//! ## Error Handling
//!
//! Configuration errors are wrapped in [`BccError::Config`] and include
//! context about which source caused the failure.

use crate::error::{BccError, ConfigError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Application configuration.
///
/// Contains all settings for blockchain clients, output formatting,
/// and portfolio management. Use [`Config::load`] to load from file
/// or [`Config::default`] for sensible defaults.
///
/// # Examples
///
/// ```rust
/// use bcc::Config;
///
/// // Load from default location or custom path
/// let config = Config::load(None).unwrap_or_default();
/// println!("Output format: {:?}", config.output.format);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// Blockchain chain client configuration.
    pub chains: ChainsConfig,

    /// Output formatting configuration.
    pub output: OutputConfig,

    /// Portfolio management configuration.
    pub portfolio: PortfolioConfig,
}

/// Blockchain client configuration.
///
/// Contains RPC endpoints and API keys for various blockchain networks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ChainsConfig {
    // =========================================================================
    // EVM-Compatible Chains
    // =========================================================================
    /// Ethereum JSON-RPC endpoint URL.
    ///
    /// Example: `https://mainnet.infura.io/v3/YOUR_PROJECT_ID`
    pub ethereum_rpc: Option<String>,

    /// BSC (BNB Smart Chain) JSON-RPC endpoint URL.
    ///
    /// Example: `https://bsc-dataseed.binance.org`
    pub bsc_rpc: Option<String>,

    /// Aegis (Wraith) blockchain JSON-RPC endpoint URL.
    ///
    /// Example: `http://localhost:8545`
    pub aegis_rpc: Option<String>,

    // =========================================================================
    // Non-EVM Chains
    // =========================================================================
    /// Solana JSON-RPC endpoint URL.
    ///
    /// Example: `https://api.mainnet-beta.solana.com`
    pub solana_rpc: Option<String>,

    /// Tron API endpoint URL (TronGrid).
    ///
    /// Example: `https://api.trongrid.io`
    pub tron_api: Option<String>,

    // =========================================================================
    // API Keys
    // =========================================================================
    /// API keys for block explorer services.
    ///
    /// Keys are service names (e.g., "etherscan", "polygonscan", "bscscan",
    /// "solscan", "tronscan").
    pub api_keys: HashMap<String, String>,
}

/// Output formatting configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    /// Output format for analysis results.
    pub format: OutputFormat,

    /// Whether to use colored output in the terminal.
    pub color: bool,
}

/// Portfolio management configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PortfolioConfig {
    /// Directory for storing portfolio data.
    ///
    /// Defaults to `~/.local/share/bcc` on Linux/macOS.
    pub data_dir: Option<PathBuf>,
}

/// Available output formats for analysis results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Human-readable table format (default).
    #[default]
    Table,

    /// JSON format for programmatic consumption.
    Json,

    /// CSV format for spreadsheet import.
    Csv,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            format: OutputFormat::Table,
            color: true,
        }
    }
}

impl Config {
    /// Loads configuration from a file path or the default location.
    ///
    /// # Arguments
    ///
    /// * `path` - Optional path to a configuration file. If `None`, looks
    ///   for config at `~/.config/bcc/config.yaml`.
    ///
    /// # Returns
    ///
    /// Returns the loaded configuration, or defaults if no config file exists.
    ///
    /// # Errors
    ///
    /// Returns [`BccError::Config`] if the file exists but cannot be read
    /// or contains invalid YAML.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use bcc::Config;
    /// use std::path::Path;
    ///
    /// // Load from default location
    /// let config = Config::load(None).unwrap_or_default();
    ///
    /// // Load from custom path
    /// let config = Config::load(Some(Path::new("/custom/config.yaml")));
    /// ```
    pub fn load(path: Option<&Path>) -> Result<Self> {
        // Determine config path: CLI arg > env var > default location
        let config_path = path
            .map(PathBuf::from)
            .or_else(|| std::env::var("BCC_CONFIG").ok().map(PathBuf::from))
            .unwrap_or_else(Self::default_path);

        // Return defaults if no config file exists
        // This allows first-run without manual setup
        if !config_path.exists() {
            tracing::debug!(
                path = %config_path.display(),
                "No config file found, using defaults"
            );
            return Ok(Self::default());
        }

        tracing::debug!(path = %config_path.display(), "Loading configuration");

        let contents = std::fs::read_to_string(&config_path).map_err(|e| {
            BccError::Config(ConfigError::Read {
                path: config_path.clone(),
                source: e,
            })
        })?;

        let config: Config =
            serde_yaml::from_str(&contents).map_err(|e| ConfigError::Parse { source: e })?;

        Ok(config)
    }

    /// Returns the default configuration file path.
    ///
    /// Checks multiple locations in order:
    /// 1. `~/.config/bcc/config.yaml` (XDG-style, preferred for CLI tools)
    /// 2. Platform-specific config dir (e.g., `~/Library/Application Support/` on macOS)
    ///
    /// Returns the first location that exists, or the XDG-style path if neither exists.
    pub fn default_path() -> PathBuf {
        // Prefer XDG-style ~/.config/bcc/ which is common for CLI tools
        if let Some(home) = dirs::home_dir() {
            let xdg_path = home.join(".config").join("bcc").join("config.yaml");
            if xdg_path.exists() {
                return xdg_path;
            }
        }

        // Check platform-specific config dir
        if let Some(config_dir) = dirs::config_dir() {
            let platform_path = config_dir.join("bcc").join("config.yaml");
            if platform_path.exists() {
                return platform_path;
            }
        }

        // Default to XDG-style path for new configs
        dirs::home_dir()
            .map(|h| h.join(".config").join("bcc").join("config.yaml"))
            .unwrap_or_else(|| PathBuf::from(".").join("bcc").join("config.yaml"))
    }

    /// Returns the configuration file path, if it can be determined.
    ///
    /// Returns `Some(path)` for the config file location, or `None` if
    /// the path cannot be determined (e.g., no home directory).
    /// Prefers the XDG-style `~/.config/bcc/` location.
    pub fn config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".config").join("bcc").join("config.yaml"))
    }

    /// Returns the default data directory for portfolio storage.
    ///
    /// On Linux/macOS: `~/.local/share/bcc`
    /// On Windows: `%LOCALAPPDATA%\bca`
    pub fn default_data_dir() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("bcc")
    }

    /// Returns the effective data directory, using config or default.
    pub fn data_dir(&self) -> PathBuf {
        self.portfolio
            .data_dir
            .clone()
            .unwrap_or_else(Self::default_data_dir)
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputFormat::Table => write!(f, "table"),
            OutputFormat::Json => write!(f, "json"),
            OutputFormat::Csv => write!(f, "csv"),
        }
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_config() {
        let config = Config::default();

        assert!(config.chains.api_keys.is_empty());
        assert!(config.chains.ethereum_rpc.is_none());
        assert!(config.chains.bsc_rpc.is_none());
        assert!(config.chains.aegis_rpc.is_none());
        assert!(config.chains.solana_rpc.is_none());
        assert!(config.chains.tron_api.is_none());
        assert_eq!(config.output.format, OutputFormat::Table);
        assert!(config.output.color);
        assert!(config.portfolio.data_dir.is_none());
    }

    #[test]
    fn test_load_from_yaml_full() {
        let yaml = r#"
chains:
  ethereum_rpc: "https://example.com/rpc"
  bsc_rpc: "https://bsc-dataseed.binance.org"
  aegis_rpc: "http://localhost:8545"
  solana_rpc: "https://api.mainnet-beta.solana.com"
  tron_api: "https://api.trongrid.io"
  api_keys:
    etherscan: "test-api-key"
    polygonscan: "another-key"
    bscscan: "bsc-key"
    solscan: "sol-key"
    tronscan: "tron-key"

output:
  format: json
  color: false

portfolio:
  data_dir: "/custom/data"
"#;

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let config = Config::load(Some(file.path())).unwrap();

        // EVM chains
        assert_eq!(
            config.chains.ethereum_rpc,
            Some("https://example.com/rpc".into())
        );
        assert_eq!(
            config.chains.bsc_rpc,
            Some("https://bsc-dataseed.binance.org".into())
        );
        assert_eq!(
            config.chains.aegis_rpc,
            Some("http://localhost:8545".into())
        );

        // Non-EVM chains
        assert_eq!(
            config.chains.solana_rpc,
            Some("https://api.mainnet-beta.solana.com".into())
        );
        assert_eq!(
            config.chains.tron_api,
            Some("https://api.trongrid.io".into())
        );

        // API keys
        assert_eq!(
            config.chains.api_keys.get("etherscan"),
            Some(&"test-api-key".into())
        );
        assert_eq!(
            config.chains.api_keys.get("polygonscan"),
            Some(&"another-key".into())
        );
        assert_eq!(
            config.chains.api_keys.get("bscscan"),
            Some(&"bsc-key".into())
        );
        assert_eq!(
            config.chains.api_keys.get("solscan"),
            Some(&"sol-key".into())
        );
        assert_eq!(
            config.chains.api_keys.get("tronscan"),
            Some(&"tron-key".into())
        );

        assert_eq!(config.output.format, OutputFormat::Json);
        assert!(!config.output.color);
        assert_eq!(
            config.portfolio.data_dir,
            Some(PathBuf::from("/custom/data"))
        );
    }

    #[test]
    fn test_load_partial_yaml_uses_defaults() {
        let yaml = r#"
chains:
  ethereum_rpc: "https://partial.example.com"
"#;

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let config = Config::load(Some(file.path())).unwrap();

        // Specified value
        assert_eq!(
            config.chains.ethereum_rpc,
            Some("https://partial.example.com".into())
        );

        // Defaults
        assert!(config.chains.api_keys.is_empty());
        assert_eq!(config.output.format, OutputFormat::Table);
        assert!(config.output.color);
    }

    #[test]
    fn test_load_missing_file_returns_defaults() {
        let config = Config::load(Some(Path::new("/nonexistent/path/config.yaml"))).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn test_load_invalid_yaml_returns_error() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"invalid: yaml: : content: [").unwrap();

        let result = Config::load(Some(file.path()));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BccError::Config(_)));
    }

    #[test]
    fn test_load_empty_file_returns_defaults() {
        let file = NamedTempFile::new().unwrap();
        // Empty file

        let config = Config::load(Some(file.path())).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn test_output_format_serialization() {
        let json_format = OutputFormat::Json;
        let serialized = serde_yaml::to_string(&json_format).unwrap();
        assert!(serialized.contains("json"));

        let deserialized: OutputFormat = serde_yaml::from_str("csv").unwrap();
        assert_eq!(deserialized, OutputFormat::Csv);
    }

    #[test]
    fn test_output_format_display() {
        assert_eq!(OutputFormat::Table.to_string(), "table");
        assert_eq!(OutputFormat::Json.to_string(), "json");
        assert_eq!(OutputFormat::Csv.to_string(), "csv");
    }

    #[test]
    fn test_default_path_is_absolute_or_relative() {
        let path = Config::default_path();
        // Should end with expected structure
        assert!(path.ends_with("bcc/config.yaml") || path.ends_with("bcc\\config.yaml"));
    }

    #[test]
    fn test_default_data_dir() {
        let data_dir = Config::default_data_dir();
        assert!(data_dir.ends_with("bcc") || data_dir.to_string_lossy().contains("bcc"));
    }

    #[test]
    fn test_data_dir_uses_config_value() {
        let config = Config {
            portfolio: PortfolioConfig {
                data_dir: Some(PathBuf::from("/custom/path")),
            },
            ..Default::default()
        };

        assert_eq!(config.data_dir(), PathBuf::from("/custom/path"));
    }

    #[test]
    fn test_data_dir_falls_back_to_default() {
        let config = Config::default();
        assert_eq!(config.data_dir(), Config::default_data_dir());
    }

    #[test]
    fn test_config_clone_and_eq() {
        let config1 = Config::default();
        let config2 = config1.clone();
        assert_eq!(config1, config2);
    }

    #[test]
    fn test_config_path_returns_some() {
        let path = Config::config_path();
        // Should return Some on systems with a home dir
        assert!(path.is_some());
        assert!(path.unwrap().to_string_lossy().contains("bcc"));
    }

    #[test]
    fn test_config_debug() {
        let config = Config::default();
        let debug = format!("{:?}", config);
        assert!(debug.contains("Config"));
        assert!(debug.contains("ChainsConfig"));
    }

    #[test]
    fn test_output_config_default() {
        let output = OutputConfig::default();
        assert_eq!(output.format, OutputFormat::Table);
        assert!(output.color);
    }

    #[test]
    fn test_config_serialization_roundtrip() {
        let mut config = Config::default();
        config
            .chains
            .api_keys
            .insert("etherscan".to_string(), "test_key".to_string());
        config.output.format = OutputFormat::Json;
        config.output.color = false;
        config.portfolio.data_dir = Some(PathBuf::from("/custom"));

        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_chains_config_with_multiple_api_keys() {
        let mut api_keys = HashMap::new();
        api_keys.insert("etherscan".into(), "key1".into());
        api_keys.insert("polygonscan".into(), "key2".into());
        api_keys.insert("bscscan".into(), "key3".into());

        let chains = ChainsConfig {
            ethereum_rpc: Some("https://rpc.example.com".into()),
            api_keys,
            ..Default::default()
        };

        assert_eq!(chains.api_keys.len(), 3);
        assert!(chains.api_keys.contains_key("etherscan"));
    }

    #[test]
    fn test_load_via_bcc_config_env_var() {
        let yaml = r#"
chains:
  ethereum_rpc: "https://env-test.example.com"
output:
  format: csv
"#;
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let path_str = file.path().to_string_lossy().to_string();
        unsafe { std::env::set_var("BCC_CONFIG", &path_str) };

        // Load with None path — should pick up the env var
        let config = Config::load(None).unwrap();
        assert_eq!(
            config.chains.ethereum_rpc,
            Some("https://env-test.example.com".into())
        );
        assert_eq!(config.output.format, OutputFormat::Csv);

        unsafe { std::env::remove_var("BCC_CONFIG") };
    }

    #[test]
    fn test_output_format_default() {
        let fmt = OutputFormat::default();
        assert_eq!(fmt, OutputFormat::Table);
    }

    #[test]
    fn test_portfolio_config_default() {
        let port = PortfolioConfig::default();
        assert!(port.data_dir.is_none());
    }

    #[test]
    fn test_chains_config_default() {
        let chains = ChainsConfig::default();
        assert!(chains.ethereum_rpc.is_none());
        assert!(chains.bsc_rpc.is_none());
        assert!(chains.aegis_rpc.is_none());
        assert!(chains.solana_rpc.is_none());
        assert!(chains.tron_api.is_none());
        assert!(chains.api_keys.is_empty());
    }
}
