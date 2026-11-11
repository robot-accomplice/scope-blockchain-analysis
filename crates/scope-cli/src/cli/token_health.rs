//! # Token Health Command
//!
//! Composite command combining DEX analytics (crawl) with optional market/order book
//! summary for stablecoins. Produces a unified health report: liquidity, volume,
//! peg deviation, and order book depth.

use crate::cli::crawl::{self, Period};
use crate::cli::csv::{csv_line, defuse_formula};
use crate::cli::market_venues::{
    ComparisonRow, VenueReport, VenueSource, checked_venues, comparison_markdown, comparison_rows,
    comparison_table, one_line, pair_in, read_token_summaries,
};
use crate::cli::output::Output;
use crate::{out, outln};
use clap::Args;
use scope::chains::{ChainClientFactory, TokenAnalytics};
use scope::config::Config;

use super::health_args::HealthArgs;
use scope::display::report;
use scope::error::{Result, ScopeError};
use scope::market::{MarketSummary, MarketVenue, fetch_token_summary_by_name_with_http};

/// The long help of `--venue`: the list form, the JSON shape and the exit
/// code. The short help is `VENUE_HELP`.
const VENUE_LONG_HELP: &str = "Market venue: a CEX id (run `scope venues list`), or a DEX: \
    ethereum (eth) or solana (sol), whose book is built from the token's most liquid pool.\n\n\
    Name several venues to compare them: repeat --venue or give a comma list \
    (--venue weex,binance). The venues need --with-market. The DEX analytics are read once. \
    The output has one market block for each venue, in the order given, then a comparison \
    table. With --format json, `market` is null, and `markets` and `comparison` are added. \
    A venue that fails does not stop the others: its block says why. The exit code is 0 \
    when the DEX analytics print, even if a venue fails. An unhealthy venue does not change it.";

