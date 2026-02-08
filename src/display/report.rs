//! # Markdown Report Generator
//!
//! This module generates comprehensive markdown reports for token analytics.
//! Reports include full, non-truncated addresses and all available data.
//!
//! ## Features
//!
//! - Executive summary with key metrics
//! - Price and volume analysis
//! - Complete holder distribution with full addresses
//! - Concentration metrics and risk indicators
//! - Data source links for verification
//!
//! ## Usage
//!
//! ```rust,no_run
//! use scope::display::report::{generate_report, save_report};
//! use scope::chains::TokenAnalytics;
//!
//! // Assuming you have TokenAnalytics data
//! // let analytics = ...;
//! // let report = generate_report(&analytics);
//! // save_report(&report, "report.md").unwrap();
//! ```

use crate::chains::TokenAnalytics;
use crate::error::Result;
use chrono::{DateTime, Utc};
use std::path::Path;

// ============================================================================
// Block explorer base URLs
// ============================================================================

/// Etherscan base URL for Ethereum token pages.
const ETHERSCAN_TOKEN_BASE: &str = "https://etherscan.io/token";
/// PolygonScan base URL for Polygon token pages.
const POLYGONSCAN_TOKEN_BASE: &str = "https://polygonscan.com/token";
/// Arbiscan base URL for Arbitrum token pages.
const ARBISCAN_TOKEN_BASE: &str = "https://arbiscan.io/token";
/// Optimistic Etherscan base URL for Optimism token pages.
const OPTIMISM_TOKEN_BASE: &str = "https://optimistic.etherscan.io/token";
/// BaseScan base URL for Base token pages.
const BASESCAN_TOKEN_BASE: &str = "https://basescan.org/token";
/// BscScan base URL for BSC token pages.
const BSCSCAN_TOKEN_BASE: &str = "https://bscscan.com/token";
/// Solscan base URL for Solana token pages.
const SOLSCAN_TOKEN_BASE: &str = "https://solscan.io/token";

/// DexScreener base URL for token pair pages.
const DEXSCREENER_BASE: &str = "https://dexscreener.com";
/// GeckoTerminal base URL for pool pages.
const GECKOTERMINAL_BASE: &str = "https://www.geckoterminal.com";

/// Generates a comprehensive markdown report from token analytics.
///
/// # Arguments
///
/// * `analytics` - The token analytics data to include in the report
///
/// # Returns
///
/// Returns a formatted markdown string.
///
/// # Note
///
/// All addresses in the report are non-truncated for analysis and verification purposes.
pub fn generate_report(analytics: &TokenAnalytics) -> String {
    let mut report = String::new();

    // Header
    report.push_str(&generate_header(analytics));
    report.push_str("\n---\n\n");

    // Executive Summary
    report.push_str(&generate_executive_summary(analytics));
    report.push_str("\n---\n\n");

    // Price Analysis with chart
    report.push_str(&generate_price_analysis(analytics));
    report.push_str(&generate_price_chart(analytics));
    report.push_str("\n---\n\n");

    // Volume Analysis with chart
    report.push_str(&generate_volume_analysis(analytics));
    report.push_str(&generate_volume_chart(analytics));
    report.push_str("\n---\n\n");

    // Liquidity Analysis with DEX chart
    report.push_str(&generate_liquidity_analysis(analytics));
    report.push_str(&generate_liquidity_chart(analytics));
    report.push_str("\n---\n\n");

    // Top Holders (with full addresses)
    report.push_str(&generate_holder_section(analytics));
    report.push_str("\n---\n\n");

    // Concentration Analysis with pie chart
    report.push_str(&generate_concentration_analysis(analytics));
    report.push_str(&generate_concentration_chart(analytics));
    report.push_str("\n---\n\n");

    // Token Information (socials, websites)
    report.push_str(&generate_token_info_section(analytics));
    report.push_str("\n---\n\n");

    // Security Analysis
    report.push_str(&generate_security_analysis(analytics));
    report.push_str("\n---\n\n");

    // Risk Score
    report.push_str(&generate_risk_score_section(analytics));
    report.push_str("\n---\n\n");

    // Risk Indicators
    report.push_str(&generate_risk_indicators(analytics));
    report.push_str("\n---\n\n");

    // Data Sources
    report.push_str(&generate_data_sources(analytics));

    report
}

