//! # Market Command
//!
//! Reports peg and order book health for stablecoin markets.
//! Fetches level-2 depth from CEX (Binance, Biconomy) or DEX (Ethereum, Solana) venues
//! and runs configurable health checks including volume and execution estimates.
//! Supports one-shot or repeated runs with configurable frequency and duration.

use crate::cli::crawl::{self, Period};
use crate::cli::output::Output;
use crate::{note, out, outln};
use clap::{Args, Subcommand};
use scope::chains::{ChainClientFactory, TokenAnalytics};
use scope::config::Config;

use super::health_args::HealthArgs;
use crate::cli::repeat::{self, RunOutput, Schedule};
use crate::cli::resolve::{
    BindingStore, ResolveContext, VenueCandidates, VenueKind, picker_for, resolve,
};
use scope::display::terminal as t;
use scope::error::{Result, ScopeError};
use scope::market::assets::TokenAddress;
use scope::market::series;
use scope::market::{
    DexAnalyticsSource, ExchangeClient, HealthThresholds, MarketSummary, MarketVenue, OrderBook,
    Pair, VENUE_HELP, VenueRegistry,
};
use std::time::Duration;

/// Help for every market pair argument; the rule is `scope::market::Pair::parse`.
const PAIR_HELP: &str = "Pair or base symbol. BASE_QUOTE, BASE/QUOTE and BASE-QUOTE work for any quote \
    (BTC_ETH). BASEQUOTE works only for a stable or fiat quote (OPUSDT, BTCEUR); WETH stays \
    WETH. A bare base (USDC) uses the venue's default quote.";

/// Default interval between summary runs when in repeat mode (60 seconds).
pub const DEFAULT_EVERY_SECS: u64 = 60;

/// `--duration` values that run until the user cancels.
const UNTIL_CANCELLED: [&str; 2] = ["forever", "inf"];

/// Default total duration when in repeat mode (1 hour).
pub const DEFAULT_DURATION_SECS: u64 = 3600;

/// Market subcommands.
#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
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
  |  Volume (24h)       125.00K USDT                    |
  |                                                     |
  |-- Health Checks                                     |
  |  + No sells below peg                               |
  |  + Bid/Ask ratio: 0.93x                             |
  |  + Bid levels: 12 >= 10 minimum                     |
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
    #[arg(default_value = scope::market::DEFAULT_PAIR, value_name = "SYMBOL", help = PAIR_HELP)]
    pub pair: String,

    #[arg(long, default_value = scope::market::DEFAULT_VENUE, value_name = "VENUE", help = VENUE_HELP)]
    pub venue: String,

    /// The base token's contract: the venue's symbol must be this token.
    #[arg(long = "token", value_name = "CONTRACT")]
    pub token_contract: Option<String>,

    /// Chain for a DEX venue. The venue already names its chain, so this is
    /// only a check: a different chain is an error. Ignored for CEX venues.
    #[arg(long, value_name = "CHAIN")]
    pub chain: Option<String>,

    /// Health threshold overrides (flag > config `market.health` > default).
    #[command(flatten)]
    pub health: HealthArgs,

    /// Output format.
    #[arg(short, long, default_value = "text")]
    pub format: SummaryFormat,

    /// Run repeatedly at this interval (e.g., 30s, 5m, 1h).
    /// Default when in repeat mode: 60s.
    #[arg(long, value_name = "INTERVAL")]
    pub every: Option<String>,

    /// Run for this total duration (e.g., 10m, 1h, 24h), or `forever`
    /// (also `inf`) to run until Esc or Ctrl-C.
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
    #[arg(default_value = scope::market::DEFAULT_PAIR, value_name = "SYMBOL", help = PAIR_HELP)]
    pub pair: String,

    /// Exchange venue (e.g., binance, mexc, bybit).
    #[arg(long, default_value = scope::market::DEFAULT_VENUE, value_name = "VENUE")]
    pub venue: String,

    /// The base token's contract: the venue's symbol must be this token.
    #[arg(long = "token", value_name = "CONTRACT")]
    pub token_contract: Option<String>,

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
    #[arg(default_value = scope::market::DEFAULT_PAIR, value_name = "SYMBOL", help = PAIR_HELP)]
    pub pair: String,

    /// Exchange venue (e.g., binance, mexc, bybit).
    #[arg(long, default_value = scope::market::DEFAULT_VENUE, value_name = "VENUE")]
    pub venue: String,

    /// The base token's contract: the venue's symbol must be this token.
    #[arg(long = "token", value_name = "CONTRACT")]
    pub token_contract: Option<String>,

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
    config: &Config,
    factory: &dyn ChainClientFactory,
    out: &Output,
) -> Result<()> {
    match args {
        MarketCommands::Summary(summary_args) => {
            run_summary(summary_args, config, factory, out).await
        }
        MarketCommands::Ohlc(ohlc_args) => run_ohlc(ohlc_args, out).await,
        MarketCommands::Trades(trades_args) => run_trades(trades_args, out).await,
    }
}

/// The venue symbol for user pair input (see [`Pair::parse`]).
///
/// Kept only for the monitor (`monitor/mod.rs`), which still calls it; the
/// market commands call `venue_symbol` directly.
pub(crate) fn venue_pair(exchange: &scope::market::ExchangeClient, input: &str) -> String {
    exchange.venue_symbol(&Pair::parse(input))
}

/// Token analytics from the crawl pipeline: the CLI and web adapter for the
/// core [`DexAnalyticsSource`] port.
pub struct CrawlDexSource<'a> {
    /// Chain clients for the token lookup.
    pub factory: &'a dyn ChainClientFactory,
    /// Where the crawl reports progress and warnings.
    pub out: &'a Output,
}

#[async_trait::async_trait]
impl DexAnalyticsSource for CrawlDexSource<'_> {
    async fn token_analytics(&self, token: &str, chain: &str) -> Result<TokenAnalytics> {
        crawl::fetch_analytics_for_input(
            token,
            chain,
            Period::Hour24,
            10,
            self.factory,
            None,
            self.out,
        )
        .await
    }
}

/// Parse duration strings like "30s", "5m", "1h", "24h" into seconds.
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
    let mut md = format!(
        "# Market Health Report: {}  \n\
        **Generated:** {}  \n\n",
        pair,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );
    md.push_str(&summary.format_markdown(Some(venue)));
    md.push_str(&scope::display::report::report_footer());
    md
}

