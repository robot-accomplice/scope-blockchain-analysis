//! # Address Analysis Command
//!
//! This module implements the `bca address` command for analyzing
//! blockchain addresses. It retrieves balance information, transaction
//! history, and token holdings.
//!
//! ## Usage
//!
//! ```bash
//! # Basic address analysis
//! bca address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2
//!
//! # Specify chain
//! bca address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --chain ethereum
//!
//! # Output as JSON
//! bca address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --format json
//! ```

use crate::chains::{
    ChainClient, ChainClientFactory, validate_solana_address, validate_tron_address,
};
use crate::config::{Config, OutputFormat};
use crate::error::Result;
use clap::Args;

/// Arguments for the address analysis command.
#[derive(Debug, Clone, Args)]
pub struct AddressArgs {
    /// The blockchain address to analyze.
    ///
    /// Must be a valid address format for the target chain
    /// (e.g., 0x-prefixed 40-character hex for Ethereum).
    #[arg(value_name = "ADDRESS")]
    pub address: String,

    /// Target blockchain network.
    ///
    /// EVM chains: ethereum, polygon, arbitrum, optimism, base, bsc, aegis
    /// Non-EVM chains: solana, tron
    #[arg(short, long, default_value = "ethereum")]
    pub chain: String,

    /// Override output format for this command.
    #[arg(short, long, value_name = "FORMAT")]
    pub format: Option<OutputFormat>,

    /// Include full transaction history.
    #[arg(long)]
    pub include_txs: bool,

    /// Include token balances (ERC-20, ERC-721).
    #[arg(long)]
    pub include_tokens: bool,

    /// Maximum number of transactions to retrieve.
    #[arg(long, default_value = "100")]
    pub limit: u32,
}

/// Result of an address analysis.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AddressReport {
    /// The analyzed address.
    pub address: String,

    /// The blockchain network.
    pub chain: String,

    /// Native token balance.
    pub balance: Balance,

    /// Transaction count (nonce).
    pub transaction_count: u64,

    /// Recent transactions (if requested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transactions: Option<Vec<TransactionSummary>>,

    /// Token balances (if requested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Vec<TokenBalance>>,
}

/// Balance representation with multiple units.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Balance {
    /// Raw balance in smallest unit (e.g., wei for Ethereum).
    pub raw: String,

    /// Human-readable balance in native token (e.g., ETH).
    pub formatted: String,

    /// Balance in USD (if price available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usd: Option<f64>,
}

/// Summary of a transaction.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransactionSummary {
    /// Transaction hash.
    pub hash: String,

    /// Block number.
    pub block_number: u64,

    /// Timestamp (Unix epoch).
    pub timestamp: u64,

    /// Sender address.
    pub from: String,

    /// Recipient address.
    pub to: Option<String>,

    /// Value transferred.
    pub value: String,

    /// Transaction status (success/failure).
    pub status: bool,
}

/// Token balance information.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenBalance {
    /// Token contract address.
    pub contract_address: String,

    /// Token symbol.
    pub symbol: String,

    /// Token name.
    pub name: String,

    /// Token decimals.
    pub decimals: u8,

    /// Raw balance.
    pub balance: String,

    /// Formatted balance.
    pub formatted_balance: String,
}

/// Executes the address analysis command.
///
/// # Arguments
///
/// * `args` - The parsed command arguments
/// * `config` - Application configuration
///
/// # Returns
///
/// Returns `Ok(())` on success, or an error if the analysis fails.
///
/// # Errors
///
/// Returns `ScopeError::InvalidAddress` if the address format is invalid.
/// Returns `ScopeError::Request` if API calls fail.
pub async fn run(
    mut args: AddressArgs,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    // Auto-infer chain if using default and address format is recognizable
    if args.chain == "ethereum"
        && let Some(inferred) = crate::chains::infer_chain_from_address(&args.address)
        && inferred != "ethereum"
    {
        tracing::info!("Auto-detected chain: {}", inferred);
        println!("Auto-detected chain: {}", inferred);
        args.chain = inferred.to_string();
    }

    tracing::info!(
        address = %args.address,
        chain = %args.chain,
        "Starting address analysis"
    );

    // Validate address format
    validate_address(&args.address, &args.chain)?;

    println!("Analyzing address on {}...", args.chain);

    let client = clients.create_chain_client(&args.chain)?;
    let report = analyze_address(&args, client.as_ref()).await?;

    // Output based on format
    let format = args.format.unwrap_or(config.output.format);
    output_report(&report, format)?;

    Ok(())
}

