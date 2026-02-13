//! # Insights Command
//!
//! Infers the type of blockchain target (address, token, transaction) from input,
//! auto-detects chain, and runs relevant Scope analyses to produce unified insights.

use crate::chains::{
    infer_chain_from_address, infer_chain_from_hash, native_symbol, ChainClientFactory,
};
use crate::cli::address::{self, AddressArgs};
use crate::cli::crawl::{fetch_analytics_for_input, Period};
use crate::cli::tx::{format_tx_markdown, fetch_transaction_report};
use crate::config::Config;
use crate::display::report;
use crate::error::Result;
use crate::market::{BinanceClient, HealthThresholds, MarketSummary, OrderBookClient};
use crate::tokens::TokenAliases;
use clap::Args;

/// Target type inferred from user input.
#[derive(Debug, Clone)]
pub enum InferredTarget {
    /// Blockchain address (EVM, Tron, or Solana).
    Address { chain: String },
    /// Transaction hash.
    Transaction { chain: String },
    /// Token symbol, name, or contract address.
    Token { chain: String },
}

/// Arguments for the insights command.
#[derive(Debug, Args)]
pub struct InsightsArgs {
    /// Target to analyze: address, transaction hash, or token (symbol/name/address).
    ///
    /// Scope infers the type and chain from format:
    /// - `0x...` (42 chars) = EVM address → ethereum
    /// - `T...` (34 chars) = Tron address → tron
    /// - Base58 (32–44 chars) = Solana address → solana
    /// - `0x...` (66 chars) = EVM tx hash
    /// - 64 hex chars = Tron tx hash
    /// - Base58 (80–90 chars) = Solana signature
    /// - Otherwise = token symbol/name (e.g. USDC, WETH)
    pub target: String,

    /// Override detected chain (ethereum, polygon, solana, tron, etc.).
    #[arg(short, long)]
    pub chain: Option<String>,

    /// Include decoded transaction input (for tx targets).
    #[arg(long)]
    pub decode: bool,

    /// Include internal transaction trace (for tx targets).
    #[arg(long)]
    pub trace: bool,
}

/// Infers the target type and chain from the input string.
pub fn infer_target(input: &str, chain_override: Option<&str>) -> InferredTarget {
    let trimmed = input.trim();

    if let Some(chain) = chain_override {
        let chain = chain.to_lowercase();
        // With override, we still need to infer type
        if infer_chain_from_hash(trimmed).is_some() {
            return InferredTarget::Transaction { chain };
        }
        if TokenAliases::is_address(trimmed) {
            return InferredTarget::Address { chain };
        }
        return InferredTarget::Token { chain };
    }

    // Transaction hash (format implies chain)
    if let Some(chain) = infer_chain_from_hash(trimmed) {
        return InferredTarget::Transaction {
            chain: chain.to_string(),
        };
    }

    // Address (format implies chain)
    if TokenAliases::is_address(trimmed) {
        let chain = infer_chain_from_address(trimmed).unwrap_or("ethereum");
        return InferredTarget::Address {
            chain: chain.to_string(),
        };
    }

    // Default: token (symbol or name)
    InferredTarget::Token {
        chain: "ethereum".to_string(),
    }
}

