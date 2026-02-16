//! # Address Analysis Command
//!
//! This module implements the `scope address` command for analyzing
//! blockchain addresses. It retrieves balance information, transaction
//! history, and token holdings.
//!
//! ## Usage
//!
//! ```bash
//! # Basic address analysis
//! scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2
//!
//! # Specify chain
//! scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --chain ethereum
//!
//! # Output as JSON
//! scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --format json
//! ```

use crate::chains::{
    ChainClient, ChainClientFactory, validate_solana_address, validate_tron_address,
};
use crate::config::{Config, OutputFormat};
use crate::error::Result;
use clap::Args;

/// Arguments for the address analysis command.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "\x1b[1mExamples:\x1b[0m
  scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2
  scope address 0x742d... --include-txs --include-tokens
  scope address 0x742d... --dossier --report dossier.md
  scope address DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy --chain solana",
    after_long_help = "\x1b[1mExamples:\x1b[0m

  \x1b[1m$ scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2\x1b[0m

  Address Analysis Report
  =======================
  Address:      0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2
  Chain:        ethereum
  Balance:      1.234567 ETH
  Value (USD):  $3,456.78
  Transactions: 142

  \x1b[1m$ scope address 0x742d... --dossier --report dossier.md\x1b[0m

  Address Analysis Report
  =======================
  Address:      0x742d35Cc...f1b3c2
  Chain:        ethereum
  Balance:      1.234567 ETH
  ...
  Risk Assessment
  =======================
  Risk Score:   35/100 (Low)
  Factors:      No sanctions matches, moderate tx volume
  Report saved to dossier.md

  \x1b[1m$ scope address DRpbCBMx...TDt1v --chain solana\x1b[0m

  Address Analysis Report
  =======================
  Address:      DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy
  Chain:        solana
  Balance:      42.500000 SOL
  Value (USD):  $5,312.50
  Transactions: 87"
)]
pub struct AddressArgs {
    /// The blockchain address to analyze.
    ///
    /// Must be a valid address format for the target chain
    /// (e.g., 0x-prefixed 40-character hex for Ethereum).
    #[arg(value_name = "ADDRESS")]
    pub address: String,

    /// Target blockchain network.
    ///
    /// EVM chains: ethereum, polygon, arbitrum, optimism, base, bsc
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

    /// Generate and save a markdown report to the specified path.
    #[arg(long, value_name = "PATH")]
    pub report: Option<std::path::PathBuf>,

    /// Produce a combined dossier: address analysis + risk assessment.
    ///
    /// Implies --include-txs and --include-tokens. Uses ETHERSCAN_API_KEY
    /// for enhanced risk analysis on Ethereum.
    #[arg(long, default_value_t = false)]
    pub dossier: bool,
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
    // Resolve address book label → address + chain
    if let Some((address, chain)) =
        crate::cli::address_book::resolve_address_book_input(&args.address, config)?
    {
        args.address = address;
        if args.chain == "ethereum" {
            args.chain = chain;
        }
    }

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

    // Dossier implies full picture: txs + tokens
    let mut analysis_args = args.clone();
    if args.dossier {
        analysis_args.include_txs = true;
        analysis_args.include_tokens = true;
    }

    let sp = crate::cli::progress::Spinner::new(&format!("Analyzing address on {}...", args.chain));

    let client = clients.create_chain_client(&args.chain)?;
    let report = analyze_address(&analysis_args, client.as_ref()).await?;

    // Dossier: fetch risk assessment (uses ETHERSCAN_API_KEY for Ethereum)
    let risk_assessment = if args.dossier {
        sp.set_message("Running risk assessment...");
        let engine = match crate::compliance::datasource::BlockchainDataClient::from_env_opt() {
            Some(client) => crate::compliance::risk::RiskEngine::with_data_client(client),
            None => crate::compliance::risk::RiskEngine::new(),
        };
        engine.assess_address(&args.address, &args.chain).await.ok()
    } else {
        None
    };

    sp.finish("Analysis complete.");

    // Output based on format
    let format = args.format.unwrap_or(config.output.format);
    if format == OutputFormat::Markdown {
        if args.dossier && risk_assessment.as_ref().is_some() {
            let risk = risk_assessment.as_ref().unwrap();
            println!(
                "{}",
                crate::cli::address_report::generate_dossier_report(&report, risk)
            );
        } else {
            println!(
                "{}",
                crate::cli::address_report::generate_address_report(&report)
            );
        }
    } else if args.dossier && risk_assessment.is_some() {
        let risk = risk_assessment.as_ref().unwrap();
        output_report(&report, format)?;
        println!();
        let risk_output =
            crate::display::format_risk_report(risk, crate::display::OutputFormat::Table, true);
        println!("{}", risk_output);
    } else {
        output_report(&report, format)?;
    }

    // Generate report if requested
    if let Some(ref report_path) = args.report {
        let markdown_report = if args.dossier {
            risk_assessment
                .as_ref()
                .map(|r| crate::cli::address_report::generate_dossier_report(&report, r))
                .unwrap_or_else(|| crate::cli::address_report::generate_address_report(&report))
        } else {
            crate::cli::address_report::generate_address_report(&report)
        };
        crate::cli::address_report::save_address_report(&markdown_report, report_path)?;
        println!("\nReport saved to: {}", report_path.display());
    }

    Ok(())
}

/// Analyzes an address using a unified chain client.
/// Exposed for use by batch report and other commands.
pub async fn analyze_address(
    args: &AddressArgs,
    client: &dyn ChainClient,
) -> Result<AddressReport> {
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
                eprintln!("  ⚠ Transaction history unavailable (use -v for details)");
                tracing::debug!("Failed to fetch transactions: {}", e);
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
                eprintln!("  ⚠ Token balances unavailable (use -v for details)");
                tracing::debug!("Failed to fetch token balances: {}", e);
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
                "Unsupported chain: {}. Supported: ethereum, polygon, arbitrum, optimism, base, bsc, solana, tron",
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
        OutputFormat::Markdown => {
            println!(
                "{}",
                crate::cli::address_report::generate_address_report(report)
            );
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
    fn test_validate_address_valid_arbitrum() {
        let result = validate_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "arbitrum");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_address_valid_optimism() {
        let result = validate_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "optimism");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_address_valid_base() {
        let result = validate_address("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", "base");
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
    fn test_transaction_summary_with_none_to_and_false_status() {
        let tx = TransactionSummary {
            hash: "0xcontract".to_string(),
            block_number: 999,
            timestamp: 1700001000,
            from: "0xfrom".to_string(),
            to: None,
            value: "0".to_string(),
            status: false,
        };
        let json = serde_json::to_string(&tx).unwrap();
        assert!(json.contains("0xcontract"));
        assert!(json.contains("\"status\":false"));
        let deserialized: TransactionSummary = serde_json::from_str(&json).unwrap();
        assert!(deserialized.to.is_none());
        assert!(!deserialized.status);
    }

    #[test]
    fn test_address_report_deserialization() {
        let json = r#"{
            "address": "0xabc123",
            "chain": "polygon",
            "balance": {"raw": "1000", "formatted": "0.001 MATIC"},
            "transaction_count": 5
        }"#;
        let report: AddressReport = serde_json::from_str(json).unwrap();
        assert_eq!(report.address, "0xabc123");
        assert_eq!(report.chain, "polygon");
        assert_eq!(report.balance.raw, "1000");
        assert_eq!(report.transaction_count, 5);
        assert!(report.transactions.is_none());
        assert!(report.tokens.is_none());
    }

    #[test]
    fn test_balance_clone() {
        let b = Balance {
            raw: "1000".to_string(),
            formatted: "1.0".to_string(),
            usd: Some(2500.0),
        };
        let c = b.clone();
        assert_eq!(b.raw, c.raw);
        assert_eq!(b.usd, c.usd);
    }

    #[test]
    fn test_address_report_clone() {
        let report = make_test_report();
        let cloned = report.clone();
        assert_eq!(report.address, cloned.address);
        assert_eq!(report.transaction_count, cloned.transaction_count);
    }

    #[test]
    fn test_address_args_clone() {
        let args = AddressArgs {
            address: "0xabc".to_string(),
            chain: "ethereum".to_string(),
            format: Some(OutputFormat::Json),
            include_txs: true,
            include_tokens: true,
            limit: 50,
            report: None,
            dossier: true,
        };
        let cloned = args.clone();
        assert_eq!(args.address, cloned.address);
        assert_eq!(args.dossier, cloned.dossier);
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

    #[test]
    fn test_output_report_markdown() {
        let report = make_test_report();
        let result = output_report(&report, OutputFormat::Markdown);
        assert!(result.is_ok());
    }

    // ========================================================================
    // Mock-based tests for analyze_address
    // ========================================================================

    use crate::chains::{
        Balance as ChainBalance, ChainClient, Token as ChainToken,
        TokenBalance as ChainTokenBalance, Transaction as ChainTransaction,
    };
    use async_trait::async_trait;

    struct MockClient;

    #[async_trait]
    impl ChainClient for MockClient {
        fn chain_name(&self) -> &str {
            "ethereum"
        }
        fn native_token_symbol(&self) -> &str {
            "ETH"
        }
        async fn get_balance(&self, _addr: &str) -> crate::error::Result<ChainBalance> {
            Ok(ChainBalance {
                raw: "1000000000000000000".into(),
                formatted: "1.0 ETH".into(),
                decimals: 18,
                symbol: "ETH".into(),
                usd_value: Some(2500.0),
            })
        }
        async fn enrich_balance_usd(&self, b: &mut ChainBalance) {
            b.usd_value = Some(2500.0);
        }
        async fn get_transaction(&self, _h: &str) -> crate::error::Result<ChainTransaction> {
            Err(crate::error::ScopeError::NotFound("mock".into()))
        }
        async fn get_transactions(
            &self,
            _addr: &str,
            _lim: u32,
        ) -> crate::error::Result<Vec<ChainTransaction>> {
            Ok(vec![ChainTransaction {
                hash: "0x1234".into(),
                block_number: Some(100),
                timestamp: Some(1700000000),
                from: "0xfrom".into(),
                to: Some("0xto".into()),
                value: "1000000000000000000".into(),
                gas_limit: 21000,
                gas_used: Some(21000),
                gas_price: "20000000000".into(),
                nonce: 1,
                input: "0x".into(),
                status: Some(true),
            }])
        }
        async fn get_block_number(&self) -> crate::error::Result<u64> {
            Ok(12345678)
        }
        async fn get_token_balances(
            &self,
            _addr: &str,
        ) -> crate::error::Result<Vec<ChainTokenBalance>> {
            Ok(vec![ChainTokenBalance {
                token: ChainToken {
                    contract_address: "0xtoken".into(),
                    symbol: "USDC".into(),
                    name: "USD Coin".into(),
                    decimals: 6,
                },
                balance: "1000000".into(),
                formatted_balance: "1.0".into(),
                usd_value: Some(1.0),
            }])
        }
        async fn get_code(&self, _addr: &str) -> crate::error::Result<String> {
            Ok("0x".into())
        }
    }

    #[tokio::test]
    async fn test_analyze_address_with_mock() {
        let args = AddressArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            format: None,
            include_txs: true,
            include_tokens: true,
            limit: 10,
            report: None,
            dossier: false,
        };
        let client = MockClient;
        let result = analyze_address(&args, &client).await;
        assert!(result.is_ok());
        let report = result.unwrap();
        assert_eq!(report.address, "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2");
        assert_eq!(report.chain, "ethereum");
        assert!(report.tokens.is_some());
        assert!(report.transactions.is_some());
    }

    #[tokio::test]
    async fn test_analyze_address_no_txs_no_tokens() {
        let args = AddressArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            format: None,
            include_txs: false,
            include_tokens: false,
            limit: 10,
            report: None,
            dossier: false,
        };
        let client = MockClient;
        let result = analyze_address(&args, &client).await;
        assert!(result.is_ok());
    }

    /// Mock client that returns Err for get_transactions; analyze_address should fall back to empty vec.
    struct FailTxMockClient;
    #[async_trait]
    impl ChainClient for FailTxMockClient {
        fn chain_name(&self) -> &str {
            "ethereum"
        }
        fn native_token_symbol(&self) -> &str {
            "ETH"
        }
        async fn get_balance(&self, _addr: &str) -> crate::error::Result<ChainBalance> {
            Ok(ChainBalance {
                raw: "1000000000000000000".into(),
                formatted: "1.0 ETH".into(),
                decimals: 18,
                symbol: "ETH".into(),
                usd_value: Some(2500.0),
            })
        }
        async fn enrich_balance_usd(&self, _b: &mut ChainBalance) {}
        async fn get_transaction(&self, _h: &str) -> crate::error::Result<ChainTransaction> {
            Err(crate::error::ScopeError::NotFound("mock".into()))
        }
        async fn get_transactions(
            &self,
            _addr: &str,
            _lim: u32,
        ) -> crate::error::Result<Vec<ChainTransaction>> {
            Err(crate::error::ScopeError::Chain("tx fetch failed".into()))
        }
        async fn get_block_number(&self) -> crate::error::Result<u64> {
            Ok(12345678)
        }
        async fn get_token_balances(
            &self,
            _addr: &str,
        ) -> crate::error::Result<Vec<ChainTokenBalance>> {
            Ok(vec![])
        }
        async fn get_code(&self, _addr: &str) -> crate::error::Result<String> {
            Ok("0x".into())
        }
    }

    /// Mock client that returns Err for get_token_balances; analyze_address should fall back to empty vec.
    struct FailTokenBalancesMockClient;
    #[async_trait]
    impl ChainClient for FailTokenBalancesMockClient {
        fn chain_name(&self) -> &str {
            "ethereum"
        }
        fn native_token_symbol(&self) -> &str {
            "ETH"
        }
        async fn get_balance(&self, _addr: &str) -> crate::error::Result<ChainBalance> {
            Ok(ChainBalance {
                raw: "1000000000000000000".into(),
                formatted: "1.0 ETH".into(),
                decimals: 18,
                symbol: "ETH".into(),
                usd_value: Some(2500.0),
            })
        }
        async fn enrich_balance_usd(&self, _b: &mut ChainBalance) {}
        async fn get_transaction(&self, _h: &str) -> crate::error::Result<ChainTransaction> {
            Err(crate::error::ScopeError::NotFound("mock".into()))
        }
        async fn get_transactions(
            &self,
            _addr: &str,
            _lim: u32,
        ) -> crate::error::Result<Vec<ChainTransaction>> {
            Ok(vec![])
        }
        async fn get_block_number(&self) -> crate::error::Result<u64> {
            Ok(12345678)
        }
        async fn get_token_balances(
            &self,
            _addr: &str,
        ) -> crate::error::Result<Vec<ChainTokenBalance>> {
            Err(crate::error::ScopeError::Chain("token balances fetch failed".into()))
        }
        async fn get_code(&self, _addr: &str) -> crate::error::Result<String> {
            Ok("0x".into())
        }
    }

    /// Mock client that returns a transaction with None for block_number, timestamp, status.
    struct PartialTxMockClient;
    #[async_trait]
    impl ChainClient for PartialTxMockClient {
        fn chain_name(&self) -> &str {
            "ethereum"
        }
        fn native_token_symbol(&self) -> &str {
            "ETH"
        }
        async fn get_balance(&self, _addr: &str) -> crate::error::Result<ChainBalance> {
            Ok(ChainBalance {
                raw: "0".into(),
                formatted: "0 ETH".into(),
                decimals: 18,
                symbol: "ETH".into(),
                usd_value: None,
            })
        }
        async fn enrich_balance_usd(&self, _b: &mut ChainBalance) {}
        async fn get_transaction(&self, _h: &str) -> crate::error::Result<ChainTransaction> {
            Err(crate::error::ScopeError::NotFound("mock".into()))
        }
        async fn get_transactions(
            &self,
            _addr: &str,
            _lim: u32,
        ) -> crate::error::Result<Vec<ChainTransaction>> {
            Ok(vec![ChainTransaction {
                hash: "0xpartial".into(),
                block_number: None,
                timestamp: None,
                from: "0xfrom".into(),
                to: None,
                value: "0.5".into(),
                gas_limit: 21000,
                gas_used: Some(21000),
                gas_price: "20000000000".into(),
                nonce: 1,
                input: "0x".into(),
                status: None,
            }])
        }
        async fn get_block_number(&self) -> crate::error::Result<u64> {
            Ok(1)
        }
        async fn get_token_balances(
            &self,
            _addr: &str,
        ) -> crate::error::Result<Vec<ChainTokenBalance>> {
            Ok(vec![])
        }
        async fn get_code(&self, _addr: &str) -> crate::error::Result<String> {
            Ok("0x".into())
        }
    }

    #[tokio::test]
    async fn test_analyze_address_tx_fallback_on_error() {
        let args = AddressArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            format: None,
            include_txs: true,
            include_tokens: false,
            limit: 10,
            report: None,
            dossier: false,
        };
        let client = FailTxMockClient;
        let result = analyze_address(&args, &client).await;
        assert!(result.is_ok());
        let report = result.unwrap();
        assert!(report.transactions.as_ref().map(|v| v.is_empty()).unwrap_or(false));
    }

    #[tokio::test]
    async fn test_analyze_address_tokens_fallback_on_error() {
        let args = AddressArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            format: None,
            include_txs: false,
            include_tokens: true,
            limit: 10,
            report: None,
            dossier: false,
        };
        let client = FailTokenBalancesMockClient;
        let result = analyze_address(&args, &client).await;
        assert!(result.is_ok());
        let report = result.unwrap();
        assert!(report.tokens.as_ref().map(|v| v.is_empty()).unwrap_or(false));
    }

    #[tokio::test]
    async fn test_analyze_address_tx_with_none_fields() {
        let args = AddressArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            format: None,
            include_txs: true,
            include_tokens: false,
            limit: 10,
            report: None,
            dossier: false,
        };
        let client = PartialTxMockClient;
        let result = analyze_address(&args, &client).await;
        assert!(result.is_ok());
        let report = result.unwrap();
        let txs = report.transactions.unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].block_number, 0);
        assert_eq!(txs[0].timestamp, 0);
        assert_eq!(txs[0].to, None);
        assert!(txs[0].status); // unwrap_or(true) when None
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
            report: None,
            dossier: false,
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
            report: None,
            dossier: false,
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
            report: None,
            dossier: false,
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
            report: None,
            dossier: false,
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
            report: None,
            dossier: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_address_args_debug() {
        let args = AddressArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            format: None,
            include_txs: false,
            include_tokens: false,
            limit: 100,
            report: None,
            dossier: false,
        };
        let debug = format!("{:?}", args);
        assert!(debug.contains("AddressArgs"));
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
            report: None,
            dossier: false,
        };
        let result = super::run(args, &config, &factory).await;
        assert!(result.is_ok());
    }
}
