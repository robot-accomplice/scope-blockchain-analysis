//! One tick: read every target's metrics and judge its rules.
//!
//! Each source is read once per target per tick, however many rules use
//! it. A failed read makes that source's cells `Error` with the exact
//! error text; other sources and other targets go on.

use super::{Ready, Sources};
use crate::cli::address::{AddressArgs, analyze_address};
use crate::cli::crawl::{self, Period};
use crate::cli::market::CrawlDexSource;
use crate::cli::output::Output;
use scope::compliance::datasource::BlockchainDataClient;
use scope::compliance::risk::RiskEngine;
use scope::market::{HealthThresholds, MarketSummary, MarketVenue, Pair, fetch_summary};
use scope::metrics::{MetricGroup, MetricId, describe, sample};
use scope::watch::{Cell, Preset, RuleStatus, Snapshot, Target, TargetSnapshot, preset};

/// The cell a preset shows when its summary could not be read.
pub const PRESET_ERROR_LABEL: &str = "market health";

/// One tick's results.
#[derive(Debug, Clone, Default)]
pub struct Tick {
    pub snapshot: Snapshot,
    /// For each target, the market summary it read, for the report and
    /// the detail pane.
    pub summaries: Vec<Option<MarketSummary>>,
}

/// One target's results.
#[derive(Debug, Clone)]
pub struct TargetReading {
    pub row: TargetSnapshot,
    pub summary: Option<MarketSummary>,
}

/// Reads and judges every target of `ready`, in order.
pub async fn tick(ready: &Ready, src: Sources<'_>, out: &Output) -> Tick {
    let mut result = Tick::default();
    for (t, contract) in ready.list.targets.iter().zip(&ready.contracts) {
        let reading = read_target(t, contract.as_deref(), src, out).await;
        result.snapshot.targets.push(reading.row);
        result.summaries.push(reading.summary);
    }
    result
}

/// What one source gave: values by metric, or the error text.
type SourceResult = Result<Vec<(MetricId, Option<f64>)>, String>;

/// Reads and judges one target.
pub async fn read_target(
    t: &Target,
    contract: Option<&str>,
    src: Sources<'_>,
    out: &Output,
) -> TargetReading {
    let thresholds = preset::thresholds(t, &src.config.market.health);
    let mut cells = Vec::new();
    let mut summary = None;

    let mut market: Option<SourceResult> = None;
    if t.needs_market() {
        match read_market(t, contract, &thresholds, src, out).await {
            Ok(s) => {
                if t.preset == Some(Preset::MarketHealth) {
                    cells.extend(preset::cells(&s));
                }
                market = Some(Ok(market_values(t, &s, &thresholds)));
                summary = Some(s);
            }
            Err(e) => {
                if t.preset == Some(Preset::MarketHealth) {
                    cells.push(Cell {
                        label: PRESET_ERROR_LABEL.to_string(),
                        rule: None,
                        value: None,
                        status: RuleStatus::Error(e.clone()),
                    });
                }
                market = Some(Err(e));
            }
        }
    }
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

    for rule in &t.rules {
        let source = match rule.metric.group() {
            MetricGroup::Market => market.as_ref(),
            MetricGroup::Token => token.as_ref(),
            MetricGroup::Address => address.as_ref(),
        };
        let (value, status) = match source {
            Some(Ok(values)) => {
                let value = values
                    .iter()
                    .find(|(id, _)| *id == rule.metric)
                    .and_then(|(_, v)| *v);
                (value, rule.eval(value))
            }
            Some(Err(e)) => (None, RuleStatus::Error(e.clone())),
            None => (None, RuleStatus::Error("no source".to_string())),
        };
        cells.push(Cell {
            label: rule.to_string(),
            rule: Some(*rule),
            value,
            status,
        });
    }
    TargetReading {
        row: TargetSnapshot::new(t.label(), cells),
        summary,
    }
}

/// The market summary of a target, the same fetch as `market summary`.
async fn read_market(
    t: &Target,
    contract: Option<&str>,
    thresholds: &HealthThresholds,
    src: Sources<'_>,
    out: &Output,
) -> Result<MarketSummary, String> {
    let venue_name = t.venue.as_deref().ok_or("no venue")?;
    let venue = MarketVenue::from_name_with_http(venue_name, src.clients.http())
        .map_err(|e| e.to_string())?;
    let dex = CrawlDexSource {
        factory: src.clients,
        out,
        token: contract.map(str::to_string),
    };
    fetch_summary(&venue, &Pair::parse(&t.id), thresholds, &dex)
        .await
        .map_err(|e| e.to_string())
}

/// Every market value of the target's rules, read from `summary` with the
/// target's thresholds (the same values the repeating view charts).
fn market_values(
    t: &Target,
    summary: &MarketSummary,
    thresholds: &HealthThresholds,
) -> Vec<(MetricId, Option<f64>)> {
    let table: Vec<_> = t
        .rules
        .iter()
        .filter(|r| r.metric.group() == MetricGroup::Market)
        .map(|r| describe(r.metric, thresholds))
        .collect();
    let s = sample(summary, &table);
    table.iter().map(|m| m.id).zip(s.values).collect()
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
    Ok(vec![
        (MetricId::PriceUsd, Some(a.price_usd)),
        (MetricId::PriceChange24hPct, Some(a.price_change_24h)),
        (MetricId::Volume24h, Some(a.volume_24h)),
        (MetricId::LiquidityUsd, Some(a.liquidity_usd)),
        (MetricId::Holders, Some(a.total_holders as f64)),
    ])
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
    Ok(values)
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
mod tests {
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
        let tick = tick(&ready(vec![t]), src, &quiet()).await;
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

    #[tokio::test]
    async fn a_failed_source_errors_its_cells_and_the_next_target_still_runs() {
        let factory = MockClientFactory::new();
        let config = scope::config::Config::default();
        let src = Sources {
            config: &config,
            clients: &factory,
        };
        let mut bad = Target::new(TargetKind::Pair, "USDC");
        bad.venue = Some("no-such-venue".into());
        bad.preset = Some(Preset::MarketHealth);
        bad.rules = vec![Rule::parse("spread_pct < 1").unwrap()];
        let mut good = Target::new(
            TargetKind::Token,
            "0x1234567890123456789012345678901234567890",
        );
        good.rules = vec![Rule::parse("price_usd > 0").unwrap()];
        let tick = tick(&ready(vec![bad, good]), src, &quiet()).await;
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
}
