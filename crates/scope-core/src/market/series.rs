//! Key values of a market summary: one metric table, one reading per run.
//!
//! [`summary_metrics`] defines the list once. Labels, units, thresholds and
//! CSV column names all come from it. [`sample`] reads one [`MarketSummary`]
//! into values in the same order as the table, and [`csv_columns`] gives the
//! CSV order of the same table.
//!
//! The table has no `timestamp` or `run` entry: the CSV writer adds those
//! two columns in front. This module has no presentation types.

use super::health::{HealthThresholds, MarketSummary, TOP3_LEVELS, TOP10_LEVELS, top_n_depth};
use super::types::HealthCheck;

/// Identifies one key value of a market summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricId {
    BestBid,
    BestAsk,
    Mid,
    /// `(mid - peg_target) / peg_target * 100`.
    PegDeviationPct,
    /// Absolute spread (ask - bid), as in the CSV `spread` column.
    Spread,
    /// Spread in percent of mid: `spread / mid * 100`, converted once here.
    SpreadPct,
    BidDepth,
    AskDepth,
    BidTop3,
    AskTop3,
    BidTop10,
    AskTop10,
    /// `1.0` when the summary is healthy, else `0.0`.
    Healthy,
}

impl MetricId {
    /// Every id, in table order.
    pub const ALL: [MetricId; 13] = [
        MetricId::BestBid,
        MetricId::BestAsk,
        MetricId::Mid,
        MetricId::PegDeviationPct,
        MetricId::Spread,
        MetricId::SpreadPct,
        MetricId::BidDepth,
        MetricId::AskDepth,
        MetricId::BidTop3,
        MetricId::AskTop3,
        MetricId::BidTop10,
        MetricId::AskTop10,
        MetricId::Healthy,
    ];
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
    pub healthy: bool,
    /// Messages of the checks that failed, in check order.
    pub failed_checks: Vec<String>,
}

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

/// The metric table, built from the thresholds in force.
pub fn summary_metrics(thresholds: &HealthThresholds) -> Vec<Metric> {
    use MetricId::*;
    use Threshold::{Max, Min, Target};
    let entry = |id, label, unit, threshold, csv_column| Metric {
        id,
        label,
        unit,
        threshold,
        csv_column,
    };
    vec![
        entry(BestBid, "Best bid", Unit::Price, None, "best_bid"),
        entry(BestAsk, "Best ask", Unit::Price, None, "best_ask"),
        entry(
            Mid,
            "Mid price",
            Unit::Price,
            Some(Target(thresholds.peg_target)),
            "mid_price",
        ),
        entry(
            PegDeviationPct,
            "Peg deviation %",
            Unit::Percent,
            None,
            "peg_deviation_pct",
        ),
        entry(Spread, "Spread", Unit::Price, None, "spread"),
        entry(
            SpreadPct,
            "Spread %",
            Unit::Percent,
            Some(Max(thresholds.max_spread_pct)),
            "spread_pct",
        ),
        entry(
            BidDepth,
            "Bid depth",
            Unit::Quote,
            Some(Min(thresholds.min_depth)),
            "bid_depth",
        ),
        entry(
            AskDepth,
            "Ask depth",
            Unit::Quote,
            Some(Min(thresholds.min_depth)),
            "ask_depth",
        ),
        entry(
            BidTop3,
            "Bid top-3 depth",
            Unit::Quote,
            Some(Min(thresholds.min_top3_depth)),
            "bid_top3_depth",
        ),
        entry(
            AskTop3,
            "Ask top-3 depth",
            Unit::Quote,
            Some(Min(thresholds.min_top3_depth)),
            "ask_top3_depth",
        ),
        entry(
            BidTop10,
            "Bid top-10 depth",
            Unit::Quote,
            Some(Min(thresholds.min_top10_depth)),
            "bid_top10_depth",
        ),
        entry(
            AskTop10,
            "Ask top-10 depth",
            Unit::Quote,
            Some(Min(thresholds.min_top10_depth)),
            "ask_top10_depth",
        ),
        entry(Healthy, "Health", Unit::Flag, None, "healthy"),
    ]
}

