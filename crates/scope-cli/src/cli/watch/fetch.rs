//! One tick: read every target's metrics and judge its rules.
//!
//! Each source is read once per target per tick, however many rules use
//! it. A failed read makes that source's cells `Error` with the exact
//! error text; other sources and other targets go on. The metrics that
//! compare with the tick before read the run's [`History`].
//!
//! A tick has two steps (design 2026-11-11, section 4). First it reads:
//! the reads of different venues run at the same time, up to
//! [`MAX_CONCURRENT_READS`], and the reads of one venue run one after the
//! other. Then it judges, target by target in the given order, with the one
//! `&mut History`. The judge step does not depend on which read finished
//! first, so concurrency changes the time of a tick and never its result.

use super::{Ready, Sources};
use crate::cli::address::{AddressArgs, analyze_address};
use crate::cli::crawl::{self, Period};
use crate::cli::market::CrawlDexSource;
use crate::cli::output::Output;
use futures::StreamExt;
use scope::compliance::datasource::BlockchainDataClient;
use scope::compliance::risk::RiskEngine;
use scope::market::{
    HealthThresholds, MarketSummary, MarketVenue, Pair, Trade, fetch_summary_with_trades,
};
use scope::metrics::{MetricGroup, MetricId, describe, sample};
use scope::watch::{
    Cell, History, MarketKey, Preset, Reading, Readings, RuleStatus, Snapshot, Target,
    TargetSnapshot, market_keys, preset,
};
use std::time::Duration;
use tokio::time::Instant;

/// The most reads that run at the same time in one tick.
pub const MAX_CONCURRENT_READS: usize = 8;

/// How long one read may take before it is dropped and its cells are
/// `Error`. It is a backstop for a read that never answers; the request
/// timeouts of the venues stay the first limit.
pub const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// The group of all token reads: they go to the same data source.
const ANALYTICS_GROUP: &str = "analytics";

/// The limits of one tick. [`tick`] uses the defaults; tests use small
/// values so they do not wait for [`READ_TIMEOUT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most reads at the same time.
    pub max_concurrent: usize,
    /// The longest time of one read.
    pub read_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_concurrent: MAX_CONCURRENT_READS,
            read_timeout: READ_TIMEOUT,
        }
    }
}

/// How a read ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Error,
    Timeout,
}

impl Outcome {
    /// The name in the JSON record and the trace.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Timeout => "timeout",
        }
    }
}

/// The record of one read: which one, when, how long and how it ended.
/// Times are in ms from the start of the tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadTiming {
    /// The row label, or `<id> token` / `<id> address`.
    pub row: String,
    /// The group key: `venue:<id>`, `analytics` or `address:<chain>`.
    pub group: String,
    /// When the read was listed: the start of the tick, for every read.
    /// `started_ms - queued_ms` is the time it waited for a free slot and
    /// for the earlier reads of its group.
    pub queued_ms: u64,
    pub started_ms: u64,
    pub took_ms: u64,
    pub outcome: Outcome,
    /// The exact error text, when the read did not end `ok`.
    pub error: Option<String>,
}

/// The record of the reads of one tick, in the given order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickTiming {
    /// The time of the read step.
    pub wall_ms: u64,
    /// The sum of the time of every read. Against `wall_ms` it shows how
    /// much time the concurrency saved.
    pub sum_ms: u64,
    /// The bound the tick ran with.
    pub bound: usize,
    pub reads: Vec<ReadTiming>,
}

/// The cell a preset shows when its summary could not be read.
pub const PRESET_ERROR_LABEL: &str = "market health";

/// One tick's results.
#[derive(Debug, Clone, Default)]
pub struct Tick {
    pub snapshot: Snapshot,
    /// For each row of the snapshot, the market summary it read, for the
    /// report and the detail pane.
    pub summaries: Vec<Option<MarketSummary>>,
    /// How long each read took: the record that explains a slow tick.
    pub timing: TickTiming,
}

/// One row's results.
#[derive(Debug, Clone)]
pub struct TargetReading {
    pub row: TargetSnapshot,
    pub summary: Option<MarketSummary>,
}

/// When a tick reads: now, and how far back the first tick looks for
/// trades (one interval), both in ms.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    pub now_ms: u64,
    pub window_ms: u64,
}

impl Clock {
    /// Now, with the watchlist's interval as the first tick's window.
    pub fn now(ready: &Ready) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            now_ms: u64::try_from(now).unwrap_or(0),
            window_ms: u64::try_from(ready.list.interval.as_millis()).unwrap_or(u64::MAX),
        }
    }
}

/// Reads and judges every target of `ready`, in order. `history` holds
/// what the ticks before read; this tick adds its reading.
pub async fn tick(ready: &Ready, src: Sources<'_>, out: &Output, history: &mut History) -> Tick {
    tick_with(ready, src, out, history, Limits::default()).await
}

/// [`tick`] with the limits of the reads.
pub async fn tick_with(
    ready: &Ready,
    src: Sources<'_>,
    out: &Output,
    history: &mut History,
    limits: Limits,
) -> Tick {
    let clock = Clock::now(ready);
    let targets: Vec<_> = ready
        .list
        .targets
        .iter()
        .zip(&ready.contracts)
        .map(|(t, contract)| (t, contract.as_deref()))
        .collect();
    let (raws, timing) = read_all(&targets, src, out, limits).await;
    let mut result = Tick::default();
    // Judged in the given order with the one `History`, whatever the order
    // in which the reads finished.
    let keys = market_keys(targets.iter().map(|(t, _)| *t));
    for (((t, _), raw), keys) in targets.iter().zip(raws).zip(&keys) {
        for reading in judge_target(t, raw, keys, src, history, clock) {
            result.snapshot.targets.push(reading.row);
            result.summaries.push(reading.summary);
        }
    }
    result.timing = timing;
    result
}

/// What a market read gave: the summary and the trade tape it read (or why
/// there is none), or the error text.
type MarketRead = Result<(MarketSummary, Result<Vec<Trade>, String>), String>;

/// The raw results of one target's reads, before they are judged.
struct Raw {
    token: Option<SourceResult>,
    address: Option<SourceResult>,
    /// For each row of [`Target::rows`]: the market read, when the row has
    /// one.
    markets: Vec<Option<MarketRead>>,
}

/// What one read job reads.
enum What<'a> {
    Token,
    Address,
    /// The market of the venue of row `row`.
    Market {
        row: usize,
        venue: &'a str,
    },
}

/// One read of a tick.
struct Job<'a> {
    /// The index of the target.
    target: usize,
    t: &'a Target,
    contract: Option<&'a str>,
    what: What<'a>,
    row: String,
    group: String,
}

/// What a job gave.
enum Done {
    Source(SourceResult),
    /// Boxed: a summary is much larger than a source's readings.
    Market(Box<MarketRead>),
}

/// The time since `start`, in ms.
fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The read jobs of `targets`, in the given order: for each target the
/// token job, the address job, then one market job for each venue row.
fn list_jobs<'a>(targets: &[(&'a Target, Option<&'a str>)]) -> Vec<Job<'a>> {
    let mut jobs = Vec::new();
    for (i, &(t, contract)) in targets.iter().enumerate() {
        let mut job = |what: What<'a>, row: String, group: String| {
            jobs.push(Job {
                target: i,
                t,
                contract,
                what,
                row,
                group,
            });
        };
        if t.needs(MetricGroup::Token) {
            job(
                What::Token,
                format!("{} token", t.id),
                ANALYTICS_GROUP.to_string(),
            );
        }
        if t.needs(MetricGroup::Address) {
            let chain = t.chain.as_deref().unwrap_or(scope::chains::DEFAULT_CHAIN);
            job(
                What::Address,
                format!("{} address", t.id),
                format!("address:{chain}"),
            );
        }
        for (row, venue) in t.rows().into_iter().enumerate() {
            if let Some(venue) = venue
                && t.needs_market()
            {
                job(
                    What::Market { row, venue },
                    t.row_label(Some(venue)),
                    format!("venue:{venue}"),
                );
            }
        }
    }
    jobs
}

