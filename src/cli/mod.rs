//! # CLI Module
//!
//! This module defines the command-line interface using `clap` with derive macros.
//! It provides the main `Cli` struct and `Commands` enum that define all
//! available commands and their arguments.
//!
//! ## UX Features
//!
//! - **Progress indicators** — Spinners and step counters via the `progress` module
//! - **Help with examples** — `after_help` blocks with example invocations
//! - **Command grouping** — Commands ordered by task (entity lookup, token
//!   analysis, compliance, data/export, config)
//! - **Shell completion** — `scope completions bash|zsh|fish` via `clap_complete`
//! - **Typo suggestions** — Built-in clap fuzzy matching for misspelled commands
//!
//! ## Command Structure
//!
//! ```text
//! scope [OPTIONS] <COMMAND>
//!
//! Entity lookup:
//!   address      Analyze a blockchain address (alias: addr)
//!   tx           Analyze a transaction (alias: transaction)
//!   contract     Analyze a smart contract (alias: ct)
//!   insights     Auto-detect target type and run analyses (alias: insight)
//!
//! Token analysis:
//!   crawl        Crawl a token for DEX analytics (alias: token)
//!   token-health DEX analytics + optional order book (alias: health)
//!   discover     Browse trending/boosted tokens (alias: disc)
//!   monitor      Live TUI dashboard (alias: mon)
//!   market       Peg and order book health for stablecoins
//!   venues       Manage exchange venue descriptors (alias: ven)
//!
//! Compliance:
//!   compliance   Risk, trace, analyze, compliance-report
//!
//! Data & export:
//!   address-book Add, remove, list, summary (alias: ab, portfolio)
//!   export       Export to JSON/CSV
//!   report       Batch and combined reports
//!
//! Config & interactive:
//!   interactive  REPL with preserved context (alias: shell)
//!   setup        Configure API keys and preferences (alias: config)
//!   completions  Generate shell completions for bash/zsh/fish
//!
//! Options:
//!   --config <PATH>   Path to configuration file
//!   -v, --verbose...  Increase logging verbosity
//!   --ai              Markdown output for agent parsing
//!   --no-color        Disable colored output
//!   -h, --help        Print help (with examples)
//!   -V, --version     Print version
//! ```

pub mod address;
pub mod address_book;
pub mod address_report;
pub mod compliance;
pub mod contract;
pub mod crawl;
pub mod discover;
pub mod errors;
pub mod export;
pub mod insights;
pub mod interactive;
pub mod market;
pub mod monitor;
pub mod progress;
pub mod report;
pub mod setup;
pub mod token_health;
pub mod tx;
pub mod venues;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

pub use address::AddressArgs;
pub use address_book::AddressBookArgs;
pub use crawl::CrawlArgs;
pub use export::ExportArgs;
pub use interactive::InteractiveArgs;
pub use monitor::MonitorArgs;
pub use setup::SetupArgs;
pub use tx::TxArgs;

/// Blockchain Analysis CLI - A tool for blockchain data analysis.
///
/// Scope provides comprehensive blockchain analysis capabilities including
/// address investigation, transaction decoding, address book management, and
/// data export functionality.
#[derive(Debug, Parser)]
#[command(
    name = "scope",
    version,
    about = "Scope Blockchain Analysis - A tool for blockchain data analysis",
    long_about = format!(
        "Scope Blockchain Analysis v{}\n\n\
         A production-grade tool for blockchain data analysis, address management,\n\
         and transaction investigation.\n\n\
         Use --help with any subcommand for detailed usage information.",
        env!("CARGO_PKG_VERSION")
    ),
    after_help = "\x1b[1mExamples:\x1b[0m\n  \
                  scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2\n  \
                  scope address @main-wallet              \x1b[2m# address book shortcut\x1b[0m\n  \
                  scope contract 0xdAC17F958D2ee523a2206206994597C13D831ec7\n  \
                  scope crawl USDC --chain ethereum\n  \
                  scope insights 0xabc123...\n  \
                  scope monitor USDC\n  \
                  scope compliance risk 0x742d...\n  \
                  scope setup\n\n\
                  \x1b[2mTip: Use @label in any command to recall an address from the address book.\x1b[0m\n\n\
                  \x1b[1mDocumentation:\x1b[0m\n  \
                  https://github.com/robot-accomplice/scope-blockchain-analysis\n  \
                  Quickstart guide: docs/QUICKSTART.md"
)]
pub struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,

    /// Path to configuration file.
    ///
    /// Overrides the default location (~/.config/scope/config.yaml).
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Increase logging verbosity.
    ///
    /// Can be specified multiple times:
    /// -v    = INFO level
    /// -vv   = DEBUG level
    /// -vvv  = TRACE level
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Disable colored output.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Output markdown to console for agent parsing.
    ///
    /// Forces all commands to emit markdown-formatted output to stdout
    /// instead of tables or JSON. Useful for LLM/agent consumption.
    #[arg(long, global = true)]
    pub ai: bool,
}

