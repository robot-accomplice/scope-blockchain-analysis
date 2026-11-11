//! # Watchlists
//!
//! A watchlist is a saved set of targets (tokens, pairs, addresses), each
//! with rules on the metrics of [`crate::metrics`], plus the interval, the
//! duration and the output files of a run. It is the "saved metric set" of
//! #47 and the output of the TUI's monitor wizard.
//!
//! This module is pure: types, rule evaluation ([`rule`]), the "market
//! health" preset ([`preset`]), snapshot comparison ([`diff`](mod@diff)),
//! the memory between ticks ([`history`]) and the file format
//! ([`file`](mod@file)). The fetchers that read the
//! metrics live in scope-cli.

pub mod diff;
pub mod file;
pub mod history;
pub mod preset;
pub mod rule;
pub mod time;

pub use diff::{Event, diff};
pub use history::{Basis, Entry, History, MarketKey, Reading, Readings, Restored};
pub use rule::{Op, Rule, RuleStatus, TargetStatus, target_status};

use crate::market::HealthOverrides;
use crate::metrics::{MetricGroup, MetricId};
use std::path::PathBuf;
use std::time::Duration;

/// What a target is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    /// A token: a symbol or a contract address.
    Token,
    /// A market pair on a venue.
    Pair,
    /// A wallet or contract address.
    Address,
}

impl TargetKind {
    /// Whether metrics of `group` apply to this kind of target.
    pub fn accepts(self, group: MetricGroup) -> bool {
        matches!(
            (self, group),
            (TargetKind::Token | TargetKind::Pair, MetricGroup::Market)
                | (TargetKind::Token, MetricGroup::Token)
                | (TargetKind::Address, MetricGroup::Address)
        )
    }

    /// The metrics that apply to this kind, in catalog order.
    pub fn metrics(self) -> Vec<MetricId> {
        MetricId::ALL
            .iter()
            .copied()
            .filter(|id| self.accepts(id.group()))
            .collect()
    }

    /// The kind's name in a watchlist file.
    pub fn name(self) -> &'static str {
        match self {
            TargetKind::Token => "token",
            TargetKind::Pair => "pair",
            TargetKind::Address => "address",
        }
    }
}

/// A named set of rules a target can use instead of writing them out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// The checks of `market summary`, with the same thresholds (P1).
    MarketHealth,
}

impl Preset {
    /// Every preset.
    pub const ALL: &'static [Preset] = &[Preset::MarketHealth];

    /// The preset's name in a watchlist file and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Preset::MarketHealth => "market health",
        }
    }

    /// The preset with this name (`market health` or `market-health`).
    pub fn from_name(name: &str) -> Option<Preset> {
        let name = name.trim().replace(['-', '_'], " ");
        Preset::ALL
            .iter()
            .copied()
            .find(|p| p.name().eq_ignore_ascii_case(&name))
    }
}

/// One thing a watchlist watches.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub kind: TargetKind,
    /// A symbol, a contract address or a wallet address.
    pub id: String,
    /// The chain; inferred when absent, as the CLI does.
    pub chain: Option<String>,
    /// The venues, for market metrics, in the user's order. Empty: none.
    pub venues: Vec<String>,
    /// A preset, evaluated before the rules.
    pub preset: Option<Preset>,
    /// Health threshold overrides for the preset and the market metrics.
    pub health: HealthOverrides,
    pub rules: Vec<Rule>,
}