/// Runs the insights command.
pub async fn run(
    args: InsightsArgs,
    _config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    let chain_override = args.chain.as_deref();
    let target = infer_target(&args.target, chain_override);

    let sp = crate::cli::progress::Spinner::new(&format!(
        "Analyzing {} on {}...",
        target_type_label(&target),
        chain_label(&target)
    ));

    let mut output = String::new();
    output.push_str("# Scope Insights\n\n");
    output.push_str(&format!("**Target:** `{}`\n\n", args.target));
    output.push_str(&format!(
        "**Detected:** {} on {}\n\n",
        target_type_label(&target),
        chain_label(&target)
    ));
    output.push_str("---\n\n");

    match &target {
        InferredTarget::Address { chain } => {
            output.push_str("## Observations\n\n");
            let addr_args = AddressArgs {
                address: args.target.clone(),
                chain: chain.clone(),
                format: Some(crate::config::OutputFormat::Markdown),
                include_txs: false,
                include_tokens: true,
                limit: 10,
                report: None,
                dossier: false,
            };
            let client = clients.create_chain_client(chain)?;
            let report = address::analyze_address(&addr_args, client.as_ref()).await?;

            // Contract vs EOA (EVM chains support get_code)
            let code_result = client.get_code(&args.target).await;
            let is_contract = code_result.as_ref().map_or(false, |c| !c.is_empty() && c != "0x");
            if code_result.is_ok() {
                output.push_str(&format!(
                    "- **Type:** {}\n",
                    if is_contract {
                        "Contract"
                    } else {
                        "Externally Owned Account (EOA)"
                    }
                ));
            }

            output.push_str(&format!(
                "- **Native balance:** {} ({})\n",
                report.balance.formatted,
                crate::chains::native_symbol(chain)
            ));
            if let Some(ref usd) = report.balance.usd {
                output.push_str(&format!("- **USD value:** ${:.2}\n", usd));
            }
            output.push_str(&format!(
                "- **Transaction count:** {}\n",
                report.transaction_count
            ));
            if let Some(ref tokens) = report.tokens {
                if !tokens.is_empty() {
                    output.push_str(&format!(
                        "- **Token holdings:** {} different tokens\n",
                        tokens.len()
                    ));
                    output.push_str("\n### Token Balances\n\n");
                    for tb in tokens.iter().take(10) {
                        output.push_str(&format!(
                            "- {}: {} ({})\n",
                            tb.symbol, tb.formatted_balance, tb.contract_address
                        ));
                    }
                    if tokens.len() > 10 {
                        output.push_str(&format!("\n*...and {} more*\n", tokens.len() - 10));
                    }
                }
            }

            // Risk assessment (compliance engine)
            let risk_assessment =
                match crate::compliance::datasource::BlockchainDataClient::from_env_opt() {
                    Some(data_client) => {
                        crate::compliance::risk::RiskEngine::with_data_client(data_client)
                            .assess_address(&args.target, chain)
                            .await
                            .ok()
                    }
                    None => crate::compliance::risk::RiskEngine::new()
                        .assess_address(&args.target, chain)
                        .await
                        .ok(),
                };

            if let Some(ref risk) = risk_assessment {
                output.push_str(&format!(
                    "\n- **Risk:** {} {:.1}/10 ({:?})\n",
                    risk.risk_level.emoji(),
                    risk.overall_score,
                    risk.risk_level
                ));
            }

            // Meta analysis
            let meta = meta_analysis_address(
                is_contract,
                report.balance.usd,
                report.tokens.as_ref().map(|t| t.len()).unwrap_or(0),
                risk_assessment.as_ref().map(|r| r.overall_score),
                risk_assessment.as_ref().map(|r| &r.risk_level),
            );
            output.push_str("\n### Synthesis\n\n");
            output.push_str(&format!("{}\n\n", meta.synthesis));
            output.push_str(&format!("**Key takeaway:** {}\n\n", meta.key_takeaway));
            if !meta.recommendations.is_empty() {
                output.push_str("**Consider:**\n");
                for rec in &meta.recommendations {
                    output.push_str(&format!("- {}\n", rec));
                }
            }
            output.push_str("\n---\n\n");
            let full_report = if let Some(ref risk) = risk_assessment {
                crate::cli::address_report::generate_dossier_report(&report, risk)
            } else {
                crate::cli::address_report::generate_address_report(&report)
            };
            output.push_str(&full_report);
        }
        InferredTarget::Transaction { chain } => {
            output.push_str("## Observations\n\n");
            let tx_report = fetch_transaction_report(
                &args.target,
                chain,
                args.decode,
                args.trace,
                clients,
            )
            .await?;

            let tx_type = classify_tx_type(
                &tx_report.transaction.input,
                tx_report.transaction.to.as_deref(),
            );
            output.push_str(&format!("- **Type:** {}\n", tx_type));

            output.push_str(&format!(
                "- **Status:** {}\n",
                if tx_report.transaction.status {
                    "Success"
                } else {
                    "Failed"
                }
            ));
            output.push_str(&format!(
                "- **From:** `{}`\n",
                tx_report.transaction.from
            ));
            output.push_str(&format!(
                "- **To:** `{}`\n",
                tx_report
                    .transaction
                    .to
                    .as_deref()
                    .unwrap_or("Contract Creation")
            ));

            let (formatted_value, high_value) = format_tx_value(
                &tx_report.transaction.value,
                chain,
            );
            output.push_str(&format!("- **Value:** {}\n", formatted_value));
            if high_value {
                output.push_str("- ⚠️ **High-value transfer**\n");
            }

            output.push_str(&format!("- **Fee:** {}\n", tx_report.gas.transaction_fee));

            // Meta analysis
            let meta = meta_analysis_tx(
                tx_type,
                tx_report.transaction.status,
                high_value,
                &tx_report.transaction.from,
                tx_report.transaction.to.as_deref(),
            );
            output.push_str("\n### Synthesis\n\n");
            output.push_str(&format!("{}\n\n", meta.synthesis));
            output.push_str(&format!("**Key takeaway:** {}\n\n", meta.key_takeaway));
            if !meta.recommendations.is_empty() {
                output.push_str("**Consider:**\n");
                for rec in &meta.recommendations {
                    output.push_str(&format!("- {}\n", rec));
                }
            }
            output.push_str("\n---\n\n");
            output.push_str(&format_tx_markdown(&tx_report));
        }
        InferredTarget::Token { chain } => {
            output.push_str("## Observations\n\n");
            let analytics = fetch_analytics_for_input(
                &args.target,
                chain,
                Period::Hour24,
                10,
                clients,
            )
            .await?;

            // Token risk summary (interpretive bullets)
            let risk_summary = report::token_risk_summary(&analytics);
            output.push_str(&format!(
                "- **Risk:** {} {}/10 ({})\n",
                risk_summary.emoji,
                risk_summary.score,
                risk_summary.level
            ));
            if !risk_summary.concerns.is_empty() {
                for c in &risk_summary.concerns {
                    output.push_str(&format!("- ⚠️ {}\n", c));
                }
            }
            if !risk_summary.positives.is_empty() {
                for p in &risk_summary.positives {
                    output.push_str(&format!("- ✅ {}\n", p));
                }
            }

            output.push_str(&format!(
                "- **Token:** {} ({})\n",
                analytics.token.symbol, analytics.token.name
            ));
            output.push_str(&format!(
                "- **Address:** `{}`\n",
                analytics.token.contract_address
            ));
            output.push_str(&format!("- **Price:** ${:.6}\n", analytics.price_usd));
            output.push_str(&format!(
                "- **Liquidity (24h):** ${}\n",
                crate::display::format_usd(analytics.liquidity_usd)
            ));
            output.push_str(&format!(
                "- **Volume (24h):** ${}\n",
                crate::display::format_usd(analytics.volume_24h)
            ));

            // Top holder context
            if let Some(top) = analytics.holders.first() {
                output.push_str(&format!(
                    "- **Top holder:** `{}` ({:.1}%)\n",
                    top.address, top.percentage
                ));
                if top.percentage > 30.0 {
                    output.push_str("  - ⚠️ High concentration risk\n");
                }
            }
            output.push_str(&format!(
                "- **Holders displayed:** {}\n",
                analytics.holders.len()
            ));

            // Stablecoin: auto-include market/peg
            let mut peg_healthy: Option<bool> = None;
            if is_stablecoin(&analytics.token.symbol) {
                let pair = format!("{}USDT", analytics.token.symbol);
                if let Ok(book) = BinanceClient::default_url()
                    .fetch_order_book(&pair)
                    .await
                {
                    let thresholds = HealthThresholds {
                        peg_target: 1.0,
                        peg_range: 0.001,
                        min_levels: 6,
                        min_depth: 3000.0,
                        min_bid_ask_ratio: 0.2,
                        max_bid_ask_ratio: 5.0,
                    };
                    let volume_24h = BinanceClient::default_url()
                        .fetch_24h_volume(&pair)
                        .await
                        .ok()
                        .flatten();
                    let summary = MarketSummary::from_order_book(
                        &book,
                        1.0,
                        &thresholds,
                        volume_24h,
                    );
                    let deviation_bps = summary
                        .mid_price
                        .map(|m| (m - 1.0) * 10_000.0)
                        .unwrap_or(0.0);
                    peg_healthy = Some(deviation_bps.abs() < 10.0);
                    let peg_status = if peg_healthy.unwrap_or(false) {
                        "✅ Peg healthy"
                    } else if deviation_bps.abs() < 50.0 {
                        "🟡 Slight peg deviation"
                    } else {
                        "⚠️ Peg deviation"
                    };
                    output.push_str(&format!(
                        "- **Market (Binance {}):** {} (deviation: {:.1} bps)\n",
                        pair, peg_status, deviation_bps
                    ));
                }
            }

            // Meta analysis
            let top_holder_pct = analytics.holders.first().map(|h| h.percentage);
            let meta = meta_analysis_token(
                &risk_summary,
                is_stablecoin(&analytics.token.symbol),
                peg_healthy,
                top_holder_pct,
                analytics.liquidity_usd,
            );
            output.push_str("\n### Synthesis\n\n");
            output.push_str(&format!("{}\n\n", meta.synthesis));
            output.push_str(&format!("**Key takeaway:** {}\n\n", meta.key_takeaway));
            if !meta.recommendations.is_empty() {
                output.push_str("**Consider:**\n");
                for rec in &meta.recommendations {
                    output.push_str(&format!("- {}\n", rec));
                }
            }
            output.push_str("\n---\n\n");
            output.push_str(&report::generate_report(&analytics));
        }
    }

    sp.finish("Insights complete.");
    println!("{}", output);
    Ok(())
}

