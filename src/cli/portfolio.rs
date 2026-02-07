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

use crate::chains::{ethereum::EthereumClient, solana::SolanaClient, tron::TronClient};
use crate::config::{Config, OutputFormat};
use crate::error::{BccError, Result};
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
            .map_err(|e| BccError::Config(crate::error::ConfigError::Parse { source: e }))?;

        Ok(portfolio)
    }

    /// Saves the portfolio to the data directory.
    pub fn save(&self, data_dir: &PathBuf) -> Result<()> {
        std::fs::create_dir_all(data_dir)?;

        let path = data_dir.join("portfolio.yaml");
        let contents = serde_yaml::to_string(self)
            .map_err(|e| BccError::Export(format!("Failed to serialize portfolio: {}", e)))?;

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
            return Err(BccError::Chain(format!(
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
pub async fn run(args: PortfolioArgs, config: &Config) -> Result<()> {
    let data_dir = config.data_dir();
    let format = args.format.unwrap_or(config.output.format);

    match args.command {
        PortfolioCommands::Add(add_args) => run_add(add_args, &data_dir).await,
        PortfolioCommands::Remove(remove_args) => run_remove(remove_args, &data_dir).await,
        PortfolioCommands::List => run_list(&data_dir, format).await,
        PortfolioCommands::Summary(summary_args) => {
            run_summary(summary_args, &data_dir, format, config).await
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
    config: &Config,
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
            config,
            args.include_tokens,
        ).await;

        // Aggregate chain balances
        if let Some(chain_bal) = balances_by_chain.get_mut(&watched.chain) {
            // For simplicity, we're showing individual balances, not aggregating
            // A more complete implementation would sum balances
            let _ = chain_bal;
        } else {
            balances_by_chain.insert(watched.chain.clone(), ChainBalance {
                native_balance: balance.clone(),
                symbol: get_native_symbol(&watched.chain),
                usd: None,
            });
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
                    let symbol = token.symbol.as_deref().unwrap_or(&token.mint[..8]);
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

/// Fetches the balance for an address on the specified chain.
async fn fetch_address_balance(
    address: &str,
    chain: &str,
    config: &Config,
    include_tokens: bool,
) -> (String, Vec<TokenSummary>) {
    let chain_lower = chain.to_lowercase();
    
    match chain_lower.as_str() {
        "solana" | "sol" => {
            fetch_solana_balance(address, config, include_tokens).await
        }
        "ethereum" | "eth" => {
            fetch_ethereum_balance(address, config).await
        }
        "tron" | "trx" => {
            fetch_tron_balance(address, config).await
        }
        _ => {
            tracing::warn!(chain = %chain, "Unknown chain, cannot fetch balance");
            ("Unknown chain".to_string(), vec![])
        }
    }
}

/// Fetches Solana balance and optionally SPL token balances.
async fn fetch_solana_balance(
    address: &str, 
    config: &Config,
    include_tokens: bool,
) -> (String, Vec<TokenSummary>) {
    let client = match SolanaClient::new(&config.chains) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Failed to create Solana client");
            return ("Error".to_string(), vec![]);
        }
    };

    // Fetch native SOL balance
    let native_balance = match client.get_balance(address).await {
        Ok(bal) => bal.formatted,
        Err(e) => {
            tracing::error!(error = %e, address = %address, "Failed to fetch SOL balance");
            "Error".to_string()
        }
    };

    // Fetch SPL token balances if requested (or always for portfolio summary)
    let tokens = if include_tokens {
        match client.get_token_balances(address).await {
            Ok(token_bals) => {
                token_bals.into_iter().map(|t| TokenSummary {
                    mint: t.mint,
                    balance: format!("{}", t.ui_amount),
                    decimals: t.decimals,
                    symbol: t.symbol,
                }).collect()
            }
            Err(e) => {
                tracing::error!(error = %e, address = %address, "Failed to fetch SPL token balances");
                vec![]
            }
        }
    } else {
        // Always fetch tokens for portfolio - the flag is for verbose output
        match client.get_token_balances(address).await {
            Ok(token_bals) => {
                token_bals.into_iter().map(|t| TokenSummary {
                    mint: t.mint,
                    balance: format!("{}", t.ui_amount),
                    decimals: t.decimals,
                    symbol: t.symbol,
                }).collect()
            }
            Err(e) => {
                tracing::warn!(error = %e, "Could not fetch token balances");
                vec![]
            }
        }
    };

    (native_balance, tokens)
}

/// Fetches Ethereum balance.
async fn fetch_ethereum_balance(address: &str, config: &Config) -> (String, Vec<TokenSummary>) {
    let client = match EthereumClient::new(&config.chains) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Failed to create Ethereum client");
            return ("Error".to_string(), vec![]);
        }
    };

    let balance = match client.get_balance(address).await {
        Ok(bal) => bal.formatted,
        Err(e) => {
            tracing::error!(error = %e, address = %address, "Failed to fetch ETH balance");
            "Error".to_string()
        }
    };

    // TODO: Add ERC20 token balance fetching
    (balance, vec![])
}

/// Fetches Tron balance.
async fn fetch_tron_balance(address: &str, config: &Config) -> (String, Vec<TokenSummary>) {
    let client = match TronClient::new(&config.chains) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Failed to create Tron client");
            return ("Error".to_string(), vec![]);
        }
    };

    let balance = match client.get_balance(address).await {
        Ok(bal) => bal.formatted,
        Err(e) => {
            tracing::error!(error = %e, address = %address, "Failed to fetch TRX balance");
            "Error".to_string()
        }
    };

    // TODO: Add TRC20 token balance fetching
    (balance, vec![])
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
}