/// Available CLI subcommands.
#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Commands {
    // -- Entity lookup --------------------------------------------------------
    /// Analyze a blockchain address.
    ///
    /// Retrieves balance, transaction history, and token holdings
    /// for the specified address.
    #[command(visible_alias = "addr")]
    Address(AddressArgs),

    /// Analyze a transaction.
    ///
    /// Decodes transaction data, traces execution, and displays
    /// detailed information about the transaction.
    #[command(visible_alias = "transaction")]
    Tx(TxArgs),

    /// Unified insights: infer chain and type, run relevant analyses.
    ///
    /// Provide any target (address, tx hash, or token) and Scope will
    /// detect it, run the appropriate analyses, and present observations.
    #[command(visible_alias = "insight")]
    Insights(insights::InsightsArgs),

    /// Analyze a smart contract.
    ///
    /// Retrieves source code, detects proxy patterns, maps access control,
    /// scans for vulnerabilities, checks DeFi patterns, and gathers external
    /// intelligence (GitHub links, audit reports).
    #[command(visible_alias = "ct")]
    Contract(contract::ContractArgs),

    // -- Token analysis -------------------------------------------------------
    /// Crawl a token for analytics data.
    ///
    /// Retrieves comprehensive token information including top holders,
    /// volume statistics, price data, and liquidity. Displays results
    /// with ASCII charts and can generate markdown reports.
    #[command(visible_alias = "token")]
    Crawl(CrawlArgs),

    /// Token health suite: DEX analytics + optional order book (crawl + market).
    ///
    /// Combines liquidity, volume, and holder data with optional market/peg
    /// summary for stablecoins. Use --with-market for order book data.
    #[command(visible_alias = "health")]
    TokenHealth(token_health::TokenHealthArgs),

    /// Discover trending and boosted tokens from DexScreener.
    ///
    /// Browse featured token profiles, recently boosted tokens,
    /// or top boosted tokens by activity.
    #[command(visible_alias = "disc")]
    Discover(discover::DiscoverArgs),

    /// Live token monitor with real-time TUI dashboard.
    ///
    /// Launches a terminal UI with price/volume charts, buy/sell gauges,
    /// transaction feed, and more. Shortcut for `scope interactive` + `monitor <token>`.
    #[command(visible_alias = "mon")]
    Monitor(MonitorArgs),

    /// Peg and order book health for stablecoin markets.
    ///
    /// Fetches level-2 depth from exchange APIs and reports
    /// peg deviation, spread, depth, and configurable health checks.
    #[command(subcommand)]
    Market(market::MarketCommands),

    /// Manage exchange venue descriptors.
    ///
    /// List available venues, view the YAML schema, initialise the
    /// user venues directory, or validate custom descriptor files.
    #[command(subcommand, visible_alias = "ven")]
    Venues(venues::VenuesCommands),

    // -- Compliance -----------------------------------------------------------
    /// Compliance and risk analysis commands.
    ///
    /// Assess risk, trace taint, detect patterns, and generate
    /// compliance reports for blockchain addresses.
    #[command(subcommand)]
    Compliance(compliance::ComplianceCommands),

    // -- Data & export --------------------------------------------------------
    /// Address book management commands.
    ///
    /// Add, remove, and list watched addresses (wallets, contracts, tokens).
    /// View aggregated balances across multiple chains.
    /// Use `@label` in any command to recall a saved address.
    #[command(visible_alias = "ab", alias = "portfolio", alias = "port")]
    AddressBook(AddressBookArgs),

    /// Export analysis data.
    ///
    /// Export transaction history, balances, or analysis results
    /// to various formats (JSON, CSV).
    Export(ExportArgs),

    /// Batch and combined report generation.
    #[command(subcommand)]
    Report(report::ReportCommands),

    // -- Config & interactive -------------------------------------------------
    /// Interactive mode with preserved context.
    ///
    /// Launch a REPL where chain, format, and other settings persist
    /// between commands for faster workflow.
    #[command(visible_alias = "shell")]
    Interactive(InteractiveArgs),

    /// Configure Scope settings and API keys.
    ///
    /// Run the setup wizard to configure API keys and preferences,
    /// or use --status to view current configuration.
    #[command(visible_alias = "config")]
    Setup(SetupArgs),

    /// Generate shell completions for bash, zsh, or fish.
    ///
    /// Output shell completion script to stdout for the specified shell.
    /// Source the output in your shell profile for tab completion.
    Completions(CompletionsArgs),

    /// Start the web UI server (localhost).
    ///
    /// Serves the same Scope features as the CLI via a local web interface
    /// at http://127.0.0.1:8080 (default). Use --daemon to run in the
    /// background, --stop to halt a running daemon.
    #[command(visible_alias = "serve")]
    Web(WebArgs),
}