fn target_type_label(target: &InferredTarget) -> &'static str {
    match target {
        InferredTarget::Address { .. } => "Address",
        InferredTarget::Transaction { .. } => "Transaction",
        InferredTarget::Token { .. } => "Token",
    }
}

fn chain_label(target: &InferredTarget) -> &str {
    match target {
        InferredTarget::Address { chain } => chain,
        InferredTarget::Transaction { chain } => chain,
        InferredTarget::Token { chain } => chain,
    }
}

/// Classifies EVM transaction from input data selector.
fn classify_tx_type(input: &str, to: Option<&str>) -> &'static str {
    if to.is_none() {
        return "Contract Creation";
    }
    let selector = input.trim_start_matches("0x").chars().take(8).collect::<String>();
    let sel = selector.to_lowercase();
    match sel.as_str() {
        "a9059cbb" => "ERC-20 Transfer",
        "095ea7b3" => "ERC-20 Approve",
        "23b872dd" => "ERC-20 Transfer From",
        "38ed1739" | "5c11d795" | "4a25d94a" | "8803dbee" | "7ff36ab5" | "18cbafe5"
        | "fb3bdb41" | "b6f9de95" => "DEX Swap",
        "ac9650d8" | "5ae401dc" => "Multicall",
        _ if input.is_empty() || input == "0x" => "Native Transfer",
        _ => "Contract Call",
    }
}

