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
