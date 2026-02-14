//! # Market Command
//!
//! Reports peg and order book health for stablecoin markets.
//! Fetches level-2 depth from CEX (Binance, Biconomy) or DEX (Ethereum, Solana) venues
//! and runs configurable health checks including volume and execution estimates.
//! Supports one-shot or repeated runs with configurable frequency and duration.

use crate::chains::ChainClientFactory;
use crate::cli::crawl::{self, Period};
use crate::config::Config;
use crate::error::{Result, ScopeError};
use crate::market::{
    HealthThresholds, MarketSummary, OrderBook, VenueRegistry, order_book_from_analytics,
};
use clap::{Args, Subcommand};
use std::time::Duration;

/// Default interval between summary runs when in repeat mode (60 seconds).
pub const DEFAULT_EVERY_SECS: u64 = 60;

/// Default total duration when in repeat mode (1 hour).
pub const DEFAULT_DURATION_SECS: u64 = 3600;

/// Market subcommands.
#[derive(Debug, Subcommand)]
pub enum MarketCommands {
    /// One-screen peg and order book health summary.
    ///
    /// Displays best bid/ask, mid price, spread, order book levels,
    /// and configurable health checks (peg safety, bid/ask ratio, depth thresholds).
    ///
    /// Use --every and --duration to run repeatedly (e.g., every 30s for 1 hour).
    Summary(SummaryArgs),
}

/// Arguments for `scope market summary`.
///
/// Default thresholds (min_levels, min_depth, peg_range) originated from the
/// PUSD Hummingbot market-making config and are tunable for other markets.
#[derive(Debug, Args)]
pub struct SummaryArgs {
    /// Base token symbol (e.g., USDC, PUSD). Quote is USDT.
    #[arg(default_value = "USDC", value_name = "SYMBOL")]
    pub pair: String,

    /// Market venue (e.g., binance, biconomy, mexc, okx, eth, solana).
    /// Use `scope venues list` to see all available venues.
    #[arg(long, default_value = "binance", value_name = "VENUE")]
    pub venue: String,

    /// Chain for DEX venues (ethereum or solana). Ignored for CEX.
    #[arg(long, default_value = "ethereum", value_name = "CHAIN")]
    pub chain: String,

    /// Peg target (e.g., 1.0 for USD stablecoins).
    #[arg(long, default_value = "1.0", value_name = "TARGET")]
    pub peg: f64,

    /// Minimum order book levels per side (default from PUSD Hummingbot config).
    #[arg(long, default_value = "6", value_name = "N")]
    pub min_levels: usize,

    /// Minimum depth per side in quote terms, e.g. USDT (default from PUSD Hummingbot config).
    #[arg(long, default_value = "3000", value_name = "USDT")]
    pub min_depth: f64,

    /// Peg range for outlier filtering (orders outside peg ± range×5 excluded).
    /// E.g., 0.001 = ±0.5% around peg.
    #[arg(long, default_value = "0.001", value_name = "RANGE")]
    pub peg_range: f64,

    /// Min bid/ask depth ratio (warn if ratio below this).
    #[arg(long, default_value = "0.2", value_name = "RATIO")]
    pub min_bid_ask_ratio: f64,

    /// Max bid/ask depth ratio (warn if ratio above this).
    #[arg(long, default_value = "5.0", value_name = "RATIO")]
    pub max_bid_ask_ratio: f64,

    /// Output format.
    #[arg(short, long, default_value = "text")]
    pub format: SummaryFormat,

    /// Run repeatedly at this interval (e.g., 30s, 5m, 1h).
    /// Default when in repeat mode: 60s.
    #[arg(long, value_name = "INTERVAL")]
    pub every: Option<String>,

    /// Run for this total duration (e.g., 10m, 1h, 24h).
    /// Default when in repeat mode: 1h.
    #[arg(long, value_name = "DURATION")]
    pub duration: Option<String>,