/// Analyzes an address using a unified chain client.
async fn analyze_address(args: &AddressArgs, client: &dyn ChainClient) -> Result<AddressReport> {
    // Fetch balance
    let mut chain_balance = client.get_balance(&args.address).await?;
    client.enrich_balance_usd(&mut chain_balance).await;

    let balance = Balance {
        raw: chain_balance.raw.clone(),
        formatted: chain_balance.formatted.clone(),
        usd: chain_balance.usd_value,
    };

    // Fetch transactions if requested
    let transactions = if args.include_txs {
        match client.get_transactions(&args.address, args.limit).await {
            Ok(txs) => Some(
                txs.into_iter()
                    .map(|tx| TransactionSummary {
                        hash: tx.hash,
                        block_number: tx.block_number.unwrap_or(0),
                        timestamp: tx.timestamp.unwrap_or(0),
                        from: tx.from,
                        to: tx.to,
                        value: tx.value,
                        status: tx.status.unwrap_or(true),
                    })
                    .collect(),
            ),
            Err(e) => {
                tracing::warn!("Failed to fetch transactions: {}", e);
                Some(vec![])
            }
        }
    } else {
        None
    };

    // Transaction count is the number we fetched (or 0)
    let transaction_count = transactions.as_ref().map(|t| t.len() as u64).unwrap_or(0);

    // Fetch token balances if requested
    let tokens = if args.include_tokens {
        match client.get_token_balances(&args.address).await {
            Ok(token_bals) => Some(
                token_bals
                    .into_iter()
                    .map(|tb| TokenBalance {
                        contract_address: tb.token.contract_address,
                        symbol: tb.token.symbol,
                        name: tb.token.name,
                        decimals: tb.token.decimals,
                        balance: tb.balance,
                        formatted_balance: tb.formatted_balance,
                    })
                    .collect(),
            ),
            Err(e) => {
                tracing::warn!("Failed to fetch token balances: {}", e);
                Some(vec![])
            }
        }
    } else {
        None
    };

    Ok(AddressReport {
        address: args.address.clone(),
        chain: args.chain.clone(),
        balance,
        transaction_count,
        transactions,
        tokens,
    })
}

/// Validates an address format for the given chain.
fn validate_address(address: &str, chain: &str) -> Result<()> {
    match chain {
        // EVM-compatible chains use 0x-prefixed 40-char hex addresses
        "ethereum" | "polygon" | "arbitrum" | "optimism" | "base" | "bsc" | "aegis" => {
            if !address.starts_with("0x") {
                return Err(crate::error::ScopeError::InvalidAddress(format!(
                    "Address must start with '0x': {}",
                    address
                )));
            }
            if address.len() != 42 {
                return Err(crate::error::ScopeError::InvalidAddress(format!(
                    "Address must be 42 characters (0x + 40 hex): {}",
                    address
                )));
            }
            // Validate hex characters
            if !address[2..].chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(crate::error::ScopeError::InvalidAddress(format!(
                    "Address contains invalid hex characters: {}",
                    address
                )));
            }
        }
        // Solana uses base58-encoded 32-byte addresses
        "solana" => {
            validate_solana_address(address)?;
        }
        // Tron uses T-prefixed base58check addresses
        "tron" => {
            validate_tron_address(address)?;
        }
        _ => {
            return Err(crate::error::ScopeError::Chain(format!(
                "Unsupported chain: {}. Supported: ethereum, polygon, arbitrum, optimism, base, bsc, aegis, solana, tron",
                chain
            )));
        }
    }
    Ok(())
}