impl Commands {
    /// Returns `true` for interactive/long-running commands that manage their
    /// own UI (TUI, REPL, web server, setup wizard, shell completions).
    /// Non-interactive commands return `false` and get a version header.
    pub fn is_interactive(&self) -> bool {
        matches!(
            self,
            Commands::Interactive(_)
                | Commands::Monitor(_)
                | Commands::Setup(_)
                | Commands::Completions(_)
                | Commands::Web(_)
        )
    }
}

/// Arguments for the web server command.
#[derive(Debug, Clone, clap::Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope web
  scope web --port 3000 --bind 0.0.0.0
  scope serve --daemon
  scope web --stop")]
pub struct WebArgs {
    /// Port to listen on.
    #[arg(long, short, default_value = "8080")]
    pub port: u16,

    /// Address to bind to.
    ///
    /// Use 127.0.0.1 for local-only (default) or 0.0.0.0 for LAN access.
    #[arg(long, default_value = "127.0.0.1")]
    pub bind: String,

    /// Run as a background daemon.
    #[arg(long, short)]
    pub daemon: bool,

    /// Stop a running daemon.
    #[arg(long)]
    pub stop: bool,
}

/// Arguments for the completions command.
#[derive(Debug, Clone, clap::Args)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope completions bash >> ~/.bashrc
  scope completions zsh > ~/.zfunc/_scope
  scope completions fish > ~/.config/fish/completions/scope.fish")]
pub struct CompletionsArgs {
    /// The shell to generate completions for.
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

impl Cli {
    /// Parses CLI arguments from the environment.
    ///
    /// This is a convenience wrapper around `clap::Parser::parse()`.
    pub fn parse_args() -> Self {
        Self::parse()
    }