impl Target {
    /// A target with no venue, preset or rules.
    pub fn new(kind: TargetKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            chain: None,
            venues: Vec::new(),
            preset: None,
            health: HealthOverrides::default(),
            rules: Vec::new(),
        }
    }

    /// The name of the whole target: `USDC@binance`, `USDC@binance,okx`,
    /// or the id.
    pub fn label(&self) -> String {
        if self.venues.is_empty() {
            self.id.clone()
        } else {
            format!("{}@{}", self.id, self.venues.join(","))
        }
    }

    /// The label of one row of the target: `USDC@binance` on a venue, the
    /// id without one.
    pub fn row_label(&self, venue: Option<&str>) -> String {
        match venue {
            Some(venue) => format!("{}@{}", self.id, venue),
            None => self.id.clone(),
        }
    }

    /// The rows the target gives, by venue (`None`: the id row). One venue
    /// or none: one row with every cell. Two or more: one market row per
    /// venue, then an id row when token or address rules need one (design
    /// 2026-11-04, section 2.3). With no market metric, the venues read
    /// nothing, so the target is its id row alone.
    pub fn rows(&self) -> Vec<Option<&str>> {
        match self.venues.as_slice() {
            [] => vec![None],
            [one] => vec![Some(one.as_str())],
            _ if !self.needs_market() => vec![None],
            many => {
                let mut rows: Vec<Option<&str>> = many.iter().map(|v| Some(v.as_str())).collect();
                if self.needs(MetricGroup::Token) || self.needs(MetricGroup::Address) {
                    rows.push(None);
                }
                rows
            }
        }
    }

    /// Whether a rule compares a tick with the one before, so the target's
    /// markets keep a memory ([`History`]).
    pub fn keeps_history(&self) -> bool {
        self.rules.iter().any(|r| r.metric.since_last_tick())
    }

    /// Whether the target reads a market summary (preset or a market rule).
    pub fn needs_market(&self) -> bool {
        self.preset == Some(Preset::MarketHealth)
            || self
                .rules
                .iter()
                .any(|r| r.metric.group() == MetricGroup::Market)
    }

    /// Whether any rule reads a metric of `group`.
    pub fn needs(&self, group: MetricGroup) -> bool {
        self.rules.iter().any(|r| r.metric.group() == group)
    }
}

/// The [`MarketKey`] of every row of every target, in list order: for each
/// target, one item for each of [`Target::rows`] (`None` for a row with no
/// venue). The ordinal counts the rows before it in the list with the same
/// id, chain and venue, so two such targets have two keys. The ticks and
/// the saved state both take their keys from here.
pub fn market_keys<'a>(
    targets: impl IntoIterator<Item = &'a Target>,
) -> Vec<Vec<Option<MarketKey>>> {
    let mut seen: std::collections::HashMap<(String, Option<String>, String), u32> =
        std::collections::HashMap::new();
    targets
        .into_iter()
        .map(|t| {
            t.rows()
                .into_iter()
                .map(|venue| {
                    let venue = venue?;
                    let n = seen
                        .entry((t.id.clone(), t.chain.clone(), venue.to_string()))
                        .or_insert(0);
                    *n += 1;
                    Some(MarketKey {
                        id: t.id.clone(),
                        chain: t.chain.clone(),
                        venue: venue.to_string(),
                        ordinal: *n,
                    })
                })
                .collect()
        })
        .collect()
}

/// Where a run writes its record.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outputs {
    /// One row per rule per tick.
    pub csv: Option<PathBuf>,
    /// Written at the end of the run.
    pub report: Option<PathBuf>,
}

/// A saved watchlist.
#[derive(Debug, Clone, PartialEq)]
pub struct Watchlist {
    pub name: String,
    /// Time from one tick's start to the next.
    pub interval: Duration,
    /// How long a run lasts; `None` runs until the user stops it.
    pub duration: Option<Duration>,
    pub outputs: Outputs,
    pub targets: Vec<Target>,
}

impl Watchlist {
    /// An empty watchlist with the default interval and duration (Q7).
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            interval: Duration::from_secs(time::DEFAULT_INTERVAL_SECS),
            duration: Some(Duration::from_secs(time::DEFAULT_DURATION_SECS)),
            outputs: Outputs::default(),
            targets: Vec::new(),
        }
    }

    /// Fetches per tick: one per source each target reads.
    pub fn fetches_per_tick(&self) -> usize {
        self.targets
            .iter()
            .map(|t| {
                usize::from(t.needs_market()) * t.venues.len().max(1)
                    + usize::from(t.needs(MetricGroup::Token))
                    + usize::from(t.needs(MetricGroup::Address))
            })
            .sum()
    }

    /// Ticks in a run with a duration: one per interval and a last one at
    /// the end, the same rule as the repeat runner. `None` without one.
    pub fn ticks(&self) -> Option<u64> {
        let every = self.interval.as_millis().max(1);
        let total = self.duration?.as_millis().div_ceil(every).saturating_add(1);
        Some(u64::try_from(total).unwrap_or(u64::MAX))
    }
}