/// Formats raw value to human-readable (e.g. wei → ETH).
fn format_tx_value(value_str: &str, chain: &str) -> (String, bool) {
    let wei: u128 = if value_str.starts_with("0x") {
        let hex_part = value_str.trim_start_matches("0x");
        if hex_part.is_empty() {
            0
        } else {
            u128::from_str_radix(hex_part, 16).unwrap_or(0)
        }
    } else {
        value_str.parse().unwrap_or(0)
    };
    let decimals = match chain.to_lowercase().as_str() {
        "ethereum" | "polygon" | "arbitrum" | "optimism" | "base" | "bsc" | "aegis" => 18,
        "solana" => 9,
        "tron" => 6,
        _ => 18,
    };
    let divisor = 10_f64.powi(decimals as i32);
    let human = wei as f64 / divisor;
    let symbol = native_symbol(chain);
    let formatted = format!("≈ {:.6} {}", human, symbol);
    // "High value" threshold: > 10 native units
    let high_value = human > 10.0;
    (formatted, high_value)
}

/// Common stablecoin symbols for auto-including market/peg analysis.
fn is_stablecoin(symbol: &str) -> bool {
    matches!(
        symbol.to_uppercase().as_str(),
        "USDC" | "USDT" | "DAI" | "BUSD" | "TUSD" | "USDP" | "FRAX" | "LUSD" | "PUSD" | "GUSD"
    )
}

/// Meta-analysis: synthesizes observations into an executive summary, key takeaway, and recommendations.
struct MetaAnalysis {
    synthesis: String,
    key_takeaway: String,
    recommendations: Vec<String>,
}

