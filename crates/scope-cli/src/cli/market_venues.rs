//! `scope market summary`, `ohlc` and `trades` on two or more venues
//! (design 2026-11-09).
//!
//! One venue keeps the code in `market.rs`, byte for byte. This module is
//! the rest: the venue list, the concurrent summary read with a comparison
//! table, the blocks of `ohlc` and `trades`, the JSON shapes and the exit
//! rule. A venue that fails never stops the others: its section says why
//! and its table row shows the error.

use crate::cli::market::{
    self, DepthHistory, OhlcArgs, OhlcFormat, SummaryArgs, SummaryFormat, TradesArgs,
};
use crate::cli::output::{Output, VenueRun, VenueSample};
use crate::cli::repeat::{self, CsvRows, RunOutput, Schedule};
use crate::{note, out, outln};
use scope::chains::ChainClientFactory;
use scope::display::terminal as t;
use scope::error::{Result, ScopeError};
use scope::market::assets::TokenAddress;
use scope::market::{ExchangeClient, HealthThresholds, MarketSummary, MarketVenue, Pair};
use scope::metrics::{self as series, Sample};

/// How the commands get a venue from its name. The CLI reads the registry;
/// a test serves a local mock.
pub(crate) type VenueSource<'a> = &'a (dyn Fn(&str) -> Result<MarketVenue> + Sync);

/// The same, for the commands that read a CEX client only.
pub(crate) type ClientSource<'a> = &'a (dyn Fn(&str) -> Result<ExchangeClient> + Sync);

/// The venues of `--venue`, in the order given. `--venue` repeats or takes
/// a comma list (clap splits it, as for `scope monitor --venue`). A name
/// twice is an error, as in a watchlist file: two reads of one venue would
/// give two rows that look like two venues.
pub(crate) fn checked_venues(list: &[String]) -> Result<Vec<String>> {
    let mut venues: Vec<String> = Vec::new();
    for venue in list {
        let venue = venue.trim();
        if venue.is_empty() {
            return Err(ScopeError::InvalidInput(
                "--venue has an empty venue name".to_string(),
            ));
        }
        if venues.iter().any(|v| v.eq_ignore_ascii_case(venue)) {
            return Err(ScopeError::InvalidInput(format!(
                "venue `{venue}` is listed twice in --venue"
            )));
        }
        venues.push(venue.to_string());
    }
    Ok(venues)
}

/// The exit rule: the command fails (exit code 1) when any venue failed.
/// An unhealthy venue does not fail it, as with one venue.
pub(crate) fn exit_result(failed: &[&str], total: usize) -> Result<()> {
    if failed.is_empty() {
        return Ok(());
    }
    Err(ScopeError::Other(format!(
        "{} of {total} venues failed: {}",
        failed.len(),
        failed.join(", ")
    )))
}