    /// Returns the log level based on verbosity flag.
    ///
    /// Maps the `-v` count to tracing log levels:
    /// - 0: WARN (default)
    /// - 1: INFO
    /// - 2: DEBUG
    /// - 3+: TRACE
    pub fn log_level(&self) -> tracing::Level {
        match self.verbose {
            0 => tracing::Level::WARN,
            1 => tracing::Level::INFO,
            2 => tracing::Level::DEBUG,
            _ => tracing::Level::TRACE,
        }
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_cli_parse_address_command() {
        let cli = Cli::try_parse_from([
            "scope",
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ])
        .unwrap();

        assert!(matches!(cli.command, Commands::Address(_)));
        assert!(cli.config.is_none());
        assert_eq!(cli.verbose, 0);
    }

    #[test]
    fn test_cli_parse_address_alias() {
        let cli = Cli::try_parse_from([
            "scope",
            "addr",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ])
        .unwrap();

        assert!(matches!(cli.command, Commands::Address(_)));
    }

    #[test]
    fn test_cli_parse_tx_command() {
        let cli = Cli::try_parse_from([
            "scope",
            "tx",
            "0xabc123def456789012345678901234567890123456789012345678901234abcd",
        ])
        .unwrap();

        assert!(matches!(cli.command, Commands::Tx(_)));
    }

    #[test]
    fn test_cli_parse_tx_alias() {
        let cli = Cli::try_parse_from([
            "scope",
            "transaction",
            "0xabc123def456789012345678901234567890123456789012345678901234abcd",
        ])
        .unwrap();

        assert!(matches!(cli.command, Commands::Tx(_)));
    }

    #[test]
    fn test_cli_parse_address_book_command() {
        let cli = Cli::try_parse_from(["scope", "address-book", "list"]).unwrap();

        assert!(matches!(cli.command, Commands::AddressBook(_)));
    }

    #[test]
    fn test_cli_parse_address_book_portfolio_alias() {
        let cli = Cli::try_parse_from(["scope", "portfolio", "list"]).unwrap();

        assert!(matches!(cli.command, Commands::AddressBook(_)));
    }

    #[test]
    fn test_cli_parse_export_command() {
        let cli = Cli::try_parse_from([
            "scope",
            "export",
            "--address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "--output",
            "data.json",
        ])
        .unwrap();

        assert!(matches!(cli.command, Commands::Export(_)));
    }

    #[test]
    fn test_cli_parse_interactive_command() {
        let cli = Cli::try_parse_from(["scope", "interactive"]).unwrap();

        assert!(matches!(cli.command, Commands::Interactive(_)));
    }

    #[test]
    fn test_cli_parse_interactive_alias() {
        let cli = Cli::try_parse_from(["scope", "shell"]).unwrap();

        assert!(matches!(cli.command, Commands::Interactive(_)));
    }

    #[test]
    fn test_cli_parse_interactive_no_banner() {
        let cli = Cli::try_parse_from(["scope", "interactive", "--no-banner"]).unwrap();

        if let Commands::Interactive(args) = cli.command {
            assert!(args.no_banner);
        } else {
            panic!("Expected Interactive command");
        }
    }

    #[test]
    fn test_cli_verbose_flag_counting() {
        let cli = Cli::try_parse_from([
            "scope",
            "-vvv",
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ])
        .unwrap();

        assert_eq!(cli.verbose, 3);
    }

    #[test]
    fn test_cli_verbose_separate_flags() {
        let cli = Cli::try_parse_from([
            "scope",
            "-v",
            "-v",
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ])
        .unwrap();

        assert_eq!(cli.verbose, 2);
    }

    #[test]
    fn test_cli_global_config_option() {
        let cli = Cli::try_parse_from([
            "scope",
            "--config",
            "/custom/path.yaml",
            "tx",
            "0xabc123def456789012345678901234567890123456789012345678901234abcd",
        ])
        .unwrap();

        assert_eq!(cli.config, Some(PathBuf::from("/custom/path.yaml")));
    }

    #[test]
    fn test_cli_config_long_flag() {
        let cli = Cli::try_parse_from([
            "scope",
            "--config",
            "/custom/config.yaml",
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ])
        .unwrap();

        assert_eq!(cli.config, Some(PathBuf::from("/custom/config.yaml")));
    }

    #[test]
    fn test_cli_no_color_flag() {
        let cli = Cli::try_parse_from([
            "scope",
            "--no-color",
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ])
        .unwrap();

        assert!(cli.no_color);
    }

    #[test]
    fn test_cli_missing_required_args_fails() {
        let result = Cli::try_parse_from(["scope", "address"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_invalid_subcommand_fails() {
        let result = Cli::try_parse_from(["scope", "invalid"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_log_level_default() {
        let cli = Cli::try_parse_from([
            "scope",
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ])
        .unwrap();

        assert_eq!(cli.log_level(), tracing::Level::WARN);
    }

    #[test]
    fn test_cli_log_level_info() {
        let cli = Cli::try_parse_from([
            "scope",
            "-v",
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ])
        .unwrap();

        assert_eq!(cli.log_level(), tracing::Level::INFO);
    }

    #[test]
    fn test_cli_log_level_debug() {
        let cli = Cli::try_parse_from([
            "scope",
            "-vv",
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ])
        .unwrap();

        assert_eq!(cli.log_level(), tracing::Level::DEBUG);
    }

    #[test]
    fn test_cli_log_level_trace() {
        let cli = Cli::try_parse_from([
            "scope",
            "-vvvv",
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ])
        .unwrap();

        assert_eq!(cli.log_level(), tracing::Level::TRACE);
    }

    #[test]
    fn test_cli_debug_impl() {
        let cli = Cli::try_parse_from([
            "scope",
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ])
        .unwrap();

        let debug_str = format!("{:?}", cli);
        assert!(debug_str.contains("Cli"));
        assert!(debug_str.contains("Address"));
    }

    // ========================================================================
    // Monitor command tests
    // ========================================================================

    #[test]
    fn test_cli_parse_monitor_command() {
        let cli = Cli::try_parse_from([
            "scope",
            "monitor",
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
        ])
        .unwrap();

        assert!(matches!(cli.command, Commands::Monitor(_)));
    }

    #[test]
    fn test_cli_parse_monitor_alias_mon() {
        let cli = Cli::try_parse_from(["scope", "mon", "USDC"]).unwrap();

        assert!(matches!(cli.command, Commands::Monitor(_)));
        if let Commands::Monitor(args) = cli.command {
            assert_eq!(args.token, "USDC");
            assert_eq!(args.chain, "ethereum"); // default
            assert!(args.layout.is_none());
            assert!(args.refresh.is_none());
            assert!(args.scale.is_none());
            assert!(args.color_scheme.is_none());
            assert!(args.export.is_none());
        }
    }

    #[test]
    fn test_cli_parse_monitor_with_chain() {
        let cli = Cli::try_parse_from(["scope", "monitor", "USDC", "--chain", "solana"]).unwrap();

        if let Commands::Monitor(args) = cli.command {
            assert_eq!(args.token, "USDC");
            assert_eq!(args.chain, "solana");
        } else {
            panic!("Expected Monitor command");
        }
    }

    #[test]
    fn test_cli_parse_monitor_chain_short_flag() {
        let cli = Cli::try_parse_from(["scope", "monitor", "PEPE", "-c", "ethereum"]).unwrap();

        if let Commands::Monitor(args) = cli.command {
            assert_eq!(args.token, "PEPE");
            assert_eq!(args.chain, "ethereum");
        } else {
            panic!("Expected Monitor command");
        }
    }

    #[test]
    fn test_cli_parse_monitor_with_layout() {
        let cli =
            Cli::try_parse_from(["scope", "monitor", "USDC", "--layout", "chart-focus"]).unwrap();

        if let Commands::Monitor(args) = cli.command {
            assert_eq!(args.layout, Some(monitor::LayoutPreset::ChartFocus));
        } else {
            panic!("Expected Monitor command");
        }
    }

    #[test]
    fn test_cli_parse_monitor_with_refresh() {
        let cli = Cli::try_parse_from(["scope", "monitor", "USDC", "--refresh", "3"]).unwrap();

        if let Commands::Monitor(args) = cli.command {
            assert_eq!(args.refresh, Some(3));
        } else {
            panic!("Expected Monitor command");
        }
    }

    #[test]
    fn test_cli_parse_monitor_with_scale() {
        let cli = Cli::try_parse_from(["scope", "monitor", "USDC", "--scale", "log"]).unwrap();

        if let Commands::Monitor(args) = cli.command {
            assert_eq!(args.scale, Some(monitor::ScaleMode::Log));
        } else {
            panic!("Expected Monitor command");
        }
    }

    #[test]
    fn test_cli_parse_monitor_with_color_scheme() {
        let cli =
            Cli::try_parse_from(["scope", "monitor", "USDC", "--color-scheme", "blue-orange"])
                .unwrap();

        if let Commands::Monitor(args) = cli.command {
            assert_eq!(args.color_scheme, Some(monitor::ColorScheme::BlueOrange));
        } else {
            panic!("Expected Monitor command");
        }
    }

    #[test]
    fn test_cli_parse_monitor_with_export() {
        let cli =
            Cli::try_parse_from(["scope", "monitor", "USDC", "--export", "/tmp/out.csv"]).unwrap();

        if let Commands::Monitor(args) = cli.command {
            assert_eq!(args.export, Some(std::path::PathBuf::from("/tmp/out.csv")));
        } else {
            panic!("Expected Monitor command");
        }
    }

    #[test]
    fn test_cli_parse_monitor_short_flags() {
        let cli = Cli::try_parse_from([
            "scope", "mon", "USDC", "-c", "solana", "-l", "compact", "-r", "10", "-s", "log", "-e",
            "data.csv",
        ])
        .unwrap();

        if let Commands::Monitor(args) = cli.command {
            assert_eq!(args.token, "USDC");
            assert_eq!(args.chain, "solana");
            assert_eq!(args.layout, Some(monitor::LayoutPreset::Compact));
            assert_eq!(args.refresh, Some(10));
            assert_eq!(args.scale, Some(monitor::ScaleMode::Log));
            assert_eq!(args.export, Some(std::path::PathBuf::from("data.csv")));
        } else {
            panic!("Expected Monitor command");
        }
    }

    #[test]
    fn test_cli_parse_monitor_missing_token_fails() {
        let result = Cli::try_parse_from(["scope", "monitor"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_parse_monitor_invalid_layout_fails() {
        let result = Cli::try_parse_from(["scope", "monitor", "USDC", "--layout", "invalid"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_parse_monitor_invalid_scale_fails() {
        let result = Cli::try_parse_from(["scope", "monitor", "USDC", "--scale", "quadratic"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_parse_market_summary() {
        let cli = Cli::try_parse_from(["scope", "market", "summary"]).unwrap();
        assert!(matches!(cli.command, Commands::Market(_)));
    }

    #[test]
    fn test_cli_parse_market_summary_with_pair() {
        let cli = Cli::try_parse_from(["scope", "market", "summary", "DAI_USDT"]).unwrap();
        if let Commands::Market(market::MarketCommands::Summary(args)) = cli.command {
            assert_eq!(args.pair, "DAI_USDT");
        } else {
            panic!("Expected Market Summary command");
        }
    }

    #[test]
    fn test_cli_parse_report_batch() {
        let cli = Cli::try_parse_from([
            "scope",
            "report",
            "batch",
            "--addresses",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "--output",
            "report.md",
        ])
        .unwrap();
        assert!(matches!(cli.command, Commands::Report(_)));
    }

    #[test]
    fn test_cli_parse_market_summary_with_thresholds() {
        let cli = Cli::try_parse_from([
            "scope",
            "market",
            "summary",
            "--peg-range",
            "0.002",
            "--min-bid-ask-ratio",
            "0.1",
            "--max-bid-ask-ratio",
            "10",
        ])
        .unwrap();
        if let Commands::Market(market::MarketCommands::Summary(args)) = cli.command {
            assert_eq!(args.peg_range, 0.002);
            assert_eq!(args.min_bid_ask_ratio, 0.1);
            assert_eq!(args.max_bid_ask_ratio, 10.0);
        } else {
            panic!("Expected Market Summary command");
        }
    }

    #[test]
    fn test_cli_parse_token_health() {
        let cli = Cli::try_parse_from(["scope", "token-health", "USDC"]).unwrap();
        if let Commands::TokenHealth(args) = cli.command {
            assert_eq!(args.token, "USDC");
            assert!(!args.with_market);
        } else {
            panic!("Expected TokenHealth command");
        }
    }

    #[test]
    fn test_cli_parse_token_health_alias() {
        let cli = Cli::try_parse_from(["scope", "health", "USDC", "--with-market"]).unwrap();
        if let Commands::TokenHealth(args) = cli.command {
            assert_eq!(args.token, "USDC");
            assert!(args.with_market);
            assert_eq!(args.venue, "binance"); // default venue
        } else {
            panic!("Expected TokenHealth command");
        }
    }

    #[test]
    fn test_cli_parse_token_health_venue_biconomy() {
        let cli = Cli::try_parse_from([
            "scope",
            "token-health",
            "USDC",
            "--with-market",
            "--venue",
            "biconomy",
        ])
        .unwrap();
        if let Commands::TokenHealth(args) = cli.command {
            assert_eq!(args.venue, "biconomy");
        } else {
            panic!("Expected TokenHealth command");
        }
    }

    #[test]
    fn test_cli_parse_token_health_venue_eth() {
        let cli = Cli::try_parse_from([
            "scope",
            "token-health",
            "USDC",
            "--with-market",
            "--venue",
            "eth",
        ])
        .unwrap();
        if let Commands::TokenHealth(args) = cli.command {
            assert_eq!(args.venue, "eth");
        } else {
            panic!("Expected TokenHealth command");
        }
    }

    #[test]
    fn test_cli_parse_token_health_venue_solana() {
        let cli = Cli::try_parse_from([
            "scope",
            "token-health",
            "USDC",
            "--with-market",
            "--venue",
            "solana",
        ])
        .unwrap();
        if let Commands::TokenHealth(args) = cli.command {
            assert_eq!(args.venue, "solana");
        } else {
            panic!("Expected TokenHealth command");
        }
    }

    #[test]
    fn test_cli_parse_ai_flag() {
        let cli = Cli::try_parse_from([
            "scope",
            "--ai",
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ])
        .unwrap();
        assert!(cli.ai);
    }
}
