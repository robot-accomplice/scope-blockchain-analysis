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
//! - Address book management (`scope address-book` / `ab`, alias: `portfolio` / `port`)
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
//! scope address-book list
//! scope report batch --addresses 0x... --output report.md --with-risk
//! scope completions zsh > ~/.zfunc/_scope
//! ```

use anyhow::Result;
use clap::Parser;
use scope::Config;
use scope::config::OutputFormat;
use scope::error::ScopeError;
use scope_cli::cli::errors::display_error;
use scope_cli::cli::output::Output;
use scope_cli::cli::prompt;
use scope_cli::cli::{Cli, Commands};
use scope_cli::note;
use std::io::{self, IsTerminal};
use tracing_subscriber::EnvFilter;

/// ASCII art banner featuring a Portia jumping spider.
/// Loaded from `assets/banner.txt` at compile time.
const BANNER: &str = include_str!("../assets/banner.txt");

/// Reports `message` as a failed command and exits with `code`.
fn fail(out: &Output, message: String, code: i32) -> ! {
    // With stderr gone there is no channel left; the code still tells.
    let _ = display_error(&ScopeError::Other(message), out);
    std::process::exit(code);
}

/// Application entry point.
///
/// Initializes logging, parses CLI arguments, loads configuration,
/// and dispatches to the appropriate command handler.
#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let cli = Cli::parse();
    let out = Output::stdio();

    // Initialize logging based on verbosity
    init_logging(cli.verbose);

    if cli.no_color {
        scope::display::terminal::set_color_override(Some(false));
    }

    // The TUI needs a terminal. Check before the config load and the setup
    // prompt, which read stdin.
    if matches!(cli.command, Commands::Interactive(_)) {
        let term = std::env::var("TERM").ok();
        if let Err(message) = scope_cli::cli::tui::tui_terminal_ok(
            io::stdin().is_terminal(),
            io::stdout().is_terminal(),
            term.as_deref(),
        ) {
            fail(&out, message, 2);
        }
    }

    // Show banner in verbose mode
    if cli.verbose > 0 {
        note!(out, "{}", BANNER);
    }

    tracing::debug!("Scope v{} starting", scope::VERSION);

    // Check if this is a setup command (don't prompt for setup if already running setup)
    let is_setup_command = matches!(cli.command, Commands::Setup(_));

    // Load configuration
    let mut config = Config::load(cli.config.as_deref()).unwrap_or_else(|e| {
        note!(
            out,
            "⚠ Could not load config, using defaults (use -v for details)"
        );
        tracing::debug!("Failed to load config: {}", e);
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
                let message = format!("Invalid bind address: {}:{}", web_args.bind, web_args.port);
                fail(&out, message, 1)
            });

        if web_args.stop {
            if let Err(e) = scope_web::stop_daemon(&out) {
                let _ = display_error(&e, &out);
                std::process::exit(1);
            }
            return Ok(());
        }

        // If running as daemon child, skip daemon forking
        if scope_web::is_daemon_child() || !web_args.daemon {
            return scope_web::start_server(addr, config, &out)
                .await
                .map_err(|e| anyhow::anyhow!(e));
        }

        // Fork daemon
        if let Err(e) = scope_web::start_daemon(addr, config, &out) {
            let _ = display_error(&e, &out);
            std::process::exit(1);
        }
        return Ok(());
    }

    // Check if config file exists and prompt for setup if needed
    if !is_setup_command && !config_file_exists(&cli) && prompt_for_setup(&out) {
        // Run setup wizard
        let setup_args = scope_cli::cli::setup::SetupArgs {
            status: false,
            key: None,
            reset: false,
        };
        if let Err(e) = scope_cli::cli::setup::run(setup_args, &config, &out).await {
            let _ = display_error(&e, &out);
        }
        // Reload config after setup
        let mut config = Config::load(cli.config.as_deref()).unwrap_or_default();
        if cli.ai {
            config.output.format = OutputFormat::Markdown;
        }
        return dispatch(cli.command, &config, &out).await;
    }

    // Dispatch the command through the unified handler
    dispatch(cli.command, &config, &out).await
}

/// Checks if a configuration file exists.
fn config_file_exists(cli: &Cli) -> bool {
    if let Some(ref path) = cli.config {
        return path.exists();
    }
    Config::config_path().map(|p| p.exists()).unwrap_or(false)
}

/// Prompts the user to run setup.
fn prompt_for_setup(out: &Output) -> bool {
    note!(out);
    note!(out, "Welcome to Scope Blockchain Analysis!");
    note!(out);
    note!(
        out,
        "No configuration file found. Would you like to run the setup wizard"
    );
    note!(out, "to configure API keys and preferences?");
    note!(out);
    let Ok(input) = prompt::ask_stdin(out, "Run setup now? [Y/n]: ") else {
        return false;
    };

    let response = input.trim().to_lowercase();
    // Default to yes if user just presses Enter
    response.is_empty() || response == "y" || response == "yes"
}

/// Runs one command through the shared dispatcher with stdout/stderr output.
///
/// Prints the version line for non-interactive commands, and on failure
/// prints the error with remediation hints and exits with status 1. Exits
/// with the TUI's code when a signal ended it.
async fn dispatch(command: Commands, config: &Config, out: &Output) -> Result<()> {
    // Print version on stderr for non-interactive commands
    if !command.is_interactive() {
        note!(out, "Scope v{}", scope::VERSION);
    }

    match scope_cli::cli::dispatch::dispatch(command, config, out).await {
        Ok(0) => Ok(()),
        // The TUI ended on a signal (128+N); it already restored the
        // terminal and saved.
        Ok(code) => std::process::exit(code),
        Err(e) => {
            tracing::debug!("Command failed: {}", e);
            // If stderr itself fails there is no channel left to report on;
            // the non-zero exit status still tells the caller that it failed.
            let _ = display_error(&e, out);
            std::process::exit(1);
        }
    }
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

    // Logs go to the diagnostics sink: stderr by default, the TUI pane while
    // the TUI owns the screen. Never stdout, which carries command data.
    let builder = tracing_subscriber::fmt()
        .with_writer(|| scope::diag::Stderr)
        .with_env_filter(filter)
        .with_target(false)
        .with_thread_ids(false);

    // At low verbosity, use a compact format without timestamps so output
    // looks like normal CLI messages rather than log entries.
    if verbosity < 2 {
        builder.without_time().with_level(false).init();
    } else {
        builder.init();
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
    fn test_config_file_exists_with_explicit_path() {
        // Create a temp file and point CLI at it
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("test_config.toml");
        std::fs::write(&config_path, "").unwrap();

        let cli = Cli::try_parse_from([
            "scope",
            "--config",
            config_path.to_str().unwrap(),
            "venues",
            "list",
        ])
        .unwrap();
        assert!(config_file_exists(&cli));
    }

    #[test]
    fn test_config_file_exists_nonexistent() {
        let cli = Cli::try_parse_from([
            "scope",
            "--config",
            "/tmp/nonexistent_scope_config_test.toml",
            "venues",
            "list",
        ])
        .unwrap();
        assert!(!config_file_exists(&cli));
    }

    #[test]
    fn test_cli_completions_parsing() {
        let result = Cli::try_parse_from(["scope", "completions", "bash"]);
        assert!(result.is_ok());
    }
}