/// The table in CSV column order: today's columns first, then the rest in
/// table order. The writer adds `timestamp` and `run` before these.
pub fn csv_columns(metrics: &[Metric]) -> Vec<&Metric> {
    let legacy = LEGACY_CSV_ORDER
        .iter()
        .filter_map(|id| metrics.iter().find(|m| m.id == *id));
    let rest = metrics.iter().filter(|m| !LEGACY_CSV_ORDER.contains(&m.id));
    legacy.chain(rest).collect()
}

/// Reads one summary into values, in the order of `metrics`.
pub fn sample(summary: &MarketSummary, metrics: &[Metric]) -> Sample {
    Sample {
        values: metrics.iter().map(|m| value_of(m.id, summary)).collect(),
        healthy: summary.healthy,
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

fn value_of(id: MetricId, s: &MarketSummary) -> Option<f64> {
    match id {
        MetricId::BestBid => s.best_bid,
        MetricId::BestAsk => s.best_ask,
        MetricId::Mid => s.mid_price,
        MetricId::PegDeviationPct => s
            .mid_price
            .filter(|_| s.peg_target > 0.0)
            .map(|mid| (mid - s.peg_target) / s.peg_target * 100.0),
        MetricId::Spread => s.spread,
        MetricId::SpreadPct => s
            .spread
            .zip(s.mid_price.filter(|mid| *mid > 0.0))
            .map(|(spread, mid)| spread / mid * 100.0),
        MetricId::BidDepth => Some(s.bid_depth),
        MetricId::AskDepth => Some(s.ask_depth),
        // The summary's levels are the book's levels cut at the peg range,
        // best first, so the top-N sum equals the one the checks use.
        MetricId::BidTop3 => Some(top_n_depth(&s.bids, TOP3_LEVELS)),
        MetricId::AskTop3 => Some(top_n_depth(&s.asks, TOP3_LEVELS)),
        MetricId::BidTop10 => Some(top_n_depth(&s.bids, TOP10_LEVELS)),
        MetricId::AskTop10 => Some(top_n_depth(&s.asks, TOP10_LEVELS)),
        MetricId::Healthy => Some(if s.healthy { 1.0 } else { 0.0 }),
    }
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

    fn healthy_summary() -> MarketSummary {
        let b = book(
            side(0.9999, -0.0001, &[400.0; 10]),
            side(1.0001, 0.0001, &[400.0; 10]),
        );
        MarketSummary::from_order_book(&b, &HealthThresholds::default(), None)
    }

    fn metric(metrics: &[Metric], id: MetricId) -> &Metric {
        metrics.iter().find(|m| m.id == id).expect("metric present")
    }

    fn value(sample: &Sample, metrics: &[Metric], id: MetricId) -> Option<f64> {
        let at = metrics.iter().position(|m| m.id == id).unwrap();
        sample.values[at]
    }

    #[test]
    fn every_metric_id_is_in_the_table_once() {
        let metrics = summary_metrics(&HealthThresholds::default());
        for id in MetricId::ALL {
            assert_eq!(metrics.iter().filter(|m| m.id == id).count(), 1, "{id:?}");
        }
        assert_eq!(metrics.len(), MetricId::ALL.len());
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
        // Spread % converts the absolute spread once: spread / mid * 100.
        let mut wide = s.clone();
        wide.mid_price = Some(2.0);
        wide.spread = Some(0.04);
        let pct = value(&sample(&wide, &metrics), &metrics, MetricId::SpreadPct);
        assert!((pct.unwrap() - 2.0).abs() < 1e-9);
        // Top-3 and top-10 depth reuse the health module sums: 3 and 10 levels of ~400.
        let top3 = value(&one, &metrics, MetricId::BidTop3).unwrap();
        let top10 = value(&one, &metrics, MetricId::AskTop10).unwrap();
        assert!((1199.0..1201.0).contains(&top3), "{top3}");
        assert!((3999.0..4005.0).contains(&top10), "{top10}");
        assert_eq!(value(&one, &metrics, MetricId::Healthy), Some(1.0));
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
        let columns: Vec<&str> = csv_columns(&metrics).iter().map(|m| m.csv_column).collect();
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
}
