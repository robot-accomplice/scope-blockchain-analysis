//! CLI commands for compliance and risk analysis

use clap::{Subcommand, Args};
use crate::compliance::risk::{RiskEngine};
use crate::compliance::datasource::{BlockchainDataClient, DataSources, analyze_patterns};
use crate::display::{format_risk_report, OutputFormat};

#[derive(Debug, Subcommand)]
pub enum ComplianceCommands {
    /// Assess risk for a blockchain address
    #[command(name = "risk")]
    Risk(RiskArgs),

    /// Trace transaction taint through multiple hops
    #[command(name = "trace")]
    Trace(TraceArgs),

    /// Detect suspicious transaction patterns
    #[command(name = "analyze")]
    Analyze(AnalyzeArgs),

    /// Generate compliance report
    #[command(name = "compliance-report")]
    ComplianceReport(ComplianceReportArgs),
}

#[derive(Debug, Args)]
pub struct RiskArgs {
    /// Address to assess
    #[arg(value_name = "ADDRESS")]
    pub address: String,

    /// Blockchain (auto-detected if not specified)
    #[arg(short, long)]
    pub chain: Option<String>,

    /// Output format
    #[arg(short, long, value_enum, default_value = "table")]
    pub format: OutputFormat,

    /// Include detailed factor breakdown
    #[arg(long)]
    pub detailed: bool,

    /// Export to file
    #[arg(short, long)]
    pub output: Option<String>,
}

#[derive(Debug, Args)]
pub struct TraceArgs {
    /// Transaction hash to trace
    #[arg(value_name = "TX_HASH")]
    pub tx_hash: String,

    /// Trace depth (hops to follow)
    #[arg(short, long, default_value = "3")]
    pub depth: u32,

    /// Flag suspicious addresses
    #[arg(long)]
    pub flag_suspicious: bool,

    /// Output format
    #[arg(short, long, value_enum, default_value = "table")]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct AnalyzeArgs {
    /// Address to analyze
    #[arg(value_name = "ADDRESS")]
    pub address: String,

    /// Pattern types to detect
    #[arg(long, value_enum, default_values = &["structuring", "layering", "integration"])]
    pub patterns: Vec<PatternType>,

    /// Time range (e.g., "30d", "6m", "1y")
    #[arg(short, long, default_value = "30d")]
    pub range: String,

    /// Output format
    #[arg(short, long, value_enum, default_value = "table")]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct ComplianceReportArgs {
    /// Address or addresses file
    #[arg(value_name = "TARGET")]
    pub target: String,

    /// Jurisdiction for compliance
    #[arg(short, long, value_enum)]
    pub jurisdiction: Jurisdiction,

    /// Report type
    #[arg(short, long, value_enum, default_value = "summary")]
    pub report_type: ReportType,

