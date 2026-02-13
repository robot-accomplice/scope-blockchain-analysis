//! # Address Report Generator
//!
//! Generates markdown reports for blockchain address analysis.

use super::address::AddressReport;
use crate::display::report::{report_footer, save_report};
use crate::error::Result;
use chrono::{DateTime, Utc};
use std::path::Path;

/// Generates a markdown report from an address analysis.
pub fn generate_address_report(report: &AddressReport) -> String {
    generate_address_report_core(report, true, true)
}

/// Generates report content without top-level header or footer (for batch reports).
pub fn generate_address_report_section(report: &AddressReport) -> String {
    generate_address_report_core(report, false, false)
}

/// Generates a combined dossier report: address analysis + risk assessment.
/// Used when `scope address --dossier` is run with `--report`.
pub fn generate_dossier_report(
    report: &AddressReport,
    risk: &crate::compliance::risk::RiskAssessment,
) -> String {
    let mut md = String::new();
    md.push_str("# Wallet Dossier\n\n");
    md.push_str(&format!(
        "**Address:** `{}`  \n**Chain:** {}  \n**Generated:** {}  \n\n",
        report.address,
        capitalize_chain(&report.chain),
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    ));
    md.push_str("---\n\n");
    md.push_str(&report_balance(report));
    md.push_str("\n---\n\n");
    md.push_str(&report_transactions(report));
    md.push_str("\n---\n\n");
    md.push_str(&report_tokens(report));
    md.push_str("\n---\n\n");
    md.push_str("## Risk Assessment\n\n");
    md.push_str(&crate::display::format_risk_report(
        risk,
        crate::display::OutputFormat::Markdown,
        true,
    ));
    md.push_str(&report_footer());
    md
}

fn generate_address_report_core(
    report: &AddressReport,
    include_header: bool,
    include_footer: bool,
) -> String {
    let mut md = String::new();

    if include_header {
        md.push_str(&report_header(report));
        md.push_str("\n---\n\n");
    }
    md.push_str(&report_balance(report));
    md.push_str("\n---\n\n");
    md.push_str(&report_transactions(report));
    md.push_str("\n---\n\n");
    md.push_str(&report_tokens(report));
    if include_footer {
        md.push_str(&report_footer());
    }

    md
}

fn report_header(report: &AddressReport) -> String {
    format!(
        "# Address Analysis Report\n\n\
        **Address:** `{}`  \n\
        **Chain:** {}  \n\
        **Generated:** {}  \n",
        report.address,
        capitalize_chain(&report.chain),
        Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    )
}

fn report_balance(report: &AddressReport) -> String {
    let mut s = String::from("## Balance Summary\n\n");
    s.push_str("| Metric | Value |\n|--------|-------|\n");
    s.push_str(&format!(
        "| Native Balance | {} |\n",
        report.balance.formatted
    ));
    if let Some(usd) = report.balance.usd {
        s.push_str(&format!("| USD Value | ${:.2} |\n", usd));
    }
    s.push_str(&format!(
        "| Transaction Count | {} |\n",
        report.transaction_count
    ));
    s
}

fn report_transactions(report: &AddressReport) -> String {
    let mut s = String::from("## Recent Transactions\n\n");
    match &report.transactions {
        Some(txs) if !txs.is_empty() => {
            s.push_str("| Hash | Block | Time | From | To | Value | Status |\n");
            s.push_str("|------|-------|------|------|-----|-------|--------|\n");
            for tx in txs.iter().take(20) {
                let ts = DateTime::<Utc>::from_timestamp(tx.timestamp as i64, 0)
                    .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "-".to_string());
                let hash_short = if tx.hash.len() > 10 {
                    format!("{}...{}", &tx.hash[..6], &tx.hash[tx.hash.len() - 4..])
                } else {
                    tx.hash.clone()
                };
                let to = tx.to.as_deref().unwrap_or("-");
                let status = if tx.status { "✓" } else { "✗" };
                s.push_str(&format!(
                    "| `{}` | {} | {} | `{}` | `{}` | {} | {} |\n",
                    hash_short, tx.block_number, ts, tx.from, to, tx.value, status
                ));
            }
            if txs.len() > 20 {
                s.push_str(&format!("\n*Showing 20 of {} transactions*\n", txs.len()));
            }
        }
        _ => s.push_str("*No transaction data available*\n"),
    }
    s
}

fn report_tokens(report: &AddressReport) -> String {
    let mut s = String::from("## Token Balances\n\n");
    match &report.tokens {
        Some(tokens) if !tokens.is_empty() => {
            s.push_str("| Token | Contract | Balance |\n");
            s.push_str("|-------|----------|--------|\n");
            for t in tokens {
                s.push_str(&format!(
                    "| {} ({}) | `{}` | {} |\n",
                    t.name, t.symbol, t.contract_address, t.formatted_balance
                ));
            }
        }
        _ => s.push_str("*No token balance data available*\n"),
    }
    s
}

