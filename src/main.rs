//! # BCC - Blockchain Crawler CLI
//!
//! Entry point for the blockchain analysis command-line tool.
//!
//! This binary provides commands for:
//! - Address analysis (`bcc address`)
//! - Transaction analysis (`bcc tx`)
//! - Portfolio management (`bcc portfolio`)
//! - Data export (`bcc export`)
//!
//! ## Usage
//!
//! ```bash
//! bcc --help
//! bcc address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2
//! bcc tx 0xabc123...
//! bcc portfolio list
//! ```

use anyhow::Result;
use bcc::Config;
use bcc::chains::DefaultClientFactory;
use bcc::cli::{Cli, Commands};
use clap::Parser;
use std::io::{self, Write};
use tracing_subscriber::EnvFilter;

/// ASCII art banner featuring a Portia jumping spider.
/// Loaded from `assets/banner.txt` at compile time.
const BANNER: &str = include_str!("../assets/banner.txt");

/// Prints the startup banner to stderr.
fn print_banner() {
    eprintln!("{}", BANNER);
}

/// Application entry point.
///
/// Initializes logging, parses CLI arguments, loads configuration,
/// and dispatches to the appropriate command handler.
#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Initialize logging based on verbosity
    init_logging(cli.verbose);

    // Show banner in verbose mode
    if cli.verbose > 0 {
        print_banner();
    }

    tracing::debug!("BCC v{} starting", bcc::VERSION);

    // Check if this is a setup command (don't prompt for setup if already running setup)
    let is_setup_command = matches!(cli.command, Commands::Setup(_));

    // Load configuration
    let config = Config::load(cli.config.as_deref()).unwrap_or_else(|e| {
        tracing::warn!("Failed to load config: {}, using defaults", e);
        Config::default()
    });

    // Check if config file exists and prompt for setup if needed
    if !is_setup_command && !config_file_exists(&cli) && prompt_for_setup() {
        // Run setup wizard
        let setup_args = bcc::cli::setup::SetupArgs {
            status: false,
            key: None,
            reset: false,
        };
        if let Err(e) = bcc::cli::setup::run(setup_args, &config).await {
            eprintln!("Setup failed: {}", e);
        }
        // Reload config after setup
        let config = Config::load(cli.config.as_deref()).unwrap_or_default();
        return run_command(cli.command, &config).await;
    }

    // Create the client factory for dependency injection
    let factory = DefaultClientFactory {
        chains_config: config.chains.clone(),
    };

    // Dispatch to command handler
    let result = match cli.command {
        Commands::Address(args) => bcc::cli::address::run(args, &config, &factory).await,
        Commands::Tx(args) => bcc::cli::tx::run(args, &config, &factory).await,
        Commands::Crawl(args) => bcc::cli::crawl::run(args, &config, &factory).await,
        Commands::Portfolio(args) => bcc::cli::portfolio::run(args, &config, &factory).await,
        Commands::Export(args) => bcc::cli::export::run(args, &config, &factory).await,
        Commands::Interactive(args) => bcc::cli::interactive::run(args, &config, &factory).await,
        Commands::Setup(args) => bcc::cli::setup::run(args, &config).await,
        Commands::Compliance(compliance_cmd) => match compliance_cmd {
            bcc::cli::compliance::ComplianceCommands::Risk(args) => {
                bcc::cli::compliance::handle_risk(args)
                    .await
                    .map_err(|e| bcc::error::BccError::Other(e.to_string()))
            }
            bcc::cli::compliance::ComplianceCommands::Trace(args) => {
                bcc::cli::compliance::handle_trace(args)
                    .await
                    .map_err(|e| bcc::error::BccError::Other(e.to_string()))
            }
            bcc::cli::compliance::ComplianceCommands::Analyze(args) => {
                bcc::cli::compliance::handle_analyze(args)
                    .await
                    .map_err(|e| bcc::error::BccError::Other(e.to_string()))
            }
            bcc::cli::compliance::ComplianceCommands::ComplianceReport(args) => {
                bcc::cli::compliance::handle_compliance_report(args)
                    .await
                    .map_err(|e| bcc::error::BccError::Other(e.to_string()))
            }
        },
    };

    // Handle errors gracefully
    if let Err(e) = result {
        tracing::error!("{}", e);
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    Ok(())
}

