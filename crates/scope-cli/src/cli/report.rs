//! # Report Command
//!
//! Batch and combined report generation for multiple addresses or tokens.

use crate::cli::address::{self, AddressArgs, AddressReport};
use crate::cli::address_report;
use clap::{Args, Subcommand};
use scope::chains::{ChainClientFactory, infer_chain_from_address};
use scope::config::Config;
use scope::error::{Result, ScopeError};

/// Report subcommands.
#[derive(Debug, Subcommand)]
pub enum ReportCommands {
    /// Generate a combined report for multiple addresses.
    ///
    /// Runs address analysis for each target and outputs a single
    /// markdown report. Targets can be comma-separated or from a file.
    Batch(BatchArgs),
}

#[derive(Debug, Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope report batch --addresses 0x742d...,0xabc1... --output report.md
  scope report batch --from-file addresses.txt --output report.md --with-risk
  scope report batch --addresses 0x742d... --chain polygon --output report.md")]
pub struct BatchArgs {
    /// Addresses to analyze (comma-separated).
    #[arg(long, value_delimiter = ',', value_name = "ADDRESS")]
    pub addresses: Vec<String>,

    /// File containing addresses (one per line, optionally "address,chain").
    #[arg(long, value_name = "PATH")]
    pub from_file: Option<std::path::PathBuf>,

    /// Output report path.
    #[arg(short, long, required = true, value_name = "PATH")]
    pub output: std::path::PathBuf,

    /// Default chain for addresses (when not specified per-address).
    #[arg(short, long, default_value = "ethereum")]
    pub chain: String,

    /// Include risk assessment per address (uses ETHERSCAN_API_KEY for Ethereum).
    #[arg(long, default_value_t = false)]
    pub with_risk: bool,
}

/// Run the report command.
pub async fn run(
    args: ReportCommands,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    match args {
        ReportCommands::Batch(batch_args) => run_batch(batch_args, config, clients).await,
    }
}

async fn run_batch(
    args: BatchArgs,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    let targets = resolve_targets(&args)?;
    if targets.is_empty() {
        return Err(ScopeError::Export(
            "No addresses to analyze. Use --addresses or --from-file.".to_string(),
        ));
    }

    let prog = crate::cli::progress::StepProgress::new(
        targets.len() as u64,
        &format!(
            "Batch report{}",
            if args.with_risk { " (with risk)" } else { "" }
        ),
    );
    let mut reports = Vec::new();
    let mut risk_assessments: Vec<Option<scope::compliance::risk::RiskAssessment>> = Vec::new();

    let engine = match scope::compliance::datasource::BlockchainDataClient::from_env_opt() {
        Some(client) => scope::compliance::risk::RiskEngine::with_data_client(client),
        None => scope::compliance::risk::RiskEngine::new(),
    };

    for (address, chain) in &targets {
        let short_addr = if address.len() > 12 {
            format!("{}...{}", &address[..6], &address[address.len() - 4..])
        } else {
            address.clone()
        };
        prog.inc(&short_addr);

        let addr_args = AddressArgs {
            address: address.clone(),
            chain: chain.clone(),
            format: Some(config.output.format),
            include_txs: true,
            include_tokens: true,
            limit: 50,
            report: None,
            dossier: false,
        };

        let client = clients.create_chain_client(chain)?;
        match address::analyze_address(&addr_args, client.as_ref()).await {
            Ok(report) => {
                let risk = if args.with_risk {
                    engine.assess_address(address, chain).await.ok()
                } else {
                    None
                };
                reports.push(report);
                risk_assessments.push(risk);
            }
            Err(e) => {
                eprintln!("Warning: Failed to analyze {}: {}", address, e);
            }
        }
    }

    prog.finish("All addresses analyzed.");
    let md = batch_report_to_markdown(&reports, &risk_assessments, args.with_risk);
    std::fs::write(&args.output, &md)?;
    println!("Batch report saved to: {}", args.output.display());
    Ok(())
}