fn capitalize_chain(chain: &str) -> String {
    let mut chars = chain.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().chain(chars).collect(),
    }
}

/// Saves an address report to a file.
pub fn save_address_report(report: &str, path: impl AsRef<Path>) -> Result<()> {
    save_report(report, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::address::{AddressReport, Balance, TokenBalance, TransactionSummary};

    fn minimal_report() -> AddressReport {
        AddressReport {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            balance: Balance {
                raw: "1000000000000000000".to_string(),
                formatted: "1.0 ETH".to_string(),
                usd: Some(3500.0),
            },
            transaction_count: 10,
            transactions: None,
            tokens: None,
        }
    }

    #[test]
    fn test_generate_address_report_section_minimal() {
        let report = minimal_report();
        let md = generate_address_report_section(&report);
        assert!(md.contains("Balance Summary"));
        assert!(md.contains("1.0 ETH"));
        assert!(md.contains("$3500.00"));
        assert!(md.contains("Transaction Count"));
        assert!(md.contains("No transaction data available"));
        assert!(md.contains("No token balance data available"));
    }

    #[test]
    fn test_generate_address_report_section_with_transactions() {
        let mut report = minimal_report();
        report.transactions = Some(vec![TransactionSummary {
            hash: "0xabc123def456".to_string(),
            block_number: 12345,
            timestamp: 1700000000,
            from: "0xfrom123".to_string(),
            to: Some("0xto456".to_string()),
            value: "1 ETH".to_string(),
            status: true,
        }]);
        let md = generate_address_report_section(&report);
        assert!(md.contains("Recent Transactions"));
        assert!(md.contains("0xabc1"));
        assert!(md.contains("12345"));
    }

    #[test]
    fn test_generate_address_report_section_with_tokens() {
        let mut report = minimal_report();
        report.tokens = Some(vec![TokenBalance {
            contract_address: "0xusdc".to_string(),
            symbol: "USDC".to_string(),
            name: "USD Coin".to_string(),
            decimals: 6,
            balance: "1000000".to_string(),
            formatted_balance: "1.0 USDC".to_string(),
        }]);
        let md = generate_address_report_section(&report);
        assert!(md.contains("Token Balances"));
        assert!(md.contains("USDC"));
        assert!(md.contains("USD Coin"));
    }

    #[test]
    fn test_generate_address_report_full() {
        let report = minimal_report();
        let md = generate_address_report(&report);
        assert!(md.contains("Address Analysis Report"));
        assert!(md.contains("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"));
        assert!(md.contains("Ethereum"));
    }

    #[test]
    fn test_capitalize_chain_empty() {
        // Covers line 156: capitalize_chain with empty string → None branch
        assert_eq!(capitalize_chain(""), "");
    }

    #[test]
    fn test_save_address_report_to_file() {
        // Covers lines 162-163: save_address_report delegates to save_report
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_addr_report.md");
        let result = save_address_report("# Test Report\n\nSome content", &path);
        assert!(result.is_ok());
        assert!(path.exists());
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("Test Report"));
    }

    #[test]
    fn test_report_transactions_more_than_20() {
        // Covers line 127: "Showing 20 of N transactions" when > 20 txs
        let mut report = minimal_report();
        let txs: Vec<TransactionSummary> = (0..25)
            .map(|i| TransactionSummary {
                hash: format!("0x{:064x}", i),
                block_number: 12345 + i,
                timestamp: 1700000000 + i * 60,
                from: "0xfrom".to_string(),
                to: Some("0xto".to_string()),
                value: "0.1 ETH".to_string(),
                status: true,
            })
            .collect();
        report.transactions = Some(txs);
        let md = generate_address_report_section(&report);
        assert!(md.contains("Showing 20 of 25 transactions"));
    }

    #[test]
    fn test_generate_dossier_report() {
        use crate::compliance::risk::{RiskAssessment, RiskCategory, RiskFactor, RiskLevel};
        use chrono::Utc;

        let report = minimal_report();
        let risk = RiskAssessment {
            address: report.address.clone(),
            chain: report.chain.clone(),
            overall_score: 3.5,
            risk_level: RiskLevel::Low,
            factors: vec![RiskFactor {
                name: "Test factor".to_string(),
                category: RiskCategory::Behavioral,
                score: 3.0,
                weight: 0.5,
                description: "A test risk factor".to_string(),
                evidence: vec!["evidence".to_string()],
            }],
            assessed_at: Utc::now(),
            recommendations: vec!["Be cautious".to_string()],
        };
        let md = generate_dossier_report(&report, &risk);
        assert!(md.contains("Wallet Dossier"));
        assert!(md.contains("Risk Assessment"));
        assert!(md.contains("Test factor"));
    }
}
