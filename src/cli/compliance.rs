//! CLI commands for compliance and risk analysis

use crate::compliance::datasource::{BlockchainDataClient, DataSources, analyze_patterns};
use crate::compliance::risk::RiskEngine;
use crate::display::{OutputFormat, format_risk_report};
use clap::{Args, Subcommand};

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
    SAR, // Suspicious Activity Report
    TravelRule,
}

/// Handle risk assessment command
pub async fn handle_risk(args: RiskArgs) -> anyhow::Result<()> {
    handle_risk_with_client(args, None).await
}

/// Handle risk assessment with an optional pre-built client (for testability).
pub async fn handle_risk_with_client(
    args: RiskArgs,
    client: Option<BlockchainDataClient>,
) -> anyhow::Result<()> {
    // Auto-detect chain if not specified
    let chain = match args.chain {
        Some(c) => c,
        None => detect_chain(&args.address)?,
    };

    println!("Assessing risk for {} on {}...", args.address, chain);

    let engine = if let Some(c) = client {
        println!("Using Etherscan API for enhanced analysis");
        RiskEngine::with_data_client(c)
    } else {
        // Try to load API key from environment
        let etherscan_key = std::env::var("ETHERSCAN_API_KEY").ok();

        if let Some(key) = etherscan_key {
            let sources = DataSources::new(key);
            let client = BlockchainDataClient::new(sources);
            println!("Using Etherscan API for enhanced analysis");
            RiskEngine::with_data_client(client)
        } else {
            println!("Note: Set ETHERSCAN_API_KEY for enhanced analysis");
            RiskEngine::new()
        }
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
    handle_trace_with_client(args, None).await
}

/// Handle transaction tracing with an optional pre-built client (for testability).
pub async fn handle_trace_with_client(
    args: TraceArgs,
    client: Option<BlockchainDataClient>,
) -> anyhow::Result<()> {
    println!("Tracing transaction {}...", args.tx_hash);
    println!("Depth: {} hops", args.depth);

    if args.flag_suspicious {
        println!("Flagging suspicious addresses enabled");
    }

    let resolved_client = if let Some(c) = client {
        Some(c)
    } else {
        std::env::var("ETHERSCAN_API_KEY").ok().map(|key| {
            let sources = DataSources::new(key);
            BlockchainDataClient::new(sources)
        })
    };

    if let Some(client) = resolved_client {
        match client.trace_transaction(&args.tx_hash, args.depth).await {
            Ok(trace) => {
                println!("\nTransaction Trace");
                println!("=================");
                println!("Root: {}", trace.root_hash);
                println!("Hops: {}", trace.hops.len());

                for hop in &trace.hops {
                    println!(
                        "  Depth {}: {} ({} ETH)",
                        hop.depth, hop.address, hop.amount
                    );
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
    handle_analyze_with_client(args, None).await
}

/// Handle pattern analysis with an optional pre-built client (for testability).
pub async fn handle_analyze_with_client(
    args: AnalyzeArgs,
    client: Option<BlockchainDataClient>,
) -> anyhow::Result<()> {
    println!("Analyzing patterns for {}...", args.address);
    println!("Patterns: {:?}", args.patterns);
    println!("Time range: {}", args.range);

    let resolved_client = if let Some(c) = client {
        Some(c)
    } else {
        std::env::var("ETHERSCAN_API_KEY").ok().map(|key| {
            let sources = DataSources::new(key);
            BlockchainDataClient::new(sources)
        })
    };

    if let Some(client) = resolved_client {
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
    println!(
        "Planned features: {} report for {:?} jurisdiction",
        format!("{:?}", args.report_type).to_lowercase(),
        args.jurisdiction
    );
    println!("\nFor now, use 'scope risk' and 'scope analyze' for compliance checks.");

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

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_chain_ethereum() {
        let result = detect_chain("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "ethereum");
    }

    #[test]
    fn test_detect_chain_solana_short() {
        // Solana 32-char address
        let result = detect_chain("DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "solana");
    }

    #[test]
    fn test_detect_chain_solana_long() {
        // Solana 44-char address
        let result = detect_chain("DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "solana");
    }

    #[test]
    fn test_detect_chain_tron() {
        let result = detect_chain("TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "tron");
    }

    #[test]
    fn test_detect_chain_bitcoin_bech32() {
        let result = detect_chain("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "bitcoin");
    }

    #[test]
    fn test_detect_chain_bitcoin_p2pkh() {
        let result = detect_chain("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "bitcoin");
    }

    #[test]
    fn test_detect_chain_bitcoin_p2sh() {
        let result = detect_chain("3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "bitcoin");
    }

    #[test]
    fn test_detect_chain_unknown() {
        let result = detect_chain("unknown_address_format_xyz");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_handle_risk_no_api_key() {
        // Should work without API key (basic scoring)
        unsafe { std::env::remove_var("ETHERSCAN_API_KEY") };
        let args = RiskArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: Some("ethereum".to_string()),
            format: OutputFormat::Table,
            detailed: false,
            output: None,
        };
        let result = handle_risk(args).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_risk_json_format() {
        unsafe { std::env::remove_var("ETHERSCAN_API_KEY") };
        let args = RiskArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: Some("ethereum".to_string()),
            format: OutputFormat::Json,
            detailed: true,
            output: None,
        };
        let result = handle_risk(args).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_risk_with_export() {
        unsafe { std::env::remove_var("ETHERSCAN_API_KEY") };
        let temp = tempfile::NamedTempFile::new().unwrap();
        let path = temp.path().to_string_lossy().to_string();
        let args = RiskArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: Some("ethereum".to_string()),
            format: OutputFormat::Table,
            detailed: false,
            output: Some(path.clone()),
        };
        let result = handle_risk(args).await;
        assert!(result.is_ok());
        assert!(std::path::Path::new(&path).exists());
    }

    #[tokio::test]
    async fn test_handle_risk_auto_detect_chain() {
        unsafe { std::env::remove_var("ETHERSCAN_API_KEY") };
        let args = RiskArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: None, // auto-detect
            format: OutputFormat::Table,
            detailed: false,
            output: None,
        };
        let result = handle_risk(args).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_trace_no_api_key() {
        unsafe { std::env::remove_var("ETHERSCAN_API_KEY") };
        let args = TraceArgs {
            tx_hash: "0xabc123".to_string(),
            depth: 3,
            flag_suspicious: true,
            format: OutputFormat::Table,
        };
        let result = handle_trace(args).await;
        assert!(result.is_ok()); // No API key → prints message, doesn't error
    }

    #[tokio::test]
    async fn test_handle_analyze_no_api_key() {
        unsafe { std::env::remove_var("ETHERSCAN_API_KEY") };
        let args = AnalyzeArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            patterns: vec![PatternType::Structuring, PatternType::Layering],
            range: "30d".to_string(),
            format: OutputFormat::Table,
        };
        let result = handle_analyze(args).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_compliance_report() {
        let args = ComplianceReportArgs {
            target: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            jurisdiction: Jurisdiction::US,
            report_type: ReportType::Summary,
            output: "/tmp/test_compliance.json".to_string(),
        };
        let result = handle_compliance_report(args).await;
        assert!(result.is_ok()); // Not yet implemented → prints message
    }

    #[tokio::test]
    async fn test_handle_compliance_report_eu_detailed() {
        let args = ComplianceReportArgs {
            target: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            jurisdiction: Jurisdiction::EU,
            report_type: ReportType::Detailed,
            output: "/tmp/test_compliance_eu.json".to_string(),
        };
        let result = handle_compliance_report(args).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_compliance_report_uk_sar() {
        let args = ComplianceReportArgs {
            target: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            jurisdiction: Jurisdiction::UK,
            report_type: ReportType::SAR,
            output: "/tmp/test_compliance_uk.json".to_string(),
        };
        let result = handle_compliance_report(args).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_compliance_report_singapore_travel_rule() {
        let args = ComplianceReportArgs {
            target: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            jurisdiction: Jurisdiction::Singapore,
            report_type: ReportType::TravelRule,
            output: "/tmp/test_compliance_sg.json".to_string(),
        };
        let result = handle_compliance_report(args).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_compliance_report_switzerland() {
        let args = ComplianceReportArgs {
            target: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            jurisdiction: Jurisdiction::Switzerland,
            report_type: ReportType::Summary,
            output: "/tmp/test_compliance_ch.json".to_string(),
        };
        let result = handle_compliance_report(args).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_risk_yaml_format() {
        unsafe { std::env::remove_var("ETHERSCAN_API_KEY") };
        let args = RiskArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: Some("ethereum".to_string()),
            format: OutputFormat::Yaml,
            detailed: false,
            output: None,
        };
        let result = handle_risk(args).await;
        assert!(result.is_ok());
    }

    // ========================================================================
    // Tests with injected mockito client (covers API-present paths)
    // ========================================================================

    fn mock_etherscan_response(txs: &[serde_json::Value]) -> String {
        serde_json::json!({
            "status": "1",
            "message": "OK",
            "result": txs
        })
        .to_string()
    }

    fn make_mock_client(base_url: &str) -> BlockchainDataClient {
        let sources = DataSources::new("test_api_key".to_string());
        BlockchainDataClient::with_base_url(sources, base_url)
    }

    #[tokio::test]
    async fn test_handle_risk_with_api_client() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_body(mock_etherscan_response(&[]))
            .create_async()
            .await;

        let client = make_mock_client(&server.url());
        let args = RiskArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: Some("ethereum".to_string()),
            format: OutputFormat::Table,
            detailed: true,
            output: None,
        };
        let result = handle_risk_with_client(args, Some(client)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_risk_with_api_client_json_export() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_body(mock_etherscan_response(&[]))
            .create_async()
            .await;

        let client = make_mock_client(&server.url());
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_string_lossy().to_string();

        let args = RiskArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: Some("ethereum".to_string()),
            format: OutputFormat::Table,
            detailed: false,
            output: Some(path.clone()),
        };
        let result = handle_risk_with_client(args, Some(client)).await;
        assert!(result.is_ok());
        assert!(std::path::Path::new(&path).exists());
    }

    #[tokio::test]
    async fn test_handle_trace_with_api_client() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_body(mock_etherscan_response(&[serde_json::json!({
                "hash": "0xabc",
                "from": "0x111",
                "to": "0x222",
                "value": "1000000000000000000",
                "timeStamp": "1700000000",
                "blockNumber": "18000000",
                "gasUsed": "21000",
                "gasPrice": "50000000000",
                "isError": "0",
                "input": "0x"
            })]))
            .create_async()
            .await;

        let client = make_mock_client(&server.url());
        let args = TraceArgs {
            tx_hash: "0xabc123def456".to_string(),
            depth: 2,
            flag_suspicious: true,
            format: OutputFormat::Table,
        };
        let result = handle_trace_with_client(args, Some(client)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_trace_with_api_client_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_body(r#"{"status":"0","message":"NOTOK","result":"Error"}"#)
            .create_async()
            .await;

        let client = make_mock_client(&server.url());
        let args = TraceArgs {
            tx_hash: "0xabc123def456".to_string(),
            depth: 3,
            flag_suspicious: false,
            format: OutputFormat::Table,
        };
        // Error path: should print error but return Ok
        let result = handle_trace_with_client(args, Some(client)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_analyze_with_api_client() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_body(mock_etherscan_response(&[serde_json::json!({
                "hash": "0xabc",
                "from": "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
                "to": "0x222",
                "value": "1000000000000000000",
                "timeStamp": "1700000000",
                "blockNumber": "18000000",
                "gasUsed": "21000",
                "gasPrice": "50000000000",
                "isError": "0",
                "input": "0x"
            })]))
            .create_async()
            .await;

        let client = make_mock_client(&server.url());
        let args = AnalyzeArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            patterns: vec![PatternType::Structuring, PatternType::Velocity],
            range: "30d".to_string(),
            format: OutputFormat::Table,
        };
        let result = handle_analyze_with_client(args, Some(client)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_analyze_with_api_client_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_body(r#"{"status":"0","message":"NOTOK","result":"Error"}"#)
            .create_async()
            .await;

        let client = make_mock_client(&server.url());
        let args = AnalyzeArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            patterns: vec![PatternType::Layering],
            range: "7d".to_string(),
            format: OutputFormat::Table,
        };
        // Error path in analyze
        let result = handle_analyze_with_client(args, Some(client)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_analyze_with_detect_chain_failure() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_body(mock_etherscan_response(&[]))
            .create_async()
            .await;

        let client = make_mock_client(&server.url());
        // Address that won't auto-detect → falls back to "ethereum"
        let args = AnalyzeArgs {
            address: "unknown_format_addr".to_string(),
            patterns: vec![PatternType::Integration],
            range: "1y".to_string(),
            format: OutputFormat::Json,
        };
        let result = handle_analyze_with_client(args, Some(client)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_risk_markdown_detailed() {
        unsafe { std::env::remove_var("ETHERSCAN_API_KEY") };
        let args = RiskArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: Some("ethereum".to_string()),
            format: OutputFormat::Markdown,
            detailed: true,
            output: None,
        };
        let result = handle_risk(args).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_trace_no_flag_suspicious() {
        unsafe { std::env::remove_var("ETHERSCAN_API_KEY") };
        let args = TraceArgs {
            tx_hash: "0xdef456".to_string(),
            depth: 5,
            flag_suspicious: false,
            format: OutputFormat::Json,
        };
        let result = handle_trace(args).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_analyze_all_patterns() {
        unsafe { std::env::remove_var("ETHERSCAN_API_KEY") };
        let args = AnalyzeArgs {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            patterns: vec![
                PatternType::Structuring,
                PatternType::Layering,
                PatternType::Integration,
                PatternType::Velocity,
                PatternType::RoundNumbers,
            ],
            range: "6m".to_string(),
            format: OutputFormat::Json,
        };
        let result = handle_analyze(args).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_pattern_type_debug() {
        let patterns = [
            PatternType::Structuring,
            PatternType::Layering,
            PatternType::Integration,
            PatternType::Velocity,
            PatternType::RoundNumbers,
        ];
        for p in &patterns {
            let debug = format!("{:?}", p);
            assert!(!debug.is_empty());
        }
    }

    #[test]
    fn test_jurisdiction_debug() {
        let jurisdictions = [
            Jurisdiction::US,
            Jurisdiction::EU,
            Jurisdiction::UK,
            Jurisdiction::Switzerland,
            Jurisdiction::Singapore,
        ];
        for j in &jurisdictions {
            let debug = format!("{:?}", j);
            assert!(!debug.is_empty());
        }
    }

    #[test]
    fn test_report_type_debug() {
        let types = [
            ReportType::Summary,
            ReportType::Detailed,
            ReportType::SAR,
            ReportType::TravelRule,
        ];
        for t in &types {
            let debug = format!("{:?}", t);
            assert!(!debug.is_empty());
        }
    }
}
