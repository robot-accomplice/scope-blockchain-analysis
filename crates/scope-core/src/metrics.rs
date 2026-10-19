//! The metric catalog: one table of key values, one reading per run.
//!
//! Market metrics are the key values of a market summary. Token and address
//! metrics are read by watchlists (`scope::watch`). Each [`MetricId`] has one
//! [`MetricGroup`]; [`describe`] gives its label, unit and threshold.
//!
//! [`summary_metrics`] defines the market list once. Labels, units, thresholds and
//! CSV column names all come from it. [`sample`] reads one [`MarketSummary`]
//! into values in the same order as the table, and [`csv_columns`] gives the
//! CSV order of the same table.
//!
//! Every market value is a field the summary computed once for its health checks
//! (spread, top-N depth, health). This module only reads them, so a charted
//! value and the check that judged it cannot differ.
//!
//! The table has no `timestamp` or `run` entry: the CSV writer adds those
//! two columns in front. This module has no presentation types.

use crate::market::health::HealthThresholds;
use crate::market::health::MarketSummary;
use crate::market::types::HealthCheck;

/// Declares [`MetricId`] and [`MetricId::ALL`] from one list of variants, so
/// the list of all ids cannot fall out of step with the enum.
macro_rules! metric_ids {
    ($($(#[$doc:meta])* $name:ident),+ $(,)?) => {
        /// Identifies one metric of the catalog.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum MetricId {
            $($(#[$doc])* $name),+
        }

        impl MetricId {
            /// Every id, in table order. `describe` matches over each
            /// variant, so a new variant also needs a table entry.
            pub const ALL: &'static [MetricId] = &[$(MetricId::$name),+];
        }
    };
}

metric_ids! {
    BestBid,
    BestAsk,
    Mid,
    /// `(mid - peg_target) / peg_target * 100`.
    PegDeviationPct,
    /// Absolute spread of the best valid bid and ask.
    Spread,
    /// The same spread in percent of its mid.
    SpreadPct,
    BidDepth,
    AskDepth,
    BidTop3,
    AskTop3,
    BidTop10,
    AskTop10,
    /// `1.0` when [`Sample::healthy`] is true, else `0.0`.
    Healthy,
    /// Token price in USD.
    PriceUsd,
    /// Token price change over 24h, in percent.
    PriceChange24hPct,
    /// Token trading volume over 24h, in USD.
    Volume24h,
    /// Token liquidity in USD.
    LiquidityUsd,
    /// Number of token holders.
    Holders,
    /// An address's balance of the chain's native coin.
    NativeBalance,
    /// An address's overall risk score, 0 to 10.
    RiskScore,
}

/// Where a metric's value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricGroup {
    /// A market summary (order book health).
    Market,
    /// Token analytics (DEX data).
    Token,
    /// An address analysis or its risk assessment.
    Address,
}

impl MetricId {
    /// The group of this metric.
    pub fn group(self) -> MetricGroup {
        use MetricId::*;
        match self {
            BestBid | BestAsk | Mid | PegDeviationPct | Spread | SpreadPct | BidDepth
            | AskDepth | BidTop3 | AskTop3 | BidTop10 | AskTop10 | Healthy => MetricGroup::Market,
            PriceUsd | PriceChange24hPct | Volume24h | LiquidityUsd | Holders => MetricGroup::Token,
            NativeBalance | RiskScore => MetricGroup::Address,
        }
    }

    /// The metric's name in rules and CSV files (`spread_pct`, `holders`).
    pub fn name(self) -> &'static str {
        describe(self, &HealthThresholds::default()).csv_column
    }

    /// The metric with this name, if any.
    pub fn from_name(name: &str) -> Option<MetricId> {
        MetricId::ALL.iter().copied().find(|id| id.name() == name)
    }
}

/// The limit a metric is judged against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Threshold {
    /// The value must not exceed this.
    Max(f64),
    /// The value must not fall below this.
    Min(f64),
    /// The value should be at this.
    Target(f64),
}

