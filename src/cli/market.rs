//! # Market Command
//!
//! Reports peg and order book health for stablecoin markets.
//! Fetches level-2 depth from CEX (Binance, Biconomy) or DEX (Ethereum, Solana) venues
//! and runs configurable health checks including volume and execution estimates.
//! Supports one-shot or repeated runs with configurable frequency and duration.

use crate::chains::ChainClientFactory;
use crate::cli::crawl::{self, Period};
use crate::config::Config;
use crate::display::terminal as t;
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

    /// Fetch OHLC/candlestick (kline) data from a CEX venue.
    Ohlc(OhlcArgs),

    /// Fetch recent trades from a CEX venue.
    Trades(TradesArgs),
}

/// Arguments for `scope market summary`.
///
/// Default thresholds (min_levels, min_depth, peg_range) originated from
/// stablecoin market-making defaults and are tunable for other markets.
#[derive(Debug, Args)]
#[command(
    after_help = "\x1b[1mExamples:\x1b[0m
  scope market summary DAI --venue binance
  scope market summary @dai-token --venue binance         \x1b[2m# address book shortcut\x1b[0m
  scope market summary USDC --venue binance --format json
  scope market summary DAI --venue binance --every 30s --duration 1h
  scope market summary DAI --venue binance --report health.md --csv peg.csv",
    after_long_help = "\x1b[1mExamples:\x1b[0m

  \x1b[1m$ scope market summary DAI --venue binance\x1b[0m

  +-- DAI/USDT (binance) ----------------------------+
  |                                                     |
  |-- Metrics                                           |
  |  Best Bid           0.9999  (-0.010%)               |
  |  Best Ask           1.0001  (+0.010%)               |
  |  Mid Price          1.0000  (+0.000%)               |
  |  Spread             0.0002  (0.020%)                |
  |  Volume (24h)       125000 USDT                     |
  |                                                     |
  |-- Health Checks                                     |
  |  + No sells below peg                               |
  |  + Bid/Ask ratio: 0.93x                             |
  |  + Bid levels: 8 >= 6 minimum                       |
  |  + Bid depth: 42000 USDT >= 3000 USDT minimum       |
  |                                                     |
  |  HEALTHY                                            |
  +-----------------------------------------------------+

  \x1b[1m$ scope market summary DAI --venue binance --every 30s --duration 1h\x1b[0m

  Monitoring DAI/USDT (binance) every 30s for 1h...
  [2026-02-16 10:00:00] Mid=1.0000 Spread=0.020% Depth=42K/45K HEALTHY
  [2026-02-16 10:00:30] Mid=1.0000 Spread=0.020% Depth=42K/44K HEALTHY
  [2026-02-16 10:01:00] Mid=0.9999 Spread=0.030% Depth=41K/44K HEALTHY
  ..."
)]
pub struct SummaryArgs {
    /// Base token symbol (e.g., USDC, DAI) or @label from address book. Quote is USDT.
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

    /// Minimum order book levels per side.
    #[arg(long, default_value = "6", value_name = "N")]
    pub min_levels: usize,

    /// Minimum depth per side in quote terms, e.g. USDT.
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

/// Arguments for `scope market ohlc`.
#[derive(Debug, Args)]
#[command(
    after_help = "\x1b[1mExamples:\x1b[0m
  scope market ohlc BTC
  scope market ohlc DAI --venue binance --interval 1d
  scope market ohlc ETH --venue mexc --limit 50 --format json",
    after_long_help = "\x1b[1mExamples:\x1b[0m

  \x1b[1m$ scope market ohlc BTC --limit 5\x1b[0m

  OHLC -- BTCUSDT (binance) interval=1h limit=5
  --------------------------------------------------------
            Open Time          Open         High          Low        Close         Volume
  --------------------------------------------------------
    2026-02-16 09:00  97250.120000  97380.540000  97210.980000  97345.670000        1234.56
    2026-02-16 08:00  97100.890000  97260.120000  97080.340000  97250.120000        1456.78
    2026-02-16 07:00  96950.230000  97120.890000  96920.560000  97100.890000        1678.90
    ...

    5 candles returned

  \x1b[1m$ scope market ohlc BTC --format json --limit 2\x1b[0m

  [
    {
      \"open_time\": 1739696400000,
      \"open\": 97250.12,
      \"high\": 97380.54,
      \"low\": 97210.98,
      \"close\": 97345.67,
      \"volume\": 1234.56,
      \"close_time\": null
    },
    ...
  ]"
)]
pub struct OhlcArgs {
    /// Trading pair symbol (e.g., USDC, BTC). Quote is USDT by default.
    #[arg(default_value = "USDC", value_name = "SYMBOL")]
    pub pair: String,

    /// Exchange venue (e.g., binance, mexc, bybit).
    #[arg(long, default_value = "binance", value_name = "VENUE")]
    pub venue: String,

    /// Candle interval (e.g., 1m, 5m, 15m, 1h, 4h, 1d).
    #[arg(long, default_value = "1h", value_name = "INTERVAL")]
    pub interval: String,

    /// Maximum number of candles to fetch.
    #[arg(long, default_value = "100", value_name = "LIMIT")]
    pub limit: u32,

    /// Output format.
    #[arg(long, default_value = "text")]
    pub format: OhlcFormat,
}

/// Arguments for `scope market trades`.
#[derive(Debug, Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope market trades BTC
  scope market trades DAI --venue binance --limit 20
  scope market trades ETH --venue okx --format json")]
pub struct TradesArgs {
    /// Trading pair symbol (e.g., USDC, BTC). Quote is USDT by default.
    #[arg(default_value = "USDC", value_name = "SYMBOL")]
    pub pair: String,

    /// Exchange venue (e.g., binance, mexc, bybit).
    #[arg(long, default_value = "binance", value_name = "VENUE")]
    pub venue: String,

    /// Maximum number of trades to fetch.
    #[arg(long, default_value = "50", value_name = "LIMIT")]
    pub limit: u32,

    /// Output format.
    #[arg(long, default_value = "text")]
    pub format: OhlcFormat,
}

/// Output format for market data commands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum OhlcFormat {
    /// Human-readable text table (default).
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
        MarketCommands::Ohlc(ohlc_args) => run_ohlc(ohlc_args).await,
        MarketCommands::Trades(trades_args) => run_trades(trades_args).await,
    }
}

/// Parse duration strings like "30s", "5m", "1h", "24h" into seconds.
/// Extract base symbol from pair (e.g. "DAI_USDT" -> "DAI", "USDCUSDT" -> "USDC", "USDC" -> "USDC").
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
            crawl::fetch_analytics_for_input(&base, chain, Period::Hour24, 10, factory, None)
                .await?;
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
            .ok_or_else(|| ScopeError::Chain("No DEX pairs after filter".to_string()))?;
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

// =============================================================================
// OHLC Command
// =============================================================================

/// Execute the `scope market ohlc` command.
async fn run_ohlc(args: OhlcArgs) -> Result<()> {
    let registry = VenueRegistry::load()?;
    let descriptor = registry.get(&args.venue).ok_or_else(|| {
        ScopeError::NotFound(format!(
            "Venue '{}' not found. Use `scope venues list` to see available venues.",
            args.venue
        ))
    })?;

    let client = crate::market::ExchangeClient::from_descriptor(descriptor);
    let pair = client.format_pair(base_symbol_from_pair(&args.pair));

    let candles = client.fetch_ohlc(&pair, &args.interval, args.limit).await?;

    match args.format {
        OhlcFormat::Json => {
            let json_candles: Vec<serde_json::Value> = candles
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "open_time": c.open_time,
                        "open": c.open,
                        "high": c.high,
                        "low": c.low,
                        "close": c.close,
                        "volume": c.volume,
                        "close_time": c.close_time,
                    })
                })
                .collect();
            let json_str = serde_json::to_string_pretty(&json_candles)
                .map_err(|e| ScopeError::Chain(format!("JSON serialization failed: {e}")))?;
            println!("{json_str}");
        }
        OhlcFormat::Text => {
            println!(
                "{}",
                t::section_header(&format!("OHLC — {} ({})", pair, args.venue))
            );
            println!("{}", t::kv_row("Interval", &args.interval));
            println!("{}", t::kv_row("Limit", &args.limit.to_string()));

            let cols = [
                t::Col {
                    label: "Open Time",
                    width: 19,
                    align: '>',
                },
                t::Col {
                    label: "Open",
                    width: 12,
                    align: '>',
                },
                t::Col {
                    label: "High",
                    width: 12,
                    align: '>',
                },
                t::Col {
                    label: "Low",
                    width: 12,
                    align: '>',
                },
                t::Col {
                    label: "Close",
                    width: 12,
                    align: '>',
                },
                t::Col {
                    label: "Volume",
                    width: 14,
                    align: '>',
                },
            ];
            println!("{}", t::table_header(&cols));

            for c in &candles {
                let dt = chrono::DateTime::from_timestamp_millis(c.open_time as i64)
                    .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| format!("{}", c.open_time));
                let open_str = format!("{:.6}", c.open);
                let high_str = format!("{:.6}", c.high);
                let low_str = format!("{:.6}", c.low);
                let close_str = format!("{:.6}", c.close);
                let volume_str = format!("{:.2}", c.volume);
                let values = [
                    dt.as_str(),
                    open_str.as_str(),
                    high_str.as_str(),
                    low_str.as_str(),
                    close_str.as_str(),
                    volume_str.as_str(),
                ];
                println!("{}", t::table_row(&cols, &values));
            }

            println!(
                "{}",
                t::info_row(&format!("{} candles returned", candles.len()))
            );
            println!("{}", t::section_footer());
        }
    }
    Ok(())
}

