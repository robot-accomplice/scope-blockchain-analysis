//! # Scope Blockchain Analysis
//!
//! Entry point for the blockchain analysis command-line tool.
//!
//! This binary provides commands for:
//!
//! **Entity lookup:**
//! - Address analysis (`scope address` / `addr`) with `--report` and `--dossier`
//! - Transaction analysis (`scope tx` / `transaction`)
//! - Unified insights (`scope insights` / `insight`) — auto-detects target type
//!
//! **Token analysis:**
//! - Token crawling (`scope crawl` / `token`) with report generation
//! - Token health suite (`scope token-health` / `health`) — DEX + optional market
//! - Token discovery (`scope discover` / `disc`) — trending/boosted from DexScreener
//! - Live monitoring (`scope monitor` / `mon`) — real-time TUI dashboard
//! - Market peg/order book health (`scope market summary`)
//!
//! **Compliance:**
//! - Risk, trace, analyze, and compliance-report (`scope compliance`)
//!
//! **Data & export:**
//! - Portfolio management (`scope portfolio` / `port`)
//! - Data export (`scope export`)
//! - Batch reporting (`scope report batch`)
//!
//! **Config & interactive:**
//! - Interactive mode (`scope interactive` / `shell`)
//! - Setup wizard (`scope setup` / `config`)
//! - Shell completions (`scope completions bash|zsh|fish`)
//!
//! ## UX Features
//!
//! - **Progress indicators** — Spinners and progress bars for long-running operations
//! - **Error remediation hints** — Actionable suggestions for common errors
//! - **Typo suggestions** — "Did you mean?" for misspelled commands
//! - **Shell completion** — Tab-completion for bash, zsh, and fish
//! - **Help with examples** — Example invocations in `--help` for every command
//! - Global `--ai` flag forces markdown output for agent/LLM parsing
//!
//! ## Usage
//!
//! ```bash
//! scope --help
//! scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2
//! scope tx 0xabc123...
//! scope insights 0xabc123...
//! scope discover --source boosts --chain ethereum
//! scope market summary USDC --format json
//! scope token-health USDC --with-market
//! scope monitor USDC --chain ethereum
//! scope portfolio list
//! scope report batch --addresses 0x... --output report.md --with-risk
//! scope completions zsh > ~/.zfunc/_scope
//! ```