/// Fetches the book and volume for the summary arguments. `--chain`, when
/// given, must name the DEX venue's chain.
async fn fetch_book_and_volume(
    args: &SummaryArgs,
    thresholds: &HealthThresholds,
    factory: &dyn ChainClientFactory,
    out: &Output,
) -> Result<(OrderBook, Option<f64>)> {
    let venue = MarketVenue::from_name(&args.venue)?;
    if let Some(chain) = &args.chain {
        venue.require_chain(chain)?;
    }
    let dex = CrawlDexSource { factory, out };
    scope::market::fetch_book_and_volume(&venue, &Pair::parse(&args.pair), thresholds, &dex).await
}

/// On a CEX venue, proves which token the base symbol is: a saved binding
/// must match the venue's coin list, and a symbol on several networks is a
/// choice (asked, or an error without a terminal). The venue symbol itself
/// does not change.
pub(crate) async fn check_cex_token(
    client: &ExchangeClient,
    base: &str,
    chain: Option<String>,
    token_flag: Option<String>,
    store: &mut dyn BindingStore,
    json: bool,
    out: &Output,
) -> Result<()> {
    let ctx = ResolveContext {
        chain,
        venue: VenueKind::Cex(client.venue_id().to_string()),
        token_flag,
        yes: false,
        save: false,
    };
    let source = VenueCandidates { client };
    let picker = picker_for(out, json);
    resolve(base, &ctx, &source, picker.as_ref(), store, out)
        .await
        .map(|_| ())
}

/// [`check_cex_token`] for a command's venue name and pair, once per
/// command; a DEX venue resolves its token in the analytics lookup.
async fn check_venue_token(
    venue: &str,
    pair: &str,
    chain: Option<String>,
    token_flag: Option<String>,
    json: bool,
    out: &Output,
) -> Result<()> {
    let Ok(MarketVenue::Cex(client)) = MarketVenue::from_name(venue) else {
        return Ok(());
    };
    let base = Pair::parse(pair).base;
    let mut aliases = scope::tokens::TokenAliases::load();
    check_cex_token(&client, &base, chain, token_flag, &mut aliases, json, out).await
}

/// The pair's tokens on the venue (network and contract), when the venue
/// lists its assets. A failed lookup is noted and gives no lines: the
/// summary itself does not depend on it.
async fn token_addresses(args: &SummaryArgs, out: &Output) -> Vec<TokenAddress> {
    let Ok(MarketVenue::Cex(client)) = MarketVenue::from_name(&args.venue) else {
        return Vec::new();
    };
    match client.fetch_token_addresses(&Pair::parse(&args.pair)).await {
        None => Vec::new(),
        Some(Ok(tokens)) => tokens,
        Some(Err(e)) => {
            note!(out, "Token addresses are not available: {e}");
            Vec::new()
        }
    }
}

/// One summary run: the summary and its report, as the CLI prints it.
async fn summary_run(
    args: &SummaryArgs,
    factory: &dyn ChainClientFactory,
    thresholds: &HealthThresholds,
    tokens: &[TokenAddress],
    run_num: Option<u64>,
    out: &Output,
) -> Result<(MarketSummary, String)> {
    let (book, volume_24h) = fetch_book_and_volume(args, thresholds, factory, out).await?;
    let summary = MarketSummary::from_order_book(&book, thresholds, volume_24h);
    let report = match args.format {
        SummaryFormat::Text => summary.format_text_with_tokens(Some(&args.venue), tokens),
        SummaryFormat::Json => {
            let mut json = summary.to_json();
            if let Some(obj) = json.as_object_mut() {
                obj.insert("run".to_string(), serde_json::json!(run_num));
                obj.insert("venue".to_string(), serde_json::json!(args.venue));
                if !tokens.is_empty() {
                    obj.insert("tokens".to_string(), serde_json::json!(tokens));
                }
            }
            format!("{}\n", serde_json::to_string_pretty(&json)?)
        }
    };
    Ok((summary, report))
}

/// Writes the `--report` file for `summary`, when one was asked for.
fn write_report(args: &SummaryArgs, summary: &MarketSummary, out: &Output) -> Result<()> {
    if let Some(ref report_path) = args.report {
        let md = market_summary_to_markdown(summary, &args.venue, &args.pair);
        std::fs::write(report_path, md)?;
        out.note_saved("Report", report_path.display());
    }
    Ok(())
}

/// The `--every`/`--duration` schedule, when either flag is set.
fn summary_schedule(args: &SummaryArgs) -> Result<Option<Schedule>> {
    if args.every.is_none() && args.duration.is_none() {
        return Ok(None);
    }
    let secs = |flag: &Option<String>, default| {
        flag.as_deref()
            .map(parse_duration)
            .transpose()
            .map(|v| v.unwrap_or(default))
    };
    let every = Duration::from_secs(secs(&args.every, DEFAULT_EVERY_SECS)?);
    if args.duration.as_deref().is_some_and(is_until_cancelled) {
        return Schedule::until_cancelled(every).map(Some);
    }
    let duration = secs(&args.duration, DEFAULT_DURATION_SECS)?;
    Schedule::new(every, Duration::from_secs(duration)).map(Some)
}

/// `--duration forever` or `inf`: run until cancelled (#96).
fn is_until_cancelled(s: &str) -> bool {
    UNTIL_CANCELLED.contains(&s.trim().to_ascii_lowercase().as_str())
}

async fn run_summary(
    args: SummaryArgs,
    config: &Config,
    factory: &dyn ChainClientFactory,
    out: &Output,
) -> Result<()> {
    let thresholds = args.health.resolve(config);
    let json = matches!(args.format, SummaryFormat::Json);
    check_venue_token(
        &args.venue,
        &args.pair,
        args.chain.clone(),
        args.token_contract.clone(),
        json,
        out,
    )
    .await?;
    // Once per command: the coin list does not change between runs.
    let tokens = token_addresses(&args, out).await;

    let Some(schedule) = summary_schedule(&args)? else {
        let (summary, report) =
            summary_run(&args, factory, &thresholds, &tokens, None, out).await?;
        out!(out, "{report}")?;
        return write_report(&args, &summary, out);
    };

    note!(
        out,
        "Running market summary every {}s {} (Ctrl+C to stop)\n",
        schedule.every.as_secs(),
        match schedule.duration {
            Some(d) => format!("for {}s", d.as_secs()),
            None => "until cancelled".to_string(),
        }
    );
    let metrics = series::summary_metrics(&thresholds);
    let last = repeat::run(&schedule, &metrics, args.csv.as_deref(), out, |n| {
        let (args, thresholds, metrics, tokens) = (&args, &thresholds, &metrics, &tokens);
        async move {
            let (summary, report) =
                summary_run(args, factory, thresholds, tokens, Some(n), out).await?;
            Ok(RunOutput {
                sample: series::sample(&summary, metrics),
                snapshot: report,
                value: summary,
            })
        }
    })
    .await?;

    if let Some(summary) = last {
        write_report(&args, &summary, out)?;
    }
    if let Some(ref csv_path) = args.csv {
        out.note_saved("Time-series CSV", csv_path.display());
    }
    Ok(())
}