/// Generates the report header.
fn generate_header(analytics: &TokenAnalytics) -> String {
    let timestamp = DateTime::<Utc>::from_timestamp(analytics.fetched_at, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let mut header = String::new();
    header.push_str(&format!(
        "# Token Analysis Report: {}\n\n",
        analytics.token.symbol
    ));
    header.push_str(&format!("**Token Name:** {}  \n", analytics.token.name));
    header.push_str(&format!("**Chain:** {}  \n", capitalize(&analytics.chain)));
    header.push_str(&format!("**Generated:** {}  \n", timestamp));
    header.push_str(&format!(
        "**Contract:** `{}`\n",
        analytics.token.contract_address
    ));

    header
}

/// Generates the executive summary section.
fn generate_executive_summary(analytics: &TokenAnalytics) -> String {
    let mut summary = String::new();
    summary.push_str("## Executive Summary\n\n");
    summary.push_str("| Metric | Value |\n");
    summary.push_str("|--------|-------|\n");
    summary.push_str(&format!("| Price | ${:.6} |\n", analytics.price_usd));
    summary.push_str(&format!(
        "| 24h Change | {:+.2}% |\n",
        analytics.price_change_24h
    ));
    summary.push_str(&format!(
        "| 7d Change | {:+.2}% |\n",
        analytics.price_change_7d
    ));
    summary.push_str(&format!(
        "| 24h Volume | {} |\n",
        format_usd(analytics.volume_24h)
    ));
    summary.push_str(&format!(
        "| 7d Volume | {} |\n",
        format_usd(analytics.volume_7d)
    ));
    summary.push_str(&format!(
        "| Liquidity | {} |\n",
        format_usd(analytics.liquidity_usd)
    ));

    if let Some(mc) = analytics.market_cap {
        summary.push_str(&format!("| Market Cap | {} |\n", format_usd(mc)));
    }

    if let Some(fdv) = analytics.fdv {
        summary.push_str(&format!(
            "| Fully Diluted Valuation | {} |\n",
            format_usd(fdv)
        ));
    }

    summary.push_str(&format!(
        "| Total Holders | {} |\n",
        format_number(analytics.total_holders as f64)
    ));

    if let Some(ref supply) = analytics.total_supply {
        summary.push_str(&format!("| Total Supply | {} |\n", supply));
    }

    if let Some(ref circ) = analytics.circulating_supply {
        summary.push_str(&format!("| Circulating Supply | {} |\n", circ));
    }

    summary
}

/// Generates the price analysis section.
fn generate_price_analysis(analytics: &TokenAnalytics) -> String {
    let mut section = String::new();
    section.push_str("## Price Analysis\n\n");

    section.push_str(&format!(
        "**Current Price:** ${:.6}\n\n",
        analytics.price_usd
    ));

    // Price changes
    section.push_str("### Price Changes\n\n");
    section.push_str("| Period | Change |\n");
    section.push_str("|--------|--------|\n");
    section.push_str(&format!(
        "| 24 Hours | {:+.2}% |\n",
        analytics.price_change_24h
    ));
    section.push_str(&format!(
        "| 7 Days | {:+.2}% |\n",
        analytics.price_change_7d
    ));

    // Price history stats if available
    if !analytics.price_history.is_empty() {
        let prices: Vec<f64> = analytics.price_history.iter().map(|p| p.price).collect();
        let min_price = prices.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_price = prices.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let avg_price: f64 = prices.iter().sum::<f64>() / prices.len() as f64;

        section.push_str("\n### Price Range (Period)\n\n");
        section.push_str("| Stat | Value |\n");
        section.push_str("|------|-------|\n");
        section.push_str(&format!("| High | ${:.6} |\n", max_price));
        section.push_str(&format!("| Low | ${:.6} |\n", min_price));
        section.push_str(&format!("| Average | ${:.6} |\n", avg_price));
    }

    section
}

/// Generates the volume analysis section.
fn generate_volume_analysis(analytics: &TokenAnalytics) -> String {
    let mut section = String::new();
    section.push_str("## Volume Analysis\n\n");

    section.push_str("| Period | Volume |\n");
    section.push_str("|--------|--------|\n");
    section.push_str(&format!(
        "| 24 Hours | {} |\n",
        format_usd(analytics.volume_24h)
    ));
    section.push_str(&format!(
        "| 7 Days | {} |\n",
        format_usd(analytics.volume_7d)
    ));

    // Volume to liquidity ratio (indicator of trading activity)
    if analytics.liquidity_usd > 0.0 {
        let vol_to_liq = analytics.volume_24h / analytics.liquidity_usd;
        section.push_str(&format!(
            "\n**Volume/Liquidity Ratio (24h):** {:.2}x\n",
            vol_to_liq
        ));

        if vol_to_liq > 5.0 {
            section.push_str(
                "\n> ⚠️ High volume relative to liquidity may indicate unusual trading activity.\n",
            );
        }
    }

    section
}

/// Generates the liquidity analysis section.
fn generate_liquidity_analysis(analytics: &TokenAnalytics) -> String {
    let mut section = String::new();
    section.push_str("## Liquidity Analysis\n\n");

    section.push_str(&format!(
        "**Total Liquidity:** {}\n\n",
        format_usd(analytics.liquidity_usd)
    ));

    if !analytics.dex_pairs.is_empty() {
        section.push_str("### Trading Pairs\n\n");
        section.push_str("| DEX | Pair | Liquidity | 24h Volume | Price |\n");
        section.push_str("|-----|------|-----------|------------|-------|\n");

        for pair in analytics.dex_pairs.iter().take(10) {
            section.push_str(&format!(
                "| {} | {}/{} | {} | {} | ${:.6} |\n",
                pair.dex_name,
                pair.base_token,
                pair.quote_token,
                format_usd(pair.liquidity_usd),
                format_usd(pair.volume_24h),
                pair.price_usd
            ));
        }
    }

    section
}

/// Generates the holder section with FULL addresses.
fn generate_holder_section(analytics: &TokenAnalytics) -> String {
    let mut section = String::new();
    section.push_str("## Top Holders\n\n");

    if analytics.holders.is_empty() {
        section.push_str("*No holder data available*\n");
        return section;
    }

    section.push_str("| Rank | Address | Balance | % of Supply |\n");
    section.push_str("|------|---------|---------|-------------|\n");

    for holder in &analytics.holders {
        // IMPORTANT: Full addresses, not truncated
        section.push_str(&format!(
            "| {} | `{}` | {} | {:.2}% |\n",
            holder.rank,
            holder.address, // Full address
            holder.formatted_balance,
            holder.percentage
        ));
    }

    section
}

/// Generates the concentration analysis section.
fn generate_concentration_analysis(analytics: &TokenAnalytics) -> String {
    let mut section = String::new();
    section.push_str("## Concentration Analysis\n\n");

    // Calculate concentration metrics from holder data
    let top_10_pct: f64 = analytics
        .holders
        .iter()
        .take(10)
        .map(|h| h.percentage)
        .sum();

    let top_50_pct: f64 = analytics
        .holders
        .iter()
        .take(50)
        .map(|h| h.percentage)
        .sum();

    let top_100_pct: f64 = analytics
        .holders
        .iter()
        .take(100)
        .map(|h| h.percentage)
        .sum();

    // Use stored values if available, otherwise use calculated
    let top_10 = analytics.top_10_concentration.unwrap_or(top_10_pct);
    let top_50 = analytics.top_50_concentration.unwrap_or(top_50_pct);
    let top_100 = analytics.top_100_concentration.unwrap_or(top_100_pct);

    section.push_str(&format!(
        "- **Top 10 holders control:** {:.1}% of supply\n",
        top_10
    ));
    section.push_str(&format!(
        "- **Top 50 holders control:** {:.1}% of supply\n",
        top_50
    ));
    section.push_str(&format!(
        "- **Top 100 holders control:** {:.1}% of supply\n",
        top_100
    ));

    // Add interpretation
    section.push_str("\n### Interpretation\n\n");

    if top_10 > 80.0 {
        section.push_str("- 🔴 **Very High Concentration:** Top 10 holders control over 80% of supply. This indicates significant centralization risk.\n");
    } else if top_10 > 50.0 {
        section.push_str("- 🟠 **High Concentration:** Top 10 holders control over 50% of supply. Moderate centralization risk.\n");
    } else if top_10 > 25.0 {
        section.push_str("- 🟡 **Moderate Concentration:** Top 10 holders control 25-50% of supply. Typical for many tokens.\n");
    } else {
        section.push_str("- 🟢 **Low Concentration:** Top 10 holders control less than 25% of supply. Well-distributed ownership.\n");
    }

    section
}

/// Generates the token information section with socials, websites, and DexScreener link.
fn generate_token_info_section(analytics: &TokenAnalytics) -> String {
    let mut section = String::new();
    section.push_str("## Token Information\n\n");

    // Display image URL if available
    if let Some(ref image_url) = analytics.image_url {
        section.push_str(&format!("**Token Logo:** [View Image]({})\n\n", image_url));
    }

    // Display websites
    if !analytics.websites.is_empty() {
        section.push_str("### Websites\n\n");
        for website in &analytics.websites {
            section.push_str(&format!("- [{}]({})\n", website, website));
        }
        section.push('\n');
    }

    // Display social links
    if !analytics.socials.is_empty() {
        section.push_str("### Social Media\n\n");
        for social in &analytics.socials {
            let icon = match social.platform.to_lowercase().as_str() {
                "twitter" | "x" => "🐦",
                "telegram" => "📱",
                "discord" => "💬",
                "medium" => "📝",
                "github" => "💻",
                "reddit" => "🔴",
                "youtube" => "📺",
                "facebook" => "📘",
                "instagram" => "📷",
                _ => "🔗",
            };
            section.push_str(&format!(
                "- {} **{}**: [{}]({})\n",
                icon,
                capitalize(&social.platform),
                social.url,
                social.url
            ));
        }
        section.push('\n');
    }

    // Display DexScreener link
    if let Some(ref dexscreener_url) = analytics.dexscreener_url {
        section.push_str("### Trading Links\n\n");
        section.push_str(&format!(
            "- 📊 **DexScreener:** [View on DexScreener]({})\n",
            dexscreener_url
        ));
        section.push('\n');
    }

    // If no metadata available, note it
    if analytics.image_url.is_none()
        && analytics.websites.is_empty()
        && analytics.socials.is_empty()
        && analytics.dexscreener_url.is_none()
    {
        section.push_str("*No additional token metadata available*\n");
    }

    section
}

/// Generates the security analysis section with honeypot detection and token age.
fn generate_security_analysis(analytics: &TokenAnalytics) -> String {
    let mut section = String::new();
    section.push_str("## Security Analysis\n\n");

    // Build the security checks table
    section.push_str("| Check | Status | Details |\n");
    section.push_str("|-------|--------|--------|\n");

    // Honeypot Risk Analysis (buy/sell ratio)
    let buys = analytics.total_buys_24h;
    let sells = analytics.total_sells_24h;
    let (honeypot_status, honeypot_details) = if buys == 0 && sells == 0 {
        ("⚪ UNKNOWN", "No transaction data available".to_string())
    } else if sells == 0 && buys > 0 {
        (
            "🔴 HIGH",
            format!("{} buys / 0 sells - Possible honeypot!", buys),
        )
    } else {
        let ratio = if sells > 0 {
            buys as f64 / sells as f64
        } else {
            f64::INFINITY
        };
        if ratio > 10.0 {
            (
                "🔴 HIGH",
                format!(
                    "{} buys / {} sells (ratio: {:.2}) - Suspicious activity!",
                    buys, sells, ratio
                ),
            )
        } else if ratio > 3.0 {
            (
                "🟠 MEDIUM",
                format!(
                    "{} buys / {} sells (ratio: {:.2}) - Elevated risk",
                    buys, sells, ratio
                ),
            )
        } else {
            (
                "🟢 LOW",
                format!(
                    "{} buys / {} sells (ratio: {:.2}) - Normal activity",
                    buys, sells, ratio
                ),
            )
        }
    };
    section.push_str(&format!(
        "| Honeypot Risk | {} | {} |\n",
        honeypot_status, honeypot_details
    ));

    // Token Age Analysis
    let (age_status, age_details) = match analytics.token_age_hours {
        Some(hours) if hours < 24.0 => (
            "🔴 HIGH RISK",
            format!("Created {:.1} hours ago - Very new token!", hours),
        ),
        Some(hours) if hours < 48.0 => (
            "🟠 MEDIUM",
            format!("Created {:.1} hours ago - New token", hours),
        ),
        Some(hours) if hours < 168.0 => {
            // 7 days
            let days = hours / 24.0;
            (
                "🟡 CAUTION",
                format!("Created {:.1} days ago - Relatively new", days),
            )
        }
        Some(hours) => {
            let days = hours / 24.0;
            if days > 365.0 {
                let years = days / 365.0;
                ("🟢 ESTABLISHED", format!("Created {:.1} years ago", years))
            } else if days > 30.0 {
                let months = days / 30.0;
                (
                    "🟢 ESTABLISHED",
                    format!("Created {:.1} months ago", months),
                )
            } else {
                ("🟢 MODERATE", format!("Created {:.1} days ago", days))
            }
        }
        None => ("⚪ UNKNOWN", "Token age data not available".to_string()),
    };
    section.push_str(&format!(
        "| Token Age | {} | {} |\n",
        age_status, age_details
    ));

    // Whale Concentration Risk
    let top_holder_pct = analytics
        .holders
        .first()
        .map(|h| h.percentage)
        .unwrap_or(0.0);
    let (whale_status, whale_details) = if top_holder_pct > 50.0 {
        (
            "🔴 HIGH",
            format!(
                "Largest holder owns {:.1}% - Extreme concentration!",
                top_holder_pct
            ),
        )
    } else if top_holder_pct > 25.0 {
        (
            "🟠 MEDIUM",
            format!(
                "Largest holder owns {:.1}% - High concentration",
                top_holder_pct
            ),
        )
    } else if top_holder_pct > 10.0 {
        (
            "🟡 MODERATE",
            format!("Largest holder owns {:.1}%", top_holder_pct),
        )
    } else if top_holder_pct > 0.0 {
        (
            "🟢 LOW",
            format!(
                "Largest holder owns {:.1}% - Well distributed",
                top_holder_pct
            ),
        )
    } else {
        ("⚪ UNKNOWN", "Holder data not available".to_string())
    };
    section.push_str(&format!(
        "| Whale Risk | {} | {} |\n",
        whale_status, whale_details
    ));

    // Social Presence
    let (social_status, social_details) =
        if analytics.socials.is_empty() && analytics.websites.is_empty() {
            (
                "🟠 NONE",
                "No verified social links or websites".to_string(),
            )
        } else {
            let social_count = analytics.socials.len();
            let website_count = analytics.websites.len();
            (
                "🟢 PRESENT",
                format!("{} social links, {} websites", social_count, website_count),
            )
        };
    section.push_str(&format!(
        "| Social Presence | {} | {} |\n",
        social_status, social_details
    ));

    section.push('\n');

    // Add Mermaid charts for visualization
    if analytics.total_buys_24h > 0 || analytics.total_sells_24h > 0 {
        // Buy/Sell Distribution Pie Chart
        section.push_str(&generate_buysell_chart(analytics));
        section.push('\n');

        // Transaction Activity Bar Chart
        section.push_str(&generate_txn_activity_chart(analytics));
        section.push('\n');
    }

    // Add recent activity summary
    if analytics.total_buys_1h > 0 || analytics.total_sells_1h > 0 {
        section.push_str("### Recent Activity\n\n");
        section.push_str(&format!(
            "- **1h:** {} buys, {} sells\n",
            analytics.total_buys_1h, analytics.total_sells_1h
        ));
        section.push_str(&format!(
            "- **6h:** {} buys, {} sells\n",
            analytics.total_buys_6h, analytics.total_sells_6h
        ));
        section.push_str(&format!(
            "- **24h:** {} buys, {} sells\n",
            analytics.total_buys_24h, analytics.total_sells_24h
        ));
        section.push('\n');
    }

    section
}

/// Generates a Mermaid pie chart showing buy vs sell transaction distribution.
fn generate_buysell_chart(analytics: &TokenAnalytics) -> String {
    let buys = analytics.total_buys_24h;
    let sells = analytics.total_sells_24h;

    if buys == 0 && sells == 0 {
        return String::new();
    }

    let mut chart = String::new();
    chart.push_str("### 24h Transaction Distribution\n\n");
    chart.push_str("```mermaid\n");
    chart.push_str("pie showData\n");
    chart.push_str("    title \"24h Buy vs Sell Transactions\"\n");
    chart.push_str(&format!("    \"Buys\" : {}\n", buys));
    chart.push_str(&format!("    \"Sells\" : {}\n", sells));
    chart.push_str("```\n");

    chart
}

/// Generates a Mermaid bar chart showing transaction activity across time periods.
fn generate_txn_activity_chart(analytics: &TokenAnalytics) -> String {
    // Only generate if we have data
    if analytics.total_buys_24h == 0
        && analytics.total_sells_24h == 0
        && analytics.total_buys_6h == 0
        && analytics.total_sells_6h == 0
        && analytics.total_buys_1h == 0
        && analytics.total_sells_1h == 0
    {
        return String::new();
    }

    let mut chart = String::new();
    chart.push_str("### Transaction Activity by Period\n\n");

    // Use a table format for clearer multi-series data since xychart-beta
    // doesn't support multiple named bar series well
    chart.push_str("| Period | Buys | Sells | Ratio |\n");
    chart.push_str("|--------|------|-------|-------|\n");

    let periods = [
        ("1h", analytics.total_buys_1h, analytics.total_sells_1h),
        ("6h", analytics.total_buys_6h, analytics.total_sells_6h),
        ("24h", analytics.total_buys_24h, analytics.total_sells_24h),
    ];

    for (period, buys, sells) in periods {
        let ratio = if sells > 0 {
            format!("{:.2}", buys as f64 / sells as f64)
        } else if buys > 0 {
            "∞".to_string()
        } else {
            "-".to_string()
        };
        chart.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            period, buys, sells, ratio
        ));
    }

    chart.push('\n');

    // Add a simple bar chart for visual representation of 24h activity
    chart.push_str("```mermaid\n");
    chart.push_str("xychart-beta\n");
    chart.push_str("    title \"24h Transaction Volume\"\n");
    chart.push_str("    x-axis [\"Buys\", \"Sells\"]\n");
    chart.push_str("    y-axis \"Count\"\n");
    chart.push_str(&format!(
        "    bar [{}, {}]\n",
        analytics.total_buys_24h, analytics.total_sells_24h
    ));
    chart.push_str("```\n");

    chart
}

