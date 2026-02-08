//! # Export Command
//!
//! This module implements the `bca export` command for exporting
//! analysis data to various formats (JSON, CSV).
//!
//! ## Usage
//!
//! ```bash
//! # Export address history to JSON
//! bca export --address 0x742d... --output history.json
//!
//! # Export to CSV
//! bca export --address 0x742d... --output history.csv --format csv
//!
//! # Export portfolio data
//! bca export --portfolio --output portfolio.json
//! ```

use crate::chains::{EthereumClient, SolanaClient, TronClient, infer_chain_from_address};
use crate::config::{Config, OutputFormat};
use crate::error::{BccError, Result};
use clap::Args;
use std::path::PathBuf;

/// Arguments for the export command.
#[derive(Debug, Clone, Args)]
pub struct ExportArgs {
    /// Address to export data for.
    #[arg(short, long, value_name = "ADDRESS", group = "source")]
    pub address: Option<String>,

    /// Export portfolio data.
    #[arg(short, long, group = "source")]
    pub portfolio: bool,

    /// Output file path.
    #[arg(short, long, value_name = "PATH")]
    pub output: PathBuf,

    /// Output format (auto-detected from extension if not specified).
    #[arg(short, long, value_name = "FORMAT")]
    pub format: Option<OutputFormat>,

    /// Target blockchain network (for address export).
    #[arg(short, long, default_value = "ethereum")]
    pub chain: String,

    /// Start date for transaction history (YYYY-MM-DD).
    #[arg(long, value_name = "DATE")]
    pub from: Option<String>,

    /// End date for transaction history (YYYY-MM-DD).
    #[arg(long, value_name = "DATE")]
    pub to: Option<String>,

    /// Maximum number of transactions to export.
    #[arg(long, default_value = "1000")]
    pub limit: u32,
}

/// Data export report.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExportReport {
    /// Export type (address, portfolio).
    pub export_type: String,

    /// Number of records exported.
    pub record_count: usize,

    /// Output file path.
    pub output_path: String,

    /// Export format.
    pub format: String,

    /// Export timestamp.
    pub exported_at: u64,
}

/// Executes the export command.
///
/// # Arguments
///
/// * `args` - The parsed command arguments
/// * `config` - Application configuration
///
/// # Returns
///
/// Returns `Ok(())` on success, or an error if the export fails.
///
/// # Errors
///
/// Returns [`BccError::Export`] if the export operation fails.
/// Returns [`BccError::Io`] if file operations fail.
pub async fn run(args: ExportArgs, config: &Config) -> Result<()> {
    // Determine format from argument or file extension
    let format = args.format.unwrap_or_else(|| detect_format(&args.output));

    tracing::info!(
        output = %args.output.display(),
        format = %format,
        "Starting export"
    );

    if args.portfolio {
        export_portfolio(&args, format, config).await
    } else if let Some(ref address) = args.address {
        export_address(address, &args, format, config).await
    } else {
        Err(BccError::Export(
            "Must specify either --address or --portfolio".to_string(),
        ))
    }
}

/// Detects output format from file extension.
fn detect_format(path: &std::path::Path) -> OutputFormat {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => OutputFormat::Json,
        Some("csv") => OutputFormat::Csv,
        _ => OutputFormat::Json, // Default to JSON
    }
}