fn resolve_targets(args: &BatchArgs) -> Result<Vec<(String, String)>> {
    let mut targets = Vec::new();

    for addr in &args.addresses {
        let chain = if args.chain == "ethereum" {
            infer_chain_from_address(addr)
                .map(String::from)
                .unwrap_or_else(|| args.chain.clone())
        } else {
            args.chain.clone()
        };
        targets.push((addr.clone(), chain));
    }

    if let Some(ref path) = args.from_file {
        if !path.exists() {
            return Err(ScopeError::Io(format!(
                "File not found: {}",
                path.display()
            )));
        }
        let content = std::fs::read_to_string(path)?;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (addr, chain) = if let Some((a, c)) = line.split_once(',') {
                (a.trim().to_string(), c.trim().to_string())
            } else {
                (
                    line.to_string(),
                    infer_chain_from_address(line)
                        .map(String::from)
                        .unwrap_or_else(|| args.chain.clone()),
                )
            };
            if !addr.is_empty() {
                targets.push((addr, chain));
            }
        }
    }

    Ok(targets)
}

fn batch_report_to_markdown(
    reports: &[AddressReport],
    risks: &[Option<scope::compliance::risk::RiskAssessment>],
    with_risk: bool,
) -> String {
    let mut md = format!(
        "# Batch Address Report{}\n\n\
        **Generated:** {}  \n\
        **Addresses:** {}  \n\n",
        if with_risk {
            " (with Risk Assessment)"
        } else {
            ""
        },
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
        reports.len()
    );

    for (i, report) in reports.iter().enumerate() {
        md.push_str(&format!(
            "---\n\n## Address {}: `{}`\n\n",
            i + 1,
            report.address
        ));
        md.push_str(&address_report::generate_address_report_section(report));

        if with_risk {
            if let Some(risk) = risks.get(i).and_then(|r| r.as_ref()) {
                md.push_str("\n### Risk Assessment\n\n");
                md.push_str(&scope::display::format_risk_report(
                    risk,
                    scope::display::OutputFormat::Markdown,
                    false,
                ));
            } else {
                md.push_str("\n### Risk Assessment\n\n*Risk assessment unavailable for this address/chain.*\n");
            }
        }
        md.push('\n');
    }

    md.push_str(&scope::display::report::report_footer());
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::address::{AddressReport, Balance, TokenBalance, TransactionSummary};
    use tempfile::NamedTempFile;

    #[test]
    fn test_resolve_targets_addresses_only() {
        let args = BatchArgs {
            addresses: vec!["0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string()],
            from_file: None,
            output: std::path::PathBuf::from("/tmp/out.md"),
            chain: "ethereum".to_string(),
            with_risk: false,
        };
        let targets = resolve_targets(&args).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2");
        assert_eq!(targets[0].1, "ethereum");
    }

    #[test]
    fn test_resolve_targets_multiple_addresses() {
        let args = BatchArgs {
            addresses: vec![
                "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
                "0x0000000000000000000000000000000000000001".to_string(),
            ],
            from_file: None,
            output: std::path::PathBuf::from("/tmp/out.md"),
            chain: "ethereum".to_string(),
            with_risk: false,
        };
        let targets = resolve_targets(&args).unwrap();
        assert_eq!(targets.len(), 2);
    }

    #[test]
    fn test_resolve_targets_from_file() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2\n# comment\n\n0x0000000000000000000000000000000000000001",
        )
        .unwrap();

        let args = BatchArgs {
            addresses: vec![],
            from_file: Some(file.path().to_path_buf()),
            output: std::path::PathBuf::from("/tmp/out.md"),
            chain: "ethereum".to_string(),
            with_risk: false,
        };
        let targets = resolve_targets(&args).unwrap();
        assert_eq!(targets.len(), 2);
    }

    #[test]
    fn test_resolve_targets_from_file_with_chain_override() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            "0x1234567890123456789012345678901234567890,polygon\n",
        )
        .unwrap();

        let args = BatchArgs {
            addresses: vec![],
            from_file: Some(file.path().to_path_buf()),
            output: std::path::PathBuf::from("/tmp/out.md"),
            chain: "ethereum".to_string(),
            with_risk: false,
        };
        let targets = resolve_targets(&args).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].1, "polygon");
    }

    #[test]
    fn test_resolve_targets_file_not_found() {
        let args = BatchArgs {
            addresses: vec![],
            from_file: Some(std::path::PathBuf::from("/nonexistent/path/12345")),
            output: std::path::PathBuf::from("/tmp/out.md"),
            chain: "ethereum".to_string(),
            with_risk: false,
        };
        let result = resolve_targets(&args);
        assert!(result.is_err());
    }

    fn minimal_report() -> AddressReport {
        AddressReport {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            balance: Balance {
                raw: "1000000000000000000".to_string(),
                formatted: "1.0 ETH".to_string(),
                usd: Some(3500.0),
            },
            transaction_count: 42,
            transactions: None,
            tokens: None,
        }
    }

    #[test]
    fn test_batch_report_to_markdown_single_report() {
        let reports = vec![minimal_report()];
        let risks = vec![None];
        let md = batch_report_to_markdown(&reports, &risks, false);
        assert!(md.contains("Batch Address Report"));
        assert!(md.contains("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"));
        assert!(md.contains("Balance Summary"));
        assert!(md.contains("1.0 ETH"));
        assert!(!md.contains("Risk Assessment"));
    }

    #[test]
    fn test_batch_report_to_markdown_with_risk_placeholder() {
        let reports = vec![minimal_report()];
        let risks = vec![None];
        let md = batch_report_to_markdown(&reports, &risks, true);
        assert!(md.contains("Risk Assessment"));
        assert!(md.contains("unavailable"));
    }

    #[test]
    fn test_batch_report_to_markdown_with_transactions_and_tokens() {
        let mut report = minimal_report();
        report.transactions = Some(vec![TransactionSummary {
            hash: "0xabc123".to_string(),
            block_number: 12345,
            timestamp: 1700000000,
            from: "0xfrom".to_string(),
            to: Some("0xto".to_string()),
            value: "1 ETH".to_string(),
            status: true,
        }]);
        report.tokens = Some(vec![TokenBalance {
            contract_address: "0xusdc".to_string(),
            symbol: "USDC".to_string(),
            name: "USD Coin".to_string(),
            decimals: 6,
            balance: "1000000".to_string(),
            formatted_balance: "1.0 USDC".to_string(),
        }]);

        let reports = vec![report];
        let risks = vec![None];
        let md = batch_report_to_markdown(&reports, &risks, false);
        assert!(md.contains("Recent Transactions"));
        assert!(md.contains("Token Balances"));
        assert!(md.contains("USDC"));
    }

    #[test]
    fn test_batch_args_debug() {
        let args = BatchArgs {
            addresses: vec!["0x123".to_string(), "0x456".to_string()],
            from_file: None,
            output: std::path::PathBuf::from("/tmp/report.md"),
            chain: "ethereum".to_string(),
            with_risk: false,
        };
        let debug = format!("{:?}", args);
        assert!(debug.contains("BatchArgs"));
        assert!(debug.contains("0x123"));
    }

    #[test]
    fn test_batch_args_with_risk() {
        let args = BatchArgs {
            addresses: vec![],
            from_file: Some(std::path::PathBuf::from("addrs.txt")),
            output: std::path::PathBuf::from("/tmp/report.md"),
            chain: "polygon".to_string(),
            with_risk: true,
        };
        assert!(args.with_risk);
        assert_eq!(args.chain, "polygon");
        assert!(args.from_file.is_some());
    }

    #[test]
    fn test_report_commands_debug() {
        let cmd = ReportCommands::Batch(BatchArgs {
            addresses: vec!["0xabc".to_string()],
            from_file: None,
            output: std::path::PathBuf::from("out.md"),
            chain: "ethereum".to_string(),
            with_risk: false,
        });
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("Batch"));
    }

    #[test]
    fn test_batch_report_to_markdown_with_risk_data() {
        use scope::compliance::risk::{RiskAssessment, RiskFactor, RiskLevel};

        let reports = vec![minimal_report()];
        let risk = RiskAssessment {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            overall_score: 3.5,
            risk_level: RiskLevel::Low,
            factors: vec![RiskFactor {
                name: "Address Age".to_string(),
                category: scope::compliance::risk::RiskCategory::Behavioral,
                score: 2.0,
                weight: 1.0,
                description: "Address is well-established".to_string(),
                evidence: vec!["Known address".to_string()],
            }],
            recommendations: vec!["Continue monitoring".to_string()],
            assessed_at: chrono::Utc::now(),
        };
        let risks = vec![Some(risk)];
        let md = batch_report_to_markdown(&reports, &risks, true);
        assert!(md.contains("Risk Assessment"));
        assert!(md.contains("with Risk Assessment"));
        // Should contain actual risk data, not "unavailable"
        assert!(!md.contains("unavailable"));
    }

    #[test]
    fn test_batch_report_to_markdown_multiple_reports() {
        let mut report1 = minimal_report();
        report1.address = "0xaaa".to_string();
        let mut report2 = minimal_report();
        report2.address = "0xbbb".to_string();
        report2.chain = "polygon".to_string();

        let reports = vec![report1, report2];
        let risks = vec![None, None];
        let md = batch_report_to_markdown(&reports, &risks, false);
        assert!(md.contains("0xaaa"));
        assert!(md.contains("0xbbb"));
        assert!(md.contains("Address 1"));
        assert!(md.contains("Address 2"));
    }

    #[test]
    fn test_resolve_targets_non_default_chain() {
        let args = BatchArgs {
            addresses: vec!["0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string()],
            from_file: None,
            output: std::path::PathBuf::from("/tmp/out.md"),
            chain: "polygon".to_string(),
            with_risk: false,
        };
        let targets = resolve_targets(&args).unwrap();
        assert_eq!(targets.len(), 1);
        // When chain is not "ethereum", it uses the provided chain directly
        assert_eq!(targets[0].1, "polygon");
    }

    #[test]
    fn test_resolve_targets_empty() {
        let args = BatchArgs {
            addresses: vec![],
            from_file: None,
            output: std::path::PathBuf::from("/tmp/out.md"),
            chain: "ethereum".to_string(),
            with_risk: false,
        };
        let targets = resolve_targets(&args).unwrap();
        assert_eq!(targets.len(), 0);
    }

    #[tokio::test]
    async fn test_run_batch_with_mock_factory() {
        use scope::chains::mocks::MockClientFactory;

        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("batch_report.md");

        let args = BatchArgs {
            addresses: vec!["0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string()],
            from_file: None,
            output: output_path.clone(),
            chain: "ethereum".to_string(),
            with_risk: false,
        };

        let config = Config::default();
        let factory = MockClientFactory::new();

        let result = run_batch(args, &config, &factory).await;
        assert!(result.is_ok());

        // Verify the report was written
        assert!(output_path.exists());
        let content = std::fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("Batch Address Report"));
        assert!(content.contains("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"));
    }

    #[tokio::test]
    async fn test_run_batch_with_risk() {
        use scope::chains::mocks::MockClientFactory;

        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("batch_risk_report.md");

        let args = BatchArgs {
            addresses: vec!["0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string()],
            from_file: None,
            output: output_path.clone(),
            chain: "ethereum".to_string(),
            with_risk: true,
        };

        let config = Config::default();
        let factory = MockClientFactory::new();

        let result = run_batch(args, &config, &factory).await;
        assert!(result.is_ok());
        let content = std::fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("Risk Assessment"));
    }

    #[tokio::test]
    async fn test_run_batch_empty_targets() {
        use scope::chains::mocks::MockClientFactory;

        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("empty.md");

        let args = BatchArgs {
            addresses: vec![],
            from_file: None,
            output: output_path,
            chain: "ethereum".to_string(),
            with_risk: false,
        };

        let config = Config::default();
        let factory = MockClientFactory::new();

        let result = run_batch(args, &config, &factory).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("No addresses"));
    }

    #[tokio::test]
    async fn test_run_dispatch() {
        use scope::chains::mocks::MockClientFactory;

        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("dispatch_report.md");

        let args = ReportCommands::Batch(BatchArgs {
            addresses: vec!["0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string()],
            from_file: None,
            output: output_path.clone(),
            chain: "ethereum".to_string(),
            with_risk: false,
        });

        let config = Config::default();
        let factory = MockClientFactory::new();

        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
    }
}