/// One cell of a target's row: a preset check or a rule.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// The check's name (preset) or the rule's text.
    pub label: String,
    /// The rule, for a rule cell.
    pub rule: Option<Rule>,
    /// The metric's value, when it has one.
    pub value: Option<f64>,
    pub status: RuleStatus,
    /// The full text of a preset check (`Spread: 0.02% (max 3%)`), for
    /// every status: a passing check keeps its numbers too.
    pub detail: Option<String>,
}

/// One target's state after a tick.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetSnapshot {
    /// [`Target::row_label`].
    pub target: String,
    /// The venue of a market row; `None` for a row without one.
    pub venue: Option<String>,
    pub cells: Vec<Cell>,
    pub status: TargetStatus,
}

impl TargetSnapshot {
    /// A row from its cells; the status comes from the cells.
    pub fn new(target: impl Into<String>, cells: Vec<Cell>) -> Self {
        let statuses: Vec<RuleStatus> = cells.iter().map(|c| c.status.clone()).collect();
        Self {
            target: target.into(),
            venue: None,
            status: target_status(&statuses),
            cells,
        }
    }

    /// The row with the venue it reads.
    pub fn on_venue(mut self, venue: Option<&str>) -> Self {
        self.venue = venue.map(str::to_string);
        self
    }
}

/// Every target's state after one tick, in watchlist order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub targets: Vec<TargetSnapshot>,
}

