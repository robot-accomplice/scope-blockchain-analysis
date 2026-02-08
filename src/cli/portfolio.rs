//! # Portfolio Management Command
//!
//! This module implements the `bca portfolio` command for managing
//! watched addresses and viewing aggregated portfolio data.
//!
//! ## Usage
//!
//! ```bash
//! # Add an address to portfolio
//! bca portfolio add 0x742d... --label "Main Wallet"
//!
//! # List watched addresses
//! bca portfolio list
//!
//! # Remove an address
//! bca portfolio remove 0x742d...
//!
//! # View portfolio summary
//! bca portfolio summary
//! ```

use crate::chains::ChainClientFactory;
use crate::config::{Config, OutputFormat};
use crate::error::{Result, ScopeError};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Arguments for the portfolio management command.
#[derive(Debug, Clone, Args)]
pub struct PortfolioArgs {
    /// Portfolio subcommand to execute.
    #[command(subcommand)]
    pub command: PortfolioCommands,

    /// Override output format.
    #[arg(short, long, global = true, value_name = "FORMAT")]
    pub format: Option<OutputFormat>,
}

/// Portfolio subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum PortfolioCommands {
    /// Add an address to the portfolio.
    Add(AddArgs),

    /// Remove an address from the portfolio.
    Remove(RemoveArgs),

    /// List all watched addresses.
    List,

    /// Show portfolio summary with balances.
    Summary(SummaryArgs),
}

/// Arguments for adding an address.
#[derive(Debug, Clone, Args)]
pub struct AddArgs {
    /// The address to add.
    #[arg(value_name = "ADDRESS")]
    pub address: String,

    /// Human-readable label for the address.
    #[arg(short, long)]
    pub label: Option<String>,

    /// Blockchain network for this address.
    #[arg(short, long, default_value = "ethereum")]
    pub chain: String,

    /// Tags for categorization.
    #[arg(short, long, value_delimiter = ',')]
    pub tags: Vec<String>,
}

/// Arguments for removing an address.
#[derive(Debug, Clone, Args)]
pub struct RemoveArgs {
    /// The address to remove.
    #[arg(value_name = "ADDRESS")]
    pub address: String,
}

/// Arguments for portfolio summary.
#[derive(Debug, Clone, Args)]
pub struct SummaryArgs {
    /// Filter by chain.
    #[arg(short, long)]
    pub chain: Option<String>,

    /// Filter by tag.
    #[arg(short, long)]
    pub tag: Option<String>,

    /// Include token balances.
    #[arg(long)]
    pub include_tokens: bool,
}

/// A watched address in the portfolio.
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

/// Portfolio data storage.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Portfolio {
    /// All watched addresses.
    pub addresses: Vec<WatchedAddress>,
}

/// Portfolio summary report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioSummary {
    /// Total number of addresses.
    pub address_count: usize,

    /// Balances by chain.
    pub balances_by_chain: HashMap<String, ChainBalance>,

    /// Total portfolio value in USD (if available).
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
    /// Token mint/contract address.
    pub mint: String,
    /// Token balance (human-readable).
    pub balance: String,
    /// Token decimals.
    pub decimals: u8,
    /// Token symbol (if known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
}

impl Portfolio {
    /// Loads the portfolio from the data directory.
    pub fn load(data_dir: &std::path::Path) -> Result<Self> {
        let path = data_dir.join("portfolio.yaml");

        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(&path)?;
        let portfolio: Portfolio = serde_yaml::from_str(&contents)
            .map_err(|e| ScopeError::Config(crate::error::ConfigError::Parse { source: e }))?;

        Ok(portfolio)
    }

    /// Saves the portfolio to the data directory.
    pub fn save(&self, data_dir: &PathBuf) -> Result<()> {
        std::fs::create_dir_all(data_dir)?;

        let path = data_dir.join("portfolio.yaml");
        let contents = serde_yaml::to_string(self)
            .map_err(|e| ScopeError::Export(format!("Failed to serialize portfolio: {}", e)))?;

        std::fs::write(&path, contents)?;
        Ok(())
    }

    /// Adds an address to the portfolio.
    pub fn add_address(&mut self, watched: WatchedAddress) -> Result<()> {
        // Check for duplicates
        if self
            .addresses
            .iter()
            .any(|a| a.address.to_lowercase() == watched.address.to_lowercase())
        {
            return Err(ScopeError::Chain(format!(
                "Address already in portfolio: {}",
                watched.address
            )));
        }

        self.addresses.push(watched);
        Ok(())
    }

