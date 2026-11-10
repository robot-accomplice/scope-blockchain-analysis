//! One tick: read every target's metrics and judge its rules.
//!
//! Each source is read once per target per tick, however many rules use
//! it. A failed read makes that source's cells `Error` with the exact
//! error text; other sources and other targets go on. The metrics that
//! compare with the tick before read the run's [`History`].

use super::{Ready, Sources};
use crate::cli::address::{AddressArgs, analyze_address};
use crate::cli::crawl::{self, Period};
use crate::cli::market::CrawlDexSource;
use crate::cli::output::Output;
use scope::compliance::datasource::BlockchainDataClient;
use scope::compliance::risk::RiskEngine;
use scope::market::{
    HealthThresholds, MarketSummary, MarketVenue, Pair, Trade, fetch_summary_with_trades,
};
use scope::metrics::{MetricGroup, MetricId, describe, sample};
use scope::watch::{
    Cell, History, Preset, Reading, Readings, RuleStatus, Snapshot, Target, TargetSnapshot, preset,
};

/// The cell a preset shows when its summary could not be read.
pub const PRESET_ERROR_LABEL: &str = "market health";

/// One tick's results.
#[derive(Debug, Clone, Default)]
pub struct Tick {
    pub snapshot: Snapshot,
    /// For each row of the snapshot, the market summary it read, for the
    /// report and the detail pane.
    pub summaries: Vec<Option<MarketSummary>>,
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
    let mut result = Tick::default();
    let clock = Clock::now(ready);
    for (t, contract) in ready.list.targets.iter().zip(&ready.contracts) {
        for reading in read_target(t, contract.as_deref(), src, out, history, clock).await {
            result.snapshot.targets.push(reading.row);
            result.summaries.push(reading.summary);
        }
    }
    result
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
    let thresholds = preset::thresholds(t, &src.config.market.health);
    let token = if t.needs(MetricGroup::Token) {
        Some(read_token(t, contract, src, out).await)
    } else {
        None
    };
    let address = if t.needs(MetricGroup::Address) {
        Some(read_address(t, src, out).await)
    } else {
        None
    };
    let rows = t.rows();
    let one_row = rows.len() == 1;
    let mut result = Vec::with_capacity(rows.len());
    for venue in rows {
        let label = t.row_label(venue);
        let mut cells = Vec::new();
        let mut summary = None;
        let mut market: Option<SourceResult> = None;
        if let Some(venue) = venue
            && t.needs_market()
        {
            match read_market(t, venue, contract, &thresholds, src, out).await {
                Ok((s, trades)) => {
                    if t.preset == Some(Preset::MarketHealth) {
                        cells.extend(preset::cells(&s));
                    }
                    let since = t.rules.iter().any(|r| r.metric.since_last_tick()).then(|| {
                        let tape = trades.as_deref().map_err(String::as_str);
                        history.read(&label, tape, s.volume_24h, clock.now_ms, clock.window_ms)
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
}