/// Checks if a configuration file exists.
fn config_file_exists(cli: &Cli) -> bool {
    if let Some(ref path) = cli.config {
        return path.exists();
    }
    Config::config_path().map(|p| p.exists()).unwrap_or(false)
}

/// Prompts the user to run setup.
fn prompt_for_setup() -> bool {
    eprintln!();
    eprintln!("Welcome to BCC (Blockchain Crawler CLI)!");
    eprintln!();
    eprintln!("No configuration file found. Would you like to run the setup wizard");
    eprintln!("to configure API keys and preferences?");
    eprintln!();
    eprint!("Run setup now? [Y/n]: ");
    io::stderr().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }

    let response = input.trim().to_lowercase();
    // Default to yes if user just presses Enter
    response.is_empty() || response == "y" || response == "yes"
}

/// Runs a command with the given config.
async fn run_command(command: Commands, config: &Config) -> Result<()> {
    let factory = DefaultClientFactory {
        chains_config: config.chains.clone(),
    };
    let result = match command {
        Commands::Address(args) => bcc::cli::address::run(args, config, &factory).await,
        Commands::Tx(args) => bcc::cli::tx::run(args, config, &factory).await,
        Commands::Crawl(args) => bcc::cli::crawl::run(args, config, &factory).await,
        Commands::Portfolio(args) => bcc::cli::portfolio::run(args, config, &factory).await,
        Commands::Export(args) => bcc::cli::export::run(args, config, &factory).await,
        Commands::Interactive(args) => bcc::cli::interactive::run(args, config, &factory).await,
        Commands::Setup(args) => bcc::cli::setup::run(args, config).await,
        Commands::Compliance(compliance_cmd) => match compliance_cmd {
            bcc::cli::compliance::ComplianceCommands::Risk(args) => {
                bcc::cli::compliance::handle_risk(args)
                    .await
                    .map_err(|e| bcc::error::BccError::Other(e.to_string()))
            }
            bcc::cli::compliance::ComplianceCommands::Trace(args) => {
                bcc::cli::compliance::handle_trace(args)
                    .await
                    .map_err(|e| bcc::error::BccError::Other(e.to_string()))
            }
            bcc::cli::compliance::ComplianceCommands::Analyze(args) => {
                bcc::cli::compliance::handle_analyze(args)
                    .await
                    .map_err(|e| bcc::error::BccError::Other(e.to_string()))
            }
            bcc::cli::compliance::ComplianceCommands::ComplianceReport(args) => {
                bcc::cli::compliance::handle_compliance_report(args)
                    .await
                    .map_err(|e| bcc::error::BccError::Other(e.to_string()))
            }
        },
    };

    if let Err(e) = result {
        tracing::error!("{}", e);
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    Ok(())
}

/// Initializes the tracing subscriber for logging.
///
/// Configures log level based on the verbosity flag:
/// - 0: WARN (default, minimal output)
/// - 1: INFO (general information)
/// - 2: DEBUG (detailed debugging)
/// - 3+: TRACE (very verbose, all details)
///
/// # Arguments
///
/// * `verbosity` - The number of `-v` flags provided
fn init_logging(verbosity: u8) {
    let level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    // Allow RUST_LOG to override if set
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("bcc={},warn", level)));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_thread_ids(false)
        .init();
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verbosity_levels() {
        // Just verify the mapping logic
        assert_eq!(
            match 0u8 {
                0 => "warn",
                1 => "info",
                2 => "debug",
                _ => "trace",
            },
            "warn"
        );
        assert_eq!(
            match 1u8 {
                0 => "warn",
                1 => "info",
                2 => "debug",
                _ => "trace",
            },
            "info"
        );
        assert_eq!(
            match 2u8 {
                0 => "warn",
                1 => "info",
                2 => "debug",
                _ => "trace",
            },
            "debug"
        );
        assert_eq!(
            match 3u8 {
                0 => "warn",
                1 => "info",
                2 => "debug",
                _ => "trace",
            },
            "trace"
        );
    }

    #[test]
    fn test_cli_parsing() {
        // Verify CLI can be parsed (basic smoke test)
        let result = Cli::try_parse_from([
            "bcc",
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cli_help_flag() {
        // Help should cause an error (it exits)
        let result = Cli::try_parse_from(["bcc", "--help"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_version_flag() {
        // Version should cause an error (it exits)
        let result = Cli::try_parse_from(["bcc", "--version"]);
        assert!(result.is_err());
    }
}