    /// Output file
    #[arg(short, long, required = true)]
    pub output: String,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum PatternType {
    Structuring,
    Layering,
    Integration,
    Velocity,
    RoundNumbers,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum Jurisdiction {
    US,
    EU,
    UK,
    Switzerland,
    Singapore,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum ReportType {
    Summary,
    Detailed,
    SAR,  // Suspicious Activity Report
    TravelRule,
}

/// Handle risk assessment command
pub async fn handle_risk(args: RiskArgs) -> anyhow::Result<()> {
    // Auto-detect chain if not specified
    let chain = match args.chain {
        Some(c) => c,
        None => detect_chain(&args.address)?,
    };

    println!("Assessing risk for {} on {}...", args.address, chain);

    // Try to load API key from environment
    let etherscan_key = std::env::var("ETHERSCAN_API_KEY").ok();
    
    let engine = if let Some(key) = etherscan_key {
        let sources = DataSources::new(key);
        let client = BlockchainDataClient::new(sources);
        println!("Using Etherscan API for enhanced analysis");
        RiskEngine::with_data_client(client)
    } else {
        println!("Note: Set ETHERSCAN_API_KEY for enhanced analysis");
        RiskEngine::new()
    };
    
    let assessment = engine.assess_address(&args.address, &chain).await?;
    
    // Format and display output
    let output = format_risk_report(&assessment, args.format, args.detailed);
    println!("{}", output);

    // Export to file if requested
    if let Some(path) = args.output {
        let json = serde_json::to_string_pretty(&assessment)?;
        std::fs::write(&path, json)?;
        println!("\nReport exported to: {}", path);
    }

    Ok(())
}

/// Handle transaction tracing command
pub async fn handle_trace(args: TraceArgs) -> anyhow::Result<()> {
    println!("Tracing transaction {}...", args.tx_hash);
    println!("Depth: {} hops", args.depth);
    
    if args.flag_suspicious {
        println!("Flagging suspicious addresses enabled");
    }

    // Try to load API key from environment
    let etherscan_key = std::env::var("ETHERSCAN_API_KEY").ok();
    
    if let Some(key) = etherscan_key {
        let sources = DataSources::new(key);
        let client = BlockchainDataClient::new(sources);
        
        match client.trace_transaction(&args.tx_hash, args.depth).await {
            Ok(trace) => {
                println!("\nTransaction Trace");
                println!("=================");
                println!("Root: {}", trace.root_hash);
                println!("Hops: {}", trace.hops.len());
                
                for hop in &trace.hops {
                    println!("  Depth {}: {} ({} ETH)", hop.depth, hop.address, hop.amount);
                }
            }
            Err(e) => {
                eprintln!("Error tracing transaction: {}", e);
            }
        }
    } else {
        println!("Set ETHERSCAN_API_KEY to enable transaction tracing");
    }
    
    Ok(())
}

/// Handle pattern analysis command
pub async fn handle_analyze(args: AnalyzeArgs) -> anyhow::Result<()> {
    println!("Analyzing patterns for {}...", args.address);
    println!("Patterns: {:?}", args.patterns);
    println!("Time range: {}", args.range);

    // Try to load API key from environment
    let etherscan_key = std::env::var("ETHERSCAN_API_KEY").ok();
    
    if let Some(key) = etherscan_key {
        let sources = DataSources::new(key);
        let client = BlockchainDataClient::new(sources);
        
        // Auto-detect chain
        let chain = match detect_chain(&args.address) {
            Ok(c) => c,
            Err(_) => "ethereum".to_string(),
        };
        
        match client.get_transactions(&args.address, &chain).await {
            Ok(txs) => {
                let analysis = analyze_patterns(&txs);
                
                println!("\nPattern Analysis Results");
                println!("========================");
                println!("Total transactions: {}", analysis.total_transactions);
                println!("Velocity: {:.2} tx/day", analysis.velocity_score);
                println!("Structuring detected: {}", analysis.structuring_detected);
                println!("Round number pattern: {}", analysis.round_number_pattern);
                println!("Unusual hour transactions: {}", analysis.unusual_hours);
            }
            Err(e) => {
                eprintln!("Error fetching transactions: {}", e);
            }
        }
    } else {
        println!("Set ETHERSCAN_API_KEY to enable pattern analysis");
    }

    Ok(())
}

/// Handle compliance report generation
pub async fn handle_compliance_report(args: ComplianceReportArgs) -> anyhow::Result<()> {
    println!("Compliance report generation is not yet implemented.");
    println!("Planned features: {} report for {:?} jurisdiction",
        format!("{:?}", args.report_type).to_lowercase(),
        args.jurisdiction
    );
    println!("\nFor now, use 'bcc risk' and 'bcc analyze' for compliance checks.");

    Ok(())
}

/// Auto-detect blockchain from address format
fn detect_chain(address: &str) -> anyhow::Result<String> {
    if address.starts_with("0x") && address.len() == 42 {
        // Could be any EVM chain, default to Ethereum
        Ok("ethereum".to_string())
    } else if address.len() == 32 || address.len() == 44 {
        // Solana base58
        Ok("solana".to_string())
    } else if address.starts_with("T") && address.len() == 34 {
        // Tron
        Ok("tron".to_string())
    } else if address.starts_with("bc1") || address.starts_with("1") || address.starts_with("3") {
        // Bitcoin
        Ok("bitcoin".to_string())
    } else {
        anyhow::bail!("Could not auto-detect chain from address: {}", address)
    }
}