/// Arguments for the token-health command.
#[derive(Debug, Args)]
#[command(
    after_help = "\x1b[1mExamples:\x1b[0m
  scope token-health USDC
  scope token-health @dai-token --with-market             \x1b[2m# address book shortcut\x1b[0m
  scope token-health DAI --with-market --venue binance
  scope token-health USDC --with-market --venue weex,binance  \x1b[2m# compare venues\x1b[0m
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

    /// The token's contract: use it and skip the symbol lookup.
    #[arg(long = "token", value_name = "CONTRACT")]
    pub token_contract: Option<String>,

    /// Target blockchain network.
    #[arg(short, long, default_value = scope::chains::DEFAULT_CHAIN)]
    pub chain: String,

    /// Include order book / market summary (for stablecoins).
    #[arg(long)]
    pub with_market: bool,

    #[arg(
        long,
        default_value = scope::market::DEFAULT_VENUE,
        value_name = "VENUE",
        value_delimiter = ',',
        help = scope::market::VENUE_HELP,
        long_help = VENUE_LONG_HELP
    )]
    pub venue: Vec<String>,

    /// Output format. `csv` has one header and one row for each venue.
    ///
    /// With `csv`, numbers are raw (no `$`, `%` or `K`/`M`/`B`). The analytics columns are in USD, `price_change_24h` and `top_10_concentration` in percent. The market columns are in the quote currency of the venue, `spread_pct` in percent and `peg_deviation_bps` in basis points. A missing value is an empty cell. The header is in the README.
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
    // Check the venues first: a bad list fails before any request.
    let venues = checked_venues(&args.venue)?;
    let many = venues.len() > 1;
    if many && !args.with_market {
        return Err(ScopeError::InvalidInput(
            "--venue needs --with-market when it names 2 or more venues".to_string(),
        ));
    }

    // Resolve address book label → address + chain
    if let Some((address, chain)) =
        crate::cli::address_book::resolve_address_book_input(&args.token, config)?
    {
        args.token = address;
        if args.chain == scope::chains::DEFAULT_CHAIN {
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
        // --token: the contract is the input; an address passes through.
        args.token_contract.as_deref().unwrap_or(&args.token),
        &args.chain,
        Period::Hour24,
        10,
        clients,
        Some(&sp),
        out,
    )
    .await?;

    // 2. Optionally fetch market summary for stablecoin
    let mut reports: Vec<VenueReport> = Vec::new();
    // The reason a single venue failed: the table prints it as a warning,
    // the CSV keeps it in the data.
    let mut single_failure: Option<String> = None;
    let market_summary = if args.with_market && many {
        sp.set_message("Fetching market data...");
        let thresholds = args.health.resolve(config);
        let source: VenueSource<'_> =
            &|name| MarketVenue::from_name_with_http(name, clients.http());
        reports = read_token_summaries(&venues, source, &analytics, &thresholds).await;
        None
    } else if args.with_market {
        sp.set_message("Fetching market data...");
        let thresholds = args.health.resolve(config);
        let summary = fetch_token_summary_by_name_with_http(
            &venues[0],
            &analytics,
            &thresholds,
            clients.http(),
        )
        .await;
        match summary {
            Ok(summary) => Some(summary),
            Err(e) => {
                sp.println(&format!("  Warning: market data unavailable: {e}"));
                single_failure = Some(one_line(&e));
                None
            }
        }
    } else {
        None
    };

    sp.finish("Token health data loaded.");

    // 3. Output combined report
    if many {
        return print_many(format, &analytics, &reports, out);
    }
    if format == scope::config::OutputFormat::Csv {
        let reports = single_reports(&venues[0], args.with_market, market_summary, single_failure);
        out!(out, "{}", token_health_csv(&analytics, &reports))?;
        return Ok(());
    }
    let venue_label = if args.with_market {
        Some(venues[0].as_str())
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

/// The report on two or more venues (design 2026-11-10, section 4). The
/// DEX analytics print once. Each venue has its own market block, in the
/// order given, then one comparison. A failed venue has a block that says
/// why. The exit code does not depend on the venues: the analytics are the
/// main result.
fn print_many(
    format: scope::config::OutputFormat,
    analytics: &TokenAnalytics,
    reports: &[VenueReport],
    out: &Output,
) -> Result<()> {
    match format {
        scope::config::OutputFormat::Markdown => {
            outln!(out, "{}", token_health_many_to_markdown(analytics, reports))?;
        }
        scope::config::OutputFormat::Json => {
            outln!(out, "{}", token_health_many_to_json(analytics, reports)?)?;
        }
        scope::config::OutputFormat::Csv => {
            out!(out, "{}", token_health_csv(analytics, reports))?;
        }
        scope::config::OutputFormat::Table => {
            output_token_health_many_table(analytics, reports, out)?;
        }
    }
    Ok(())
}

/// The header of `--format csv`: the DEX analytics (9 columns), then the
/// market columns of one venue (11 columns). The order is fixed; a new
/// column goes at the end (design 2026-11-11, section 5).
const CSV_HEADER: &str = "symbol,chain,contract,price_usd,price_change_24h,volume_24h,\
    liquidity_usd,market_cap,top_10_concentration,venue,status,mid,spread_pct,bid_depth,ask_depth,\
    market_volume_24h,peg_deviation_bps,bot_signs,bot_signs_state,error";

/// The number of market columns (venue to error).
const CSV_MARKET_COLUMNS: usize = 11;

/// The separator of the bot sign names inside one cell.
const CSV_LIST_SEPARATOR: &str = ";";

/// The reports of the one-venue path, in the shape of the many-venue path,
/// so the CSV has one writer for 1 and N venues. Without `--with-market`
/// there is no report, and the CSV row has empty market cells.
fn single_reports(
    venue: &str,
    with_market: bool,
    summary: Option<MarketSummary>,
    failure: Option<String>,
) -> Vec<VenueReport> {
    if !with_market {
        return Vec::new();
    }
    let result = match summary {
        Some(summary) => Ok(summary),
        None => Err(failure.unwrap_or_default()),
    };
    vec![VenueReport {
        venue: venue.to_string(),
        tokens: Vec::new(),
        result,
    }]
}

/// A number as the JSON writes it (`f64` text, no rounding, no unit). A
/// missing or non-finite value is an empty cell.
fn csv_number(value: Option<f64>) -> String {
    value
        .filter(|v| v.is_finite())
        .map(|v| v.to_string())
        .unwrap_or_default()
}

/// The name a row enum has in the JSON `comparison` array.
fn csv_name(value: &impl serde::Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

/// The CSV of a token-health run: the header, then one row for each venue
/// in the order given (design 2026-11-11). The market cells come from the
/// same `ComparisonRow` as the tables and the JSON `comparison`. Without a
/// venue there is one row with empty market cells.
fn token_health_csv(analytics: &TokenAnalytics, reports: &[VenueReport]) -> String {
    let leading = [
        defuse_formula(&analytics.token.symbol),
        analytics.chain.clone(),
        analytics.token.contract_address.clone(),
        csv_number(Some(analytics.price_usd)),
        csv_number(Some(analytics.price_change_24h)),
        csv_number(Some(analytics.volume_24h)),
        csv_number(Some(analytics.liquidity_usd)),
        csv_number(analytics.market_cap),
        csv_number(analytics.top_10_concentration),
    ];
    let market: Vec<Vec<String>> = if reports.is_empty() {
        vec![vec![String::new(); CSV_MARKET_COLUMNS]]
    } else {
        comparison_rows(reports)
            .iter()
            .map(csv_market_cells)
            .collect()
    };
    let mut csv = format!("{CSV_HEADER}\n");
    for cells in market {
        let row: Vec<String> = leading.iter().cloned().chain(cells).collect();
        csv.push_str(&csv_line(&row));
    }
    csv
}

fn csv_market_cells(row: &ComparisonRow) -> Vec<String> {
    let signs = row.bot_signs.clone().unwrap_or_default();
    vec![
        row.venue.clone(),
        csv_name(&row.status),
        csv_number(row.mid),
        csv_number(row.spread_pct),
        csv_number(row.bid_depth),
        csv_number(row.ask_depth),
        csv_number(row.volume_24h),
        csv_number(row.peg_deviation_bps),
        defuse_formula(&signs.join(CSV_LIST_SEPARATOR)),
        csv_name(&row.bot_signs_state),
        defuse_formula(row.error.as_deref().unwrap_or_default()),
    ]
}

fn token_health_many_to_markdown(analytics: &TokenAnalytics, reports: &[VenueReport]) -> String {
    let mut md = report::generate_report(analytics);
    md.push_str("\n---\n\n## Market / Order Book\n\n");
    for r in reports {
        md.push_str(&format!("### {}\n\n", r.venue));
        match &r.result {
            Ok(summary) => md.push_str(&summary.format_markdown(Some(&r.venue))),
            Err(why) => md.push_str(&format!("No market data: {why}\n")),
        }
        md.push('\n');
    }
    md.push_str("### Comparison\n\n");
    md.push_str(&comparison_markdown(reports));
    md.push_str(&report::report_footer());
    md
}

/// `market` stays in the output and is `null`, so a script that reads it
/// for one venue gets a clear `null` and not a changed type.
fn token_health_many_to_json(
    analytics: &TokenAnalytics,
    reports: &[VenueReport],
) -> Result<String> {
    let markets: Vec<serde_json::Value> = reports
        .iter()
        .map(|r| match &r.result {
            Ok(s) => serde_json::json!({"venue": r.venue, "summary": s.to_json()}),
            Err(why) => serde_json::json!({"venue": r.venue, "error": why}),
        })
        .collect();
    let json = serde_json::json!({
        "analytics": analytics,
        "market": null,
        "markets": markets,
        "comparison": comparison_rows(reports),
    });
    serde_json::to_string_pretty(&json).map_err(ScopeError::Json)
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

    write_analytics_block(analytics, out)?;
    if let Some(summary) = market {
        write_market_block(MARKET_HEADING, summary, venue, out)?;
    }
    outln!(out, "{}", t::section_footer())?;
    Ok(())
}

/// The heading of the market block of one venue.
const MARKET_HEADING: &str = "Market / Order Book";

/// The report on two or more venues, as a table: the analytics once, one
/// market block for each venue, in the order given, then the comparison.
fn output_token_health_many_table(
    analytics: &TokenAnalytics,
    reports: &[VenueReport],
    out: &Output,
) -> Result<()> {
    use scope::display::terminal as t;

    write_analytics_block(analytics, out)?;
    for r in reports {
        let heading = format!("{MARKET_HEADING} ({})", r.venue);
        match &r.result {
            Ok(summary) => write_market_block(&heading, summary, Some(&r.venue), out)?,
            Err(why) => {
                outln!(out, "{}", t::subsection_header(&heading))?;
                outln!(out, "{}", t::warning_row(&format!("No market data: {why}")))?;
            }
        }
    }
    outln!(out, "{}", t::section_footer())?;
    out!(
        out,
        "{}",
        comparison_table(&pair_in(&analytics.token.symbol, reports), reports)
    )?;
    Ok(())
}

/// The title and the DEX Analytics rows.
fn write_analytics_block(analytics: &TokenAnalytics, out: &Output) -> Result<()> {
    use scope::display::terminal as t;

    let title = format!("{} ({})", analytics.token.symbol, analytics.token.name);
    outln!(out, "{}", t::section_header(&title))?;

    // DEX Analytics subsection
    outln!(out, "{}", t::subsection_header("DEX Analytics"))?;
    outln!(
        out,
        "{}",
        t::kv_row(
            "Price",
            &scope::display::format::format_price_usd(analytics.price_usd)
        )
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
            &scope::display::format_usd(analytics.volume_24h)
        )
    )?;
    outln!(
        out,
        "{}",
        t::kv_row(
            "Liquidity",
            &scope::display::format_usd(analytics.liquidity_usd)
        )
    )?;
    if let Some(mc) = analytics.market_cap {
        outln!(
            out,
            "{}",
            t::kv_row("Market Cap", &scope::display::format_usd(mc))
        )?;
    }
    if let Some(top10) = analytics.top_10_concentration {
        outln!(
            out,
            "{}",
            t::kv_row("Top 10 Holders", &format!("{:.1}%", top10))
        )?;
    }

    Ok(())
}

/// One market block: the venue, the peg and book rows, the health checks and
/// the status line.
fn write_market_block(
    heading: &str,
    summary: &MarketSummary,
    venue: Option<&str>,
    out: &Output,
) -> Result<()> {
    use scope::display::terminal as t;

    outln!(out, "{}", t::subsection_header(heading))?;
    if let Some(v) = venue {
        outln!(out, "{}", t::kv_row("Venue", v))?;
    }
    outln!(
        out,
        "{}",
        t::kv_row(
            "Peg Target",
            &format!(
                "{:.d$}",
                summary.peg_target,
                d = scope::display::format::price_decimals(summary.peg_target)
            ),
        )
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
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
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

    pub(crate) fn make_test_analytics(with_dex_pairs: bool) -> TokenAnalytics {
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
            bot_patterns: None,
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
            token_contract: None,
            chain: "ethereum".to_string(),
            with_market: false,
            venue: vec!["binance".to_string()],
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
        let err = fetch_token_summary_by_name_with_http(
            "sol",
            &analytics,
            &t,
            scope::http::test_transport(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("reads chain solana"), "{err}");
        assert!(
            fetch_token_summary_by_name_with_http(
                "ETH",
                &analytics,
                &t,
                scope::http::test_transport()
            )
            .await
            .is_ok()
        );
        // An unknown name is the one unknown-venue error (a warning here).
        let err = fetch_token_summary_by_name_with_http(
            "polygon",
            &analytics,
            &t,
            scope::http::test_transport(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("Unknown venue"), "{err}");
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

    pub(crate) fn make_test_dex_token_data(pairs: Vec<DexPair>) -> DexTokenData {
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
            token_contract: None,
            chain: "ethereum".to_string(),
            with_market: false,
            venue: vec!["binance".to_string()],
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
            token_contract: None,
            chain: "ethereum".to_string(),
            with_market: false,
            venue: vec!["binance".to_string()],
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
            token_contract: None,
            chain: "ethereum".to_string(),
            with_market: false,
            venue: vec!["binance".to_string()],
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
            token_contract: None,
            chain: "ethereum".to_string(),
            with_market: true,
            venue: vec!["eth".to_string()],
            format: OutputFormat::Table,
            health: Default::default(),
        };

        let result = run(args, &config, &factory, &quiet()).await;
        assert!(result.is_ok());
    }

    fn dex_market_args(venue: &str) -> TokenHealthArgs {
        TokenHealthArgs {
            token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            token_contract: None,
            chain: "ethereum".to_string(),
            with_market: true,
            venue: vec![venue.to_string()],
            format: OutputFormat::Json,
            health: Default::default(),
        }
    }

    pub(crate) fn pair_with_liquidity(liquidity_usd: f64) -> DexPair {
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
                token_contract: None,
                chain: "ethereum".to_string(),
                with_market: false,
                venue: vec!["binance".to_string()],
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
            token_contract: None,
            chain: "ethereum".to_string(),
            with_market: false,
            venue: vec!["binance".to_string()],
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
            token_contract: None,
            chain: "ethereum".to_string(),
            with_market: false,
            venue: vec!["binance".to_string()],
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

    // ---- venue list (design 2026-11-10, section 4) ----

    use crate::cli::ansi::strip;
    use clap::Parser;
    use scope::chains::dex::{DexDataSource, TokenSearchResult};
    use scope::chains::{ChainClient, ChainClientFactory};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The DEX mock, counting how many times the crawl reads the token. The
    /// analytics must be read once, however many venues the user names.
    struct CountingFactory {
        inner: MockClientFactory,
        token_reads: Arc<AtomicUsize>,
    }

    struct CountingDex {
        inner: Box<dyn DexDataSource>,
        token_reads: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl DexDataSource for CountingDex {
        async fn get_token_price(&self, chain: &str, address: &str) -> Option<f64> {
            self.inner.get_token_price(chain, address).await
        }
        async fn get_native_token_price(&self, chain: &str) -> Option<f64> {
            self.inner.get_native_token_price(chain).await
        }
        async fn get_token_data(&self, chain: &str, address: &str) -> Result<DexTokenData> {
            self.token_reads.fetch_add(1, Ordering::SeqCst);
            self.inner.get_token_data(chain, address).await
        }
        async fn search_tokens(
            &self,
            query: &str,
            chain: Option<&str>,
        ) -> Result<Vec<TokenSearchResult>> {
            self.inner.search_tokens(query, chain).await
        }
    }

    impl ChainClientFactory for CountingFactory {
        fn create_chain_client(&self, chain: &str) -> Result<Box<dyn ChainClient>> {
            self.inner.create_chain_client(chain)
        }
        fn create_dex_client(&self) -> Box<dyn DexDataSource> {
            Box::new(CountingDex {
                inner: self.inner.create_dex_client(),
                token_reads: self.token_reads.clone(),
            })
        }
        fn http(&self) -> Arc<dyn scope::http::HttpClient> {
            self.inner.http()
        }
    }

    fn counting_factory() -> CountingFactory {
        let mut inner = MockClientFactory::new();
        inner.mock_dex.token_data = Some(make_test_dex_token_data(vec![pair_with_liquidity(
            90_000.0,
        )]));
        CountingFactory {
            inner,
            token_reads: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn venue_args(venues: &[&str], with_market: bool, format: OutputFormat) -> TokenHealthArgs {
        TokenHealthArgs {
            with_market,
            venue: venues.iter().map(|v| v.to_string()).collect(),
            format,
            ..dex_market_args("eth")
        }
    }

    /// Runs the command and returns its result, its data and its notes.
    async fn run_captured(
        args: TokenHealthArgs,
        factory: &dyn ChainClientFactory,
    ) -> (Result<()>, String, String) {
        let (out, cap) = Output::capture();
        let result = run(args, &Config::default(), factory, &out).await;
        (result, strip(&cap.out()), strip(&cap.err()))
    }

    fn parse_venues(line: &str) -> Vec<String> {
        let argv = std::iter::once("scope").chain(line.split_whitespace());
        match crate::cli::Cli::try_parse_from(argv).unwrap().command {
            crate::cli::Commands::TokenHealth(a) => a.venue,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_venue_flag_repeats_or_takes_a_comma_list_like_market_summary() {
        // One flag shape for the user across `market`, `monitor` and here.
        let both = ["weex", "binance"];
        let base = "token-health USDC --with-market";
        assert_eq!(
            parse_venues(&format!("{base} --venue weex --venue binance")),
            both
        );
        assert_eq!(parse_venues(&format!("{base} --venue weex,binance")), both);
        assert_eq!(parse_venues(&format!("{base} --venue weex")), ["weex"]);
        assert_eq!(parse_venues(base), [scope::market::DEFAULT_VENUE]);
    }

    #[tokio::test]
    async fn a_bad_venue_list_fails_before_any_request() {
        // A typo must not cost a crawl: the check comes before the analytics.
        for (venues, with_market, why) in [
            (vec!["eth", "ETH"], true, "listed twice"),
            (vec!["eth", " "], true, "empty venue name"),
            (
                vec!["eth", "ethereum"],
                false,
                "--venue needs --with-market",
            ),
        ] {
            let factory = counting_factory();
            let (result, _, _) = run_captured(
                venue_args(&venues, with_market, OutputFormat::Table),
                &factory,
            )
            .await;
            let err = result.unwrap_err().to_string();
            assert!(err.contains(why), "{venues:?}: {err}");
            assert_eq!(factory.token_reads.load(Ordering::SeqCst), 0, "{venues:?}");
        }
    }

    #[tokio::test]
    async fn one_venue_without_with_market_keeps_the_old_silent_rule() {
        // Q4: only a list fails. A script that passes one --venue without
        // --with-market keeps working.
        let factory = counting_factory();
        let (result, text, _) =
            run_captured(venue_args(&["eth"], false, OutputFormat::Table), &factory).await;
        result.unwrap();
        assert!(!text.contains("Market / Order Book"), "{text}");
    }

    /// What `develop` printed for one venue with these mocks (`eth`, one
    /// pool of 90000 USD). The files in `tests/fixtures/venue_list/` were
    /// captured from `develop` at commit c09a5640c164, before the venue list
    /// existed, by running the same inputs through that commit's code. The
    /// code under test did not render them, so a change of the one-venue
    /// output fails the test. `csv` printed the same bytes as `table` then;
    /// its fixture is now written by hand (design 2026-11-11, section 8, test
    /// 14), and `one_venue_csv_equals_the_hand_typed_fixture` reads it.
    fn develop_output(format: OutputFormat) -> String {
        mask_run_details(&develop_bytes(format))
    }

    /// A report names the second it was made and the scope version. Neither
    /// is part of the layout under test, and the fixtures hold the values of
    /// the day they were captured. Both sides are masked.
    pub(crate) fn mask_run_details(text: &str) -> String {
        const TIME: &str = r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} UTC";
        const VERSION: &str = r"Scope v\d+\.\d+\.\d+";
        // JSON: the fetch time and the token age (hours since the pool began,
        // measured against now).
        const FETCHED_AT: &str = r#""fetched_at": \d+"#;
        const TOKEN_AGE: &str = r#""token_age_hours": [\d.]+"#;
        let time = regex::Regex::new(TIME).unwrap();
        let version = regex::Regex::new(VERSION).unwrap();
        let fetched = regex::Regex::new(FETCHED_AT).unwrap();
        let age = regex::Regex::new(TOKEN_AGE).unwrap();
        let text = time.replace_all(text, "<time>");
        let text = version.replace_all(&text, "Scope v<version>");
        let text = fetched.replace_all(&text, r#""fetched_at": <time>"#);
        age.replace_all(&text, r#""token_age_hours": <hours>"#)
            .into_owned()
    }

    fn develop_bytes(format: OutputFormat) -> String {
        let bytes = match format {
            OutputFormat::Table => {
                include_str!("../../tests/fixtures/venue_list/token_health_one_venue.table")
            }
            OutputFormat::Csv => {
                include_str!("../../tests/fixtures/venue_list/token_health_one_venue.csv")
            }
            OutputFormat::Markdown => {
                include_str!("../../tests/fixtures/venue_list/token_health_one_venue.markdown")
            }
            OutputFormat::Json => {
                include_str!("../../tests/fixtures/venue_list/token_health_one_venue.json")
            }
        };
        // A checkout may turn LF into CRLF; the output itself is LF.
        bytes.replace("\r\n", "\n")
    }

    #[tokio::test]
    async fn one_venue_gives_exactly_the_old_output_in_every_format() {
        // The compatibility promise: a script written for one venue reads
        // the same bytes after the list exists.
        // `csv` is not here: it is a real CSV now, and its fixture is typed by
        // hand (`one_venue_csv_equals_the_hand_typed_fixture`).
        for format in [
            OutputFormat::Table,
            OutputFormat::Markdown,
            OutputFormat::Json,
        ] {
            let factory = counting_factory();
            let (result, text, _) =
                run_captured(venue_args(&["eth"], true, format), &factory).await;
            result.unwrap();
            assert_eq!(
                mask_run_details(&text),
                develop_output(format),
                "{format:?}"
            );
        }
        // And the JSON keeps its two keys, `market` an object.
        let factory = counting_factory();
        let (_, text, _) =
            run_captured(venue_args(&["eth"], true, OutputFormat::Json), &factory).await;
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["analytics", "market"]);
        assert!(json["market"].is_object());
    }

    #[tokio::test]
    async fn two_venues_in_a_table_give_the_analytics_once_a_block_each_then_the_comparison() {
        let factory = counting_factory();
        let (result, text, _) = run_captured(
            venue_args(&["eth", "ethereum"], true, OutputFormat::Table),
            &factory,
        )
        .await;
        result.unwrap();
        assert_eq!(text.matches("DEX Analytics").count(), 1, "{text}");
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle}\n{text}"))
        };
        let (first, second, table) = (
            at("Market / Order Book (eth)"),
            at("Market / Order Book (ethereum)"),
            at("Comparison"),
        );
        assert!(first < second && second < table, "order\n{text}");
        assert!(
            !text[table..].contains("Market / Order Book"),
            "the table is the last block\n{text}"
        );
    }

    // --- `--format csv` (design 2026-11-11) ---------------------------------

    /// The header, typed out. A script reads columns by name or by position,
    /// so a change of name or order must fail here and send the author to the
    /// CHANGELOG rule.
    const EXPECTED_CSV_HEADER: &str = "symbol,chain,contract,price_usd,price_change_24h,volume_24h,liquidity_usd,market_cap,top_10_concentration,venue,status,mid,spread_pct,bid_depth,ask_depth,market_volume_24h,peg_deviation_bps,bot_signs,bot_signs_state,error";

    /// The number of cells in every row (9 analytics, 11 market).
    const EXPECTED_CSV_COLUMNS: usize = 20;

    /// Column positions the tests read.
    const COL_SYMBOL: usize = 0;
    const COL_PRICE_CHANGE: usize = 4;
    const COL_VENUE: usize = 9;
    const COL_STATUS: usize = 10;
    const COL_MID: usize = 11;
    const COL_BOT_SIGNS: usize = 17;
    const COL_ERROR: usize = 19;
    /// The columns that hold a number (or nothing).
    const NUMERIC_COLUMNS: [usize; 12] = [3, 4, 5, 6, 7, 8, 11, 12, 13, 14, 15, 16];

    /// A CSV reader by the rules of RFC 4180: it is the check that a row
    /// splits into the cells the writer meant.
    fn parse_csv(text: &str) -> Vec<Vec<String>> {
        let (mut rows, mut row, mut cell) = (Vec::new(), Vec::new(), String::new());
        let (mut quoted, mut chars) = (false, text.chars().peekable());
        while let Some(c) = chars.next() {
            match (quoted, c) {
                (true, '"') if chars.peek() == Some(&'"') => {
                    cell.push('"');
                    chars.next();
                }
                (true, '"') => quoted = false,
                (true, _) => cell.push(c),
                (false, '"') => quoted = true,
                (false, ',') => row.push(std::mem::take(&mut cell)),
                (false, '\n') => {
                    row.push(std::mem::take(&mut cell));
                    rows.push(std::mem::take(&mut row));
                }
                (false, _) => cell.push(c),
            }
        }
        assert!(
            !quoted && cell.is_empty() && row.is_empty(),
            "unfinished row"
        );
        rows
    }

    async fn csv_of(venues: &[&str], with_market: bool) -> Vec<Vec<String>> {
        let (result, text, _) = run_captured(
            venue_args(venues, with_market, OutputFormat::Csv),
            &counting_factory(),
        )
        .await;
        result.expect("the CSV prints and the exit code is 0");
        parse_csv(&text)
    }

    fn report(venue: &str, result: std::result::Result<MarketSummary, String>) -> VenueReport {
        VenueReport {
            venue: venue.to_string(),
            tokens: Vec::new(),
            result,
        }
    }

    #[test]
    fn the_csv_header_is_stable() {
        let csv = token_health_csv(&make_test_analytics(false), &[]);
        assert_eq!(csv.lines().next().unwrap(), EXPECTED_CSV_HEADER);
        assert_eq!(EXPECTED_CSV_HEADER.split(',').count(), EXPECTED_CSV_COLUMNS);
    }

    #[tokio::test]
    async fn the_header_is_the_same_for_one_venue_many_venues_and_no_market() {
        // A reader that handles one case must handle the others.
        for (venues, with_market) in [
            (vec!["eth"], false),
            (vec!["eth"], true),
            (vec!["eth", "ethereum"], true),
        ] {
            let rows = csv_of(&venues, with_market).await;
            assert_eq!(rows[0].join(","), EXPECTED_CSV_HEADER, "{venues:?}");
            assert!(rows.iter().all(|r| r.len() == EXPECTED_CSV_COLUMNS));
        }
    }

    #[tokio::test]
    async fn one_row_for_each_venue_in_the_order_given() {
        // The order is a promise made for every other format of this
        // command. The list is not in alphabetical order.
        let rows = csv_of(&["ethereum", "solana", "eth"], true).await;
        let venues: Vec<&str> = rows[1..].iter().map(|r| r[COL_VENUE].as_str()).collect();
        assert_eq!(venues, ["ethereum", "solana", "eth"]);
    }

    #[tokio::test]
    async fn no_market_gives_one_row_with_empty_market_cells() {
        // The analytics stay useful without --with-market.
        let rows = csv_of(&["eth"], false).await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][COL_SYMBOL], "USDC");
        assert!(
            rows[1][COL_VENUE..].iter().all(String::is_empty),
            "venue and status are empty too: {:?}",
            rows[1]
        );
    }

    #[test]
    fn a_failed_venue_has_a_row_with_status_error_and_the_reason() {
        // A reason holds a comma and a double quote: the row must still have
        // 20 cells, the number cells must be empty, and the venues around it
        // must not change.
        let why = "HTTP 500, body \"bad\"";
        let reports = [
            report("a", Ok(make_test_market_summary())),
            report("b", Err(why.to_string())),
            report("c", Ok(make_test_market_summary())),
        ];
        let rows = parse_csv(&token_health_csv(&make_test_analytics(false), &reports));
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|r| r.len() == EXPECTED_CSV_COLUMNS));
        assert_eq!(rows[2][COL_STATUS], "error");
        assert_eq!(rows[2][COL_ERROR], why);
        assert!(rows[2][COL_MID..COL_BOT_SIGNS].iter().all(String::is_empty));
        assert_eq!(rows[1][COL_STATUS], "healthy");
        assert_eq!(rows[3][COL_STATUS], "healthy");
        assert_eq!(rows[1][COL_ERROR], "");
    }

    #[tokio::test]
    async fn a_failed_venue_does_not_change_the_exit_code() {
        let rows = csv_of(&["eth", "solana"], true).await;
        assert_eq!(rows[2][COL_STATUS], "error");
        assert!(rows[2][COL_ERROR].contains("solana"), "{:?}", rows[2]);
    }

    #[tokio::test]
    async fn a_one_venue_failure_has_an_error_row() {
        // The table shows only a warning. A script that reads stdout must
        // see the failure in the data, and tell "no market data" from
        // "venue not asked".
        let rows = csv_of(&["solana"], true).await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][COL_VENUE], "solana");
        assert_eq!(rows[1][COL_STATUS], "error");
        assert!(rows[1][COL_ERROR].contains("reads chain"), "{:?}", rows[1]);
    }

    #[test]
    fn numbers_are_raw_not_formatted() {
        // A formatted cell ($, %, K/M/B, a thousands comma) breaks a sum.
        let reports = [report("a", Ok(make_test_market_summary()))];
        let rows = parse_csv(&token_health_csv(&make_test_analytics(false), &reports));
        let mut checked = 0;
        for col in NUMERIC_COLUMNS {
            let cell = &rows[1][col];
            if cell.is_empty() {
                continue;
            }
            assert!(cell.parse::<f64>().is_ok(), "column {col}: {cell}");
            assert!(
                !cell.contains(['$', '%', ',', 'K', 'M', 'B']),
                "column {col}: {cell}"
            );
            checked += 1;
        }
        assert!(checked >= 8, "the fixture fills most number cells");
    }

    #[test]
    fn missing_and_non_finite_numbers_are_empty_cells() {
        let mut analytics = make_test_analytics(false);
        analytics.price_usd = f64::NAN;
        analytics.volume_24h = f64::INFINITY;
        analytics.market_cap = None;
        let mut summary = make_test_market_summary();
        summary.mid_price = None;
        summary.volume_24h = Some(f64::NEG_INFINITY);
        let rows = parse_csv(&token_health_csv(&analytics, &[report("a", Ok(summary))]));
        let row = &rows[1];
        assert_eq!(row[3], "", "NaN price");
        assert_eq!(row[5], "", "infinite volume");
        assert_eq!(row[7], "", "unknown market cap");
        assert_eq!(row[COL_MID], "", "no mid");
        assert_eq!(row[15], "", "infinite venue volume");
        assert_eq!(row[6], "100000000", "a known value stays");
    }

    #[tokio::test]
    async fn the_cells_equal_the_json_comparison_for_the_same_run() {
        // The CSV and the JSON `comparison` come from the same rows. This
        // fails if someone gives the CSV its own source.
        let venues = ["eth", "solana", "ethereum"];
        let rows = csv_of(&venues, true).await;
        let (_, json, _) = run_captured(
            venue_args(&venues, true, OutputFormat::Json),
            &counting_factory(),
        )
        .await;
        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        let number = |v: &serde_json::Value| v.as_f64().map(|n| n.to_string()).unwrap_or_default();
        for (row, cmp) in rows[1..].iter().zip(json["comparison"].as_array().unwrap()) {
            assert_eq!(row[COL_VENUE], cmp["venue"].as_str().unwrap());
            assert_eq!(row[COL_STATUS], cmp["status"].as_str().unwrap());
            assert_eq!(row[11], number(&cmp["mid"]));
            assert_eq!(row[12], number(&cmp["spread_pct"]));
            assert_eq!(row[13], number(&cmp["bid_depth"]));
            assert_eq!(row[14], number(&cmp["ask_depth"]));
            assert_eq!(row[15], number(&cmp["volume_24h"]));
            assert_eq!(row[16], number(&cmp["peg_deviation_bps"]));
            assert_eq!(row[18], cmp["bot_signs_state"].as_str().unwrap());
            assert_eq!(row[COL_ERROR], cmp["error"].as_str().unwrap_or(""));
        }
    }

    #[test]
    fn text_that_looks_like_a_formula_is_neutralised_and_numbers_are_not() {
        let mut analytics = make_test_analytics(false);
        analytics.token.symbol = "=HYPERLINK(\"http://x\",\"y\")".to_string();
        let reports = [report("a", Err("-cmd|' /C calc'!A0".to_string()))];
        let rows = parse_csv(&token_health_csv(&analytics, &reports));
        assert_eq!(rows[1][COL_SYMBOL], "'=HYPERLINK(\"http://x\",\"y\")");
        assert_eq!(rows[1][COL_ERROR], "'-cmd|' /C calc'!A0");
        // A negative number is correct data.
        assert_eq!(rows[1][COL_PRICE_CHANGE], "-0.01");
    }

    #[tokio::test]
    async fn stdout_is_only_csv() {
        // A pipe user parses the whole of stdout. No box character, no
        // colour code, no blank line, and the failed venue is in the data.
        let (result, text, _) = run_captured(
            venue_args(&["eth", "solana"], true, OutputFormat::Csv),
            &counting_factory(),
        )
        .await;
        result.unwrap();
        assert!(!text.contains(['\u{1b}', '│', '┌', '─', '└']), "{text}");
        assert!(text.ends_with('\n') && !text.ends_with("\n\n"), "{text:?}");
        let rows = parse_csv(&text);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.len() == EXPECTED_CSV_COLUMNS));
    }

    #[tokio::test]
    async fn the_ai_flag_still_prints_markdown_for_csv() {
        // --ai sets the config format to Markdown, and that wins, as before.
        let mut config = Config::default();
        config.output.format = OutputFormat::Markdown;
        let (out, cap) = Output::capture();
        run(
            venue_args(&["eth"], true, OutputFormat::Csv),
            &config,
            &counting_factory(),
            &out,
        )
        .await
        .unwrap();
        let text = strip(&cap.out());
        assert!(text.contains("## Market / Order Book"), "{text}");
        assert!(!text.starts_with(EXPECTED_CSV_HEADER), "{text}");
    }

    #[tokio::test]
    async fn one_venue_csv_equals_the_hand_typed_fixture() {
        // The fixture is typed by hand from the design's column rules and the
        // mock inputs. It is not captured from the writer. No mask: the CSV
        // holds no time and no version.
        let (result, text, _) = run_captured(
            venue_args(&["eth"], true, OutputFormat::Csv),
            &counting_factory(),
        )
        .await;
        result.unwrap();
        assert_eq!(text, develop_bytes(OutputFormat::Csv));
    }

    #[tokio::test]
    async fn two_venues_one_failed_equals_the_hand_typed_fixture() {
        let (result, text, _) = run_captured(
            venue_args(&["eth", "solana"], true, OutputFormat::Csv),
            &counting_factory(),
        )
        .await;
        result.unwrap();
        let fixture =
            include_str!("../../tests/fixtures/venue_list/token_health_two_venues_one_failed.csv")
                .replace("\r\n", "\n");
        assert_eq!(text, fixture);
    }

    #[tokio::test]
    async fn two_venues_in_json_keep_market_null_and_add_markets_and_comparison() {
        let factory = counting_factory();
        let (result, text, _) = run_captured(
            venue_args(&["eth", "ethereum"], true, OutputFormat::Json),
            &factory,
        )
        .await;
        result.unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(json["market"].is_null(), "a clear null, not a changed type");
        assert!(json["analytics"].is_object());
        let markets = json["markets"].as_array().unwrap();
        let venues: Vec<&str> = markets
            .iter()
            .map(|m| m["venue"].as_str().unwrap())
            .collect();
        assert_eq!(venues, ["eth", "ethereum"]);
        assert!(markets[0]["summary"]["healthy"].is_boolean());
        let rows = json["comparison"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r["mid"].is_number()), "raw numbers");
    }

    #[tokio::test]
    async fn two_venues_in_markdown_give_a_section_each_then_the_comparison() {
        let factory = counting_factory();
        let (result, text, _) = run_captured(
            venue_args(&["eth", "ethereum"], true, OutputFormat::Markdown),
            &factory,
        )
        .await;
        result.unwrap();
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle}\n{text}"))
        };
        let (head, a, b, cmp) = (
            at("## Market / Order Book"),
            at("### eth\n"),
            at("### ethereum\n"),
            at("### Comparison"),
        );
        assert!(head < a && a < b && b < cmp, "{text}");
        assert!(text[cmp..].contains("| ethereum |"), "{text}");
    }

    #[tokio::test]
    async fn a_failing_venue_does_not_hide_the_others_and_the_exit_code_stays_zero() {
        // `solana` cannot price an ethereum token; `nowhere` is unknown. The
        // analytics are the main result, so the code is 0 (decision Q5).
        let venues = ["solana", "eth", "nowhere"];
        let factory = counting_factory();
        let (result, text, _) =
            run_captured(venue_args(&venues, true, OutputFormat::Table), &factory).await;
        result.expect("a failed venue is a warning, as with one venue");
        assert!(text.contains("Market / Order Book (solana)"), "{text}");
        assert!(text.contains("No market data: "), "{text}");
        assert!(text.contains("Market / Order Book (eth)"), "{text}");
        assert!(
            text.contains("Peg Target"),
            "the good block is whole\n{text}"
        );

        let (result, text, _) =
            run_captured(venue_args(&venues, true, OutputFormat::Json), &factory).await;
        result.unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let markets = json["markets"].as_array().unwrap();
        assert!(markets[0]["error"].as_str().unwrap().contains("solana"));
        assert!(markets[1]["summary"].is_object(), "eth still answers");
        assert!(
            markets[2]["error"]
                .as_str()
                .unwrap()
                .contains("Unknown venue")
        );
        let status: Vec<&str> = json["comparison"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["status"].as_str().unwrap())
            .collect();
        assert_eq!(status[0], "error");
        assert_ne!(status[1], "error");
        assert_eq!(status[2], "error");
    }

    #[tokio::test]
    async fn every_venue_failing_still_prints_the_analytics_and_exits_zero() {
        let factory = counting_factory();
        let (result, text, _) = run_captured(
            venue_args(&["nowhere", "solana"], true, OutputFormat::Table),
            &factory,
        )
        .await;
        result.unwrap();
        assert!(
            text.contains("DEX Analytics") && text.contains("Comparison"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn the_analytics_are_read_once_for_any_number_of_venues() {
        // The DEX crawl is the slow, rate-limited part. A loop that reran it
        // for each venue would triple the cost for no new data.
        let (one, three) = (counting_factory(), counting_factory());
        run_captured(venue_args(&["eth"], true, OutputFormat::Json), &one)
            .await
            .0
            .unwrap();
        run_captured(
            venue_args(&["eth", "ethereum", "nowhere"], true, OutputFormat::Json),
            &three,
        )
        .await
        .0
        .unwrap();
        let reads = one.token_reads.load(Ordering::SeqCst);
        assert!(reads >= 1);
        assert_eq!(three.token_reads.load(Ordering::SeqCst), reads);
    }

    #[test]
    fn the_help_names_the_list_form_and_the_exit_rule() {
        use clap::CommandFactory;
        let cli = crate::cli::Cli::command();
        let help = cli
            .find_subcommand("token-health")
            .unwrap()
            .clone()
            .render_long_help()
            .to_string();
        assert!(help.contains("--venue weex,binance"), "{help}");
        assert!(help.contains("The exit code is 0"), "{help}");
    }

    #[test]
    fn the_format_help_names_the_csv_unit_rule() {
        // A raw number has no unit in the cell, so the help must say it.
        use clap::CommandFactory;
        let help = crate::cli::Cli::command()
            .find_subcommand("token-health")
            .unwrap()
            .clone()
            .render_long_help()
            .to_string();
        assert!(
            help.contains("one header and one row for each venue"),
            "{help}"
        );
        assert!(help.contains("basis points"), "{help}");
    }
}
