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
use crate::market::{
    BinanceClient, HealthThresholds, MarketSummary, MarketVenue, order_book_from_analytics,
};
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
    let sp = crate::cli::progress::Spinner::new("Fetching token health data...");
    let analytics =
        crawl::fetch_analytics_for_input(&args.token, &args.chain, Period::Hour24, 10, clients)
            .await?;

    // 2. Optionally fetch market summary for stablecoin
    let market_summary = if args.with_market {
        sp.set_message("Fetching market data...");
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
                    Ok(book) => {
                        let volume_24h = match args.market_venue {
                            MarketVenue::Binance => BinanceClient::default_url()
                                .fetch_24h_volume(&pair)
                                .await
                                .ok()
                                .flatten(),
                            _ => None,
                        };
                        Some(MarketSummary::from_order_book(
                            &book,
                            1.0,
                            &thresholds,
                            volume_24h,
                        ))
                    }
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
                let volume_24h = Some(best_pair.volume_24h);
                Some(MarketSummary::from_order_book(
                    &book,
                    1.0,
                    &thresholds,
                    volume_24h,
                ))
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

    sp.finish("Token health data loaded.");

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
        crate::display::format_large_number(analytics.volume_24h)
    );
    println!(
        "Liquidity:       ${}",
        crate::display::format_large_number(analytics.liquidity_usd)
    );
    if let Some(mc) = analytics.market_cap {
        println!("Market Cap:      ${}", crate::display::format_large_number(mc));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::dex::DexTokenData;
    use crate::chains::mocks::MockClientFactory;
    use crate::chains::{DexPair, Token, TokenAnalytics, TokenHolder, TokenSocial};
    use crate::config::OutputFormat;
    use crate::market::{HealthCheck, MarketSummary};

    fn make_test_analytics(with_dex_pairs: bool) -> TokenAnalytics {
        TokenAnalytics {
            token: Token {
                contract_address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                decimals: 6,
            },
            chain: "ethereum".to_string(),
            holders: vec![TokenHolder {
                address: "0x1234".to_string(),
                balance: "1000000".to_string(),
                formatted_balance: "1.0".to_string(),
                percentage: 10.0,
                rank: 1,
            }],
            total_holders: 1000,
            volume_24h: 5_000_000.0,
            volume_7d: 25_000_000.0,
            price_usd: 0.9999,
            price_change_24h: -0.01,
            price_change_7d: 0.02,
            liquidity_usd: 100_000_000.0,
            market_cap: Some(30_000_000_000.0),
            fdv: None,
            total_supply: None,
            circulating_supply: None,
            price_history: vec![],
            volume_history: vec![],
            holder_history: vec![],
            dex_pairs: if with_dex_pairs {
                vec![DexPair {
                    dex_name: "Uniswap V3".to_string(),
                    pair_address: "0xpair".to_string(),
                    base_token: "USDC".to_string(),
                    quote_token: "WETH".to_string(),
                    price_usd: 0.9999,
                    volume_24h: 5_000_000.0,
                    liquidity_usd: 50_000_000.0,
                    price_change_24h: -0.01,
                    buys_24h: 1000,
                    sells_24h: 900,
                    buys_6h: 300,
                    sells_6h: 250,
                    buys_1h: 50,
                    sells_1h: 45,
                    pair_created_at: Some(1600000000),
                    url: Some("https://dexscreener.com/ethereum/0xpair".to_string()),
                }]
            } else {
                vec![]
            },
            fetched_at: 1700003600,
            top_10_concentration: Some(35.5),
            top_50_concentration: Some(55.0),
            top_100_concentration: Some(65.0),
            price_change_6h: 0.01,
            price_change_1h: -0.005,
            total_buys_24h: 1000,
            total_sells_24h: 900,
            total_buys_6h: 300,
            total_sells_6h: 250,
            total_buys_1h: 50,
            total_sells_1h: 45,
            token_age_hours: Some(25000.0),
            image_url: None,
            websites: vec!["https://centre.io".to_string()],
            socials: vec![TokenSocial {
                platform: "twitter".to_string(),
                url: "https://twitter.com/circle".to_string(),
            }],
            dexscreener_url: Some("https://dexscreener.com/ethereum/0xpair".to_string()),
        }
    }

    fn make_test_market_summary() -> MarketSummary {
        use crate::market::{ExecutionEstimate, ExecutionSide};
        MarketSummary {
            pair: "USDC/USDT".to_string(),
            peg_target: 1.0,
            best_bid: Some(0.9999),
            best_ask: Some(1.0001),
            mid_price: Some(1.0),
            spread: Some(0.0002),
            volume_24h: Some(1_000_000.0),
            execution_10k_buy: Some(ExecutionEstimate {
                notional_usdt: 10_000.0,
                side: ExecutionSide::Buy,
                vwap: 1.0001,
                slippage_bps: 1.0,
                fillable: true,
            }),
            execution_10k_sell: Some(ExecutionEstimate {
                notional_usdt: 10_000.0,
                side: ExecutionSide::Sell,
                vwap: 0.9999,
                slippage_bps: 1.0,
                fillable: true,
            }),
            asks: vec![],
            bids: vec![],
            ask_outliers: 0,
            bid_outliers: 0,
            ask_depth: 5000.0,
            bid_depth: 6000.0,
            checks: vec![
                HealthCheck::Pass("No sells below peg".to_string()),
                HealthCheck::Pass("Bid/Ask ratio: 1.20x".to_string()),
            ],
            healthy: true,
        }
    }

    #[test]
    fn test_format_venue() {
        assert_eq!(format_venue(MarketVenue::Binance), "Binance Spot");
        assert_eq!(format_venue(MarketVenue::Biconomy), "Biconomy");
        assert_eq!(format_venue(MarketVenue::Ethereum), "Ethereum DEX");
        assert_eq!(format_venue(MarketVenue::Solana), "Solana DEX");
    }

    #[test]
    fn test_format_large_number() {
        assert_eq!(crate::display::format_large_number(1_500_000_000.0), "1.50B");
        assert_eq!(crate::display::format_large_number(2_500_000.0), "2.50M");
        assert_eq!(crate::display::format_large_number(3_500.0), "3.50K");
        assert_eq!(crate::display::format_large_number(99.99), "99.99");
    }

    #[test]
    fn test_token_health_to_markdown_without_market() {
        let analytics = make_test_analytics(false);
        let md = token_health_to_markdown(&analytics, None, None);
        assert!(md.contains("USDC"));
        assert!(md.contains("USD Coin"));
        assert!(!md.contains("Market / Order Book"));
    }

    #[test]
    fn test_token_health_to_markdown_with_market() {
        let analytics = make_test_analytics(false);
        let market = make_test_market_summary();
        let md = token_health_to_markdown(&analytics, Some(&market), Some(MarketVenue::Binance));
        assert!(md.contains("Market / Order Book"));
        assert!(md.contains("Binance Spot"));
        assert!(md.contains("0.9999"));
        assert!(md.contains("Yes"));
        assert!(md.contains("Health Checks"));
    }

    #[test]
    fn test_token_health_to_json_without_market() {
        let analytics = make_test_analytics(false);
        let json = token_health_to_json(&analytics, None).unwrap();
        assert!(json.contains("\"analytics\""));
        assert!(json.contains("\"market\": null"));
        assert!(json.contains("USDC"));
    }

    #[test]
    fn test_token_health_to_json_with_market() {
        let analytics = make_test_analytics(false);
        let market = make_test_market_summary();
        let json = token_health_to_json(&analytics, Some(&market)).unwrap();
        assert!(json.contains("\"market\""));
        assert!(json.contains("\"peg_target\": 1.0"));
        assert!(json.contains("\"healthy\": true"));
    }

    #[test]
    fn test_output_token_health_table_without_market() {
        let analytics = make_test_analytics(false);
        let result = output_token_health_table(&analytics, None, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_token_health_table_with_market() {
        let analytics = make_test_analytics(false);
        let market = make_test_market_summary();
        let result =
            output_token_health_table(&analytics, Some(&market), Some(MarketVenue::Biconomy));
        assert!(result.is_ok());
    }

    fn make_test_dex_token_data(pairs: Vec<DexPair>) -> DexTokenData {
        DexTokenData {
            address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            symbol: "USDC".to_string(),
            name: "USD Coin".to_string(),
            price_usd: 0.9999,
            price_change_24h: -0.01,
            price_change_6h: 0.01,
            price_change_1h: -0.005,
            price_change_5m: 0.0,
            volume_24h: 5_000_000.0,
            volume_6h: 1_250_000.0,
            volume_1h: 250_000.0,
            liquidity_usd: 100_000_000.0,
            market_cap: Some(30_000_000_000.0),
            fdv: Some(30_000_000_000.0),
            pairs,
            price_history: vec![],
            volume_history: vec![],
            total_buys_24h: 1000,
            total_sells_24h: 900,
            total_buys_6h: 300,
            total_sells_6h: 250,
            total_buys_1h: 50,
            total_sells_1h: 45,
            earliest_pair_created_at: Some(1600000000),
            image_url: None,
            websites: vec![],
            socials: vec![crate::chains::dex::TokenSocial {
                platform: "twitter".to_string(),
                url: "https://twitter.com/circle".to_string(),
            }],
            dexscreener_url: None,
        }
    }

    #[tokio::test]
    async fn test_run_token_health_table() {
        let mut factory = MockClientFactory::new();
        factory.mock_dex.token_data = Some(make_test_dex_token_data(vec![]));

        let config = Config::default();
        let args = TokenHealthArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            with_market: false,
            market_venue: MarketVenue::Binance,
            format: OutputFormat::Table,
        };

        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_token_health_json() {
        let mut factory = MockClientFactory::new();
        let mut data = make_test_dex_token_data(vec![]);
        data.price_usd = 1.0;
        data.volume_24h = 1_000_000.0;
        data.liquidity_usd = 5_000_000.0;
        factory.mock_dex.token_data = Some(data);

        let config = Config::default();
        let args = TokenHealthArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            with_market: false,
            market_venue: MarketVenue::Binance,
            format: OutputFormat::Json,
        };

        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_token_health_markdown() {
        let mut factory = MockClientFactory::new();
        factory.mock_dex.token_data = Some(make_test_dex_token_data(vec![]));

        let config = Config::default();
        let args = TokenHealthArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            with_market: false,
            market_venue: MarketVenue::Binance,
            format: OutputFormat::Markdown,
        };

        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
    }

    /// Test DEX venue with dex_pairs: synthesizes order book from analytics.
    #[tokio::test]
    async fn test_run_token_health_dex_market() {
        let mut factory = MockClientFactory::new();
        let pair = DexPair {
            dex_name: "Uniswap V3".to_string(),
            pair_address: "0xpair".to_string(),
            base_token: "USDC".to_string(),
            quote_token: "WETH".to_string(),
            price_usd: 0.9999,
            volume_24h: 5_000_000.0,
            liquidity_usd: 50_000_000.0,
            price_change_24h: -0.01,
            buys_24h: 1000,
            sells_24h: 900,
            buys_6h: 300,
            sells_6h: 250,
            buys_1h: 50,
            sells_1h: 45,
            pair_created_at: Some(1600000000),
            url: Some("https://dexscreener.com/ethereum/0xpair".to_string()),
        };
        factory.mock_dex.token_data = Some(make_test_dex_token_data(vec![pair]));

        let config = Config::default();
        let args = TokenHealthArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            with_market: true,
            market_venue: MarketVenue::Ethereum,
            format: OutputFormat::Table,
        };

        let result = run(args, &config, &factory).await;
        assert!(result.is_ok());
    }
}