/// What a metric's number measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// A price in the quote asset.
    Price,
    Percent,
    /// An amount in the quote asset (for example USDT).
    Quote,
    /// `1.0` or `0.0`.
    Flag,
    /// An amount in US dollars.
    Usd,
    /// A whole number of things (holders).
    Count,
    /// An amount of a chain's native coin.
    Native,
    /// A score (risk, 0 to 10).
    Score,
}

/// One entry of the metric table.
#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub id: MetricId,
    pub label: &'static str,
    pub unit: Unit,
    pub threshold: Option<Threshold>,
    pub csv_column: &'static str,
}

/// One run's reading. `values[i]` belongs to the metric at index `i` of the
/// table passed to [`sample`]. A missing value is `None`, never zero.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub values: Vec<Option<f64>>,
    /// The one source of health: the `Healthy` metric value is derived from it.
    pub healthy: bool,
    /// Messages of the checks that failed, in check order.
    pub failed_checks: Vec<String>,
}

/// A CSV column the caller needs is missing from the table it passed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("the metric table has no entry for {0:?}")]
pub struct MissingMetric(pub MetricId);

/// CSV columns that exist today, in their fixed order. Other columns follow.
const LEGACY_CSV_ORDER: [MetricId; 7] = [
    MetricId::BestBid,
    MetricId::BestAsk,
    MetricId::Mid,
    MetricId::Spread,
    MetricId::BidDepth,
    MetricId::AskDepth,
    MetricId::Healthy,
];

/// The market metric table, built from the thresholds in force.
pub fn summary_metrics(thresholds: &HealthThresholds) -> Vec<Metric> {
    MetricId::ALL
        .iter()
        .filter(|id| id.group() == MetricGroup::Market)
        .map(|id| describe(*id, thresholds))
        .collect()
}

/// The catalog entry of `id`. Market thresholds come from `t`; token and
/// address metrics have no default threshold.
pub fn describe(id: MetricId, t: &HealthThresholds) -> Metric {
    use MetricId::*;
    use Threshold::{Max, Min, Target};
    let (label, unit, threshold, csv_column) = match id {
        BestBid => ("Best bid", Unit::Price, None, "best_bid"),
        BestAsk => ("Best ask", Unit::Price, None, "best_ask"),
        Mid => (
            "Mid price",
            Unit::Price,
            Some(Target(t.peg_target)),
            "mid_price",
        ),
        PegDeviationPct => ("Peg deviation %", Unit::Percent, None, "peg_deviation_pct"),
        Spread => ("Spread", Unit::Price, None, "spread"),
        SpreadPct => (
            "Spread %",
            Unit::Percent,
            Some(Max(t.max_spread_pct)),
            "spread_pct",
        ),
        BidDepth => (
            "Bid depth",
            Unit::Quote,
            Some(Min(t.min_depth)),
            "bid_depth",
        ),
        AskDepth => (
            "Ask depth",
            Unit::Quote,
            Some(Min(t.min_depth)),
            "ask_depth",
        ),
        BidTop3 => (
            "Bid top-3 depth",
            Unit::Quote,
            Some(Min(t.min_top3_depth)),
            "bid_top3_depth",
        ),
        AskTop3 => (
            "Ask top-3 depth",
            Unit::Quote,
            Some(Min(t.min_top3_depth)),
            "ask_top3_depth",
        ),
        BidTop10 => (
            "Bid top-10 depth",
            Unit::Quote,
            Some(Min(t.min_top10_depth)),
            "bid_top10_depth",
        ),
        AskTop10 => (
            "Ask top-10 depth",
            Unit::Quote,
            Some(Min(t.min_top10_depth)),
            "ask_top10_depth",
        ),
        Healthy => ("Health", Unit::Flag, None, "healthy"),
        PriceUsd => ("Price", Unit::Usd, None, "price_usd"),
        PriceChange24hPct => (
            "Price change 24h %",
            Unit::Percent,
            None,
            "price_change_24h_pct",
        ),
        Volume24h => ("Volume 24h", Unit::Usd, None, "volume_24h"),
        LiquidityUsd => ("Liquidity", Unit::Usd, None, "liquidity_usd"),
        Holders => ("Holders", Unit::Count, None, "holders"),
        NativeBalance => ("Native balance", Unit::Native, None, "native_balance"),
        RiskScore => ("Risk score", Unit::Score, None, "risk_score"),
    };
    Metric {
        id,
        label,
        unit,
        threshold,
        csv_column,
    }
}

