//! A target on the command line: `scope watch USDC --venue weex --once`
//! (design 2026-11-11, section 2).
//!
//! The command line alone decides what the first argument means. With any
//! target flag ([`TargetArgs::is_set`]) it is a target. Without one it is a
//! saved name or a path, as before. The files on disk never decide, so
//! saving a list named `USDC` cannot change what a cron job does.
//!
//! The target is built by [`target_from_args`], the same function that fills
//! the monitor wizard, so a words mean one watch in both commands. The
//! result is checked as a file is: its canonical text is parsed back with
//! the venue ids. Every error here is exit code 3, and none makes a request.

use super::headless::{Context, WatchArgs};
use super::monitor::{BuiltTarget, target_from_args};
use super::{is_watchlist_path, watchlist_path};
use scope::watch::{Watchlist, file};

/// What a run from the command line tells the user and the `start` record
/// (design section 2.4).
#[derive(Debug, Clone, PartialEq)]
pub struct Inline {
    /// The notes that building the target gave.
    pub notes: Vec<String>,
    /// The rules that came from config `monitor.alerts`.
    pub alert_rules: Vec<String>,
    /// The name of a saved list that this run ignores, when one exists.
    pub shadowed: Option<String>,
    /// The canonical text of the effective list: what a person can paste
    /// into a file to get the same run. Set by [`validate`].
    pub definition: String,
}

impl Inline {
    /// The lines of the start note: the notes of the build, the saved list
    /// that this run ignores, and the rules the user did not type.
    pub fn notes_for_user(&self) -> Vec<String> {
        let mut lines = self.notes.clone();
        if let Some(name) = &self.shadowed {
            lines.push(format!(
                "A saved watchlist named {name} exists. This run ignores it. Drop the target flags to run it."
            ));
        }
        if !self.alert_rules.is_empty() {
            lines.push(format!(
                "Rules from config `monitor.alerts`: {}.",
                self.alert_rules.join(", ")
            ));
        }
        lines
    }
}

/// The watchlist that the command line describes, before the run flags
/// (`--every`, `--duration`, `--once`, `--csv`, `--report`) are applied.
///
/// # Errors
///
/// The lines to print: a path with target flags, an unknown chain, or an
/// error of the target flags.
pub fn build(args: &WatchArgs, ctx: &Context<'_>) -> Result<(Watchlist, Inline), Vec<String>> {
    if is_watchlist_path(&args.list) {
        return Err(vec![format!(
            "`{}` is a path, and a file holds its own targets. Drop the target flags to run the file, or drop the path to watch a token.",
            args.list
        )]);
    }
    if let Some(chain) = args.target.chain.as_deref()
        && scope::chains::registry::chain(chain).is_none()
    {
        return Err(vec![format!(
            "unknown chain `{chain}`; use one of: {}",
            scope::chains::registry::client_chain_list()
        )]);
    }
    let BuiltTarget {
        target,
        notes,
        alert_rules,
    } = target_from_args(&args.list, &args.target, ctx.config).map_err(|e| vec![e])?;
    let mut list = Watchlist::new(format!("cli-{}", target.id));
    list.targets.push(target);
    let shadowed = watchlist_path(&args.list, &ctx.dir)
        .exists()
        .then(|| args.list.clone());
    let inline = Inline {
        notes,
        alert_rules,
        shadowed,
        definition: String::new(),
    };
    Ok((list, inline))
}

