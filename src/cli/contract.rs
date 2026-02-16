//! # Contract Analysis Command
//!
//! Performs comprehensive smart contract analysis including source code
//! retrieval, proxy detection, access control mapping, vulnerability scanning,
//! DeFi protocol checks, and external intelligence gathering.

use crate::chains::ChainClientFactory;
use crate::config::Config;
use crate::contract;
use crate::error::Result;
use clap::Args;

/// Arguments for the contract analysis command.
#[derive(Debug, Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope contract 0xdAC17F958D2ee523a2206206994597C13D831ec7
  scope contract 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --chain ethereum
  scope contract 0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D --json")]
pub struct ContractArgs {
    /// Contract address to analyze.
    pub address: String,

    /// Chain to use (default: ethereum).
    #[arg(long, short, default_value = "ethereum")]
    pub chain: String,

    /// Output raw JSON instead of formatted report.
    #[arg(long)]
    pub json: bool,
}

/// Run the contract analysis command.
pub async fn run(
    args: &ContractArgs,
    _config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    let spinner = crate::cli::progress::Spinner::new("Analyzing contract...");

    let client = clients.create_chain_client(&args.chain)?;
    let http_client = reqwest::Client::new();

    let analysis =
        contract::analyze_contract(&args.address, &args.chain, client.as_ref(), &http_client)
            .await?;

    spinner.finish("Contract analysis complete");

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&analysis)
                .unwrap_or_else(|_| "Failed to serialize".to_string())
        );
    } else {
        print_contract_report(&analysis);
    }

    Ok(())
}

/// Print a formatted contract analysis report to the terminal.
fn print_contract_report(analysis: &contract::ContractAnalysis) {
    println!("\n{}", "=".repeat(72));
    println!("  CONTRACT ANALYSIS: {}", analysis.address);
    println!(
        "  Chain: {} | Verified: {}",
        analysis.chain,
        if analysis.is_verified { "Yes" } else { "No" }
    );
    println!("{}", "=".repeat(72));

    // Security Score
    let score_bar = format!(
        "[{}{}] {}/100",
        "#".repeat((analysis.security_score as usize) / 5),
        "-".repeat(20 - (analysis.security_score as usize) / 5),
        analysis.security_score
    );
    println!("\n  Security Score: {}", score_bar);
    println!("  {}", analysis.security_summary);

    // Source Info
    if let Some(src) = &analysis.source_info {
        println!("\n--- Source Code ---");
        println!("  Contract Name: {}", src.contract_name);
        println!("  Compiler: {}", src.compiler_version);
        println!("  EVM Version: {}", src.evm_version);
        println!("  License: {}", src.license_type);
        println!(
            "  Optimization: {}",
            if src.optimization_used {
                format!("Yes ({} runs)", src.optimization_runs)
            } else {
                "No".to_string()
            }
        );
        println!("  ABI Functions: {}", src.parsed_abi.len());
    }

    // Proxy Info
    if let Some(proxy) = &analysis.proxy_info {
        println!("\n--- Proxy Detection ---");
        if proxy.is_proxy {
            println!("  Type: {}", proxy.proxy_type);
            if let Some(impl_addr) = &proxy.implementation_address {
                println!("  Implementation: {}", impl_addr);
            }
            if let Some(admin) = &proxy.admin_address {
                println!("  Admin: {}", admin);
            }
        } else {
            println!("  Not a proxy contract");
        }
        for detail in &proxy.details {
            println!("  - {}", detail);
        }
    }

    // Access Control
    if let Some(ac) = &analysis.access_control {
        println!("\n--- Access Control ---");
        if let Some(pattern) = &ac.ownership_pattern {
            println!("  Ownership: {}", pattern);
        }
        println!(
            "  Renounced: {}",
            if ac.has_renounced_ownership {
                "Yes"
            } else {
                "No"
            }
        );
        println!(
            "  Role-based: {}",
            if ac.has_role_based_access {
                "Yes"
            } else {
                "No"
            }
        );
        if ac.uses_tx_origin {
            println!("  WARNING: Uses tx.origin for authorization");
        }
        if !ac.roles.is_empty() {
            println!("  Roles: {}", ac.roles.join(", "));
        }
        if !ac.privileged_functions.is_empty() {
            println!("  Privileged functions:");
            for pf in &ac.privileged_functions {
                println!("    - {} ({:?}): {}", pf.name, pf.risk, pf.capability);
            }
        }
        println!("\n  Auth: {}", ac.auth_analysis.summary);
    }

    // Vulnerabilities
    if !analysis.vulnerabilities.is_empty() {
        println!("\n--- Vulnerability Findings ---");
        for vuln in &analysis.vulnerabilities {
            let severity_indicator = match vuln.severity {
                contract::vulnerability::Severity::Critical => "[!!]",
                contract::vulnerability::Severity::High => "[! ]",
                contract::vulnerability::Severity::Medium => "[* ]",
                contract::vulnerability::Severity::Low => "[. ]",
                contract::vulnerability::Severity::Informational => "[i ]",
            };
            println!(
                "  {} {} - {} ({})",
                severity_indicator, vuln.id, vuln.title, vuln.severity
            );
            println!("      {}", vuln.description);
            println!("      Fix: {}", vuln.recommendation);
        }
    } else {
        println!("\n--- Vulnerability Findings ---");
        println!("  No heuristic findings triggered.");
    }

    // DeFi Analysis
    if let Some(defi) = &analysis.defi_analysis {
        println!("\n--- DeFi Analysis ---");
        println!("  Protocol Type: {}", defi.protocol_type);
        if !defi.token_standards.is_empty() {
            let standards: Vec<String> =
                defi.token_standards.iter().map(|s| s.to_string()).collect();
            println!("  Token Standards: {}", standards.join(", "));
        }
        if defi.has_oracle_dependency {
            for oracle in &defi.oracle_info {
                println!("  Oracle: {} ({})", oracle.provider, oracle.usage);
            }
        }
        if defi.has_flash_loan_risk {
            println!("  Flash Loan Risk: Yes");
        }
        for dex in &defi.dex_integrations {
            println!(
                "  DEX: {} - slippage: {}, deadline: {}",
                dex.dex,
                if dex.has_slippage_protection {
                    "Yes"
                } else {
                    "NO"
                },
                if dex.has_deadline_protection {
                    "Yes"
                } else {
                    "NO"
                }
            );
        }
        if !defi.risk_factors.is_empty() {
            println!("  Risk Factors:");
            for rf in &defi.risk_factors {
                println!(
                    "    - {} (severity {}/10): {}",
                    rf.name, rf.severity, rf.description
                );
            }
        }
    }

    // External Info
    if let Some(ext) = &analysis.external_info {
        println!("\n--- External Intelligence ---");
        println!("  Explorer: {}", ext.explorer_url);
        if let Some(repo) = &ext.github_repo {
            println!("  GitHub: {}", repo);
        }
        if let Some(verified) = &ext.sourcify_verified {
            println!(
                "  Sourcify: {}",
                if *verified {
                    "Verified"
                } else {
                    "Not verified"
                }
            );
        }
        if !ext.audit_reports.is_empty() {
            println!("  Audit Reports:");
            for report in &ext.audit_reports {
                println!("    - {} ({})", report.auditor, report.scope);
                if !report.url.is_empty() {
                    println!("      {}", report.url);
                }
            }
        }
    }

    println!("\n{}", "=".repeat(72));
}