/// The job indexes by group, in the order the groups first appear.
fn group_jobs(jobs: &[Job<'_>]) -> Vec<Vec<usize>> {
    let mut keys: Vec<&str> = Vec::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, job) in jobs.iter().enumerate() {
        match keys.iter().position(|k| *k == job.group) {
            Some(g) => groups[g].push(i),
            None => {
                keys.push(&job.group);
                groups.push(vec![i]);
            }
        }
    }
    groups
}

/// Reads every job of `targets`: the groups run at the same time, up to the
/// bound, and the jobs of one group run one after the other. Each job
/// carries its index, so the results come back in the given order whatever
/// the order of finishing. A job that outlives the timeout is dropped (its
/// requests with it) and gives an error; the other jobs go on. Nothing is
/// spawned: when the caller drops this future, every read stops.
async fn read_all(
    targets: &[(&Target, Option<&str>)],
    src: Sources<'_>,
    out: &Output,
    limits: Limits,
) -> (Vec<Raw>, TickTiming) {
    let jobs = list_jobs(targets);
    let start = Instant::now();
    let finished: Vec<Vec<(usize, Done, ReadTiming)>> = futures::stream::iter(group_jobs(&jobs))
        .map(|group| run_group(group, &jobs, src, out, limits.read_timeout, start))
        // At least 1: a bound of 0 would never start a group.
        .buffer_unordered(limits.max_concurrent.max(1))
        .collect()
        .await;
    let mut done: Vec<Option<(Done, ReadTiming)>> = jobs.iter().map(|_| None).collect();
    for (i, d, timing) in finished.into_iter().flatten() {
        done[i] = Some((d, timing));
    }
    let mut raws: Vec<Raw> = targets
        .iter()
        .map(|(t, _)| Raw {
            token: None,
            address: None,
            markets: t.rows().iter().map(|_| None).collect(),
        })
        .collect();
    let mut timing = TickTiming {
        wall_ms: elapsed_ms(start),
        bound: limits.max_concurrent,
        ..TickTiming::default()
    };
    for (job, slot) in jobs.iter().zip(done) {
        let Some((d, read)) = slot else { continue };
        let raw = &mut raws[job.target];
        match (&job.what, d) {
            (What::Token, Done::Source(r)) => raw.token = Some(r),
            (What::Address, Done::Source(r)) => raw.address = Some(r),
            (What::Market { row, .. }, Done::Market(r)) => raw.markets[*row] = Some(*r),
            _ => unreachable!("a job gives the result of its own kind"),
        }
        tracing::info!(
            row = %read.row,
            group = %read.group,
            queued_ms = read.queued_ms,
            started_ms = read.started_ms,
            took_ms = read.took_ms,
            outcome = read.outcome.name(),
            error = read.error.as_deref().unwrap_or(""),
            "watch read"
        );
        timing.sum_ms += read.took_ms;
        timing.reads.push(read);
    }
    (raws, timing)
}

