//! The "market health" preset: the checks of `market summary`, as they are.
//!
//! The preset does not expand into `metric op value` rules (#47, P1). Some
//! checks are not one metric against one value (no sells below the peg, the
//! bid/ask ratio band, depth against 24h volume), and checks can warn or be
//! `n/a`. So the grid shows [`MarketSummary::checks`] one to one: the same
//! book and thresholds give the same results as `market summary`.

use super::{Cell, RuleStatus, Target};
use crate::market::types::HealthCheck;
use crate::market::{HealthThresholds, MarketSummary};

/// The thresholds a target's preset and market rules use: the target's
/// overrides over `base` (config `market.health`, or the defaults). The
/// same order as `market summary`'s flags.
pub fn thresholds(target: &Target, base: &HealthThresholds) -> HealthThresholds {
    base.with_overrides(&target.health)
}

/// One cell per check of `summary`, in check order.
pub fn cells(summary: &MarketSummary) -> Vec<Cell> {
    summary
        .checks
        .iter()
        .map(|check| {
            let (message, status) = match check {
                HealthCheck::Pass(m) => (m, RuleStatus::Pass),
                HealthCheck::Warn(m) => (m, RuleStatus::Warn(m.clone())),
                HealthCheck::Fail(m) => (m, RuleStatus::Fail(m.clone())),
                HealthCheck::NotApplicable(m) => (m, RuleStatus::NotApplicable(m.clone())),
            };
            Cell {
                label: check_name(message),
                rule: None,
                value: check_value(message),
                status,
                detail: Some(message.clone()),
            }
        })
        .collect()
}

/// A check's measured value: the first number after its name, as its
/// message gives it (`Bid depth: 4200 USDT (min 3000)` is 4200; the limit
/// comes after the value). `None` for a check without a colon or a number
/// (`No sells below peg`).
fn check_value(message: &str) -> Option<f64> {
    let (_, rest) = message.split_once(':')?;
    let start = rest.find(|c: char| c.is_ascii_digit() || c == '-')?;
    let number: String = rest[start..]
        .chars()
        .enumerate()
        .take_while(|(i, c)| c.is_ascii_digit() || *c == '.' || (*i == 0 && *c == '-'))
        .map(|(_, c)| c)
        .collect();
    number.parse().ok()
}