impl Snapshot {
    /// How many targets have a breach.
    pub fn breaches(&self) -> usize {
        self.targets
            .iter()
            .filter(|t| t.status == TargetStatus::Breached)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_accept_their_metric_groups() {
        use MetricGroup::*;
        assert!(TargetKind::Pair.accepts(Market));
        assert!(!TargetKind::Pair.accepts(Token));
        assert!(TargetKind::Token.accepts(Market));
        assert!(TargetKind::Token.accepts(Token));
        assert!(!TargetKind::Token.accepts(Address));
        assert!(TargetKind::Address.accepts(Address));
        assert!(!TargetKind::Address.accepts(Market));
        assert!(TargetKind::Address.metrics().contains(&MetricId::RiskScore));
        assert!(!TargetKind::Pair.metrics().contains(&MetricId::Holders));
    }

    #[test]
    fn preset_names_parse_in_several_spellings() {
        for s in [
            "market health",
            "Market Health",
            "market-health",
            "market_health",
        ] {
            assert_eq!(Preset::from_name(s), Some(Preset::MarketHealth), "{s}");
        }
        assert_eq!(Preset::from_name("peg"), None);
    }

    #[test]
    fn label_names_the_venues() {
        let mut t = Target::new(TargetKind::Pair, "USDC");
        assert_eq!(t.label(), "USDC");
        t.venues = vec!["binance".into()];
        assert_eq!(t.label(), "USDC@binance");
        t.venues.push("okx".into());
        assert_eq!(t.label(), "USDC@binance,okx");
        assert_eq!(t.row_label(Some("okx")), "USDC@okx");
        assert_eq!(t.row_label(None), "USDC");
    }

    // One venue keeps today's single row, so old CSV files keep their
    // labels; with several, venue-free cells get their own row so that a
    // holders breach is not reported as a breach on one venue.
    #[test]
    fn rows_split_by_venue_and_keep_venue_free_cells_apart() {
        let mut t = Target::new(TargetKind::Token, "USDN");
        assert_eq!(t.rows(), vec![None]);
        t.venues = vec!["weex".into()];
        t.rules.push(Rule::parse("holders >= 10").unwrap());
        assert_eq!(t.rows(), vec![Some("weex")]);
        t.venues.push("binance".into());
        t.preset = Some(Preset::MarketHealth);
        assert_eq!(t.rows(), vec![Some("weex"), Some("binance"), None]);
        t.rules.clear();
        assert_eq!(t.rows(), vec![Some("weex"), Some("binance")]);
        // No market metric: the venues have nothing to read.
        t.preset = None;
        t.rules.push(Rule::parse("holders >= 10").unwrap());
        assert_eq!(t.rows(), vec![None]);
    }

    #[test]
    fn fetches_count_each_source_once_per_target() {
        let mut w = Watchlist::new("w");
        let mut pair = Target::new(TargetKind::Pair, "USDC");
        pair.preset = Some(Preset::MarketHealth);
        pair.rules.push(Rule::parse("spread_pct < 0.5").unwrap());
        let mut token = Target::new(TargetKind::Token, "PEPE");
        token.rules.push(Rule::parse("holders >= 10").unwrap());
        token.rules.push(Rule::parse("price_usd > 0").unwrap());
        w.targets = vec![pair, token];
        assert_eq!(w.fetches_per_tick(), 2);
        // One market read per venue.
        w.targets[0].venues = vec!["binance".into(), "okx".into()];
        assert_eq!(w.fetches_per_tick(), 3);
    }

    #[test]
    fn defaults_are_market_summary_defaults_and_ticks_follow_the_runner() {
        let w = Watchlist::new("w");
        assert_eq!(w.interval, Duration::from_secs(60));
        assert_eq!(w.duration, Some(Duration::from_secs(3600)));
        assert_eq!(w.ticks(), Some(61));
        let w = Watchlist {
            duration: None,
            ..Watchlist::new("w")
        };
        assert_eq!(w.ticks(), None);
    }

    #[test]
    fn snapshot_status_comes_from_the_cells() {
        let cell = |status| Cell {
            label: "x".into(),
            rule: None,
            value: None,
            status,
            detail: None,
        };
        let row = TargetSnapshot::new(
            "A",
            vec![cell(RuleStatus::Pass), cell(RuleStatus::Fail("f".into()))],
        );
        assert_eq!(row.status, TargetStatus::Breached);
        let s = Snapshot {
            targets: vec![row, TargetSnapshot::new("B", vec![cell(RuleStatus::Pass)])],
        };
        assert_eq!(s.breaches(), 1);
    }

    fn pair(id: &str, chain: Option<&str>, venues: &[&str]) -> Target {
        let mut t = Target::new(TargetKind::Pair, id);
        t.chain = chain.map(str::to_string);
        t.venues = venues.iter().map(|v| v.to_string()).collect();
        t.rules.push(Rule::parse("largest_trade < 1").unwrap());
        t
    }

    fn texts(keys: &[Vec<Option<MarketKey>>]) -> Vec<Vec<String>> {
        keys.iter()
            .map(|t| {
                t.iter()
                    .map(|k| k.as_ref().map_or("-".into(), ToString::to_string))
                    .collect()
            })
            .collect()
    }

    // Design 2026-11-11, section 3.2.7: the key decides which entries a
    // changed list still uses. The ordinal tells two targets of one market
    // apart; the chain and the id are in the key; a row with no venue has
    // none.
    #[test]
    fn a_market_key_has_the_ordinal_the_chain_and_the_venue() {
        let targets = [
            pair("USDC", None, &["weex"]),
            pair("USDC", None, &["weex", "binance"]),
            pair("USDC", Some("base"), &["weex"]),
            pair("DAI", None, &["weex"]),
            pair("USDC", None, &["weex"]),
        ];
        assert_eq!(
            texts(&market_keys(&targets)),
            [
                vec!["USDC@weex"],
                vec!["USDC@weex#2", "USDC@binance"],
                vec!["USDC@weex[base]"],
                vec!["DAI@weex"],
                vec!["USDC@weex#3"],
            ]
        );
        // No venue, no market, no key.
        let mut token = Target::new(TargetKind::Token, "USDN");
        token.rules.push(Rule::parse("holders >= 1").unwrap());
        assert_eq!(texts(&market_keys([&token])), [vec!["-"]]);
        assert!(!token.keeps_history());
        assert!(targets[0].keeps_history());
    }
}