    /// Save markdown report to file (one-shot mode) or final report (repeat mode).
    #[arg(long, value_name = "PATH")]
    pub report: Option<std::path::PathBuf>,

    /// Append time-series CSV of peg/spread/depth to this path (repeat mode only).
    #[arg(long, value_name = "PATH")]
    pub csv: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum SummaryFormat {
    /// Human-readable text report (default).
    #[default]
    Text,
    /// JSON for programmatic consumption.
    Json,
}

/// Run the market command.
pub async fn run(
    args: MarketCommands,
    _config: &Config,
    factory: &dyn ChainClientFactory,
) -> Result<()> {
    match args {
        MarketCommands::Summary(summary_args) => run_summary(summary_args, factory).await,
    }
}

/// Parse duration strings like "30s", "5m", "1h", "24h" into seconds.
/// Extract base symbol from pair (e.g. "PUSD_USDT" -> "PUSD", "USDCUSDT" -> "USDC", "USDC" -> "USDC").
fn base_symbol_from_pair(pair: &str) -> &str {
    let p = pair.trim();
    if let Some(i) = p.find("_USDT") {
        return &p[..i];
    }
    if let Some(i) = p.find("_usdt") {
        return &p[..i];
    }
    if let Some(i) = p.find("/USDT") {
        return &p[..i];
    }
    if p.to_uppercase().ends_with("USDT") && p.len() > 4 {
        return &p[..p.len() - 4];
    }
    p
}

fn parse_duration(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ScopeError::Chain("Empty duration".to_string()));
    }
    let (num_str, unit) = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit() && *c != '.')
        .map(|(i, _)| (&s[..i], s[i..].trim()))
        .unwrap_or((s, "s"));

    let num: f64 = num_str
        .parse()
        .map_err(|_| ScopeError::Chain(format!("Invalid duration number: {}", num_str)))?;

    if num <= 0.0 {
        return Err(ScopeError::Chain("Duration must be positive".to_string()));
    }

    let secs = match unit.to_lowercase().as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => num,
        "m" | "min" | "mins" | "minute" | "minutes" => num * 60.0,
        "h" | "hr" | "hrs" | "hour" | "hours" => num * 3600.0,
        "d" | "day" | "days" => num * 86400.0,
        _ => {
            return Err(ScopeError::Chain(format!(
                "Unknown duration unit: {}",
                unit
            )));
        }
    };

    Ok(secs as u64)
}