/// A check's name: its message up to the first `:` (`Spread: 0.02%` is
/// `Spread`), or the whole message.
fn check_name(message: &str) -> String {
    message
        .split_once(':')
        .map_or(message, |(name, _)| name)
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::types::{BookSource, OrderBook, OrderBookLevel};
    use crate::market::{HealthOverrides, order_book_from_analytics};
    use crate::watch::{TargetKind, TargetStatus, target_status};

    fn side(start: f64, step: f64, qtys: &[f64]) -> Vec<OrderBookLevel> {
        qtys.iter()
            .enumerate()
            .map(|(i, &quantity)| OrderBookLevel {
                price: start + step * i as f64,
                quantity,
            })
            .collect()
    }

    fn book(qty: f64) -> OrderBook {
        OrderBook {
            pair: "USDC/USDT".to_string(),
            source: BookSource::Exchange,
            bids: side(0.9999, -0.0001, &[qty; 10]),
            asks: side(1.0001, 0.0001, &[qty; 10]),
        }
    }

    fn summary(book: &OrderBook) -> MarketSummary {
        MarketSummary::from_order_book(book, &HealthThresholds::default(), Some(1_000_000.0))
    }

    #[test]
    fn a_check_value_is_the_first_number_after_its_name() {
        assert_eq!(check_value("Bid/Ask ratio: 1.82x"), Some(1.82));
        assert_eq!(check_value("Bid depth: 4200 USDT (min 3000)"), Some(4200.0));
        assert_eq!(check_value("Bid levels: 3 < 10 minimum"), Some(3.0));
        // The digits in the name (top-3) are not the value.
        assert_eq!(
            check_value("Bid top-3 depth: 1250 USDT (min 300)"),
            Some(1250.0)
        );
        assert_eq!(check_value("Peg deviation: -0.12%"), Some(-0.12));
        assert_eq!(check_value("No sells below peg"), None);
    }

    #[test]
    fn a_passing_check_keeps_its_value_and_text() {
        // The 0.8.0 smoke test: passing checks had no value in the CSV and
        // "-" in the detail pane.
        let cells = cells(&summary(&book(400.0)));
        let depth = cells.iter().find(|c| c.label == "Bid depth").unwrap();
        assert_eq!(depth.status, RuleStatus::Pass);
        assert!(depth.value.is_some_and(|v| v > 0.0), "{depth:?}");
        assert!(depth.detail.as_deref().unwrap().starts_with("Bid depth: "));
        assert!(cells.iter().all(|c| c.detail.is_some()));
    }

    #[test]
    fn one_cell_per_check_in_order_with_the_same_status() {
        for b in [book(400.0), book(1.0)] {
            let s = summary(&b);
            let cells = cells(&s);
            assert_eq!(cells.len(), s.checks.len());
            for (cell, check) in cells.iter().zip(&s.checks) {
                let same = matches!(
                    (&cell.status, check),
                    (RuleStatus::Pass, HealthCheck::Pass(_))
                        | (RuleStatus::Warn(_), HealthCheck::Warn(_))
                        | (RuleStatus::Fail(_), HealthCheck::Fail(_))
                        | (RuleStatus::NotApplicable(_), HealthCheck::NotApplicable(_))
                );
                assert!(same, "{cell:?} vs {check:?}");
            }
            let statuses: Vec<RuleStatus> = cells.iter().map(|c| c.status.clone()).collect();
            assert_eq!(
                target_status(&statuses) == TargetStatus::Breached,
                !s.healthy,
                "breached exactly when market summary is not healthy"
            );
        }
    }

    #[test]
    fn a_thin_book_breaches() {
        let s = summary(&book(1.0));
        assert!(!s.healthy);
        let statuses: Vec<RuleStatus> = cells(&s).into_iter().map(|c| c.status).collect();
        assert_eq!(target_status(&statuses), TargetStatus::Breached);
    }

    #[test]
    fn na_checks_on_an_amm_book_stay_na_and_do_not_breach() {
        let pair = crate::chains::DexPair {
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
        };
        let t = HealthThresholds::default();
        let b = order_book_from_analytics("ethereum", &pair, "USDC", &t);
        let s = MarketSummary::from_order_book(&b, &t, Some(pair.volume_24h));
        let cells = cells(&s);
        let na = cells
            .iter()
            .filter(|c| matches!(c.status, RuleStatus::NotApplicable(_)))
            .count();
        let checks_na = s
            .checks
            .iter()
            .filter(|c| matches!(c, HealthCheck::NotApplicable(_)))
            .count();
        assert_eq!(na, checks_na);
        let statuses: Vec<RuleStatus> = cells.into_iter().map(|c| c.status).collect();
        assert_eq!(
            target_status(&statuses) == TargetStatus::Breached,
            !s.healthy
        );
    }

    #[test]
    fn overrides_win_over_the_base_like_the_flags() {
        let base = HealthThresholds {
            max_spread_pct: 2.0,
            min_depth: 500.0,
            ..HealthThresholds::default()
        };
        let mut target = Target::new(TargetKind::Pair, "USDC");
        target.health = HealthOverrides {
            max_spread_pct: Some(0.5),
            ..HealthOverrides::default()
        };
        let t = thresholds(&target, &base);
        assert_eq!(t.max_spread_pct, 0.5, "the target's value wins");
        assert_eq!(t.min_depth, 500.0, "the base (config) value stays");
        assert_eq!(t.peg_target, HealthThresholds::default().peg_target);
    }

    #[test]
    fn check_names_are_the_part_before_the_colon() {
        assert_eq!(check_name("Spread: 0.02%"), "Spread");
        assert_eq!(check_name("No sells below peg"), "No sells below peg");
    }
}