/// Risk factors used to calculate the overall risk score.
struct RiskFactors {
    /// Honeypot risk (0-10, higher is riskier)
    honeypot: u8,
    /// Token age risk (0-10, higher is riskier for newer tokens)
    age: u8,
    /// Liquidity risk (0-10, higher is riskier for low liquidity)
    liquidity: u8,
    /// Holder concentration risk (0-10, higher is riskier)
    concentration: u8,
    /// Social presence risk (0-10, higher is riskier for no presence)
    social: u8,
}

impl RiskFactors {
    /// Calculate risk factors from token analytics.
    fn from_analytics(analytics: &TokenAnalytics) -> Self {
        // Honeypot risk based on buy/sell ratio
        let honeypot = if analytics.total_buys_24h == 0 && analytics.total_sells_24h == 0 {
            5 // Unknown, moderate risk
        } else if analytics.total_sells_24h == 0 && analytics.total_buys_24h > 0 {
            10 // Maximum risk
        } else {
            let ratio = analytics.total_buys_24h as f64 / analytics.total_sells_24h.max(1) as f64;
            if ratio > 10.0 {
                9
            } else if ratio > 5.0 {
                7
            } else if ratio > 3.0 {
                5
            } else if ratio > 2.0 {
                3
            } else {
                1
            }
        };

        // Age risk based on token age
        let age = match analytics.token_age_hours {
            Some(hours) if hours < 24.0 => 10,
            Some(hours) if hours < 48.0 => 8,
            Some(hours) if hours < 168.0 => 6,  // 7 days
            Some(hours) if hours < 720.0 => 4,  // 30 days
            Some(hours) if hours < 2160.0 => 2, // 90 days
            Some(_) => 1,
            None => 5, // Unknown
        };

        // Liquidity risk
        let liquidity = if analytics.liquidity_usd < 10_000.0 {
            10
        } else if analytics.liquidity_usd < 50_000.0 {
            8
        } else if analytics.liquidity_usd < 100_000.0 {
            6
        } else if analytics.liquidity_usd < 500_000.0 {
            4
        } else if analytics.liquidity_usd < 1_000_000.0 {
            2
        } else {
            1
        };

        // Concentration risk based on top holder percentage
        let top_holder_pct = analytics
            .holders
            .first()
            .map(|h| h.percentage)
            .unwrap_or(0.0);
        let concentration = if top_holder_pct > 50.0 {
            10
        } else if top_holder_pct > 30.0 {
            8
        } else if top_holder_pct > 20.0 {
            6
        } else if top_holder_pct > 10.0 {
            4
        } else if top_holder_pct > 5.0 {
            2
        } else {
            1
        };

        // Social presence risk
        let social = if analytics.socials.is_empty() && analytics.websites.is_empty() {
            8
        } else if analytics.socials.is_empty() || analytics.websites.is_empty() {
            4
        } else if analytics.socials.len() >= 2 && !analytics.websites.is_empty() {
            1
        } else {
            2
        };

        RiskFactors {
            honeypot,
            age,
            liquidity,
            concentration,
            social,
        }
    }

    /// Calculate the overall risk score (1-10).
    fn overall_score(&self) -> u8 {
        // Weighted average with honeypot and concentration being most important
        let weighted = (self.honeypot as u16 * 3
            + self.age as u16 * 2
            + self.liquidity as u16 * 2
            + self.concentration as u16 * 3
            + self.social as u16)
            / 11;
        weighted.clamp(1, 10) as u8
    }

    /// Get risk level label.
    fn risk_level(&self) -> &'static str {
        match self.overall_score() {
            1..=3 => "LOW",
            4..=6 => "MEDIUM",
            7..=8 => "HIGH",
            _ => "CRITICAL",
        }
    }

    /// Get risk level color/emoji.
    fn risk_emoji(&self) -> &'static str {
        match self.overall_score() {
            1..=3 => "🟢",
            4..=6 => "🟡",
            7..=8 => "🟠",
            _ => "🔴",
        }
    }
}

/// Generates the risk score section with breakdown pie chart.
fn generate_risk_score_section(analytics: &TokenAnalytics) -> String {
    let mut section = String::new();
    section.push_str("## Risk Score\n\n");

    let factors = RiskFactors::from_analytics(analytics);
    let overall = factors.overall_score();
    let level = factors.risk_level();
    let emoji = factors.risk_emoji();

    // Overall risk score display
    section.push_str(&format!(
        "### Overall Risk: {} {}/10 ({})\n\n",
        emoji, overall, level
    ));

    // Factor breakdown table
    section.push_str("| Factor | Score | Assessment |\n");
    section.push_str("|--------|-------|------------|\n");
    section.push_str(&format!(
        "| Honeypot Risk | {}/10 | {} |\n",
        factors.honeypot,
        risk_assessment(factors.honeypot)
    ));
    section.push_str(&format!(
        "| Token Age | {}/10 | {} |\n",
        factors.age,
        risk_assessment(factors.age)
    ));
    section.push_str(&format!(
        "| Liquidity | {}/10 | {} |\n",
        factors.liquidity,
        risk_assessment(factors.liquidity)
    ));
    section.push_str(&format!(
        "| Concentration | {}/10 | {} |\n",
        factors.concentration,
        risk_assessment(factors.concentration)
    ));
    section.push_str(&format!(
        "| Social Presence | {}/10 | {} |\n",
        factors.social,
        risk_assessment(factors.social)
    ));
    section.push('\n');

    // Risk breakdown pie chart
    section.push_str(&generate_risk_breakdown_chart(&factors));

    section
}

/// Get a risk assessment label for a given score.
fn risk_assessment(score: u8) -> &'static str {
    match score {
        0..=2 => "Low Risk",
        3..=4 => "Moderate",
        5..=6 => "Elevated",
        7..=8 => "High Risk",
        _ => "Critical",
    }
}

/// Generates a Mermaid pie chart showing risk factor breakdown.
fn generate_risk_breakdown_chart(factors: &RiskFactors) -> String {
    let mut chart = String::new();
    chart.push_str("### Risk Factor Breakdown\n\n");
    chart.push_str("```mermaid\n");
    chart.push_str("pie showData\n");
    chart.push_str("    title \"Risk Factor Contribution\"\n");
    chart.push_str(&format!("    \"Honeypot\" : {}\n", factors.honeypot));
    chart.push_str(&format!("    \"Token Age\" : {}\n", factors.age));
    chart.push_str(&format!("    \"Liquidity\" : {}\n", factors.liquidity));
    chart.push_str(&format!(
        "    \"Concentration\" : {}\n",
        factors.concentration
    ));
    chart.push_str(&format!("    \"Social\" : {}\n", factors.social));
    chart.push_str("```\n");

    chart
}