/// Checks the effective list as a file is checked: it is written as the
/// canonical text and parsed back with the venue ids and the rules of each
/// kind. Returns the parsed list and sets `inline.definition` to the text.
///
/// # Errors
///
/// The errors of the file parser, one for each line to print.
pub fn validate(
    list: &Watchlist,
    venues: &[String],
    once: bool,
    inline: &mut Inline,
) -> Result<Watchlist, Vec<String>> {
    // `--once` is a property of the run and not of the job: a file cannot
    // say a duration of 0, and the same job in cron has the same memory
    // whether it runs once or for an hour.
    let mut defined = list.clone();
    if once {
        defined.duration = Watchlist::new("").duration;
    }
    let text = file::to_text(&defined);
    let mut parsed = file::parse(&text, &list.name, Some(venues))
        .map_err(|errors| errors.0.iter().map(ToString::to_string).collect::<Vec<_>>())?;
    inline.definition = text;
    if once {
        parsed.duration = list.duration;
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::output::Output;
    use crate::cli::{Cli, Commands};
    use clap::Parser;
    use scope::chains::mocks::MockClientFactory;
    use scope::config::Config;

    pub(crate) fn watch_args(argv: &str) -> WatchArgs {
        let words: Vec<&str> = std::iter::once("scope")
            .chain(argv.split_whitespace())
            .collect();
        let Commands::Watch(args) = Cli::try_parse_from(words).unwrap().command else {
            panic!("not watch")
        };
        args
    }

    fn built(
        argv: &str,
        config: &Config,
        saved: &[&str],
    ) -> Result<(Watchlist, Inline), Vec<String>> {
        let dir = tempfile::tempdir().unwrap();
        for name in saved {
            std::fs::write(dir.path().join(format!("{name}.yaml")), "targets: []\n").unwrap();
        }
        let factory = MockClientFactory::new();
        let (out, _) = Output::capture();
        let ctx = Context {
            config,
            clients: &factory,
            out: &out,
            dir: dir.path().to_path_buf(),
            state_dir: dir.path().join("state"),
            argv: Vec::new(),
            venues: vec!["weex".into(), "binance".into()],
        };
        build(&watch_args(argv), &ctx)
    }

    /// Why: a user who tries a target in the wizard must get the same watch
    /// in cron. Each row of the table of section 2.2.2: `scope watch` and
    /// `scope monitor` build equal targets from the same words.
    #[test]
    fn scope_watch_builds_the_target_that_scope_monitor_builds() {
        let rows = [
            "USDC --venue weex",
            "USDC --venue weex,binance",
            "USDN --venue weex --pair USDN_USDT",
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --chain ethereum",
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --chain base",
            "@dai-token --chain ethereum",
            "USDC --venue weex --rule spread_pct<0.5 --max-spread-pct 0.2",
        ];
        let mut config = Config::default();
        config.monitor.alerts.price_min = Some(0.99);
        for row in rows {
            let (list, inline) = built(&format!("watch {row}"), &config, &[]).unwrap();
            let monitor = Cli::try_parse_from(
                std::iter::once("scope")
                    .chain(["monitor"])
                    .chain(row.split_whitespace()),
            )
            .unwrap();
            let Commands::Monitor(m) = monitor.command else {
                panic!("not monitor")
            };
            let (wl, notes) = super::super::monitor::watchlist_from_args(&m, &config).unwrap();
            assert_eq!(list.targets, wl.targets, "{row}");
            assert_eq!(inline.notes, notes, "{row}");
        }
    }

    /// Why: the table of section 2.2.2 is the contract; a change must show.
    #[test]
    fn the_forms_map_to_kind_id_chain_venues_preset_and_rules() {
        use scope::watch::{Preset, TargetKind};
        let config = Config::default();
        let one = |argv: &str| built(&format!("watch {argv}"), &config, &[]).unwrap().0;
        let rules = |w: &Watchlist| -> Vec<String> {
            w.targets[0].rules.iter().map(ToString::to_string).collect()
        };

        let w = one("USDC --venue weex");
        let t = &w.targets[0];
        assert_eq!((t.kind, t.id.as_str()), (TargetKind::Pair, "USDC"));
        assert_eq!(t.venues, ["weex"]);
        assert_eq!(t.preset, Some(Preset::MarketHealth));
        assert!(rules(&w).is_empty());
        assert_eq!(w.name, "cli-USDC");
        assert_eq!(w.duration, Some(std::time::Duration::from_secs(3600)));

        let w = one("USDN --venue weex --pair USDN_USDT");
        assert_eq!(w.targets[0].id, "USDN_USDT");

        let w = one("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --chain base");
        let t = &w.targets[0];
        assert_eq!(t.kind, TargetKind::Token);
        assert_eq!(t.chain.as_deref(), Some("base"));
        assert!(t.venues.is_empty() && t.preset.is_none());
        assert_eq!(rules(&w), ["price_usd > 0", "volume_24h >= 0"]);

        let w = one("0xWallet --kind address --rule native_balance>1");
        let t = &w.targets[0];
        assert_eq!((t.kind, t.id.as_str()), (TargetKind::Address, "0xWallet"));
        assert_eq!(rules(&w), ["native_balance > 1"]);
    }

    /// Why (test 9): `--chain` has no clap default now, so both cases need
    /// a test. `--chain ethereum` is a target flag, so the word is a token
    /// and not a saved name; the default chain is not written on the
    /// target, as in `scope monitor`.
    #[test]
    fn a_chain_flag_makes_an_inline_token_and_the_default_chain_is_not_stored() {
        let config = Config::default();
        let args = watch_args("watch USDC --chain ethereum");
        assert!(args.target.is_set());
        let (w, _) = built("watch USDC --chain ethereum", &config, &["USDC"]).unwrap();
        let t = &w.targets[0];
        assert_eq!(t.kind, scope::watch::TargetKind::Token);
        assert_eq!(t.chain, None);
        assert!(!watch_args("watch USDC").target.is_set());
    }

    /// Why: the cron job must mean one thing, whatever files exist.
    #[test]
    fn the_command_line_decides_between_a_target_and_a_saved_name() {
        let config = Config::default();
        // A saved list named USDC exists: the target flag still wins, and
        // the result says which list it ignores.
        let (_, inline) = built("watch USDC --venue weex", &config, &["USDC"]).unwrap();
        assert_eq!(inline.shadowed.as_deref(), Some("USDC"));
        let (_, inline) = built("watch USDC --venue weex", &config, &[]).unwrap();
        assert_eq!(inline.shadowed, None);
        // A path names a file: target flags are an error.
        let e = built("watch ./pegs.yaml --venue weex", &config, &[]).unwrap_err();
        assert!(e[0].contains("is a path"), "{e:?}");
        let e = built("watch pegs.yaml --chain base", &config, &[]).unwrap_err();
        assert!(e[0].contains("is a path"), "{e:?}");
    }

    /// Why: every one of these is exit code 3 and must be found before any
    /// request, so a cron job fails on its first line and not on its tenth.
    #[test]
    fn the_input_errors_are_found_before_any_request() {
        let config = Config::default();
        let err = |argv: &str| -> String {
            let (w, mut inline) = match built(&format!("watch {argv}"), &config, &[]) {
                Ok(ok) => ok,
                Err(e) => return e.join("\n"),
            };
            let venues = vec!["weex".to_string(), "binance".to_string()];
            validate(&w, &venues, false, &mut inline)
                .map(|_| "no error".to_string())
                .unwrap_or_else(|e| e.join("\n"))
        };
        assert!(err("USDC --venue nowhere").contains("nowhere"));
        assert!(err("USDC --venue weex --preset peg").contains("unknown preset"));
        assert!(err("USDC --preset market-health").contains("need a venue"));
        assert!(err("USDC --pair USDC_USDT --chain base").contains("--pair needs a venue"));
        assert!(err("USDC --venue weex --rule nonsense").contains("--rule"));
        assert!(err("USDC --venue weex --rule holders>1").contains("holders"));
        assert!(err("0xW --kind address").contains("needs a rule"));
        assert!(err("USDC --chain nochain").contains("unknown chain"));
        assert_eq!(err("USDC --venue weex"), "no error");
    }

    /// Why: a rule the user did not type must be visible (Q4). The config
    /// alerts apply to an inline target, and the result lists them.
    #[test]
    fn config_alerts_apply_and_are_listed() {
        let mut config = Config::default();
        config.monitor.alerts.price_min = Some(0.99);
        config.monitor.alerts.whale_min_usd = Some(100_000.0);
        let (w, inline) =
            built("watch USDC --venue weex --rule spread_pct<1", &config, &[]).unwrap();
        let rules: Vec<String> = w.targets[0].rules.iter().map(ToString::to_string).collect();
        assert_eq!(
            rules,
            [
                "spread_pct < 1",
                "mid_price >= 0.99",
                "largest_trade < 100000"
            ]
        );
        assert_eq!(
            inline.alert_rules,
            ["mid_price >= 0.99", "largest_trade < 100000"]
        );
    }

    /// Why: a person can paste the `definition` of the `start` record into
    /// a file and get the same run.
    #[test]
    fn the_definition_parses_back_to_the_same_list() {
        let mut config = Config::default();
        config.monitor.alerts.price_max = Some(1.01);
        let (list, mut inline) = built(
            "watch USDN --venue weex,binance --rule spread_pct<1 --max-spread-pct 0.3 --every 5m",
            &config,
            &[],
        )
        .unwrap();
        let venues = vec!["weex".to_string(), "binance".to_string()];
        let parsed = validate(&list, &venues, false, &mut inline).unwrap();
        assert_eq!(parsed, list);
        let again = file::parse(&inline.definition, &list.name, Some(&venues)).unwrap();
        assert_eq!(again, list);
        assert!(inline.definition.contains("spread_pct < 1"));
    }
}