use anyhow::Result;
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use scope::Config;
use scope::chains::DefaultClientFactory;
use scope::cli::{Cli, Commands};
use scope::config::OutputFormat;
use scope::error::ScopeError;
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

    tracing::debug!("Scope v{} starting", scope::VERSION);

    // Check if this is a setup command (don't prompt for setup if already running setup)
    let is_setup_command = matches!(cli.command, Commands::Setup(_));

    // Load configuration
    let mut config = Config::load(cli.config.as_deref()).unwrap_or_else(|e| {
        tracing::warn!("Failed to load config: {}, using defaults", e);
        Config::default()
    });

    // --ai forces markdown output to console for agent parsing
    if cli.ai {
        config.output.format = OutputFormat::Markdown;
    }

    // Handle web command early (before setup check and factory creation)
    if let Commands::Web(ref web_args) = cli.command {
        let addr: std::net::SocketAddr = format!("{}:{}", web_args.bind, web_args.port)
            .parse()
            .unwrap_or_else(|_| {
                eprintln!("Invalid bind address: {}:{}", web_args.bind, web_args.port);
                std::process::exit(1);
            });

        if web_args.stop {
            if let Err(e) = scope::web::stop_daemon() {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
            return Ok(());
        }

        // If running as daemon child, skip daemon forking
        if scope::web::is_daemon_child() || !web_args.daemon {
            return scope::web::start_server(addr, config)
                .await
                .map_err(|e| anyhow::anyhow!(e));
        }

        // Fork daemon
        if let Err(e) = scope::web::start_daemon(addr, config) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return Ok(());
    }

    // Check if config file exists and prompt for setup if needed
    if !is_setup_command && !config_file_exists(&cli) && prompt_for_setup() {
        // Run setup wizard
        let setup_args = scope::cli::setup::SetupArgs {
            status: false,
            key: None,
            reset: false,
        };
        if let Err(e) = scope::cli::setup::run(setup_args, &config).await {
            eprintln!("Setup failed: {}", e);
        }
        // Reload config after setup
        let mut config = Config::load(cli.config.as_deref()).unwrap_or_default();
        if cli.ai {
            config.output.format = OutputFormat::Markdown;
        }
        return run_command(cli.command, &config).await;
    }

    // Create the client factory for dependency injection
    let factory = DefaultClientFactory {
        chains_config: config.chains.clone(),
    };

    // Dispatch to command handler
    let result = match cli.command {
        Commands::Completions(args) => {
            let mut cmd = Cli::command();
            generate(args.shell, &mut cmd, "scope", &mut io::stdout());
            Ok(())
        }
        Commands::Address(args) => scope::cli::address::run(args, &config, &factory).await,
        Commands::Tx(args) => scope::cli::tx::run(args, &config, &factory).await,
        Commands::Crawl(args) => scope::cli::crawl::run(args, &config, &factory).await,
        Commands::Portfolio(args) => scope::cli::portfolio::run(args, &config, &factory).await,
        Commands::Export(args) => scope::cli::export::run(args, &config, &factory).await,
        Commands::Interactive(args) => scope::cli::interactive::run(args, &config, &factory).await,
        Commands::Monitor(args) => scope::cli::monitor::run_direct(args, &config, &factory).await,
        Commands::Setup(args) => scope::cli::setup::run(args, &config).await,
        Commands::Compliance(compliance_cmd) => match compliance_cmd {
            scope::cli::compliance::ComplianceCommands::Risk(args) => {
                scope::cli::compliance::handle_risk(args)
                    .await
                    .map_err(|e| scope::error::ScopeError::Other(e.to_string()))
            }
            scope::cli::compliance::ComplianceCommands::Trace(args) => {
                scope::cli::compliance::handle_trace(args)
                    .await
                    .map_err(|e| scope::error::ScopeError::Other(e.to_string()))
            }
            scope::cli::compliance::ComplianceCommands::Analyze(args) => {
                scope::cli::compliance::handle_analyze(args)
                    .await
                    .map_err(|e| scope::error::ScopeError::Other(e.to_string()))
            }
            scope::cli::compliance::ComplianceCommands::ComplianceReport(args) => {
                scope::cli::compliance::handle_compliance_report(args)
                    .await
                    .map_err(|e| scope::error::ScopeError::Other(e.to_string()))
            }
        },
        Commands::Market(cmd) => scope::cli::market::run(cmd, &config, &factory).await,
        Commands::TokenHealth(args) => scope::cli::token_health::run(args, &config, &factory).await,
        Commands::Report(cmd) => scope::cli::report::run(cmd, &config, &factory).await,
        Commands::Discover(args) => scope::cli::discover::run(args, config.output.format)
            .await
            .map_err(|e| scope::error::ScopeError::Other(e.to_string())),
        Commands::Insights(args) => scope::cli::insights::run(args, &config, &factory).await,
        // Web command is handled above before factory creation
        Commands::Web(_) => unreachable!("Web command handled before dispatch"),
    };

    // Handle errors gracefully with remediation hints
    if let Err(e) = result {
        tracing::error!("{}", e);
        display_error(&e);
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
    eprintln!("Welcome to Scope Blockchain Analysis!");
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
        Commands::Completions(args) => {
            let mut cmd = Cli::command();
            generate(args.shell, &mut cmd, "scope", &mut io::stdout());
            Ok(())
        }
        Commands::Address(args) => scope::cli::address::run(args, config, &factory).await,
        Commands::Tx(args) => scope::cli::tx::run(args, config, &factory).await,
        Commands::Crawl(args) => scope::cli::crawl::run(args, config, &factory).await,
        Commands::Portfolio(args) => scope::cli::portfolio::run(args, config, &factory).await,
        Commands::Export(args) => scope::cli::export::run(args, config, &factory).await,
        Commands::Interactive(args) => scope::cli::interactive::run(args, config, &factory).await,
        Commands::Monitor(args) => scope::cli::monitor::run_direct(args, config, &factory).await,
        Commands::Setup(args) => scope::cli::setup::run(args, config).await,
        Commands::Compliance(compliance_cmd) => match compliance_cmd {
            scope::cli::compliance::ComplianceCommands::Risk(args) => {
                scope::cli::compliance::handle_risk(args)
                    .await
                    .map_err(|e| scope::error::ScopeError::Other(e.to_string()))
            }
            scope::cli::compliance::ComplianceCommands::Trace(args) => {
                scope::cli::compliance::handle_trace(args)
                    .await
                    .map_err(|e| scope::error::ScopeError::Other(e.to_string()))
            }
            scope::cli::compliance::ComplianceCommands::Analyze(args) => {
                scope::cli::compliance::handle_analyze(args)
                    .await
                    .map_err(|e| scope::error::ScopeError::Other(e.to_string()))
            }
            scope::cli::compliance::ComplianceCommands::ComplianceReport(args) => {
                scope::cli::compliance::handle_compliance_report(args)
                    .await
                    .map_err(|e| scope::error::ScopeError::Other(e.to_string()))
            }
        },
        Commands::Market(cmd) => scope::cli::market::run(cmd, config, &factory).await,
        Commands::TokenHealth(args) => scope::cli::token_health::run(args, config, &factory).await,
        Commands::Report(cmd) => scope::cli::report::run(cmd, config, &factory).await,
        Commands::Discover(args) => scope::cli::discover::run(args, config.output.format)
            .await
            .map_err(|e| scope::error::ScopeError::Other(e.to_string())),
        Commands::Insights(args) => scope::cli::insights::run(args, config, &factory).await,
        // Web command is handled in main() before factory creation
        Commands::Web(_) => unreachable!("Web command handled before dispatch"),
    };

    if let Err(e) = result {
        tracing::error!("{}", e);
        display_error(&e);
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
        .unwrap_or_else(|_| EnvFilter::new(format!("scope={},warn", level)));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_thread_ids(false)
        .init();
}

/// Displays an error with a remediation hint when available.
///
/// Maps common error types to actionable suggestions so users
/// know how to recover without consulting documentation.
fn display_error(e: &ScopeError) {
    eprintln!("Error: {}", e);

    if let Some(hint) = error_suggestion(e) {
        eprintln!();
        eprintln!("Hint: {}", hint);
    }
}

/// Returns a user-facing suggestion for common error types.
fn error_suggestion(e: &ScopeError) -> Option<&'static str> {
    match e {
        ScopeError::InvalidAddress(_) => Some(
            "Ensure the address format matches the target chain.\n      \
             EVM: 0x followed by 40 hex characters\n      \
             Solana: base58 encoded public key\n      \
             Tron: T followed by base58 characters",
        ),
        ScopeError::InvalidHash(_) => Some(
            "Ensure the transaction hash matches the target chain.\n      \
             EVM: 0x followed by 64 hex characters\n      \
             Solana: base58 encoded signature",
        ),
        ScopeError::Config(_) => Some("Run `scope setup` to create or repair your configuration."),
        ScopeError::Request(_) | ScopeError::Network(_) => Some(
            "Check your network connection and try again.\n      \
             Use -v for more details on the failing request.",
        ),
        ScopeError::Api(msg)
            if msg.contains("401") || msg.contains("403") || msg.contains("key") =>
        {
            Some(
                "Your API key may be missing or invalid.\n      Run `scope setup --key <provider>` to configure it.",
            )
        }
        ScopeError::NotFound(_) => Some(
            "The resource was not found. Verify the address, hash, or token exists on the specified chain.",
        ),
        _ => None,
    }
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
            "scope",
            "address",
            "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
        ]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cli_help_flag() {
        // Help should cause an error (it exits)
        let result = Cli::try_parse_from(["scope", "--help"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_version_flag() {
        // Version should cause an error (it exits)
        let result = Cli::try_parse_from(["scope", "--version"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_error_suggestion_invalid_address() {
        let err = ScopeError::InvalidAddress("bad".into());
        let hint = error_suggestion(&err);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("EVM"));
    }

    #[test]
    fn test_error_suggestion_invalid_hash() {
        let err = ScopeError::InvalidHash("bad".into());
        let hint = error_suggestion(&err);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("64 hex"));
    }

    #[test]
    fn test_error_suggestion_config() {
        use std::path::PathBuf;
        let err = ScopeError::Config(scope::error::ConfigError::NotFound {
            path: PathBuf::from("/missing"),
        });
        let hint = error_suggestion(&err);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("scope setup"));
    }

    #[test]
    fn test_error_suggestion_network() {
        let err = ScopeError::Network("timeout".into());
        let hint = error_suggestion(&err);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("network"));
    }

    #[test]
    fn test_error_suggestion_api_auth() {
        let err = ScopeError::Api("401 Unauthorized".into());
        let hint = error_suggestion(&err);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("API key"));
    }

    #[test]
    fn test_error_suggestion_not_found() {
        let err = ScopeError::NotFound("address".into());
        let hint = error_suggestion(&err);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("not found"));
    }

    #[test]
    fn test_error_suggestion_other_returns_none() {
        let err = ScopeError::Other("random".into());
        assert!(error_suggestion(&err).is_none());
    }

    #[test]
    fn test_cli_completions_parsing() {
        let result = Cli::try_parse_from(["scope", "completions", "bash"]);
        assert!(result.is_ok());
    }
}
