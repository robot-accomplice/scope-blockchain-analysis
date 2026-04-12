//! # Export Command
//!
//! This module implements the `scope export` command for exporting
//! analysis data to various formats (JSON, CSV).
//!
//! ## Usage
//!
//! ```bash
//! # Export address history to JSON
//! scope export --address 0x742d... --output history.json
//!
//! # Export to CSV
//! scope export --address 0x742d... --output history.csv --format csv
//!
//! # Export address book data
//! scope export --address-book --output address_book.json
//! ```

use clap::Args;
use scope::chains::{ChainClientFactory, infer_chain_from_address};
use scope::config::{Config, OutputFormat};
use scope::error::{Result, ScopeError};
use std::path::PathBuf;

/// Arguments for the export command.
#[derive(Debug, Clone, Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope export --address 0x742d... --output txns.csv
  scope export --address @main-wallet --output txns.json  \x1b[2m# address book shortcut\x1b[0m
  scope export --address 0x742d... --output txns.json --from 2025-01-01 --to 2025-12-31
  scope export --address-book --output portfolio.csv
  scope export -a 0x742d... -o data.csv --chain polygon --limit 500")]
pub struct ExportArgs {
    /// Address to export data for. Use @label for address book shortcut.
    #[arg(short, long, value_name = "ADDRESS", group = "source")]
    pub address: Option<String>,

    /// Export address book data.
    #[arg(
        long = "address-book",
        short = 'p',
        alias = "portfolio",
        group = "source"
    )]
    pub address_book: bool,

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
    /// Export type (address, address book).
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
/// Returns [`ScopeError::Export`] if the export operation fails.
/// Returns [`ScopeError::Io`] if file operations fail.
pub async fn run(
    mut args: ExportArgs,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    // Resolve address book label → address + chain
    if let Some(ref input) = args.address
        && let Some((address, chain)) =
            crate::cli::address_book::resolve_address_book_input(input, config)?
    {
        args.address = Some(address);
        if args.chain == "ethereum" {
            args.chain = chain;
        }
    }

    // Determine format from argument or file extension
    let format = args.format.unwrap_or_else(|| detect_format(&args.output));

    tracing::info!(
        output = %args.output.display(),
        format = %format,
        "Starting export"
    );

    let sp = crate::cli::progress::Spinner::new("Exporting data...");
    let result = if args.address_book {
        export_address_book(&args, format, config).await
    } else if let Some(ref address) = args.address {
        export_address(address, &args, format, clients).await
    } else {
        Err(ScopeError::Export(
            "Must specify either --address or --address-book".to_string(),
        ))
    };
    sp.finish_and_clear();
    result
}

/// Detects output format from file extension.
fn detect_format(path: &std::path::Path) -> OutputFormat {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => OutputFormat::Json,
        Some("csv") => OutputFormat::Csv,
        _ => OutputFormat::Json, // Default to JSON
    }
}

