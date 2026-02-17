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
#[command(
    after_help = "\x1b[1mExamples:\x1b[0m
  scope contract 0xdAC17F958D2ee523a2206206994597C13D831ec7
  scope ct @usdt-contract                                 \x1b[2m# address book shortcut\x1b[0m
  scope ct 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --chain polygon
  scope contract 0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D --json",
    after_long_help = "\x1b[1mExamples:\x1b[0m

  \x1b[1m$ scope contract 0xdAC17F958D2ee523a2206206994597C13D831ec7\x1b[0m

  ========================================================================
    CONTRACT ANALYSIS: 0xdAC17F958D2ee523a2206206994597C13D831ec7
    Chain: ethereum | Verified: Yes
  ========================================================================

    Security Score: [################----] 80/100

  --- Source Code ---
    Contract Name: TetherToken
    Compiler: v0.4.18+commit.9cf6e910
    Optimization: No

  --- Proxy Detection ---
    Not a proxy contract

  --- Access Control ---
    Ownership: Ownable
    Renounced: No
    Privileged functions:
      - pause (High): Can pause transfers
      - addBlacklist (High): Can blacklist addresses

  --- Vulnerability Findings ---
    [. ] SC-TX-ORIGIN - tx.origin authorization (Low)

  --- DeFi Analysis ---
    Protocol Type: Token
    Token Standards: ERC-20

  --- External Intelligence ---
    Explorer: https://etherscan.io/address/0xdAC17...
    Sourcify: Verified
    Audit Reports:
      - Trail of Bits (TetherToken)

  ========================================================================

  \x1b[1m$ scope ct 0xA0b86991... --json\x1b[0m

  {
    \"address\": \"0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48\",
    \"chain\": \"ethereum\",
    \"is_verified\": true,
    \"security_score\": 85,
    \"security_summary\": \"Verified contract with ...\",
    \"source_info\": { ... },
    \"proxy_info\": { ... },
    \"vulnerabilities\": [ ... ],
    ...
  }"
)]
pub struct ContractArgs {
    /// Contract address to analyze.
    ///
    /// Must be a valid address on the target chain. The address must be
    /// a deployed smart contract (not an externally owned account).
    /// Use @label to resolve from the address book (e.g., @usdt-contract).
    #[arg(value_name = "ADDRESS")]
    pub address: String,

    /// Target blockchain network.
    ///
    /// EVM chains with Etherscan-compatible APIs:
    /// ethereum, polygon, arbitrum, optimism, base, bsc
    #[arg(long, short, default_value = "ethereum")]
    pub chain: String,

    /// Output raw JSON instead of formatted report.
    ///
    /// Useful for piping to `jq` or feeding to other tools.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::ContractAnalysis;

    fn minimal_analysis() -> ContractAnalysis {
        ContractAnalysis {
            address: "0xtest".to_string(),
            chain: "ethereum".to_string(),
            is_verified: false,
            source_info: None,
            proxy_info: None,
            access_control: None,
            vulnerabilities: vec![],
            defi_analysis: None,
            external_info: None,
            security_score: 30,
            security_summary: "Unverified contract".to_string(),
        }
    }

    #[test]
    fn test_print_report_minimal() {
        print_contract_report(&minimal_analysis());
    }

    #[test]
    fn test_print_report_verified_with_source() {
        let mut a = minimal_analysis();
        a.is_verified = true;
        a.security_score = 75;
        a.source_info = Some(crate::contract::source::ContractSource {
            contract_name: "TestToken".to_string(),
            source_code: "contract T {}".to_string(),
            abi: "[]".to_string(),
            compiler_version: "v0.8.19".to_string(),
            optimization_used: true,
            optimization_runs: 200,
            evm_version: "paris".to_string(),
            license_type: "MIT".to_string(),
            is_proxy: false,
            implementation_address: None,
            constructor_arguments: String::new(),
            library: String::new(),
            swarm_source: String::new(),
            parsed_abi: vec![],
        });
        print_contract_report(&a);
    }

    #[test]
    fn test_print_report_source_no_optimization() {
        let mut a = minimal_analysis();
        a.is_verified = true;
        a.source_info = Some(crate::contract::source::ContractSource {
            contract_name: "T".to_string(),
            source_code: String::new(),
            abi: "[]".to_string(),
            compiler_version: "v0.8.19".to_string(),
            optimization_used: false,
            optimization_runs: 0,
            evm_version: "paris".to_string(),
            license_type: "MIT".to_string(),
            is_proxy: false,
            implementation_address: None,
            constructor_arguments: String::new(),
            library: String::new(),
            swarm_source: String::new(),
            parsed_abi: vec![],
        });
        print_contract_report(&a);
    }

    #[test]
    fn test_print_report_with_proxy() {
        let mut a = minimal_analysis();
        a.proxy_info = Some(crate::contract::proxy::ProxyInfo {
            is_proxy: true,
            proxy_type: "EIP-1967".to_string(),
            implementation_address: Some("0ximpl".to_string()),
            admin_address: Some("0xadmin".to_string()),
            beacon_address: None,
            details: vec!["Proxy detected".to_string()],
        });
        print_contract_report(&a);
    }

    #[test]
    fn test_print_report_not_proxy() {
        let mut a = minimal_analysis();
        a.proxy_info = Some(crate::contract::proxy::ProxyInfo {
            is_proxy: false,
            proxy_type: "None".to_string(),
            implementation_address: None,
            admin_address: None,
            beacon_address: None,
            details: vec![],
        });
        print_contract_report(&a);
    }

    #[test]
    fn test_print_report_access_control() {
        let mut a = minimal_analysis();
        a.access_control = Some(crate::contract::access::AccessControlMap {
            ownership_pattern: Some("Ownable".to_string()),
            has_renounced_ownership: true,
            has_role_based_access: true,
            uses_tx_origin: true,
            tx_origin_locations: vec![],
            modifiers: vec![],
            privileged_functions: vec![crate::contract::access::PrivilegedFunction {
                name: "mint".to_string(),
                modifiers: vec!["onlyOwner".to_string()],
                capability: "Mint tokens".to_string(),
                risk: crate::contract::access::PrivilegeRisk::Critical,
            }],
            roles: vec!["MINTER_ROLE".to_string()],
            auth_analysis: crate::contract::access::AuthAnalysis {
                msg_sender_checks: 1,
                tx_origin_checks: 1,
                has_origin_sender_comparison: false,
                summary: "Mixed auth".to_string(),
            },
        });
        print_contract_report(&a);
    }

    #[test]
    fn test_print_report_vulns() {
        let mut a = minimal_analysis();
        a.vulnerabilities = vec![
            contract::vulnerability::VulnerabilityFinding {
                id: "V-1".to_string(),
                title: "Critical issue".to_string(),
                severity: contract::vulnerability::Severity::Critical,
                category: contract::vulnerability::VulnCategory::Reentrancy,
                description: "desc".to_string(),
                source_location: None,
                recommendation: "fix".to_string(),
            },
            contract::vulnerability::VulnerabilityFinding {
                id: "V-2".to_string(),
                title: "High issue".to_string(),
                severity: contract::vulnerability::Severity::High,
                category: contract::vulnerability::VulnCategory::UncheckedCall,
                description: "desc".to_string(),
                source_location: None,
                recommendation: "fix".to_string(),
            },
            contract::vulnerability::VulnerabilityFinding {
                id: "V-3".to_string(),
                title: "Medium".to_string(),
                severity: contract::vulnerability::Severity::Medium,
                category: contract::vulnerability::VulnCategory::Delegatecall,
                description: "desc".to_string(),
                source_location: None,
                recommendation: "fix".to_string(),
            },
            contract::vulnerability::VulnerabilityFinding {
                id: "V-4".to_string(),
                title: "Low".to_string(),
                severity: contract::vulnerability::Severity::Low,
                category: contract::vulnerability::VulnCategory::TxOrigin,
                description: "desc".to_string(),
                source_location: None,
                recommendation: "fix".to_string(),
            },
            contract::vulnerability::VulnerabilityFinding {
                id: "V-5".to_string(),
                title: "Info".to_string(),
                severity: contract::vulnerability::Severity::Informational,
                category: contract::vulnerability::VulnCategory::Informational,
                description: "desc".to_string(),
                source_location: None,
                recommendation: "fix".to_string(),
            },
        ];
        print_contract_report(&a);
    }

    #[test]
    fn test_print_report_defi() {
        let mut a = minimal_analysis();
        a.defi_analysis = Some(crate::contract::defi::DefiAnalysis {
            protocol_type: crate::contract::defi::ProtocolType::DEX,
            has_oracle_dependency: true,
            oracle_info: vec![crate::contract::defi::OracleInfo {
                provider: "Chainlink".to_string(),
                usage: "Price feed".to_string(),
                risks: vec![],
            }],
            has_flash_loan_risk: true,
            flash_loan_info: vec!["Flash loan detected".to_string()],
            dex_integrations: vec![crate::contract::defi::DexIntegration {
                dex: "Uniswap".to_string(),
                integration_type: "Swap".to_string(),
                has_slippage_protection: false,
                has_deadline_protection: true,
            }],
            lending_patterns: vec![],
            token_standards: vec![crate::contract::defi::TokenStandard::ERC20],
            staking_patterns: vec![],
            risk_factors: vec![crate::contract::defi::DefiRiskFactor {
                name: "Test risk".to_string(),
                description: "A risk".to_string(),
                severity: 7,
            }],
        });
        print_contract_report(&a);
    }

    #[test]
    fn test_print_report_external() {
        let mut a = minimal_analysis();
        a.external_info = Some(crate::contract::external::ExternalInfo {
            explorer_url: "https://etherscan.io/address/0xtest".to_string(),
            github_repo: Some("https://github.com/test/repo".to_string()),
            sourcify_verified: Some(true),
            deployer: None,
            audit_reports: vec![crate::contract::external::AuditReport {
                auditor: "Trail of Bits".to_string(),
                scope: "Token".to_string(),
                url: "https://audit.com".to_string(),
                date: None,
            }],
            metadata: vec![],
        });
        print_contract_report(&a);
    }

    #[test]
    fn test_print_report_external_sourcify_false() {
        let mut a = minimal_analysis();
        a.external_info = Some(crate::contract::external::ExternalInfo {
            explorer_url: "https://etherscan.io/address/0xtest".to_string(),
            github_repo: None,
            sourcify_verified: Some(false),
            deployer: None,
            audit_reports: vec![],
            metadata: vec![],
        });
        print_contract_report(&a);
    }

    #[test]
    fn test_print_report_access_control_empty_roles() {
        let mut a = minimal_analysis();
        a.access_control = Some(crate::contract::access::AccessControlMap {
            ownership_pattern: Some("Ownable".to_string()),
            has_renounced_ownership: false,
            has_role_based_access: false,
            uses_tx_origin: false,
            tx_origin_locations: vec![],
            modifiers: vec![],
            privileged_functions: vec![],
            roles: vec![],
            auth_analysis: crate::contract::access::AuthAnalysis {
                msg_sender_checks: 0,
                tx_origin_checks: 0,
                has_origin_sender_comparison: false,
                summary: "No auth checks".to_string(),
            },
        });
        print_contract_report(&a);
    }

    #[test]
    fn test_print_report_external_audit_with_url() {
        let mut a = minimal_analysis();
        a.external_info = Some(crate::contract::external::ExternalInfo {
            explorer_url: "https://etherscan.io/address/0xtest".to_string(),
            github_repo: None,
            sourcify_verified: None,
            deployer: None,
            audit_reports: vec![crate::contract::external::AuditReport {
                auditor: "CertiK".to_string(),
                scope: "Full".to_string(),
                url: "https://certik.com/audit.pdf".to_string(),
                date: None,
            }],
            metadata: vec![],
        });
        print_contract_report(&a);
    }

    #[test]
    fn test_print_report_access_control_with_roles() {
        let mut a = minimal_analysis();
        a.access_control = Some(crate::contract::access::AccessControlMap {
            ownership_pattern: None,
            has_renounced_ownership: false,
            has_role_based_access: true,
            uses_tx_origin: false,
            tx_origin_locations: vec![],
            modifiers: vec![],
            privileged_functions: vec![],
            roles: vec!["ADMIN_ROLE".to_string(), "MINTER_ROLE".to_string()],
            auth_analysis: crate::contract::access::AuthAnalysis {
                msg_sender_checks: 2,
                tx_origin_checks: 0,
                has_origin_sender_comparison: false,
                summary: "Role-based".to_string(),
            },
        });
        print_contract_report(&a);
    }

    #[test]
    fn test_print_report_defi_empty_token_standards() {
        let mut a = minimal_analysis();
        a.defi_analysis = Some(crate::contract::defi::DefiAnalysis {
            protocol_type: crate::contract::defi::ProtocolType::Other,
            has_oracle_dependency: false,
            oracle_info: vec![],
            has_flash_loan_risk: false,
            flash_loan_info: vec![],
            dex_integrations: vec![],
            lending_patterns: vec![],
            token_standards: vec![],
            staking_patterns: vec![],
            risk_factors: vec![],
        });
        print_contract_report(&a);
    }

    #[test]
    fn test_print_report_proxy_no_impl_or_admin() {
        let mut a = minimal_analysis();
        a.proxy_info = Some(crate::contract::proxy::ProxyInfo {
            is_proxy: true,
            proxy_type: "Minimal Proxy".to_string(),
            implementation_address: None,
            admin_address: None,
            beacon_address: None,
            details: vec!["Minimal proxy".to_string()],
        });
        print_contract_report(&a);
    }
}
