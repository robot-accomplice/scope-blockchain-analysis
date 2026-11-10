//! # Monitor arguments
//!
//! `scope monitor <token>` opens the monitor wizard in the TUI, filled from
//! these arguments (#47). The wizard turns them into a watchlist. The old live
//! dashboard (`--dashboard`) was removed after its deprecation in 0.8.0.

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
  scope monitor @dai-token                                  \x1b[2m# address book shortcut\x1b[0m
  scope monitor BTC --venue binance --layout exchange --dashboard   \x1b[2m# deprecated\x1b[0m")]
pub struct MonitorArgs {
    /// Token address or symbol to monitor.
    ///
    /// Can be a contract address (0x...) or a token symbol on `--chain`.
    /// A symbol that names more than one token there lists them so you
    /// can choose (only exact symbols match).
    /// Use @label to resolve from the address book (e.g., @dai-token).
    pub token: String,

    /// The token's contract: use it and skip the symbol lookup.
    #[arg(long = "token", value_name = "CONTRACT")]
    pub token_contract: Option<String>,

    /// Target blockchain network.
    ///
    /// Determines which chain to query for token data.
    #[arg(short, long, default_value = scope::chains::DEFAULT_CHAIN)]
    pub chain: String,

    /// Refresh interval in seconds.
    ///
    /// How often to fetch new data from the API.
    #[arg(short, long)]
    pub refresh: Option<u64>,

    /// Start CSV export immediately, writing to the given path.
    #[arg(short, long, value_name = "PATH")]
    pub export: Option<PathBuf>,

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

    /// The watchlist options: `scope monitor` opens the monitor wizard in
    /// the TUI with these filled in (#47).
    #[command(flatten)]
    pub watch: MonitorWatchArgs,
}

/// The options of the monitor wizard (#47). The wizard turns the monitor
/// arguments into a watchlist; these set its timing, outputs and checks.
#[derive(Debug, Clone, Default, Args)]
pub struct MonitorWatchArgs {
    /// Interval between runs, as a duration (30s, 5m). The same as
    /// `--refresh` in seconds. Default: 60s.
    #[arg(long, value_name = "INTERVAL", conflicts_with = "refresh")]
    pub every: Option<String>,

    /// How long the run lasts (10m, 1h, or `forever`). Default: 1h.
    #[arg(long, value_name = "DURATION")]
    pub duration: Option<String>,

    /// Write one CSV row per check per run to this path.
    #[arg(long, value_name = "PATH")]
    pub csv: Option<PathBuf>,

    /// Write the report to this path when the run ends or is stopped.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,

    /// The checks: `market-health` (the default with `--venue`) or `none`.
    #[arg(long, value_name = "PRESET")]
    pub preset: Option<String>,

    /// Save the watchlist under this name (`watch open <name>` runs it again).
    #[arg(long, value_name = "NAME")]
    pub save: Option<String>,

    /// Start the run at once, without the wizard.
    #[arg(long)]
    pub yes: bool,

    /// Health threshold overrides for the market health checks.
    #[command(flatten)]
    pub health: crate::cli::health_args::HealthArgs,
}
