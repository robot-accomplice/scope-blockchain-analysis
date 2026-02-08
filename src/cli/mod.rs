//! # CLI Module
//!
//! This module defines the command-line interface using `clap` with derive macros.
//! It provides the main `Cli` struct and `Commands` enum that define all
//! available commands and their arguments.
//!
//! ## Command Structure
//!
//! ```text
//! scope [OPTIONS] <COMMAND>
//!
//! Commands:
//!   address      Analyze a blockchain address
//!   tx           Analyze a transaction
//!   portfolio    Portfolio management commands
//!   export       Export analysis data
//!   interactive  Interactive mode with preserved context
//!
//! Options:
//!   --config <PATH>   Path to configuration file
//!   -v, --verbose...  Increase logging verbosity
//!   -h, --help        Print help
//!   -V, --version     Print version
//! ```

pub mod address;
pub mod compliance;
pub mod crawl;
pub mod export;
pub mod interactive;
pub mod monitor;
pub mod portfolio;
pub mod setup;
pub mod tx;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

pub use address::AddressArgs;
pub use crawl::CrawlArgs;
pub use export::ExportArgs;
pub use interactive::InteractiveArgs;
pub use portfolio::PortfolioArgs;
pub use setup::SetupArgs;
pub use tx::TxArgs;

/// Blockchain Analysis CLI - A tool for blockchain data analysis.
///
/// Scope provides comprehensive blockchain analysis capabilities including
/// address investigation, transaction decoding, portfolio tracking, and
/// data export functionality.
#[derive(Debug, Parser)]
#[command(
    name = "scope",
    version,
    about = "Scope Blockchain Analysis - A tool for blockchain data analysis",
    long_about = "Scope Blockchain Analysis is a production-grade tool for \
                  blockchain data analysis, portfolio tracking, and transaction investigation.\n\n\
                  Use --help with any subcommand for detailed usage information."
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
}

/// Available CLI subcommands.
#[derive(Debug, Subcommand)]
pub enum Commands {
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

    /// Crawl a token for analytics data.
    ///
    /// Retrieves comprehensive token information including top holders,
    /// volume statistics, price data, and liquidity. Displays results
    /// with ASCII charts and can generate markdown reports.
    #[command(visible_alias = "token")]
    Crawl(CrawlArgs),

    /// Portfolio management commands.
    ///
    /// Add, remove, and list watched addresses. View aggregated
    /// portfolio balances across multiple chains.
    #[command(visible_alias = "port")]
    Portfolio(PortfolioArgs),

    /// Export analysis data.
    ///
    /// Export transaction history, balances, or analysis results
    /// to various formats (JSON, CSV).
    Export(ExportArgs),

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

    /// Compliance and risk analysis commands.
    ///
    /// Assess risk, trace taint, detect patterns, and generate
    /// compliance reports for blockchain addresses.
    #[command(subcommand)]
    Compliance(compliance::ComplianceCommands),
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
    fn test_cli_parse_portfolio_command() {
        let cli = Cli::try_parse_from(["scope", "portfolio", "list"]).unwrap();

        assert!(matches!(cli.command, Commands::Portfolio(_)));
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
}
