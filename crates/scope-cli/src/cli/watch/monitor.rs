//! `scope monitor <args>` as a watchlist: the arguments fill the monitor
//! wizard (#47, design section 5.3).

use crate::cli::monitor::MonitorArgs;
use crate::cli::watch_args::{KindArg, TargetArgs};
use scope::config::Config;
use scope::metrics::MetricId;
use scope::watch::time::{is_until_cancelled, parse_duration};
use scope::watch::{Op, Preset, Rule, Target, TargetKind, Watchlist};
use std::time::Duration;

/// The interval of an `--every` value (`30s`, `5m`). `scope monitor` and
/// `scope watch` both read the flag through this function.
///
/// # Errors
///
/// A value that does not parse. The text names the flag.
pub fn parse_interval(every: &str) -> Result<Duration, String> {
    parse_duration(every)
        .map(Duration::from_secs)
        .map_err(|e| format!("--every: {e}"))
}

/// The length of a run from a `--duration` value: `None` for `forever`.
/// Read by `scope monitor` and `scope watch`, like [`parse_interval`].
///
/// # Errors
///
/// A value that does not parse. The text names the flag.
pub fn parse_run_duration(duration: &str) -> Result<Option<Duration>, String> {
    if is_until_cancelled(duration) {
        return Ok(None);
    }
    parse_duration(duration)
        .map(|secs| Some(Duration::from_secs(secs)))
        .map_err(|e| format!("--duration: {e}"))
}

/// The target that the arguments describe, with what the user should read
/// about it. The one definition of what `USDC --venue weex` watches: the
/// monitor wizard and `scope watch` both build their target here.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltTarget {
    pub target: Target,
    /// Flags that do nothing, alerts that are dropped.
    pub notes: Vec<String>,
    /// The rules that came from config `monitor.alerts`, not from the user.
    pub alert_rules: Vec<String>,
}