// =============================================================================
// Trades Command
// =============================================================================

/// Execute the `scope market trades` command.
async fn run_trades(args: TradesArgs) -> Result<()> {
    let registry = VenueRegistry::load()?;
    let descriptor = registry.get(&args.venue).ok_or_else(|| {
        ScopeError::NotFound(format!(
            "Venue '{}' not found. Use `scope venues list` to see available venues.",
            args.venue
        ))
    })?;

    let client = crate::market::ExchangeClient::from_descriptor(descriptor);
    let pair = client.format_pair(base_symbol_from_pair(&args.pair));

    let trades = client.fetch_recent_trades(&pair, args.limit).await?;

    match args.format {
        OhlcFormat::Json => {
            let json_trades: Vec<serde_json::Value> = trades
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "price": t.price,
                        "quantity": t.quantity,
                        "quote_quantity": t.quote_quantity,
                        "timestamp_ms": t.timestamp_ms,
                        "side": format!("{:?}", t.side),
                    })
                })
                .collect();
            let json_str = serde_json::to_string_pretty(&json_trades)
                .map_err(|e| ScopeError::Chain(format!("JSON serialization failed: {e}")))?;
            println!("{json_str}");
        }
        OhlcFormat::Text => {
            println!(
                "{}",
                t::section_header(&format!("Recent Trades — {} ({})", pair, args.venue))
            );

            let cols = [
                t::Col {
                    label: "Time",
                    width: 10,
                    align: '>',
                },
                t::Col {
                    label: "Side",
                    width: 5,
                    align: '>',
                },
                t::Col {
                    label: "Price",
                    width: 12,
                    align: '>',
                },
                t::Col {
                    label: "Qty",
                    width: 12,
                    align: '>',
                },
            ];
            println!("{}", t::table_header(&cols));

            for t in &trades {
                let time = chrono::DateTime::from_timestamp_millis(t.timestamp_ms as i64)
                    .map(|d| d.format("%H:%M:%S").to_string())
                    .unwrap_or_else(|| "?".to_string());
                let side_str = match t.side {
                    crate::market::TradeSide::Buy => "BUY",
                    crate::market::TradeSide::Sell => "SELL",
                };
                let price_str = format!("{:.6}", t.price);
                let qty_str = format!("{:.2}", t.quantity);
                let values = [
                    time.as_str(),
                    side_str,
                    price_str.as_str(),
                    qty_str.as_str(),
                ];
                println!("{}", t::table_row(&cols, &values));
            }

            println!(
                "{}",
                t::info_row(&format!("{} trades returned", trades.len()))
            );
            println!("{}", t::section_footer());
        }
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

        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
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

        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
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
    fn test_parse_duration_unknown_unit_error_message() {
        let result = parse_duration("30z");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Unknown duration unit"));
        assert!(err.to_string().contains("z"));
    }

    #[test]
    fn test_parse_duration_invalid_number_error() {
        let result = parse_duration("abc30s");
        assert!(result.is_err());
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
        assert_eq!(base_symbol_from_pair("DAI_USDT"), "DAI");
        assert_eq!(base_symbol_from_pair("USDC_USDT"), "USDC");
    }

    #[test]
    fn test_base_symbol_from_pair_lowercase_underscore() {
        assert_eq!(base_symbol_from_pair("dai_usdt"), "dai");
    }

    #[test]
    fn test_base_symbol_from_pair_slash() {
        assert_eq!(base_symbol_from_pair("USDC/USDT"), "USDC");
    }

    #[test]
    fn test_base_symbol_from_pair_concat() {
        assert_eq!(base_symbol_from_pair("USDCUSDT"), "USDC");
        assert_eq!(base_symbol_from_pair("DAIUSDT"), "DAI");
    }

    #[test]
    fn test_base_symbol_from_pair_plain() {
        assert_eq!(base_symbol_from_pair("USDC"), "USDC");
        assert_eq!(base_symbol_from_pair("ETH"), "ETH");
    }

    #[test]
    fn test_base_symbol_from_pair_whitespace() {
        assert_eq!(base_symbol_from_pair("  DAI_USDT  "), "DAI");
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
    fn test_ohlc_args_deserialization() {
        use crate::cli::{Cli, Commands};
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "scope",
            "market",
            "ohlc",
            "USDC",
            "--venue",
            "binance",
            "--interval",
            "1h",
            "--limit",
            "50",
        ])
        .unwrap();
        if let Commands::Market(MarketCommands::Ohlc(args)) = cli.command {
            assert_eq!(args.pair, "USDC");
            assert_eq!(args.venue, "binance");
            assert_eq!(args.interval, "1h");
            assert_eq!(args.limit, 50);
        } else {
            panic!("Expected Market Ohlc command");
        }
    }

    #[test]
    fn test_trades_args_deserialization() {
        use crate::cli::{Cli, Commands};
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "scope", "market", "trades", "BTC", "--venue", "mexc", "--limit", "100",
        ])
        .unwrap();
        if let Commands::Market(MarketCommands::Trades(args)) = cli.command {
            assert_eq!(args.pair, "BTC");
            assert_eq!(args.venue, "mexc");
            assert_eq!(args.limit, 100);
        } else {
            panic!("Expected Market Trades command");
        }
    }

    #[test]
    fn test_base_symbol_from_pair_various_inputs() {
        // Additional coverage for edge cases
        assert_eq!(base_symbol_from_pair("USDC"), "USDC");
        assert_eq!(base_symbol_from_pair("BTCUSDT"), "BTC");
        assert_eq!(base_symbol_from_pair("ETH/USDT"), "ETH");
        assert_eq!(base_symbol_from_pair("DAI_USDT"), "DAI");
        assert_eq!(base_symbol_from_pair("X"), "X"); // short symbol, no USDT suffix
        assert_eq!(base_symbol_from_pair(""), "");
    }

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

        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
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

        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
        };
        let _result = run_summary(args, &factory).await;
    }

    // ====================================================================
    // OHLC command tests
    // ====================================================================

    #[test]
    fn test_ohlc_format_default() {
        let fmt: OhlcFormat = Default::default();
        assert_eq!(fmt, OhlcFormat::Text);
    }

    #[test]
    fn test_ohlc_format_display() {
        // ValueEnum-derived parsing
        assert_eq!(format!("{:?}", OhlcFormat::Text), "Text");
        assert_eq!(format!("{:?}", OhlcFormat::Json), "Json");
    }

    #[test]
    fn test_ohlc_args_default_values() {
        // Verify we can construct OhlcArgs with defaults
        let args = OhlcArgs {
            pair: "BTC".to_string(),
            venue: "binance".to_string(),
            interval: "1h".to_string(),
            limit: 100,
            format: OhlcFormat::Text,
        };
        assert_eq!(args.pair, "BTC");
        assert_eq!(args.venue, "binance");
        assert_eq!(args.interval, "1h");
        assert_eq!(args.limit, 100);
    }

    #[test]
    fn test_trades_args_construction() {
        let args = TradesArgs {
            pair: "ETH".to_string(),
            venue: "okx".to_string(),
            limit: 50,
            format: OhlcFormat::Json,
        };
        assert_eq!(args.pair, "ETH");
        assert_eq!(args.venue, "okx");
        assert_eq!(args.limit, 50);
    }

    #[tokio::test]
    async fn test_run_ohlc_unknown_venue() {
        let args = OhlcArgs {
            pair: "BTC".to_string(),
            venue: "nonexistent_venue".to_string(),
            interval: "1h".to_string(),
            limit: 10,
            format: OhlcFormat::Text,
        };
        let result = run_ohlc(args).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not found"),
            "expected 'not found' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_run_trades_unknown_venue() {
        let args = TradesArgs {
            pair: "BTC".to_string(),
            venue: "nonexistent_venue".to_string(),
            limit: 10,
            format: OhlcFormat::Text,
        };
        let result = run_trades(args).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not found"),
            "expected 'not found' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_run_dispatches_ohlc() {
        let cmd = MarketCommands::Ohlc(OhlcArgs {
            pair: "BTC".to_string(),
            venue: "nonexistent_test_venue".to_string(),
            interval: "1h".to_string(),
            limit: 5,
            format: OhlcFormat::Text,
        });
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
        };
        let config = Config::default();
        let result = run(cmd, &config, &factory).await;
        // Should error with venue not found
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_run_dispatches_trades() {
        let cmd = MarketCommands::Trades(TradesArgs {
            pair: "ETH".to_string(),
            venue: "nonexistent_test_venue".to_string(),
            limit: 5,
            format: OhlcFormat::Json,
        });
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
        };
        let config = Config::default();
        let result = run(cmd, &config, &factory).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_run_ohlc_text_format_with_real_venue() {
        // Uses a real venue with a real API call. May succeed or fail depending
        // on network availability. Exercises venue resolution and client creation.
        let args = OhlcArgs {
            pair: "BTC".to_string(),
            venue: "binance".to_string(),
            interval: "1h".to_string(),
            limit: 3,
            format: OhlcFormat::Text,
        };
        let _result = run_ohlc(args).await;
        // Don't assert success — depends on network
    }

    #[tokio::test]
    async fn test_run_ohlc_json_format_with_real_venue() {
        let args = OhlcArgs {
            pair: "ETH".to_string(),
            venue: "binance".to_string(),
            interval: "15m".to_string(),
            limit: 2,
            format: OhlcFormat::Json,
        };
        let _result = run_ohlc(args).await;
    }

    #[tokio::test]
    async fn test_run_trades_text_format_with_real_venue() {
        let args = TradesArgs {
            pair: "BTC".to_string(),
            venue: "binance".to_string(),
            limit: 5,
            format: OhlcFormat::Text,
        };
        let _result = run_trades(args).await;
    }

    #[tokio::test]
    async fn test_run_trades_json_format_with_real_venue() {
        let args = TradesArgs {
            pair: "ETH".to_string(),
            venue: "binance".to_string(),
            limit: 3,
            format: OhlcFormat::Json,
        };
        let _result = run_trades(args).await;
    }

    #[tokio::test]
    async fn test_run_ohlc_multiple_venues() {
        // Exercise venue resolution for several built-in venues
        for venue in &["mexc", "okx", "bybit"] {
            let args = OhlcArgs {
                pair: "BTC".to_string(),
                venue: venue.to_string(),
                interval: "1h".to_string(),
                limit: 2,
                format: OhlcFormat::Json,
            };
            let _result = run_ohlc(args).await;
        }
    }

    #[tokio::test]
    async fn test_run_trades_multiple_venues() {
        for venue in &["mexc", "okx", "bybit"] {
            let args = TradesArgs {
                pair: "BTC".to_string(),
                venue: venue.to_string(),
                limit: 3,
                format: OhlcFormat::Text,
            };
            let _result = run_trades(args).await;
        }
    }

    // ====================================================================
    // parse_duration — additional edge cases for full coverage
    // ====================================================================

    #[test]
    fn test_parse_duration_whitespace_empty() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("   ").is_err());
    }

    #[test]
    fn test_parse_duration_sec_secs_second_seconds() {
        assert_eq!(parse_duration("1sec").unwrap(), 1);
        assert_eq!(parse_duration("2secs").unwrap(), 2);
        assert_eq!(parse_duration("1second").unwrap(), 1);
        assert_eq!(parse_duration("3seconds").unwrap(), 3);
    }

    #[test]
    fn test_parse_duration_minute_minutes() {
        assert_eq!(parse_duration("1minute").unwrap(), 60);
        assert_eq!(parse_duration("2minutes").unwrap(), 120);
    }

    #[test]
    fn test_parse_duration_hr_hrs_hour_hours() {
        assert_eq!(parse_duration("1hr").unwrap(), 3600);
        assert_eq!(parse_duration("2hrs").unwrap(), 7200);
        assert_eq!(parse_duration("1hour").unwrap(), 3600);
        assert_eq!(parse_duration("0.5hours").unwrap(), 1800);
    }

    #[test]
    fn test_parse_duration_number_only_defaults_to_seconds() {
        assert_eq!(parse_duration("1.5").unwrap(), 1);
        assert_eq!(parse_duration("42").unwrap(), 42);
    }

    #[test]
    fn test_parse_duration_trimmed_input() {
        assert_eq!(parse_duration("  30s  ").unwrap(), 30);
        assert_eq!(parse_duration(" 5m ").unwrap(), 300);
    }

    #[test]
    fn test_parse_duration_invalid_number_format() {
        assert!(parse_duration("1.2.3s").is_err());
        assert!(parse_duration("abc").is_err());
    }

    // ====================================================================
    // dex_venue_to_chain — unknown venue fallback
    // ====================================================================

    #[test]
    fn test_dex_venue_to_chain_unknown_returns_ethereum() {
        assert_eq!(dex_venue_to_chain("binance"), "ethereum");
        assert_eq!(dex_venue_to_chain("kraken"), "ethereum");
        assert_eq!(dex_venue_to_chain("unknown"), "ethereum");
    }

    // ====================================================================
    // market_summary_to_markdown — mixed Pass/Fail checks
    // ====================================================================

    #[test]
    fn test_market_summary_to_markdown_mixed_pass_fail_checks() {
        use crate::market::{HealthCheck, MarketSummary};
        let summary = MarketSummary {
            pair: "TESTUSDT".to_string(),
            peg_target: 1.0,
            best_bid: Some(0.9999),
            best_ask: Some(1.0001),
            mid_price: Some(1.0000),
            spread: Some(0.0002),
            volume_24h: Some(500_000.0),
            bid_depth: 40_000.0,
            ask_depth: 45_000.0,
            bid_outliers: 0,
            ask_outliers: 0,
            healthy: false,
            checks: vec![
                HealthCheck::Pass("Spread within range".to_string()),
                HealthCheck::Fail("Bid depth below minimum".to_string()),
            ],
            execution_10k_buy: None,
            execution_10k_sell: None,
            asks: vec![],
            bids: vec![],
        };
        let md = market_summary_to_markdown(&summary, "TestVenue", "TESTUSDT");
        assert!(md.contains("✓ Spread within range"));
        assert!(md.contains("✗ Bid depth below minimum"));
        assert!(md.contains("✗")); // unhealthy indicator in table
    }

    #[test]
    fn test_market_summary_to_markdown_empty_checks() {
        use crate::market::MarketSummary;
        let summary = MarketSummary {
            pair: "X".to_string(),
            peg_target: 1.0,
            best_bid: Some(1.0),
            best_ask: Some(1.0),
            mid_price: Some(1.0),
            spread: Some(0.0),
            volume_24h: None,
            bid_depth: 100.0,
            ask_depth: 100.0,
            bid_outliers: 0,
            ask_outliers: 0,
            healthy: true,
            checks: vec![],
            execution_10k_buy: None,
            execution_10k_sell: None,
            asks: vec![],
            bids: vec![],
        };
        let md = market_summary_to_markdown(&summary, "Venue", "X");
        assert!(md.contains("Market Health Report"));
        assert!(md.contains("Health Checks"));
    }

    // ====================================================================
    // OhlcFormat / SummaryFormat trait coverage
    // ====================================================================

    #[test]
    fn test_ohlc_format_partial_eq() {
        assert_eq!(OhlcFormat::Text, OhlcFormat::Text);
        assert_eq!(OhlcFormat::Json, OhlcFormat::Json);
        assert_ne!(OhlcFormat::Text, OhlcFormat::Json);
    }

    #[test]
    fn test_summary_format_clone_copy() {
        let text = SummaryFormat::Text;
        let cloned = text;
        assert!(matches!(cloned, SummaryFormat::Text));
        assert!(matches!(text, SummaryFormat::Text));
    }

    // ====================================================================
    // run_summary — error paths and report/csv
    // ====================================================================

    #[tokio::test]
    async fn test_run_summary_interval_zero_error() {
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
            every: Some("0.1s".to_string()),
            duration: Some("1m".to_string()),
            report: None,
            csv: None,
        };
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
        };
        let result = run_summary(args, &factory).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Interval must be positive") || err.contains("positive"),
            "expected interval error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_run_summary_one_shot_with_report() {
        let report_dir = tempfile::tempdir().unwrap();
        let report_path = report_dir.path().join("report.md");
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
            report: Some(report_path.clone()),
            csv: None,
        };
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
        };
        let result = run_summary(args, &factory).await;
        if result.is_ok() {
            let content = std::fs::read_to_string(&report_path).unwrap();
            assert!(content.contains("Market Health Report"));
        }
    }

    // ====================================================================
    // MarketCommands and struct Debug/construction
    // ====================================================================

    #[test]
    fn test_market_commands_debug() {
        let cmd = MarketCommands::Summary(SummaryArgs {
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
        });
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("Summary"));

        let ohlc_cmd = MarketCommands::Ohlc(OhlcArgs {
            pair: "BTC".to_string(),
            venue: "binance".to_string(),
            interval: "1h".to_string(),
            limit: 100,
            format: OhlcFormat::Text,
        });
        assert!(format!("{:?}", ohlc_cmd).contains("Ohlc"));

        let trades_cmd = MarketCommands::Trades(TradesArgs {
            pair: "ETH".to_string(),
            venue: "binance".to_string(),
            limit: 50,
            format: OhlcFormat::Json,
        });
        assert!(format!("{:?}", trades_cmd).contains("Trades"));
    }

    #[test]
    fn test_summary_args_with_report_csv_options() {
        let args = SummaryArgs {
            pair: "DAI".to_string(),
            venue: "binance".to_string(),
            chain: "ethereum".to_string(),
            peg: 1.0,
            min_levels: 6,
            min_depth: 3000.0,
            peg_range: 0.001,
            min_bid_ask_ratio: 0.2,
            max_bid_ask_ratio: 5.0,
            format: SummaryFormat::Json,
            every: Some("30s".to_string()),
            duration: Some("1h".to_string()),
            report: Some(std::path::PathBuf::from("/tmp/report.md")),
            csv: Some(std::path::PathBuf::from("/tmp/data.csv")),
        };
        assert_eq!(args.pair, "DAI");
        assert_eq!(args.venue, "binance");
        assert!(args.every.is_some());
        assert!(args.duration.is_some());
        assert!(args.report.is_some());
        assert!(args.csv.is_some());
    }

    #[test]
    fn test_base_symbol_from_pair_4char_usdt() {
        // "USDT" itself has len 4, so doesn't satisfy p.len() > 4
        assert_eq!(base_symbol_from_pair("USDT"), "USDT");
    }

    #[test]
    fn test_market_summary_to_markdown_unhealthy() {
        use crate::market::{HealthCheck, MarketSummary};
        let summary = MarketSummary {
            pair: "X".to_string(),
            peg_target: 1.0,
            best_bid: Some(0.99),
            best_ask: Some(1.01),
            mid_price: Some(1.0),
            spread: Some(0.02),
            volume_24h: None,
            bid_depth: 100.0,
            ask_depth: 100.0,
            bid_outliers: 0,
            ask_outliers: 0,
            healthy: false,
            checks: vec![HealthCheck::Fail("Peg deviation too high".to_string())],
            execution_10k_buy: None,
            execution_10k_sell: None,
            asks: vec![],
            bids: vec![],
        };
        let md = market_summary_to_markdown(&summary, "Test", "X");
        assert!(md.contains("✗"));
        assert!(md.contains("Peg deviation too high"));
    }

    #[test]
    fn test_ohlc_format_eq() {
        assert_eq!(OhlcFormat::Text, OhlcFormat::Text);
        assert_ne!(OhlcFormat::Text, OhlcFormat::Json);
    }

    #[test]
    fn test_trades_args_default_venue() {
        let args = TradesArgs {
            pair: "USDC".to_string(),
            venue: "binance".to_string(),
            limit: 50,
            format: OhlcFormat::Text,
        };
        assert_eq!(args.pair, "USDC");
        assert_eq!(args.venue, "binance");
    }
}