/// Exports portfolio data.
async fn export_portfolio(args: &ExportArgs, format: OutputFormat, config: &Config) -> Result<()> {
    use crate::cli::portfolio::Portfolio;

    let data_dir = config.data_dir();
    let portfolio = Portfolio::load(&data_dir)?;

    let content = match format {
        OutputFormat::Json => serde_json::to_string_pretty(&portfolio)?,
        OutputFormat::Csv => {
            let mut csv = String::from("address,label,chain,tags,added_at\n");
            for addr in &portfolio.addresses {
                csv.push_str(&format!(
                    "{},{},{},{},{}\n",
                    addr.address,
                    addr.label.as_deref().unwrap_or(""),
                    addr.chain,
                    addr.tags.join(";"),
                    addr.added_at
                ));
            }
            csv
        }
        OutputFormat::Table => {
            return Err(BccError::Export(
                "Table format not supported for file export".to_string(),
            ));
        }
    };

    std::fs::write(&args.output, &content)?;

    let report = ExportReport {
        export_type: "portfolio".to_string(),
        record_count: portfolio.addresses.len(),
        output_path: args.output.display().to_string(),
        format: format.to_string(),
        exported_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    println!(
        "Exported {} portfolio addresses to {}",
        report.record_count, report.output_path
    );

    Ok(())
}

/// Exports address transaction history.
async fn export_address(
    address: &str,
    args: &ExportArgs,
    format: OutputFormat,
    config: &Config,
) -> Result<()> {
    // Auto-detect chain if default
    let chain = if args.chain == "ethereum" {
        infer_chain_from_address(address)
            .unwrap_or("ethereum")
            .to_string()
    } else {
        args.chain.clone()
    };

    tracing::info!(
        address = %address,
        chain = %chain,
        "Exporting address data"
    );

    println!("Fetching transactions for {} on {}...", address, chain);

    // Fetch real transaction history
    let chain_txs = match chain.as_str() {
        "solana" | "sol" => {
            let client = SolanaClient::new(&config.chains)?;
            client.get_transactions(address, args.limit).await?
        }
        "tron" | "trx" => {
            let client = TronClient::new(&config.chains)?;
            client.get_transactions(address, args.limit).await?
        }
        _ => {
            let client = EthereumClient::for_chain(&chain, &config.chains)?;
            client.get_transactions(address, args.limit).await?
        }
    };

    // Apply date filtering if --from / --to are provided
    let from_ts = args.from.as_deref().and_then(parse_date_to_ts);
    let to_ts = args.to.as_deref().and_then(parse_date_to_ts);

    let transactions: Vec<TransactionExport> = chain_txs
        .into_iter()
        .filter(|tx| {
            let ts = tx.timestamp.unwrap_or(0);
            if let Some(from) = from_ts
                && ts < from
            {
                return false;
            }
            if let Some(to) = to_ts
                && ts > to
            {
                return false;
            }
            true
        })
        .map(|tx| TransactionExport {
            hash: tx.hash,
            block_number: tx.block_number.unwrap_or(0),
            timestamp: tx.timestamp.unwrap_or(0),
            from: tx.from,
            to: tx.to,
            value: tx.value,
            gas_used: tx.gas_used.unwrap_or(0),
            status: tx.status.unwrap_or(true),
        })
        .collect();

    let content = match format {
        OutputFormat::Json => serde_json::to_string_pretty(&ExportData {
            address: address.to_string(),
            chain: chain.clone(),
            transactions: transactions.clone(),
            exported_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        })?,
        OutputFormat::Csv => {
            let mut csv = String::from("hash,block,timestamp,from,to,value,gas_used,status\n");
            for tx in &transactions {
                csv.push_str(&format!(
                    "{},{},{},{},{},{},{},{}\n",
                    tx.hash,
                    tx.block_number,
                    tx.timestamp,
                    tx.from,
                    tx.to.as_deref().unwrap_or(""),
                    tx.value,
                    tx.gas_used,
                    tx.status
                ));
            }
            csv
        }
        OutputFormat::Table => {
            return Err(BccError::Export(
                "Table format not supported for file export".to_string(),
            ));
        }
    };

    std::fs::write(&args.output, &content)?;

    let report = ExportReport {
        export_type: "address".to_string(),
        record_count: transactions.len(),
        output_path: args.output.display().to_string(),
        format: format.to_string(),
        exported_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    println!(
        "Exported {} transactions to {}",
        report.record_count, report.output_path
    );

    Ok(())
}

/// Parses a YYYY-MM-DD date string to a Unix timestamp.
fn parse_date_to_ts(date: &str) -> Option<u64> {
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;

    // Simple calculation: days since epoch * 86400
    // Use chrono-like calculation without the crate
    // For simplicity, use a basic approach
    let days_from_epoch = days_since_epoch(year, month, day)?;
    Some((days_from_epoch as u64) * 86400)
}

/// Calculates days since Unix epoch (1970-01-01) for a given date.
fn days_since_epoch(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let m = if month <= 2 { month + 9 } else { month - 3 } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let doy = (153 * m as u64 + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe as i64 - 719468;
    Some(days)
}

/// Exported transaction data.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransactionExport {
    /// Transaction hash.
    pub hash: String,

    /// Block number.
    pub block_number: u64,

    /// Timestamp.
    pub timestamp: u64,

    /// Sender address.
    pub from: String,

    /// Recipient address.
    pub to: Option<String>,

    /// Value transferred.
    pub value: String,

    /// Gas used.
    pub gas_used: u64,

    /// Transaction status.
    pub status: bool,
}

/// Export data container.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExportData {
    /// The exported address.
    pub address: String,

    /// The chain.
    pub chain: String,

    /// Transactions.
    pub transactions: Vec<TransactionExport>,

    /// Export timestamp.
    pub exported_at: u64,
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_format_json() {
        let path = PathBuf::from("output.json");
        assert_eq!(detect_format(&path), OutputFormat::Json);
    }

    #[test]
    fn test_detect_format_csv() {
        let path = PathBuf::from("output.csv");
        assert_eq!(detect_format(&path), OutputFormat::Csv);
    }

    #[test]
    fn test_detect_format_unknown_defaults_to_json() {
        let path = PathBuf::from("output.txt");
        assert_eq!(detect_format(&path), OutputFormat::Json);
    }

    #[test]
    fn test_detect_format_no_extension() {
        let path = PathBuf::from("output");
        assert_eq!(detect_format(&path), OutputFormat::Json);
    }

    #[test]
    fn test_export_args_parsing() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: ExportArgs,
        }

        let cli = TestCli::try_parse_from([
            "test",
            "--address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "--output",
            "output.json",
        ])
        .unwrap();

        assert_eq!(
            cli.args.address,
            Some("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string())
        );
        assert_eq!(cli.args.output, PathBuf::from("output.json"));
        assert!(!cli.args.portfolio);
    }

    #[test]
    fn test_export_args_portfolio_flag() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: ExportArgs,
        }

        let cli =
            TestCli::try_parse_from(["test", "--portfolio", "--output", "portfolio.json"]).unwrap();

        assert!(cli.args.portfolio);
        assert!(cli.args.address.is_none());
    }

    #[test]
    fn test_export_args_with_all_options() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: ExportArgs,
        }

        let cli = TestCli::try_parse_from([
            "test",
            "--address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "--output",
            "output.csv",
            "--format",
            "csv",
            "--chain",
            "polygon",
            "--from",
            "2024-01-01",
            "--to",
            "2024-12-31",
            "--limit",
            "500",
        ])
        .unwrap();

        assert_eq!(cli.args.chain, "polygon");
        assert_eq!(cli.args.from, Some("2024-01-01".to_string()));
        assert_eq!(cli.args.to, Some("2024-12-31".to_string()));
        assert_eq!(cli.args.limit, 500);
        assert_eq!(cli.args.format, Some(OutputFormat::Csv));
    }

    #[test]
    fn test_export_report_serialization() {
        let report = ExportReport {
            export_type: "address".to_string(),
            record_count: 100,
            output_path: "/tmp/output.json".to_string(),
            format: "json".to_string(),
            exported_at: 1700000000,
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("address"));
        assert!(json.contains("100"));
        assert!(json.contains("/tmp/output.json"));
    }

    #[test]
    fn test_transaction_export_serialization() {
        let tx = TransactionExport {
            hash: "0xabc123".to_string(),
            block_number: 12345,
            timestamp: 1700000000,
            from: "0xfrom".to_string(),
            to: Some("0xto".to_string()),
            value: "1.5".to_string(),
            gas_used: 21000,
            status: true,
        };

        let json = serde_json::to_string(&tx).unwrap();
        assert!(json.contains("0xabc123"));
        assert!(json.contains("12345"));
        assert!(json.contains("21000"));
    }

    #[test]
    fn test_export_data_serialization() {
        let data = ExportData {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            transactions: vec![],
            exported_at: 1700000000,
        };

        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"));
        assert!(json.contains("ethereum"));
    }

    #[tokio::test]
    async fn test_export_portfolio_json() {
        use crate::cli::portfolio::{Portfolio, WatchedAddress};

        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();
        let output_path = temp_dir.path().join("portfolio.json");

        // Create a test portfolio
        let portfolio = Portfolio {
            addresses: vec![WatchedAddress {
                address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
                label: Some("Test".to_string()),
                chain: "ethereum".to_string(),
                tags: vec![],
                added_at: 1700000000,
            }],
        };
        portfolio.save(&data_dir).unwrap();

        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(data_dir),
            },
            ..Default::default()
        };

        let args = ExportArgs {
            address: None,
            portfolio: true,
            output: output_path.clone(),
            format: Some(OutputFormat::Json),
            chain: "ethereum".to_string(),
            from: None,
            to: None,
            limit: 1000,
        };

        let result = export_portfolio(&args, OutputFormat::Json, &config).await;
        assert!(result.is_ok());
        assert!(output_path.exists());

        let content = std::fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"));
    }

    #[tokio::test]
    async fn test_export_portfolio_csv() {
        use crate::cli::portfolio::{Portfolio, WatchedAddress};

        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();
        let output_path = temp_dir.path().join("portfolio.csv");

        let portfolio = Portfolio {
            addresses: vec![WatchedAddress {
                address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
                label: Some("Test Wallet".to_string()),
                chain: "ethereum".to_string(),
                tags: vec!["personal".to_string()],
                added_at: 1700000000,
            }],
        };
        portfolio.save(&data_dir).unwrap();

        let config = Config {
            portfolio: crate::config::PortfolioConfig {
                data_dir: Some(data_dir),
            },
            ..Default::default()
        };

        let args = ExportArgs {
            address: None,
            portfolio: true,
            output: output_path.clone(),
            format: Some(OutputFormat::Csv),
            chain: "ethereum".to_string(),
            from: None,
            to: None,
            limit: 1000,
        };

        let result = export_portfolio(&args, OutputFormat::Csv, &config).await;
        assert!(result.is_ok());

        let content = std::fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("address,label,chain,tags,added_at"));
        assert!(content.contains("Test Wallet"));
        assert!(content.contains("personal"));
    }
}
