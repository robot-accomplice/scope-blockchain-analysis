//! Shared order book health threshold flags.
//!
//! Every command that runs order book health checks flattens [`HealthArgs`],
//! so all of them accept the same overrides. Resolution order:
//! flag > config file `market.health` > built-in default.

use clap::Args;
use scope::config::Config;
use scope::market::{HealthOverrides, HealthThresholds, health};

/// Order book health threshold overrides.
///
/// A flag left unset keeps the value from config `market.health`, or the
/// built-in default when the config file does not set it.
#[derive(Debug, Clone, Default, Args)]
#[command(next_help_heading = "Health thresholds")]
pub struct HealthArgs {
    #[arg(long = "peg", value_name = "TARGET", help = with_default("Peg target (e.g., 1.0 for USD stablecoins).", health::DEFAULT_PEG_TARGET))]
    pub peg_target: Option<f64>,

    #[arg(long, value_name = "RANGE", help = with_default("Peg range for outlier filtering (orders outside peg ± range×5 excluded). E.g., 0.001 = ±0.5% around peg.", health::DEFAULT_PEG_RANGE))]
    pub peg_range: Option<f64>,

    #[arg(long, value_name = "N", help = with_default("Minimum valid order book levels (price > 0, quantity > 0) per side.", health::DEFAULT_MIN_LEVELS))]
    pub min_levels: Option<usize>,

    #[arg(long, value_name = "USDT", help = with_default("Minimum in-band depth per side in quote terms, e.g. USDT.", health::DEFAULT_MIN_DEPTH))]
    pub min_depth: Option<f64>,

    #[arg(long, value_name = "RATIO", help = with_default("Min bid/ask depth ratio (warn if ratio below this).", health::DEFAULT_MIN_BID_ASK_RATIO))]
    pub min_bid_ask_ratio: Option<f64>,

    #[arg(long, value_name = "RATIO", help = with_default("Max bid/ask depth ratio (warn if ratio above this).", health::DEFAULT_MAX_BID_ASK_RATIO))]
    pub max_bid_ask_ratio: Option<f64>,

    #[arg(long, value_name = "PCT", help = with_default("Max spread between best bid and best ask, in percent of mid price.", health::DEFAULT_MAX_SPREAD_PCT))]
    pub max_spread_pct: Option<f64>,

    #[arg(long, value_name = "USDT", help = with_default("Min depth of the top 3 valid levels per side, in quote terms.", health::DEFAULT_MIN_TOP3_DEPTH))]
    pub min_top3_depth: Option<f64>,

    #[arg(long, value_name = "USDT", help = with_default("Min depth of the top 10 valid levels per side, in quote terms.", health::DEFAULT_MIN_TOP10_DEPTH))]
    pub min_top10_depth: Option<f64>,
}

/// Help text with the built-in default, so `--help` names the value in force.
fn with_default(desc: &str, default: impl std::fmt::Display) -> String {
    format!("{desc} [default: config `market.health`, else {default}]")
}

impl HealthArgs {
    /// Resolves thresholds: these flags over config `market.health`.
    pub fn resolve(&self, config: &Config) -> HealthThresholds {
        config.market.health.with_overrides(&HealthOverrides {
            peg_target: self.peg_target,
            peg_range: self.peg_range,
            min_levels: self.min_levels,
            min_depth: self.min_depth,
            min_bid_ask_ratio: self.min_bid_ask_ratio,
            max_bid_ask_ratio: self.max_bid_ask_ratio,
            max_spread_pct: self.max_spread_pct,
            min_top3_depth: self.min_top3_depth,
            min_top10_depth: self.min_top10_depth,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        health: HealthArgs,
    }

    fn resolve(argv: &[&str], config: &Config) -> HealthThresholds {
        let mut full = vec!["test"];
        full.extend_from_slice(argv);
        Cli::parse_from(full).health.resolve(config)
    }

    #[test]
    fn test_no_flags_no_config_uses_builtin_defaults() {
        assert_eq!(
            resolve(&[], &Config::default()),
            HealthThresholds::default()
        );
    }

    #[test]
    fn test_config_replaces_builtin_default() {
        let mut config = Config::default();
        config.market.health.min_levels = 20;
        let t = resolve(&[], &config);
        assert_eq!(t.min_levels, 20);
        // Keys the config does not change keep the built-in default.
        assert_eq!(t.min_top3_depth, HealthThresholds::default().min_top3_depth);
    }

    #[test]
    fn test_flag_wins_over_config() {
        let mut config = Config::default();
        config.market.health.max_spread_pct = 5.0;
        config.market.health.min_levels = 20;
        let t = resolve(&["--max-spread-pct", "1.5"], &config);
        assert_eq!(t.max_spread_pct, 1.5);
        assert_eq!(t.min_levels, 20);
    }

    #[test]
    fn test_peg_flag_sets_peg_target() {
        let t = resolve(&["--peg", "0.98"], &Config::default());
        assert_eq!(t.peg_target, 0.98);
    }
}