    /// Removes an address from the portfolio.
    pub fn remove_address(&mut self, address: &str) -> Result<bool> {
        let original_len = self.addresses.len();
        self.addresses
            .retain(|a| a.address.to_lowercase() != address.to_lowercase());

        Ok(self.addresses.len() < original_len)
    }

    /// Finds an address in the portfolio.
    pub fn find_address(&self, address: &str) -> Option<&WatchedAddress> {
        self.addresses
            .iter()
            .find(|a| a.address.to_lowercase() == address.to_lowercase())
    }
}

/// Executes the portfolio command.
pub async fn run(
    args: PortfolioArgs,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    let data_dir = config.data_dir();
    let format = args.format.unwrap_or(config.output.format);

    match args.command {
        PortfolioCommands::Add(add_args) => run_add(add_args, &data_dir).await,
        PortfolioCommands::Remove(remove_args) => run_remove(remove_args, &data_dir).await,
        PortfolioCommands::List => run_list(&data_dir, format).await,
        PortfolioCommands::Summary(summary_args) => {
            run_summary(summary_args, &data_dir, format, clients).await
        }
    }
}

async fn run_add(args: AddArgs, data_dir: &PathBuf) -> Result<()> {
    tracing::info!(address = %args.address, "Adding address to portfolio");

    let mut portfolio = Portfolio::load(data_dir)?;

    let watched = WatchedAddress {
        address: args.address.clone(),
        label: args.label.clone(),
        chain: args.chain.clone(),
        tags: args.tags.clone(),
        added_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    portfolio.add_address(watched)?;
    portfolio.save(data_dir)?;

    println!(
        "Added {} to portfolio{}",
        args.address,
        args.label
            .map(|l| format!(" as '{}'", l))
            .unwrap_or_default()
    );

    Ok(())
}

async fn run_remove(args: RemoveArgs, data_dir: &PathBuf) -> Result<()> {
    tracing::info!(address = %args.address, "Removing address from portfolio");

    let mut portfolio = Portfolio::load(data_dir)?;
    let removed = portfolio.remove_address(&args.address)?;

    if removed {
        portfolio.save(data_dir)?;
        println!("Removed {} from portfolio", args.address);
    } else {
        println!("Address not found in portfolio: {}", args.address);
    }

    Ok(())
}

async fn run_list(data_dir: &std::path::Path, format: OutputFormat) -> Result<()> {
    let portfolio = Portfolio::load(data_dir)?;

    if portfolio.addresses.is_empty() {
        println!("Portfolio is empty. Add addresses with 'bca portfolio add <address>'");
        return Ok(());
    }

    match format {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&portfolio.addresses)?;
            println!("{}", json);
        }
        OutputFormat::Csv => {
            println!("address,label,chain,tags");
            for addr in &portfolio.addresses {
                println!(
                    "{},{},{},{}",
                    addr.address,
                    addr.label.as_deref().unwrap_or(""),
                    addr.chain,
                    addr.tags.join(";")
                );
            }
        }
        OutputFormat::Table => {
            println!("Portfolio Addresses");
            println!("===================");
            for addr in &portfolio.addresses {
                println!(
                    "  {} ({}) - {}{}",
                    addr.address,
                    addr.chain,
                    addr.label.as_deref().unwrap_or("No label"),
                    if addr.tags.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", addr.tags.join(", "))
                    }
                );
            }
            println!("\nTotal: {} addresses", portfolio.addresses.len());
        }
    }

    Ok(())
}