/// Builds the target of `token` (the positional argument) and the target
/// flags `t`. The rules are:
///
/// - With `--venue`: a `Pair` target. Its id is `--pair`, else the token.
///   It gets the `market health` preset unless `--preset none`.
/// - Without `--venue`: a `Token` target. Its id is `--token`, else the
///   token. It gets `price_usd > 0` and `volume_24h >= 0`.
/// - `--kind` picks the kind. `--kind address` is a wallet: it has no
///   default rules, so it needs `--rule`.
/// - `--rule` values are added after the preset and the default rules.
/// - The alerts of config `monitor.alerts` follow, as rules.
///
/// # Errors
///
/// An unknown preset, a `--rule` that does not parse, a flag that needs a
/// venue (or must not have one), or an address with no rule.
pub fn target_from_args(
    token: &str,
    t: &TargetArgs,
    config: &Config,
) -> Result<BuiltTarget, String> {
    let mut notes = Vec::new();
    let kind = t.kind.map_or_else(
        || {
            if t.venue.is_empty() {
                TargetKind::Token
            } else {
                TargetKind::Pair
            }
        },
        |k| match k {
            KindArg::Pair => TargetKind::Pair,
            KindArg::Token => TargetKind::Token,
            KindArg::Address => TargetKind::Address,
        },
    );
    let preset = match t.preset.as_deref() {
        None => (kind == TargetKind::Pair).then_some(Preset::MarketHealth),
        Some(p) if p.eq_ignore_ascii_case("none") => None,
        Some(p) => Some(
            Preset::from_name(p)
                .ok_or_else(|| format!("unknown preset `{p}`; use market-health or none"))?,
        ),
    };
    let mut rules = Vec::new();
    for text in &t.rule {
        rules.push(Rule::parse(text).map_err(|e| format!("--rule `{text}`: {e}"))?);
    }
    let mut target = match kind {
        TargetKind::Pair => {
            if t.venue.is_empty() {
                return Err("--kind pair needs a venue (`--venue binance`)".to_string());
            }
            let mut target = Target::new(
                TargetKind::Pair,
                t.pair.clone().unwrap_or_else(|| token.to_string()),
            );
            target.venues = t.venue.clone();
            target
        }
        TargetKind::Token => {
            if t.venue.is_empty() && preset.is_some() {
                return Err("the market health checks need a venue (`--venue binance`)".to_string());
            }
            if t.pair.is_some() && t.venue.is_empty() {
                return Err("--pair needs a venue (`--venue binance`)".to_string());
            }
            let mut target = Target::new(
                TargetKind::Token,
                t.token_contract
                    .clone()
                    .unwrap_or_else(|| token.to_string()),
            );
            target.venues = t.venue.clone();
            // The price and volume charts of the old dashboard.
            target.rules = vec![
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
            target
        }
        TargetKind::Address => {
            if !t.venue.is_empty() || t.pair.is_some() || preset.is_some() {
                return Err(
                    "--kind address has no venue, pair or checks: give it `--rule`".to_string(),
                );
            }
            if rules.is_empty() {
                return Err(
                    "--kind address needs a rule, for example --rule \"native_balance > 1\""
                        .to_string(),
                );
            }
            Target::new(
                TargetKind::Address,
                t.token_contract
                    .clone()
                    .unwrap_or_else(|| token.to_string()),
            )
        }
    };
    if let Some(chain) = t.chain.as_deref()
        && chain != scope::chains::DEFAULT_CHAIN
    {
        target.chain = Some(chain.to_string());
    }
    target.preset = preset;
    target.health = t.health.overrides();
    target.rules.extend(rules);

    // The monitor's alerts become rules: price bounds on the price the
    // target has; a whale or a volume spike on the venue's trades and
    // volume since the last tick. A rule says what must hold, so an alert
    // at a limit is a rule below it.
    let alerts = &config.monitor.alerts;
    let before = target.rules.len();
    let price = match target.kind {
        TargetKind::Pair => MetricId::Mid,
        _ => MetricId::PriceUsd,
    };
    let venue_alerts = [
        (alerts.whale_min_usd, MetricId::LargestTrade, Op::Lt),
        (
            alerts.volume_spike_threshold_pct,
            MetricId::VolumeChangePct,
            Op::Le,
        ),
    ];
    if target.kind == TargetKind::Address {
        if alerts.price_min.is_some()
            || alerts.price_max.is_some()
            || venue_alerts.iter().any(|(limit, _, _)| limit.is_some())
        {
            notes.push(
                "The alerts of config `monitor.alerts` read a price or a market. They do not apply to an address."
                    .to_string(),
            );
        }
    } else {
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
        if !target.venues.is_empty() {
            for (limit, metric, op) in venue_alerts {
                if let Some(value) = limit {
                    target.rules.push(Rule { metric, op, value });
                }
            }
        } else if venue_alerts.iter().any(|(limit, _, _)| limit.is_some()) {
            notes.push(
                "The whale and volume-spike alerts of config `monitor.alerts` need a venue (`--venue binance`): they read its trades and 24h volume."
                    .to_string(),
            );
        }
    }
    let alert_rules = target.rules[before..]
        .iter()
        .map(ToString::to_string)
        .collect();
    Ok(BuiltTarget {
        target,
        notes,
        alert_rules,
    })
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
    let run = &w.run;
    let mut list = Watchlist::new(
        w.save
            .clone()
            .unwrap_or_else(|| format!("monitor-{}", args.token.to_lowercase())),
    );
    if let Some(every) = &run.every {
        list.interval = parse_interval(every)?;
    } else if let Some(secs) = args.refresh {
        if secs == 0 {
            return Err("--refresh must be at least 1 second".to_string());
        }
        list.interval = Duration::from_secs(secs);
    }
    if let Some(d) = &run.duration {
        list.duration = parse_run_duration(d)?;
    }
    list.outputs.csv = run.csv.clone().or_else(|| args.export.clone());
    list.outputs.report = run.report.clone();

    let BuiltTarget {
        target, mut notes, ..
    } = target_from_args(&args.token, &args.target, config)?;
    if args.export.is_some() && run.csv.is_none() {
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

    // `scope monitor` and `scope watch` share these two functions, so a
    // value means the same in both commands.
    #[test]
    fn the_timing_flags_have_one_parser() {
        assert_eq!(parse_interval("30s"), Ok(Duration::from_secs(30)));
        assert_eq!(
            parse_run_duration("1h"),
            Ok(Some(Duration::from_secs(3600)))
        );
        assert_eq!(parse_run_duration("forever"), Ok(None));
        assert!(parse_interval("0s").unwrap_err().starts_with("--every:"));
        assert!(
            parse_run_duration("soon")
                .unwrap_err()
                .starts_with("--duration:")
        );
        // `scope monitor` gives the same list as the functions.
        let w = from("monitor USDC --venue binance --every 5m --duration 2d")
            .unwrap()
            .0;
        assert_eq!(Some(w.interval), parse_interval("5m").ok());
        assert_eq!(w.duration, parse_run_duration("2d").unwrap());
    }

    #[test]
    fn the_trigger_case_is_usdn_on_weex_with_the_health_checks_every_30s_for_1h() {
        let (w, notes) = from("monitor USDN --venue weex --every 30s --duration 1h").unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(w.interval, Duration::from_secs(30));
        assert_eq!(w.duration, Some(Duration::from_secs(3600)));
        let t = &w.targets[0];
        assert_eq!((t.kind, t.id.as_str()), (TargetKind::Pair, "USDN"));
        assert_eq!(t.venues.as_slice(), ["weex"]);
        assert_eq!(t.preset, Some(Preset::MarketHealth));
    }

    // Design 2026-11-04, section 2.8: one token watched on several venues.
    #[test]
    fn venue_repeats_or_takes_a_comma_list() {
        let a = from("monitor USDN --venue weex --venue binance").unwrap().0;
        let b = from("monitor USDN --venue weex,binance").unwrap().0;
        assert_eq!(a, b);
        assert_eq!(a.targets.len(), 1, "one target, not one per venue");
        assert_eq!(a.targets[0].venues, ["weex", "binance"]);
        assert_eq!(a.targets[0].preset, Some(Preset::MarketHealth));
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
    fn export_is_the_csv_and_says_so() {
        let (w, notes) = from("monitor USDC --venue binance --export x.csv").unwrap();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("--export is now --csv"));
        assert_eq!(
            w.outputs.csv.as_deref(),
            Some(std::path::Path::new("x.csv"))
        );
    }

    #[test]
    fn every_alert_becomes_a_rule_on_a_venue() {
        let mut config = Config::default();
        config.monitor.alerts.price_min = Some(0.99);
        config.monitor.alerts.price_max = Some(1.01);
        config.monitor.alerts.whale_min_usd = Some(100_000.0);
        config.monitor.alerts.volume_spike_threshold_pct = Some(50.0);
        let (w, notes) = from_with("monitor USDC --venue binance", &config).unwrap();
        let rules: Vec<String> = w.targets[0].rules.iter().map(ToString::to_string).collect();
        assert_eq!(
            rules,
            [
                "mid_price >= 0.99",
                "mid_price <= 1.01",
                "largest_trade < 100000",
                "volume_change_pct <= 50"
            ]
        );
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn without_a_venue_the_whale_and_spike_alerts_say_they_need_one() {
        let mut config = Config::default();
        config.monitor.alerts.price_min = Some(0.99);
        config.monitor.alerts.volume_spike_threshold_pct = Some(50.0);
        let (w, notes) = from_with("monitor PEPE", &config).unwrap();
        let rules: Vec<String> = w.targets[0].rules.iter().map(ToString::to_string).collect();
        assert!(
            rules.contains(&"price_usd >= 0.99".to_string()),
            "{rules:?}"
        );
        assert!(
            !rules.iter().any(|r| r.starts_with("volume_change_pct")),
            "{rules:?}"
        );
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("need a venue"), "{notes:?}");
        // Neither alert set: no note.
        let (_, notes) = from_with("monitor PEPE", &Config::default()).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
    }
}