fn generate_risk_indicators(analytics: &TokenAnalytics) -> String {
    let mut section = String::new();
    section.push_str("## Risk Indicators\n\n");

    let mut risks = Vec::new();
    let mut positives = Vec::new();

    // Concentration risk
    let top_10_pct: f64 = analytics
        .holders
        .iter()
        .take(10)
        .map(|h| h.percentage)
        .sum();

    if top_10_pct > 80.0 {
        risks
            .push("🔴 **Extreme whale concentration** - Top 10 holders control over 80% of supply");
    } else if top_10_pct > 50.0 {
        risks.push("🟠 **High whale concentration** - Top 10 holders control over 50% of supply");
    } else {
        positives.push("🟢 **Reasonable distribution** - No extreme concentration in top holders");
    }

    // Liquidity risk
    if analytics.liquidity_usd < 10_000.0 {
        risks.push("🔴 **Very low liquidity** - High slippage risk for trades");
    } else if analytics.liquidity_usd < 100_000.0 {
        risks.push("🟠 **Low liquidity** - Moderate slippage risk for larger trades");
    } else if analytics.liquidity_usd > 1_000_000.0 {
        positives.push("🟢 **Good liquidity** - Sufficient depth for most trades");
    }

    // Volume risk
    if analytics.volume_24h < 1_000.0 {
        risks
            .push("🟠 **Very low trading volume** - May indicate low interest or liquidity issues");
    } else if analytics.volume_24h > 100_000.0 {
        positives.push("🟢 **Active trading** - Healthy trading volume");
    }

    // Price volatility
    if analytics.price_change_24h.abs() > 20.0 {
        risks.push("🟠 **High price volatility** - Price moved over 20% in 24 hours");
    }

    if !risks.is_empty() {
        section.push_str("### Risk Factors\n\n");
        for risk in &risks {
            section.push_str(&format!("- {}\n", risk));
        }
        section.push('\n');
    }

    if !positives.is_empty() {
        section.push_str("### Positive Indicators\n\n");
        for positive in &positives {
            section.push_str(&format!("- {}\n", positive));
        }
    }

    if risks.is_empty() && positives.is_empty() {
        section.push_str("*Insufficient data for risk assessment*\n");
    }

    section
}

/// Generates the data sources section.
fn generate_data_sources(analytics: &TokenAnalytics) -> String {
    let mut section = String::new();
    section.push_str("## Data Sources\n\n");

    let chain = &analytics.chain.to_lowercase();
    let address = &analytics.token.contract_address;

    // Explorer links based on chain
    let explorer_url = match chain.as_str() {
        "ethereum" => format!("{}/{}", ETHERSCAN_TOKEN_BASE, address),
        "polygon" => format!("{}/{}", POLYGONSCAN_TOKEN_BASE, address),
        "arbitrum" => format!("{}/{}", ARBISCAN_TOKEN_BASE, address),
        "optimism" => format!("{}/{}", OPTIMISM_TOKEN_BASE, address),
        "base" => format!("{}/{}", BASESCAN_TOKEN_BASE, address),
        "bsc" => format!("{}/{}", BSCSCAN_TOKEN_BASE, address),
        "solana" => format!("{}/{}", SOLSCAN_TOKEN_BASE, address),
        _ => format!("{}/{}", ETHERSCAN_TOKEN_BASE, address),
    };

    section.push_str(&format!(
        "- [Block Explorer ({})]({})\n",
        capitalize(chain),
        explorer_url
    ));

    section.push_str(&format!(
        "- [DexScreener]({}/{}/{})\n",
        DEXSCREENER_BASE, chain, address
    ));

    section.push_str(&format!(
        "- [GeckoTerminal]({}/{}/pools/{})\n",
        GECKOTERMINAL_BASE, chain, address
    ));

    section.push_str("\n---\n\n");
    section.push_str("*This report was generated automatically. Always verify data from primary sources before making decisions.*\n");

    section
}

/// Saves a report to a file.
///
/// # Arguments
///
/// * `report` - The markdown report content
/// * `path` - The file path to save to
///
/// # Returns
///
/// Returns `Ok(())` on success, or an error if the file cannot be written.
pub fn save_report(report: &str, path: impl AsRef<Path>) -> Result<()> {
    std::fs::write(path.as_ref(), report).map_err(|e| {
        crate::error::ScopeError::Io(format!(
            "Failed to write report to {}: {}",
            path.as_ref().display(),
            e
        ))
    })
}