/// The table in CSV column order: today's columns first, then the rest in
/// table order. The writer adds `timestamp` and `run` before these.
///
/// Fails when the table lacks one of today's columns, so a writer never
/// emits a header that silently drops it.
pub fn csv_columns(metrics: &[Metric]) -> Result<Vec<&Metric>, MissingMetric> {
    let mut columns = Vec::with_capacity(metrics.len());
    for id in LEGACY_CSV_ORDER {
        let metric = metrics
            .iter()
            .find(|m| m.id == id)
            .ok_or(MissingMetric(id))?;
        columns.push(metric);
    }
    columns.extend(metrics.iter().filter(|m| !LEGACY_CSV_ORDER.contains(&m.id)));
    Ok(columns)
}

/// Reads one summary into values, in the order of `metrics`.
pub fn sample(summary: &MarketSummary, metrics: &[Metric]) -> Sample {
    let healthy = summary.healthy;
    Sample {
        values: metrics
            .iter()
            .map(|m| value_of(m.id, summary, healthy))
            .collect(),
        healthy,
        failed_checks: summary
            .checks
            .iter()
            .filter_map(|c| match c {
                HealthCheck::Fail(message) => Some(message.clone()),
                _ => None,
            })
            .collect(),
    }
}

fn value_of(id: MetricId, s: &MarketSummary, healthy: bool) -> Option<f64> {
    match id {
        MetricId::BestBid => s.best_bid,
        MetricId::BestAsk => s.best_ask,
        MetricId::Mid => s.mid_price,
        MetricId::PegDeviationPct => s.mid_price.and_then(|mid| s.peg_deviation_pct(mid)),
        MetricId::Spread => s.spread,
        MetricId::SpreadPct => s.spread_pct,
        MetricId::BidDepth => Some(s.bid_depth),
        MetricId::AskDepth => Some(s.ask_depth),
        MetricId::BidTop3 => Some(s.top_depth.bid_top3),
        MetricId::AskTop3 => Some(s.top_depth.ask_top3),
        MetricId::BidTop10 => Some(s.top_depth.bid_top10),
        MetricId::AskTop10 => Some(s.top_depth.ask_top10),
        MetricId::Healthy => Some(f64::from(u8::from(healthy))),
        // A market summary has no token or address values.
        _ => None,
    }
}

/// The CSV columns every series file starts with, before the metrics.
const CSV_LEAD: &str = "timestamp,run";

/// What a CSV cell holds when the metric has no value.
const CSV_NO_VALUE: &str = "-";

/// The CSV header line for `columns` (see [`csv_columns`]), with its newline.
pub fn csv_header(columns: &[&Metric]) -> String {
    let mut line = CSV_LEAD.to_string();
    for m in columns {
        line.push(',');
        line.push_str(m.csv_column);
    }
    line.push('\n');
    line
}

