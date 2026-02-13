//! # Report Command
//!
//! Batch and combined report generation for multiple addresses or tokens.

use crate::chains::{ChainClientFactory, infer_chain_from_address};
use crate::cli::address::{self, AddressArgs, AddressReport};
use crate::cli::address_report;
use crate::config::Config;
use crate::error::{Result, ScopeError};
use clap::{Args, Subcommand};

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

    println!(
        "Generating batch report for {} address(es){}...",
        targets.len(),
        if args.with_risk { " (with risk)" } else { "" }
    );
    let mut reports = Vec::new();
    let mut risk_assessments: Vec<Option<crate::compliance::risk::RiskAssessment>> = Vec::new();

    let engine = match crate::compliance::datasource::BlockchainDataClient::from_env_opt() {
        Some(client) => crate::compliance::risk::RiskEngine::with_data_client(client),
        None => crate::compliance::risk::RiskEngine::new(),
    };

    for (address, chain) in &targets {
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

    let md = batch_report_to_markdown(&reports, &risk_assessments, args.with_risk);
    std::fs::write(&args.output, &md)?;
    println!("\nBatch report saved to: {}", args.output.display());
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
    risks: &[Option<crate::compliance::risk::RiskAssessment>],
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
                md.push_str(&crate::display::format_risk_report(
                    risk,
                    crate::display::OutputFormat::Markdown,
                    false,
                ));
            } else {
                md.push_str("\n### Risk Assessment\n\n*Risk assessment unavailable for this address/chain.*\n");
            }
        }
        md.push('\n');
    }

    md.push_str(&crate::display::report::report_footer());
    md
}
