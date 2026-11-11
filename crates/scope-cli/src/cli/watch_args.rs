//! The flags that `scope monitor` and `scope watch` share, defined once
//! (design 2026-11-11, sections 2.2.3 and 7, decision D1).
//!
//! - [`TargetArgs`]: what to watch (`--venue`, `--chain`, `--pair`,
//!   `--token`, `--preset`, the health flags, `--rule`, `--kind`).
//! - [`RunArgs`]: when and where (`--every`, `--duration`, `--csv`,
//!   `--report`).
//!
//! The positional argument stays in each command, because its meaning
//! differs: a token in `scope monitor`; a name, a path or a token in
//! `scope watch`. Each command sets the help text of the [`RunArgs`] flags
//! for itself (`mut_arg`), because the two commands say different things
//! about them (`--every` with `--once`, `--refresh`).

use super::health_args::HealthArgs;
use clap::{Args, ValueEnum};
use scope::market::HealthOverrides;
use std::path::PathBuf;

/// The kind of target, when the default is not the one you want.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum KindArg {
    /// A pair on one or more venues. The default with `--venue`.
    Pair,
    /// A token. The default without `--venue`.
    Token,
    /// A wallet address. It has no checks of its own: give `--rule`.
    Address,
}

/// What to watch.
#[derive(Debug, Clone, Default, Args)]
pub struct TargetArgs {
    /// The token's contract: use it and skip the symbol lookup.
    #[arg(long = "token", value_name = "CONTRACT")]
    pub token_contract: Option<String>,

    /// Target blockchain network.
    ///
    /// Determines which chain to query for token data.
    ///
    /// [default: ethereum]
    #[arg(short, long)]
    pub chain: Option<String>,

    /// Exchange venue to watch the token on (e.g., binance, mexc).
    ///
    /// Repeat it, or give a comma list, to watch the token on several
    /// venues (`--venue weex,binance`).
    #[arg(long, value_name = "VENUE", value_delimiter = ',')]
    pub venue: Vec<String>,

    /// Direct trading pair for exchange-only mode (e.g., DAI_USDT).
    ///
    /// Bypasses DexScreener token resolution entirely and uses the exchange
    /// ticker as the data source. Requires `--venue` to be specified.
    /// Use this when the token is not listed on DexScreener.
    #[arg(long, value_name = "PAIR")]
    pub pair: Option<String>,

    /// The checks: `market-health` (the default with `--venue`) or `none`.
    #[arg(long, value_name = "PRESET")]
    pub preset: Option<String>,

    /// A rule that must hold, for example "spread_pct < 0.5". Repeat it for
    /// more rules. Use it with `--kind address` to watch a wallet.
    #[arg(long, value_name = "RULE")]
    pub rule: Vec<String>,

    /// The kind of target. The default is a pair with `--venue`, else a
    /// token.
    #[arg(long, value_enum, value_name = "KIND")]
    pub kind: Option<KindArg>,

    /// Health threshold overrides for the market health checks.
    #[command(flatten)]
    pub health: HealthArgs,
}

impl TargetArgs {
    /// True when any target flag is given. In `scope watch`, the positional
    /// argument is then a target and not a saved name. One pure function:
    /// a test sets every flag and checks it counts.
    pub fn is_set(&self) -> bool {
        self.token_contract.is_some()
            || self.chain.is_some()
            || !self.venue.is_empty()
            || self.pair.is_some()
            || self.preset.is_some()
            || !self.rule.is_empty()
            || self.kind.is_some()
            || self.health.overrides() != HealthOverrides::default()
    }
}

/// When a run happens and where it writes.
#[derive(Debug, Clone, Default, Args)]
pub struct RunArgs {
    /// Time from one run to the next, as a duration (30s, 5m).
    #[arg(long, value_name = "INTERVAL")]
    pub every: Option<String>,

    /// How long the run lasts (10m, 1h, or `forever`).
    #[arg(long, value_name = "DURATION")]
    pub duration: Option<String>,

    /// Write one CSV row for each check and run to this path.
    #[arg(long, value_name = "PATH")]
    pub csv: Option<PathBuf>,

    /// Write the report to this path when the run ends or is stopped.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::FromArgMatches;

    /// Why: a flag that `is_set` does not count would turn an inline target
    /// into a saved name with no error. The test reads the flags from the
    /// struct, so a new flag is covered without an edit here.
    #[test]
    fn is_set_counts_every_target_flag() {
        let command = TargetArgs::augment_args(clap::Command::new("t"));
        let ids: Vec<(String, String)> = command
            .get_arguments()
            .filter_map(|a| Some((a.get_id().to_string(), a.get_long()?.to_string())))
            .collect();
        assert!(ids.len() >= 8, "the flags were not found: {ids:?}");
        assert!(!TargetArgs::default().is_set());
        for (id, long) in ids {
            // The first value that the flag's parser takes.
            let parsed = ["1", "address", "ethereum", "x"].iter().find_map(|value| {
                let argv = vec!["t".to_string(), format!("--{long}"), (*value).to_string()];
                let flag_only = vec!["t".to_string(), format!("--{long}")];
                [argv, flag_only].into_iter().find_map(|argv| {
                    let m = command.clone().try_get_matches_from(argv).ok()?;
                    TargetArgs::from_arg_matches(&m).ok()
                })
            });
            let args = parsed.unwrap_or_else(|| panic!("--{long} ({id}) takes no tried value"));
            assert!(args.is_set(), "--{long} ({id}) does not make is_set true");
        }
    }