/// Runs the jobs of one group, one after the other, each under the
/// timeout. Returns each job's index, result and timing.
async fn run_group(
    group: Vec<usize>,
    jobs: &[Job<'_>],
    src: Sources<'_>,
    out: &Output,
    read_timeout: Duration,
    start: Instant,
) -> Vec<(usize, Done, ReadTiming)> {
    // A read never prompts. In a terminal two prompts at the same time
    // would read the same stdin; the question is an error instead. The
    // TUI's list takes one question at a time, so it stays.
    let out = out.clone().without_prompts();
    let mut results = Vec::with_capacity(group.len());
    for i in group {
        let job = &jobs[i];
        let started_ms = elapsed_ms(start);
        let read = tokio::time::timeout(read_timeout, run_job(job, src, &out)).await;
        let took_ms = elapsed_ms(start).saturating_sub(started_ms);
        let (done, outcome, error) = match read {
            Ok(done) => {
                let error = match &done {
                    Done::Source(Err(e)) => Some(e.clone()),
                    Done::Market(m) => m.as_ref().as_ref().err().cloned(),
                    Done::Source(Ok(_)) => None,
                };
                let outcome = if error.is_some() {
                    Outcome::Error
                } else {
                    Outcome::Ok
                };
                (done, outcome, error)
            }
            Err(_) => {
                let text = format!("read timed out after {read_timeout:?}");
                let done = match job.what {
                    What::Market { .. } => Done::Market(Box::new(Err(text.clone()))),
                    _ => Done::Source(Err(text.clone())),
                };
                (done, Outcome::Timeout, Some(text))
            }
        };
        results.push((
            i,
            done,
            ReadTiming {
                row: job.row.clone(),
                group: job.group.clone(),
                queued_ms: 0,
                started_ms,
                took_ms,
                outcome,
                error,
            },
        ));
    }
    results
}

/// Reads one source.
async fn run_job(job: &Job<'_>, src: Sources<'_>, out: &Output) -> Done {
    match job.what {
        What::Token => Done::Source(read_token(job.t, job.contract, src, out).await),
        What::Address => Done::Source(read_address(job.t, src, out).await),
        What::Market { venue, .. } => {
            let thresholds = preset::thresholds(job.t, &src.config.market.health);
            Done::Market(Box::new(
                read_market(job.t, venue, job.contract, &thresholds, src, out).await,
            ))
        }
    }
}

/// What one source gave: a reading by metric, or the error text.
type SourceResult = Result<Vec<(MetricId, Reading)>, String>;

/// Every value of `values` as a reading: a value or none.
fn readings(values: Vec<(MetricId, Option<f64>)>) -> Vec<(MetricId, Reading)> {
    values.into_iter().map(|(id, v)| (id, Ok(v))).collect()
}

/// Reads and judges one target: one reading per row of
/// [`Target::rows`]. Each venue's market is read once; the token and the
/// address sources once for the target.
pub async fn read_target(
    t: &Target,
    contract: Option<&str>,
    src: Sources<'_>,
    out: &Output,
    history: &mut History,
    clock: Clock,
) -> Vec<TargetReading> {
    let (mut raws, _) = read_all(&[(t, contract)], src, out, Limits::default()).await;
    let keys = market_keys([t]).remove(0);
    judge_target(t, raws.remove(0), &keys, src, history, clock)
}

/// Judges one target from what its reads gave: one reading per row of
/// [`Target::rows`]. `keys` has the market key of each row. It builds the
/// cells and reads the `history`; it does not read from the network.
fn judge_target(
    t: &Target,
    raw: Raw,
    keys: &[Option<MarketKey>],
    src: Sources<'_>,
    history: &mut History,
    clock: Clock,
) -> Vec<TargetReading> {
    let thresholds = preset::thresholds(t, &src.config.market.health);
    let Raw {
        token,
        address,
        mut markets,
    } = raw;
    let rows = t.rows();
    let one_row = rows.len() == 1;
    let mut result = Vec::with_capacity(rows.len());
    for (row, venue) in rows.into_iter().enumerate() {
        let label = t.row_label(venue);
        let mut cells = Vec::new();
        let mut summary = None;
        let mut market: Option<SourceResult> = None;
        if let Some(read) = markets[row].take() {
            match read {
                Ok((s, trades)) => {
                    if t.preset == Some(Preset::MarketHealth) {
                        cells.extend(preset::cells(&s));
                    }
                    let since = keys[row].as_ref().filter(|_| t.keeps_history()).map(|key| {
                        let tape = trades.as_deref().map_err(String::as_str);
                        history.read(key, tape, s.volume_24h, clock.now_ms, clock.window_ms)
                    });
                    market = Some(Ok(market_values(t, &s, &thresholds, since.as_ref())));
                    summary = Some(s);
                }
                Err(e) => {
                    if t.preset == Some(Preset::MarketHealth) {
                        cells.push(Cell {
                            label: PRESET_ERROR_LABEL.to_string(),
                            rule: None,
                            value: None,
                            status: RuleStatus::Error(e.clone()),
                            detail: None,
                        });
                    }
                    market = Some(Err(e));
                }
            }
        }
        // One row holds every rule; split rows hold the market rules on
        // the venue rows and the others on the id row.
        let rules = t
            .rules
            .iter()
            .filter(|r| one_row || (r.metric.group() == MetricGroup::Market) == venue.is_some());
        for rule in rules {
            let source = match rule.metric.group() {
                MetricGroup::Market => market.as_ref(),
                MetricGroup::Token => token.as_ref(),
                MetricGroup::Address => address.as_ref(),
            };
            let (value, status) = match source {
                Some(Ok(values)) => {
                    let reading = values
                        .iter()
                        .find(|(id, _)| *id == rule.metric)
                        .map_or(Ok(None), |(_, r)| r.clone());
                    match reading {
                        Ok(value) => (value, rule.eval(value)),
                        Err(status) => (None, status),
                    }
                }
                Some(Err(e)) => (None, RuleStatus::Error(e.clone())),
                None if rule.metric.group() == MetricGroup::Market => {
                    (None, RuleStatus::Error("no venue".to_string()))
                }
                None => (None, RuleStatus::Error("no source".to_string())),
            };
            cells.push(Cell {
                label: rule.to_string(),
                rule: Some(*rule),
                value,
                status,
                detail: None,
            });
        }
        result.push(TargetReading {
            row: TargetSnapshot::new(label, cells).on_venue(venue),
            summary,
        });
    }
    result
}

/// The market summary of a target on `venue_name`, the same fetch as
/// `market summary`, with the venue's trade tape it read (or why there is
/// none).
async fn read_market(
    t: &Target,
    venue_name: &str,
    contract: Option<&str>,
    thresholds: &HealthThresholds,
    src: Sources<'_>,
    out: &Output,
) -> Result<(MarketSummary, Result<Vec<Trade>, String>), String> {
    let venue = MarketVenue::from_name_with_http(venue_name, src.clients.http())
        .map_err(|e| e.to_string())?;
    let dex = CrawlDexSource {
        factory: src.clients,
        out,
        token: contract.map(str::to_string),
    };
    fetch_summary_with_trades(&venue, &Pair::parse(&t.id), thresholds, &dex)
        .await
        .map_err(|e| e.to_string())
}

/// Every market value of the target's rules, read from `summary` with the
/// target's thresholds (the same values the repeating view charts), and
/// the since-last-tick ones from `since`.
fn market_values(
    t: &Target,
    summary: &MarketSummary,
    thresholds: &HealthThresholds,
    since: Option<&Readings>,
) -> Vec<(MetricId, Reading)> {
    let table: Vec<_> = t
        .rules
        .iter()
        .filter(|r| r.metric.group() == MetricGroup::Market)
        .map(|r| describe(r.metric, thresholds))
        .collect();
    let s = sample(summary, &table);
    table
        .iter()
        .zip(s.values)
        .map(|(m, v)| {
            let reading = since.and_then(|r| r.get(m.id)).cloned().unwrap_or(Ok(v));
            (m.id, reading)
        })
        .collect()
}

/// Token analytics, through the crawl pipeline.
async fn read_token(
    t: &Target,
    contract: Option<&str>,
    src: Sources<'_>,
    out: &Output,
) -> SourceResult {
    let chain = t.chain.as_deref().unwrap_or(scope::chains::DEFAULT_CHAIN);
    let input = contract.unwrap_or(&t.id);
    let a =
        crawl::fetch_analytics_for_input(input, chain, Period::Hour24, 10, src.clients, None, out)
            .await
            .map_err(|e| e.to_string())?;
    Ok(readings(vec![
        (MetricId::PriceUsd, Some(a.price_usd)),
        (MetricId::PriceChange24hPct, Some(a.price_change_24h)),
        (MetricId::Volume24h, Some(a.volume_24h)),
        (MetricId::LiquidityUsd, Some(a.liquidity_usd)),
        (MetricId::Holders, Some(a.total_holders as f64)),
    ]))
}

/// The address values the target's rules read: its native balance and its
/// risk score, each fetched only when a rule needs it.
async fn read_address(t: &Target, src: Sources<'_>, out: &Output) -> SourceResult {
    let chain = t
        .chain
        .clone()
        .unwrap_or_else(|| scope::chains::DEFAULT_CHAIN.to_string());
    let needs = |m: MetricId| t.rules.iter().any(|r| r.metric == m);
    let mut values = Vec::new();
    if needs(MetricId::NativeBalance) {
        let args = AddressArgs {
            address: t.id.clone(),
            chain: chain.clone(),
            format: None,
            include_txs: false,
            include_tokens: false,
            limit: 1,
            report: None,
            dossier: false,
        };
        let client = src
            .clients
            .create_chain_client(&chain)
            .map_err(|e| e.to_string())?;
        let report = analyze_address(&args, client.as_ref(), out)
            .await
            .map_err(|e| e.to_string())?;
        values.push((
            MetricId::NativeBalance,
            leading_number(&report.balance.formatted),
        ));
    }
    if needs(MetricId::RiskScore) {
        let engine = RiskEngine::with_optional_client(
            BlockchainDataClient::from_env_opt_with_http(src.clients.http()),
        );
        let risk = engine
            .assess_address(&t.id, &chain)
            .await
            .map_err(|e| e.to_string())?;
        values.push((MetricId::RiskScore, Some(f64::from(risk.overall_score))));
    }
    Ok(readings(values))
}

/// The number at the start of a formatted amount (`1,234.5 ETH` is 1234.5).
fn leading_number(text: &str) -> Option<f64> {
    text.split_whitespace()
        .next()?
        .replace(',', "")
        .parse()
        .ok()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use scope::chains::mocks::MockClientFactory;
    use scope::watch::{Rule, TargetKind, TargetStatus, Watchlist};

    fn ready(targets: Vec<Target>) -> Ready {
        let contracts = vec![None; targets.len()];
        Ready {
            list: Watchlist {
                targets,
                ..Watchlist::new("t")
            },
            contracts,
        }
    }

    fn quiet() -> Output {
        Output::capture().0
    }

    #[test]
    fn leading_numbers() {
        assert_eq!(leading_number("1,234.5 ETH"), Some(1234.5));
        assert_eq!(leading_number("0.1"), Some(0.1));
        assert_eq!(leading_number("n/a"), None);
    }

    #[tokio::test]
    async fn token_rules_read_the_analytics_once_and_judge_each_rule() {
        let factory = MockClientFactory::new();
        let config = scope::config::Config::default();
        let src = Sources {
            config: &config,
            clients: &factory,
        };
        let mut t = Target::new(
            TargetKind::Token,
            "0x1234567890123456789012345678901234567890",
        );
        t.chain = Some("ethereum".into());
        t.rules = vec![
            Rule::parse("price_usd > 0").unwrap(),
            Rule::parse("holders >= 100000000000").unwrap(),
        ];
        let tick = tick(&ready(vec![t]), src, &quiet(), &mut History::default()).await;
        let row = &tick.snapshot.targets[0];
        assert_eq!(row.cells.len(), 2);
        assert_eq!(row.cells[0].label, "price_usd > 0");
        assert!(row.cells[0].value.is_some(), "{row:?}");
        assert_eq!(row.cells[0].status, RuleStatus::Pass, "{row:?}");
        assert!(
            matches!(row.cells[1].status, RuleStatus::Fail(_)),
            "{row:?}"
        );
        assert_eq!(row.status, TargetStatus::Breached);
        assert!(tick.summaries[0].is_none());
    }

    /// The checks of `market summary <pair> --venue <venue> --format json`
    /// with `flags`, as (status, message), and its health.
    async fn summary_checks(
        venue: &str,
        flags: &[&str],
        factory: &MockClientFactory,
    ) -> (Vec<(String, String)>, bool) {
        let mut argv = vec!["scope", "market", "summary", "USDC", "--venue", venue];
        argv.extend(flags);
        argv.extend(["--format", "json"]);
        let cli = <crate::cli::Cli as clap::Parser>::try_parse_from(argv).unwrap();
        let (o, cap) = Output::capture();
        crate::cli::dispatch::dispatch_with(
            cli.command,
            &scope::config::Config::default(),
            factory,
            &o,
        )
        .await
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&cap.out()).unwrap();
        let checks = v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["status"].as_str().unwrap().to_string(),
                    c["message"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        (checks, v["healthy"].as_bool().unwrap())
    }

    /// #47 acceptance: the monitor wizard's market health preset gives the
    /// same checks, in the same order and with the same status and health,
    /// as `market summary` for the same venue and thresholds.
    #[tokio::test]
    async fn the_market_health_preset_gives_the_checks_of_market_summary() {
        let factory = usdc_on_eth();
        let config = scope::config::Config::default();
        let src = Sources {
            config: &config,
            clients: &factory,
        };
        for (flags, overrides) in [
            (vec![], scope::market::HealthOverrides::default()),
            (
                vec!["--max-spread-pct", "0.000001", "--min-depth", "1e15"],
                scope::market::HealthOverrides {
                    max_spread_pct: Some(0.000001),
                    min_depth: Some(1e15),
                    ..Default::default()
                },
            ),
        ] {
            let (checks, healthy) = summary_checks("eth", &flags, &factory).await;
            // The strict thresholds make failures, so both statuses are compared.
            assert_eq!(healthy, flags.is_empty(), "flags {flags:?}: {checks:?}");
            let mut t = Target::new(TargetKind::Pair, "USDC");
            t.venues = vec!["eth".into()];
            t.preset = Some(Preset::MarketHealth);
            t.health = overrides;
            let tick = tick(&ready(vec![t]), src, &quiet(), &mut History::default()).await;
            let row = &tick.snapshot.targets[0];
            let cells: Vec<(String, String)> = row
                .cells
                .iter()
                .map(|c| (c.status.name().to_string(), c.label.clone()))
                .collect();
            let expected: Vec<(String, String)> = checks
                .iter()
                .map(|(status, message)| {
                    let name = message.split_once(':').map_or(message.as_str(), |(n, _)| n);
                    (status.clone(), name.trim().to_string())
                })
                .collect();
            assert!(!expected.is_empty());
            assert_eq!(cells, expected, "flags {flags:?}");
            for (cell, (_, message)) in row.cells.iter().zip(&checks) {
                if let Some(m) = cell.status.message() {
                    assert_eq!(m, message, "the same text");
                }
            }
            assert_eq!(
                row.status == TargetStatus::Breached,
                !healthy,
                "flags {flags:?}"
            );
        }
    }

    /// #122: the run's history carries the volume from tick to tick, and a
    /// venue with no trade tape says so in the largest-trade cell.
    #[tokio::test]
    async fn since_last_tick_rules_read_the_history_of_the_run() {
        let factory = usdc_on_eth();
        let config = scope::config::Config::default();
        let src = Sources {
            config: &config,
            clients: &factory,
        };
        let mut t = Target::new(TargetKind::Pair, "USDC");
        t.venues = vec!["eth".into()];
        t.rules = vec![
            Rule::parse("largest_trade < 50000").unwrap(),
            Rule::parse("volume_change_pct <= 100").unwrap(),
            Rule::parse("mid_price > 0").unwrap(),
        ];
        let ready = ready(vec![t]);
        let mut history = History::default();
        let first = tick(&ready, src, &quiet(), &mut history).await;
        let row = &first.snapshot.targets[0];
        assert!(
            matches!(&row.cells[0].status, RuleStatus::Error(m) if m == "largest_trade: a DEX pool has no trade tape here"),
            "{row:?}"
        );
        assert!(
            matches!(&row.cells[1].status, RuleStatus::NotApplicable(m) if m.contains("second reading")),
            "{row:?}"
        );
        assert_eq!(row.cells[2].status, RuleStatus::Pass, "{row:?}");
        // The second tick compares with the first: the mock's volume holds.
        let second = tick(&ready, src, &quiet(), &mut history).await;
        let cell = &second.snapshot.targets[0].cells[1];
        assert_eq!(cell.value, Some(0.0), "{cell:?}");
        assert_eq!(cell.status, RuleStatus::Pass);
    }

    /// A factory with one USDC on the DEX venue's chain (`eth`), the same
    /// for `market summary` and a watchlist.
    pub(crate) fn usdc_on_eth() -> MockClientFactory {
        let mut factory = MockClientFactory::new();
        factory.mock_dex.search_results = vec![scope::chains::TokenSearchResult {
            address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
            symbol: "USDC".to_string(),
            name: "USD Coin".to_string(),
            chain: "ethereum".to_string(),
            price_usd: Some(1.0),
            volume_24h: 1_000_000.0,
            liquidity_usd: 5_000_000.0,
            market_cap: None,
        }];
        if let Some(td) = factory.mock_dex.token_data.as_mut() {
            td.address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string();
            td.symbol = "USDC".to_string();
            td.pairs = vec![scope::chains::DexPair {
                dex_name: "uniswap".into(),
                pair_address: "0xpool".into(),
                base_token: "USDC".into(),
                quote_token: "USDT".into(),
                price_usd: 1.0,
                volume_24h: 1_000_000.0,
                liquidity_usd: 50_000_000.0,
                liquidity_base: Some(25_000_000.0),
                price_change_24h: 0.0,
                buys_24h: 0,
                sells_24h: 0,
                buys_6h: 0,
                sells_6h: 0,
                buys_1h: 0,
                sells_1h: 0,
                pair_created_at: None,
                url: None,
            }];
        }
        factory
    }

    #[tokio::test]
    async fn address_rules_read_the_native_balance_and_the_risk_score() {
        // Offline: with an Etherscan key configured on the test machine the
        // risk engine would otherwise call out through the transport.
        let factory = crate::cli::dispatch::tests::OfflineFactory::new();
        let config = scope::config::Config::default();
        let src = Sources {
            config: &config,
            clients: &factory,
        };
        let mut t = Target::new(
            TargetKind::Address,
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        );
        t.rules = vec![
            // The mock's balance is formatted "1.0".
            Rule::parse("native_balance > 0.5").unwrap(),
            Rule::parse("native_balance < 0.5").unwrap(),
            Rule::parse("risk_score <= 10").unwrap(),
        ];
        let tick = tick(&ready(vec![t]), src, &quiet(), &mut History::default()).await;
        let row = &tick.snapshot.targets[0];
        assert_eq!(row.cells.len(), 3, "{row:?}");
        assert_eq!(row.cells[0].value, Some(1.0), "{row:?}");
        assert_eq!(row.cells[0].status, RuleStatus::Pass, "{row:?}");
        assert!(
            matches!(row.cells[1].status, RuleStatus::Fail(_)),
            "{row:?}"
        );
        let risk = row.cells[2].value.expect("a risk score");
        assert!((0.0..=10.0).contains(&risk), "{risk}");
        assert_eq!(row.cells[2].status, RuleStatus::Pass, "{row:?}");
        assert_eq!(row.status, TargetStatus::Breached);
        assert!(tick.summaries[0].is_none(), "no market source was read");
    }

    #[tokio::test]
    async fn an_address_on_its_own_chain_reads_its_balance() {
        let factory = crate::cli::dispatch::tests::OfflineFactory::new();
        let config = scope::config::Config::default();
        let src = Sources {
            config: &config,
            clients: &factory,
        };
        let mut t = Target::new(
            TargetKind::Address,
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        );
        t.chain = Some("polygon".into());
        t.rules = vec![Rule::parse("native_balance >= 1").unwrap()];
        let clock = Clock {
            now_ms: 0,
            window_ms: 60_000,
        };
        let readings = read_target(&t, None, src, &quiet(), &mut History::default(), clock).await;
        let [reading] = readings.as_slice() else {
            panic!("one row: {readings:?}");
        };
        assert_eq!(reading.row.cells.len(), 1);
        assert_eq!(reading.row.cells[0].value, Some(1.0));
        assert_eq!(reading.row.status, TargetStatus::Passed);
    }

    #[tokio::test]
    async fn a_failed_source_errors_its_cells_and_the_next_target_still_runs() {
        let factory = MockClientFactory::new();
        let config = scope::config::Config::default();
        let src = Sources {
            config: &config,
            clients: &factory,
        };
        let mut bad = Target::new(TargetKind::Pair, "USDC");
        bad.venues = vec!["no-such-venue".into()];
        bad.preset = Some(Preset::MarketHealth);
        bad.rules = vec![Rule::parse("spread_pct < 1").unwrap()];
        let mut good = Target::new(
            TargetKind::Token,
            "0x1234567890123456789012345678901234567890",
        );
        good.rules = vec![Rule::parse("price_usd > 0").unwrap()];
        let tick = tick(
            &ready(vec![bad, good]),
            src,
            &quiet(),
            &mut History::default(),
        )
        .await;
        let row = &tick.snapshot.targets[0];
        assert_eq!(row.cells[0].label, PRESET_ERROR_LABEL);
        let RuleStatus::Error(text) = &row.cells[0].status else {
            panic!("{row:?}")
        };
        assert!(
            text.contains("no-such-venue"),
            "the exact error text: {text}"
        );
        assert!(matches!(row.cells[1].status, RuleStatus::Error(_)));
        assert_eq!(row.status, TargetStatus::Unknown, "never passed");
        assert_eq!(tick.snapshot.targets[1].status, TargetStatus::Passed);
    }

    /// Design 2026-11-04, section 2.3: a target on two venues gives one
    /// row per venue, so a venue that fails does not hide the one that
    /// works, and the token cells go on an id row, read once, so a
    /// holders breach is not reported as a breach of one venue.
    #[tokio::test]
    async fn a_target_on_two_venues_gives_a_row_per_venue_and_an_id_row() {
        let factory = usdc_on_eth();
        let config = scope::config::Config::default();
        let src = Sources {
            config: &config,
            clients: &factory,
        };
        let mut t = Target::new(TargetKind::Token, "USDC");
        t.chain = Some("ethereum".into());
        t.venues = vec!["eth".into(), "no-such-venue".into()];
        t.rules = vec![
            Rule::parse("mid_price > 0").unwrap(),
            Rule::parse("volume_change_pct <= 100").unwrap(),
            Rule::parse("price_usd > 0").unwrap(),
        ];
        let mut ready = ready(vec![t]);
        ready.contracts = vec![Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".into())];
        let mut history = History::default();
        let first = tick(&ready, src, &quiet(), &mut history).await;
        let labels: Vec<(&str, Option<&str>)> = first
            .snapshot
            .targets
            .iter()
            .map(|r| (r.target.as_str(), r.venue.as_deref()))
            .collect();
        assert_eq!(
            labels,
            vec![
                ("USDC@eth", Some("eth")),
                ("USDC@no-such-venue", Some("no-such-venue")),
                ("USDC", None),
            ]
        );
        let [eth, bad, id] = first.snapshot.targets.as_slice() else {
            unreachable!()
        };
        let names =
            |r: &TargetSnapshot| r.cells.iter().map(|c| c.label.clone()).collect::<Vec<_>>();
        assert_eq!(names(eth), ["mid_price > 0", "volume_change_pct <= 100"]);
        assert_eq!(eth.cells[0].status, RuleStatus::Pass, "{eth:?}");
        assert!(
            bad.cells
                .iter()
                .all(|c| matches!(&c.status, RuleStatus::Error(m) if m.contains("no-such-venue"))),
            "{bad:?}"
        );
        assert_eq!(names(id), ["price_usd > 0"]);
        assert_eq!(id.status, TargetStatus::Passed, "{id:?}");
        // Summaries line up with the rows: only the venue that read has one.
        let read: Vec<bool> = first.summaries.iter().map(Option::is_some).collect();
        assert_eq!(read, [true, false, false]);
        // The second tick compares each venue with its own first reading.
        let second = tick(&ready, src, &quiet(), &mut history).await;
        assert_eq!(second.snapshot.targets[0].cells[1].value, Some(0.0));
        assert!(matches!(
            second.snapshot.targets[1].cells[1].status,
            RuleStatus::Error(_)
        ));
    }

    // ---- Concurrent reads (design 2026-11-11, section 4) -----------------
    //
    // These tests run on a paused tokio clock. A test venue answers after a
    // set time on that clock, so "the tick took 3L" is an exact equality and
    // no test waits on the wall clock. The transport is the real
    // `HttpClient` seam: the real `read_market` and the real venue mapping
    // run on top of it.

    use scope::http::{HttpClient, Request, Response};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, Once};

    /// How a test venue answers a request.
    #[derive(Clone, Copy)]
    pub(crate) enum Answer {
        /// After this time on the paused clock.
        After(Duration),
        /// At once, with an error.
        Fails,
        /// Never.
        Never,
    }

    /// A market read makes this many requests, one after the other: the
    /// order book, the ticker and the trades.
    pub(crate) const REQUESTS_PER_READ: u32 = 3;

    /// The latency of one request in the tests.
    pub(crate) const LATENCY: Duration = Duration::from_secs(2);

    /// The time of one whole market read at [`LATENCY`] a request.
    pub(crate) const READ: Duration = Duration::from_secs(2 * REQUESTS_PER_READ as u64);

    /// The venues the tests may name. Each is the Binance descriptor with its
    /// own id and host, written once into the sandbox of the test process.
    pub(crate) const TEST_VENUES: [&str; 8] = [
        "lat-a", "lat-b", "lat-c", "lat-d", "lat-e", "lat-f", "lat-g", "lat-h",
    ];

    /// The quote volume the ticker of a test venue gives on its first call,
    /// and what each later call adds, so the volume changes from tick to
    /// tick in a way that depends on the number of reads of that venue.
    const BASE_VOLUME: f64 = 100_000.0;
    const VOLUME_STEP: f64 = 1_000.0;

    /// The descriptor of a test venue: the shape of Binance's API, with `ID`
    /// for its id and host.
    const VENUE_YAML: &str = r#"
id: ID
name: ID
base_url: https://ID.test
timeout_secs: 15
symbol:
  template: "{base}{quote}"
  default_quote: USDT
capabilities:
  order_book:
    path: /api/v3/depth
    params:
      symbol: "{pair}"
      limit: "100"
    response:
      asks_key: asks
      bids_key: bids
      level_format: positional
  ticker:
    path: /api/v3/ticker/24hr
    params:
      symbol: "{pair}"
    response:
      last_price: lastPrice
      quote_volume_24h: quoteVolume
  trades:
    path: /api/v3/trades
    params:
      symbol: "{pair}"
      limit: "{limit}"
    response:
      price: price
      quantity: qty
      quote_quantity: quoteQty
      timestamp_ms: time
      id: id
      side:
        field: isBuyerMaker
        mapping:
          "true": sell
          "false": buy
"#;

    fn install_test_venues() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let dir = scope::paths::venues_dir();
            std::fs::create_dir_all(&dir).unwrap();
            for id in TEST_VENUES {
                let yaml = VENUE_YAML.replace("ID", id);
                std::fs::write(dir.join(format!("{id}.yaml")), yaml).unwrap();
            }
        });
    }

    #[derive(Default)]
    struct Gauge {
        in_flight: usize,
        max_in_flight: usize,
        started: usize,
        /// For each host: (in flight, most in flight).
        hosts: HashMap<String, (usize, usize)>,
        tickers: HashMap<String, u32>,
        /// The trades the venues give: (age, size in quote). `None`: the
        /// default tape.
        tape: Option<Vec<(Duration, f64)>>,
    }

    /// A transport with a latency for each venue that counts the requests in
    /// flight, in all and for each host.
    pub(crate) struct LatencyHttp {
        /// (venue id, answer, mid price) for each venue.
        venues: Vec<(String, Answer, f64)>,
        gauge: Mutex<Gauge>,
    }

    /// Counts one request in flight while it lives, so a request that is
    /// dropped (a timeout, a cancel) counts out.
    struct InFlight<'a> {
        http: &'a LatencyHttp,
        host: String,
    }

    impl Drop for InFlight<'_> {
        fn drop(&mut self) {
            let mut g = self.http.gauge.lock().unwrap();
            g.in_flight -= 1;
            g.hosts.get_mut(&self.host).unwrap().0 -= 1;
        }
    }

    impl LatencyHttp {
        /// A transport for these venues. The mid price of a venue is `1.000`
        /// plus `0.001` for each place in the list, so a summary shows which
        /// venue it came from.
        pub(crate) fn new(venues: &[(&str, Answer)]) -> Arc<Self> {
            install_test_venues();
            Arc::new(Self {
                venues: venues
                    .iter()
                    .enumerate()
                    .map(|(i, (id, answer))| ((*id).to_string(), *answer, price(i)))
                    .collect(),
                gauge: Mutex::default(),
            })
        }

        /// The most requests in flight at once, over all venues.
        pub(crate) fn max_in_flight(&self) -> usize {
            self.gauge.lock().unwrap().max_in_flight
        }

        /// The most requests in flight at once to one venue.
        pub(crate) fn max_in_flight_to(&self, venue: &str) -> usize {
            let g = self.gauge.lock().unwrap();
            g.hosts.get(&format!("{venue}.test")).map_or(0, |h| h.1)
        }

        /// Gives these trades from now on: (age, size in quote).
        pub(crate) fn set_tape(&self, tape: &[(Duration, f64)]) {
            self.gauge.lock().unwrap().tape = Some(tape.to_vec());
        }

        /// The requests in flight now.
        pub(crate) fn in_flight(&self) -> usize {
            self.gauge.lock().unwrap().in_flight
        }

        /// The requests that have started.
        pub(crate) fn started(&self) -> usize {
            self.gauge.lock().unwrap().started
        }

        fn enter(&self, host: &str) -> InFlight<'_> {
            let mut g = self.gauge.lock().unwrap();
            g.in_flight += 1;
            g.max_in_flight = g.max_in_flight.max(g.in_flight);
            g.started += 1;
            let h = g.hosts.entry(host.to_string()).or_default();
            h.0 += 1;
            h.1 = h.1.max(h.0);
            InFlight {
                http: self,
                host: host.to_string(),
            }
        }
    }

    /// The mid price of the venue at place `i`.
    pub(crate) fn price(i: usize) -> f64 {
        1.0 + 0.001 * i as f64
    }

    #[async_trait::async_trait]
    impl HttpClient for LatencyHttp {
        async fn send(&self, request: Request) -> scope::error::Result<Response> {
            let host = request
                .url
                .strip_prefix("https://")
                .and_then(|u| u.split('/').next())
                .unwrap_or_default()
                .to_string();
            let id = host.trim_end_matches(".test");
            let Some((_, answer, mid)) = self.venues.iter().find(|(v, ..)| v == id) else {
                return Err(scope::error::ScopeError::Other(format!(
                    "no test venue at {host}"
                )));
            };
            let _flight = self.enter(&host);
            match answer {
                Answer::After(latency) => tokio::time::sleep(*latency).await,
                Answer::Fails => {
                    return Err(scope::error::ScopeError::Other(format!("{id} is down")));
                }
                Answer::Never => std::future::pending().await,
            }
            let body = if request.url.contains("/depth") {
                let level = |p: f64, i: usize| format!(r#"["{:.5}","{}"]"#, p, 40_000 + i * 997);
                let bids: Vec<String> = (0..20)
                    .map(|i| level(mid - 0.0001 * (i + 1) as f64, i))
                    .collect();
                let asks: Vec<String> = (0..20)
                    .map(|i| level(mid + 0.0001 * (i + 1) as f64, i))
                    .collect();
                format!(
                    r#"{{"bids":[{}],"asks":[{}]}}"#,
                    bids.join(","),
                    asks.join(",")
                )
            } else if request.url.contains("ticker/24hr") {
                let mut g = self.gauge.lock().unwrap();
                let n = g.tickers.entry(host.clone()).or_default();
                let volume = BASE_VOLUME + VOLUME_STEP * f64::from(*n);
                *n += 1;
                format!(r#"{{"lastPrice":"{mid:.5}","quoteVolume":"{volume}"}}"#)
            } else {
                // By default, trades a second old, so they are inside the
                // first window.
                let tape = self.gauge.lock().unwrap().tape.clone();
                let tape = tape.unwrap_or_else(|| {
                    vec![
                        (Duration::from_secs(1), 500.0),
                        (Duration::from_secs(1), 2_500.0),
                        (Duration::from_secs(1), 100.0),
                    ]
                });
                let now = chrono::Utc::now().timestamp_millis();
                let trades: Vec<String> = tape
                    .iter()
                    .enumerate()
                    .map(|(id, (age, quote))| {
                        let at = now - i64::try_from(age.as_millis()).unwrap();
                        format!(
                            r#"{{"id":{id},"price":"{mid:.5}","qty":"{quote}","quoteQty":"{quote}","time":{at},"isBuyerMaker":true}}"#
                        )
                    })
                    .collect();
                format!("[{}]", trades.join(","))
            };
            Ok(Response {
                status_code: 200,
                headers: HashMap::new(),
                body,
            })
        }
    }

    /// A factory whose transport is `http`.
    pub(crate) struct LatencyFactory {
        mock: MockClientFactory,
        pub(crate) http: Arc<LatencyHttp>,
    }

    impl LatencyFactory {
        pub(crate) fn new(venues: &[(&str, Answer)]) -> Self {
            Self {
                mock: MockClientFactory::new(),
                http: LatencyHttp::new(venues),
            }
        }
    }

    impl scope::chains::ChainClientFactory for LatencyFactory {
        fn create_chain_client(
            &self,
            chain: &str,
        ) -> scope::error::Result<Box<dyn scope::chains::ChainClient>> {
            self.mock.create_chain_client(chain)
        }

        fn create_dex_client(&self) -> Box<dyn scope::chains::DexDataSource> {
            self.mock.create_dex_client()
        }

        fn http(&self) -> Arc<dyn HttpClient> {
            self.http.clone()
        }
    }

    /// A pair `id` on `venue` with one market rule.
    pub(crate) fn pair_on(id: &str, venue: &str) -> Target {
        let mut t = Target::new(TargetKind::Pair, id);
        t.venues = vec![venue.into()];
        t.rules = vec![Rule::parse("mid_price > 0").unwrap()];
        t
    }

    /// One USDC pair target on each of `venues`.
    fn one_each(venues: &[&str]) -> Vec<Target> {
        venues.iter().map(|v| pair_on("USDC", v)).collect()
    }

    /// Runs a tick and returns it with its time on the paused clock.
    async fn timed(
        ready: &Ready,
        factory: &LatencyFactory,
        history: &mut History,
        limits: Limits,
    ) -> (Tick, Duration) {
        let config = scope::config::Config::default();
        let src = Sources {
            config: &config,
            clients: factory,
        };
        let start = Instant::now();
        let tick = tick_with(ready, src, &quiet(), history, limits).await;
        (tick, start.elapsed())
    }

    fn all_pass(tick: &Tick) {
        for row in &tick.snapshot.targets {
            assert_eq!(row.status, TargetStatus::Passed, "{row:?}");
        }
    }

    fn at_latency(venues: &[&'static str]) -> Vec<(&'static str, Answer)> {
        venues
            .iter()
            .map(|v| (*v, Answer::After(LATENCY)))
            .collect()
    }

    /// This is the feature: four venues cost the time of one. Read one after
    /// the other they would cost four times as much.
    #[tokio::test(start_paused = true)]
    async fn reads_of_different_venues_run_at_the_same_time() {
        let venues = ["lat-a", "lat-b", "lat-c", "lat-d"];
        let factory = LatencyFactory::new(&at_latency(&venues));
        let ready = ready(one_each(&venues));
        let (tick, took) =
            timed(&ready, &factory, &mut History::default(), Limits::default()).await;
        all_pass(&tick);
        assert_eq!(took, READ);
        assert_eq!(tick.timing.wall_ms, READ.as_millis() as u64);
        assert_eq!(tick.timing.sum_ms, 4 * READ.as_millis() as u64);
        assert_eq!(factory.http.max_in_flight(), 4);
    }

    /// The bound is the protection against too many requests at once. It
    /// must fail if the bound is 1 (the tick would take 6 reads) or 6 (one).
    #[tokio::test(start_paused = true)]
    async fn the_bound_limits_the_reads_at_the_same_time() {
        let venues = ["lat-a", "lat-b", "lat-c", "lat-d", "lat-e", "lat-f"];
        let factory = LatencyFactory::new(&at_latency(&venues));
        let ready = ready(one_each(&venues));
        let limits = Limits {
            max_concurrent: 2,
            ..Limits::default()
        };
        let (tick, took) = timed(&ready, &factory, &mut History::default(), limits).await;
        all_pass(&tick);
        assert_eq!(took, 3 * READ);
        assert_eq!(factory.http.max_in_flight(), 2);
        assert_eq!(tick.timing.bound, 2);
    }

    /// One read at a time for each venue keeps the pattern of requests to
    /// each host as it was before the change, so a rate limit that a venue
    /// sets does not become a venue error (and exit code 2).
    #[tokio::test(start_paused = true)]
    async fn one_venue_gets_one_read_at_a_time() {
        let factory = LatencyFactory::new(&at_latency(&["lat-a", "lat-b"]));
        let ready = ready(vec![
            pair_on("USDC", "lat-a"),
            pair_on("DAI", "lat-a"),
            pair_on("FRAX", "lat-a"),
            pair_on("USDC", "lat-b"),
        ]);
        let (tick, took) =
            timed(&ready, &factory, &mut History::default(), Limits::default()).await;
        all_pass(&tick);
        assert_eq!(factory.http.max_in_flight_to("lat-a"), 1);
        assert_eq!(took, 3 * READ, "the three reads of lat-a are in a line");
        // The reads of one group start when the one before ends.
        let starts: Vec<u64> = tick
            .timing
            .reads
            .iter()
            .filter(|r| r.group == "venue:lat-a")
            .map(|r| r.started_ms)
            .collect();
        let read_ms = READ.as_millis() as u64;
        assert_eq!(starts, [0, read_ms, 2 * read_ms]);
    }

    /// The report and the CSV zip the rows with the summaries by position,
    /// so the order must not follow the speed of the venues.
    #[tokio::test(start_paused = true)]
    async fn the_order_of_the_results_does_not_depend_on_their_speed() {
        let venues = ["lat-a", "lat-b", "lat-c", "lat-d"];
        // The first venue is the slowest.
        let answers: Vec<(&str, Answer)> = venues
            .iter()
            .enumerate()
            .map(|(i, v)| (*v, Answer::After(LATENCY * (4 - i as u32))))
            .collect();
        let factory = LatencyFactory::new(&answers);
        let ready = ready(one_each(&venues));
        let (tick, _) = timed(&ready, &factory, &mut History::default(), Limits::default()).await;
        let labels: Vec<_> = tick
            .snapshot
            .targets
            .iter()
            .map(|r| r.target.clone())
            .collect();
        assert_eq!(
            labels,
            ["USDC@lat-a", "USDC@lat-b", "USDC@lat-c", "USDC@lat-d"]
        );
        for (i, summary) in tick.summaries.iter().enumerate() {
            let mid = summary.as_ref().and_then(|s| s.mid_price).unwrap();
            assert!(
                (mid - price(i)).abs() < 1e-9,
                "row {i} holds the summary of another venue: {mid}"
            );
        }
        let reads: Vec<_> = tick.timing.reads.iter().map(|r| r.row.clone()).collect();
        assert_eq!(reads, labels, "the timing record is in the given order too");
    }

    /// Concurrency must change the time of a tick and never its result. Two
    /// ticks with the since-last-tick rules make the `History` part of the
    /// proof.
    #[tokio::test(start_paused = true)]
    async fn the_result_is_the_result_of_one_read_at_a_time() {
        let venues = ["lat-a", "lat-b", "lat-c"];
        let targets = || -> Vec<Target> {
            let mut list = Vec::new();
            for v in venues {
                for id in ["USDC", "DAI"] {
                    let mut t = pair_on(id, v);
                    t.rules = vec![
                        Rule::parse("volume_change_pct <= 100").unwrap(),
                        Rule::parse("largest_trade < 1000").unwrap(),
                        Rule::parse("mid_price > 0").unwrap(),
                    ];
                    list.push(t);
                }
            }
            list
        };
        let mut seen = Vec::new();
        for max_concurrent in [1, MAX_CONCURRENT_READS] {
            let factory = LatencyFactory::new(&at_latency(&venues));
            let ready = ready(targets());
            let limits = Limits {
                max_concurrent,
                ..Limits::default()
            };
            let mut history = History::default();
            let mut ticks = Vec::new();
            for _ in 0..2 {
                let (tick, _) = timed(&ready, &factory, &mut history, limits).await;
                ticks.push(format!("{:?} {:?}", tick.snapshot, tick.summaries));
            }
            seen.push(ticks);
        }
        assert_eq!(seen[0], seen[1]);
        // The proof is empty if the second tick has no volume change: the
        // venue gives a new volume to each read.
        assert!(
            seen[0][1].contains("volume_change_pct <= 100") && seen[0][1].contains("value: Some(1"),
            "{}",
            seen[0][1]
        );
    }

    /// A venue that never answers must not stop a cron job: the tick ends at
    /// the timeout, the cell has the exact error and the others are normal.
    #[tokio::test(start_paused = true)]
    async fn a_read_that_never_answers_ends_at_the_timeout() {
        let factory = LatencyFactory::new(&[
            ("lat-a", Answer::After(LATENCY)),
            ("lat-b", Answer::Never),
            ("lat-c", Answer::After(LATENCY)),
        ]);
        let ready = ready(one_each(&["lat-a", "lat-b", "lat-c"]));
        let config = scope::config::Config::default();
        let src = Sources {
            config: &config,
            clients: &factory,
        };
        let start = Instant::now();
        // `tick`, not `tick_with`: the default timeout is the one in use.
        let tick = tick(&ready, src, &quiet(), &mut History::default()).await;
        assert_eq!(start.elapsed(), READ_TIMEOUT);
        let [a, b, c] = tick.snapshot.targets.as_slice() else {
            panic!("{:?}", tick.snapshot)
        };
        assert_eq!(a.status, TargetStatus::Passed);
        assert_eq!(c.status, TargetStatus::Passed);
        assert_eq!(b.status, TargetStatus::Unknown, "{b:?}");
        assert_eq!(
            b.cells[0].status,
            RuleStatus::Error("read timed out after 60s".to_string())
        );
        let hung = &tick.timing.reads[1];
        assert_eq!(hung.outcome, Outcome::Timeout);
        assert_eq!(hung.took_ms, READ_TIMEOUT.as_millis() as u64);
        assert_eq!(hung.error.as_deref(), Some("read timed out after 60s"));
        assert_eq!(
            factory.http.in_flight(),
            0,
            "the dropped request counts out"
        );
    }

    /// One failed read and one slow read do not stop the fast one, and the
    /// failure is a result like the others.
    #[tokio::test(start_paused = true)]
    async fn a_failed_read_and_a_slow_read_do_not_stop_the_others() {
        let factory = LatencyFactory::new(&[
            ("lat-a", Answer::Fails),
            ("lat-b", Answer::After(5 * LATENCY)),
            ("lat-c", Answer::After(LATENCY)),
        ]);
        let ready = ready(one_each(&["lat-a", "lat-b", "lat-c"]));
        let (tick, took) =
            timed(&ready, &factory, &mut History::default(), Limits::default()).await;
        let statuses: Vec<_> = tick.snapshot.targets.iter().map(|r| r.status).collect();
        assert_eq!(
            statuses,
            [
                TargetStatus::Unknown,
                TargetStatus::Passed,
                TargetStatus::Passed
            ]
        );
        let RuleStatus::Error(text) = &tick.snapshot.targets[0].cells[0].status else {
            panic!("{:?}", tick.snapshot.targets[0])
        };
        assert!(text.contains("lat-a is down"), "the exact error: {text}");
        assert_eq!(took, 5 * READ, "the slow read sets the time");
        let outcomes: Vec<_> = tick.timing.reads.iter().map(|r| r.outcome).collect();
        assert_eq!(outcomes, [Outcome::Error, Outcome::Ok, Outcome::Ok]);
    }

    /// A tick that is longer than the interval moves the grid: concurrency
    /// inside a tick must not become concurrency between ticks. One venue
    /// would show two requests at once if two ticks overlapped.
    #[tokio::test(start_paused = true)]
    async fn a_tick_longer_than_the_interval_does_not_overlap_the_next() {
        let factory = LatencyFactory::new(&at_latency(&["lat-a"]));
        let mut ready = ready(one_each(&["lat-a"]));
        ready.list.interval = LATENCY;
        ready.list.duration = Some(3 * READ);
        let config = scope::config::Config::default();
        let src = Sources {
            config: &config,
            clients: &factory,
        };
        let mut ticks = 0;
        crate::cli::watch::run::run_ticks(
            &ready,
            src,
            &quiet(),
            History::default(),
            |_, _, _, _, _| {
                ticks += 1;
            },
        )
        .await
        .unwrap();
        assert!(ticks >= 2, "the run had {ticks} ticks");
        assert_eq!(factory.http.max_in_flight(), 1);
    }

    /// A signal drops the whole tick. No read may stay behind, and none may
    /// start after.
    #[tokio::test(start_paused = true)]
    async fn a_stop_in_the_middle_of_a_tick_leaves_no_read_behind() {
        let venues = ["lat-a", "lat-b", "lat-c"];
        let factory = LatencyFactory::new(&at_latency(&venues));
        let ready = ready(one_each(&venues));
        let config = scope::config::Config::default();
        let src = Sources {
            config: &config,
            clients: &factory,
        };
        let mut history = History::default();
        let out = quiet();
        tokio::select! {
            _ = tick(&ready, src, &out, &mut history) => panic!("the tick ended first"),
            () = tokio::time::sleep(LATENCY / 2) => {}
        }
        assert_eq!(factory.http.in_flight(), 0);
        let started = factory.http.started();
        assert_eq!(started, 3, "one request of each venue had started");
        tokio::time::sleep(10 * READ).await;
        assert_eq!(
            factory.http.started(),
            started,
            "nothing ran on after the drop"
        );
    }

    /// Tests pass small limits so they do not wait for the real ones, and
    /// the real ones must not drift.
    #[tokio::test(start_paused = true)]
    async fn the_limits_are_the_named_constants_and_tick_with_takes_others() {
        assert_eq!(
            Limits::default(),
            Limits {
                max_concurrent: 8,
                read_timeout: Duration::from_secs(60),
            }
        );
        assert_eq!(MAX_CONCURRENT_READS, 8);
        assert_eq!(READ_TIMEOUT, Duration::from_secs(60));

        let factory = LatencyFactory::new(&[("lat-a", Answer::Never)]);
        let ready = ready(one_each(&["lat-a"]));
        let limits = Limits {
            read_timeout: 5 * LATENCY,
            ..Limits::default()
        };
        let (tick, took) = timed(&ready, &factory, &mut History::default(), limits).await;
        assert_eq!(took, 5 * LATENCY);
        assert_eq!(
            tick.snapshot.targets[0].cells[0].status,
            RuleStatus::Error("read timed out after 10s".to_string())
        );
    }

    /// A bound of 0 would never start a read; it is read as 1.
    #[tokio::test(start_paused = true)]
    async fn a_bound_of_zero_still_reads() {
        let factory = LatencyFactory::new(&at_latency(&["lat-a", "lat-b"]));
        let ready = ready(one_each(&["lat-a", "lat-b"]));
        let limits = Limits {
            max_concurrent: 0,
            ..Limits::default()
        };
        let (tick, took) = timed(&ready, &factory, &mut History::default(), limits).await;
        all_pass(&tick);
        assert_eq!(took, 2 * READ);
    }

    /// Design 2026-11-11, section 3.1 and test 4 (Q11). Two targets with
    /// the same id and venue are valid in a list. Each must read the memory
    /// of its own: on a first tick both have none. If they share one entry,
    /// the second target reads what the first one has just written, and its
    /// cells differ from the first one's.
    #[tokio::test(start_paused = true)]
    async fn two_targets_with_one_id_and_venue_do_not_share_a_memory() {
        let factory = LatencyFactory::new(&at_latency(&["lat-a"]));
        let mut twin = pair_on("USDC", "lat-a");
        twin.rules = vec![
            Rule::parse("volume_change_pct <= 100").unwrap(),
            Rule::parse("largest_trade < 100000").unwrap(),
        ];
        let ready = ready(vec![twin.clone(), twin]);
        let (tick, _) = timed(&ready, &factory, &mut History::default(), Limits::default()).await;
        let [first, second] = tick.snapshot.targets.as_slice() else {
            panic!("{:?}", tick.snapshot)
        };
        let cells = |r: &TargetSnapshot| {
            r.cells
                .iter()
                .map(|c| (c.value, c.status.clone()))
                .collect::<Vec<_>>()
        };
        assert!(
            matches!(first.cells[0].status, RuleStatus::NotApplicable(_)),
            "a first tick has no second reading: {first:?}"
        );
        assert_eq!(cells(first), cells(second), "{first:?} / {second:?}");
    }
}
