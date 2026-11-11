//! The peg deviation of a market row, for the venue chart and for the watch
//! CSV. One reading, one rule: the chart's legend glyph and the CSV status
//! of the same row come from [`status`], so they cannot disagree.

use scope::market::MarketSummary;
use scope::market::health::{DeviationGap, PegBand, PegBands};
use scope::watch::RuleStatus;

/// The metric name of the deviation row in the watch CSV.
pub const METRIC: &str = "peg_deviation_bps";

/// The `source` of the deviation row in the watch CSV. A row of this source
/// is a reading for the chart, not a rule or a preset check: it is never a
/// breach, whatever its status says.
pub const SOURCE: &str = "chart";

/// The status words of the deviation rows and the legend glyphs.
const SLIGHT_WORDS: &str = "slight deviation";
const OFF_WORDS: &str = "off the peg";
const ON_WORDS: &str = "on the peg";
const NO_READING: &str = "no reading";

/// A venue's status on the deviation chart: the peg band its deviation
/// falls in (the same rule as the price colours), or why it has none.
pub fn status(bands: Option<&PegBands>, bps: Option<f64>, gap: Option<DeviationGap>) -> RuleStatus {
    match bps.and_then(|bps| bands?.classify_bps(bps)) {
        Some(PegBand::OnPeg) => RuleStatus::Pass,
        Some(PegBand::Slight) => RuleStatus::Warn(SLIGHT_WORDS.to_string()),
        Some(PegBand::Off) => RuleStatus::Fail(OFF_WORDS.to_string()),
        None => RuleStatus::NotApplicable(gap.map_or(NO_READING, DeviationGap::reason).to_string()),
    }
}

/// One market row's deviation at one tick, as the CSV records it.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub bps: Option<f64>,
    pub status: RuleStatus,
    /// The band and the verdict in words, or why there is no value.
    pub detail: String,
}

/// The reading of a row from its summary; `None` is a failed read.
pub fn reading(summary: Option<&MarketSummary>) -> Reading {
    let Some(summary) = summary else {
        let gap = DeviationGap::NoPrice;
        return Reading {
            bps: None,
            status: status(None, None, Some(gap)),
            detail: gap.reason().to_string(),
        };
    };
    let bps = summary.mid_peg_deviation_bps();
    let bands = summary.peg_bands();
    let status = status(Some(&bands), bps, summary.peg_deviation_gap());
    let detail = match (bps, bands.range_bps()) {
        (Some(_), Some(half)) => format!(
            "{} (band ±{half:.1} bps)",
            status.message().unwrap_or(ON_WORDS)
        ),
        _ => status.message().unwrap_or(NO_READING).to_string(),
    };
    Reading {
        bps,
        status,
        detail,
    }
}
