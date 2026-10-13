//! # Token Health Command
//!
//! Composite command combining DEX analytics (crawl) with optional market/order book
//! summary for stablecoins. Produces a unified health report: liquidity, volume,
//! peg deviation, and order book depth.

use crate::cli::crawl::{self, Period};
use crate::cli::output::Output;
use crate::outln;
use clap::Args;
use scope::chains::{ChainClientFactory, TokenAnalytics};
use scope::config::Config;

use super::health_args::HealthArgs;
use scope::display::report;
use scope::error::{Result, ScopeError};
use scope::market::{MarketSummary, MarketVenue, fetch_token_summary};

/// Arguments for the token-health command.
#[derive(Debug, Args)]
#[command(
    after_help = "\x1b[1mExamples:\x1b[0m
  scope token-health USDC
  scope token-health @dai-token --with-market             \x1b[2m# address book shortcut\x1b[0m
  scope token-health DAI --with-market --venue binance
  scope token-health 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --format json",
    after_long_help = "\x1b[1mExamples:\x1b[0m

  \x1b[1m$ scope token-health USDC\x1b[0m

  +-- USDC (USD Coin) ---------------------------------+
  |                                                     |
  |-- DEX Analytics                                     |
  |  Price              $0.9999                         |
  |  24h Change         -0.01%                          |
  |  24h Volume         $5.00M                          |
  |  Liquidity          $100.00M                        |
  |  Market Cap         $30.00B                         |
  |  Top 10 Holders     35.5%                           |
  +-----------------------------------------------------+

  \x1b[1m$ scope token-health DAI --with-market --venue binance\x1b[0m

  +-- DAI (Dai Stablecoin) -----------------------------+
  |                                                     |
  |-- DEX Analytics                                     |
  |  Price              $0.9999                         |
  |  24h Change         +0.02%                          |
  |  24h Volume         $50.00K                         |
  |  Liquidity          $250.00K                        |
  |                                                     |
  |-- Market / Order Book                               |
  |  Venue              binance                        |
  |  Best Bid           0.9999                          |
  |  Best Ask           1.0001                          |
  |  Bid Depth          6000 USDT                       |
  |  Ask Depth          5000 USDT                       |
  |                                                     |
  |  + No sells below peg                               |
  |  + Bid/Ask ratio: 1.20x                             |
  |                                                     |
  |  HEALTHY                                            |
  +-----------------------------------------------------+"
)]
pub struct TokenHealthArgs {
    /// Token symbol or contract address (e.g., USDC, 0xA0b86991...).
    /// Use @label to resolve from the address book (e.g., @dai-token).
    pub token: String,

    /// Target blockchain network.
    #[arg(short, long, default_value = "ethereum")]
    pub chain: String,

    /// Include order book / market summary (for stablecoins).
    #[arg(long)]
    pub with_market: bool,

    #[arg(long, default_value = scope::market::DEFAULT_VENUE, help = scope::market::VENUE_HELP)]
    pub venue: String,

    /// Output format.
    #[arg(short, long, default_value = "table")]
    pub format: scope::config::OutputFormat,

    /// Health threshold overrides for --with-market.
    #[command(flatten)]
    pub health: HealthArgs,
}

/// Runs the token-health command.
pub async fn run(
    mut args: TokenHealthArgs,
    config: &Config,
    clients: &dyn ChainClientFactory,
    out: &Output,
) -> Result<()> {
    // Resolve address book label → address + chain
    if let Some((address, chain)) =
        crate::cli::address_book::resolve_address_book_input(&args.token, config)?
    {
        args.token = address;
        if args.chain == "ethereum" {
            args.chain = chain;
        }
    }

    // --ai sets config.output.format to Markdown; respect that override
    let format = if config.output.format == scope::config::OutputFormat::Markdown {
        config.output.format
    } else {
        args.format
    };
    // 1. Fetch DEX analytics (crawl)
    let sp = crate::cli::progress::Spinner::new("Fetching token health data...", out);
    let analytics = crawl::fetch_analytics_for_input(
        &args.token,
        &args.chain,
        Period::Hour24,
        10,
        clients,
        Some(&sp),
        out,
    )
    .await?;

    // 2. Optionally fetch market summary for stablecoin
    let market_summary = if args.with_market {
        sp.set_message("Fetching market data...");
        let thresholds = args.health.resolve(config);
        let summary = match MarketVenue::from_name(&args.venue) {
            Ok(venue) => fetch_token_summary(&venue, &analytics, &thresholds).await,
            Err(e) => Err(e),
        };
        match summary {
            Ok(summary) => Some(summary),
            Err(e) => {
                sp.println(&format!("  Warning: market data unavailable: {e}"));
                None
            }
        }
    } else {
        None
    };

    sp.finish("Token health data loaded.");

    // 3. Output combined report
    let venue_label = if args.with_market {
        Some(args.venue.as_str())
    } else {
        None
    };
    match format {
        scope::config::OutputFormat::Markdown => {
            let md = token_health_to_markdown(&analytics, market_summary.as_ref(), venue_label);
            outln!(out, "{}", md)?;
        }
        scope::config::OutputFormat::Json => {
            let json = token_health_to_json(&analytics, market_summary.as_ref())?;
            outln!(out, "{}", json)?;
        }
        scope::config::OutputFormat::Table | scope::config::OutputFormat::Csv => {
            output_token_health_table(&analytics, market_summary.as_ref(), venue_label, out)?;
        }
    }

    Ok(())
}

fn token_health_to_markdown(
    analytics: &TokenAnalytics,
    market: Option<&MarketSummary>,
    venue: Option<&str>,
) -> String {
    // Use crawl's full report as base
    let mut md = report::generate_report(analytics);

    if let Some(summary) = market {
        md.push_str("\n---\n\n");
        md.push_str("## Market / Order Book\n\n");
        md.push_str(&summary.format_markdown(venue));
    }

    md.push_str(&report::report_footer());
    md
}

fn token_health_to_json(
    analytics: &TokenAnalytics,
    market: Option<&MarketSummary>,
) -> Result<String> {
    let market_json = market.map(|m| m.to_json());
    let json = serde_json::json!({
        "analytics": analytics,
        "market": market_json
    });
    serde_json::to_string_pretty(&json).map_err(ScopeError::Json)
}

fn output_token_health_table(
    analytics: &TokenAnalytics,
    market: Option<&MarketSummary>,
    venue: Option<&str>,
    out: &Output,
) -> Result<()> {
    use scope::display::terminal as t;

    let title = format!("{} ({})", analytics.token.symbol, analytics.token.name);
    outln!(out, "{}", t::section_header(&title))?;

    // DEX Analytics subsection
    outln!(out, "{}", t::subsection_header("DEX Analytics"))?;
    outln!(
        out,
        "{}",
        t::kv_row("Price", &format!("${:.6}", analytics.price_usd))
    )?;
    outln!(
        out,
        "{}",
        t::kv_row_delta(
            "24h Change",
            analytics.price_change_24h,
            &format!("{:+.2}%", analytics.price_change_24h)
        )
    )?;
    outln!(
        out,
        "{}",
        t::kv_row(
            "24h Volume",
            &format!(
                "${}",
                scope::display::format_large_number(analytics.volume_24h)
            )
        )
    )?;
    outln!(
        out,
        "{}",
        t::kv_row(
            "Liquidity",
            &format!(
                "${}",
                scope::display::format_large_number(analytics.liquidity_usd)
            )
        )
    )?;
    if let Some(mc) = analytics.market_cap {
        outln!(
            out,
            "{}",
            t::kv_row(
                "Market Cap",
                &format!("${}", scope::display::format_large_number(mc))
            )
        )?;
    }
    if let Some(top10) = analytics.top_10_concentration {
        outln!(
            out,
            "{}",
            t::kv_row("Top 10 Holders", &format!("{:.1}%", top10))
        )?;
    }

    // Market / Order Book subsection
    if let Some(summary) = market {
        outln!(out, "{}", t::subsection_header("Market / Order Book"))?;
        if let Some(v) = venue {
            outln!(out, "{}", t::kv_row("Venue", v))?;
        }
        outln!(
            out,
            "{}",
            t::kv_row("Peg Target", &format!("{:.4}", summary.peg_target))
        )?;
        if let Some(b) = summary.best_bid {
            outln!(
                out,
                "{}",
                t::kv_row("Best Bid", &t::format_price_peg(b, &summary.peg_bands()))
            )?;
        }
        if let Some(a) = summary.best_ask {
            outln!(
                out,
                "{}",
                t::kv_row("Best Ask", &t::format_price_peg(a, &summary.peg_bands()))
            )?;
        }
        if let Some(m) = summary.mid_price {
            outln!(
                out,
                "{}",
                t::kv_row("Mid Price", &t::format_price_peg(m, &summary.peg_bands()))
            )?;
        }
        outln!(
            out,
            "{}",
            t::kv_row("Bid Depth", &format!("{:.0} USDT", summary.bid_depth))
        )?;
        outln!(
            out,
            "{}",
            t::kv_row("Ask Depth", &format!("{:.0} USDT", summary.ask_depth))
        )?;
        outln!(out, "{}", t::blank_row())?;

        // Health checks
        for check in &summary.checks {
            outln!(out, "{}", check.render_text())?;
        }
        outln!(out, "{}", t::blank_row())?;
        outln!(out, "{}", t::status_line(summary.healthy))?;
    }

    outln!(out, "{}", t::section_footer())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope::chains::dex::DexTokenData;
    use scope::chains::mocks::MockClientFactory;
    use scope::chains::{DexPair, Token, TokenAnalytics, TokenHolder, TokenSocial};
    use scope::config::OutputFormat;
    use scope::market::DexVenue;
    use scope::market::{HealthCheck, MarketSummary};

    fn quiet() -> crate::cli::output::Output {
        crate::cli::output::Output::capture().0
    }

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
                    liquidity_base: None,
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
        use scope::market::{ExecutionEstimate, ExecutionSide};
        MarketSummary {
            pair: "USDC/USDT".to_string(),
            source: scope::market::BookSource::Exchange,
            peg_target: 1.0,
            peg_range: scope::market::health::DEFAULT_PEG_RANGE,
            best_bid: Some(0.9999),
            best_ask: Some(1.0001),
            mid_price: Some(1.0),
            spread: Some(0.0002),
            spread_pct: None,
            top_depth: Default::default(),
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
    fn test_is_dex_venue() {
        assert!(DexVenue::parse("eth").is_some());
        assert!(DexVenue::parse("ethereum").is_some());
        assert!(DexVenue::parse("solana").is_some());
        assert!(DexVenue::parse("binance").is_none());
        assert!(DexVenue::parse("okx").is_none());
    }

    #[test]
    fn test_is_dex_venue_values() {
        assert!(DexVenue::parse("eth").is_some());
        assert!(DexVenue::parse("ethereum").is_some());
        assert!(DexVenue::parse("solana").is_some());
        assert!(DexVenue::parse("binance").is_none());
        assert!(DexVenue::parse("mexc").is_none());
    }

    #[test]
    fn test_dex_venue_to_chain_values() {
        assert_eq!(
            DexVenue::parse("eth").map(DexVenue::chain),
            Some("ethereum")
        );
        assert_eq!(
            DexVenue::parse("ethereum").map(DexVenue::chain),
            Some("ethereum")
        );
        assert_eq!(
            DexVenue::parse("solana").map(DexVenue::chain),
            Some("solana")
        );
        // An unknown name is not a DEX venue; it used to map to ethereum.
        assert_eq!(DexVenue::parse("unknown"), None);
    }

    #[test]
    fn test_token_health_args_debug() {
        let args = TokenHealthArgs {
            token: "USDC".to_string(),
            chain: "ethereum".to_string(),
            with_market: false,
            venue: "binance".to_string(),
            format: scope::config::OutputFormat::Table,
            health: Default::default(),
        };
        let debug = format!("{:?}", args);
        assert!(debug.contains("TokenHealthArgs"));
    }

    #[test]
    fn test_format_large_number() {
        assert_eq!(
            scope::display::format_large_number(1_500_000_000.0),
            "1.50B"
        );
        assert_eq!(scope::display::format_large_number(2_500_000.0), "2.50M");
        assert_eq!(scope::display::format_large_number(3_500.0), "3.50K");
        assert_eq!(scope::display::format_large_number(99.99), "99.99");
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
        let md = token_health_to_markdown(&analytics, Some(&market), Some("binance"));
        assert!(md.contains("Market / Order Book"));
        assert!(md.contains("binance"));
        assert!(md.contains("0.9999"));
        assert!(md.contains("| Healthy | ✓ |"));
        assert!(md.contains("Health Checks"));
    }

    #[test]
    fn test_token_health_to_markdown_without_venue() {
        let analytics = make_test_analytics(false);
        let market = make_test_market_summary();
        let md = token_health_to_markdown(&analytics, Some(&market), None);
        assert!(md.contains("Market / Order Book"));
        assert!(!md.contains("Venue:")); // Should not include venue when None
        assert!(md.contains("0.9999"));
        assert!(md.contains("| Healthy | ✓ |"));
    }

    #[test]
    fn test_token_health_to_markdown_unhealthy_market() {
        let analytics = make_test_analytics(false);
        let mut market = make_test_market_summary();
        market.healthy = false;
        market.checks = vec![
            HealthCheck::Pass("Some check passed".to_string()),
            HealthCheck::Fail("Peg deviation too high".to_string()),
            HealthCheck::Fail("Insufficient bid depth".to_string()),
        ];
        let md = token_health_to_markdown(&analytics, Some(&market), Some("binance"));
        assert!(md.contains("Market / Order Book"));
        assert!(md.contains("| Healthy | ✗ |")); // Should show unhealthy
        assert!(md.contains("Health Checks"));
        assert!(md.contains("✗")); // Should show fail checks
        assert!(md.contains("Peg deviation too high"));
        assert!(md.contains("Insufficient bid depth"));
    }

    /// token-health and market share one chain rule: a DEX venue never builds
    /// a book from another chain's pools, and `sol` names the Solana DEX.
    #[tokio::test]
    async fn test_dex_venue_on_another_chain_is_an_error() {
        let analytics = make_test_analytics(true);
        let t = scope::market::HealthThresholds::default();
        let err = fetch_token_summary(&MarketVenue::from_name("sol").unwrap(), &analytics, &t)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("reads chain solana"), "{err}");
        assert!(
            fetch_token_summary(&MarketVenue::from_name("ETH").unwrap(), &analytics, &t)
                .await
                .is_ok()
        );
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
    fn test_token_health_to_json_with_fail_checks() {
        let analytics = make_test_analytics(false);
        let mut market = make_test_market_summary();
        market.healthy = false;
        market.checks = vec![
            HealthCheck::Pass("Bid/Ask ratio OK".to_string()),
            HealthCheck::Fail("Peg deviation exceeds threshold".to_string()),
            HealthCheck::Fail("Ask depth below minimum".to_string()),
        ];
        let json = token_health_to_json(&analytics, Some(&market)).unwrap();
        assert!(json.contains("\"market\""));
        assert!(json.contains("\"healthy\": false"));
        assert!(json.contains("\"status\": \"pass\""));
        assert!(json.contains("\"status\": \"fail\""));
        assert!(json.contains("Peg deviation exceeds threshold"));
        assert!(json.contains("Ask depth below minimum"));
    }

    #[test]
    fn test_output_token_health_table_without_market() {
        let analytics = make_test_analytics(false);
        let result = output_token_health_table(&analytics, None, None, &quiet());
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_token_health_table_with_market() {
        let analytics = make_test_analytics(false);
        let market = make_test_market_summary();
        let result =
            output_token_health_table(&analytics, Some(&market), Some("biconomy"), &quiet());
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_token_health_table_no_market_cap() {
        let mut analytics = make_test_analytics(false);
        analytics.market_cap = None;
        analytics.top_10_concentration = None;
        let result = output_token_health_table(&analytics, None, None, &quiet());
        assert!(result.is_ok());
        // Should not panic when market_cap and top_10_concentration are None
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
            socials: vec![scope::chains::dex::TokenSocial {
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
            venue: "binance".to_string(),
            format: OutputFormat::Table,
            health: Default::default(),
        };

        let result = run(args, &config, &factory, &quiet()).await;
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
            venue: "binance".to_string(),
            format: OutputFormat::Json,
            health: Default::default(),
        };

        let result = run(args, &config, &factory, &quiet()).await;
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
            venue: "binance".to_string(),
            format: OutputFormat::Markdown,
            health: Default::default(),
        };

        let result = run(args, &config, &factory, &quiet()).await;
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
            liquidity_base: None,
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
            venue: "eth".to_string(),
            format: OutputFormat::Table,
            health: Default::default(),
        };

        let result = run(args, &config, &factory, &quiet()).await;
        assert!(result.is_ok());
    }

    fn dex_market_args(venue: &str) -> TokenHealthArgs {
        TokenHealthArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            with_market: true,
            venue: venue.to_string(),
            format: OutputFormat::Json,
            health: Default::default(),
        }
    }

    fn pair_with_liquidity(liquidity_usd: f64) -> DexPair {
        DexPair {
            dex_name: "Uniswap V3".to_string(),
            pair_address: format!("0xpair{liquidity_usd}"),
            base_token: "USDC".to_string(),
            quote_token: "USDT".to_string(),
            price_usd: 1.0,
            volume_24h: 1_000.0,
            liquidity_usd,
            liquidity_base: None,
            price_change_24h: 0.0,
            buys_24h: 0,
            sells_24h: 0,
            buys_6h: 0,
            sells_6h: 0,
            buys_1h: 0,
            sells_1h: 0,
            pair_created_at: None,
            url: None,
        }
    }

    /// Several pools: the market section picks the deepest one.
    #[tokio::test]
    async fn test_run_token_health_dex_market_multiple_pairs() {
        let mut factory = MockClientFactory::new();
        factory.mock_dex.token_data = Some(make_test_dex_token_data(vec![
            pair_with_liquidity(1_000.0),
            pair_with_liquidity(90_000.0),
            pair_with_liquidity(5_000.0),
        ]));
        let result = run(
            dex_market_args("eth"),
            &Config::default(),
            &factory,
            &quiet(),
        )
        .await;
        assert!(result.is_ok());
    }

    /// Missing market data only warns: the token report must still succeed.
    #[tokio::test]
    async fn test_run_token_health_dex_market_chain_mismatch_still_reports() {
        let mut factory = MockClientFactory::new();
        factory.mock_dex.token_data =
            Some(make_test_dex_token_data(vec![pair_with_liquidity(1_000.0)]));
        // Token is on ethereum; the solana venue cannot price it.
        let result = run(
            dex_market_args("solana"),
            &Config::default(),
            &factory,
            &quiet(),
        )
        .await;
        assert!(result.is_ok());
    }

    /// No DEX pools only warns: the token report must still succeed.
    #[tokio::test]
    async fn test_run_token_health_dex_market_no_pairs_still_reports() {
        let mut factory = MockClientFactory::new();
        factory.mock_dex.token_data = Some(make_test_dex_token_data(vec![]));
        let result = run(
            dex_market_args("eth"),
            &Config::default(),
            &factory,
            &quiet(),
        )
        .await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_dex_venue_to_chain_case_insensitive() {
        assert_eq!(
            DexVenue::parse("ETH").map(DexVenue::chain),
            Some("ethereum")
        );
        assert_eq!(
            DexVenue::parse("Ethereum").map(DexVenue::chain),
            Some("ethereum")
        );
        assert_eq!(
            DexVenue::parse("SOLANA").map(DexVenue::chain),
            Some("solana")
        );
    }

    #[test]
    fn test_is_dex_venue_uppercase() {
        assert!(DexVenue::parse("ETH").is_some());
        assert!(DexVenue::parse("SOLANA").is_some());
        assert!(DexVenue::parse("BINANCE").is_none());
    }

    #[test]
    fn test_token_health_to_markdown_market_missing_prices() {
        let analytics = make_test_analytics(false);
        let mut market = make_test_market_summary();
        market.best_bid = None;
        market.best_ask = None;
        market.mid_price = None;
        market.spread = None;
        let md = token_health_to_markdown(&analytics, Some(&market), Some("binance"));
        assert!(md.contains("Market / Order Book"));
        assert!(md.contains("-")); // Dash for missing prices in table
    }

    #[test]
    fn test_token_health_to_markdown_market_empty_checks() {
        let analytics = make_test_analytics(false);
        let mut market = make_test_market_summary();
        market.checks = vec![];
        let md = token_health_to_markdown(&analytics, Some(&market), Some("binance"));
        assert!(md.contains("Market / Order Book"));
        assert!(!md.contains("Health Checks:")); // Empty checks = no section
    }

    #[test]
    fn test_token_health_to_json_empty_checks() {
        let analytics = make_test_analytics(false);
        let mut market = make_test_market_summary();
        market.checks = vec![];
        let json = token_health_to_json(&analytics, Some(&market)).unwrap();
        assert!(json.contains("\"market\""));
        assert!(json.contains("\"checks\": []"));
    }

    #[test]
    fn test_output_token_health_table_market_missing_prices() {
        let analytics = make_test_analytics(false);
        let mut market = make_test_market_summary();
        market.best_bid = None;
        market.best_ask = None;
        market.mid_price = None;
        let result =
            output_token_health_table(&analytics, Some(&market), Some("binance"), &quiet());
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_token_health_table_market_empty_checks() {
        let analytics = make_test_analytics(false);
        let mut market = make_test_market_summary();
        market.checks = vec![];
        let result = output_token_health_table(&analytics, Some(&market), None, &quiet());
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_token_health_table_market_without_venue() {
        let analytics = make_test_analytics(false);
        let market = make_test_market_summary();
        let result = output_token_health_table(&analytics, Some(&market), None, &quiet());
        assert!(result.is_ok());
    }

    #[test]
    fn test_token_health_args_all_formats() {
        for format in [
            OutputFormat::Table,
            OutputFormat::Json,
            OutputFormat::Csv,
            OutputFormat::Markdown,
        ] {
            let args = TokenHealthArgs {
                token: "USDC".to_string(),
                chain: "ethereum".to_string(),
                with_market: false,
                venue: "binance".to_string(),
                format,
                health: Default::default(),
            };
            assert_eq!(args.format, format);
        }
    }

    #[tokio::test]
    async fn test_run_token_health_csv() {
        let mut factory = MockClientFactory::new();
        factory.mock_dex.token_data = Some(make_test_dex_token_data(vec![]));

        let config = Config::default();
        let args = TokenHealthArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            with_market: false,
            venue: "binance".to_string(),
            format: OutputFormat::Csv,
            health: Default::default(),
        };

        let result = run(args, &config, &factory, &quiet()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_token_health_config_markdown_override() {
        let mut factory = MockClientFactory::new();
        factory.mock_dex.token_data = Some(make_test_dex_token_data(vec![]));

        let mut config = Config::default();
        config.output.format = OutputFormat::Markdown;
        let args = TokenHealthArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            chain: "ethereum".to_string(),
            with_market: false,
            venue: "binance".to_string(),
            format: OutputFormat::Table, // Args say Table, config overrides to Markdown
            health: Default::default(),
        };

        let result = run(args, &config, &factory, &quiet()).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_token_health_to_json_market_none_values() {
        let analytics = make_test_analytics(false);
        let mut market = make_test_market_summary();
        market.best_bid = None;
        market.best_ask = None;
        market.mid_price = None;
        market.spread = None;
        let json = token_health_to_json(&analytics, Some(&market)).unwrap();
        assert!(json.contains("\"market\""));
        assert!(json.contains("null")); // None serializes as null
    }

    #[test]
    fn test_token_health_to_markdown_report_footer() {
        let analytics = make_test_analytics(false);
        let md = token_health_to_markdown(&analytics, None, None);
        assert!(!md.is_empty());
        assert!(md.contains("USDC"));
        assert!(md.contains("USD Coin"));
    }

    #[test]
    fn test_output_token_health_table_with_dex_pairs_analytics() {
        let analytics = make_test_analytics(true);
        let result = output_token_health_table(&analytics, None, None, &quiet());
        assert!(result.is_ok());
    }

    #[test]
    fn test_token_health_to_markdown_market_with_venue_none() {
        let analytics = make_test_analytics(false);
        let market = make_test_market_summary();
        let md = token_health_to_markdown(&analytics, Some(&market), None);
        assert!(md.contains("Market / Order Book"));
        assert!(!md.contains("**Venue:**"));
    }

    #[test]
    fn test_output_token_health_table_market_no_bid_ask() {
        let analytics = make_test_analytics(false);
        let mut market = make_test_market_summary();
        market.best_bid = None;
        market.best_ask = None;
        market.mid_price = None;
        let result =
            output_token_health_table(&analytics, Some(&market), Some("binance"), &quiet());
        assert!(result.is_ok());
    }
}
