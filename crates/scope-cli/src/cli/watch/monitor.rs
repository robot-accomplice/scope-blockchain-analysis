//! `scope monitor <args>` as a watchlist: the arguments fill the monitor
//! wizard (#47, design section 5.3).

use crate::cli::monitor::MonitorArgs;
use scope::config::Config;
use scope::metrics::MetricId;
use scope::watch::time::{is_until_cancelled, parse_duration};
use scope::watch::{Op, Preset, Rule, Target, TargetKind, Watchlist};
use std::time::Duration;

/// The issue that brings back the whale and volume-spike alerts (Q2).
pub const ALERTS_ISSUE: &str = "#122";

/// The note for a dashboard-only flag the wizard does not use.
fn ignored(flag: &str) -> String {
    format!(
        "{flag} has no effect: the watchlist grid has no layouts. Use `--dashboard` for the old dashboard (deprecated)."
    )
}

/// The watchlist the monitor arguments describe, with the notes the user
/// should read (flags that do nothing, alerts that are dropped).
///
/// # Errors
///
/// An interval or duration that does not parse, or an unknown preset.
pub fn watchlist_from_args(
    args: &MonitorArgs,
    config: &Config,
) -> Result<(Watchlist, Vec<String>), String> {
    let w = &args.watch;
    let mut notes = Vec::new();
    let mut list = Watchlist::new(
        w.save
            .clone()
            .unwrap_or_else(|| format!("monitor-{}", args.token.to_lowercase())),
    );
    if let Some(every) = &w.every {
        list.interval =
            Duration::from_secs(parse_duration(every).map_err(|e| format!("--every: {e}"))?);
    } else if let Some(secs) = args.refresh {
        if secs == 0 {
            return Err("--refresh must be at least 1 second".to_string());
        }
        list.interval = Duration::from_secs(secs);
    }
    if let Some(d) = &w.duration {
        list.duration = if is_until_cancelled(d) {
            None
        } else {
            Some(Duration::from_secs(
                parse_duration(d).map_err(|e| format!("--duration: {e}"))?,
            ))
        };
    }
    list.outputs.csv = w.csv.clone().or_else(|| args.export.clone());
    list.outputs.report = w.report.clone();

    let preset = match w.preset.as_deref() {
        None => args.venue.is_some().then_some(Preset::MarketHealth),
        Some(p) if p.eq_ignore_ascii_case("none") => None,
        Some(p) => Some(
            Preset::from_name(p)
                .ok_or_else(|| format!("unknown preset `{p}`; use market-health or none"))?,
        ),
    };
    let mut target = match &args.venue {
        Some(venue) => {
            let mut t = Target::new(
                TargetKind::Pair,
                args.pair.clone().unwrap_or_else(|| args.token.clone()),
            );
            t.venue = Some(venue.clone());
            t
        }
        None => {
            if preset.is_some() {
                return Err("the market health checks need a venue (`--venue binance`)".to_string());
            }
            let mut t = Target::new(
                TargetKind::Token,
                args.token_contract
                    .clone()
                    .unwrap_or_else(|| args.token.clone()),
            );
            // The price and volume charts of the old dashboard.
            t.rules = vec![
                Rule {
                    metric: MetricId::PriceUsd,
                    op: Op::Gt,
                    value: 0.0,
                },
                Rule {
                    metric: MetricId::Volume24h,
                    op: Op::Ge,
                    value: 0.0,
                },
            ];
            t
        }
    };
    if args.chain != scope::chains::DEFAULT_CHAIN {
        target.chain = Some(args.chain.clone());
    }
    target.preset = preset;
    target.health = w.health.overrides();

    // The monitor's alerts: price bounds become rules on the price the
    // target has; whale and volume-spike alerts have no metric yet.
    let alerts = &config.monitor.alerts;
    let price = match target.kind {
        TargetKind::Pair => MetricId::Mid,
        _ => MetricId::PriceUsd,
    };
    if let Some(min) = alerts.price_min {
        target.rules.push(Rule {
            metric: price,
            op: Op::Ge,
            value: min,
        });
    }
    if let Some(max) = alerts.price_max {
        target.rules.push(Rule {
            metric: price,
            op: Op::Le,
            value: max,
        });
    }
    if alerts.whale_min_usd.is_some() || alerts.volume_spike_threshold_pct.is_some() {
        notes.push(format!(
            "The whale and volume-spike alerts of config `monitor.alerts` are not checked: they have no watchlist metric yet ({ALERTS_ISSUE})."
        ));
    }
    for (set, flag) in [
        (args.layout.is_some(), "--layout"),
        (args.scale.is_some(), "--scale"),
        (args.color_scheme.is_some(), "--color-scheme"),
    ] {
        if set {
            notes.push(ignored(flag));
        }
    }
    if args.export.is_some() && w.csv.is_none() {
        notes.push("--export is now --csv: it writes one row per check per run.".to_string());
    }
    list.targets.push(target);
    Ok((list, notes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;

    fn from(argv: &str) -> Result<(Watchlist, Vec<String>), String> {
        from_with(argv, &Config::default())
    }

    fn from_with(argv: &str, config: &Config) -> Result<(Watchlist, Vec<String>), String> {
        let words: Vec<&str> = std::iter::once("scope")
            .chain(argv.split_whitespace())
            .collect();
        let cli = Cli::try_parse_from(words).map_err(|e| e.to_string())?;
        let crate::cli::Commands::Monitor(args) = cli.command else {
            panic!("not monitor")
        };
        watchlist_from_args(&args, config)
    }

    #[test]
    fn the_trigger_case_is_usdn_on_weex_with_the_health_checks_every_30s_for_1h() {
        let (w, notes) = from("monitor USDN --venue weex --every 30s --duration 1h").unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(w.interval, Duration::from_secs(30));
        assert_eq!(w.duration, Some(Duration::from_secs(3600)));
        let t = &w.targets[0];
        assert_eq!((t.kind, t.id.as_str()), (TargetKind::Pair, "USDN"));
        assert_eq!(t.venue.as_deref(), Some("weex"));
        assert_eq!(t.preset, Some(Preset::MarketHealth));
    }

    #[test]
    fn refresh_and_every_give_the_same_interval_and_defaults_are_60s_1h() {
        let a = from("monitor USDC --venue binance --refresh 30").unwrap().0;
        let b = from("monitor USDC --venue binance --every 30s").unwrap().0;
        assert_eq!(a.interval, b.interval);
        let d = from("monitor USDC --venue binance").unwrap().0;
        assert_eq!(d.interval, Duration::from_secs(60));
        assert_eq!(d.duration, Some(Duration::from_secs(3600)));
        assert!(
            from("monitor USDC --refresh 5 --every 5s").is_err(),
            "they conflict"
        );
        let f = from("monitor USDC --venue binance --duration forever")
            .unwrap()
            .0;
        assert_eq!(f.duration, None);
    }

    #[test]
    fn pair_csv_report_health_preset_save_map() {
        let (w, notes) = from(
            "monitor DAI --venue binance --pair DAI_USDT --csv a.csv --report a.md --max-spread-pct 0.5 --save stables",
        )
        .unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(w.name, "stables");
        assert_eq!(
            w.outputs.csv.as_deref(),
            Some(std::path::Path::new("a.csv"))
        );
        assert_eq!(
            w.outputs.report.as_deref(),
            Some(std::path::Path::new("a.md"))
        );
        let t = &w.targets[0];
        assert_eq!(t.id, "DAI_USDT");
        assert_eq!(t.health.max_spread_pct, Some(0.5));
        let none = from("monitor DAI --venue binance --preset none").unwrap().0;
        assert_eq!(none.targets[0].preset, None);
        assert!(
            from("monitor DAI --venue binance --preset peg")
                .unwrap_err()
                .contains("unknown preset")
        );
    }

    #[test]
    fn without_a_venue_the_token_gets_price_and_volume_and_no_preset() {
        let (w, _) = from("monitor PEPE --chain solana").unwrap();
        let t = &w.targets[0];
        assert_eq!(t.kind, TargetKind::Token);
        assert_eq!(t.chain.as_deref(), Some("solana"));
        let names: Vec<String> = t.rules.iter().map(ToString::to_string).collect();
        assert_eq!(names, ["price_usd > 0", "volume_24h >= 0"]);
        assert!(
            from("monitor PEPE --preset market-health")
                .unwrap_err()
                .contains("need a venue")
        );
    }

    #[test]
    fn dashboard_flags_give_one_note_each_and_export_is_the_csv() {
        let (w, notes) = from(
            "monitor USDC --venue binance --layout chart-focus --scale log --color-scheme monochrome --export x.csv",
        )
        .unwrap();
        assert_eq!(notes.len(), 4, "{notes:?}");
        assert!(notes[0].starts_with("--layout has no effect"));
        assert!(notes[3].contains("--export is now --csv"));
        assert_eq!(
            w.outputs.csv.as_deref(),
            Some(std::path::Path::new("x.csv"))
        );
    }

    #[test]
    fn price_alerts_become_rules_and_whale_alerts_name_the_issue() {
        let mut config = Config::default();
        config.monitor.alerts.price_min = Some(0.99);
        config.monitor.alerts.price_max = Some(1.01);
        config.monitor.alerts.whale_min_usd = Some(100_000.0);
        let (w, notes) = from_with("monitor USDC --venue binance", &config).unwrap();
        let rules: Vec<String> = w.targets[0].rules.iter().map(ToString::to_string).collect();
        assert_eq!(rules, ["mid_price >= 0.99", "mid_price <= 1.01"]);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains(ALERTS_ISSUE), "{notes:?}");
        let (w, _) = from_with("monitor PEPE", &config).unwrap();
        let rules: Vec<String> = w.targets[0].rules.iter().map(ToString::to_string).collect();
        assert!(
            rules.contains(&"price_usd >= 0.99".to_string()),
            "{rules:?}"
        );
    }
}