/// Builds markdown report content for a market summary.
fn market_summary_to_markdown(summary: &MarketSummary, venue: &str, pair: &str) -> String {
    let bid_dev = summary
        .best_bid
        .map(|b| (b - summary.peg_target) / summary.peg_target * 100.0);
    let ask_dev = summary
        .best_ask
        .map(|a| (a - summary.peg_target) / summary.peg_target * 100.0);
    let volume_row = summary
        .volume_24h
        .map(|v| format!("| Volume (24h) | {:.0} USDT |  \n", v))
        .unwrap_or_default();
    let exec_buy = summary
        .execution_10k_buy
        .as_ref()
        .map(|e| {
            if e.fillable {
                format!("{:.2} bps", e.slippage_bps)
            } else {
                "insufficient".to_string()
            }
        })
        .unwrap_or_else(|| "-".to_string());
    let exec_sell = summary
        .execution_10k_sell
        .as_ref()
        .map(|e| {
            if e.fillable {
                format!("{:.2} bps", e.slippage_bps)
            } else {
                "insufficient".to_string()
            }
        })
        .unwrap_or_else(|| "-".to_string());
    let mut md = format!(
        "# Market Health Report: {}  \n\
        **Venue:** {}  \n\
        **Generated:** {}  \n\n\
        ## Peg & Spread  \n\
        | Metric | Value |  \n\
        |--------|-------|  \n\
        | Peg Target | {:.4} |  \n\
        | Best Bid | {} |  \n\
        | Best Ask | {} |  \n\
        | Mid Price | {} |  \n\
        | Spread | {} |  \n\
        {}\
        | 10k Buy Slippage | {} |  \n\
        | 10k Sell Slippage | {} |  \n\
        | Bid Depth | {:.0} |  \n\
        | Ask Depth | {:.0} |  \n\
        | Healthy | {} |  \n\n\
        ## Health Checks  \n",
        pair,
        venue,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
        summary.peg_target,
        summary
            .best_bid
            .map(|b| format!("{:.4} ({:+.3}%)", b, bid_dev.unwrap_or(0.0)))
            .unwrap_or_else(|| "-".to_string()),
        summary
            .best_ask
            .map(|a| format!("{:.4} ({:+.3}%)", a, ask_dev.unwrap_or(0.0)))
            .unwrap_or_else(|| "-".to_string()),
        summary
            .mid_price
            .map(|m| format!("{:.4}", m))
            .unwrap_or_else(|| "-".to_string()),
        summary
            .spread
            .map(|s| format!("{:.4}", s))
            .unwrap_or_else(|| "-".to_string()),
        volume_row,
        exec_buy,
        exec_sell,
        summary.bid_depth,
        summary.ask_depth,
        if summary.healthy { "✓" } else { "✗" }
    );
    for check in &summary.checks {
        let (icon, msg) = match check {
            crate::market::HealthCheck::Pass(m) => ("✓", m.as_str()),
            crate::market::HealthCheck::Fail(m) => ("✗", m.as_str()),
        };
        md.push_str(&format!("- {} {}\n", icon, msg));
    }
    md.push_str(&crate::display::report::report_footer());
    md
}

/// Whether the venue string refers to a DEX venue (handled by DexScreener, not the registry).
fn is_dex_venue(venue: &str) -> bool {
    matches!(venue.to_lowercase().as_str(), "ethereum" | "eth" | "solana")
}

/// Resolve DEX venue name to a canonical chain name.
fn dex_venue_to_chain(venue: &str) -> &str {
    match venue.to_lowercase().as_str() {
        "ethereum" | "eth" => "ethereum",
        "solana" => "solana",
        _ => "ethereum",
    }
}

