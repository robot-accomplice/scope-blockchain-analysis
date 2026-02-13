//! # Token Health Command
//!
//! Composite command combining DEX analytics (crawl) with optional market/order book
//! summary for stablecoins. Produces a unified health report: liquidity, volume,
//! peg deviation, and order book depth.

use crate::chains::{ChainClientFactory, TokenAnalytics};
use crate::cli::crawl::{self, Period};
use crate::config::Config;
use crate::display::report;
use crate::error::{Result, ScopeError};
use crate::market::{HealthThresholds, MarketSummary, MarketVenue, order_book_from_analytics};
use clap::Args;

/// Arguments for the token-health command.
#[derive(Debug, Args)]
pub struct TokenHealthArgs {
    /// Token symbol or contract address (e.g., USDC, 0xA0b86991...).
    pub token: String,

    /// Target blockchain network.
    #[arg(short, long, default_value = "ethereum")]
    pub chain: String,

    /// Include order book / market summary (for stablecoins).
    #[arg(long)]
    pub with_market: bool,

    /// Market venue for order book data: binance, biconomy, eth, solana.
    /// CEX: binance=USDCUSDT, biconomy=USDC_USDT. DEX (eth/solana): uses chain liquidity.
    #[arg(long, default_value = "binance")]
    pub market_venue: MarketVenue,

    /// Output format.
    #[arg(short, long, default_value = "table")]
    pub format: crate::config::OutputFormat,
}