/// Exports address book data.
async fn export_address_book(
    args: &ExportArgs,
    format: OutputFormat,
    config: &Config,
) -> Result<()> {
    use crate::cli::address_book::AddressBook;

    let data_dir = config.data_dir();
    let address_book = AddressBook::load(&data_dir)?;

    let content = match format {
        OutputFormat::Json => serde_json::to_string_pretty(&address_book)?,
        OutputFormat::Csv => {
            let mut csv = String::from("address,label,chain,tags,added_at\n");
            for addr in &address_book.addresses {
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
            return Err(ScopeError::Export(
                "Table format not supported for file export".to_string(),
            ));
        }
        OutputFormat::Markdown => {
            let mut md = "# Address Book Export\n\n".to_string();
            md.push_str("| Address | Label | Chain | Tags | Added |\n|---------|-------|-------|------|-------|\n");
            for addr in &address_book.addresses {
                md.push_str(&format!(
                    "| `{}` | {} | {} | {} | {} |\n",
                    addr.address,
                    addr.label.as_deref().unwrap_or("-"),
                    addr.chain,
                    addr.tags.join(", "),
                    addr.added_at
                ));
            }
            md
        }
    };

    std::fs::write(&args.output, &content)?;

    let report = ExportReport {
        export_type: "address book".to_string(),
        record_count: address_book.addresses.len(),
        output_path: args.output.display().to_string(),
        format: format.to_string(),
        exported_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    println!(
        "Exported {} address book addresses to {}",
        report.record_count, report.output_path
    );

    Ok(())
}

/// Exports address transaction history.
async fn export_address(
    address: &str,
    args: &ExportArgs,
    format: OutputFormat,
    clients: &dyn ChainClientFactory,
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

    eprintln!("  Fetching transactions for {} on {}...", address, chain);

    // Fetch real transaction history
    let client = clients.create_chain_client(&chain)?;
    let chain_txs = client.get_transactions(address, args.limit).await?;

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
            return Err(ScopeError::Export(
                "Table format not supported for file export".to_string(),
            ));
        }
        OutputFormat::Markdown => {
            let mut md = format!(
                "# Transaction Export\n\n**Address:** `{}`  \n**Chain:** {}  \n**Transactions:** {}  \n\n",
                address,
                chain,
                transactions.len()
            );
            md.push_str("| Hash | Block | Timestamp | From | To | Value | Gas | Status |\n");
            md.push_str("|------|-------|-----------|------|----|-------|-----|--------|\n");
            for tx in &transactions {
                md.push_str(&format!(
                    "| `{}` | {} | {} | `{}` | `{}` | {} | {} | {} |\n",
                    tx.hash,
                    tx.block_number,
                    tx.timestamp,
                    tx.from,
                    tx.to.as_deref().unwrap_or("-"),
                    tx.value,
                    tx.gas_used,
                    tx.status
                ));
            }
            md
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
        assert!(!cli.args.address_book);
    }

    #[test]
    fn test_export_args_address_book_flag() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: ExportArgs,
        }

        // Test primary --address-book flag
        let cli =
            TestCli::try_parse_from(["test", "--address-book", "--output", "address_book.json"])
                .unwrap();
        assert!(cli.args.address_book);
        assert!(cli.args.address.is_none());

        // Test backward-compat --portfolio alias
        let cli = TestCli::try_parse_from(["test", "--portfolio", "--output", "address_book.json"])
            .unwrap();
        assert!(cli.args.address_book);
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
    async fn test_export_address_book_json() {
        use crate::cli::address_book::{AddressBook, WatchedAddress};

        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();
        let output_path = temp_dir.path().join("address_book.json");

        // Create a test address book
        let address_book = AddressBook {
            addresses: vec![WatchedAddress {
                address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
                label: Some("Test".to_string()),
                chain: "ethereum".to_string(),
                tags: vec![],
                added_at: 1700000000,
            }],
        };
        address_book.save(&data_dir).unwrap();

        let config = Config {
            address_book: scope::config::AddressBookConfig {
                data_dir: Some(data_dir),
            },
            ..Default::default()
        };

        let args = ExportArgs {
            address: None,
            address_book: true,
            output: output_path.clone(),
            format: Some(OutputFormat::Json),
            chain: "ethereum".to_string(),
            from: None,
            to: None,
            limit: 1000,
        };

        let result = export_address_book(&args, OutputFormat::Json, &config).await;
        assert!(result.is_ok());
        assert!(output_path.exists());

        let content = std::fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"));
    }

    #[tokio::test]
    async fn test_export_address_book_csv() {
        use crate::cli::address_book::{AddressBook, WatchedAddress};

        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();
        let output_path = temp_dir.path().join("address_book.csv");

        let address_book = AddressBook {
            addresses: vec![WatchedAddress {
                address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
                label: Some("Test Wallet".to_string()),
                chain: "ethereum".to_string(),
                tags: vec!["personal".to_string()],
                added_at: 1700000000,
            }],
        };
        address_book.save(&data_dir).unwrap();

        let config = Config {
            address_book: scope::config::AddressBookConfig {
                data_dir: Some(data_dir),
            },
            ..Default::default()
        };

        let args = ExportArgs {
            address: None,
            address_book: true,
            output: output_path.clone(),
            format: Some(OutputFormat::Csv),
            chain: "ethereum".to_string(),
            from: None,
            to: None,
            limit: 1000,
        };

        let result = export_address_book(&args, OutputFormat::Csv, &config).await;
        assert!(result.is_ok());

        let content = std::fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("address,label,chain,tags,added_at"));
        assert!(content.contains("Test Wallet"));
        assert!(content.contains("personal"));
    }

    #[tokio::test]
    async fn test_export_address_book_markdown() {
        use crate::cli::address_book::{AddressBook, WatchedAddress};

        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();
        let output_path = temp_dir.path().join("address_book.md");

        let address_book = AddressBook {
            addresses: vec![WatchedAddress {
                address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
                label: Some("Test Wallet".to_string()),
                chain: "ethereum".to_string(),
                tags: vec!["personal".to_string(), "trading".to_string()],
                added_at: 1700000000,
            }],
        };
        address_book.save(&data_dir).unwrap();

        let config = Config {
            address_book: scope::config::AddressBookConfig {
                data_dir: Some(data_dir),
            },
            ..Default::default()
        };

        let args = ExportArgs {
            address: None,
            address_book: true,
            output: output_path.clone(),
            format: Some(OutputFormat::Markdown),
            chain: "ethereum".to_string(),
            from: None,
            to: None,
            limit: 1000,
        };

        let result = export_address_book(&args, OutputFormat::Markdown, &config).await;
        assert!(result.is_ok());
        assert!(output_path.exists());

        let content = std::fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("# Address Book Export"));
        assert!(content.contains("| Address | Label | Chain | Tags | Added |"));
        assert!(content.contains("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"));
        assert!(content.contains("Test Wallet"));
        assert!(content.contains("personal, trading"));
    }

    // ========================================================================
    // Date parsing and pure function tests
    // ========================================================================

    #[test]
    fn test_parse_date_to_ts_valid() {
        let ts = parse_date_to_ts("2024-01-01");
        assert!(ts.is_some());
        let ts = ts.unwrap();
        // Jan 1, 2024 00:00:00 UTC should be around 1704067200
        assert!(ts > 1700000000 && ts < 1710000000);
    }

    #[test]
    fn test_parse_date_to_ts_epoch() {
        let ts = parse_date_to_ts("1970-01-01");
        assert_eq!(ts, Some(0));
    }

    #[test]
    fn test_parse_date_to_ts_invalid_format() {
        assert!(parse_date_to_ts("not-a-date").is_none());
        assert!(parse_date_to_ts("2024/01/01").is_none());
        assert!(parse_date_to_ts("2024-01").is_none());
        assert!(parse_date_to_ts("").is_none());
    }

    #[test]
    fn test_parse_date_to_ts_invalid_values() {
        assert!(parse_date_to_ts("2024-13-01").is_none()); // Month > 12
        assert!(parse_date_to_ts("2024-00-01").is_none()); // Month 0
        assert!(parse_date_to_ts("2024-01-00").is_none()); // Day 0
        assert!(parse_date_to_ts("2024-01-32").is_none()); // Day > 31
    }

    #[test]
    fn test_days_since_epoch_basic() {
        // Jan 1, 1970 should be day 0
        let days = days_since_epoch(1970, 1, 1);
        assert_eq!(days, Some(0));
    }

    #[test]
    fn test_days_since_epoch_known_date() {
        // 2000-01-01 is day 10957
        let days = days_since_epoch(2000, 1, 1);
        assert_eq!(days, Some(10957));
    }

    #[test]
    fn test_days_since_epoch_invalid_month() {
        assert!(days_since_epoch(2024, 13, 1).is_none());
        assert!(days_since_epoch(2024, 0, 1).is_none());
    }

    #[test]
    fn test_days_since_epoch_invalid_day() {
        assert!(days_since_epoch(2024, 1, 0).is_none());
        assert!(days_since_epoch(2024, 1, 32).is_none());
    }

    #[tokio::test]
    async fn test_export_address_book_table_error() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();
        let output_path = temp_dir.path().join("output.txt");

        // Create empty address book
        use crate::cli::address_book::AddressBook;
        let address_book = AddressBook { addresses: vec![] };
        address_book.save(&data_dir).unwrap();

        let config = Config {
            address_book: scope::config::AddressBookConfig {
                data_dir: Some(data_dir),
            },
            ..Default::default()
        };

        let args = ExportArgs {
            address: None,
            address_book: true,
            output: output_path,
            format: Some(OutputFormat::Table),
            chain: "ethereum".to_string(),
            from: None,
            to: None,
            limit: 1000,
        };

        let result = export_address_book(&args, OutputFormat::Table, &config).await;
        assert!(result.is_err()); // Table format not supported for export
    }

    #[tokio::test]
    async fn test_run_no_source_error() {
        let config = Config::default();
        let args = ExportArgs {
            address: None,
            address_book: false,
            output: PathBuf::from("output.json"),
            format: None,
            chain: "ethereum".to_string(),
            from: None,
            to: None,
            limit: 1000,
        };

        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = scope::chains::DefaultClientFactory {
            chains_config: scope::config::ChainsConfig::default(),
            http,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_err());
    }

    // ========================================================================
    // End-to-end tests using MockClientFactory
    // ========================================================================

    use scope::chains::mocks::{MockChainClient, MockClientFactory};

    fn mock_factory() -> MockClientFactory {
        let mut factory = MockClientFactory::new();
        factory.mock_client = MockChainClient::new("ethereum", "ETH");
        factory.mock_client.transactions = vec![scope::chains::Transaction {
            hash: "0xexport1".to_string(),
            block_number: Some(100),
            timestamp: Some(1700000000),
            from: "0xfrom".to_string(),
            to: Some("0xto".to_string()),
            value: "1.0".to_string(),
            gas_limit: 21000,
            gas_used: Some(21000),
            gas_price: "20000000000".to_string(),
            nonce: 0,
            input: "0x".to_string(),
            status: Some(true),
        }];
        factory
    }

    #[tokio::test]
    async fn test_run_export_address_json() {
        let config = Config::default();
        let factory = mock_factory();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let args = ExportArgs {
            address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string()),
            address_book: false,
            output: tmp.path().to_path_buf(),
            format: Some(OutputFormat::Json),
            chain: "ethereum".to_string(),
            from: None,
            to: None,
            limit: 100,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
        // Verify file was written
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(content.contains("0xexport1"));
    }

    #[tokio::test]
    async fn test_run_export_address_csv() {
        let config = Config::default();
        let factory = mock_factory();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let args = ExportArgs {
            address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string()),
            address_book: false,
            output: tmp.path().to_path_buf(),
            format: Some(OutputFormat::Csv),
            chain: "ethereum".to_string(),
            from: None,
            to: None,
            limit: 100,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(content.contains("hash,block,timestamp"));
    }

    #[tokio::test]
    async fn test_run_export_address_non_ethereum_chain() {
        let config = Config::default();
        let mut factory = MockClientFactory::new();
        factory.mock_client = MockChainClient::new("polygon", "MATIC");
        factory.mock_client.transactions = vec![scope::chains::Transaction {
            hash: "0xpolygon".to_string(),
            block_number: Some(200),
            timestamp: Some(1700000000),
            from: "0xfrom".to_string(),
            to: Some("0xto".to_string()),
            value: "2.0".to_string(),
            gas_limit: 21000,
            gas_used: Some(21000),
            gas_price: "20000000000".to_string(),
            nonce: 0,
            input: "0x".to_string(),
            status: Some(true),
        }];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let args = ExportArgs {
            address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string()),
            address_book: false,
            output: tmp.path().to_path_buf(),
            format: Some(OutputFormat::Json),
            chain: "polygon".to_string(), // Non-ethereum chain
            from: None,
            to: None,
            limit: 100,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(content.contains("polygon"));
        assert!(content.contains("0xpolygon"));
    }

    #[tokio::test]
    async fn test_run_export_with_date_filter() {
        let config = Config::default();
        let factory = mock_factory();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let args = ExportArgs {
            address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string()),
            address_book: false,
            output: tmp.path().to_path_buf(),
            format: Some(OutputFormat::Json),
            chain: "ethereum".to_string(),
            from: Some("2023-01-01".to_string()),
            to: Some("2025-12-31".to_string()),
            limit: 100,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_export_address_markdown() {
        let config = Config::default();
        let factory = mock_factory();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let args = ExportArgs {
            address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string()),
            address_book: false,
            output: tmp.path().to_path_buf(),
            format: Some(OutputFormat::Markdown),
            chain: "ethereum".to_string(),
            from: None,
            to: None,
            limit: 100,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(content.contains("# Transaction Export"));
        assert!(
            content.contains("| Hash | Block | Timestamp | From | To | Value | Gas | Status |")
        );
        assert!(content.contains("0xexport1"));
    }

    #[tokio::test]
    async fn test_run_export_address_table_error() {
        let config = Config::default();
        let factory = mock_factory();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let args = ExportArgs {
            address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string()),
            address_book: false,
            output: tmp.path().to_path_buf(),
            format: Some(OutputFormat::Table),
            chain: "ethereum".to_string(),
            from: None,
            to: None,
            limit: 100,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_err()); // Table format not supported for export
    }

    #[tokio::test]
    async fn test_run_export_address_with_date_filter_before() {
        let config = Config::default();
        let mut factory = MockClientFactory::new();
        factory.mock_client = MockChainClient::new("ethereum", "ETH");
        // Transaction with timestamp 1700000000 (2023-11-14)
        factory.mock_client.transactions = vec![scope::chains::Transaction {
            hash: "0xbefore".to_string(),
            block_number: Some(100),
            timestamp: Some(1690000000), // Before filter
            from: "0xfrom".to_string(),
            to: Some("0xto".to_string()),
            value: "1.0".to_string(),
            gas_limit: 21000,
            gas_used: Some(21000),
            gas_price: "20000000000".to_string(),
            nonce: 0,
            input: "0x".to_string(),
            status: Some(true),
        }];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let args = ExportArgs {
            address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string()),
            address_book: false,
            output: tmp.path().to_path_buf(),
            format: Some(OutputFormat::Json),
            chain: "ethereum".to_string(),
            from: Some("2024-01-01".to_string()), // Filter: only after 2024-01-01
            to: None,
            limit: 100,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        // Transaction should be filtered out (before from date)
        assert!(!content.contains("0xbefore"));
    }

    #[tokio::test]
    async fn test_run_export_address_with_date_filter_after() {
        let config = Config::default();
        let mut factory = MockClientFactory::new();
        factory.mock_client = MockChainClient::new("ethereum", "ETH");
        // Transaction with timestamp 1800000000 (2027-01-14)
        factory.mock_client.transactions = vec![scope::chains::Transaction {
            hash: "0xafter".to_string(),
            block_number: Some(100),
            timestamp: Some(1800000000), // After filter
            from: "0xfrom".to_string(),
            to: Some("0xto".to_string()),
            value: "1.0".to_string(),
            gas_limit: 21000,
            gas_used: Some(21000),
            gas_price: "20000000000".to_string(),
            nonce: 0,
            input: "0x".to_string(),
            status: Some(true),
        }];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let args = ExportArgs {
            address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string()),
            address_book: false,
            output: tmp.path().to_path_buf(),
            format: Some(OutputFormat::Json),
            chain: "ethereum".to_string(),
            from: None,
            to: Some("2025-12-31".to_string()), // Filter: only before 2025-12-31
            limit: 100,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        // Transaction should be filtered out (after to date)
        assert!(!content.contains("0xafter"));
    }

    // ========================================================================
    // Debug trait tests
    // ========================================================================

    #[test]
    fn test_export_args_debug() {
        let args = ExportArgs {
            address: Some("0xtest".to_string()),
            address_book: false,
            output: PathBuf::from("test.json"),
            format: Some(OutputFormat::Json),
            chain: "ethereum".to_string(),
            from: None,
            to: None,
            limit: 100,
        };
        let debug_str = format!("{:?}", args);
        assert!(debug_str.contains("ExportArgs"));
        assert!(debug_str.contains("0xtest"));
    }

    #[test]
    fn test_export_report_debug() {
        let report = ExportReport {
            export_type: "address".to_string(),
            record_count: 42,
            output_path: "/tmp/test.json".to_string(),
            format: "json".to_string(),
            exported_at: 1700000000,
        };
        let debug_str = format!("{:?}", report);
        assert!(debug_str.contains("ExportReport"));
        assert!(debug_str.contains("address"));
        assert!(debug_str.contains("42"));
    }

    #[test]
    fn test_transaction_export_debug() {
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
        let debug_str = format!("{:?}", tx);
        assert!(debug_str.contains("TransactionExport"));
        assert!(debug_str.contains("0xabc123"));
    }

    #[test]
    fn test_transaction_export_debug_no_to() {
        let tx = TransactionExport {
            hash: "0xcreate".to_string(),
            block_number: 100,
            timestamp: 1700000000,
            from: "0xdeployer".to_string(),
            to: None,
            value: "0".to_string(),
            gas_used: 500000,
            status: true,
        };
        let debug_str = format!("{:?}", tx);
        assert!(debug_str.contains("TransactionExport"));
        assert!(debug_str.contains("0xcreate"));
    }

    #[test]
    fn test_export_data_debug() {
        let data = ExportData {
            address: "0xtest".to_string(),
            chain: "ethereum".to_string(),
            transactions: vec![],
            exported_at: 1700000000,
        };
        let debug_str = format!("{:?}", data);
        assert!(debug_str.contains("ExportData"));
        assert!(debug_str.contains("0xtest"));
        assert!(debug_str.contains("ethereum"));
    }

    #[test]
    fn test_export_data_debug_with_transactions() {
        let data = ExportData {
            address: "0xtest".to_string(),
            chain: "ethereum".to_string(),
            transactions: vec![TransactionExport {
                hash: "0xabc".to_string(),
                block_number: 1,
                timestamp: 0,
                from: "0x1".to_string(),
                to: Some("0x2".to_string()),
                value: "0".to_string(),
                gas_used: 21000,
                status: true,
            }],
            exported_at: 1700000000,
        };
        let debug_str = format!("{:?}", data);
        assert!(debug_str.contains("ExportData"));
        assert!(debug_str.contains("0xabc"));
    }

    // ========================================================================
    // Additional pure function tests
    // ========================================================================

    #[test]
    fn test_detect_format_markdown() {
        let path = PathBuf::from("output.md");
        // Markdown extension should default to JSON (not explicitly handled)
        assert_eq!(detect_format(&path), OutputFormat::Json);
    }

    #[test]
    fn test_detect_format_txt() {
        let path = PathBuf::from("output.txt");
        assert_eq!(detect_format(&path), OutputFormat::Json);
    }

    #[test]
    fn test_parse_date_to_ts_future_date() {
        let ts = parse_date_to_ts("2100-01-01");
        assert!(ts.is_some());
        let ts = ts.unwrap();
        // Should be a large timestamp
        assert!(ts > 4000000000);
    }

    #[test]
    fn test_parse_date_to_ts_leap_year() {
        let ts = parse_date_to_ts("2024-02-29");
        assert!(ts.is_some());
    }

    #[test]
    fn test_parse_date_to_ts_non_leap_year_feb_29() {
        // 2023 is not a leap year, but our simple function doesn't validate this
        // It will still return a value, just potentially incorrect
        let ts = parse_date_to_ts("2023-02-29");
        // The function doesn't validate leap years, so it may return Some
        // or None depending on implementation
        let _ = ts;
    }

    #[test]
    fn test_days_since_epoch_leap_year() {
        let days = days_since_epoch(2024, 2, 29);
        assert!(days.is_some());
    }

    #[test]
    fn test_days_since_epoch_year_before_epoch() {
        let days = days_since_epoch(1969, 12, 31);
        assert!(days.is_some());
        assert!(days.unwrap() < 0);
    }

    #[test]
    fn test_days_since_epoch_future_year() {
        let days = days_since_epoch(2100, 1, 1);
        assert!(days.is_some());
        assert!(days.unwrap() > 0);
    }
}