/// Outputs the address report in the specified format.
fn output_report(report: &AddressReport, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(report)?;
            println!("{}", json);
        }
        OutputFormat::Csv => {
            // CSV format for address is a single row
            println!("address,chain,balance,transaction_count");
            println!(
                "{},{},{},{}",
                report.address, report.chain, report.balance.formatted, report.transaction_count
            );
        }
        OutputFormat::Table => {
            println!("Address Analysis Report");
            println!("=======================");
            println!("Address:      {}", report.address);
            println!("Chain:        {}", report.chain);
            println!("Balance:      {}", report.balance.formatted);
            if let Some(usd) = report.balance.usd {
                println!("Value (USD):  ${:.2}", usd);
            }
            println!("Transactions: {}", report.transaction_count);

            if let Some(ref tokens) = report.tokens
                && !tokens.is_empty()
            {
                println!("\nToken Balances:");
                for token in tokens {
                    println!(
                        "  {} ({}): {}",
                        token.name, token.symbol, token.formatted_balance
                    );
                }
            }
        }
    }
    Ok(())
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_address_valid_ethereum() {
        let result = validate_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "ethereum");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_address_valid_lowercase() {
        let result = validate_address("0x742d35cc6634c0532925a3b844bc9e7595f1b3c2", "ethereum");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_address_valid_polygon() {
        let result = validate_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "polygon");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_address_missing_prefix() {
        let result = validate_address("742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "ethereum");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("0x"));
    }

    #[test]
    fn test_validate_address_too_short() {
        let result = validate_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3", "ethereum");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("42 characters"));
    }

    #[test]
    fn test_validate_address_too_long() {
        let result = validate_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2a", "ethereum");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_address_invalid_hex() {
        let result = validate_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1bXYZ", "ethereum");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid hex"));
    }

    #[test]
    fn test_validate_address_unsupported_chain() {
        let result = validate_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "bitcoin");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unsupported chain")
        );
    }

    #[test]
    fn test_validate_address_valid_bsc() {
        let result = validate_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "bsc");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_address_valid_aegis() {
        let result = validate_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "aegis");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_address_valid_solana() {
        let result = validate_address("DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy", "solana");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_address_invalid_solana() {
        // EVM address should fail for Solana
        let result = validate_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "solana");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_address_valid_tron() {
        let result = validate_address("TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf", "tron");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_address_invalid_tron() {
        // EVM address should fail for Tron
        let result = validate_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "tron");
        assert!(result.is_err());
    }

    #[test]
    fn test_address_args_default_values() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: AddressArgs,
        }

        let cli = TestCli::try_parse_from(["test", "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"])
            .unwrap();

        assert_eq!(cli.args.chain, "ethereum");
        assert_eq!(cli.args.limit, 100);
        assert!(!cli.args.include_txs);
        assert!(!cli.args.include_tokens);
        assert!(cli.args.format.is_none());
    }

    #[test]
    fn test_address_args_with_options() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: AddressArgs,
        }

        let cli = TestCli::try_parse_from([
            "test",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "--chain",
            "polygon",
            "--include-txs",
            "--include-tokens",
            "--limit",
            "50",
            "--format",
            "json",
        ])
        .unwrap();

        assert_eq!(cli.args.chain, "polygon");
        assert_eq!(cli.args.limit, 50);
        assert!(cli.args.include_txs);
        assert!(cli.args.include_tokens);
        assert_eq!(cli.args.format, Some(OutputFormat::Json));
    }

    #[test]
    fn test_address_report_serialization() {
        let report = AddressReport {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            balance: Balance {
                raw: "1000000000000000000".to_string(),
                formatted: "1.0".to_string(),
                usd: Some(3500.0),
            },
            transaction_count: 42,
            transactions: None,
            tokens: None,
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"));
        assert!(json.contains("ethereum"));
        assert!(json.contains("3500"));

        // Verify None fields are skipped
        assert!(!json.contains("transactions"));
        assert!(!json.contains("tokens"));
    }

    #[test]
    fn test_balance_serialization() {
        let balance = Balance {
            raw: "1000000000000000000".to_string(),
            formatted: "1.0 ETH".to_string(),
            usd: None,
        };

        let json = serde_json::to_string(&balance).unwrap();
        assert!(json.contains("1000000000000000000"));
        assert!(json.contains("1.0 ETH"));
        assert!(!json.contains("usd")); // None should be skipped
    }

    #[test]
    fn test_transaction_summary_serialization() {
        let tx = TransactionSummary {
            hash: "0xabc123".to_string(),
            block_number: 12345,
            timestamp: 1700000000,
            from: "0xfrom".to_string(),
            to: Some("0xto".to_string()),
            value: "1.0".to_string(),
            status: true,
        };

        let json = serde_json::to_string(&tx).unwrap();
        assert!(json.contains("0xabc123"));
        assert!(json.contains("12345"));
        assert!(json.contains("true"));
    }

    #[test]
    fn test_token_balance_serialization() {
        let token = TokenBalance {
            contract_address: "0xtoken".to_string(),
            symbol: "USDC".to_string(),
            name: "USD Coin".to_string(),
            decimals: 6,
            balance: "1000000".to_string(),
            formatted_balance: "1.0".to_string(),
        };

        let json = serde_json::to_string(&token).unwrap();
        assert!(json.contains("USDC"));
        assert!(json.contains("USD Coin"));
        assert!(json.contains("\"decimals\":6"));
    }

    // ========================================================================
    // Output formatting tests
    // ========================================================================

    fn make_test_report() -> AddressReport {
        AddressReport {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            balance: Balance {
                raw: "1000000000000000000".to_string(),
                formatted: "1.0 ETH".to_string(),
                usd: Some(3500.0),
            },
            transaction_count: 42,
            transactions: Some(vec![TransactionSummary {
                hash: "0xabc".to_string(),
                block_number: 12345,
                timestamp: 1700000000,
                from: "0xfrom".to_string(),
                to: Some("0xto".to_string()),
                value: "1.0".to_string(),
                status: true,
            }]),
            tokens: Some(vec![TokenBalance {
                contract_address: "0xusdc".to_string(),
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                decimals: 6,
                balance: "1000000".to_string(),
                formatted_balance: "1.0".to_string(),
            }]),
        }
    }

    #[test]
    fn test_output_report_json() {
        let report = make_test_report();
        let result = output_report(&report, OutputFormat::Json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_report_csv() {
        let report = make_test_report();
        let result = output_report(&report, OutputFormat::Csv);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_report_table() {
        let report = make_test_report();
        let result = output_report(&report, OutputFormat::Table);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_report_table_no_usd() {
        let mut report = make_test_report();
        report.balance.usd = None;
        let result = output_report(&report, OutputFormat::Table);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_report_table_no_tokens() {
        let mut report = make_test_report();
        report.tokens = None;
        let result = output_report(&report, OutputFormat::Table);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_report_table_empty_tokens() {
        let mut report = make_test_report();
        report.tokens = Some(vec![]);
        let result = output_report(&report, OutputFormat::Table);
        assert!(result.is_ok());
    }

    // ========================================================================
    // End-to-end tests using MockClientFactory
    // ========================================================================

    use crate::chains::mocks::{MockChainClient, MockClientFactory};

    fn mock_factory() -> MockClientFactory {
        MockClientFactory::new()
    }

    #[tokio::test]
    async fn test_run_ethereum_address() {
        let config = Config::default();
        let factory = mock_factory();
        let args = AddressArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            format: Some(OutputFormat::Json),
            include_txs: false,
            include_tokens: false,
            limit: 10,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_with_transactions() {
        let config = Config::default();
        let mut factory = mock_factory();
        factory.mock_client.transactions = vec![factory.mock_client.transaction.clone()];
        let args = AddressArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            format: Some(OutputFormat::Json),
            include_txs: true,
            include_tokens: false,
            limit: 10,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_with_tokens() {
        let config = Config::default();
        let mut factory = mock_factory();
        factory.mock_client.token_balances = vec![crate::chains::TokenBalance {
            token: crate::chains::Token {
                contract_address: "0xusdc".to_string(),
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                decimals: 6,
            },
            balance: "1000000".to_string(),
            formatted_balance: "1.0".to_string(),
            usd_value: Some(1.0),
        }];
        let args = AddressArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            format: Some(OutputFormat::Table),
            include_txs: false,
            include_tokens: true,
            limit: 10,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_auto_detect_solana() {
        let config = Config::default();
        let mut factory = mock_factory();
        factory.mock_client = MockChainClient::new("solana", "SOL");
        let args = AddressArgs {
            // This is a Solana address format
            address: "DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy".to_string(),
            chain: "ethereum".to_string(), // Will be auto-detected
            format: Some(OutputFormat::Json),
            include_txs: false,
            include_tokens: false,
            limit: 10,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_csv_format() {
        let config = Config::default();
        let factory = mock_factory();
        let args = AddressArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            format: Some(OutputFormat::Csv),
            include_txs: false,
            include_tokens: false,
            limit: 10,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_all_features() {
        let config = Config::default();
        let mut factory = mock_factory();
        factory.mock_client.transactions = vec![factory.mock_client.transaction.clone()];
        factory.mock_client.token_balances = vec![crate::chains::TokenBalance {
            token: crate::chains::Token {
                contract_address: "0xtoken".to_string(),
                symbol: "TEST".to_string(),
                name: "Test Token".to_string(),
                decimals: 18,
            },
            balance: "1000000000000000000".to_string(),
            formatted_balance: "1.0".to_string(),
            usd_value: None,
        }];
        let args = AddressArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            format: Some(OutputFormat::Table),
            include_txs: true,
            include_tokens: true,
            limit: 50,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }
}