/// Runs the token-health command.
pub async fn run(
    args: TokenHealthArgs,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    // --ai sets config.output.format to Markdown; respect that override
    let format = if config.output.format == crate::config::OutputFormat::Markdown {
        config.output.format
    } else {
        args.format
    };
    // 1. Fetch DEX analytics (crawl)
    let analytics =
        crawl::fetch_analytics_for_input(&args.token, &args.chain, Period::Hour24, 10, clients)
            .await?;

    // 2. Optionally fetch market summary for stablecoin
    let market_summary = if args.with_market {
        let thresholds = HealthThresholds {
            peg_target: 1.0,
            peg_range: 0.001,
            min_levels: 6,
            min_depth: 3000.0,
            min_bid_ask_ratio: 0.2,
            max_bid_ask_ratio: 5.0,
        };
        if args.market_venue.is_cex() {
            let pair = args.market_venue.format_pair(&analytics.token.symbol);
            if let Some(client) = args.market_venue.create_client() {
                match client.fetch_order_book(&pair).await {
                    Ok(book) => Some(MarketSummary::from_order_book(&book, 1.0, &thresholds)),
                    Err(e) => {
                        tracing::warn!(
                            "Market data unavailable for {} on {:?}: {}",
                            pair,
                            args.market_venue,
                            e
                        );
                        None
                    }
                }
            } else {
                None
            }
        } else {
            // DEX venues: synthesize from analytics (only when chain matches venue)
            let venue_chain = match args.market_venue {
                MarketVenue::Ethereum => "ethereum",
                MarketVenue::Solana => "solana",
                _ => &analytics.chain,
            };
            if analytics.chain.eq_ignore_ascii_case(venue_chain) && !analytics.dex_pairs.is_empty()
            {
                let best_pair = analytics
                    .dex_pairs
                    .iter()
                    .max_by(|a, b| {
                        a.liquidity_usd
                            .partial_cmp(&b.liquidity_usd)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap(); // safe: we checked !is_empty
                let book =
                    order_book_from_analytics(&analytics.chain, best_pair, &analytics.token.symbol);
                Some(MarketSummary::from_order_book(&book, 1.0, &thresholds))
            } else {
                if analytics.chain.ne(venue_chain) {
                    tracing::warn!(
                        "DEX venue {:?} requires --chain {}; got {}. Use matching chain for DEX depth.",
                        args.market_venue,
                        venue_chain,
                        analytics.chain
                    );
                } else if analytics.dex_pairs.is_empty() {
                    tracing::warn!(
                        "No DEX pairs found for {} on {}",
                        analytics.token.symbol,
                        analytics.chain
                    );
                }
                None
            }
        }
    } else {
        None
    };

    // 3. Output combined report
    match format {
        crate::config::OutputFormat::Markdown => {
            let venue_label = args.with_market.then_some(args.market_venue);
            let md = token_health_to_markdown(&analytics, market_summary.as_ref(), venue_label);
            println!("{}", md);
        }
        crate::config::OutputFormat::Json => {
            let json = token_health_to_json(&analytics, market_summary.as_ref())?;
            println!("{}", json);
        }
        crate::config::OutputFormat::Table | crate::config::OutputFormat::Csv => {
            let venue_label = args.with_market.then_some(args.market_venue);
            output_token_health_table(&analytics, market_summary.as_ref(), venue_label)?;
        }
    }

    Ok(())
}

fn token_health_to_markdown(
    analytics: &TokenAnalytics,
    market: Option<&MarketSummary>,
    venue: Option<MarketVenue>,
) -> String {
    // Use crawl's full report as base
    let mut md = report::generate_report(analytics);

    if let Some(summary) = market {
        md.push_str("\n---\n\n");
        md.push_str("## Market / Order Book\n\n");
        if let Some(v) = venue {
            md.push_str(&format!("**Venue:** {}  \n\n", format_venue(v)));
        }
        md.push_str(&format!(
            "| Metric | Value |\n|--------|-------|\n\
             | Peg Target | {:.4} |\n\
             | Best Bid | {} |\n\
             | Best Ask | {} |\n\
             | Mid Price | {} |\n\
             | Spread | {} |\n\
             | Bid Depth | {:.0} |\n\
             | Ask Depth | {:.0} |\n\
             | Healthy | {} |\n",
            summary.peg_target,
            summary
                .best_bid
                .map(|b| format!("{:.4}", b))
                .unwrap_or_else(|| "-".to_string()),
            summary
                .best_ask
                .map(|a| format!("{:.4}", a))
                .unwrap_or_else(|| "-".to_string()),
            summary
                .mid_price
                .map(|m| format!("{:.4}", m))
                .unwrap_or_else(|| "-".to_string()),
            summary
                .spread
                .map(|s| format!("{:.4}", s))
                .unwrap_or_else(|| "-".to_string()),
            summary.bid_depth,
            summary.ask_depth,
            if summary.healthy { "Yes" } else { "No" }
        ));
        if !summary.checks.is_empty() {
            md.push_str("\n**Health Checks:**\n");
            for check in &summary.checks {
                let (icon, msg) = match check {
                    crate::market::HealthCheck::Pass(m) => ("✓", m.as_str()),
                    crate::market::HealthCheck::Fail(m) => ("✗", m.as_str()),
                };
                md.push_str(&format!("- {} {}\n", icon, msg));
            }
        }
    }

    md.push_str(&report::report_footer());
    md
}

fn token_health_to_json(
    analytics: &TokenAnalytics,
    market: Option<&MarketSummary>,
) -> Result<String> {
    let market_json = market.map(|m| {
        serde_json::json!({
            "peg_target": m.peg_target,
            "best_bid": m.best_bid,
            "best_ask": m.best_ask,
            "mid_price": m.mid_price,
            "spread": m.spread,
            "bid_depth": m.bid_depth,
            "ask_depth": m.ask_depth,
            "healthy": m.healthy,
            "checks": m.checks.iter().map(|c| match c {
                crate::market::HealthCheck::Pass(msg) => serde_json::json!({"status": "pass", "message": msg}),
                crate::market::HealthCheck::Fail(msg) => serde_json::json!({"status": "fail", "message": msg}),
            }).collect::<Vec<_>>()
        })
    });
    let json = serde_json::json!({
        "analytics": analytics,
        "market": market_json
    });
    serde_json::to_string_pretty(&json).map_err(|e| ScopeError::Other(e.to_string()))
}

fn format_venue(venue: MarketVenue) -> &'static str {
    match venue {
        MarketVenue::Binance => "Binance Spot",
        MarketVenue::Biconomy => "Biconomy",
        MarketVenue::Ethereum => "Ethereum DEX",
        MarketVenue::Solana => "Solana DEX",
    }
}

fn output_token_health_table(
    analytics: &TokenAnalytics,
    market: Option<&MarketSummary>,
    venue: Option<MarketVenue>,
) -> Result<()> {
    // DEX section
    println!(
        "\n# Token Health: {} ({})\n",
        analytics.token.symbol, analytics.token.name
    );
    println!("## DEX Analytics");
    println!("{}", "=".repeat(50));
    println!("Price:           ${:.6}", analytics.price_usd);
    println!("24h Change:      {:+.2}%", analytics.price_change_24h);
    println!(
        "24h Volume:      ${}",
        format_large_number(analytics.volume_24h)
    );
    println!(
        "Liquidity:       ${}",
        format_large_number(analytics.liquidity_usd)
    );
    if let Some(mc) = analytics.market_cap {
        println!("Market Cap:      ${}", format_large_number(mc));
    }
    if let Some(top10) = analytics.top_10_concentration {
        println!("Top 10 Holders:  {:.1}%", top10);
    }

    if let Some(summary) = market {
        println!();
        println!("## Market / Order Book");
        println!("{}", "=".repeat(50));
        if let Some(v) = venue {
            println!("Venue:           {}", format_venue(v));
        }
        println!("Peg Target:      {:.4}", summary.peg_target);
        if let Some(b) = summary.best_bid {
            println!("Best Bid:        {:.4}", b);
        }
        if let Some(a) = summary.best_ask {
            println!("Best Ask:        {:.4}", a);
        }
        if let Some(m) = summary.mid_price {
            println!("Mid Price:       {:.4}", m);
        }
        println!("Bid Depth:       {:.0}", summary.bid_depth);
        println!("Ask Depth:       {:.0}", summary.ask_depth);
        println!(
            "Healthy:         {}",
            if summary.healthy { "Yes" } else { "No" }
        );
        for check in &summary.checks {
            let (icon, msg) = match check {
                crate::market::HealthCheck::Pass(m) => ("✓", m.as_str()),
                crate::market::HealthCheck::Fail(m) => ("✗", m.as_str()),
            };
            println!("  {} {}", icon, msg);
        }
    }

    println!();
    Ok(())
}

fn format_large_number(value: f64) -> String {
    if value >= 1_000_000_000.0 {
        format!("{:.2}B", value / 1_000_000_000.0)
    } else if value >= 1_000_000.0 {
        format!("{:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.2}K", value / 1_000.0)
    } else {
        format!("{:.2}", value)
    }
}