    /// Why: the one place that defines `--chain` says what the default is,
    /// and the clap default is gone (it hid whether the user typed it).
    #[test]
    fn the_chain_help_names_the_default_chain() {
        let help = TargetArgs::augment_args(clap::Command::new("t"))
            .render_long_help()
            .to_string();
        assert!(
            help.contains(&format!("[default: {}]", scope::chains::DEFAULT_CHAIN)),
            "{help}"
        );
    }

    /// The `--help` of a command, as plain text.
    fn help_of(command: &str) -> String {
        use clap::CommandFactory;
        crate::cli::Cli::command()
            .find_subcommand_mut(command)
            .unwrap()
            .render_long_help()
            .to_string()
    }

    /// The help of a flag: the line with its name, then its text.
    fn flag_help(help: &str, flag: &str) -> Option<String> {
        let lines: Vec<&str> = help.lines().collect();
        let at = lines.iter().position(|l| {
            l.trim_start().starts_with(&format!("--{flag} <"))
                || l.contains(&format!(" --{flag} <"))
        })?;
        Some(lines[at + 1].trim().to_string())
    }

    /// Why (decision D1): the four run flags are defined once in
    /// [`RunArgs`], and the merge changes no visible text. These are the
    /// texts of both commands before the merge; each command keeps its own,
    /// because they say different things (`--refresh`, `--once`).
    #[test]
    fn the_merge_of_the_run_flags_changes_no_help_text() {
        let monitor = help_of("monitor");
        let watch = help_of("watch");
        let expected = [
            (
                "every",
                "Interval between runs, as a duration (30s, 5m). The same as `--refresh` in seconds. Default: 60s",
                "Time from one tick start to the next (30s, 5m). Overrides the file. With --once it does not wait: it says how far apart the runs are (the look-back of the first tick and the age at which the saved state is stale)",
            ),
            (
                "duration",
                "How long the run lasts (10m, 1h, or `forever`). Default: 1h",
                "How long the run lasts (10m, 1h, or `forever`). Overrides the file",
            ),
            (
                "csv",
                "Write one CSV row per check per run to this path",
                "Write one CSV row for each cell and tick. Overrides the file",
            ),
            (
                "report",
                "Write the report to this path when the run ends or is stopped",
                "Write the report when the run ends or stops. Overrides the file",
            ),
        ];
        for (flag, monitor_text, watch_text) in expected {
            assert_eq!(
                flag_help(&monitor, flag).as_deref(),
                Some(monitor_text),
                "monitor --{flag}"
            );
            assert_eq!(
                flag_help(&watch, flag).as_deref(),
                Some(watch_text),
                "watch --{flag}"
            );
        }
        // The value names did not change either.
        for (flag, value) in [
            ("every", "INTERVAL"),
            ("duration", "DURATION"),
            ("csv", "PATH"),
            ("report", "PATH"),
        ] {
            for help in [&monitor, &watch] {
                assert!(
                    help.contains(&format!("--{flag} <{value}>")),
                    "--{flag}: {help}"
                );
            }
        }
    }

    /// Why: share, do not copy. Both commands take every flag of
    /// [`TargetArgs`] and [`RunArgs`] with the same argument ids, and the
    /// help of each target flag is the same text in both. A copy would
    /// drift.
    #[test]
    fn both_commands_take_the_shared_flags_with_the_same_help() {
        use clap::CommandFactory;
        let mut cli = crate::cli::Cli::command();
        let shared: Vec<String> = TargetArgs::augment_args(clap::Command::new("t"))
            .get_arguments()
            .chain(RunArgs::augment_args(clap::Command::new("r")).get_arguments())
            .map(|a| a.get_id().to_string())
            .collect();
        assert!(shared.len() >= 12, "{shared:?}");
        let mut ids = |name: &str| -> Vec<String> {
            cli.find_subcommand_mut(name)
                .unwrap()
                .get_arguments()
                .map(|a| a.get_id().to_string())
                .collect()
        };
        let (monitor, watch) = (ids("monitor"), ids("watch"));
        for id in &shared {
            assert!(monitor.contains(id), "monitor lacks {id}");
            assert!(watch.contains(id), "watch lacks {id}");
        }
        let (monitor_help, watch_help) = (help_of("monitor"), help_of("watch"));
        for flag in [
            "token", "chain", "venue", "pair", "preset", "rule", "kind", "peg",
        ] {
            let a = flag_help(&monitor_help, flag);
            assert!(a.is_some(), "monitor help has no --{flag}");
            assert_eq!(a, flag_help(&watch_help, flag), "--{flag}");
        }
    }
}
