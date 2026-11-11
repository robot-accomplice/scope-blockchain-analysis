//! # Monitor arguments
//!
//! `scope monitor <token>` opens the monitor wizard in the TUI, filled from
//! these arguments (#47). The wizard turns them into a watchlist. The old live
//! dashboard (`--dashboard`) was removed after its deprecation in 0.8.0.

use crate::cli::watch_args::{RunArgs, TargetArgs};
use clap::Args;
use std::path::PathBuf;

// ============================================================================
// CLI Arguments
// ============================================================================

/// Arguments for the top-level `monitor` command.
///
/// Opens the monitor wizard in the TUI, filled from these arguments.
///
/// # Examples
///
/// ```bash
/// # Monitor by token address
/// scope monitor 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48
///
/// # Monitor by symbol on a specific chain
/// scope monitor USDC --chain ethereum
///
/// # Short alias
/// scope mon PEPE -c ethereum
///
/// # Custom refresh interval
/// scope monitor USDC --refresh 3
/// ```
#[derive(Debug, Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope monitor USDN --venue weex --every 30s --duration 1h
  scope monitor DAI --venue binance --pair DAI_USDT --report dai.md
  scope monitor USDC --venue binance --save stables --yes   \x1b[2m# save, start at once\x1b[0m
  scope monitor PEPE --chain ethereum                       \x1b[2m# price and volume\x1b[0m
  scope monitor @dai-token                                  \x1b[2m# address book shortcut\x1b[0m",
    mut_arg("every", |a| a.help(EVERY_HELP).conflicts_with("refresh")),
    mut_arg("duration", |a| a.help(DURATION_HELP)),
    mut_arg("csv", |a| a.help(CSV_HELP)),
    mut_arg("report", |a| a.help(REPORT_HELP)))]
pub struct MonitorArgs {
    /// Token address or symbol to monitor.
    ///
    /// Can be a contract address (0x...) or a token symbol on `--chain`.
    /// A symbol that names more than one token there lists them so you
    /// can choose (only exact symbols match).
    /// Use @label to resolve from the address book (e.g., @dai-token).
    pub token: String,

    /// What to watch: the flags that `scope watch` takes too.
    #[command(flatten)]
    pub target: TargetArgs,

    /// Refresh interval in seconds.
    ///
    /// How often to fetch new data from the API.
    #[arg(short, long)]
    pub refresh: Option<u64>,

    /// Start CSV export immediately, writing to the given path.
    #[arg(short, long, value_name = "PATH")]
    pub export: Option<PathBuf>,

    /// The watchlist options: `scope monitor` opens the monitor wizard in
    /// the TUI with these filled in (#47).
    #[command(flatten)]
    pub watch: MonitorWatchArgs,
}

/// The help of the shared run flags in `scope monitor`.
const EVERY_HELP: &str = "Interval between runs, as a duration (30s, 5m). The same as `--refresh` in seconds. Default: 60s";
const DURATION_HELP: &str = "How long the run lasts (10m, 1h, or `forever`). Default: 1h";
const CSV_HELP: &str = "Write one CSV row per check per run to this path";
const REPORT_HELP: &str = "Write the report to this path when the run ends or is stopped";

/// The options of the monitor wizard (#47). The wizard turns the monitor
/// arguments into a watchlist; these set its timing, outputs and name.
#[derive(Debug, Clone, Default, Args)]
pub struct MonitorWatchArgs {
    /// Timing and outputs, shared with `scope watch`.
    #[command(flatten)]
    pub run: RunArgs,

    /// Save the watchlist under this name. To run it again with no screen,
    /// use `scope watch <name>`. To open it in the grid, start `scope tui`
    /// and enter `watch <name>` there.
    #[arg(long, value_name = "NAME")]
    pub save: Option<String>,

    /// Start the run at once, without the wizard.
    #[arg(long)]
    pub yes: bool,
}

#[cfg(test)]
mod tests {
    use crate::cli::Cli;
    use clap::Parser;

    // The dashboard flags are gone (#193). A help example that uses one
    // fails when a user copies it.
    #[test]
    fn the_help_names_no_flag_of_the_removed_dashboard() {
        let help = Cli::try_parse_from(["scope", "monitor", "--help"])
            .unwrap_err()
            .to_string();
        assert!(help.contains("Examples:"), "{help}");
        for flag in ["--dashboard", "--layout", "--scale", "--color-scheme"] {
            assert!(!help.contains(flag), "the help names {flag}: {help}");
        }
    }
}