/// A reason on one line, for a table cell and a JSON field.
fn one_line(error: &ScopeError) -> String {
    error
        .to_string()
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

// =============================================================================
// Summary
// =============================================================================

/// One venue of a summary command, prepared once: its token resolved and
/// its coin list read. The preparation may ask the user, so it runs one
/// venue after the other.
struct Leg {
    venue: String,
    ready: std::result::Result<Ready, String>,
}

struct Ready {
    market: MarketVenue,
    token: Option<String>,
    tokens: Vec<TokenAddress>,
}

/// One venue's result in one run.
pub(crate) struct VenueReport {
    pub venue: String,
    pub tokens: Vec<TokenAddress>,
    pub result: std::result::Result<MarketSummary, String>,
}

async fn prepare(
    args: &SummaryArgs,
    venues: &[String],
    source: VenueSource<'_>,
    factory: &dyn ChainClientFactory,
    out: &Output,
) -> Vec<Leg> {
    let mut legs = Vec::new();
    for name in venues {
        let ready = async {
            let market = source(name)?;
            let resolved = market::resolve_market_token(args, &market, factory, out).await?;
            let tokens = market::token_addresses(args, &market, out).await;
            Ok::<_, ScopeError>(Ready {
                market,
                token: resolved.or_else(|| args.token_contract.clone()),
                tokens,
            })
        }
        .await
        .map_err(|e| one_line(&e));
        legs.push(Leg {
            venue: name.clone(),
            ready,
        });
    }
    legs
}

/// Reads every venue at once. The reports keep the order of the legs.
async fn read_all(
    args: &SummaryArgs,
    legs: &[Leg],
    thresholds: &HealthThresholds,
    histories: &[DepthHistory],
    factory: &dyn ChainClientFactory,
    out: &Output,
) -> Vec<VenueReport> {
    let reads = legs.iter().enumerate().map(|(i, leg)| async move {
        let (result, tokens) = match &leg.ready {
            Err(why) => (Err(why.clone()), Vec::new()),
            Ok(ready) => {
                let summary = market::read_summary(
                    args,
                    &ready.market,
                    ready.token.clone(),
                    thresholds,
                    factory,
                    histories.get(i),
                    out,
                )
                .await;
                (summary.map_err(|e| one_line(&e)), ready.tokens.clone())
            }
        };
        VenueReport {
            venue: leg.venue.clone(),
            tokens,
            result,
        }
    });
    futures::future::join_all(reads).await
}

/// Runs `market summary` on two or more venues, once or on a schedule.
pub(crate) async fn run_summary(
    args: &SummaryArgs,
    venues: &[String],
    schedule: Option<Schedule>,
    thresholds: &HealthThresholds,
    source: VenueSource<'_>,
    factory: &dyn ChainClientFactory,
    out: &Output,
) -> Result<()> {
    // Once per command: the token check and the coin list do not change
    // between runs.
    let legs = prepare(args, venues, source, factory, out).await;
    let Some(schedule) = schedule else {
        let reports = read_all(args, &legs, thresholds, &[], factory, out).await;
        out!(out, "{}", render(args, &reports, None)?)?;
        write_report(args, &reports, out)?;
        return exit_result(&failed(&reports), reports.len());
    };

    note!(
        out,
        "Running market summary on {} venues every {}s {} (Ctrl+C to stop)\n",
        venues.len(),
        schedule.every.as_secs(),
        match schedule.duration {
            Some(d) => format!("for {}s", d.as_secs()),
            None => "until cancelled".to_string(),
        }
    );
    let metrics = series::summary_metrics(thresholds);
    let histories: Vec<DepthHistory> = venues.iter().map(|_| DepthHistory::default()).collect();
    let last = repeat::run_with(
        &schedule,
        &metrics,
        args.csv.as_deref(),
        CsvRows::PerVenue,
        out,
        |n| {
            let (legs, histories, metrics) = (&legs, &histories, &metrics);
            async move {
                let reports = read_all(args, legs, thresholds, histories, factory, out).await;
                // Some venues failing is a row of errors. Every venue
                // failing ends the repeat, as one venue failing does.
                if reports.iter().all(|r| r.result.is_err()) {
                    return Err(ScopeError::Other(format!(
                        "no venue gave a summary: {}",
                        reasons(&reports)
                    )));
                }
                Ok(RunOutput {
                    sample: aggregate(&reports, metrics.len()),
                    snapshot: render(args, &reports, Some(n))?,
                    view: None,
                    venues: Some(VenueRun {
                        samples: reports.iter().map(|r| venue_sample(r, metrics)).collect(),
                        comparison: comparison_table(&pair_of(args, &reports), &reports),
                    }),
                    value: reports,
                })
            }
        },
    )
    .await?;

    let Some(reports) = last else {
        return Ok(());
    };
    write_report(args, &reports, out)?;
    if let Some(ref csv_path) = args.csv {
        out.note_saved("Time-series CSV", csv_path.display());
    }
    exit_result(&failed(&reports), reports.len())
}

/// The names of the venues that gave no summary.
fn failed(reports: &[VenueReport]) -> Vec<&str> {
    reports
        .iter()
        .filter(|r| r.result.is_err())
        .map(|r| r.venue.as_str())
        .collect()
}

/// `venue: reason` for each failed venue.
fn reasons(reports: &[VenueReport]) -> String {
    reports
        .iter()
        .filter_map(|r| Some(format!("{}: {}", r.venue, r.result.as_ref().err()?)))
        .collect::<Vec<_>>()
        .join("; ")
}

/// A venue's reading for the series: its key values, or none and why.
fn venue_sample(report: &VenueReport, metrics: &[scope::metrics::Metric]) -> VenueSample {
    match &report.result {
        Ok(summary) => VenueSample {
            venue: report.venue.clone(),
            sample: series::sample(summary, metrics),
            error: None,
        },
        Err(why) => VenueSample {
            venue: report.venue.clone(),
            sample: Sample {
                values: vec![None; metrics.len()],
                healthy: false,
                failed_checks: vec![why.clone()],
            },
            error: Some(why.clone()),
        },
    }
}

/// The run's one sample: healthy only when every venue is. The values stay
/// empty; each venue has its own in the run's venue readings.
fn aggregate(reports: &[VenueReport], metrics: usize) -> Sample {
    let failed_checks: Vec<String> = reports
        .iter()
        .flat_map(|r| match &r.result {
            Err(why) => vec![format!("{}: {why}", r.venue)],
            Ok(s) if !s.healthy => {
                let mut fails = series::sample(s, &[]).failed_checks;
                if fails.is_empty() {
                    fails.push("unhealthy".to_string());
                }
                fails
                    .into_iter()
                    .map(|f| format!("{}: {f}", r.venue))
                    .collect()
            }
            Ok(_) => Vec::new(),
        })
        .collect();
    Sample {
        values: vec![None; metrics],
        healthy: failed_checks.is_empty(),
        failed_checks,
    }
}

/// The pair as the sections title it: the first summary's, else the user's.
fn pair_of(args: &SummaryArgs, reports: &[VenueReport]) -> String {
    reports
        .iter()
        .find_map(|r| r.result.as_ref().ok().map(|s| s.pair.clone()))
        .unwrap_or_else(|| args.pair.clone())
}

/// The run as the CLI prints it, in the command's format.
fn render(args: &SummaryArgs, reports: &[VenueReport], run: Option<u64>) -> Result<String> {
    match args.format {
        SummaryFormat::Text => Ok(text_report(&args.pair, reports)),
        SummaryFormat::Json => json_report(reports, run),
    }
}

/// One section per venue, in the order given, then the comparison table.
pub(crate) fn text_report(user_pair: &str, reports: &[VenueReport]) -> String {
    let pair = reports
        .iter()
        .find_map(|r| r.result.as_ref().ok().map(|s| s.pair.clone()))
        .unwrap_or_else(|| user_pair.to_string());
    let mut sections: Vec<String> = reports
        .iter()
        .map(|r| match &r.result {
            Ok(summary) => summary.format_text_with_tokens(Some(&r.venue), &r.tokens),
            Err(why) => failed_section(user_pair, &r.venue, why),
        })
        .collect();
    sections.push(comparison_table(&pair, reports));
    // Each section starts with a blank line of its own.
    sections.concat()
}

/// The section of a venue that gave no summary: the pair, the venue, why.
fn failed_section(pair: &str, venue: &str, why: &str) -> String {
    format!(
        "{}\n{}\n{}\n",
        t::section_header(&format!("{pair} ({venue})")),
        t::warning_row(&format!("No summary: {why}")),
        t::section_footer()
    )
}

/// The JSON of a run on several venues: a list with one item per venue, in
/// the order given. An item is the summary of one venue (the shape of a
/// single-venue run, with its `venue` field), or `venue` and `error` when
/// the venue failed.
pub(crate) fn json_report(reports: &[VenueReport], run: Option<u64>) -> Result<String> {
    let items: Vec<serde_json::Value> = reports
        .iter()
        .map(|r| match &r.result {
            Ok(s) => market::summary_json(s, &r.venue, &r.tokens, run),
            Err(why) => serde_json::json!({"run": run, "venue": r.venue, "error": why}),
        })
        .collect();
    Ok(format!("{}\n", serde_json::to_string_pretty(&items)?))
}

/// One row of the comparison table, as text.
struct Row {
    venue: String,
    mid: String,
    spread: String,
    bid: String,
    ask: String,
    volume: String,
    health: String,
    signs: String,
}

/// The most columns of the last cell of the comparison table.
const NOTE_COLS: usize = 60;

/// What a cell shows when the venue has no value.
const NO_VALUE: &str = "-";

fn row(report: &VenueReport) -> Row {
    let venue = report.venue.clone();
    let summary = match &report.result {
        Ok(s) => s,
        Err(why) => {
            return Row {
                venue,
                mid: NO_VALUE.into(),
                spread: NO_VALUE.into(),
                bid: NO_VALUE.into(),
                ask: NO_VALUE.into(),
                volume: NO_VALUE.into(),
                health: "ERROR".into(),
                signs: why.clone(),
            };
        }
    };
    let money = scope::display::format_large_number;
    Row {
        venue,
        mid: summary
            .mid_price
            .map_or(NO_VALUE.into(), scope::display::format::format_price),
        spread: summary
            .spread_pct
            .map_or(NO_VALUE.into(), |p| format!("{p:.3}%")),
        bid: money(summary.bid_depth),
        ask: money(summary.ask_depth),
        volume: summary.volume_24h.map_or(NO_VALUE.into(), money),
        health: if summary.healthy {
            "HEALTHY"
        } else {
            "UNHEALTHY"
        }
        .into(),
        signs: bot_signs(summary),
    }
}

/// The bot-pattern signs: the count, then the names. No tape is not none.
fn bot_signs(summary: &MarketSummary) -> String {
    match &summary.bot_patterns {
        None => NO_VALUE.to_string(),
        Some(found) if found.findings.is_empty() => match found.tape_note {
            Some(_) => "none (no trade tape)".to_string(),
            None => "none".to_string(),
        },
        Some(found) => {
            let names: Vec<String> = found.findings.iter().map(|f| f.label()).collect();
            format!("{}: {}", names.len(), names.join(", "))
        }
    }
}

const HEADERS: [&str; 8] = [
    "Venue",
    "Mid",
    "Spread",
    "Bid depth",
    "Ask depth",
    "Volume 24h",
    "Health",
    "Bot signs or error",
];

/// The comparison table: one row per venue.
pub(crate) fn comparison_table(pair: &str, reports: &[VenueReport]) -> String {
    // A long reason would stretch every rule of the table; the section of
    // the venue has the whole reason.
    let rows: Vec<Row> = reports
        .iter()
        .map(|r| {
            let mut row = row(r);
            row.signs = crate::cli::textfit::cut_with_ellipsis(&row.signs, NOTE_COLS);
            row
        })
        .collect();
    let width = |header: &str, cell: fn(&Row) -> &str| {
        rows.iter()
            .map(|r| cell(r).chars().count())
            .chain([header.chars().count()])
            .max()
            .unwrap_or(0)
    };
    let col = |i: usize, align: char, cell: fn(&Row) -> &str| t::Col {
        label: HEADERS[i],
        width: width(HEADERS[i], cell),
        align,
    };
    let cols = [
        col(0, '<', |r| &r.venue),
        col(1, '>', |r| &r.mid),
        col(2, '>', |r| &r.spread),
        col(3, '>', |r| &r.bid),
        col(4, '>', |r| &r.ask),
        col(5, '>', |r| &r.volume),
        col(6, '<', |r| &r.health),
        col(7, '<', |r| &r.signs),
    ];
    let mut text = format!(
        "{}\n{}\n",
        t::section_header(&format!("Comparison — {pair}")),
        t::table_header(&cols)
    );
    for r in &rows {
        let cells = [
            r.venue.as_str(),
            &r.mid,
            &r.spread,
            &r.bid,
            &r.ask,
            &r.volume,
            &r.health,
            &r.signs,
        ];
        text.push_str(&t::table_row(&cols, &cells));
        text.push('\n');
    }
    text.push_str(&t::section_footer());
    text.push('\n');
    text
}

/// The comparison as a markdown table.
fn comparison_markdown(reports: &[VenueReport]) -> String {
    let mut md = format!(
        "| {} |\n|{}|\n",
        HEADERS.join(" | "),
        vec!["---"; HEADERS.len()].join("|")
    );
    for r in reports.iter().map(row) {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            r.venue, r.mid, r.spread, r.bid, r.ask, r.volume, r.health, r.signs
        ));
    }
    md
}