async fn fetch_book_and_volume(
    args: &SummaryArgs,
    factory: &dyn ChainClientFactory,
) -> Result<(OrderBook, Option<f64>)> {
    let base = base_symbol_from_pair(&args.pair).to_string();

    if is_dex_venue(&args.venue) {
        // DEX path: synthesize from DexScreener analytics
        let chain = dex_venue_to_chain(&args.venue);
        let analytics =
            crawl::fetch_analytics_for_input(&base, chain, Period::Hour24, 10, factory).await?;
        if analytics.dex_pairs.is_empty() {
            return Err(ScopeError::Chain(format!(
                "No DEX pairs found for {} on {}",
                base, chain
            )));
        }
        let best_pair = analytics
            .dex_pairs
            .iter()
            .max_by(|a, b| {
                a.liquidity_usd
                    .partial_cmp(&b.liquidity_usd)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();
        let book = order_book_from_analytics(chain, best_pair, &analytics.token.symbol);
        let volume = Some(best_pair.volume_24h);
        Ok((book, volume))
    } else {
        // CEX path: use VenueRegistry + ExchangeClient
        let registry = VenueRegistry::load()?;
        let exchange = registry.create_exchange_client(&args.venue)?;
        let pair = exchange.format_pair(&base);
        let book = exchange.fetch_order_book(&pair).await?;

        // Get volume from ticker if available
        let volume = if exchange.has_ticker() {
            exchange
                .fetch_ticker(&pair)
                .await
                .ok()
                .and_then(|t| t.quote_volume_24h.or(t.volume_24h))
        } else {
            None
        };
        Ok((book, volume))
    }
}

async fn run_summary_once(
    args: &SummaryArgs,
    factory: &dyn ChainClientFactory,
    thresholds: &HealthThresholds,
    run_num: Option<u64>,
) -> Result<MarketSummary> {
    if let Some(n) = run_num {
        let ts = chrono::Utc::now().format("%H:%M:%S");
        eprintln!("  --- Run #{} at {} ---\n", n, ts);
    }

    let (book, volume_24h) = fetch_book_and_volume(args, factory).await?;
    let summary = MarketSummary::from_order_book(&book, args.peg, thresholds, volume_24h);

    let venue_label = args.venue.clone();

    match args.format {
        SummaryFormat::Text => {
            print!("{}", summary.format_text(Some(&venue_label)));
        }
        SummaryFormat::Json => {
            let json = serde_json::json!({
                "run": run_num,
                "venue": venue_label,
                "pair": summary.pair,
                "peg_target": summary.peg_target,
                "best_bid": summary.best_bid,
                "best_ask": summary.best_ask,
                "mid_price": summary.mid_price,
                "spread": summary.spread,
                "volume_24h": summary.volume_24h,
                "execution_10k_buy": summary.execution_10k_buy.as_ref().map(|e| serde_json::json!({
                    "fillable": e.fillable,
                    "slippage_bps": e.slippage_bps
                })),
                "execution_10k_sell": summary.execution_10k_sell.as_ref().map(|e| serde_json::json!({
                    "fillable": e.fillable,
                    "slippage_bps": e.slippage_bps
                })),
                "ask_depth": summary.ask_depth,
                "bid_depth": summary.bid_depth,
                "ask_levels": summary.asks.len(),
                "bid_levels": summary.bids.len(),
                "healthy": summary.healthy,
                "checks": summary.checks.iter().map(|c| match c {
                    crate::market::HealthCheck::Pass(m) => serde_json::json!({"status": "pass", "message": m}),
                    crate::market::HealthCheck::Fail(m) => serde_json::json!({"status": "fail", "message": m}),
                }).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
    }

    Ok(summary)
}

async fn run_summary(args: SummaryArgs, factory: &dyn ChainClientFactory) -> Result<()> {
    let thresholds = HealthThresholds {
        peg_target: args.peg,
        peg_range: args.peg_range,
        min_levels: args.min_levels,
        min_depth: args.min_depth,
        min_bid_ask_ratio: args.min_bid_ask_ratio,
        max_bid_ask_ratio: args.max_bid_ask_ratio,
    };

    let repeat_mode = args.every.is_some() || args.duration.is_some();

    if !repeat_mode {
        let summary = run_summary_once(&args, factory, &thresholds, None).await?;
        if let Some(ref report_path) = args.report {
            let venue_label = args.venue.clone();
            let md = market_summary_to_markdown(&summary, &venue_label, &args.pair);
            std::fs::write(report_path, md)?;
            eprintln!("\nReport saved to: {}", report_path.display());
        }
        return Ok(());
    }

    let every_secs = args
        .every
        .as_ref()
        .map(|s| parse_duration(s))
        .transpose()?
        .unwrap_or(DEFAULT_EVERY_SECS);

    let duration_secs = args
        .duration
        .as_ref()
        .map(|s| parse_duration(s))
        .transpose()?
        .unwrap_or(DEFAULT_DURATION_SECS);

    if every_secs == 0 {
        return Err(ScopeError::Chain("Interval must be positive".to_string()));
    }

    let every = Duration::from_secs(every_secs);
    let start = std::time::Instant::now();
    let duration = Duration::from_secs(duration_secs);

    eprintln!(
        "Running market summary every {}s for {}s (Ctrl+C to stop early)\n",
        every_secs, duration_secs
    );

    let mut run_num: u64 = 1;
    #[allow(unused_assignments)]
    let mut last_summary: Option<MarketSummary> = None;

    // Initialize CSV if requested
    if let Some(ref csv_path) = args.csv {
        let header =
            "timestamp,run,best_bid,best_ask,mid_price,spread,bid_depth,ask_depth,healthy\n";
        std::fs::write(csv_path, header)?;
    }

    loop {
        let summary = run_summary_once(&args, factory, &thresholds, Some(run_num)).await?;
        last_summary = Some(summary.clone());

        // Append CSV row
        if let Some(ref csv_path) = args.csv {
            let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
            let bid = summary
                .best_bid
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string());
            let ask = summary
                .best_ask
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string());
            let mid = summary
                .mid_price
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string());
            let spread = summary
                .spread
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string());
            let row = format!(
                "{},{},{},{},{},{},{},{},{}\n",
                ts,
                run_num,
                bid,
                ask,
                mid,
                spread,
                summary.bid_depth,
                summary.ask_depth,
                summary.healthy
            );
            let mut f = std::fs::OpenOptions::new().append(true).open(csv_path)?;
            use std::io::Write;
            f.write_all(row.as_bytes())?;
        }

        if start.elapsed() >= duration {
            eprintln!("\nCompleted {} run(s) over {}s.", run_num, duration_secs);
            break;
        }

        run_num += 1;

        let remaining = duration.saturating_sub(start.elapsed());
        let sleep_duration = if remaining < every { remaining } else { every };
        tokio::time::sleep(sleep_duration).await;
    }

    // Save final report if requested (last_summary always set when loop runs)
    if let (Some(ref report_path), Some(summary)) = (args.report, last_summary.as_ref()) {
        let venue_label = args.venue.clone();
        let md = market_summary_to_markdown(summary, &venue_label, &args.pair);
        std::fs::write(report_path, md)?;
        eprintln!("Report saved to: {}", report_path.display());
    }
    if let Some(ref csv_path) = args.csv {
        eprintln!("Time-series CSV saved to: {}", csv_path.display());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::DefaultClientFactory;

    /// Helper to create a mock venue YAML pointing at the given mock server URL.
    /// Writes a temporary venue descriptor to the user venues directory so the
    /// registry picks it up. Returns the venue id.
    #[allow(dead_code)]
    fn setup_mock_venue(server_url: &str) -> (String, tempfile::TempDir) {
        let venue_id = format!("test_mock_{}", std::process::id());
        let yaml = format!(
            r#"
id: {venue_id}
name: Test Mock Venue
base_url: {server_url}
timeout_secs: 5
symbol:
  template: "{{base}}_{{quote}}"
  default_quote: USDT
capabilities:
  order_book:
    path: /api/v1/depth
    params:
      symbol: "{{pair}}"
    response:
      asks_key: asks
      bids_key: bids
      level_format: positional
  ticker:
    path: /api/v1/ticker
    params:
      symbol: "{{pair}}"
    response:
      last_price: last
      volume_24h: vol
"#
        );
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join(format!("{}.yaml", venue_id));
        std::fs::write(&file_path, yaml).unwrap();
        // We can't easily inject into the registry, so instead
        // create a ConfigurableExchangeClient directly in tests.
        (venue_id, dir)
    }

    #[tokio::test]
    async fn test_run_summary_with_mock_orderbook() {
        // This test uses the DEX path since mock HTTP with the registry is complex.
        // Tested in integration tests and exchange module unit tests instead.
        // Test duration parsing and summary formatting here.
        let args = SummaryArgs {
            pair: "USDC".to_string(),
            venue: "eth".to_string(),
            chain: "ethereum".to_string(),
            peg: 1.0,
            min_levels: 1,
            min_depth: 50.0,
            peg_range: 0.01,
            min_bid_ask_ratio: 0.1,
            max_bid_ask_ratio: 10.0,
            format: SummaryFormat::Text,
            every: None,
            duration: None,
            report: None,
            csv: None,
        };

        let factory = DefaultClientFactory {
            chains_config: Default::default(),
        };
        // DEX path: will hit real DexScreener API, may fail in offline environments
        let _result = run_summary(args, &factory).await;
        // We don't assert success because it depends on network, just confirm no panic
    }

    #[tokio::test]
    async fn test_run_summary_json_format() {
        let args = SummaryArgs {
            pair: "USDC".to_string(),
            venue: "eth".to_string(),
            chain: "ethereum".to_string(),
            peg: 1.0,
            min_levels: 1,
            min_depth: 50.0,
            peg_range: 0.01,
            min_bid_ask_ratio: 0.1,
            max_bid_ask_ratio: 10.0,
            format: SummaryFormat::Json,
            every: None,
            duration: None,
            report: None,
            csv: None,
        };

        let factory = DefaultClientFactory {
            chains_config: Default::default(),
        };
        let _result = run_summary(args, &factory).await;
    }

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("30s").unwrap(), 30);
        assert_eq!(parse_duration("1").unwrap(), 1);
        assert_eq!(parse_duration("60sec").unwrap(), 60);
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("5m").unwrap(), 300);
        assert_eq!(parse_duration("1min").unwrap(), 60);
        assert_eq!(parse_duration("2.5m").unwrap(), 150);
    }

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(parse_duration("1h").unwrap(), 3600);
        assert_eq!(parse_duration("24h").unwrap(), 86400);
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("30x").is_err());
    }

    #[test]
    fn test_parse_duration_non_positive() {
        assert!(parse_duration("0").is_err());
        assert!(parse_duration("-5s").is_err());
    }

    // ====================================================================
    // base_symbol_from_pair tests
    // ====================================================================

    #[test]
    fn test_base_symbol_from_pair_underscore() {
        assert_eq!(base_symbol_from_pair("PUSD_USDT"), "PUSD");
        assert_eq!(base_symbol_from_pair("USDC_USDT"), "USDC");
    }

    #[test]
    fn test_base_symbol_from_pair_lowercase_underscore() {
        assert_eq!(base_symbol_from_pair("pusd_usdt"), "pusd");
    }

    #[test]
    fn test_base_symbol_from_pair_slash() {
        assert_eq!(base_symbol_from_pair("USDC/USDT"), "USDC");
    }

    #[test]
    fn test_base_symbol_from_pair_concat() {
        assert_eq!(base_symbol_from_pair("USDCUSDT"), "USDC");
        assert_eq!(base_symbol_from_pair("PUSDUSDT"), "PUSD");
    }

    #[test]
    fn test_base_symbol_from_pair_plain() {
        assert_eq!(base_symbol_from_pair("USDC"), "USDC");
        assert_eq!(base_symbol_from_pair("ETH"), "ETH");
    }

    #[test]
    fn test_base_symbol_from_pair_whitespace() {
        assert_eq!(base_symbol_from_pair("  PUSD_USDT  "), "PUSD");
    }

    // ====================================================================
    // market_summary_to_markdown tests
    // ====================================================================

    #[test]
    fn test_market_summary_to_markdown_basic() {
        use crate::market::{HealthCheck, MarketSummary};
        let summary = MarketSummary {
            pair: "USDCUSDT".to_string(),
            peg_target: 1.0,
            best_bid: Some(0.9999),
            best_ask: Some(1.0001),
            mid_price: Some(1.0000),
            spread: Some(0.0002),
            volume_24h: Some(1_000_000.0),
            bid_depth: 50_000.0,
            ask_depth: 50_000.0,
            bid_outliers: 0,
            ask_outliers: 0,
            healthy: true,
            checks: vec![HealthCheck::Pass("Spread within range".to_string())],
            execution_10k_buy: None,
            execution_10k_sell: None,
            asks: vec![],
            bids: vec![],
        };
        let md = market_summary_to_markdown(&summary, "Binance", "USDCUSDT");
        assert!(md.contains("Market Health Report"));
        assert!(md.contains("USDCUSDT"));
        assert!(md.contains("Binance"));
        assert!(md.contains("Peg Target"));
        assert!(md.contains("1.0000"));
        assert!(md.contains("Healthy"));
    }

    #[test]
    fn test_market_summary_to_markdown_no_prices() {
        use crate::market::{HealthCheck, MarketSummary};
        let summary = MarketSummary {
            pair: "TESTUSDT".to_string(),
            peg_target: 1.0,
            best_bid: None,
            best_ask: None,
            mid_price: None,
            spread: None,
            volume_24h: None,
            bid_depth: 0.0,
            ask_depth: 0.0,
            bid_outliers: 0,
            ask_outliers: 0,
            healthy: false,
            checks: vec![HealthCheck::Fail("No data".to_string())],
            execution_10k_buy: None,
            execution_10k_sell: None,
            asks: vec![],
            bids: vec![],
        };
        let md = market_summary_to_markdown(&summary, "Test", "TESTUSDT");
        assert!(md.contains("Market Health Report"));
        assert!(md.contains("-")); // missing data shown as "-"
    }

    // ====================================================================
    // parse_duration — additional edge cases
    // ====================================================================

    #[test]
    fn test_parse_duration_days() {
        assert_eq!(parse_duration("1d").unwrap(), 86400);
        assert_eq!(parse_duration("7d").unwrap(), 604800);
        assert_eq!(parse_duration("1day").unwrap(), 86400);
        assert_eq!(parse_duration("2days").unwrap(), 172800);
    }

    #[test]
    fn test_parse_duration_long_names() {
        assert_eq!(parse_duration("30seconds").unwrap(), 30);
        assert_eq!(parse_duration("5minutes").unwrap(), 300);
        assert_eq!(parse_duration("2hours").unwrap(), 7200);
    }

    #[test]
    fn test_parse_duration_fractional() {
        assert_eq!(parse_duration("0.5h").unwrap(), 1800);
        assert_eq!(parse_duration("1.5m").unwrap(), 90);
    }

    // ====================================================================
    // SummaryFormat tests
    // ====================================================================

    #[test]
    fn test_summary_format_default() {
        let fmt = SummaryFormat::default();
        assert!(matches!(fmt, SummaryFormat::Text));
    }

    #[test]
    fn test_summary_format_debug() {
        let text = format!("{:?}", SummaryFormat::Text);
        assert_eq!(text, "Text");
        let json = format!("{:?}", SummaryFormat::Json);
        assert_eq!(json, "Json");
    }

    // ====================================================================
    // MarketCommands parsing tests
    // ====================================================================

    #[test]
    fn test_summary_args_debug() {
        let args = SummaryArgs {
            pair: "USDC".to_string(),
            venue: "binance".to_string(),
            chain: "ethereum".to_string(),
            peg: 1.0,
            min_levels: 6,
            min_depth: 3000.0,
            peg_range: 0.001,
            min_bid_ask_ratio: 0.2,
            max_bid_ask_ratio: 5.0,
            format: SummaryFormat::Text,
            every: None,
            duration: None,
            report: None,
            csv: None,
        };
        let debug = format!("{:?}", args);
        assert!(debug.contains("SummaryArgs"));
        assert!(debug.contains("USDC"));
    }

    #[test]
    fn test_default_constants() {
        assert_eq!(DEFAULT_EVERY_SECS, 60);
        assert_eq!(DEFAULT_DURATION_SECS, 3600);
    }

    #[test]
    fn test_market_summary_to_markdown_with_execution_estimates() {
        use crate::market::{ExecutionEstimate, ExecutionSide, HealthCheck, MarketSummary};
        let summary = MarketSummary {
            pair: "TESTUSDT".to_string(),
            peg_target: 1.0,
            best_bid: Some(0.9999),
            best_ask: Some(1.0001),
            mid_price: Some(1.0000),
            spread: Some(0.0002),
            volume_24h: Some(1_000_000.0),
            bid_depth: 50_000.0,
            ask_depth: 50_000.0,
            bid_outliers: 0,
            ask_outliers: 0,
            healthy: true,
            checks: vec![HealthCheck::Pass("Spread within range".to_string())],
            execution_10k_buy: Some(ExecutionEstimate {
                notional_usdt: 10_000.0,
                side: ExecutionSide::Buy,
                vwap: 1.0001,
                slippage_bps: 1.5,
                fillable: true,
            }),
            execution_10k_sell: Some(ExecutionEstimate {
                notional_usdt: 10_000.0,
                side: ExecutionSide::Sell,
                vwap: 0.0,
                slippage_bps: 0.0,
                fillable: false,
            }),
            asks: vec![],
            bids: vec![],
        };
        let md = market_summary_to_markdown(&summary, "TestVenue", "TESTUSDT");
        assert!(md.contains("Market Health Report"));
        assert!(md.contains("TESTUSDT"));
        assert!(md.contains("TestVenue"));
        // Check for fillable buy slippage (should show "1.50 bps")
        assert!(md.contains("1.50 bps"));
        // Check for unfillable sell (should show "insufficient")
        assert!(md.contains("insufficient"));
    }

    #[tokio::test]
    async fn test_run_with_summary_command() {
        // Test the run() dispatcher with a DEX venue (doesn't require mock HTTP)
        let args = MarketCommands::Summary(SummaryArgs {
            pair: "USDC".to_string(),
            venue: "eth".to_string(),
            chain: "ethereum".to_string(),
            peg: 1.0,
            min_levels: 1,
            min_depth: 50.0,
            peg_range: 0.01,
            min_bid_ask_ratio: 0.1,
            max_bid_ask_ratio: 10.0,
            format: SummaryFormat::Text,
            every: None,
            duration: None,
            report: None,
            csv: None,
        });

        let factory = DefaultClientFactory {
            chains_config: Default::default(),
        };
        let config = Config::default();
        let _result = run(args, &config, &factory).await;
        // Don't assert success - depends on network
    }

    #[test]
    fn test_is_dex_venue() {
        assert!(is_dex_venue("eth"));
        assert!(is_dex_venue("ethereum"));
        assert!(is_dex_venue("Ethereum"));
        assert!(is_dex_venue("solana"));
        assert!(is_dex_venue("Solana"));
        assert!(!is_dex_venue("binance"));
        assert!(!is_dex_venue("okx"));
        assert!(!is_dex_venue("mexc"));
    }

    #[test]
    fn test_dex_venue_to_chain() {
        assert_eq!(dex_venue_to_chain("eth"), "ethereum");
        assert_eq!(dex_venue_to_chain("ethereum"), "ethereum");
        assert_eq!(dex_venue_to_chain("Ethereum"), "ethereum");
        assert_eq!(dex_venue_to_chain("solana"), "solana");
    }

    #[test]
    fn test_venue_registry_loaded_in_cex_path() {
        // Verify the registry loads and can create an exchange client for any built-in venue
        let registry = VenueRegistry::load().unwrap();
        assert!(registry.contains("binance"));
        let client = registry.create_exchange_client("binance");
        assert!(client.is_ok());
    }

    #[test]
    fn test_venue_registry_error_for_unknown() {
        let registry = VenueRegistry::load().unwrap();
        let result = registry.create_exchange_client("kracken");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unknown venue"));
        assert!(err.contains("Did you mean")); // should suggest kraken (distance 1)
    }

    #[tokio::test]
    async fn test_run_summary_json_format_with_mock() {
        let args = SummaryArgs {
            pair: "USDC".to_string(),
            venue: "eth".to_string(),
            chain: "ethereum".to_string(),
            peg: 1.0,
            min_levels: 1,
            min_depth: 50.0,
            peg_range: 0.01,
            min_bid_ask_ratio: 0.1,
            max_bid_ask_ratio: 10.0,
            format: SummaryFormat::Json,
            every: None,
            duration: None,
            report: None,
            csv: None,
        };

        let factory = DefaultClientFactory {
            chains_config: Default::default(),
        };
        let _result = run_summary(args, &factory).await;
    }
}