// =============================================================================
// OHLC Command
// =============================================================================

/// Execute the `scope market ohlc` command.
async fn run_ohlc(args: OhlcArgs, out: &Output) -> Result<()> {
    check_venue_token(
        &args.venue,
        &args.pair,
        None,
        args.token_contract.clone(),
        false,
        out,
    )
    .await?;
    let client = VenueRegistry::load()?.create_exchange_client(&args.venue)?;
    run_ohlc_with(&client, args, out).await
}

/// `scope market ohlc` on `client`, the venue that `args.venue` names.
async fn run_ohlc_with(client: &ExchangeClient, args: OhlcArgs, out: &Output) -> Result<()> {
    let pair = client.venue_symbol(&Pair::parse(&args.pair));

    let candles = client.fetch_ohlc(&pair, &args.interval, args.limit).await?;

    match args.format {
        OhlcFormat::Json => {
            let json_str = serde_json::to_string_pretty(&candles)
                .map_err(|e| ScopeError::Chain(format!("JSON serialization failed: {e}")))?;
            outln!(out, "{json_str}")?;
        }
        OhlcFormat::Text => {
            outln!(
                out,
                "{}",
                t::section_header(&format!("OHLC — {} ({})", pair, args.venue))
            )?;
            outln!(out, "{}", t::kv_row("Interval", &args.interval))?;
            outln!(out, "{}", t::kv_row("Limit", &args.limit.to_string()))?;

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
            outln!(out, "{}", t::table_header(&cols))?;

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
                outln!(out, "{}", t::table_row(&cols, &values))?;
            }

            outln!(
                out,
                "{}",
                t::info_row(&format!("{} candles returned", candles.len()))
            )?;
            outln!(out, "{}", t::section_footer())?;
        }
    }
    Ok(())
}

// =============================================================================
// Trades Command
// =============================================================================

/// Execute the `scope market trades` command.
async fn run_trades(args: TradesArgs, out: &Output) -> Result<()> {
    check_venue_token(
        &args.venue,
        &args.pair,
        None,
        args.token_contract.clone(),
        false,
        out,
    )
    .await?;
    let client = VenueRegistry::load()?.create_exchange_client(&args.venue)?;
    run_trades_with(&client, args, out).await
}