async fn run_summary(
    args: SummaryArgs,
    data_dir: &std::path::Path,
    format: OutputFormat,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    let portfolio = Portfolio::load(data_dir)?;

    if portfolio.addresses.is_empty() {
        println!("Portfolio is empty. Add addresses with 'bca portfolio add <address>'");
        return Ok(());
    }

    // Filter addresses
    let filtered: Vec<_> = portfolio
        .addresses
        .iter()
        .filter(|a| args.chain.as_ref().is_none_or(|c| &a.chain == c))
        .filter(|a| args.tag.as_ref().is_none_or(|t| a.tags.contains(t)))
        .collect();

    // Fetch balances for each address
    let mut address_summaries = Vec::new();
    let mut balances_by_chain: HashMap<String, ChainBalance> = HashMap::new();

    for watched in &filtered {
        let (balance, tokens) = fetch_address_balance(
            &watched.address,
            &watched.chain,
            clients,
            args.include_tokens,
        )
        .await;

        // Aggregate chain balances
        if let Some(chain_bal) = balances_by_chain.get_mut(&watched.chain) {
            // For simplicity, we're showing individual balances, not aggregating
            // A more complete implementation would sum balances
            let _ = chain_bal;
        } else {
            balances_by_chain.insert(
                watched.chain.clone(),
                ChainBalance {
                    native_balance: balance.clone(),
                    symbol: get_native_symbol(&watched.chain),
                    usd: None,
                },
            );
        }

        address_summaries.push(AddressSummary {
            address: watched.address.clone(),
            label: watched.label.clone(),
            chain: watched.chain.clone(),
            balance,
            usd: None,
            tokens,
        });
    }

    let summary = PortfolioSummary {
        address_count: filtered.len(),
        balances_by_chain,
        total_usd: None,
        addresses: address_summaries,
    };

    match format {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&summary)?;
            println!("{}", json);
        }
        OutputFormat::Csv => {
            println!("address,label,chain,balance,usd");
            for addr in &summary.addresses {
                println!(
                    "{},{},{},{},{}",
                    addr.address,
                    addr.label.as_deref().unwrap_or(""),
                    addr.chain,
                    addr.balance,
                    addr.usd.map_or(String::new(), |u| format!("{:.2}", u))
                );
            }
        }
        OutputFormat::Table => {
            println!("Portfolio Summary");
            println!("=================");
            println!("Addresses: {}", summary.address_count);
            println!();

            for addr in &summary.addresses {
                println!(
                    "  {} ({}) - {} {}",
                    addr.label.as_deref().unwrap_or(&addr.address),
                    addr.chain,
                    addr.balance,
                    addr.usd.map_or(String::new(), |u| format!("(${:.2})", u))
                );

                // Show token balances
                for token in &addr.tokens {
                    let mint_short = if token.mint.len() >= 8 {
                        &token.mint[..8]
                    } else {
                        &token.mint
                    };
                    let symbol = token.symbol.as_deref().unwrap_or(mint_short);
                    println!("    └─ {} {}", token.balance, symbol);
                }
            }

            if let Some(total) = summary.total_usd {
                println!();
                println!("Total Value: ${:.2}", total);
            }
        }
    }

    Ok(())
}

/// Fetches the balance for an address on the specified chain using the factory.
async fn fetch_address_balance(
    address: &str,
    chain: &str,
    clients: &dyn ChainClientFactory,
    _include_tokens: bool,
) -> (String, Vec<TokenSummary>) {
    let client = match clients.create_chain_client(chain) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, chain = %chain, "Failed to create chain client");
            return ("Error".to_string(), vec![]);
        }
    };

    // Fetch native balance
    let native_balance = match client.get_balance(address).await {
        Ok(bal) => bal.formatted,
        Err(e) => {
            tracing::error!(error = %e, address = %address, "Failed to fetch balance");
            "Error".to_string()
        }
    };

    // Always fetch token balances for portfolio summary
    let tokens = match client.get_token_balances(address).await {
        Ok(token_bals) => token_bals
            .into_iter()
            .map(|tb| TokenSummary {
                mint: tb.token.contract_address,
                balance: tb.formatted_balance,
                decimals: tb.token.decimals,
                symbol: Some(tb.token.symbol),
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "Could not fetch token balances");
            vec![]
        }
    };

    (native_balance, tokens)
}