/// Writes the `--report` file: one report per venue, then the comparison.
fn write_report(args: &SummaryArgs, reports: &[VenueReport], out: &Output) -> Result<()> {
    let Some(path) = &args.report else {
        return Ok(());
    };
    let mut md = format!(
        "# Market Health Report: {}  \n**Generated:** {}  \n\n",
        pair_of(args, reports),
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );
    for r in reports {
        match &r.result {
            Ok(s) => md.push_str(&s.format_markdown(Some(&r.venue))),
            Err(why) => md.push_str(&format!("## {}\n\nNo summary: {why}\n", r.venue)),
        }
        md.push('\n');
    }
    md.push_str("## Comparison\n\n");
    md.push_str(&comparison_markdown(reports));
    md.push_str(&scope::display::report::report_footer());
    std::fs::write(path, md)?;
    out.note_saved("Report", path.display());
    Ok(())
}

// =============================================================================
// OHLC and trades
// =============================================================================

/// One venue's block of an `ohlc` or `trades` command: the data, or why
/// there is none.
struct Block<T> {
    venue: String,
    data: std::result::Result<(String, Vec<T>), String>,
}

/// Prints the blocks of a command: text, one labelled block per venue
/// (each starts with a blank line), or JSON, a list with one item per venue
/// (`venue` and `key`, or `venue` and `error`). Returns the exit result.
fn print_blocks<T: serde::Serialize>(
    json: bool,
    key: &str,
    blocks: &[Block<T>],
    title: &str,
    pair: &str,
    text: impl Fn(&str, &str, &[T], &Output) -> Result<()>,
    out: &Output,
) -> Result<()> {
    if json {
        let items: Vec<serde_json::Value> = blocks
            .iter()
            .map(|b| match &b.data {
                Ok((_, rows)) => serde_json::json!({"venue": b.venue, key: rows}),
                Err(why) => serde_json::json!({"venue": b.venue, "error": why}),
            })
            .collect();
        outln!(out, "{}", serde_json::to_string_pretty(&items)?)?;
    } else {
        for b in blocks {
            // Each block starts with a blank line of its own.
            match &b.data {
                Ok((symbol, rows)) => text(&b.venue, symbol, rows, out)?,
                Err(why) => {
                    outln!(
                        out,
                        "{}",
                        t::section_header(&format!("{title} — {pair} ({})", b.venue))
                    )?;
                    outln!(out, "{}", t::warning_row(&format!("No data: {why}")))?;
                    outln!(out, "{}", t::section_footer())?;
                }
            }
        }
    }
    let failed: Vec<&str> = blocks
        .iter()
        .filter(|b| b.data.is_err())
        .map(|b| b.venue.as_str())
        .collect();
    exit_result(&failed, blocks.len())
}