/// `scope market trades` on `client`, the venue that `args.venue` names.
async fn run_trades_with(client: &ExchangeClient, args: TradesArgs, out: &Output) -> Result<()> {
    let pair = client.venue_symbol(&Pair::parse(&args.pair));

    let trades = client.fetch_recent_trades(&pair, args.limit).await?;

    match args.format {
        OhlcFormat::Json => {
            let json_str = serde_json::to_string_pretty(&trades)
                .map_err(|e| ScopeError::Chain(format!("JSON serialization failed: {e}")))?;
            outln!(out, "{json_str}")?;
        }
        OhlcFormat::Text => {
            outln!(
                out,
                "{}",
                t::section_header(&format!("Recent Trades — {} ({})", pair, args.venue))
            )?;

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
            outln!(out, "{}", t::table_header(&cols))?;

            for t in &trades {
                let time = chrono::DateTime::from_timestamp_millis(t.timestamp_ms as i64)
                    .map(|d| d.format("%H:%M:%S").to_string())
                    .unwrap_or_else(|| "?".to_string());
                let side_str = match t.side {
                    scope::market::TradeSide::Buy => "BUY",
                    scope::market::TradeSide::Sell => "SELL",
                };
                let price_str = format!("{:.6}", t.price);
                let qty_str = format!("{:.2}", t.quantity);
                let values = [
                    time.as_str(),
                    side_str,
                    price_str.as_str(),
                    qty_str.as_str(),
                ];
                outln!(out, "{}", t::table_row(&cols, &values))?;
            }

            outln!(
                out,
                "{}",
                t::info_row(&format!("{} trades returned", trades.len()))
            )?;
            outln!(out, "{}", t::section_footer())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope::chains::DefaultClientFactory;
    use scope::market::DexVenue;

    fn quiet() -> crate::cli::output::Output {
        crate::cli::output::Output::capture().0
    }

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
            token_contract: None,
            chain: None,
            health: HealthArgs {
                peg_target: Some(1.0),
                peg_range: Some(0.01),
                min_levels: Some(1),
                min_depth: Some(50.0),
                min_bid_ask_ratio: Some(0.1),
                max_bid_ask_ratio: Some(10.0),
                ..Default::default()
            },
            format: SummaryFormat::Text,
            every: None,
            duration: None,
            report: None,
            csv: None,
        };

        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
        };
        // DEX path: will hit real DexScreener API, may fail in offline environments
        let _result = run_summary(args, &Config::default(), &factory, &quiet()).await;
        // We don't assert success because it depends on network, just confirm no panic
    }

    #[tokio::test]
    async fn test_run_summary_json_format() {
        let args = SummaryArgs {
            pair: "USDC".to_string(),
            venue: "eth".to_string(),
            token_contract: None,
            chain: None,
            health: HealthArgs {
                peg_target: Some(1.0),
                peg_range: Some(0.01),
                min_levels: Some(1),
                min_depth: Some(50.0),
                min_bid_ask_ratio: Some(0.1),
                max_bid_ask_ratio: Some(10.0),
                ..Default::default()
            },
            format: SummaryFormat::Json,
            every: None,
            duration: None,
            report: None,
            csv: None,
        };

        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
        };
        let _result = run_summary(args, &Config::default(), &factory, &quiet()).await;
    }

    #[test]
    fn test_duration_forever_has_no_end_and_the_default_stays_one_hour() {
        // #96: `--duration forever` (or `inf`) runs until cancelled; leaving
        // the flag out keeps the 1 h default so scripts do not change.
        let args = |duration: Option<&str>| SummaryArgs {
            pair: "USDC".to_string(),
            venue: "binance".to_string(),
            token_contract: None,
            chain: None,
            health: HealthArgs::default(),
            format: SummaryFormat::Text,
            every: Some("30s".to_string()),
            duration: duration.map(str::to_string),
            report: None,
            csv: None,
        };
        for word in ["forever", "inf", "FOREVER"] {
            let s = summary_schedule(&args(Some(word))).unwrap().unwrap();
            assert_eq!(s.duration, None, "{word}");
        }
        let s = summary_schedule(&args(None)).unwrap().unwrap();
        assert_eq!(s.duration, Some(Duration::from_secs(DEFAULT_DURATION_SECS)));
        let s = summary_schedule(&args(Some("10m"))).unwrap().unwrap();
        assert_eq!(s.duration, Some(Duration::from_secs(600)));
    }

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("30s").unwrap(), 30);
        assert!(
            parse_duration("forever").is_err(),
            "not a span; --duration only"
        );
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
        assert_eq!(Pair::parse("DAI_USDT").base, "DAI");
        assert_eq!(Pair::parse("USDC_USDT").base, "USDC");
    }

    #[test]
    fn test_base_symbol_from_pair_lowercase_underscore() {
        assert_eq!(Pair::parse("dai_usdt").base, "dai");
    }

    #[test]
    fn test_base_symbol_from_pair_slash() {
        assert_eq!(Pair::parse("USDC/USDT").base, "USDC");
    }

    #[test]
    fn test_base_symbol_from_pair_concat() {
        assert_eq!(Pair::parse("USDCUSDT").base, "USDC");
        assert_eq!(Pair::parse("DAIUSDT").base, "DAI");
    }

    #[test]
    fn test_base_symbol_from_pair_plain() {
        assert_eq!(Pair::parse("USDC").base, "USDC");
        assert_eq!(Pair::parse("ETH").base, "ETH");
    }

    #[test]
    fn test_base_symbol_from_pair_whitespace() {
        assert_eq!(Pair::parse("  DAI_USDT  ").base, "DAI");
    }

    #[test]
    fn test_normalize_pair_input_key_value() {
        assert_eq!(Pair::parse("pair=USDT_PUSD").to_string(), "USDT/PUSD");
        assert_eq!(
            Pair::parse("pair_symbol=USDT_PUSD").to_string(),
            "USDT/PUSD"
        );
        assert_eq!(Pair::parse("USDT_PUSD").to_string(), "USDT/PUSD");
    }

    #[test]
    fn test_parse_pair_components_delimited() {
        assert_eq!(Pair::parse("USDT_PUSD").to_string(), "USDT/PUSD");
        assert_eq!(Pair::parse("ETH/USDC").to_string(), "ETH/USDC");
        assert_eq!(Pair::parse("BTC-USDT").to_string(), "BTC/USDT");
    }

    #[test]
    fn test_parse_pair_components_concat_common_quote() {
        assert_eq!(Pair::parse("BTCUSDT").to_string(), "BTC/USDT");
        assert_eq!(Pair::parse("ETHUSD").to_string(), "ETH/USD");
        assert_eq!(Pair::parse("ARB").quote, None);
    }

    // ====================================================================
    // market_summary_to_markdown tests
    // ====================================================================

    #[test]
    fn test_market_summary_to_markdown_basic() {
        use scope::market::{HealthCheck, MarketSummary};
        let summary = MarketSummary {
            pair: "USDCUSDT".to_string(),
            source: scope::market::BookSource::Exchange,
            peg_target: 1.0,
            peg_range: scope::market::health::DEFAULT_PEG_RANGE,
            best_bid: Some(0.9999),
            best_ask: Some(1.0001),
            mid_price: Some(1.0000),
            spread: Some(0.0002),
            spread_pct: None,
            top_depth: Default::default(),
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
        use scope::market::{HealthCheck, MarketSummary};
        let summary = MarketSummary {
            pair: "TESTUSDT".to_string(),
            source: scope::market::BookSource::Exchange,
            peg_target: 1.0,
            peg_range: scope::market::health::DEFAULT_PEG_RANGE,
            best_bid: None,
            best_ask: None,
            mid_price: None,
            spread: None,
            spread_pct: None,
            top_depth: Default::default(),
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
        assert_eq!(Pair::parse("USDC").base, "USDC");
        assert_eq!(Pair::parse("BTCUSDT").base, "BTC");
        assert_eq!(Pair::parse("ETH/USDT").base, "ETH");
        assert_eq!(Pair::parse("DAI_USDT").base, "DAI");
        assert_eq!(Pair::parse("X").base, "X"); // short symbol, no USDT suffix
        assert_eq!(Pair::parse("").base, "");
    }

    #[test]
    fn test_summary_args_debug() {
        let args = SummaryArgs {
            pair: "USDC".to_string(),
            venue: "binance".to_string(),
            token_contract: None,
            chain: None,
            health: HealthArgs {
                peg_target: Some(1.0),
                peg_range: Some(0.001),
                min_levels: Some(6),
                min_depth: Some(3000.0),
                min_bid_ask_ratio: Some(0.2),
                max_bid_ask_ratio: Some(5.0),
                ..Default::default()
            },
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
    fn test_markdown_leaves_out_the_peg_deviation_when_there_is_none() {
        // A peg target of 0 has no deviation: no "(+0.000%)", the same as the text report.
        use scope::market::{BookSource, OrderBook, OrderBookLevel};
        let level = |price| OrderBookLevel {
            price,
            quantity: 100.0,
        };
        let book = OrderBook {
            pair: "USDC/USDT".to_string(),
            source: BookSource::Exchange,
            bids: vec![level(0.9999)],
            asks: vec![level(1.0001)],
        };
        let thresholds = HealthThresholds {
            peg_target: 0.0,
            ..HealthThresholds::default()
        };
        let summary = MarketSummary::from_order_book(&book, &thresholds, None);
        let md = market_summary_to_markdown(&summary, "binance", "USDC");
        assert!(md.contains("0.9999") && !md.contains("0.9999 ("), "{md}");
        assert!(!md.contains("(+0.000%)"), "{md}");
    }

    #[test]
    fn test_market_summary_to_markdown_with_execution_estimates() {
        use scope::market::{ExecutionEstimate, ExecutionSide, HealthCheck, MarketSummary};
        let summary = MarketSummary {
            pair: "TESTUSDT".to_string(),
            source: scope::market::BookSource::Exchange,
            peg_target: 1.0,
            peg_range: scope::market::health::DEFAULT_PEG_RANGE,
            best_bid: Some(0.9999),
            best_ask: Some(1.0001),
            mid_price: Some(1.0000),
            spread: Some(0.0002),
            spread_pct: None,
            top_depth: Default::default(),
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
            token_contract: None,
            chain: None,
            health: HealthArgs {
                peg_target: Some(1.0),
                peg_range: Some(0.01),
                min_levels: Some(1),
                min_depth: Some(50.0),
                min_bid_ask_ratio: Some(0.1),
                max_bid_ask_ratio: Some(10.0),
                ..Default::default()
            },
            format: SummaryFormat::Text,
            every: None,
            duration: None,
            report: None,
            csv: None,
        });

        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
        };
        let config = Config::default();
        let _result = run(args, &config, &factory, &quiet()).await;
        // Don't assert success - depends on network
    }

    #[test]
    fn test_is_dex_venue() {
        assert!(DexVenue::parse("eth").is_some());
        assert!(DexVenue::parse("ethereum").is_some());
        assert!(DexVenue::parse("Ethereum").is_some());
        assert!(DexVenue::parse("solana").is_some());
        assert!(DexVenue::parse("Solana").is_some());
        assert!(DexVenue::parse("binance").is_none());
        assert!(DexVenue::parse("okx").is_none());
        assert!(DexVenue::parse("mexc").is_none());
    }

    #[test]
    fn test_dex_venue_to_chain() {
        assert_eq!(
            DexVenue::parse("eth").map(DexVenue::chain),
            Some("ethereum")
        );
        assert_eq!(
            DexVenue::parse("ethereum").map(DexVenue::chain),
            Some("ethereum")
        );
        assert_eq!(
            DexVenue::parse("Ethereum").map(DexVenue::chain),
            Some("ethereum")
        );
        assert_eq!(
            DexVenue::parse("solana").map(DexVenue::chain),
            Some("solana")
        );
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
            token_contract: None,
            chain: None,
            health: HealthArgs {
                peg_target: Some(1.0),
                peg_range: Some(0.01),
                min_levels: Some(1),
                min_depth: Some(50.0),
                min_bid_ask_ratio: Some(0.1),
                max_bid_ask_ratio: Some(10.0),
                ..Default::default()
            },
            format: SummaryFormat::Json,
            every: None,
            duration: None,
            report: None,
            csv: None,
        };

        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
        };
        let _result = run_summary(args, &Config::default(), &factory, &quiet()).await;
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
            token_contract: None,
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
            token_contract: None,
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
            token_contract: None,
            interval: "1h".to_string(),
            limit: 10,
            format: OhlcFormat::Text,
        };
        let result = run_ohlc(args, &quiet()).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Unknown venue"),
            "expected 'Unknown venue' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_run_trades_unknown_venue() {
        let args = TradesArgs {
            pair: "BTC".to_string(),
            venue: "nonexistent_venue".to_string(),
            token_contract: None,
            limit: 10,
            format: OhlcFormat::Text,
        };
        let result = run_trades(args, &quiet()).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Unknown venue"),
            "expected 'Unknown venue' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_run_dispatches_ohlc() {
        let cmd = MarketCommands::Ohlc(OhlcArgs {
            pair: "BTC".to_string(),
            venue: "nonexistent_test_venue".to_string(),
            token_contract: None,
            interval: "1h".to_string(),
            limit: 5,
            format: OhlcFormat::Text,
        });
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
        };
        let config = Config::default();
        let result = run(cmd, &config, &factory, &quiet()).await;
        // Should error with venue not found
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_run_dispatches_trades() {
        let cmd = MarketCommands::Trades(TradesArgs {
            pair: "ETH".to_string(),
            venue: "nonexistent_test_venue".to_string(),
            token_contract: None,
            limit: 5,
            format: OhlcFormat::Json,
        });
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
        };
        let config = Config::default();
        let result = run(cmd, &config, &factory, &quiet()).await;
        assert!(result.is_err());
    }

    /// A venue at `server` whose coin list is `/coins`.
    fn venue_with_coins(server: &mockito::ServerGuard) -> ExchangeClient {
        let mut desc = VenueRegistry::load()
            .unwrap()
            .get("binance")
            .unwrap()
            .clone();
        desc.base_url = server.url();
        desc.capabilities.assets =
            Some(serde_yaml::from_str("path: /coins\nresponse: {}\n").unwrap());
        ExchangeClient::from_descriptor(&desc)
    }

    async fn coins(server: &mut mockito::ServerGuard, body: &str) -> mockito::Mock {
        server
            .mock("GET", "/coins")
            .with_status(200)
            .with_body(body)
            .create_async()
            .await
    }

    #[tokio::test]
    async fn test_a_cex_run_stops_when_the_saved_token_is_not_the_venues() {
        // A saved USDN on ethereum (0xAAA); the venue lists USDN on ETH as
        // 0xBBB. The summary must not report on the venue's token.
        let mut server = mockito::Server::new_async().await;
        let _m = coins(
            &mut server,
            r#"[{"coin":"USDN","networkList":[{"network":"ETH","contractAddress":"0xBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"}]}]"#,
        )
        .await;
        let client = venue_with_coins(&server);
        let mut store = scope::tokens::TokenAliases::default();
        store.add_binding(
            "USDN",
            "ethereum",
            "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "USDN",
            Some("binance"),
        );
        let err = check_cex_token(
            &client,
            "USDN",
            Some("ethereum".into()),
            None,
            &mut store,
            false,
            &quiet(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ScopeError::TokenMismatch { .. }), "{err}");
    }

    #[tokio::test]
    async fn test_a_cex_symbol_on_two_networks_stops_without_a_terminal() {
        let mut server = mockito::Server::new_async().await;
        let _m = coins(
            &mut server,
            r#"[{"coin":"USDN","networkList":[
                {"network":"ETH","contractAddress":"0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"},
                {"network":"BSC","contractAddress":"0xBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"}]}]"#,
        )
        .await;
        let client = venue_with_coins(&server);
        let mut store = scope::tokens::TokenAliases::default();
        let err = check_cex_token(&client, "USDN", None, None, &mut store, false, &quiet())
            .await
            .unwrap_err();
        assert!(matches!(err, ScopeError::AmbiguousToken { .. }), "{err}");
        // With --token the run goes on.
        assert!(
            check_cex_token(
                &client,
                "USDN",
                None,
                Some("0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into()),
                &mut store,
                false,
                &quiet()
            )
            .await
            .is_ok()
        );
    }

    /// The binance mapping with a local reply, so the test needs no network (#39).
    fn binance_at(server: &mockito::ServerGuard) -> ExchangeClient {
        scope::market::testing::venue_at("binance", &server.url())
    }

    const KLINES: &str = r#"[[1700000000000,"1.0","1.5","0.5","1.25","1000.5",1700003599999],
        [1700003600000,"1.25","2.0","1.0","1.75","20.25",1700007199999]]"#;
    const TRADES: &str = r#"[{"id":1,"price":"1.5","qty":"2.5","quoteQty":"3.75","time":1700000000000,"isBuyerMaker":true},
        {"id":2,"price":"1.6","qty":"1.0","quoteQty":"1.6","time":1700000001000,"isBuyerMaker":false}]"#;

    fn ohlc_args(format: OhlcFormat) -> OhlcArgs {
        OhlcArgs {
            pair: "BTC".to_string(),
            venue: "binance".to_string(),
            token_contract: None,
            interval: "1h".to_string(),
            limit: 2,
            format,
        }
    }

    fn trades_args(format: OhlcFormat) -> TradesArgs {
        TradesArgs {
            pair: "BTC".to_string(),
            venue: "binance".to_string(),
            token_contract: None,
            limit: 2,
            format,
        }
    }

    async fn serve(path: &str, body: &str) -> mockito::ServerGuard {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", path)
            .match_query(mockito::Matcher::UrlEncoded(
                "symbol".into(),
                "BTCUSDT".into(),
            ))
            .with_body(body)
            .create_async()
            .await;
        server
    }

    #[tokio::test]
    async fn test_ohlc_text_shows_every_candle() {
        let server = serve("/api/v3/klines", KLINES).await;
        let (out, cap) = Output::capture();
        run_ohlc_with(&binance_at(&server), ohlc_args(OhlcFormat::Text), &out)
            .await
            .unwrap();
        let text = crate::cli::ansi::strip(&cap.out());
        assert!(text.contains("OHLC — BTCUSDT (binance)"), "{text}");
        assert!(text.contains("1.250000"), "{text}");
        assert!(text.contains("1000.50"), "{text}");
        assert!(text.contains("2 candles returned"), "{text}");
    }

    #[tokio::test]
    async fn test_ohlc_json_keeps_every_field() {
        let server = serve("/api/v3/klines", KLINES).await;
        let (out, cap) = Output::capture();
        run_ohlc_with(&binance_at(&server), ohlc_args(OhlcFormat::Json), &out)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&cap.out()).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2);
        assert_eq!(v[0]["open_time"], 1_700_000_000_000u64);
        assert_eq!(v[1]["close"], 1.75);
        assert_eq!(v[1]["volume"], 20.25);
    }

    #[tokio::test]
    async fn test_trades_text_shows_side_price_and_quantity() {
        let server = serve("/api/v3/trades", TRADES).await;
        let (out, cap) = Output::capture();
        run_trades_with(&binance_at(&server), trades_args(OhlcFormat::Text), &out)
            .await
            .unwrap();
        let text = crate::cli::ansi::strip(&cap.out());
        assert!(text.contains("Recent Trades — BTCUSDT (binance)"), "{text}");
        // isBuyerMaker true is a sell; false is a buy.
        assert!(text.contains("SELL") && text.contains("BUY"), "{text}");
        assert!(text.contains("1.500000") && text.contains("2.50"), "{text}");
        assert!(text.contains("2 trades returned"), "{text}");
    }

    #[tokio::test]
    async fn test_trades_json_keeps_every_field() {
        let server = serve("/api/v3/trades", TRADES).await;
        let (out, cap) = Output::capture();
        run_trades_with(&binance_at(&server), trades_args(OhlcFormat::Json), &out)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&cap.out()).unwrap();
        assert_eq!(v[0]["side"], "sell");
        assert_eq!(v[1]["side"], "buy");
        assert_eq!(v[0]["id"], "1");
        assert_eq!(v[0]["quote_quantity"], 3.75);
        assert_eq!(v[1]["timestamp_ms"], 1_700_000_001_000u64);
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
    // An unknown venue is not a DEX venue (it used to map to ethereum)
    // ====================================================================

    #[test]
    fn test_unknown_venue_is_not_a_dex_venue() {
        assert_eq!(DexVenue::parse("binance"), None);
        assert_eq!(DexVenue::parse("kraken"), None);
        assert_eq!(DexVenue::parse("unknown"), None);
    }

    // ====================================================================
    // market_summary_to_markdown — mixed Pass/Fail checks
    // ====================================================================

    #[test]
    fn test_market_summary_to_markdown_mixed_pass_fail_checks() {
        use scope::market::{HealthCheck, MarketSummary};
        let summary = MarketSummary {
            pair: "TESTUSDT".to_string(),
            source: scope::market::BookSource::Exchange,
            peg_target: 1.0,
            peg_range: scope::market::health::DEFAULT_PEG_RANGE,
            best_bid: Some(0.9999),
            best_ask: Some(1.0001),
            mid_price: Some(1.0000),
            spread: Some(0.0002),
            spread_pct: None,
            top_depth: Default::default(),
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
        use scope::market::MarketSummary;
        let summary = MarketSummary {
            pair: "X".to_string(),
            source: scope::market::BookSource::Exchange,
            peg_target: 1.0,
            peg_range: scope::market::health::DEFAULT_PEG_RANGE,
            best_bid: Some(1.0),
            best_ask: Some(1.0),
            mid_price: Some(1.0),
            spread: Some(0.0),
            spread_pct: None,
            top_depth: Default::default(),
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
        // No checks, no checks section.
        assert!(!md.contains("Health Checks:"));
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
            token_contract: None,
            chain: None,
            health: HealthArgs {
                peg_target: Some(1.0),
                peg_range: Some(0.01),
                min_levels: Some(1),
                min_depth: Some(50.0),
                min_bid_ask_ratio: Some(0.1),
                max_bid_ask_ratio: Some(10.0),
                ..Default::default()
            },
            format: SummaryFormat::Text,
            every: Some("0.1s".to_string()),
            duration: Some("1m".to_string()),
            report: None,
            csv: None,
        };
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: Default::default(),
            http,
        };
        let result = run_summary(args, &Config::default(), &factory, &quiet()).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Interval must be positive") || err.contains("positive"),
            "expected interval error, got: {}",
            err
        );
    }

    // ====================================================================
    // MarketCommands and struct Debug/construction
    // ====================================================================

    #[test]
    fn test_market_commands_debug() {
        let cmd = MarketCommands::Summary(SummaryArgs {
            pair: "USDC".to_string(),
            venue: "binance".to_string(),
            token_contract: None,
            chain: None,
            health: HealthArgs {
                peg_target: Some(1.0),
                peg_range: Some(0.001),
                min_levels: Some(6),
                min_depth: Some(3000.0),
                min_bid_ask_ratio: Some(0.2),
                max_bid_ask_ratio: Some(5.0),
                ..Default::default()
            },
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
            token_contract: None,
            interval: "1h".to_string(),
            limit: 100,
            format: OhlcFormat::Text,
        });
        assert!(format!("{:?}", ohlc_cmd).contains("Ohlc"));

        let trades_cmd = MarketCommands::Trades(TradesArgs {
            pair: "ETH".to_string(),
            venue: "binance".to_string(),
            token_contract: None,
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
            token_contract: None,
            chain: None,
            health: HealthArgs {
                peg_target: Some(1.0),
                peg_range: Some(0.001),
                min_levels: Some(6),
                min_depth: Some(3000.0),
                min_bid_ask_ratio: Some(0.2),
                max_bid_ask_ratio: Some(5.0),
                ..Default::default()
            },
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
        assert_eq!(Pair::parse("USDT").base, "USDT");
    }

    #[test]
    fn test_market_summary_to_markdown_unhealthy() {
        use scope::market::{HealthCheck, MarketSummary};
        let summary = MarketSummary {
            pair: "X".to_string(),
            source: scope::market::BookSource::Exchange,
            peg_target: 1.0,
            peg_range: scope::market::health::DEFAULT_PEG_RANGE,
            best_bid: Some(0.99),
            best_ask: Some(1.01),
            mid_price: Some(1.0),
            spread: Some(0.02),
            spread_pct: None,
            top_depth: Default::default(),
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
            token_contract: None,
            limit: 50,
            format: OhlcFormat::Text,
        };
        assert_eq!(args.pair, "USDC");
        assert_eq!(args.venue, "binance");
    }

    // ------------------------------------------------------------------
    // Offline DEX-path tests (mock factory, no network)
    // ------------------------------------------------------------------

    const USDC_ADDR: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";

    /// Mock factory whose DEX data has the given pairs (liquidity, volume).
    fn dex_factory(pairs: &[(f64, f64)]) -> scope::chains::mocks::MockClientFactory {
        let mut factory = scope::chains::mocks::MockClientFactory::new();
        let data = factory.mock_dex.token_data.as_mut().unwrap();
        data.symbol = "USDC".to_string();
        data.pairs = pairs
            .iter()
            .enumerate()
            .map(|(i, &(liquidity_usd, volume_24h))| scope::chains::DexPair {
                dex_name: format!("DEX {i}"),
                pair_address: format!("0xpair{i}"),
                base_token: "USDC".to_string(),
                quote_token: "USDT".to_string(),
                price_usd: 1.0,
                volume_24h,
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
            })
            .collect();
        factory
    }

    fn dex_args(format: SummaryFormat) -> SummaryArgs {
        SummaryArgs {
            pair: USDC_ADDR.to_string(),
            venue: "eth".to_string(),
            token_contract: None,
            chain: None,
            health: HealthArgs::default(),
            format,
            every: None,
            duration: None,
            report: None,
            csv: None,
        }
    }

    #[tokio::test]
    async fn test_dex_book_uses_most_liquid_pair() {
        // The synthetic book must come from the deepest pool, not the first one.
        let factory = dex_factory(&[(1_000.0, 10.0), (50_000.0, 777.0), (2_000.0, 20.0)]);
        let (book, volume) = fetch_book_and_volume(
            &dex_args(SummaryFormat::Text),
            &HealthThresholds::default(),
            &factory,
            &quiet(),
        )
        .await
        .unwrap();
        assert_eq!(volume, Some(777.0));
        let depth = book.total_depth(scope::market::BookSide::Bid)
            + book.total_depth(scope::market::BookSide::Ask);
        assert!((depth - 50_000.0).abs() < 1.0, "depth {depth}");
    }

    /// `--venue sol` is the Solana DEX (it used to go to the CEX registry
    /// and fail with "unknown venue").
    #[tokio::test]
    async fn test_venue_sol_reads_the_solana_dex() {
        let factory = dex_factory(&[(50_000.0, 777.0)]);
        let mut args = dex_args(SummaryFormat::Text);
        args.venue = "sol".to_string();
        let result =
            fetch_book_and_volume(&args, &HealthThresholds::default(), &factory, &quiet()).await;
        let (book, volume) = result.unwrap();
        assert_eq!(volume, Some(777.0));
        assert_eq!(book.pair, "USDC/USD");
    }

    /// `--chain` is one rule with the venue: a chain that is not the DEX
    /// venue's chain is an error, in any case.
    #[tokio::test]
    async fn test_chain_must_match_the_dex_venue() {
        let factory = dex_factory(&[(50_000.0, 1.0)]);
        let mut args = dex_args(SummaryFormat::Text);
        args.chain = Some("Solana".to_string());
        let err = fetch_book_and_volume(&args, &HealthThresholds::default(), &factory, &quiet())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("reads chain ethereum"), "{err}");

        args.chain = Some("ETH".to_string());
        assert!(
            fetch_book_and_volume(&args, &HealthThresholds::default(), &factory, &quiet())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_unknown_venue_is_an_error() {
        let factory = dex_factory(&[(50_000.0, 1.0)]);
        let mut args = dex_args(SummaryFormat::Text);
        args.venue = "polygon".to_string();
        let err = fetch_book_and_volume(&args, &HealthThresholds::default(), &factory, &quiet())
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("Unknown venue \"polygon\""),
            "{err}"
        );
    }

    #[tokio::test]
    async fn test_dex_no_pairs_is_error() {
        let factory = dex_factory(&[]);
        let err = fetch_book_and_volume(
            &dex_args(SummaryFormat::Text),
            &HealthThresholds::default(),
            &factory,
            &quiet(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("No DEX pairs found"), "{err}");
    }

    #[tokio::test]
    async fn test_run_summary_once_json_reflects_resolved_thresholds() {
        let factory = dex_factory(&[(50_000.0, 1_000.0)]);
        let mut args = dex_args(SummaryFormat::Json);
        // A one-level synthetic book is healthy only with loosened thresholds.
        args.health = HealthArgs {
            min_levels: Some(1),
            min_top10_depth: Some(1.0),
            ..Default::default()
        };
        let thresholds = args.health.resolve(&Config::default());
        let (summary, _) = summary_run(&args, &factory, &thresholds, &[], Some(1), &quiet())
            .await
            .unwrap();
        assert!(summary.healthy, "{:?}", summary.checks);

        let strict = HealthThresholds::default();
        let (summary, _) = summary_run(&args, &factory, &strict, &[], None, &quiet())
            .await
            .unwrap();
        assert!(!summary.healthy);
    }

    #[tokio::test]
    async fn test_dex_json_shows_amm_source_and_na_checks() {
        // #37 acceptance: JSON shows "status": "n/a", and the book source.
        let mut factory = dex_factory(&[(20_000_000.0, 1_000.0)]);
        factory.mock_dex.token_data.as_mut().unwrap().pairs[0].liquidity_base = Some(10_000_000.0);
        let args = dex_args(SummaryFormat::Json);
        let thresholds = args.health.resolve(&Config::default());
        let (summary, report) = summary_run(&args, &factory, &thresholds, &[], None, &quiet())
            .await
            .unwrap();
        assert!(summary.healthy, "{:?}", summary.checks);
        let v: serde_json::Value = serde_json::from_str(&report).unwrap();
        assert_eq!(v["book_source"], "synthetic_amm");
        let statuses: Vec<&str> = v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["status"].as_str().unwrap())
            .collect();
        assert_eq!(
            statuses.iter().filter(|s| **s == "n/a").count(),
            3,
            "{:?}",
            statuses
        );
        assert!(!statuses.contains(&"fail"));
    }

    #[tokio::test]
    async fn test_dex_without_reserves_says_estimate() {
        let factory = dex_factory(&[(50_000.0, 1_000.0)]);
        let args = dex_args(SummaryFormat::Text);
        let thresholds = args.health.resolve(&Config::default());
        let (_, report) = summary_run(&args, &factory, &thresholds, &[], None, &quiet())
            .await
            .unwrap();
        assert!(report.contains("pool reserves not reported"), "{report}");
    }

    #[tokio::test]
    async fn test_run_summary_one_shot_writes_report() {
        let dir = tempfile::tempdir().unwrap();
        let report = dir.path().join("report.md");
        let factory = dex_factory(&[(50_000.0, 1_000.0)]);
        let mut args = dex_args(SummaryFormat::Text);
        args.report = Some(report.clone());
        run_summary(args, &Config::default(), &factory, &quiet())
            .await
            .unwrap();
        let md = std::fs::read_to_string(&report).unwrap();
        assert!(md.contains("Market Health Report"), "{md}");
        assert!(md.contains(USDC_ADDR), "{md}");
    }

    #[tokio::test]
    async fn test_run_summary_repeat_mode_writes_csv_rows_and_report() {
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("series.csv");
        let report = dir.path().join("final.md");
        let factory = dex_factory(&[(50_000.0, 1_000.0)]);
        let mut args = dex_args(SummaryFormat::Json);
        args.every = Some("1s".to_string());
        args.duration = Some("1s".to_string());
        args.csv = Some(csv.clone());
        args.report = Some(report.clone());
        run_summary(args, &Config::default(), &factory, &quiet())
            .await
            .unwrap();

        let body = std::fs::read_to_string(&csv).unwrap();
        let mut lines = body.lines();
        // Today's columns stay first and in order; #58 appends the rest.
        let header = lines.next().unwrap();
        assert!(
            header.starts_with(
                "timestamp,run,best_bid,best_ask,mid_price,spread,bid_depth,ask_depth,healthy,"
            ),
            "{header}"
        );
        assert!(
            header.contains("spread_pct") && header.ends_with("ask_top10_depth"),
            "{header}"
        );
        // One run at t=0, one after the 1s sleep: the loop must not stop early.
        let rows: Vec<_> = lines.collect();
        assert_eq!(rows.len(), 2, "{body}");
        assert!(rows[0].split(',').nth(1) == Some("1"));
        assert!(rows[1].split(',').nth(1) == Some("2"));
        assert!(report.exists());
    }

    #[tokio::test]
    async fn test_run_summary_repeat_publishes_run_progress() {
        // The TUI draws `run N of M` from this hook, so the total must be
        // known at the start and the run number must advance.
        let (out, captured) = Output::capture_merged();
        let hook = out.progress_hook().expect("a merged capture has a hook");
        let factory = dex_factory(&[(50_000.0, 1_000.0)]);
        let mut args = dex_args(SummaryFormat::Json);
        args.every = Some("1s".to_string());
        args.duration = Some("2s".to_string());
        run_summary(args, &Config::default(), &factory, &out)
            .await
            .unwrap();
        let p = hook.lock().unwrap().clone();
        // Runs at 0s, 1s and 2s.
        assert_eq!(p.total, Some(3), "{p:?}");
        assert_eq!(p.run, 3, "{p:?}");
        assert_eq!(p.next_at, None, "no next run after the last: {p:?}");
        assert!(captured.out().contains("every 1s for 2s"));
        assert!(captured.out().contains("Completed 3 run(s)"));
    }

    #[tokio::test]
    async fn test_a_failed_run_ends_a_repeating_summary() {
        // A run that errors stops the repeat and returns the error, so the
        // pane footer is `✗ Failed in`. It does not skip to the next run.
        let (out, _captured) = Output::capture_merged();
        let hook = out.progress_hook().unwrap();
        let factory = dex_factory(&[]);
        let mut args = dex_args(SummaryFormat::Json);
        args.every = Some("1s".to_string());
        args.duration = Some("30s".to_string());
        let started = std::time::Instant::now();
        let result = run_summary(args, &Config::default(), &factory, &out).await;
        assert!(result.is_err(), "no DEX pairs is a failed run");
        assert_eq!(hook.lock().unwrap().run, 1, "no second run after the error");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "it did not sleep on"
        );
    }
}