/// Returns the native token symbol for a chain.
fn get_native_symbol(chain: &str) -> String {
    match chain.to_lowercase().as_str() {
        "solana" | "sol" => "SOL".to_string(),
        "ethereum" | "eth" => "ETH".to_string(),
        "tron" | "trx" => "TRX".to_string(),
        _ => "???".to_string(),
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_portfolio() -> Portfolio {
        Portfolio {
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
    fn test_portfolio_default() {
        let portfolio = Portfolio::default();
        assert!(portfolio.addresses.is_empty());
    }

    #[test]
    fn test_portfolio_add_address() {
        let mut portfolio = Portfolio::default();

        let watched = WatchedAddress {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            label: Some("Test".to_string()),
            chain: "ethereum".to_string(),
            tags: vec![],
            added_at: 0,
        };

        let result = portfolio.add_address(watched);
        assert!(result.is_ok());
        assert_eq!(portfolio.addresses.len(), 1);
    }

    #[test]
    fn test_portfolio_add_duplicate_fails() {
        let mut portfolio = Portfolio::default();

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

        portfolio.add_address(watched1).unwrap();
        let result = portfolio.add_address(watched2);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("already in portfolio")
        );
    }

    #[test]
    fn test_portfolio_remove_address() {
        let mut portfolio = create_test_portfolio();
        let original_len = portfolio.addresses.len();

        let removed = portfolio
            .remove_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2")
            .unwrap();

        assert!(removed);
        assert_eq!(portfolio.addresses.len(), original_len - 1);
    }

    #[test]
    fn test_portfolio_remove_nonexistent() {
        let mut portfolio = create_test_portfolio();
        let original_len = portfolio.addresses.len();

        let removed = portfolio.remove_address("0xnonexistent").unwrap();

        assert!(!removed);
        assert_eq!(portfolio.addresses.len(), original_len);
    }

    #[test]
    fn test_portfolio_find_address() {
        let portfolio = create_test_portfolio();

        let found = portfolio.find_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2");
        assert!(found.is_some());
        assert_eq!(found.unwrap().label, Some("Main Wallet".to_string()));

        let not_found = portfolio.find_address("0xnonexistent");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_portfolio_find_address_case_insensitive() {
        let portfolio = create_test_portfolio();

        let found = portfolio.find_address("0x742D35CC6634C0532925A3B844BC9E7595F1B3C2");
        assert!(found.is_some());
    }

    #[test]
    fn test_portfolio_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();

        let portfolio = create_test_portfolio();
        portfolio.save(&data_dir).unwrap();

        let loaded = Portfolio::load(&data_dir).unwrap();
        assert_eq!(loaded.addresses.len(), portfolio.addresses.len());
        assert_eq!(loaded.addresses[0].address, portfolio.addresses[0].address);
    }

    #[test]
    fn test_portfolio_load_nonexistent_returns_default() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();

        let portfolio = Portfolio::load(&data_dir).unwrap();
        assert!(portfolio.addresses.is_empty());
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
    fn test_portfolio_summary_serialization() {
        let summary = PortfolioSummary {
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
    fn test_portfolio_args_parsing() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: PortfolioArgs,
        }

        let cli = TestCli::try_parse_from(["test", "list"]).unwrap();
        assert!(matches!(cli.args.command, PortfolioCommands::List));
    }

    #[test]
    fn test_portfolio_add_args_parsing() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: PortfolioArgs,
        }

        let cli = TestCli::try_parse_from([
            "test",
            "add",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "--label",
            "My Wallet",
            "--chain",
            "polygon",
            "--tags",
            "personal,defi",
        ])
        .unwrap();

        if let PortfolioCommands::Add(add_args) = cli.args.command {
            assert_eq!(
                add_args.address,
                "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"
            );
            assert_eq!(add_args.label, Some("My Wallet".to_string()));
            assert_eq!(add_args.chain, "polygon");
            assert_eq!(add_args.tags, vec!["personal", "defi"]);
        } else {
            panic!("Expected Add command");
        }
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

    // ========================================================================
    // Native symbol tests
    // ========================================================================

    #[test]
    fn test_get_native_symbol_solana() {
        assert_eq!(get_native_symbol("solana"), "SOL");
        assert_eq!(get_native_symbol("sol"), "SOL");
    }

    #[test]
    fn test_get_native_symbol_ethereum() {
        assert_eq!(get_native_symbol("ethereum"), "ETH");
        assert_eq!(get_native_symbol("eth"), "ETH");
    }

    #[test]
    fn test_get_native_symbol_tron() {
        assert_eq!(get_native_symbol("tron"), "TRX");
        assert_eq!(get_native_symbol("trx"), "TRX");
    }

    #[test]
    fn test_get_native_symbol_unknown() {
        assert_eq!(get_native_symbol("bitcoin"), "???");
        assert_eq!(get_native_symbol("unknown"), "???");
    }

    // ========================================================================
    // End-to-end tests using MockClientFactory
    // ========================================================================

    use crate::chains::mocks::MockClientFactory;

    fn mock_factory() -> MockClientFactory {
        MockClientFactory::new()
    }

    #[tokio::test]
    async fn test_run_portfolio_list_empty() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();
        let args = PortfolioArgs {
            command: PortfolioCommands::List,
            format: Some(OutputFormat::Table),
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_portfolio_add_and_list() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        // Add address
        let add_args = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
                label: Some("Test Wallet".to_string()),
                chain: "ethereum".to_string(),
                tags: vec!["test".to_string()],
            }),
            format: Some(OutputFormat::Table),
        };
        let result = super::run(add_args, &config, &factory).await;
        assert!(result.is_ok());

        // List
        let list_args = PortfolioArgs {
            command: PortfolioCommands::List,
            format: Some(OutputFormat::Json),
        };
        let result = super::run(list_args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_portfolio_summary_with_mock() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        // Add address first
        let add_args = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
                label: Some("Test".to_string()),
                chain: "ethereum".to_string(),
                tags: vec![],
            }),
            format: None,
        };
        super::run(add_args, &config, &factory).await.unwrap();

        // Summary
        let summary_args = PortfolioArgs {
            command: PortfolioCommands::Summary(SummaryArgs {
                chain: None,
                tag: None,
                include_tokens: false,
            }),
            format: Some(OutputFormat::Json),
        };
        let result = super::run(summary_args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_portfolio_remove() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        // Add then remove
        let add_args = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xtest".to_string(),
                label: None,
                chain: "ethereum".to_string(),
                tags: vec![],
            }),
            format: None,
        };
        super::run(add_args, &config, &factory).await.unwrap();

        let remove_args = PortfolioArgs {
            command: PortfolioCommands::Remove(RemoveArgs {
                address: "0xtest".to_string(),
            }),
            format: None,
        };
        let result = super::run(remove_args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_portfolio_summary_csv() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        // Add address
        let add_args = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xtest".to_string(),
                label: Some("TestAddr".to_string()),
                chain: "ethereum".to_string(),
                tags: vec!["defi".to_string()],
            }),
            format: None,
        };
        super::run(add_args, &config, &factory).await.unwrap();

        // CSV summary
        let summary_args = PortfolioArgs {
            command: PortfolioCommands::Summary(SummaryArgs {
                chain: None,
                tag: None,
                include_tokens: false,
            }),
            format: Some(OutputFormat::Csv),
        };
        let result = super::run(summary_args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_portfolio_summary_table() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        // Add address
        let add_args = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xtest".to_string(),
                label: Some("TestAddr".to_string()),
                chain: "ethereum".to_string(),
                tags: vec![],
            }),
            format: None,
        };
        super::run(add_args, &config, &factory).await.unwrap();

        // Table summary
        let summary_args = PortfolioArgs {
            command: PortfolioCommands::Summary(SummaryArgs {
                chain: None,
                tag: None,
                include_tokens: true,
            }),
            format: Some(OutputFormat::Table),
        };
        let result = super::run(summary_args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_portfolio_summary_with_chain_filter() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        // Add addresses on different chains
        let add_eth = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xeth".to_string(),
                label: None,
                chain: "ethereum".to_string(),
                tags: vec![],
            }),
            format: None,
        };
        super::run(add_eth, &config, &factory).await.unwrap();

        let add_poly = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xpoly".to_string(),
                label: None,
                chain: "polygon".to_string(),
                tags: vec![],
            }),
            format: None,
        };
        super::run(add_poly, &config, &factory).await.unwrap();

        // Filter by chain
        let summary_args = PortfolioArgs {
            command: PortfolioCommands::Summary(SummaryArgs {
                chain: Some("ethereum".to_string()),
                tag: None,
                include_tokens: false,
            }),
            format: Some(OutputFormat::Json),
        };
        let result = super::run(summary_args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_portfolio_summary_with_tag_filter() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        // Add addresses with tags
        let add_args = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xdefi".to_string(),
                label: None,
                chain: "ethereum".to_string(),
                tags: vec!["defi".to_string()],
            }),
            format: None,
        };
        super::run(add_args, &config, &factory).await.unwrap();

        // Filter by tag
        let summary_args = PortfolioArgs {
            command: PortfolioCommands::Summary(SummaryArgs {
                chain: None,
                tag: Some("defi".to_string()),
                include_tokens: false,
            }),
            format: Some(OutputFormat::Json),
        };
        let result = super::run(summary_args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_portfolio_summary_no_format() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        let add_args = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xtest".to_string(),
                label: None,
                chain: "ethereum".to_string(),
                tags: vec![],
            }),
            format: None,
        };
        super::run(add_args, &config, &factory).await.unwrap();

        let summary_args = PortfolioArgs {
            command: PortfolioCommands::Summary(SummaryArgs {
                chain: None,
                tag: None,
                include_tokens: false,
            }),
            format: None, // Default format
        };
        let result = super::run(summary_args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_portfolio_summary_empty() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        // Summary with no addresses added
        let summary_args = PortfolioArgs {
            command: PortfolioCommands::Summary(SummaryArgs {
                chain: None,
                tag: None,
                include_tokens: false,
            }),
            format: Some(OutputFormat::Table),
        };
        let result = super::run(summary_args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_portfolio_add_with_tags() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        let add_args = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xtagged".to_string(),
                label: Some("Tagged".to_string()),
                chain: "ethereum".to_string(),
                tags: vec!["defi".to_string(), "whale".to_string()],
            }),
            format: None,
        };
        let result = super::run(add_args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_native_symbol_polygon() {
        assert_eq!(get_native_symbol("polygon"), "???");
    }

    #[test]
    fn test_get_native_symbol_bsc() {
        assert_eq!(get_native_symbol("bsc"), "???");
    }

    #[tokio::test]
    async fn test_run_portfolio_list_csv_format() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        // Add address
        let add_args = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xCSV_test".to_string(),
                label: Some("CsvAddr".to_string()),
                chain: "ethereum".to_string(),
                tags: vec!["test".to_string()],
            }),
            format: None,
        };
        super::run(add_args, &config, &factory).await.unwrap();

        // List with CSV
        let list_args = PortfolioArgs {
            command: PortfolioCommands::List,
            format: Some(OutputFormat::Csv),
        };
        let result = super::run(list_args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_portfolio_list_table_format() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        // Add addresses with and without labels
        let add_args = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xTable_test1".to_string(),
                label: Some("LabeledAddr".to_string()),
                chain: "ethereum".to_string(),
                tags: vec!["personal".to_string(), "defi".to_string()],
            }),
            format: None,
        };
        super::run(add_args, &config, &factory).await.unwrap();

        let add_args2 = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xTable_test2".to_string(),
                label: None,
                chain: "polygon".to_string(),
                tags: vec![],
            }),
            format: None,
        };
        super::run(add_args2, &config, &factory).await.unwrap();

        // List with Table
        let list_args = PortfolioArgs {
            command: PortfolioCommands::List,
            format: Some(OutputFormat::Table),
        };
        let result = super::run(list_args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_portfolio_summary_table_with_tokens() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        // Add address
        let add_args = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xTokenTest".to_string(),
                label: Some("TokenAddr".to_string()),
                chain: "ethereum".to_string(),
                tags: vec![],
            }),
            format: None,
        };
        super::run(add_args, &config, &factory).await.unwrap();

        // Summary with Table and tokens included
        let summary_args = PortfolioArgs {
            command: PortfolioCommands::Summary(SummaryArgs {
                chain: None,
                tag: None,
                include_tokens: true,
            }),
            format: Some(OutputFormat::Table),
        };
        let result = super::run(summary_args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_portfolio_summary_multiple_chains() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        // Add addresses on the same chain to test chain balance aggregation
        let add1 = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xMulti1".to_string(),
                label: None,
                chain: "ethereum".to_string(),
                tags: vec![],
            }),
            format: None,
        };
        super::run(add1, &config, &factory).await.unwrap();

        let add2 = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xMulti2".to_string(),
                label: None,
                chain: "ethereum".to_string(),
                tags: vec![],
            }),
            format: None,
        };
        super::run(add2, &config, &factory).await.unwrap();

        // Summary - should aggregate chain balances
        let summary_args = PortfolioArgs {
            command: PortfolioCommands::Summary(SummaryArgs {
                chain: None,
                tag: None,
                include_tokens: false,
            }),
            format: Some(OutputFormat::Table),
        };
        let result = super::run(summary_args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_portfolio_list_no_format() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(tmp_dir.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = mock_factory();

        // Add address
        let add_args = PortfolioArgs {
            command: PortfolioCommands::Add(AddArgs {
                address: "0xNoFmt".to_string(),
                label: Some("Test".to_string()),
                chain: "ethereum".to_string(),
                tags: vec![],
            }),
            format: None,
        };
        super::run(add_args, &config, &factory).await.unwrap();

        // List with default format (None -> Table)
        let list_args = PortfolioArgs {
            command: PortfolioCommands::List,
            format: None,
        };
        let result = super::run(list_args, &config, &factory).await;
        assert!(result.is_ok());
    }
}