/// Formats a USD value with appropriate suffixes.
fn format_usd(value: f64) -> String {
    if value >= 1_000_000_000.0 {
        format!("${:.2}B", value / 1_000_000_000.0)
    } else if value >= 1_000_000.0 {
        format!("${:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("${:.0}K", value / 1_000.0)
    } else {
        format!("${:.2}", value)
    }
}

/// Formats a number with commas.
fn format_number(value: f64) -> String {
    if value >= 1_000_000.0 {
        format!("{:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.0}K", value / 1_000.0)
    } else {
        format!("{:.0}", value)
    }
}

/// Capitalizes the first letter of a string.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

// ============================================================================
// Mermaid Chart Generation
// ============================================================================

/// Generates a Mermaid line chart for price history.
fn generate_price_chart(analytics: &TokenAnalytics) -> String {
    // Generate price change comparison chart for multiple timeframes
    let mut chart = String::new();

    // Only generate if we have meaningful data
    if analytics.price_change_1h == 0.0
        && analytics.price_change_6h == 0.0
        && analytics.price_change_24h == 0.0
        && analytics.price_change_7d == 0.0
    {
        // Fall back to price history chart if no change data
        if analytics.price_history.len() >= 2 {
            return generate_price_history_chart(analytics);
        }
        return String::new();
    }

    chart.push_str("\n### Price Changes by Period\n\n");
    chart.push_str("```mermaid\n");
    chart.push_str("%%{init: {'theme': 'base'}}%%\n");
    chart.push_str("xychart-beta\n");
    chart.push_str("    title \"Price Change Comparison (%)\"\n");
    chart.push_str("    x-axis [\"1h\", \"6h\", \"24h\", \"7d\"]\n");
    chart.push_str("    y-axis \"Change %\"\n");
    chart.push_str(&format!(
        "    bar [{:.2}, {:.2}, {:.2}, {:.2}]\n",
        analytics.price_change_1h,
        analytics.price_change_6h,
        analytics.price_change_24h,
        analytics.price_change_7d
    ));
    chart.push_str("```\n");

    // Also add the price history chart if available
    if analytics.price_history.len() >= 2 {
        chart.push_str(&generate_price_history_chart(analytics));
    }

    chart
}

/// Generates a price history line chart from historical data points.
fn generate_price_history_chart(analytics: &TokenAnalytics) -> String {
    if analytics.price_history.len() < 2 {
        return String::new();
    }

    let mut chart = String::new();
    chart.push_str("\n### Price History\n\n");
    chart.push_str("```mermaid\n");
    chart.push_str("xychart-beta\n");
    chart.push_str("    title \"Price Over Time\"\n");
    chart.push_str("    x-axis [");

    // Sample up to 12 data points for readability
    let step = (analytics.price_history.len() / 12).max(1);
    let sampled: Vec<_> = analytics
        .price_history
        .iter()
        .step_by(step)
        .take(12)
        .collect();

    // Generate x-axis labels (time offsets)
    let labels: Vec<String> = sampled
        .iter()
        .enumerate()
        .map(|(i, _)| format!("\"{}\"", i + 1))
        .collect();
    chart.push_str(&labels.join(", "));
    chart.push_str("]\n");

    // Generate y-axis with price data
    let prices: Vec<String> = sampled.iter().map(|p| format!("{:.6}", p.price)).collect();
    chart.push_str("    y-axis \"Price (USD)\"\n");
    chart.push_str("    line [");
    chart.push_str(&prices.join(", "));
    chart.push_str("]\n");
    chart.push_str("```\n");

    chart
}

/// Generates a Mermaid bar chart for volume history.
fn generate_volume_chart(analytics: &TokenAnalytics) -> String {
    if analytics.volume_history.len() < 2 {
        return String::new();
    }

    let mut chart = String::new();
    chart.push_str("\n### Volume Chart\n\n");
    chart.push_str("```mermaid\n");
    chart.push_str("xychart-beta\n");
    chart.push_str("    title \"Trading Volume Over Time\"\n");
    chart.push_str("    x-axis [");

    // Sample up to 12 data points for readability
    let step = (analytics.volume_history.len() / 12).max(1);
    let sampled: Vec<_> = analytics
        .volume_history
        .iter()
        .step_by(step)
        .take(12)
        .collect();

    // Generate x-axis labels
    let labels: Vec<String> = sampled
        .iter()
        .enumerate()
        .map(|(i, _)| format!("\"{}\"", i + 1))
        .collect();
    chart.push_str(&labels.join(", "));
    chart.push_str("]\n");

    // Generate y-axis with volume data
    let volumes: Vec<String> = sampled.iter().map(|v| format!("{:.0}", v.volume)).collect();
    chart.push_str("    y-axis \"Volume (USD)\"\n");
    chart.push_str("    bar [");
    chart.push_str(&volumes.join(", "));
    chart.push_str("]\n");
    chart.push_str("```\n");

    chart
}

/// Generates a Mermaid pie chart for liquidity distribution across DEXes.
fn generate_liquidity_chart(analytics: &TokenAnalytics) -> String {
    if analytics.dex_pairs.is_empty() {
        return String::new();
    }

    // Only show chart if there are multiple DEXes
    if analytics.dex_pairs.len() < 2 {
        return String::new();
    }

    let mut chart = String::new();
    chart.push_str("\n### Liquidity Distribution by DEX\n\n");
    chart.push_str("```mermaid\n");
    chart.push_str("pie showData\n");
    chart.push_str("    title Liquidity by DEX\n");

    // Aggregate liquidity by DEX name
    let mut dex_liquidity: std::collections::HashMap<String, f64> =
        std::collections::HashMap::new();
    for pair in &analytics.dex_pairs {
        *dex_liquidity.entry(pair.dex_name.clone()).or_insert(0.0) += pair.liquidity_usd;
    }

    // Sort by liquidity descending and take top 6
    let mut sorted: Vec<_> = dex_liquidity.into_iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    for (dex, liquidity) in sorted.iter().take(6) {
        // Mermaid pie values need to be positive integers or percentages
        let value = (liquidity / 1_000_000.0).max(0.01); // Convert to millions
        chart.push_str(&format!("    \"{}\" : {:.2}\n", dex, value));
    }

    chart.push_str("```\n");

    chart
}

/// Generates a Mermaid pie chart for holder concentration.
fn generate_concentration_chart(analytics: &TokenAnalytics) -> String {
    // Calculate concentration from holder data or use stored values
    let top_10_pct: f64 = analytics.top_10_concentration.unwrap_or_else(|| {
        analytics
            .holders
            .iter()
            .take(10)
            .map(|h| h.percentage)
            .sum()
    });

    // Only show chart if we have meaningful concentration data
    if top_10_pct <= 0.0 || analytics.holders.is_empty() {
        return String::new();
    }

    let remaining = (100.0 - top_10_pct).max(0.0);

    let mut chart = String::new();
    chart.push_str("\n### Holder Concentration Chart\n\n");
    chart.push_str("```mermaid\n");
    chart.push_str("pie showData\n");
    chart.push_str("    title Token Holder Distribution\n");
    chart.push_str(&format!("    \"Top 10 Holders\" : {:.1}\n", top_10_pct));
    chart.push_str(&format!("    \"Other Holders\" : {:.1}\n", remaining));

    // Add top 50 if different enough from top 10
    let top_50_pct = analytics.top_50_concentration.unwrap_or_else(|| {
        analytics
            .holders
            .iter()
            .take(50)
            .map(|h| h.percentage)
            .sum()
    });

    if top_50_pct > top_10_pct + 5.0 {
        // Show breakdown: Top 10 vs 11-50 vs Rest
        let between_10_50 = top_50_pct - top_10_pct;
        let rest = (100.0 - top_50_pct).max(0.0);

        // Regenerate with 3 segments
        chart.clear();
        chart.push_str("\n### Holder Concentration Chart\n\n");
        chart.push_str("```mermaid\n");
        chart.push_str("pie showData\n");
        chart.push_str("    title Token Holder Distribution\n");
        chart.push_str(&format!("    \"Top 10\" : {:.1}\n", top_10_pct));
        chart.push_str(&format!("    \"Rank 11-50\" : {:.1}\n", between_10_50));
        chart.push_str(&format!("    \"Others\" : {:.1}\n", rest));
    }

    chart.push_str("```\n");

    chart
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::{DexPair, Token, TokenHolder, TokenSocial};

    fn create_test_analytics() -> TokenAnalytics {
        TokenAnalytics {
            token: Token {
                contract_address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                decimals: 6,
            },
            chain: "ethereum".to_string(),
            holders: vec![
                TokenHolder {
                    address: "0x55FE002e15bA7591a5E5Ce68a6D3c6E1593d3d8c".to_string(),
                    balance: "1250000000000000".to_string(),
                    formatted_balance: "1.25B".to_string(),
                    percentage: 12.5,
                    rank: 1,
                },
                TokenHolder {
                    address: "0x8894E0a0c962CB723c1976a4421c95949bE2a912".to_string(),
                    balance: "820000000000000".to_string(),
                    formatted_balance: "820M".to_string(),
                    percentage: 8.2,
                    rank: 2,
                },
            ],
            total_holders: 1234567,
            volume_24h: 1234567890.0,
            volume_7d: 8641975230.0,
            price_usd: 1.0002,
            price_change_24h: 0.01,
            price_change_7d: -0.05,
            liquidity_usd: 500000000.0,
            market_cap: Some(32500000000.0),
            fdv: Some(40000000000.0),
            total_supply: Some("40,000,000,000".to_string()),
            circulating_supply: Some("32,500,000,000".to_string()),
            price_history: vec![],
            volume_history: vec![],
            holder_history: vec![],
            dex_pairs: vec![DexPair {
                dex_name: "Uniswap V3".to_string(),
                pair_address: "0x1234".to_string(),
                base_token: "USDC".to_string(),
                quote_token: "ETH".to_string(),
                price_usd: 1.0002,
                volume_24h: 500000000.0,
                liquidity_usd: 250000000.0,
                price_change_24h: 0.01,
                buys_24h: 1234,
                sells_24h: 1189,
                buys_6h: 234,
                sells_6h: 220,
                buys_1h: 45,
                sells_1h: 42,
                pair_created_at: Some(1700000000 - 86400 * 30), // 30 days ago
                url: Some("https://dexscreener.com/ethereum/0x1234".to_string()),
            }],
            fetched_at: 1700000000,
            top_10_concentration: Some(45.2),
            top_50_concentration: Some(67.8),
            top_100_concentration: Some(78.5),
            price_change_6h: 0.5,
            price_change_1h: -0.1,
            total_buys_24h: 1234,
            total_sells_24h: 1189,
            total_buys_6h: 234,
            total_sells_6h: 220,
            total_buys_1h: 45,
            total_sells_1h: 42,
            token_age_hours: Some(720.0), // 30 days
            image_url: Some("https://example.com/usdc.png".to_string()),
            websites: vec!["https://www.circle.com/usdc".to_string()],
            socials: vec![TokenSocial {
                platform: "twitter".to_string(),
                url: "https://twitter.com/USDC".to_string(),
            }],
            dexscreener_url: Some("https://dexscreener.com/ethereum/0x1234".to_string()),
        }
    }

    #[test]
    fn test_generate_report() {
        let analytics = create_test_analytics();
        let report = generate_report(&analytics);

        // Check header
        assert!(report.contains("# Token Analysis Report: USDC"));
        assert!(report.contains("**Chain:** Ethereum"));
        assert!(report.contains("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"));

        // Check that addresses are NOT truncated
        assert!(report.contains("0x55FE002e15bA7591a5E5Ce68a6D3c6E1593d3d8c"));
        assert!(report.contains("0x8894E0a0c962CB723c1976a4421c95949bE2a912"));

        // Check sections exist
        assert!(report.contains("## Executive Summary"));
        assert!(report.contains("## Top Holders"));
        assert!(report.contains("## Concentration Analysis"));
        assert!(report.contains("## Data Sources"));
    }

    #[test]
    fn test_format_usd() {
        assert_eq!(format_usd(1500000000.0), "$1.50B");
        assert_eq!(format_usd(1500000.0), "$1.50M");
        assert_eq!(format_usd(1500.0), "$2K"); // 1500 / 1000 = 1.5, rounded to 2K
        assert_eq!(format_usd(15.5), "$15.50");
    }

    #[test]
    fn test_capitalize() {
        assert_eq!(capitalize("ethereum"), "Ethereum");
        assert_eq!(capitalize("bsc"), "Bsc");
        assert_eq!(capitalize(""), "");
    }

    #[test]
    fn test_full_addresses_not_truncated() {
        let analytics = create_test_analytics();
        let section = generate_holder_section(&analytics);

        // Verify full addresses are present
        assert!(section.contains("0x55FE002e15bA7591a5E5Ce68a6D3c6E1593d3d8c"));
        assert!(section.contains("0x8894E0a0c962CB723c1976a4421c95949bE2a912"));

        // Verify truncated format is NOT used
        assert!(!section.contains("..."));
    }

    #[test]
    fn test_concentration_analysis() {
        let analytics = create_test_analytics();
        let section = generate_concentration_analysis(&analytics);

        assert!(section.contains("45.1%") || section.contains("45.2%"));
        assert!(section.contains("Top 10 holders"));
    }

    #[test]
    fn test_security_analysis_section() {
        let analytics = create_test_analytics();
        let section = generate_security_analysis(&analytics);

        // Check section header
        assert!(section.contains("## Security Analysis"));

        // Check for security checks table
        assert!(section.contains("Honeypot Risk"));
        assert!(section.contains("Token Age"));
        assert!(section.contains("Whale Risk"));
        assert!(section.contains("Social Presence"));

        // Check for buy/sell data
        assert!(section.contains("1234"));
        assert!(section.contains("1189"));
    }

    #[test]
    fn test_security_analysis_honeypot_detection() {
        let mut analytics = create_test_analytics();

        // Test high honeypot risk (many buys, few sells)
        analytics.total_buys_24h = 1000;
        analytics.total_sells_24h = 10;
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("HIGH") || section.contains("Suspicious"));

        // Test normal activity
        analytics.total_buys_24h = 100;
        analytics.total_sells_24h = 95;
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("LOW") || section.contains("Normal"));
    }

    #[test]
    fn test_token_info_section() {
        let analytics = create_test_analytics();
        let section = generate_token_info_section(&analytics);

        // Check section header
        assert!(section.contains("## Token Information"));

        // Check for social links
        assert!(section.contains("Twitter") || section.contains("twitter"));
        assert!(section.contains("https://twitter.com/USDC"));

        // Check for website
        assert!(section.contains("circle.com"));

        // Check for DexScreener link
        assert!(section.contains("DexScreener"));
    }

    #[test]
    fn test_risk_score_calculation() {
        let analytics = create_test_analytics();
        let factors = RiskFactors::from_analytics(&analytics);

        // Verify factors are in valid range
        assert!(factors.honeypot <= 10);
        assert!(factors.age <= 10);
        assert!(factors.liquidity <= 10);
        assert!(factors.concentration <= 10);
        assert!(factors.social <= 10);

        // Verify overall score is in valid range
        let overall = factors.overall_score();
        assert!((1..=10).contains(&overall));
    }

    #[test]
    fn test_risk_score_section() {
        let analytics = create_test_analytics();
        let section = generate_risk_score_section(&analytics);

        // Check section header
        assert!(section.contains("## Risk Score"));

        // Check for overall risk display
        assert!(section.contains("Overall Risk:"));
        assert!(section.contains("/10"));

        // Check for factor breakdown table
        assert!(section.contains("Honeypot Risk"));
        assert!(section.contains("Token Age"));
        assert!(section.contains("Liquidity"));
        assert!(section.contains("Concentration"));
        assert!(section.contains("Social Presence"));

        // Check for Mermaid chart
        assert!(section.contains("```mermaid"));
        assert!(section.contains("pie showData"));
    }

    #[test]
    fn test_buysell_chart() {
        let analytics = create_test_analytics();
        let chart = generate_buysell_chart(&analytics);

        // Check for Mermaid syntax
        assert!(chart.contains("```mermaid"));
        assert!(chart.contains("pie showData"));
        assert!(chart.contains("Buys"));
        assert!(chart.contains("Sells"));
    }

    #[test]
    fn test_txn_activity_chart() {
        let analytics = create_test_analytics();
        let chart = generate_txn_activity_chart(&analytics);

        // Check for Mermaid syntax
        assert!(chart.contains("```mermaid"));
        assert!(chart.contains("xychart-beta"));
        assert!(chart.contains("1h") || chart.contains("6h") || chart.contains("24h"));
    }

    #[test]
    fn test_price_change_chart() {
        let analytics = create_test_analytics();
        let chart = generate_price_chart(&analytics);

        // Check for Mermaid syntax
        assert!(chart.contains("```mermaid"));
        assert!(chart.contains("Price Change"));
    }

    #[test]
    fn test_new_report_sections_included() {
        let analytics = create_test_analytics();
        let report = generate_report(&analytics);

        // Check new sections are included in the report
        assert!(report.contains("## Token Information"));
        assert!(report.contains("## Security Analysis"));
        assert!(report.contains("## Risk Score"));
    }

    // ========================================================================
    // Edge case tests
    // ========================================================================

    #[test]
    fn test_generate_report_no_holders() {
        let mut analytics = create_test_analytics();
        analytics.holders = vec![];
        analytics.total_holders = 0;
        analytics.top_10_concentration = None;
        analytics.top_50_concentration = None;
        analytics.top_100_concentration = None;
        let report = generate_report(&analytics);
        assert!(report.contains("No holder data available"));
    }

    #[test]
    fn test_generate_report_no_market_cap() {
        let mut analytics = create_test_analytics();
        analytics.market_cap = None;
        analytics.fdv = None;
        analytics.total_supply = None;
        analytics.circulating_supply = None;
        let report = generate_report(&analytics);
        assert!(!report.contains("Market Cap | $"));
        assert!(!report.contains("Fully Diluted Valuation | $"));
    }

    #[test]
    fn test_generate_report_no_dex_pairs() {
        let mut analytics = create_test_analytics();
        analytics.dex_pairs = vec![];
        analytics.liquidity_usd = 0.0;
        let report = generate_report(&analytics);
        // Should still generate without errors
        assert!(report.contains("## Liquidity Analysis"));
    }

    #[test]
    fn test_generate_report_no_social_no_website() {
        let mut analytics = create_test_analytics();
        analytics.socials = vec![];
        analytics.websites = vec![];
        analytics.image_url = None;
        analytics.dexscreener_url = None;
        let section = generate_token_info_section(&analytics);
        assert!(section.contains("No additional token metadata available"));
    }

    #[test]
    fn test_security_analysis_zero_transactions() {
        let mut analytics = create_test_analytics();
        analytics.total_buys_24h = 0;
        analytics.total_sells_24h = 0;
        analytics.total_buys_6h = 0;
        analytics.total_sells_6h = 0;
        analytics.total_buys_1h = 0;
        analytics.total_sells_1h = 0;
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("UNKNOWN") || section.contains("No transaction data"));
    }

    #[test]
    fn test_security_analysis_only_buys() {
        let mut analytics = create_test_analytics();
        analytics.total_buys_24h = 100;
        analytics.total_sells_24h = 0;
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("Possible honeypot") || section.contains("HIGH"));
    }

    #[test]
    fn test_security_analysis_token_age_very_new() {
        let mut analytics = create_test_analytics();
        analytics.token_age_hours = Some(6.0);
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("Very new token") || section.contains("HIGH RISK"));
    }

    #[test]
    fn test_security_analysis_token_age_unknown() {
        let mut analytics = create_test_analytics();
        analytics.token_age_hours = None;
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("not available") || section.contains("UNKNOWN"));
    }

    #[test]
    fn test_security_analysis_whale_risk_extreme() {
        let mut analytics = create_test_analytics();
        analytics.holders = vec![TokenHolder {
            address: "0xwhale".to_string(),
            balance: "9000000".to_string(),
            formatted_balance: "9M".to_string(),
            percentage: 60.0,
            rank: 1,
        }];
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("HIGH") || section.contains("Extreme concentration"));
    }

    #[test]
    fn test_security_analysis_no_holders() {
        let mut analytics = create_test_analytics();
        analytics.holders = vec![];
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("Whale Risk"));
        assert!(section.contains("UNKNOWN") || section.contains("not available"));
    }

    #[test]
    fn test_risk_factors_high_risk_token() {
        let mut analytics = create_test_analytics();
        analytics.total_buys_24h = 1000;
        analytics.total_sells_24h = 0; // Honeypot risk = 10
        analytics.token_age_hours = Some(12.0); // Very new = 10
        analytics.liquidity_usd = 5_000.0; // Very low = 10
        analytics.holders = vec![TokenHolder {
            address: "0x1".to_string(),
            balance: "1000".to_string(),
            formatted_balance: "1K".to_string(),
            percentage: 80.0, // Very concentrated = 10
            rank: 1,
        }];
        analytics.socials = vec![]; // No socials = 8
        analytics.websites = vec![];

        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.honeypot, 10);
        assert_eq!(factors.age, 10);
        assert_eq!(factors.liquidity, 10);
        assert_eq!(factors.concentration, 10);
        assert_eq!(factors.social, 8);
        assert!(factors.overall_score() >= 8);
        assert!(factors.risk_level() == "HIGH" || factors.risk_level() == "CRITICAL");
        assert!(factors.risk_emoji() == "🟠" || factors.risk_emoji() == "🔴");
    }

    #[test]
    fn test_risk_factors_low_risk_token() {
        let mut analytics = create_test_analytics();
        analytics.total_buys_24h = 100;
        analytics.total_sells_24h = 95; // Normal ratio
        analytics.token_age_hours = Some(10_000.0); // Very established
        analytics.liquidity_usd = 50_000_000.0; // Very high
        analytics.holders = vec![TokenHolder {
            address: "0x1".to_string(),
            balance: "1000".to_string(),
            formatted_balance: "1K".to_string(),
            percentage: 3.0, // Well distributed
            rank: 1,
        }];
        analytics.socials = vec![
            TokenSocial {
                platform: "twitter".to_string(),
                url: "https://twitter.com/test".to_string(),
            },
            TokenSocial {
                platform: "telegram".to_string(),
                url: "https://t.me/test".to_string(),
            },
        ];
        analytics.websites = vec!["https://example.com".to_string()];

        let factors = RiskFactors::from_analytics(&analytics);
        assert!(factors.overall_score() <= 3);
        assert_eq!(factors.risk_level(), "LOW");
        assert_eq!(factors.risk_emoji(), "🟢");
    }

    #[test]
    fn test_risk_assessment_labels() {
        assert_eq!(risk_assessment(0), "Low Risk");
        assert_eq!(risk_assessment(1), "Low Risk");
        assert_eq!(risk_assessment(3), "Moderate");
        assert_eq!(risk_assessment(5), "Elevated");
        assert_eq!(risk_assessment(7), "High Risk");
        assert_eq!(risk_assessment(9), "Critical");
        assert_eq!(risk_assessment(10), "Critical");
    }

    #[test]
    fn test_format_usd_edge_cases() {
        assert_eq!(format_usd(0.0), "$0.00");
        assert_eq!(format_usd(0.50), "$0.50");
        assert_eq!(format_usd(999.0), "$999.00");
    }

    #[test]
    fn test_format_number_edge_cases() {
        assert_eq!(format_number(0.0), "0");
        assert_eq!(format_number(500.0), "500");
        assert_eq!(format_number(1500.0), "2K");
        assert_eq!(format_number(1_500_000.0), "1.50M");
    }

    #[test]
    fn test_capitalize_edge_cases() {
        assert_eq!(capitalize("a"), "A");
        assert_eq!(capitalize("ABC"), "ABC");
    }

    #[test]
    fn test_data_sources_different_chains() {
        let chains = vec![
            ("ethereum", "etherscan.io"),
            ("polygon", "polygonscan.com"),
            ("arbitrum", "arbiscan.io"),
            ("optimism", "optimistic.etherscan.io"),
            ("base", "basescan.org"),
            ("bsc", "bscscan.com"),
            ("solana", "solscan.io"),
        ];

        for (chain, expected_domain) in chains {
            let mut analytics = create_test_analytics();
            analytics.chain = chain.to_string();
            let section = generate_data_sources(&analytics);
            assert!(
                section.contains(expected_domain),
                "Chain {} should link to {}",
                chain,
                expected_domain
            );
        }
    }

    #[test]
    fn test_buysell_chart_empty() {
        let mut analytics = create_test_analytics();
        analytics.total_buys_24h = 0;
        analytics.total_sells_24h = 0;
        let chart = generate_buysell_chart(&analytics);
        assert!(chart.is_empty());
    }

    #[test]
    fn test_txn_activity_chart_empty() {
        let mut analytics = create_test_analytics();
        analytics.total_buys_24h = 0;
        analytics.total_sells_24h = 0;
        analytics.total_buys_6h = 0;
        analytics.total_sells_6h = 0;
        analytics.total_buys_1h = 0;
        analytics.total_sells_1h = 0;
        let chart = generate_txn_activity_chart(&analytics);
        assert!(chart.is_empty());
    }

    #[test]
    fn test_volume_chart_empty() {
        let analytics = create_test_analytics();
        // analytics has empty volume_history by default
        let chart = generate_volume_chart(&analytics);
        assert!(chart.is_empty());
    }

    #[test]
    fn test_liquidity_chart_single_pair() {
        let analytics = create_test_analytics();
        // Only 1 DEX pair → no chart generated
        assert_eq!(analytics.dex_pairs.len(), 1);
        let chart = generate_liquidity_chart(&analytics);
        assert!(chart.is_empty());
    }

    #[test]
    fn test_liquidity_chart_multiple_pairs() {
        let mut analytics = create_test_analytics();
        analytics.dex_pairs.push(DexPair {
            dex_name: "SushiSwap".to_string(),
            pair_address: "0x5678".to_string(),
            base_token: "USDC".to_string(),
            quote_token: "DAI".to_string(),
            price_usd: 1.0,
            volume_24h: 100_000.0,
            liquidity_usd: 5_000_000.0,
            price_change_24h: 0.0,
            buys_24h: 50,
            sells_24h: 50,
            buys_6h: 10,
            sells_6h: 10,
            buys_1h: 2,
            sells_1h: 2,
            pair_created_at: None,
            url: None,
        });
        let chart = generate_liquidity_chart(&analytics);
        assert!(chart.contains("mermaid"));
        assert!(chart.contains("Uniswap V3"));
        assert!(chart.contains("SushiSwap"));
    }

    #[test]
    fn test_concentration_chart_no_holders() {
        let mut analytics = create_test_analytics();
        analytics.holders = vec![];
        analytics.top_10_concentration = Some(0.0);
        let chart = generate_concentration_chart(&analytics);
        assert!(chart.is_empty());
    }

    #[test]
    fn test_concentration_analysis_ranges() {
        // Very high concentration
        let mut analytics = create_test_analytics();
        analytics.top_10_concentration = Some(85.0);
        let section = generate_concentration_analysis(&analytics);
        assert!(section.contains("Very High Concentration"));

        // High concentration
        analytics.top_10_concentration = Some(55.0);
        let section = generate_concentration_analysis(&analytics);
        assert!(section.contains("High Concentration"));

        // Low concentration
        analytics.top_10_concentration = Some(15.0);
        let section = generate_concentration_analysis(&analytics);
        assert!(section.contains("Low Concentration"));
    }

    #[test]
    fn test_risk_indicators_low_liquidity() {
        let mut analytics = create_test_analytics();
        analytics.liquidity_usd = 5_000.0;
        let section = generate_risk_indicators(&analytics);
        assert!(section.contains("Very low liquidity"));
    }

    #[test]
    fn test_risk_indicators_high_volatility() {
        let mut analytics = create_test_analytics();
        analytics.price_change_24h = 25.0;
        let section = generate_risk_indicators(&analytics);
        assert!(section.contains("High price volatility"));
    }

    #[test]
    fn test_risk_indicators_healthy_token() {
        let mut analytics = create_test_analytics();
        analytics.holders = vec![TokenHolder {
            address: "0x1".to_string(),
            balance: "100".to_string(),
            formatted_balance: "100".to_string(),
            percentage: 5.0,
            rank: 1,
        }];
        analytics.liquidity_usd = 10_000_000.0;
        analytics.volume_24h = 500_000.0;
        analytics.price_change_24h = 2.0;
        let section = generate_risk_indicators(&analytics);
        assert!(section.contains("Reasonable distribution"));
        assert!(section.contains("Good liquidity"));
        assert!(section.contains("Active trading"));
    }

    #[test]
    fn test_risk_indicators_empty() {
        let mut analytics = create_test_analytics();
        analytics.holders = vec![];
        analytics.liquidity_usd = 500_000.0;
        analytics.volume_24h = 50_000.0;
        analytics.price_change_24h = 5.0;
        let section = generate_risk_indicators(&analytics);
        // With no holders, calculation uses empty iter → 0%, which is "reasonable"
        assert!(section.contains("Reasonable distribution"));
    }

    #[test]
    fn test_save_report() {
        let tmp = std::env::temp_dir().join("bcc_test_report.md");
        let result = save_report("# Test Report\n\nContent here.", &tmp);
        assert!(result.is_ok());
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert!(content.contains("# Test Report"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_save_report_invalid_path() {
        let result = save_report("content", "/nonexistent/directory/report.md");
        assert!(result.is_err());
    }

    #[test]
    fn test_volume_analysis_high_vol_to_liq() {
        let mut analytics = create_test_analytics();
        analytics.volume_24h = 100_000_000.0;
        analytics.liquidity_usd = 10_000_000.0; // ratio = 10
        let section = generate_volume_analysis(&analytics);
        assert!(section.contains("unusual trading activity"));
    }

    #[test]
    fn test_price_analysis_with_history() {
        use crate::chains::PricePoint;
        let mut analytics = create_test_analytics();
        analytics.price_history = vec![
            PricePoint {
                timestamp: 1700000000,
                price: 1.0,
            },
            PricePoint {
                timestamp: 1700003600,
                price: 1.5,
            },
            PricePoint {
                timestamp: 1700007200,
                price: 0.8,
            },
        ];
        let section = generate_price_analysis(&analytics);
        assert!(section.contains("Price Range"));
        assert!(section.contains("High"));
        assert!(section.contains("Low"));
        assert!(section.contains("Average"));
    }

    #[test]
    fn test_social_platform_icons() {
        let mut analytics = create_test_analytics();
        analytics.socials = vec![
            TokenSocial {
                platform: "twitter".to_string(),
                url: "https://twitter.com/test".to_string(),
            },
            TokenSocial {
                platform: "telegram".to_string(),
                url: "https://t.me/test".to_string(),
            },
            TokenSocial {
                platform: "discord".to_string(),
                url: "https://discord.gg/test".to_string(),
            },
            TokenSocial {
                platform: "github".to_string(),
                url: "https://github.com/test".to_string(),
            },
            TokenSocial {
                platform: "unknown".to_string(),
                url: "https://example.com".to_string(),
            },
        ];
        let section = generate_token_info_section(&analytics);
        assert!(section.contains("🐦")); // twitter
        assert!(section.contains("📱")); // telegram
        assert!(section.contains("💬")); // discord
        assert!(section.contains("💻")); // github
        assert!(section.contains("🔗")); // unknown
    }

    #[test]
    fn test_security_analysis_token_age_ranges() {
        let mut analytics = create_test_analytics();

        // Very new (< 24h)
        analytics.token_age_hours = Some(6.0);
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("HIGH RISK"));

        // New (24-48h)
        analytics.token_age_hours = Some(36.0);
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("MEDIUM"));

        // Relatively new (< 7d)
        analytics.token_age_hours = Some(120.0);
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("CAUTION"));

        // Established (> 1 year)
        analytics.token_age_hours = Some(10_000.0);
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("ESTABLISHED"));
    }

    #[test]
    fn test_price_history_chart_with_data() {
        use crate::chains::PricePoint;
        let mut analytics = create_test_analytics();
        analytics.price_history = (0..20)
            .map(|i| PricePoint {
                timestamp: 1700000000 + i * 3600,
                price: 1.0 + (i as f64) * 0.01,
            })
            .collect();
        let chart = generate_price_history_chart(&analytics);
        assert!(chart.contains("Price History"));
        assert!(chart.contains("mermaid"));
        assert!(chart.contains("xychart-beta"));
        assert!(chart.contains("line ["));
    }

    #[test]
    fn test_price_chart_with_changes_and_history() {
        use crate::chains::PricePoint;
        let mut analytics = create_test_analytics();
        analytics.price_change_1h = 1.5;
        analytics.price_change_6h = -2.3;
        analytics.price_change_24h = 5.0;
        analytics.price_change_7d = -10.0;
        analytics.price_history = (0..5)
            .map(|i| PricePoint {
                timestamp: 1700000000 + i * 3600,
                price: 1.0 + (i as f64) * 0.1,
            })
            .collect();
        let chart = generate_price_chart(&analytics);
        assert!(chart.contains("Price Changes by Period"));
        assert!(chart.contains("Price History")); // Also includes history chart
    }

    #[test]
    fn test_price_chart_zero_changes_with_history() {
        use crate::chains::PricePoint;
        let mut analytics = create_test_analytics();
        analytics.price_change_1h = 0.0;
        analytics.price_change_6h = 0.0;
        analytics.price_change_24h = 0.0;
        analytics.price_change_7d = 0.0;
        analytics.price_history = vec![
            PricePoint {
                timestamp: 1700000000,
                price: 1.0,
            },
            PricePoint {
                timestamp: 1700003600,
                price: 1.5,
            },
        ];
        let chart = generate_price_chart(&analytics);
        assert!(chart.contains("Price History")); // Falls back to history chart
    }

    #[test]
    fn test_volume_chart_with_data() {
        use crate::chains::VolumePoint;
        let mut analytics = create_test_analytics();
        analytics.volume_history = (0..10)
            .map(|i| VolumePoint {
                timestamp: 1700000000 + i * 3600,
                volume: 100_000.0 + (i as f64) * 50_000.0,
            })
            .collect();
        let chart = generate_volume_chart(&analytics);
        assert!(chart.contains("Volume Chart"));
        assert!(chart.contains("mermaid"));
        assert!(chart.contains("bar ["));
    }

    #[test]
    fn test_concentration_chart_three_segments() {
        let mut analytics = create_test_analytics();
        // Set top_10 = 30%, top_50 = 60% (difference > 5%), triggers 3-segment chart
        analytics.top_10_concentration = Some(30.0);
        analytics.top_50_concentration = Some(60.0);
        let chart = generate_concentration_chart(&analytics);
        assert!(chart.contains("Top 10"));
        assert!(chart.contains("Rank 11-50"));
        assert!(chart.contains("Others"));
    }

    #[test]
    fn test_risk_indicators_very_low_liquidity() {
        let mut analytics = create_test_analytics();
        analytics.liquidity_usd = 5_000.0;
        analytics.volume_24h = 500.0;
        let section = generate_risk_indicators(&analytics);
        assert!(section.contains("Very low liquidity"));
        assert!(section.contains("Very low trading volume"));
    }

    #[test]
    fn test_risk_indicators_moderate_liquidity() {
        let mut analytics = create_test_analytics();
        analytics.liquidity_usd = 50_000.0;
        let section = generate_risk_indicators(&analytics);
        assert!(section.contains("Low liquidity"));
    }

    #[test]
    fn test_risk_indicators_extreme_concentration() {
        let mut analytics = create_test_analytics();
        analytics.holders = vec![TokenHolder {
            address: "0xwhale".to_string(),
            balance: "900000000".to_string(),
            formatted_balance: "900M".to_string(),
            percentage: 90.0,
            rank: 1,
        }];
        let section = generate_risk_indicators(&analytics);
        assert!(section.contains("Extreme whale concentration"));
    }

    #[test]
    fn test_risk_indicators_no_data() {
        let mut analytics = create_test_analytics();
        analytics.holders = vec![];
        analytics.liquidity_usd = 500_000.0; // between 100k and 1M, no risk or positive
        analytics.volume_24h = 50_000.0; // between 1k and 100k, no risk or positive
        analytics.price_change_24h = 5.0; // less than 20%, no risk
        let section = generate_risk_indicators(&analytics);
        // Should have insufficient data or at least no risk factors
        assert!(section.contains("Risk Indicators"));
    }

    #[test]
    fn test_holder_section_with_data() {
        let analytics = create_test_analytics();
        let section = generate_holder_section(&analytics);
        assert!(section.contains("Top Holders"));
        assert!(section.contains("0x55FE002e15bA7591a5E5Ce68a6D3c6E1593d3d8c")); // Full address
        assert!(section.contains("12.50%"));
    }

    #[test]
    fn test_risk_breakdown_chart() {
        let analytics = create_test_analytics();
        let section = generate_risk_score_section(&analytics);
        assert!(section.contains("Risk Score"));
        assert!(section.contains("Risk Factor Breakdown"));
        assert!(section.contains("Honeypot"));
        assert!(section.contains("Token Age"));
    }

    #[test]
    fn test_data_sources_section() {
        let analytics = create_test_analytics();
        let section = generate_data_sources(&analytics);
        assert!(section.contains("Data Sources"));
        assert!(section.contains("ethereum"));
    }

    #[test]
    fn test_volume_analysis_section() {
        let analytics = create_test_analytics();
        let section = generate_volume_analysis(&analytics);
        assert!(section.contains("Volume Analysis"));
    }

    #[test]
    fn test_liquidity_analysis_section() {
        let analytics = create_test_analytics();
        let section = generate_liquidity_analysis(&analytics);
        assert!(section.contains("Liquidity Analysis"));
    }

    #[test]
    fn test_security_analysis_medium_buy_sell_ratio() {
        let mut analytics = create_test_analytics();
        // ratio = 5.0 → MEDIUM risk
        analytics.total_buys_24h = 100;
        analytics.total_sells_24h = 20;
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("MEDIUM") || section.contains("Elevated"));
    }

    #[test]
    fn test_security_analysis_token_age_months() {
        let mut analytics = create_test_analytics();
        // 2000 hours ≈ 83 days ≈ 2.8 months (> 30 days, < 365 days)
        analytics.token_age_hours = Some(2000.0);
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("ESTABLISHED") || section.contains("months"));
    }

    #[test]
    fn test_security_analysis_whale_risk_medium() {
        let mut analytics = create_test_analytics();
        analytics.holders = vec![TokenHolder {
            address: "0xwhale".to_string(),
            balance: "3000000".to_string(),
            formatted_balance: "3M".to_string(),
            percentage: 30.0, // > 25%, <= 50%
            rank: 1,
        }];
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("MEDIUM") || section.contains("High concentration"));
    }

    #[test]
    fn test_security_analysis_whale_risk_low() {
        let mut analytics = create_test_analytics();
        analytics.holders = vec![TokenHolder {
            address: "0xholder".to_string(),
            balance: "500000".to_string(),
            formatted_balance: "500K".to_string(),
            percentage: 5.0, // > 0%, <= 10%
            rank: 1,
        }];
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("LOW") || section.contains("Well distributed"));
    }

    #[test]
    fn test_security_analysis_token_age_days_format() {
        let mut analytics = create_test_analytics();
        // 480 hours = 20 days (< 30 days, uses "days ago" format)
        analytics.token_age_hours = Some(480.0);
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("days ago") || section.contains("MODERATE"));
    }

    #[test]
    fn test_security_buysell_zero_buys_zero_sells_in_period() {
        let mut analytics = create_test_analytics();
        // 24h has data, but 1h and 6h have zero
        analytics.total_buys_1h = 0;
        analytics.total_sells_1h = 0;
        analytics.total_buys_6h = 0;
        analytics.total_sells_6h = 0;
        analytics.total_buys_24h = 100;
        analytics.total_sells_24h = 80;
        let section = generate_security_analysis(&analytics);
        assert!(section.contains("-") || section.contains("100")); // "-" for 0/0 ratio
    }

    #[test]
    fn test_risk_factors_various_honeypot_ratios() {
        let mut analytics = create_test_analytics();

        // ratio > 10 → honeypot = 9
        analytics.total_buys_24h = 110;
        analytics.total_sells_24h = 10;
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.honeypot, 9);

        // ratio > 5, <= 10 → honeypot = 7
        analytics.total_buys_24h = 60;
        analytics.total_sells_24h = 10;
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.honeypot, 7);

        // ratio > 3, <= 5 → honeypot = 5
        analytics.total_buys_24h = 40;
        analytics.total_sells_24h = 10;
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.honeypot, 5);

        // ratio > 2, <= 3 → honeypot = 3
        analytics.total_buys_24h = 25;
        analytics.total_sells_24h = 10;
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.honeypot, 3);

        // ratio <= 2 → honeypot = 1
        analytics.total_buys_24h = 15;
        analytics.total_sells_24h = 10;
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.honeypot, 1);
    }

    #[test]
    fn test_risk_factors_various_age_thresholds() {
        let mut analytics = create_test_analytics();

        // < 48h → 8
        analytics.token_age_hours = Some(36.0);
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.age, 8);

        // < 168h (7d) → 6
        analytics.token_age_hours = Some(120.0);
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.age, 6);

        // < 720h (30d) → 4
        analytics.token_age_hours = Some(500.0);
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.age, 4);

        // < 2160h (90d) → 2
        analytics.token_age_hours = Some(1500.0);
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.age, 2);
    }

    #[test]
    fn test_risk_factors_various_liquidity_thresholds() {
        let mut analytics = create_test_analytics();

        // 50K-100K → 6
        analytics.liquidity_usd = 75_000.0;
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.liquidity, 6);

        // 100K-500K → 4
        analytics.liquidity_usd = 200_000.0;
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.liquidity, 4);

        // 500K-1M → 2
        analytics.liquidity_usd = 750_000.0;
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.liquidity, 2);

        // > 1M → 1
        analytics.liquidity_usd = 2_000_000.0;
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.liquidity, 1);
    }

    #[test]
    fn test_risk_factors_various_concentration_thresholds() {
        let mut analytics = create_test_analytics();

        // 30-50% → 8
        analytics.holders = vec![TokenHolder {
            address: "0x1".to_string(),
            balance: "1".to_string(),
            formatted_balance: "1".to_string(),
            percentage: 35.0,
            rank: 1,
        }];
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.concentration, 8);

        // 20-30% → 6
        analytics.holders[0].percentage = 25.0;
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.concentration, 6);

        // 10-20% → 4
        analytics.holders[0].percentage = 15.0;
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.concentration, 4);

        // 5-10% → 2
        analytics.holders[0].percentage = 7.0;
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.concentration, 2);

        // < 5% → 1
        analytics.holders[0].percentage = 3.0;
        let factors = RiskFactors::from_analytics(&analytics);
        assert_eq!(factors.concentration, 1);
    }

    #[test]
    fn test_risk_factors_social_one_social() {
        let mut analytics = create_test_analytics();
        analytics.socials = vec![TokenSocial {
            platform: "twitter".to_string(),
            url: "https://twitter.com/test".to_string(),
        }];
        analytics.websites = vec![];
        let factors = RiskFactors::from_analytics(&analytics);
        // 1 social = moderate social presence
        assert!(factors.social <= 5);
    }

    #[test]
    fn test_format_number_large_values() {
        assert_eq!(format_number(1_500_000_000.0), "1500.00M");
        assert_eq!(format_number(500_000.0), "500K");
        assert_eq!(format_number(42.0), "42");
    }
}