/// One CSV row, with its newline. `sample` reads in the order of `metrics`
/// (the table [`sample`] used); `columns` sets the cell order. A flag is
/// `true` or `false`; a missing value is `-`.
pub fn csv_row(
    columns: &[&Metric],
    metrics: &[Metric],
    timestamp: &str,
    run: u64,
    sample: &Sample,
) -> String {
    let mut line = format!("{timestamp},{run}");
    for column in columns {
        let value = metrics
            .iter()
            .position(|m| m.id == column.id)
            .and_then(|i| sample.values.get(i).copied().flatten());
        line.push(',');
        match (column.unit, value) {
            (Unit::Flag, Some(v)) => line.push_str(if v != 0.0 { "true" } else { "false" }),
            (_, Some(v)) => line.push_str(&v.to_string()),
            (_, None) => line.push_str(CSV_NO_VALUE),
        }
    }
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::health::{HealthThresholds, MarketSummary};
    use crate::market::types::{BookSource, OrderBook, OrderBookLevel};

    fn side(start: f64, step: f64, qtys: &[f64]) -> Vec<OrderBookLevel> {
        qtys.iter()
            .enumerate()
            .map(|(i, &quantity)| OrderBookLevel {
                price: start + step * i as f64,
                quantity,
            })
            .collect()
    }

    fn book(bids: Vec<OrderBookLevel>, asks: Vec<OrderBookLevel>) -> OrderBook {
        OrderBook {
            pair: "USDC/USDT".to_string(),
            source: BookSource::Exchange,
            bids,
            asks,
        }
    }

    fn healthy_book() -> OrderBook {
        book(
            side(0.9999, -0.0001, &[400.0; 10]),
            side(1.0001, 0.0001, &[400.0; 10]),
        )
    }

    fn summarize(book: &OrderBook) -> MarketSummary {
        MarketSummary::from_order_book(book, &HealthThresholds::default(), None)
    }

    fn healthy_summary() -> MarketSummary {
        summarize(&healthy_book())
    }

    fn thin_summary() -> MarketSummary {
        summarize(&book(
            side(0.9999, -0.0001, &[1.0; 2]),
            side(1.0001, 0.0001, &[1.0; 2]),
        ))
    }

    /// Sum of the first `n` levels, the same way the health module sums them.
    fn top_sum(levels: &[OrderBookLevel], n: usize) -> f64 {
        levels.iter().take(n).map(OrderBookLevel::value).sum()
    }

    fn metric(metrics: &[Metric], id: MetricId) -> &Metric {
        metrics.iter().find(|m| m.id == id).expect("metric present")
    }

    fn value(sample: &Sample, metrics: &[Metric], id: MetricId) -> Option<f64> {
        let at = metrics.iter().position(|m| m.id == id).unwrap();
        sample.values[at]
    }

    #[test]
    fn summary_metrics_are_the_market_group_only() {
        let m = summary_metrics(&HealthThresholds::default());
        assert!(m.iter().all(|m| m.id.group() == MetricGroup::Market));
        assert_eq!(m.len(), 13, "the market table did not change");
    }

    #[test]
    fn every_catalog_entry_has_a_unique_name_that_maps_back() {
        let mut names = std::collections::HashSet::new();
        for &id in MetricId::ALL {
            let m = describe(id, &HealthThresholds::default());
            assert!(!m.label.is_empty(), "{id:?} has no label");
            assert!(names.insert(id.name()), "{} appears twice", id.name());
            assert_eq!(MetricId::from_name(id.name()), Some(id));
        }
        assert_eq!(MetricId::from_name("no_such_metric"), None);
    }

    #[test]
    fn token_and_address_metrics_have_their_groups_and_no_default_threshold() {
        use MetricId::*;
        for id in [
            PriceUsd,
            PriceChange24hPct,
            Volume24h,
            LiquidityUsd,
            Holders,
        ] {
            assert_eq!(id.group(), MetricGroup::Token, "{id:?}");
        }
        for id in [NativeBalance, RiskScore] {
            assert_eq!(id.group(), MetricGroup::Address, "{id:?}");
        }
        for id in [PriceUsd, Holders, NativeBalance, RiskScore] {
            assert_eq!(describe(id, &HealthThresholds::default()).threshold, None);
        }
        assert_eq!(Holders.name(), "holders");
        assert_eq!(
            describe(Holders, &HealthThresholds::default()).unit,
            Unit::Count
        );
    }

    #[test]
    fn a_summary_gives_no_value_for_a_token_metric() {
        let t = HealthThresholds::default();
        let metrics = vec![describe(MetricId::PriceUsd, &t)];
        assert_eq!(sample(&healthy_summary(), &metrics).values, vec![None]);
    }

    #[test]
    fn every_table_entry_has_a_label_and_a_unique_csv_column() {
        let metrics = summary_metrics(&HealthThresholds::default());
        assert!(!metrics.is_empty());
        let mut columns = std::collections::HashSet::new();
        for m in &metrics {
            assert!(!m.label.is_empty(), "{:?} has no label", m.id);
            assert!(!m.csv_column.is_empty(), "{:?} has no csv column", m.id);
            assert!(
                columns.insert(m.csv_column),
                "{} appears twice",
                m.csv_column
            );
        }
    }

    #[test]
    fn thresholds_come_from_health_thresholds() {
        let t = HealthThresholds {
            peg_target: 0.5,
            max_spread_pct: 1.5,
            min_depth: 700.0,
            min_top3_depth: 11.0,
            min_top10_depth: 22.0,
            ..HealthThresholds::default()
        };
        let m = summary_metrics(&t);
        assert_eq!(
            metric(&m, MetricId::Mid).threshold,
            Some(Threshold::Target(0.5))
        );
        assert_eq!(
            metric(&m, MetricId::SpreadPct).threshold,
            Some(Threshold::Max(1.5))
        );
        for id in [MetricId::BidDepth, MetricId::AskDepth] {
            assert_eq!(metric(&m, id).threshold, Some(Threshold::Min(700.0)));
        }
        for id in [MetricId::BidTop3, MetricId::AskTop3] {
            assert_eq!(metric(&m, id).threshold, Some(Threshold::Min(11.0)));
        }
        for id in [MetricId::BidTop10, MetricId::AskTop10] {
            assert_eq!(metric(&m, id).threshold, Some(Threshold::Min(22.0)));
        }
        assert_eq!(metric(&m, MetricId::Healthy).threshold, None);
    }

    #[test]
    fn sample_reads_the_summary_values() {
        let s = healthy_summary();
        let metrics = summary_metrics(&HealthThresholds::default());
        let one = sample(&s, &metrics);
        assert_eq!(one.values.len(), metrics.len());
        assert!(one.healthy);
        assert!(one.failed_checks.is_empty());
        let mid = value(&one, &metrics, MetricId::Mid).unwrap();
        assert!((mid - 1.0).abs() < 1e-12);
        // Peg deviation: (mid - peg) / peg * 100.
        let mut shifted = s.clone();
        shifted.mid_price = Some(1.002);
        let dev = value(
            &sample(&shifted, &metrics),
            &metrics,
            MetricId::PegDeviationPct,
        );
        assert!((dev.unwrap() - 0.2).abs() < 1e-9);
        // Spread and spread % are the summary's fields: 0.0002 on a mid of 1.0.
        let spread_pct = value(&one, &metrics, MetricId::SpreadPct).unwrap();
        assert!((spread_pct - 0.02).abs() < 1e-9, "{spread_pct}");
        let spread = value(&one, &metrics, MetricId::Spread).unwrap();
        assert!((spread - 0.0002).abs() < 1e-12, "{spread}");
        // Top-N depth: the exact sums of the first 3 and 10 levels.
        let b = healthy_book();
        assert_eq!(
            value(&one, &metrics, MetricId::BidTop3),
            Some(top_sum(&s.bids, 3))
        );
        assert_eq!(
            value(&one, &metrics, MetricId::AskTop10),
            Some(top_sum(&b.asks, 10))
        );
        assert_eq!(value(&one, &metrics, MetricId::Healthy), Some(1.0));
        assert_eq!(
            value(
                &sample(&thin_summary(), &metrics),
                &metrics,
                MetricId::Healthy
            ),
            Some(0.0)
        );
    }

    #[test]
    fn missing_values_stay_none() {
        let empty = MarketSummary::from_order_book(
            &book(vec![], vec![]),
            &HealthThresholds::default(),
            None,
        );
        let metrics = summary_metrics(&HealthThresholds::default());
        let one = sample(&empty, &metrics);
        for id in [
            MetricId::Mid,
            MetricId::PegDeviationPct,
            MetricId::SpreadPct,
            MetricId::BestBid,
            MetricId::BestAsk,
            MetricId::Spread,
        ] {
            assert_eq!(value(&one, &metrics, id), None, "{id:?}");
        }
        assert_eq!(value(&one, &metrics, MetricId::Healthy), Some(0.0));
    }

    #[test]
    fn failed_checks_list_the_failing_messages() {
        let thin = MarketSummary::from_order_book(
            &book(
                side(0.9999, -0.0001, &[1.0; 2]),
                side(1.0001, 0.0001, &[1.0; 2]),
            ),
            &HealthThresholds::default(),
            None,
        );
        let one = sample(&thin, &summary_metrics(&HealthThresholds::default()));
        assert!(!one.healthy);
        assert!(
            one.failed_checks
                .iter()
                .any(|m| m.starts_with("Bid levels"))
        );
        assert!(one.failed_checks.iter().any(|m| m.starts_with("Bid depth")));
        assert!(!one.failed_checks.iter().any(|m| m.starts_with("Spread")));
    }

    #[test]
    fn csv_columns_start_with_todays_header_and_append_the_rest() {
        let metrics = summary_metrics(&HealthThresholds::default());
        let columns: Vec<&str> = csv_columns(&metrics)
            .unwrap()
            .iter()
            .map(|m| m.csv_column)
            .collect();
        assert_eq!(
            columns[..7],
            [
                "best_bid",
                "best_ask",
                "mid_price",
                "spread",
                "bid_depth",
                "ask_depth",
                "healthy"
            ]
        );
        assert_eq!(columns.len(), metrics.len());
        let mut unique = columns.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), columns.len(), "csv column names are unique");
    }

    #[test]
    fn csv_columns_fail_when_a_legacy_column_is_missing() {
        let mut metrics = summary_metrics(&HealthThresholds::default());
        metrics.retain(|m| m.id != MetricId::Spread);
        assert_eq!(csv_columns(&metrics), Err(MissingMetric(MetricId::Spread)));
    }

    #[test]
    fn synthetic_amm_book_has_no_spread_like_its_check() {
        let mut b = healthy_book();
        b.source = BookSource::SyntheticAmm;
        let s = summarize(&b);
        assert!(matches!(
            s.checks.iter().find(|c| c.message().starts_with("Spread")),
            Some(HealthCheck::NotApplicable(_))
        ));
        let metrics = summary_metrics(&HealthThresholds::default());
        let one = sample(&s, &metrics);
        assert_eq!(value(&one, &metrics, MetricId::Spread), None);
        assert_eq!(value(&one, &metrics, MetricId::SpreadPct), None);
        assert!(value(&one, &metrics, MetricId::Mid).is_some());
    }

    #[test]
    fn spread_uses_the_best_valid_level_like_its_check() {
        let mut bids = side(1.0, -0.0001, &[400.0; 10]);
        bids[0].quantity = 0.0; // invalid top level: the best valid bid is 0.9999
        let s = summarize(&book(bids, side(1.0001, 0.0001, &[400.0; 10])));
        let metrics = summary_metrics(&HealthThresholds::default());
        let one = sample(&s, &metrics);
        let spread = value(&one, &metrics, MetricId::Spread).unwrap();
        assert!((spread - 0.0002).abs() < 1e-12, "{spread}");
        let pct = value(&one, &metrics, MetricId::SpreadPct).unwrap();
        assert!(
            s.checks
                .iter()
                .any(|c| c.message() == format!("Spread: {pct:.3}% (max 3.0%)"))
        );
    }

    #[test]
    fn top_depth_equals_what_the_check_judged_on_a_book_with_outliers() {
        // 4 levels near the peg, then 6 outliers far below it.
        let mut bids = side(0.9999, -0.0001, &[100.0; 4]);
        bids.extend(side(0.9, -0.01, &[100.0; 6]));
        let b = book(bids, side(1.0001, 0.0001, &[400.0; 10]));
        let s = summarize(&b);
        let metrics = summary_metrics(&HealthThresholds::default());
        let one = sample(&s, &metrics);
        let top10 = value(&one, &metrics, MetricId::BidTop10).unwrap();
        // Eligible levels only: the outliers are not in top-N depth.
        assert_eq!(top10, top_sum(&s.bids, 10));
        assert_ne!(top10, top_sum(&b.bids, 10));
        assert_eq!(top10, s.bid_depth);
        let judged = format!("Bid top-10 depth: {top10:.0} USDT");
        assert!(s.checks.iter().any(|c| c.message().starts_with(&judged)));
        assert_eq!(
            value(&one, &metrics, MetricId::BidTop3),
            Some(top_sum(&s.bids, 3))
        );
    }

    #[test]
    fn zero_peg_target_gives_no_deviation() {
        let metrics = summary_metrics(&HealthThresholds::default());
        let mut zero_peg = healthy_summary();
        zero_peg.peg_target = 0.0;
        let one = sample(&zero_peg, &metrics);
        assert_eq!(value(&one, &metrics, MetricId::PegDeviationPct), None);
        assert!(value(&one, &metrics, MetricId::Mid).is_some());
    }

    #[test]
    fn a_zero_priced_level_is_not_a_best_price() {
        let mut bids = side(0.9999, -0.0001, &[400.0; 3]);
        bids.insert(
            0,
            OrderBookLevel {
                price: 0.0,
                quantity: 50.0,
            },
        );
        let s = summarize(&book(bids, side(1.0001, 0.0001, &[400.0; 3])));
        let metrics = summary_metrics(&HealthThresholds::default());
        let one = sample(&s, &metrics);
        assert_eq!(value(&one, &metrics, MetricId::BestBid), Some(0.9999));
        let pct = value(&one, &metrics, MetricId::SpreadPct).unwrap();
        assert!(pct.is_finite() && pct > 0.0, "{pct}");
    }

    #[test]
    fn summary_prices_are_the_order_books_with_a_zero_quantity_top_level() {
        let mut bids = side(1.0, -0.0001, &[400.0; 10]);
        bids[0].quantity = 0.0;
        let mut asks = side(1.0001, 0.0001, &[400.0; 10]);
        asks[0].quantity = 0.0;
        let b = book(bids, asks);
        let s = summarize(&b);
        assert_eq!(s.best_bid, b.best_bid());
        assert_eq!(s.best_ask, b.best_ask());
        assert_eq!(s.mid_price, b.mid_price());
        assert_eq!(s.spread, b.spread());
        assert_eq!(s.spread_pct, b.spread_pct());
        let (bid, ask) = (s.best_bid.unwrap(), s.best_ask.unwrap());
        assert!((s.spread.unwrap() - (ask - bid)).abs() < 1e-12);
        let mid = s.mid_price.unwrap();
        assert!((s.spread_pct.unwrap() - s.spread.unwrap() / mid * 100.0).abs() < 1e-12);
    }

    // The CSV writer must keep today's header and cells byte for byte, and
    // only append the new columns.
    #[test]
    fn csv_keeps_the_old_columns_first_and_their_cell_format() {
        let metrics = summary_metrics(&HealthThresholds::default());
        let columns = csv_columns(&metrics).unwrap();
        let header = csv_header(&columns);
        assert!(
            header.starts_with(
                "timestamp,run,best_bid,best_ask,mid_price,spread,bid_depth,ask_depth,healthy,"
            ),
            "{header}"
        );
        assert!(header.ends_with("ask_top10_depth\n"), "{header}");
        let mut values = vec![None; metrics.len()];
        let at = |id| metrics.iter().position(|m| m.id == id).unwrap();
        values[at(MetricId::BestBid)] = Some(0.9999);
        values[at(MetricId::BidDepth)] = Some(1500.0);
        values[at(MetricId::Healthy)] = Some(0.0);
        let s = Sample {
            values,
            healthy: false,
            failed_checks: vec![],
        };
        let row = csv_row(&columns, &metrics, "2026-10-13T00:00:00Z", 7, &s);
        assert!(
            row.starts_with("2026-10-13T00:00:00Z,7,0.9999,-,-,-,1500,-,false,"),
            "{row}"
        );
        assert_eq!(row.matches(',').count(), header.matches(',').count());
    }
}