fn meta_analysis_address(
    is_contract: bool,
    usd_value: Option<f64>,
    token_count: usize,
    risk_score: Option<f32>,
    risk_level: Option<&crate::compliance::risk::RiskLevel>,
) -> MetaAnalysis {
    let mut synthesis_parts = Vec::new();
    let profile = if is_contract {
        "contract"
    } else {
        "wallet (EOA)"
    };
    synthesis_parts.push(format!("A {} on chain.", profile));

    if let Some(usd) = usd_value {
        if usd > 1_000_000.0 {
            synthesis_parts.push("Significant value held.".to_string());
        } else if usd > 10_000.0 {
            synthesis_parts.push("Moderate value.".to_string());
        } else if usd < 1.0 {
            synthesis_parts.push("Minimal value.".to_string());
        }
    }

    if token_count > 5 {
        synthesis_parts.push("Diversified token exposure.".to_string());
    } else if token_count == 1 && token_count > 0 {
        synthesis_parts.push("Concentrated in a single token.".to_string());
    }

    if let (Some(score), Some(level)) = (risk_score, risk_level) {
        if score >= 7.0 {
            synthesis_parts.push(format!("Elevated risk ({:?}).", level));
        } else if score <= 3.0 {
            synthesis_parts.push("Low risk profile.".to_string());
        }
    }

    let synthesis = if synthesis_parts.is_empty() {
        "Address analyzed with available on-chain data.".to_string()
    } else {
        synthesis_parts.join(" ")
    };

    let key_takeaway = if let (Some(score), Some(level)) = (risk_score, risk_level) {
        if score >= 7.0 {
            format!("Risk assessment warrants closer scrutiny ({:.1}/10).", score)
        } else {
            format!(
                "Overall risk: {:?} ({:.1}/10).",
                level, score
            )
        }
    } else if is_contract {
        "Contract address — verify intended interaction before use.".to_string()
    } else if usd_value.map(|u| u > 100_000.0).unwrap_or(false) {
        "High-value wallet — standard due diligence applies.".to_string()
    } else {
        "Review full report for transaction and token details.".to_string()
    };

    let mut recommendations = Vec::new();
    if risk_score.map(|s| s >= 6.0).unwrap_or(false) {
        recommendations.push("Monitor for unusual transaction patterns.".to_string());
    }
    if token_count > 0 {
        recommendations.push("Verify token contracts before large interactions.".to_string());
    }
    if is_contract {
        recommendations.push("Confirm contract source and audit status.".to_string());
    }

    MetaAnalysis {
        synthesis,
        key_takeaway,
        recommendations,
    }
}

fn meta_analysis_tx(
    tx_type: &str,
    status: bool,
    high_value: bool,
    _from: &str,
    _to: Option<&str>,
) -> MetaAnalysis {
    let mut synthesis_parts = Vec::new();

    if !status {
        synthesis_parts.push("Transaction failed.".to_string());
    }

    synthesis_parts.push(format!("{} between parties.", tx_type));

    if high_value {
        synthesis_parts.push("High-value transfer.".to_string());
    }

    let synthesis = synthesis_parts.join(" ");

    let key_takeaway = if !status {
        "Failed transaction — check revert reason and contract state.".to_string()
    } else if high_value && tx_type == "Native Transfer" {
        "Large native transfer — verify recipient and intent.".to_string()
    } else if high_value {
        "High-value operation — standard verification recommended.".to_string()
    } else {
        format!("Routine {} — review full details if needed.", tx_type)
    };

    let mut recommendations = Vec::new();
    if !status {
        recommendations.push("Inspect contract logs for revert reason.".to_string());
    }
    if high_value {
        recommendations.push("Confirm recipient address and amount.".to_string());
    }
    if tx_type.contains("Approval") {
        recommendations.push("Verify approved spender and allowance amount.".to_string());
    }

    MetaAnalysis {
        synthesis,
        key_takeaway,
        recommendations,
    }
}