/// The CEX client of `venue`, after the check that the base symbol is the
/// venue's token (as for one venue).
async fn checked_client(
    venue: &str,
    pair: &str,
    token_flag: Option<String>,
    json: bool,
    make: ClientSource<'_>,
    out: &Output,
) -> Result<ExchangeClient> {
    let client = make(venue)?;
    let mut aliases = scope::tokens::TokenAliases::load();
    let base = Pair::parse(pair).base;
    market::check_cex_token(&client, &base, None, token_flag, &mut aliases, json, out).await?;
    Ok(client)
}

/// Runs `market ohlc` on two or more venues, one after the other: a venue
/// may ask the user which token it means.
pub(crate) async fn run_ohlc(
    args: &OhlcArgs,
    venues: &[String],
    make: ClientSource<'_>,
    out: &Output,
) -> Result<()> {
    let json = matches!(args.format, OhlcFormat::Json);
    let width = t::terminal_width();
    let mut blocks = Vec::new();
    for venue in venues {
        let data = async {
            let client = checked_client(
                venue,
                &args.pair,
                args.token_contract.clone(),
                json,
                make,
                out,
            )
            .await?;
            market::fetch_candles(&client, args, width).await
        }
        .await
        .map_err(|e| one_line(&e));
        blocks.push(Block {
            venue: venue.clone(),
            data,
        });
    }
    print_blocks(
        json,
        "candles",
        &blocks,
        "OHLC",
        &args.pair,
        |venue, symbol, candles, out| {
            market::write_ohlc_text(args, venue, symbol, candles, width, out)
        },
        out,
    )
}

