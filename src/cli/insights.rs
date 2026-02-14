//! # Insights Command
//!
//! Infers the type of blockchain target (address, token, transaction) from input,
//! auto-detects chain, and runs relevant Scope analyses to produce unified insights.

use crate::chains::{
    ChainClientFactory, infer_chain_from_address, infer_chain_from_hash, native_symbol,
};
use crate::cli::address::{self, AddressArgs};
use crate::cli::crawl::{Period, fetch_analytics_for_input};
use crate::cli::tx::{fetch_transaction_report, format_tx_markdown};
use crate::config::Config;
use crate::display::report;
use crate::error::Result;
use crate::market::{HealthThresholds, MarketSummary, VenueRegistry};
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
    mut args: InsightsArgs,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    // Resolve address book label → address + chain
    if let Some((address, chain)) =
        crate::cli::address_book::resolve_address_book_input(&args.target, config)?
    {
        args.target = address;
        if args.chain.is_none() {
            args.chain = Some(chain);
        }
    }

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
            let is_contract = code_result
                .as_ref()
                .is_ok_and(|c| !c.is_empty() && c != "0x");
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
            if let Some(ref tokens) = report.tokens
                && !tokens.is_empty()
            {
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
            let tx_report =
                fetch_transaction_report(&args.target, chain, args.decode, args.trace, clients)
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
            output.push_str(&format!("- **From:** `{}`\n", tx_report.transaction.from));
            output.push_str(&format!(
                "- **To:** `{}`\n",
                tx_report
                    .transaction
                    .to
                    .as_deref()
                    .unwrap_or("Contract Creation")
            ));

            let (formatted_value, high_value) =
                format_tx_value(&tx_report.transaction.value, chain);
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
                Some(&sp),
            )
            .await?;

            // Token risk summary (interpretive bullets)
            let risk_summary = report::token_risk_summary(&analytics);
            output.push_str(&format!(
                "- **Risk:** {} {}/10 ({})\n",
                risk_summary.emoji, risk_summary.score, risk_summary.level
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

            // Stablecoin: auto-include market/peg via venue registry
            let mut peg_healthy: Option<bool> = None;
            if is_stablecoin(&analytics.token.symbol)
                && let Ok(registry) = VenueRegistry::load()
            {
                // Try binance first, fall back to any available CEX
                let venue_id = if registry.contains("binance") {
                    "binance"
                } else {
                    registry.list().first().copied().unwrap_or("binance")
                };
                if let Ok(exchange) = registry.create_exchange_client(venue_id) {
                    let pair = exchange.format_pair(&analytics.token.symbol);
                    if let Ok(book) = exchange.fetch_order_book(&pair).await {
                        let thresholds = HealthThresholds {
                            peg_target: 1.0,
                            peg_range: 0.001,
                            min_levels: 6,
                            min_depth: 3000.0,
                            min_bid_ask_ratio: 0.2,
                            max_bid_ask_ratio: 5.0,
                        };
                        let volume_24h = if exchange.has_ticker() {
                            exchange
                                .fetch_ticker(&pair)
                                .await
                                .ok()
                                .and_then(|t| t.quote_volume_24h.or(t.volume_24h))
                        } else {
                            None
                        };
                        let summary =
                            MarketSummary::from_order_book(&book, 1.0, &thresholds, volume_24h);
                        let deviation_bps = summary
                            .mid_price
                            .map(|m| (m - 1.0) * 10_000.0)
                            .unwrap_or(0.0);
                        peg_healthy = Some(deviation_bps.abs() < 10.0);
                        let peg_status = if peg_healthy.unwrap_or(false) {
                            "Peg healthy"
                        } else if deviation_bps.abs() < 50.0 {
                            "Slight peg deviation"
                        } else {
                            "Peg deviation"
                        };
                        output.push_str(&format!(
                            "- **Market ({} {}):** {} (deviation: {:.1} bps)\n",
                            exchange.venue_name(),
                            pair,
                            peg_status,
                            deviation_bps
                        ));
                    }
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
    let selector = input
        .trim_start_matches("0x")
        .chars()
        .take(8)
        .collect::<String>();
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
    let divisor = 10_f64.powi(decimals);
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
            format!(
                "Risk assessment warrants closer scrutiny ({:.1}/10).",
                score
            )
        } else {
            format!("Overall risk: {:?} ({:.1}/10).", level, score)
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

    if is_stablecoin && let Some(healthy) = peg_healthy {
        if healthy {
            synthesis_parts.push("Stablecoin peg is healthy on observed venue.".to_string());
        } else {
            synthesis_parts
                .push("Stablecoin peg deviation detected — verify on multiple venues.".to_string());
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
            risk_summary
                .concerns
                .first()
                .cloned()
                .unwrap_or_else(|| "multiple factors".to_string())
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
        recommendations
            .push("Consider smaller position sizes or avoid until risk clears.".to_string());
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
    use crate::chains::{
        Balance as ChainBalance, ChainClient, ChainClientFactory, DexDataSource,
        Token as ChainToken, TokenBalance as ChainTokenBalance, Transaction as ChainTransaction,
    };
    use async_trait::async_trait;

    // ====================================================================
    // Mock Chain Client for testing run() paths
    // ====================================================================

    struct MockChainClient;

    #[async_trait]
    impl ChainClient for MockChainClient {
        fn chain_name(&self) -> &str {
            "ethereum"
        }
        fn native_token_symbol(&self) -> &str {
            "ETH"
        }
        async fn get_balance(&self, _address: &str) -> crate::error::Result<ChainBalance> {
            Ok(ChainBalance {
                raw: "1000000000000000000".to_string(),
                formatted: "1.0 ETH".to_string(),
                decimals: 18,
                symbol: "ETH".to_string(),
                usd_value: Some(2500.0),
            })
        }
        async fn enrich_balance_usd(&self, balance: &mut ChainBalance) {
            balance.usd_value = Some(2500.0);
        }
        async fn get_transaction(&self, _hash: &str) -> crate::error::Result<ChainTransaction> {
            Ok(ChainTransaction {
                hash: "0xabc123def456789012345678901234567890123456789012345678901234abcd"
                    .to_string(),
                block_number: Some(12345678),
                timestamp: Some(1700000000),
                from: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
                to: Some("0xdAC17F958D2ee523a2206206994597C13D831ec7".to_string()),
                value: "1000000000000000000".to_string(),
                gas_limit: 21000,
                gas_used: Some(21000),
                gas_price: "20000000000".to_string(),
                nonce: 42,
                input: "0xa9059cbb0000000000000000000000001234".to_string(),
                status: Some(true),
            })
        }
        async fn get_transactions(
            &self,
            _address: &str,
            _limit: u32,
        ) -> crate::error::Result<Vec<ChainTransaction>> {
            Ok(vec![])
        }
        async fn get_block_number(&self) -> crate::error::Result<u64> {
            Ok(12345678)
        }
        async fn get_token_balances(
            &self,
            _address: &str,
        ) -> crate::error::Result<Vec<ChainTokenBalance>> {
            Ok(vec![
                ChainTokenBalance {
                    token: ChainToken {
                        contract_address: "0xdAC17F958D2ee523a2206206994597C13D831ec7".to_string(),
                        symbol: "USDT".to_string(),
                        name: "Tether USD".to_string(),
                        decimals: 6,
                    },
                    balance: "1000000".to_string(),
                    formatted_balance: "1.0".to_string(),
                    usd_value: Some(1.0),
                },
                ChainTokenBalance {
                    token: ChainToken {
                        contract_address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
                        symbol: "USDC".to_string(),
                        name: "USD Coin".to_string(),
                        decimals: 6,
                    },
                    balance: "5000000".to_string(),
                    formatted_balance: "5.0".to_string(),
                    usd_value: Some(5.0),
                },
            ])
        }
        async fn get_code(&self, _address: &str) -> crate::error::Result<String> {
            Ok("0x".to_string()) // EOA
        }
    }

    struct MockFactory;

    impl ChainClientFactory for MockFactory {
        fn create_chain_client(&self, _chain: &str) -> crate::error::Result<Box<dyn ChainClient>> {
            Ok(Box::new(MockChainClient))
        }
        fn create_dex_client(&self) -> Box<dyn DexDataSource> {
            crate::chains::DefaultClientFactory {
                chains_config: Default::default(),
            }
            .create_dex_client()
        }
    }

    // Mock that returns a contract address
    struct MockContractClient;

    #[async_trait]
    impl ChainClient for MockContractClient {
        fn chain_name(&self) -> &str {
            "ethereum"
        }
        fn native_token_symbol(&self) -> &str {
            "ETH"
        }
        async fn get_balance(&self, _address: &str) -> crate::error::Result<ChainBalance> {
            Ok(ChainBalance {
                raw: "0".to_string(),
                formatted: "0.0 ETH".to_string(),
                decimals: 18,
                symbol: "ETH".to_string(),
                usd_value: Some(0.0),
            })
        }
        async fn enrich_balance_usd(&self, _balance: &mut ChainBalance) {}
        async fn get_transaction(&self, hash: &str) -> crate::error::Result<ChainTransaction> {
            Ok(ChainTransaction {
                hash: hash.to_string(),
                block_number: Some(100),
                timestamp: Some(1700000000),
                from: "0xfrom".to_string(),
                to: None, // contract creation
                value: "0".to_string(),
                gas_limit: 100000,
                gas_used: Some(80000),
                gas_price: "10000000000".to_string(),
                nonce: 0,
                input: "0x60806040".to_string(),
                status: Some(false), // failed tx
            })
        }
        async fn get_transactions(
            &self,
            _address: &str,
            _limit: u32,
        ) -> crate::error::Result<Vec<ChainTransaction>> {
            Ok(vec![])
        }
        async fn get_block_number(&self) -> crate::error::Result<u64> {
            Ok(100)
        }
        async fn get_token_balances(
            &self,
            _address: &str,
        ) -> crate::error::Result<Vec<ChainTokenBalance>> {
            Ok(vec![])
        }
        async fn get_code(&self, _address: &str) -> crate::error::Result<String> {
            Ok("0x6080604052".to_string()) // contract
        }
    }

    struct MockContractFactory;

    impl ChainClientFactory for MockContractFactory {
        fn create_chain_client(&self, _chain: &str) -> crate::error::Result<Box<dyn ChainClient>> {
            Ok(Box::new(MockContractClient))
        }
        fn create_dex_client(&self) -> Box<dyn DexDataSource> {
            crate::chains::DefaultClientFactory {
                chains_config: Default::default(),
            }
            .create_dex_client()
        }
    }

    // Mock DexDataSource for token tests
    struct MockDexDataSource;

    #[async_trait]
    impl DexDataSource for MockDexDataSource {
        async fn get_token_price(&self, _chain: &str, _address: &str) -> Option<f64> {
            Some(1.0)
        }

        async fn get_native_token_price(&self, _chain: &str) -> Option<f64> {
            Some(2500.0)
        }

        async fn get_token_data(
            &self,
            _chain: &str,
            address: &str,
        ) -> crate::error::Result<crate::chains::dex::DexTokenData> {
            use crate::chains::{DexPair, PricePoint, VolumePoint};
            Ok(crate::chains::dex::DexTokenData {
                address: address.to_string(),
                symbol: "TEST".to_string(),
                name: "Test Token".to_string(),
                price_usd: 1.5,
                price_change_24h: 5.2,
                price_change_6h: 2.1,
                price_change_1h: 0.5,
                price_change_5m: 0.1,
                volume_24h: 1_000_000.0,
                volume_6h: 250_000.0,
                volume_1h: 50_000.0,
                liquidity_usd: 500_000.0,
                market_cap: Some(10_000_000.0),
                fdv: Some(12_000_000.0),
                pairs: vec![DexPair {
                    dex_name: "Uniswap V3".to_string(),
                    pair_address: "0xpair123".to_string(),
                    base_token: "TEST".to_string(),
                    quote_token: "USDC".to_string(),
                    price_usd: 1.5,
                    liquidity_usd: 500_000.0,
                    volume_24h: 1_000_000.0,
                    price_change_24h: 5.2,
                    buys_24h: 100,
                    sells_24h: 80,
                    buys_6h: 20,
                    sells_6h: 15,
                    buys_1h: 5,
                    sells_1h: 3,
                    pair_created_at: Some(1690000000),
                    url: Some("https://dexscreener.com/ethereum/0xpair123".to_string()),
                }],
                price_history: vec![PricePoint {
                    timestamp: 1690000000,
                    price: 1.5,
                }],
                volume_history: vec![VolumePoint {
                    timestamp: 1690000000,
                    volume: 1_000_000.0,
                }],
                total_buys_24h: 100,
                total_sells_24h: 80,
                total_buys_6h: 20,
                total_sells_6h: 15,
                total_buys_1h: 5,
                total_sells_1h: 3,
                earliest_pair_created_at: Some(1690000000),
                image_url: None,
                websites: Vec::new(),
                socials: Vec::new(),
                dexscreener_url: Some("https://dexscreener.com/ethereum/test".to_string()),
            })
        }

        async fn search_tokens(
            &self,
            _query: &str,
            _chain: Option<&str>,
        ) -> crate::error::Result<Vec<crate::chains::TokenSearchResult>> {
            Ok(vec![crate::chains::TokenSearchResult {
                address: "0xTEST1234567890123456789012345678901234567".to_string(),
                symbol: "TEST".to_string(),
                name: "Test Token".to_string(),
                chain: "ethereum".to_string(),
                price_usd: Some(1.5),
                volume_24h: 1_000_000.0,
                liquidity_usd: 500_000.0,
                market_cap: Some(10_000_000.0),
            }])
        }
    }

    // Mock ChainClient that returns holders with high concentration
    struct MockTokenChainClient;

    #[async_trait]
    impl ChainClient for MockTokenChainClient {
        fn chain_name(&self) -> &str {
            "ethereum"
        }
        fn native_token_symbol(&self) -> &str {
            "ETH"
        }
        async fn get_balance(&self, _address: &str) -> crate::error::Result<ChainBalance> {
            Ok(ChainBalance {
                raw: "1000000000000000000".to_string(),
                formatted: "1.0 ETH".to_string(),
                decimals: 18,
                symbol: "ETH".to_string(),
                usd_value: Some(2500.0),
            })
        }
        async fn enrich_balance_usd(&self, balance: &mut ChainBalance) {
            balance.usd_value = Some(2500.0);
        }
        async fn get_transaction(&self, _hash: &str) -> crate::error::Result<ChainTransaction> {
            Ok(ChainTransaction {
                hash: "0xabc123".to_string(),
                block_number: Some(12345678),
                timestamp: Some(1700000000),
                from: "0xfrom".to_string(),
                to: Some("0xto".to_string()),
                value: "0".to_string(),
                gas_limit: 21000,
                gas_used: Some(21000),
                gas_price: "20000000000".to_string(),
                nonce: 42,
                input: "0x".to_string(),
                status: Some(true),
            })
        }
        async fn get_transactions(
            &self,
            _address: &str,
            _limit: u32,
        ) -> crate::error::Result<Vec<ChainTransaction>> {
            Ok(vec![])
        }
        async fn get_block_number(&self) -> crate::error::Result<u64> {
            Ok(12345678)
        }
        async fn get_token_balances(
            &self,
            _address: &str,
        ) -> crate::error::Result<Vec<ChainTokenBalance>> {
            Ok(vec![])
        }
        async fn get_code(&self, _address: &str) -> crate::error::Result<String> {
            Ok("0x".to_string())
        }
        async fn get_token_holders(
            &self,
            _address: &str,
            _limit: u32,
        ) -> crate::error::Result<Vec<crate::chains::TokenHolder>> {
            // Return holders with high concentration (>30%) to trigger warning
            Ok(vec![
                crate::chains::TokenHolder {
                    address: "0x1111111111111111111111111111111111111111".to_string(),
                    balance: "3500000000000000000000000".to_string(),
                    formatted_balance: "3500000.0".to_string(),
                    percentage: 35.0, // >30% triggers concentration warning
                    rank: 1,
                },
                crate::chains::TokenHolder {
                    address: "0x2222222222222222222222222222222222222222".to_string(),
                    balance: "1500000000000000000000000".to_string(),
                    formatted_balance: "1500000.0".to_string(),
                    percentage: 15.0,
                    rank: 2,
                },
                crate::chains::TokenHolder {
                    address: "0x3333333333333333333333333333333333333333".to_string(),
                    balance: "1000000000000000000000000".to_string(),
                    formatted_balance: "1000000.0".to_string(),
                    percentage: 10.0,
                    rank: 3,
                },
            ])
        }
    }

    // Factory for token tests with mocks
    struct MockTokenFactory;

    impl ChainClientFactory for MockTokenFactory {
        fn create_chain_client(&self, _chain: &str) -> crate::error::Result<Box<dyn ChainClient>> {
            Ok(Box::new(MockTokenChainClient))
        }
        fn create_dex_client(&self) -> Box<dyn DexDataSource> {
            Box::new(MockDexDataSource)
        }
    }

    // ====================================================================
    // run() function tests with mocks
    // ====================================================================

    #[tokio::test]
    async fn test_run_address_eoa() {
        let config = Config::default();
        let factory = MockFactory;
        let args = InsightsArgs {
            target: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: None,
            decode: false,
            trace: false,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_address_contract() {
        let config = Config::default();
        let factory = MockContractFactory;
        let args = InsightsArgs {
            target: "0xdAC17F958D2ee523a2206206994597C13D831ec7".to_string(),
            chain: None,
            decode: false,
            trace: false,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_transaction() {
        let config = Config::default();
        let factory = MockFactory;
        let args = InsightsArgs {
            target: "0xabc123def456789012345678901234567890123456789012345678901234abcd"
                .to_string(),
            chain: None,
            decode: false,
            trace: false,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_transaction_failed() {
        let config = Config::default();
        let factory = MockContractFactory;
        let args = InsightsArgs {
            target: "0xabc123def456789012345678901234567890123456789012345678901234abcd"
                .to_string(),
            chain: Some("ethereum".to_string()),
            decode: true,
            trace: false,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_address_with_chain_override() {
        let config = Config::default();
        let factory = MockFactory;
        let args = InsightsArgs {
            target: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: Some("polygon".to_string()),
            decode: false,
            trace: false,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_insights_run_token() {
        let config = Config::default();
        let factory = MockTokenFactory;
        let args = InsightsArgs {
            target: "TEST".to_string(),
            chain: Some("ethereum".to_string()),
            decode: false,
            trace: false,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_insights_run_token_with_concentration_warning() {
        let config = Config::default();
        let factory = MockTokenFactory;
        let args = InsightsArgs {
            target: "0xTEST1234567890123456789012345678901234567".to_string(),
            chain: Some("ethereum".to_string()),
            decode: false,
            trace: false,
        };
        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    // ====================================================================
    // Existing tests below
    // ====================================================================

    #[test]
    fn test_infer_target_evm_address() {
        let t = infer_target("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", None);
        assert!(matches!(t, InferredTarget::Address { chain } if chain == "ethereum"));
    }

    #[test]
    fn test_infer_target_tron_address() {
        let t = infer_target("TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf", None);
        assert!(matches!(t, InferredTarget::Address { chain } if chain == "tron"));
    }

    #[test]
    fn test_infer_target_solana_address() {
        let t = infer_target("DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy", None);
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
        let t = infer_target(
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            Some("polygon"),
        );
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
        assert!(is_stablecoin("BUSD"));
        assert!(is_stablecoin("TUSD"));
        assert!(is_stablecoin("USDP"));
        assert!(is_stablecoin("FRAX"));
        assert!(is_stablecoin("LUSD"));
        assert!(is_stablecoin("PUSD"));
        assert!(is_stablecoin("GUSD"));
        assert!(!is_stablecoin("ETH"));
        assert!(!is_stablecoin("PEPE"));
        assert!(!is_stablecoin("WBTC"));
    }

    #[test]
    fn test_is_stablecoin_empty_string() {
        assert!(!is_stablecoin(""));
    }

    #[test]
    fn test_is_stablecoin_case_insensitive() {
        // to_uppercase() makes comparison case-insensitive
        assert!(is_stablecoin("UsDc"));
        assert!(is_stablecoin("FraX"));
        assert!(!is_stablecoin("SOL")); // SOL is not a stablecoin
    }

    // ====================================================================
    // target_type_label and chain_label tests
    // ====================================================================

    #[test]
    fn test_target_type_label_address() {
        let t = InferredTarget::Address {
            chain: "ethereum".to_string(),
        };
        assert_eq!(target_type_label(&t), "Address");
    }

    #[test]
    fn test_target_type_label_transaction() {
        let t = InferredTarget::Transaction {
            chain: "ethereum".to_string(),
        };
        assert_eq!(target_type_label(&t), "Transaction");
    }

    #[test]
    fn test_target_type_label_token() {
        let t = InferredTarget::Token {
            chain: "ethereum".to_string(),
        };
        assert_eq!(target_type_label(&t), "Token");
    }

    #[test]
    fn test_chain_label_address() {
        let t = InferredTarget::Address {
            chain: "polygon".to_string(),
        };
        assert_eq!(chain_label(&t), "polygon");
    }

    #[test]
    fn test_chain_label_transaction() {
        let t = InferredTarget::Transaction {
            chain: "tron".to_string(),
        };
        assert_eq!(chain_label(&t), "tron");
    }

    #[test]
    fn test_chain_label_token() {
        let t = InferredTarget::Token {
            chain: "solana".to_string(),
        };
        assert_eq!(chain_label(&t), "solana");
    }

    // ====================================================================
    // classify_tx_type — expanded edge cases
    // ====================================================================

    #[test]
    fn test_classify_tx_type_dex_swaps() {
        assert_eq!(
            classify_tx_type("0x38ed173900000...", Some("0xrouter")),
            "DEX Swap"
        );
        assert_eq!(
            classify_tx_type("0x5c11d79500000...", Some("0xrouter")),
            "DEX Swap"
        );
        assert_eq!(
            classify_tx_type("0x4a25d94a00000...", Some("0xrouter")),
            "DEX Swap"
        );
        assert_eq!(
            classify_tx_type("0x8803dbee00000...", Some("0xrouter")),
            "DEX Swap"
        );
        assert_eq!(
            classify_tx_type("0x7ff36ab500000...", Some("0xrouter")),
            "DEX Swap"
        );
        assert_eq!(
            classify_tx_type("0x18cbafe500000...", Some("0xrouter")),
            "DEX Swap"
        );
        assert_eq!(
            classify_tx_type("0xfb3bdb4100000...", Some("0xrouter")),
            "DEX Swap"
        );
        assert_eq!(
            classify_tx_type("0xb6f9de9500000...", Some("0xrouter")),
            "DEX Swap"
        );
    }

    #[test]
    fn test_classify_tx_type_multicall() {
        assert_eq!(
            classify_tx_type("0xac9650d800000...", Some("0xcontract")),
            "Multicall"
        );
        assert_eq!(
            classify_tx_type("0x5ae401dc00000...", Some("0xcontract")),
            "Multicall"
        );
    }

    #[test]
    fn test_classify_tx_type_transfer_from() {
        assert_eq!(
            classify_tx_type("0x23b872dd00000...", Some("0xtoken")),
            "ERC-20 Transfer From"
        );
    }

    #[test]
    fn test_classify_tx_type_contract_call() {
        assert_eq!(
            classify_tx_type("0xdeadbeef00000...", Some("0xcontract")),
            "Contract Call"
        );
    }

    #[test]
    fn test_classify_tx_type_native_transfer_empty() {
        assert_eq!(classify_tx_type("", Some("0xrecipient")), "Native Transfer");
    }

    // ====================================================================
    // format_tx_value — expanded edge cases
    // ====================================================================

    #[test]
    fn test_format_tx_value_zero() {
        let (fmt, high) = format_tx_value("0x0", "ethereum");
        assert!(fmt.contains("0.000000"));
        assert!(fmt.contains("ETH"));
        assert!(!high);
    }

    #[test]
    fn test_format_tx_value_empty_hex() {
        let (fmt, high) = format_tx_value("0x", "ethereum");
        assert!(fmt.contains("0.000000"));
        assert!(!high);
    }

    #[test]
    fn test_format_tx_value_decimal_string() {
        let (fmt, high) = format_tx_value("1000000000000000000", "ethereum"); // 1 ETH
        assert!(fmt.contains("1.0"));
        assert!(fmt.contains("ETH"));
        assert!(!high);
    }

    #[test]
    fn test_format_tx_value_solana() {
        let (fmt, high) = format_tx_value("1000000000", "solana"); // 1 SOL (9 decimals)
        assert!(fmt.contains("1.0"));
        assert!(fmt.contains("SOL"));
        assert!(!high);
    }

    #[test]
    fn test_format_tx_value_tron() {
        let (fmt, high) = format_tx_value("1000000", "tron"); // 1 TRX (6 decimals)
        assert!(fmt.contains("1.0"));
        assert!(fmt.contains("TRX"));
        assert!(!high);
    }

    #[test]
    fn test_format_tx_value_polygon() {
        let (fmt, _) = format_tx_value("1000000000000000000", "polygon");
        assert!(fmt.contains("MATIC") || fmt.contains("POL"));
    }

    #[test]
    fn test_format_tx_value_bsc() {
        let (fmt, _) = format_tx_value("1000000000000000000", "bsc");
        assert!(fmt.contains("BNB"));
    }

    #[test]
    fn test_format_tx_value_high_value_threshold() {
        // > 10 native units = high value
        let (_, high) = format_tx_value("11000000000000000000", "ethereum"); // 11 ETH
        assert!(high);
        let (_, high2) = format_tx_value("10000000000000000000", "ethereum"); // 10 ETH
        assert!(!high2); // exactly 10 is not > 10
    }

    // ====================================================================
    // meta_analysis_address tests
    // ====================================================================

    #[test]
    fn test_meta_analysis_address_contract_high_value() {
        let meta = meta_analysis_address(true, Some(2_000_000.0), 10, None, None);
        assert!(meta.synthesis.contains("contract"));
        assert!(meta.synthesis.contains("Significant value"));
        assert!(meta.synthesis.contains("Diversified"));
        assert!(meta.recommendations.iter().any(|r| r.contains("contract")));
    }

    #[test]
    fn test_meta_analysis_address_eoa_moderate_value() {
        let meta = meta_analysis_address(false, Some(50_000.0), 3, None, None);
        assert!(meta.synthesis.contains("wallet (EOA)"));
        assert!(meta.synthesis.contains("Moderate value"));
    }

    #[test]
    fn test_meta_analysis_address_minimal_value() {
        let meta = meta_analysis_address(false, Some(0.5), 0, None, None);
        assert!(meta.synthesis.contains("Minimal value"));
    }

    #[test]
    fn test_meta_analysis_address_single_token() {
        let meta = meta_analysis_address(false, None, 1, None, None);
        assert!(meta.synthesis.contains("Concentrated in a single token"));
    }

    #[test]
    fn test_meta_analysis_address_high_risk() {
        use crate::compliance::risk::RiskLevel;
        let level = RiskLevel::High;
        let meta = meta_analysis_address(false, None, 0, Some(8.5), Some(&level));
        assert!(meta.synthesis.contains("Elevated risk"));
        assert!(meta.key_takeaway.contains("scrutiny"));
        assert!(
            meta.recommendations
                .iter()
                .any(|r| r.contains("unusual transaction"))
        );
    }

    #[test]
    fn test_meta_analysis_address_low_risk() {
        use crate::compliance::risk::RiskLevel;
        let level = RiskLevel::Low;
        let meta = meta_analysis_address(false, None, 0, Some(2.0), Some(&level));
        assert!(meta.synthesis.contains("Low risk"));
    }

    #[test]
    fn test_meta_analysis_address_contract_no_value() {
        let meta = meta_analysis_address(true, None, 0, None, None);
        assert!(meta.key_takeaway.contains("Contract address"));
        assert!(
            meta.recommendations
                .iter()
                .any(|r| r.contains("Confirm contract"))
        );
    }

    #[test]
    fn test_meta_analysis_address_high_value_wallet() {
        let meta = meta_analysis_address(false, Some(150_000.0), 0, None, None);
        assert!(meta.key_takeaway.contains("High-value wallet"));
    }

    #[test]
    fn test_meta_analysis_address_default_takeaway() {
        let meta = meta_analysis_address(false, Some(5_000.0), 0, None, None);
        assert!(meta.key_takeaway.contains("Review full report"));
    }

    #[test]
    fn test_meta_analysis_address_with_tokens_recommendation() {
        let meta = meta_analysis_address(false, None, 3, None, None);
        assert!(
            meta.recommendations
                .iter()
                .any(|r| r.contains("Verify token contracts"))
        );
    }

    // ====================================================================
    // meta_analysis_tx tests
    // ====================================================================

    #[test]
    fn test_meta_analysis_tx_successful_native_transfer() {
        let meta = meta_analysis_tx("Native Transfer", true, false, "0xfrom", Some("0xto"));
        assert!(meta.synthesis.contains("Native Transfer"));
        assert!(meta.key_takeaway.contains("Routine"));
        assert!(meta.recommendations.is_empty());
    }

    #[test]
    fn test_meta_analysis_tx_failed() {
        let meta = meta_analysis_tx("Contract Call", false, false, "0xfrom", Some("0xto"));
        assert!(meta.synthesis.contains("failed"));
        assert!(meta.key_takeaway.contains("Failed transaction"));
        assert!(meta.recommendations.iter().any(|r| r.contains("revert")));
    }

    #[test]
    fn test_meta_analysis_tx_high_value_native() {
        let meta = meta_analysis_tx("Native Transfer", true, true, "0xfrom", Some("0xto"));
        assert!(meta.synthesis.contains("High-value"));
        assert!(meta.key_takeaway.contains("Large native transfer"));
        assert!(meta.recommendations.iter().any(|r| r.contains("recipient")));
    }

    #[test]
    fn test_meta_analysis_tx_high_value_contract_call() {
        let meta = meta_analysis_tx("DEX Swap", true, true, "0xfrom", Some("0xto"));
        assert!(meta.key_takeaway.contains("High-value operation"));
    }

    #[test]
    fn test_meta_analysis_tx_erc20_approve() {
        let meta = meta_analysis_tx("ERC-20 Approval", true, false, "0xfrom", Some("0xto"));
        assert!(meta.recommendations.iter().any(|r| r.contains("spender")));
    }

    #[test]
    fn test_meta_analysis_tx_failed_high_value() {
        let meta = meta_analysis_tx("Contract Call", false, true, "0xfrom", Some("0xto"));
        assert!(meta.synthesis.contains("failed"));
        assert!(meta.synthesis.contains("High-value"));
        assert!(meta.recommendations.len() >= 2);
    }

    // ====================================================================
    // meta_analysis_token tests
    // ====================================================================

    #[test]
    fn test_meta_analysis_token_low_risk() {
        let summary = report::TokenRiskSummary {
            score: 2,
            level: "Low",
            emoji: "🟢",
            concerns: vec![],
            positives: vec!["Good liquidity".to_string()],
        };
        let meta = meta_analysis_token(&summary, false, None, None, 2_000_000.0);
        assert!(meta.synthesis.contains("Low-risk"));
        assert!(meta.synthesis.contains("Strong liquidity"));
        assert!(meta.key_takeaway.contains("Favorable"));
    }

    #[test]
    fn test_meta_analysis_token_high_risk() {
        let summary = report::TokenRiskSummary {
            score: 8,
            level: "High",
            emoji: "🔴",
            concerns: vec!["Low liquidity".to_string()],
            positives: vec![],
        };
        let meta = meta_analysis_token(&summary, false, None, None, 10_000.0);
        assert!(meta.synthesis.contains("Elevated risk"));
        assert!(meta.synthesis.contains("Limited liquidity"));
        assert!(meta.key_takeaway.contains("High risk"));
        assert!(
            meta.recommendations
                .iter()
                .any(|r| r.contains("smaller position"))
        );
    }

    #[test]
    fn test_meta_analysis_token_moderate_risk() {
        let summary = report::TokenRiskSummary {
            score: 5,
            level: "Medium",
            emoji: "🟡",
            concerns: vec!["Some concern".to_string()],
            positives: vec!["Some positive".to_string()],
        };
        let meta = meta_analysis_token(&summary, false, None, None, 500_000.0);
        assert!(meta.synthesis.contains("Moderate risk"));
        assert!(meta.key_takeaway.contains("Risk 5/10"));
    }

    #[test]
    fn test_meta_analysis_token_stablecoin_healthy_peg() {
        let summary = report::TokenRiskSummary {
            score: 2,
            level: "Low",
            emoji: "🟢",
            concerns: vec![],
            positives: vec!["Stable peg".to_string()],
        };
        let meta = meta_analysis_token(&summary, true, Some(true), None, 5_000_000.0);
        assert!(meta.synthesis.contains("Stablecoin peg is healthy"));
    }

    #[test]
    fn test_meta_analysis_token_stablecoin_unhealthy_peg() {
        let summary = report::TokenRiskSummary {
            score: 4,
            level: "Medium",
            emoji: "🟡",
            concerns: vec![],
            positives: vec![],
        };
        let meta = meta_analysis_token(&summary, true, Some(false), None, 500_000.0);
        assert!(meta.synthesis.contains("peg deviation"));
        assert!(meta.key_takeaway.contains("deviating from peg"));
        assert!(meta.recommendations.iter().any(|r| r.contains("peg")));
    }

    #[test]
    fn test_meta_analysis_token_concentration_risk() {
        let summary = report::TokenRiskSummary {
            score: 5,
            level: "Medium",
            emoji: "🟡",
            concerns: vec![],
            positives: vec![],
        };
        let meta = meta_analysis_token(&summary, false, None, Some(45.0), 500_000.0);
        assert!(meta.synthesis.contains("Concentration risk"));
        assert!(
            meta.recommendations
                .iter()
                .any(|r| r.contains("top holder"))
        );
    }

    #[test]
    fn test_meta_analysis_token_low_liquidity_low_risk() {
        let summary = report::TokenRiskSummary {
            score: 3,
            level: "Low",
            emoji: "🟢",
            concerns: vec![],
            positives: vec![],
        };
        let meta = meta_analysis_token(&summary, false, None, None, 50_000.0);
        assert!(
            meta.recommendations
                .iter()
                .any(|r| r.contains("limit orders") || r.contains("slippage"))
        );
    }

    #[test]
    fn test_meta_analysis_token_stablecoin_no_peg_data() {
        let summary = report::TokenRiskSummary {
            score: 3,
            level: "Low",
            emoji: "🟢",
            concerns: vec![],
            positives: vec![],
        };
        let meta = meta_analysis_token(&summary, true, None, None, 1_000_000.0);
        // When peg_healthy is None, recommendation should still suggest verifying peg
        assert!(meta.recommendations.iter().any(|r| r.contains("peg")));
    }

    // ====================================================================
    // infer_target — additional edge cases
    // ====================================================================

    #[test]
    fn test_infer_target_tx_hash_with_chain_override() {
        let t = infer_target(
            "0xabc123def456789012345678901234567890123456789012345678901234abcd",
            Some("polygon"),
        );
        assert!(matches!(t, InferredTarget::Transaction { chain } if chain == "polygon"));
    }

    #[test]
    fn test_infer_target_whitespace_trimming() {
        let t = infer_target("  USDC  ", None);
        assert!(matches!(t, InferredTarget::Token { .. }));
    }

    #[test]
    fn test_infer_target_long_token_name() {
        let t = infer_target("some-random-token-name", None);
        assert!(matches!(t, InferredTarget::Token { chain } if chain == "ethereum"));
    }

    // ====================================================================
    // InsightsArgs struct validation
    // ====================================================================

    #[test]
    fn test_insights_args_debug() {
        let args = InsightsArgs {
            target: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: Some("ethereum".to_string()),
            decode: true,
            trace: false,
        };
        let debug_str = format!("{:?}", args);
        assert!(debug_str.contains("InsightsArgs"));
        assert!(debug_str.contains("0x742d"));
    }

    // ====================================================================
    // Additional tests for classify_tx_type — selector matches and edge cases
    // ====================================================================

    #[test]
    fn test_classify_tx_type_contract_creation() {
        assert_eq!(classify_tx_type("0xa9059cbb...", None), "Contract Creation");
    }

    #[test]
    fn test_classify_tx_type_erc20_transfer() {
        assert_eq!(
            classify_tx_type("0xa9059cbb00000000", Some("0x1234")),
            "ERC-20 Transfer"
        );
    }

    #[test]
    fn test_classify_tx_type_erc20_approve() {
        assert_eq!(
            classify_tx_type("0x095ea7b3...", Some("0x1234")),
            "ERC-20 Approve"
        );
    }

    #[test]
    fn test_classify_tx_type_erc20_transfer_from() {
        assert_eq!(
            classify_tx_type("0x23b872dd...", Some("0x1234")),
            "ERC-20 Transfer From"
        );
    }

    #[test]
    fn test_classify_tx_type_dex_swap() {
        assert_eq!(
            classify_tx_type("0x38ed1739...", Some("0x1234")),
            "DEX Swap"
        );
        assert_eq!(
            classify_tx_type("0x7ff36ab5...", Some("0x1234")),
            "DEX Swap"
        );
    }

    #[test]
    fn test_classify_tx_type_native_transfer() {
        assert_eq!(classify_tx_type("0x", Some("0x1234")), "Native Transfer");
        assert_eq!(classify_tx_type("", Some("0x1234")), "Native Transfer");
    }

    #[test]
    fn test_classify_tx_type_unknown_contract_call() {
        assert_eq!(
            classify_tx_type("0xdeadbeef12345678", Some("0x1234")),
            "Contract Call"
        );
    }

    // ====================================================================
    // Additional tests for format_tx_value
    // ====================================================================

    #[test]
    fn test_format_tx_value_ethereum_wei() {
        let (fmt, high) = format_tx_value("1000000000000000000", "ethereum");
        assert!(fmt.contains("1.000000"));
        assert!(fmt.contains("ETH"));
        assert!(!high); // 1 ETH < 10 threshold
    }

    #[test]
    fn test_format_tx_value_hex() {
        let (fmt, _) = format_tx_value("0xde0b6b3a7640000", "ethereum");
        // 0xde0b6b3a7640000 = 10^18 = 1 ETH
        assert!(fmt.contains("ETH"));
    }

    #[test]
    fn test_format_tx_value_high_value() {
        // 100 ETH in wei = 100000000000000000000
        let (_, high) = format_tx_value("100000000000000000000", "ethereum");
        assert!(high); // 100 ETH > 10
    }

    #[test]
    fn test_format_tx_value_zero_decimal() {
        let (fmt, high) = format_tx_value("0", "ethereum");
        assert!(fmt.contains("0.000000"));
        assert!(!high);
    }

    #[test]
    fn test_format_tx_value_solana_additional() {
        let (fmt, _) = format_tx_value("1000000000", "solana"); // 1 SOL
        assert!(fmt.contains("SOL"));
    }

    #[test]
    fn test_format_tx_value_tron_additional() {
        let (fmt, _) = format_tx_value("1000000", "tron"); // 1 TRX
        assert!(fmt.contains("TRX"));
    }

    #[test]
    fn test_format_tx_value_empty_hex_additional() {
        let (fmt, _) = format_tx_value("0x", "ethereum");
        assert!(fmt.contains("0.000000"));
    }

    // ====================================================================
    // Combined tests for target_type_label and chain_label
    // ====================================================================

    #[test]
    fn test_target_type_label_combined() {
        assert_eq!(
            target_type_label(&InferredTarget::Address {
                chain: "eth".to_string()
            }),
            "Address"
        );
        assert_eq!(
            target_type_label(&InferredTarget::Transaction {
                chain: "eth".to_string()
            }),
            "Transaction"
        );
        assert_eq!(
            target_type_label(&InferredTarget::Token {
                chain: "eth".to_string()
            }),
            "Token"
        );
    }

    #[test]
    fn test_chain_label_combined() {
        assert_eq!(
            chain_label(&InferredTarget::Address {
                chain: "ethereum".to_string()
            }),
            "ethereum"
        );
        assert_eq!(
            chain_label(&InferredTarget::Transaction {
                chain: "polygon".to_string()
            }),
            "polygon"
        );
        assert_eq!(
            chain_label(&InferredTarget::Token {
                chain: "solana".to_string()
            }),
            "solana"
        );
    }

    // ====================================================================
    // Additional tests for meta_analysis_address
    // ====================================================================

    #[test]
    fn test_meta_analysis_address_contract_high_risk() {
        use crate::compliance::risk::RiskLevel;
        let meta = meta_analysis_address(
            true,
            Some(2_000_000.0),
            10,
            Some(8.0),
            Some(&RiskLevel::High),
        );
        assert!(meta.synthesis.contains("contract"));
        assert!(meta.synthesis.contains("Significant value"));
        assert!(meta.key_takeaway.contains("scrutiny"));
        assert!(!meta.recommendations.is_empty());
    }

    #[test]
    fn test_meta_analysis_address_wallet_low_risk() {
        use crate::compliance::risk::RiskLevel;
        let meta = meta_analysis_address(false, Some(0.5), 0, Some(2.0), Some(&RiskLevel::Low));
        assert!(meta.synthesis.contains("wallet"));
        assert!(meta.synthesis.contains("Minimal value"));
    }

    #[test]
    fn test_meta_analysis_address_no_risk_data() {
        let meta = meta_analysis_address(false, None, 0, None, None);
        assert!(!meta.synthesis.is_empty());
        assert!(meta.key_takeaway.contains("Review full report"));
    }

    // ====================================================================
    // Additional tests for meta_analysis_tx
    // ====================================================================

    #[test]
    fn test_meta_analysis_tx_failed_additional() {
        let meta = meta_analysis_tx("Contract Call", false, false, "0x...", Some("0x..."));
        assert!(meta.synthesis.contains("failed"));
        assert!(meta.key_takeaway.contains("Failed"));
    }

    #[test]
    fn test_meta_analysis_tx_high_value_native_additional() {
        let meta = meta_analysis_tx("Native Transfer", true, true, "0x...", Some("0x..."));
        assert!(meta.synthesis.contains("High-value"));
        assert!(meta.key_takeaway.contains("Large native transfer"));
    }

    #[test]
    fn test_meta_analysis_tx_routine() {
        let meta = meta_analysis_tx("ERC-20 Transfer", true, false, "0x...", Some("0x..."));
        assert!(meta.key_takeaway.contains("Routine"));
    }

    // ====================================================================
    // Additional tests for meta_analysis_token
    // ====================================================================

    #[test]
    fn test_meta_analysis_token_high_risk_additional() {
        let risk = report::TokenRiskSummary {
            score: 8,
            level: "High",
            emoji: "🔴",
            concerns: vec!["Low liquidity".to_string()],
            positives: vec![],
        };
        let meta = meta_analysis_token(&risk, false, None, None, 10_000.0);
        assert!(meta.synthesis.contains("Elevated risk"));
        assert!(meta.key_takeaway.contains("High risk"));
    }

    #[test]
    fn test_meta_analysis_token_stablecoin_peg_healthy() {
        let risk = report::TokenRiskSummary {
            score: 2,
            level: "Low",
            emoji: "🟢",
            concerns: vec![],
            positives: vec!["Strong liquidity".to_string()],
        };
        let meta = meta_analysis_token(&risk, true, Some(true), Some(5.0), 5_000_000.0);
        assert!(meta.synthesis.contains("peg is healthy"));
        assert!(meta.synthesis.contains("Strong liquidity"));
    }

    #[test]
    fn test_meta_analysis_token_stablecoin_peg_unhealthy() {
        let risk = report::TokenRiskSummary {
            score: 5,
            level: "Medium",
            emoji: "🟡",
            concerns: vec!["Peg deviation".to_string()],
            positives: vec![],
        };
        let meta = meta_analysis_token(&risk, true, Some(false), Some(40.0), 100_000.0);
        assert!(meta.synthesis.contains("peg deviation"));
        assert!(meta.synthesis.contains("Concentration risk"));
    }
}