fn meta_analysis_token(
    risk_summary: &report::TokenRiskSummary,
    is_stablecoin: bool,
    peg_healthy: Option<bool>,
    top_holder_pct: Option<f64>,
    liquidity_usd: f64,
) -> MetaAnalysis {
    let mut synthesis_parts = Vec::new();

    if risk_summary.score <= 3 {
        synthesis_parts.push("Low-risk token with healthy metrics.".to_string());
    } else if risk_summary.score >= 7 {
        synthesis_parts.push("Elevated risk — multiple concerns identified.".to_string());
    } else {
        synthesis_parts.push("Moderate risk — mixed signals.".to_string());
    }

    if is_stablecoin {
        if let Some(healthy) = peg_healthy {
            if healthy {
                synthesis_parts.push("Stablecoin peg is healthy on observed venue.".to_string());
            } else {
                synthesis_parts.push("Stablecoin peg deviation detected — verify on multiple venues.".to_string());
            }
        }
    }

    if top_holder_pct.map(|p| p > 30.0).unwrap_or(false) {
        synthesis_parts.push("Concentration risk: top holder holds significant share.".to_string());
    }

    if liquidity_usd > 1_000_000.0 {
        synthesis_parts.push("Strong liquidity depth.".to_string());
    } else if liquidity_usd < 50_000.0 {
        synthesis_parts.push("Limited liquidity — slippage risk for larger trades.".to_string());
    }

    let synthesis = synthesis_parts.join(" ");

    let key_takeaway = if risk_summary.score >= 7 {
        format!(
            "High risk ({}): {} — exercise caution.",
            risk_summary.score,
            risk_summary.concerns.first().cloned().unwrap_or_else(|| "multiple factors".to_string())
        )
    } else if is_stablecoin && peg_healthy == Some(false) {
        "Stablecoin deviating from peg — check additional venues before trading.".to_string()
    } else if !risk_summary.positives.is_empty() && risk_summary.concerns.is_empty() {
        "Favorable risk profile — standard diligence applies.".to_string()
    } else {
        format!(
            "Risk {}/10 ({}) — weigh concerns against use case.",
            risk_summary.score, risk_summary.level
        )
    };

    let mut recommendations = Vec::new();
    if risk_summary.score >= 6 {
        recommendations.push("Consider smaller position sizes or avoid until risk clears.".to_string());
    }
    if top_holder_pct.map(|p| p > 25.0).unwrap_or(false) {
        recommendations.push("Monitor top holder movements for distribution changes.".to_string());
    }
    if is_stablecoin && peg_healthy != Some(true) {
        recommendations.push("Verify peg across multiple DEX/CEX venues.".to_string());
    }
    if liquidity_usd < 100_000.0 && risk_summary.score <= 5 {
        recommendations.push("Use limit orders or split trades to manage slippage.".to_string());
    }

    MetaAnalysis {
        synthesis,
        key_takeaway,
        recommendations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_target_evm_address() {
        let t = infer_target(
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            None,
        );
        assert!(matches!(t, InferredTarget::Address { chain } if chain == "ethereum"));
    }

    #[test]
    fn test_infer_target_tron_address() {
        let t = infer_target("TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf", None);
        assert!(matches!(t, InferredTarget::Address { chain } if chain == "tron"));
    }

    #[test]
    fn test_infer_target_solana_address() {
        let t = infer_target(
            "DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy",
            None,
        );
        assert!(matches!(t, InferredTarget::Address { chain } if chain == "solana"));
    }

    #[test]
    fn test_infer_target_evm_tx_hash() {
        let t = infer_target(
            "0xabc123def456789012345678901234567890123456789012345678901234abcd",
            None,
        );
        assert!(matches!(t, InferredTarget::Transaction { chain } if chain == "ethereum"));
    }

    #[test]
    fn test_infer_target_tron_tx_hash() {
        let t = infer_target(
            "abc123def456789012345678901234567890123456789012345678901234abcd",
            None,
        );
        assert!(matches!(t, InferredTarget::Transaction { chain } if chain == "tron"));
    }

    #[test]
    fn test_infer_target_token_symbol() {
        let t = infer_target("USDC", None);
        assert!(matches!(t, InferredTarget::Token { chain } if chain == "ethereum"));
    }

    #[test]
    fn test_infer_target_chain_override() {
        let t = infer_target("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", Some("polygon"));
        assert!(matches!(t, InferredTarget::Address { chain } if chain == "polygon"));
    }

    #[test]
    fn test_infer_target_token_with_chain_override() {
        let t = infer_target("USDC", Some("solana"));
        assert!(matches!(t, InferredTarget::Token { chain } if chain == "solana"));
    }

    #[test]
    fn test_classify_tx_type() {
        assert_eq!(
            classify_tx_type("0xa9059cbb1234...", Some("0xto")),
            "ERC-20 Transfer"
        );
        assert_eq!(
            classify_tx_type("0x095ea7b3abcd...", Some("0xto")),
            "ERC-20 Approve"
        );
        assert_eq!(classify_tx_type("0x", Some("0xto")), "Native Transfer");
        assert_eq!(classify_tx_type("", None), "Contract Creation");
    }

    #[test]
    fn test_format_tx_value() {
        let (fmt, high) = format_tx_value("0xDE0B6B3A7640000", "ethereum"); // 1 ETH
        assert!(fmt.contains("1.0") && fmt.contains("ETH"));
        assert!(!high);
        let (_, high2) = format_tx_value("0x52B7D2DCC80CD2E4000000", "ethereum"); // 100 ETH
        assert!(high2);
    }

    #[test]
    fn test_is_stablecoin() {
        assert!(is_stablecoin("USDC"));
        assert!(is_stablecoin("usdt"));
        assert!(is_stablecoin("DAI"));
        assert!(!is_stablecoin("ETH"));
        assert!(!is_stablecoin("PEPE"));
    }
}