/// Runs `market trades` on two or more venues, one after the other.
pub(crate) async fn run_trades(
    args: &TradesArgs,
    venues: &[String],
    make: ClientSource<'_>,
    out: &Output,
) -> Result<()> {
    let json = matches!(args.format, OhlcFormat::Json);
    let mut blocks = Vec::new();
    for venue in venues {
        let data = async {
            let client = checked_client(
                venue,
                &args.pair,
                args.token_contract.clone(),
                json,
                make,
                out,
            )
            .await?;
            market::fetch_trades(&client, args).await
        }
        .await
        .map_err(|e| one_line(&e));
        blocks.push(Block {
            venue: venue.clone(),
            data,
        });
    }
    print_blocks(
        json,
        "trades",
        &blocks,
        "Recent Trades",
        &args.pair,
        market::write_trades_text,
        out,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::health_args::HealthArgs;
    use crate::cli::market::MarketCommands;
    use clap::Parser;
    use scope::chains::mocks::MockClientFactory;
    use scope::market::testing::venue_at;
    use std::sync::Arc;
    use std::time::Duration;

    fn strip(text: &str) -> String {
        crate::cli::ansi::strip(text)
    }

    /// A depth reply of `levels` levels a side, `qty` each.
    fn depth(bid: f64, ask: f64, levels: usize, qty: f64) -> String {
        let side = |start: f64, step: f64| -> Vec<String> {
            (0..levels)
                // Sizes that differ level by level: an even ladder is a
                // bot sign of its own.
                .map(|i| {
                    let size = qty + ((i * 7919) % 13) as f64 * 1_000.0;
                    // Uneven steps too, for the same reason.
                    let at = i as f64 + ((i * i) % 3) as f64 * 0.3;
                    format!(r#"["{:.5}","{size}"]"#, start + step * at)
                })
                .collect()
        };
        format!(
            r#"{{"bids":[{}],"asks":[{}]}}"#,
            side(bid, -0.0001).join(","),
            side(ask, 0.0001).join(",")
        )
    }

    /// A trade tape of `n` trades: one size every 2s (a bot), or sizes
    /// that all differ (nothing to find).
    fn tape(n: usize, bot: bool) -> String {
        let trades: Vec<String> = (0..n)
            .map(|i| {
                let qty = if bot { 100.0 } else { 3.0 + i as f64 * 7.13 };
                format!(
                    r#"{{"id":{i},"price":"1.0001","qty":"{qty}","quoteQty":"{qty}","time":{},"isBuyerMaker":{}}}"#,
                    // A bot trades on a timer; people do not.
                    1_700_000_000_000u64
                        + if bot {
                            i as u64 * 2_000
                        } else {
                            (i as u64 * 2_000) + (i as u64 * i as u64 * 977) % 1_900
                        },
                    i % 2 == 0
                )
            })
            .collect();
        format!("[{}]", trades.join(","))
    }

    /// One mocked venue: its depth, 24h volume and trades.
    struct Served {
        id: &'static str,
        server: mockito::ServerGuard,
    }

    /// The paths of a venue's endpoints: depth, ticker, trades, klines.
    fn paths(id: &str) -> [&'static str; 4] {
        if id == "weex" {
            [
                "/api/v3/market/depth",
                "/api/v3/market/ticker/24hr",
                "/api/v3/market/trades",
                "/api/v3/market/klines",
            ]
        } else {
            [
                "/api/v3/depth",
                "/api/v3/ticker/24hr",
                "/api/v3/trades",
                "/api/v3/klines",
            ]
        }
    }

    impl Served {
        async fn new(id: &'static str, book: String, trades: String) -> Self {
            let [depth_path, ticker_path, trades_path, klines_path] = paths(id);
            let mut server = mockito::Server::new_async().await;
            for (path, body) in [
                (depth_path, book),
                (
                    ticker_path,
                    r#"{"lastPrice":"1.0000","quoteVolume":"125000"}"#.to_string(),
                ),
                (trades_path, trades),
                (klines_path, KLINES.to_string()),
            ] {
                server
                    .mock("GET", path)
                    .match_query(mockito::Matcher::Any)
                    .with_body(body)
                    .create_async()
                    .await;
            }
            Self { id, server }
        }

        /// A healthy book at the peg with a tape that shows nothing.
        async fn healthy(id: &'static str) -> Self {
            Self::new(id, depth(0.9999, 1.0001, 20, 40_000.0), tape(40, false)).await
        }
    }

    /// The venues of a test: a name maps to its mock; any other name is an
    /// unknown venue, with the real error.
    fn source(served: &[&Served]) -> impl Fn(&str) -> Result<MarketVenue> + Sync + use<> {
        let urls: Vec<(String, String)> = served
            .iter()
            .map(|s| (s.id.to_string(), s.server.url()))
            .collect();
        move |name: &str| match urls.iter().find(|(id, _)| id == name) {
            Some((id, url)) => Ok(MarketVenue::Cex(Box::new(venue_at(id, url)))),
            None => MarketVenue::from_name_with_http(name, scope::http::test_transport()),
        }
    }

    fn clients(served: &[&Served]) -> impl Fn(&str) -> Result<ExchangeClient> + Sync + use<> {
        let venues = source(served);
        move |name: &str| match venues(name)? {
            MarketVenue::Cex(client) => Ok(*client),
            MarketVenue::Dex(_) => Err(ScopeError::InvalidInput("a DEX has no client".into())),
        }
    }

    fn names(venues: &[&str]) -> Vec<String> {
        venues.iter().map(|v| v.to_string()).collect()
    }

    fn args(venues: &[&str], format: SummaryFormat) -> SummaryArgs {
        SummaryArgs {
            pair: "USDCUSDT".to_string(),
            venue: names(venues),
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

    async fn summary_of(
        args: &SummaryArgs,
        served: &[&Served],
        schedule: Option<Schedule>,
    ) -> (Result<()>, String) {
        let (out, cap) = Output::capture();
        let venues = checked_venues(&args.venue).unwrap();
        let result = run_summary(
            args,
            &venues,
            schedule,
            &args.health.resolve(&scope::config::Config::default()),
            &source(served),
            &MockClientFactory::new(),
            &out,
        )
        .await;
        (result, strip(&cap.out()))
    }

    // ---- the venue list ----

    #[test]
    fn the_venue_flag_repeats_or_takes_a_comma_list_like_scope_monitor() {
        // One parser for the user: the same flag shape as `scope monitor`.
        let parse = |line: &str| {
            let argv = std::iter::once("scope").chain(line.split_whitespace());
            match crate::cli::Cli::try_parse_from(argv).unwrap().command {
                crate::cli::Commands::Market(MarketCommands::Summary(a)) => a.venue,
                crate::cli::Commands::Market(MarketCommands::Ohlc(a)) => a.venue,
                crate::cli::Commands::Market(MarketCommands::Trades(a)) => a.venue,
                other => panic!("{other:?}"),
            }
        };
        for leaf in ["summary", "ohlc", "trades"] {
            let both = ["weex", "binance"];
            let base = format!("market {leaf} USDC");
            assert_eq!(parse(&format!("{base} --venue weex --venue binance")), both);
            assert_eq!(parse(&format!("{base} --venue weex,binance")), both);
            assert_eq!(parse(&format!("{base} --venue weex")), ["weex"]);
            assert_eq!(parse(&base), [scope::market::DEFAULT_VENUE], "{leaf}");
        }
    }

    #[test]
    fn a_venue_named_twice_is_an_error_because_it_would_read_one_market_twice() {
        assert_eq!(
            checked_venues(&names(&["weex", "binance"])).unwrap(),
            ["weex", "binance"],
            "the order given is kept"
        );
        let err = checked_venues(&names(&["weex", "WEEX"])).unwrap_err();
        assert!(err.to_string().contains("`WEEX` is listed twice"), "{err}");
        let err = checked_venues(&names(&["weex", " "])).unwrap_err();
        assert!(err.to_string().contains("empty venue name"), "{err}");
    }

    #[test]
    fn the_help_of_each_command_says_how_to_name_venues_and_the_exit_code() {
        use clap::CommandFactory;
        let cli = crate::cli::Cli::command();
        let market = cli.find_subcommand("market").unwrap();
        for leaf in ["summary", "ohlc", "trades"] {
            let help = market
                .find_subcommand(leaf)
                .unwrap()
                .clone()
                .render_long_help()
                .to_string();
            assert!(help.contains("--venue weex,binance"), "{leaf}\n{help}");
            assert!(
                help.contains("The exit code is 1 when any venue"),
                "{leaf}\n{help}"
            );
        }
    }

    // ---- summary ----

    #[tokio::test]
    async fn a_summary_gives_a_section_per_venue_in_order_then_a_comparison() {
        let binance = Served::healthy("binance").await;
        let weex = Served::healthy("weex").await;
        let args = args(&["weex", "binance"], SummaryFormat::Text);
        let (result, text) = summary_of(&args, &[&binance, &weex], None).await;
        result.unwrap();
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle}\n{text}"))
        };
        let (w, b, c) = (
            at("USDC/USDT (weex)"),
            at("USDC/USDT (binance)"),
            at("Comparison — USDC/USDT"),
        );
        assert!(
            w < b && b < c,
            "venues in the order given, then the table\n{text}"
        );
        let table = &text[c..];
        for head in [
            "Venue",
            "Mid",
            "Spread",
            "Bid depth",
            "Ask depth",
            "Volume 24h",
            "Health",
            "Bot signs",
        ] {
            assert!(table.contains(head), "{head}\n{table}");
        }
        let rows: Vec<&str> = table.lines().filter(|l| l.contains("HEALTHY")).collect();
        assert_eq!(rows.len(), 2, "one row per venue\n{table}");
        assert!(
            rows[0].contains("weex") && rows[1].contains("binance"),
            "{table}"
        );
        assert!(
            rows[0].contains("1.0000") && rows[0].contains("125.00K"),
            "{}",
            rows[0]
        );
    }

    #[tokio::test]
    async fn the_table_has_the_verdict_and_the_bot_signs_of_each_venue() {
        // weex trades one size every 2s; binance trades look human. The
        // depegged book is unhealthy while the other venue is fine.
        let bot = Served::new("weex", depth(0.9999, 1.0001, 20, 40_000.0), tape(60, true)).await;
        let off = Served::new(
            "binance",
            depth(0.9499, 0.9501, 20, 40_000.0),
            tape(60, false),
        )
        .await;
        let args = args(&["weex", "binance"], SummaryFormat::Text);
        let (result, text) = summary_of(&args, &[&bot, &off], None).await;
        result.expect("an unhealthy venue is a verdict, not a failure (as with one venue)");
        let table = &text[text.find("Comparison").unwrap()..];
        let weex = table.lines().find(|l| l.contains("weex")).unwrap();
        let binance = table.lines().find(|l| l.contains("binance")).unwrap();
        assert!(weex.contains(" HEALTHY"), "{weex}");
        assert!(
            weex.contains("Repeated size"),
            "the names of the signs: {weex}"
        );
        assert!(
            weex.contains(": Repeated size") || weex.contains(": Few sizes"),
            "the count comes first, then the names: {weex}"
        );
        assert!(binance.contains("UNHEALTHY"), "{binance}");
        assert!(binance.contains("none"), "no signs: {binance}");
    }

    #[tokio::test]
    async fn a_failing_venue_does_not_hide_the_others() {
        // The reason for the whole design: one bad pair or one down venue
        // must not cost the user the venues that answered.
        let binance = Served::healthy("binance").await;
        let weex = Served::healthy("weex").await;
        let args = args(&["binance", "nowhere", "weex"], SummaryFormat::Text);
        let (result, text) = summary_of(&args, &[&binance, &weex], None).await;
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle}\n{text}"))
        };
        assert!(
            at("USDC/USDT (binance)") < at("USDCUSDT (nowhere)")
                && at("USDCUSDT (nowhere)") < at("USDC/USDT (weex)"),
            "{text}"
        );
        assert!(
            text.contains("No summary: Unknown venue"),
            "the section says why\n{text}"
        );
        let row = text
            .lines()
            .find(|l| l.contains("nowhere") && l.contains("ERROR"))
            .unwrap_or_else(|| panic!("{text}"));
        assert!(
            row.contains("Unknown venue"),
            "the table row shows the error: {row}"
        );
        let table = &text[text.find("Comparison").unwrap()..];
        assert_eq!(table.matches(" HEALTHY").count(), 2, "{table}");
        assert_eq!(
            result.unwrap_err().to_string(),
            "1 of 3 venues failed: nowhere",
            "exit code 1 names the venue"
        );
    }

    #[tokio::test]
    async fn a_venue_that_answers_an_error_fails_that_venue_only() {
        // A venue that answers 400 for the pair: the same row, and the
        // other venue is read.
        let weex = Served::healthy("weex").await;
        let mut bad = mockito::Server::new_async().await;
        bad.mock("GET", mockito::Matcher::Any)
            .match_query(mockito::Matcher::Any)
            .with_status(400)
            .with_body(r#"{"code":-1121,"msg":"Invalid symbol."}"#)
            .create_async()
            .await;
        let bad = Served {
            id: "binance",
            server: bad,
        };
        let args = args(&["binance", "weex"], SummaryFormat::Text);
        let (result, text) = summary_of(&args, &[&bad, &weex], None).await;
        assert!(text.contains("USDC/USDT (weex)"), "{text}");
        assert!(text.contains("No summary:"), "{text}");
        assert!(result.is_err());
    }

    /// How long a stub holds a connection for the other venue to connect.
    /// Sequential reads reach it and the test fails; concurrent reads never
    /// wait this long.
    const HOLD_LIMIT: Duration = Duration::from_secs(5);

    /// What the two stubs share: the venues that have connected (a bit
    /// each), and whether any client waited out `HOLD_LIMIT` for the other.
    struct Meeting {
        seen: tokio::sync::watch::Sender<u8>,
        timed_out: std::sync::atomic::AtomicBool,
    }

    /// A TCP stub in front of `target` (`host:port`) for the venue with the
    /// bit `me`. It holds each client until both venues have connected (or
    /// `HOLD_LIMIT` passes, which it records), then connects it through. No
    /// client waits out the limit only if the venues are read at the same
    /// time. It holds no thread. Returns its URL.
    async fn held(target: String, me: u8, meeting: Arc<Meeting>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut client, _)) = listener.accept().await {
                let target = target.clone();
                let meeting = meeting.clone();
                tokio::spawn(async move {
                    meeting.seen.send_modify(|bits| *bits |= me);
                    let mut watch = meeting.seen.subscribe();
                    let met =
                        tokio::time::timeout(HOLD_LIMIT, watch.wait_for(|bits| *bits == 0b11))
                            .await
                            .is_ok();
                    if !met {
                        meeting
                            .timed_out
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    if let Ok(mut upstream) = tokio::net::TcpStream::connect(target).await {
                        let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
                    }
                });
            }
        });
        url
    }

    #[tokio::test]
    async fn venues_are_read_at_the_same_time() {
        // Each venue's stub holds its connection until the other venue has
        // connected too. Both are connected at once only if the reads run
        // together; read one after the other, the first waits out
        // `HOLD_LIMIT` and the other never connects in that time. No clock
        // ratio, so a loaded machine does not change the result.
        let meeting = Arc::new(Meeting {
            seen: tokio::sync::watch::channel(0u8).0,
            timed_out: std::sync::atomic::AtomicBool::new(false),
        });
        let (a, b) = (
            Served::healthy("binance").await,
            Served::healthy("weex").await,
        );
        let urls = [
            (
                "binance".to_string(),
                held(a.server.host_with_port(), 0b01, meeting.clone()).await,
            ),
            (
                "weex".to_string(),
                held(b.server.host_with_port(), 0b10, meeting.clone()).await,
            ),
        ];
        let source = move |name: &str| {
            let (id, url) = urls.iter().find(|(id, _)| id == name).unwrap();
            Ok(MarketVenue::Cex(Box::new(venue_at(id, url))))
        };
        let args = args(&["binance", "weex"], SummaryFormat::Text);
        let venues = checked_venues(&args.venue).unwrap();
        let (out, cap) = Output::capture();
        run_summary(
            &args,
            &venues,
            None,
            &HealthThresholds::default(),
            &source,
            &MockClientFactory::new(),
            &out,
        )
        .await
        .unwrap();
        let text = strip(&cap.out());
        assert!(
            text.contains("(binance)") && text.contains("(weex)"),
            "{text}"
        );
        assert_eq!(*meeting.seen.borrow(), 0b11, "both venues connected");
        assert!(
            !meeting.timed_out.load(std::sync::atomic::Ordering::SeqCst),
            "a venue waited for the other: the reads ran one after the other"
        );
    }

    #[tokio::test]
    async fn json_is_a_list_with_a_venue_in_each_item_and_an_error_for_a_failed_one() {
        let weex = Served::healthy("weex").await;
        let args = args(&["weex", "nowhere"], SummaryFormat::Json);
        let (result, text) = summary_of(&args, &[&weex], None).await;
        assert!(result.is_err());
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let items = v.as_array().expect("a list for two venues");
        assert_eq!(items.len(), 2);
        // The first item is the single-venue object, field for field.
        assert_eq!(items[0]["venue"], "weex");
        assert_eq!(items[0]["run"], serde_json::Value::Null);
        assert!(items[0]["healthy"].is_boolean() && items[0]["checks"].is_array());
        assert_eq!(items[1]["venue"], "nowhere");
        assert!(
            items[1]["error"]
                .as_str()
                .unwrap()
                .contains("Unknown venue")
        );
        assert!(items[1].get("checks").is_none());
    }

    #[tokio::test]
    async fn the_report_file_has_each_venue_and_the_comparison() {
        let (binance, weex) = (
            Served::healthy("binance").await,
            Served::healthy("weex").await,
        );
        let dir = tempfile::tempdir().unwrap();
        let mut args = args(&["weex", "binance"], SummaryFormat::Text);
        args.report = Some(dir.path().join("report.md"));
        let (result, _) = summary_of(&args, &[&binance, &weex], None).await;
        result.unwrap();
        let md = std::fs::read_to_string(args.report.unwrap()).unwrap();
        assert!(md.contains("weex") && md.contains("binance"), "{md}");
        assert!(
            md.contains("## Comparison") && md.contains("| Venue | Mid |"),
            "{md}"
        );
        assert!(
            md.contains("| weex |") && md.contains("| binance |"),
            "{md}"
        );
    }

    #[tokio::test]
    async fn a_repeat_on_two_venues_keeps_going_when_one_fails_and_writes_a_row_per_venue() {
        let weex = Served::healthy("weex").await;
        let dir = tempfile::tempdir().unwrap();
        let mut args = args(&["weex", "nowhere"], SummaryFormat::Text);
        args.csv = Some(dir.path().join("peg.csv"));
        let schedule = Schedule::new(Duration::from_millis(40), Duration::from_millis(40)).ok();
        let (out, _cap) = Output::capture_merged();
        let venues = checked_venues(&args.venue).unwrap();
        let result = run_summary(
            &args,
            &venues,
            schedule,
            &HealthThresholds::default(),
            &source(&[&weex]),
            &MockClientFactory::new(),
            &out,
        )
        .await;
        // The last run still has a failed venue: the exit result says so,
        // after every run was made and written.
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("1 of 2 venues failed")
        );
        let csv = std::fs::read_to_string(args.csv.unwrap()).unwrap();
        let lines: Vec<&str> = csv.lines().collect();
        assert!(lines[0].starts_with("timestamp,run,venue,"), "{csv}");
        assert!(lines.len() >= 5, "two or more runs, a row per venue: {csv}");
        assert!(
            lines[1].contains(",1,weex,") && lines[2].contains(",1,nowhere,"),
            "{csv}"
        );
        assert!(
            lines[2].contains(",-,-,"),
            "a failed venue has no values: {}",
            lines[2]
        );
        // The screen's series has the venue readings of each run.
        let series = out.series_hook().unwrap();
        let series = series.lock().unwrap();
        let record = series.runs.back().unwrap();
        let last = record.venues.as_ref().unwrap();
        assert_eq!(
            last.samples
                .iter()
                .map(|s| s.venue.as_str())
                .collect::<Vec<_>>(),
            ["weex", "nowhere"]
        );
        assert!(last.samples[0].sample.values.iter().any(Option::is_some));
        assert!(
            last.samples[1]
                .error
                .as_deref()
                .unwrap()
                .contains("Unknown venue")
        );
        assert!(
            last.comparison.contains("Comparison"),
            "{}",
            last.comparison
        );
        assert!(!record.sample.healthy, "a failed venue is a failed run");
    }

    #[tokio::test]
    async fn a_repeat_ends_with_an_error_when_every_venue_fails() {
        // As one failing venue does: nothing to chart, so stop and say why.
        let args = args(&["nowhere", "elsewhere"], SummaryFormat::Text);
        let schedule = Schedule::new(Duration::from_millis(40), Duration::from_millis(40)).ok();
        let (result, _) = summary_of(&args, &[], schedule).await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no venue gave a summary"), "{err}");
        assert!(
            err.contains("nowhere") && err.contains("elsewhere"),
            "{err}"
        );
    }

    // ---- ohlc and trades ----

    const KLINES: &str = r#"[[1700000000000,"1.0","1.5","0.5","1.25","1000.5",1700003599999],
        [1700003600000,"1.25","2.0","1.0","1.75","20.25",1700007199999]]"#;

    fn ohlc_args(venues: &[&str], format: OhlcFormat) -> OhlcArgs {
        OhlcArgs {
            pair: "USDCUSDT".to_string(),
            venue: names(venues),
            token_contract: None,
            interval: "1h".to_string(),
            limit: Some(2),
            table: false,
            format,
        }
    }

    fn trades_args(venues: &[&str], format: OhlcFormat) -> TradesArgs {
        TradesArgs {
            pair: "USDCUSDT".to_string(),
            venue: names(venues),
            token_contract: None,
            limit: 5,
            format,
        }
    }

    #[tokio::test]
    async fn ohlc_gives_a_labelled_block_per_venue_in_order_and_the_failed_one_says_why() {
        let (binance, weex) = (
            Served::healthy("binance").await,
            Served::healthy("weex").await,
        );
        let (out, cap) = Output::capture();
        let list = ["weex", "nowhere", "binance"];
        let args = ohlc_args(&list, OhlcFormat::Text);
        let result = run_ohlc(&args, &names(&list), &clients(&[&binance, &weex]), &out).await;
        let text = strip(&cap.out());
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle}\n{text}"))
        };
        assert!(
            at("OHLC — USDCUSDT (weex)") < at("OHLC — USDCUSDT (nowhere)")
                && at("OHLC — USDCUSDT (nowhere)") < at("OHLC — USDCUSDT (binance)"),
            "{text}"
        );
        assert!(text.contains("No data: Unknown venue"), "{text}");
        assert_eq!(
            text.matches("1.7500 (+75.00%)").count(),
            2,
            "both good venues drew: {text}"
        );
        assert_eq!(
            result.unwrap_err().to_string(),
            "1 of 3 venues failed: nowhere"
        );
    }

    #[tokio::test]
    async fn ohlc_and_trades_json_are_a_list_of_venue_items_so_the_venue_is_never_lost() {
        // A flat list of candles could not say which venue a candle came
        // from, nor hold the error of a failed venue.
        let (binance, weex) = (
            Served::healthy("binance").await,
            Served::healthy("weex").await,
        );
        let served = clients(&[&binance, &weex]);
        let (out, cap) = Output::capture();
        let list = ["weex", "binance"];
        run_ohlc(
            &ohlc_args(&list, OhlcFormat::Json),
            &names(&list),
            &served,
            &out,
        )
        .await
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&cap.out()).unwrap();
        assert_eq!(v[0]["venue"], "weex");
        assert_eq!(v[1]["venue"], "binance");
        assert_eq!(v[1]["candles"].as_array().unwrap().len(), 2);
        assert_eq!(v[1]["candles"][1]["close"], 1.75);

        let (out, cap) = Output::capture();
        let list = ["binance", "nowhere"];
        let result = run_trades(
            &trades_args(&list, OhlcFormat::Json),
            &names(&list),
            &served,
            &out,
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&cap.out()).unwrap();
        assert_eq!(v[0]["venue"], "binance");
        assert_eq!(v[0]["trades"].as_array().unwrap().len(), 5, "the limit");
        assert_eq!(v[1]["venue"], "nowhere");
        assert!(v[1]["error"].as_str().unwrap().contains("Unknown venue"));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn trades_gives_a_block_per_venue() {
        let (binance, weex) = (
            Served::healthy("binance").await,
            Served::healthy("weex").await,
        );
        let (out, cap) = Output::capture();
        let list = ["binance", "weex"];
        run_trades(
            &trades_args(&list, OhlcFormat::Text),
            &names(&list),
            &clients(&[&binance, &weex]),
            &out,
        )
        .await
        .unwrap();
        let text = strip(&cap.out());
        let b = text.find("Recent Trades — USDCUSDT (binance)").unwrap();
        let w = text.find("Recent Trades — USDCUSDT (weex)").unwrap();
        assert!(b < w, "{text}");
        assert_eq!(text.matches("5 trades returned").count(), 2, "{text}");
    }

    // ---- the series ----

    #[test]
    fn a_long_reason_does_not_stretch_the_comparison_table() {
        // The section of the venue has the whole reason; the table cuts it.
        let report = VenueReport {
            venue: "binance".into(),
            tokens: Vec::new(),
            result: Err("HTTP 400 Bad Request ".repeat(30)),
        };
        let table = strip(&comparison_table("USDN/USDT", &[report]));
        let widest = table.lines().map(|l| l.chars().count()).max().unwrap();
        assert!(
            widest < 140,
            "{widest} columns
{table}"
        );
        assert!(table.contains("ERROR") && table.contains('…'), "{table}");
    }

    #[test]
    fn a_failed_venue_has_no_values_in_the_series_and_fails_the_run() {
        let metrics = series::summary_metrics(&HealthThresholds::default());
        let report = VenueReport {
            venue: "nowhere".into(),
            tokens: Vec::new(),
            result: Err("Unknown venue".into()),
        };
        let sample = venue_sample(&report, &metrics);
        assert!(
            sample.sample.values.iter().all(Option::is_none),
            "never zero"
        );
        assert_eq!(sample.error.as_deref(), Some("Unknown venue"));
        let run = aggregate(&[report], metrics.len());
        assert!(!run.healthy);
        assert_eq!(run.failed_checks, ["nowhere: Unknown venue"]);
    }
}
