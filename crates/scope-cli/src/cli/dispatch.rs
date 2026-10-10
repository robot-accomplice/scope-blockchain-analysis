//! # Command Dispatch
//!
//! Routes a parsed [`Commands`] value to its handler. The `scope` binary and
//! the TUI both call [`dispatch`], so a command gives the same result in
//! both places. Process concerns (the version line, error display, the exit
//! code) stay with the caller.

use crate::cli::output::Output;
use crate::cli::{Cli, Commands};
use crate::{errln, out};
use clap::CommandFactory;
use clap_complete::generate;
use scope::Config;
use scope::chains::{ChainClientFactory, DefaultClientFactory};
use scope::error::{Result, ScopeError};
use scope::http::HttpClient;
use std::sync::Arc;

/// Routes one command to its handler.
///
/// Creates the HTTP transport and client factory, then runs the handler
/// with `out` as its output.
///
/// # Errors
///
/// Returns the handler's error. `Commands::Web` is an error here: the web
/// server lives in the `scope-bca` crate, and its binary starts it before
/// dispatch.
pub async fn dispatch(command: Commands, config: &Config, out: &Output) -> Result<()> {
    let http: Arc<dyn HttpClient> = create_http_client(config, out)?;
    let factory = DefaultClientFactory {
        chains_config: config.chains.clone(),
        http,
    };
    dispatch_with(command, config, &factory, out).await
}

/// Routes one command to its handler, using the given client factory.
///
/// The TUI and the tests call this with their own factory (for example a
/// mock). [`dispatch`] calls it with the default factory.
///
/// # Errors
///
/// Returns the handler's error. See [`dispatch`].
pub async fn dispatch_with(
    command: Commands,
    config: &Config,
    clients: &dyn ChainClientFactory,
    out: &Output,
) -> Result<()> {
    match command {
        Commands::Completions(args) => {
            let mut cmd = Cli::command();
            let mut buf = Vec::new();
            generate(args.shell, &mut cmd, "scope", &mut buf);
            out!(out, "{}", String::from_utf8_lossy(&buf))?;
            Ok(())
        }
        Commands::Address(args) => crate::cli::address::run(args, config, clients, out).await,
        Commands::Tx(args) => crate::cli::tx::run(args, config, clients, out).await,
        Commands::Crawl(args) => crate::cli::crawl::run(args, config, clients, out).await,
        Commands::AddressBook(args) => {
            crate::cli::address_book::run(args, config, clients, out).await
        }
        Commands::Export(args) => crate::cli::export::run(args, config, clients, out).await,
        // Boxed: the TUI calls dispatch_with, so this arm makes an async
        // cycle. The TUI refuses a nested `interactive`, so it never recurses.
        Commands::Interactive(args) => {
            Box::pin(crate::cli::interactive::run(args, config, clients)).await
        }
        Commands::Monitor(args) => crate::cli::monitor::run_direct(args, config, clients).await,
        Commands::Setup(args) => crate::cli::setup::run(args, config, out).await,
        Commands::Compliance(compliance_cmd) => {
            dispatch_compliance(compliance_cmd, config, out).await
        }
        Commands::Market(cmd) => crate::cli::market::run(cmd, config, clients, out).await,
        Commands::TokenHealth(args) => {
            crate::cli::token_health::run(args, config, clients, out).await
        }
        Commands::Venues(cmd) => crate::cli::venues::run(cmd, out),
        Commands::Report(cmd) => crate::cli::report::run(cmd, config, clients, out).await,
        Commands::Discover(args) => crate::cli::discover::run(args, config.output.format, out)
            .await
            .map_err(|e| ScopeError::Discovery(e.to_string())),
        Commands::Insights(args) => crate::cli::insights::run(args, config, clients, out).await,
        Commands::Contract(ref args) => crate::cli::contract::run(args, config, clients, out).await,
        Commands::Web(_) => Err(ScopeError::Other(
            "the web server cannot run from here; start it with `scope web`".into(),
        )),
    }
}

/// Routes a compliance subcommand. `@label` inputs resolve through the
/// address book first.
async fn dispatch_compliance(
    cmd: crate::cli::compliance::ComplianceCommands,
    config: &Config,
    out: &Output,
) -> Result<()> {
    use crate::cli::address_book::resolve_address_book_input;
    use crate::cli::compliance::{self, ComplianceCommands};

    let result = match cmd {
        ComplianceCommands::Risk(mut args) => {
            if let Some((addr, chain)) = resolve_address_book_input(&args.address, config)? {
                args.address = addr;
                if args.chain.is_none() {
                    args.chain = Some(chain);
                }
            }
            compliance::handle_risk(args, out).await
        }
        ComplianceCommands::Trace(args) => compliance::handle_trace(args, out).await,
        ComplianceCommands::Analyze(mut args) => {
            if let Some((addr, _chain)) = resolve_address_book_input(&args.address, config)? {
                args.address = addr;
            }
            compliance::handle_analyze(args, out).await
        }
        ComplianceCommands::ComplianceReport(mut args) => {
            if !std::path::Path::new(&args.target).exists()
                && let Some((addr, _chain)) = resolve_address_book_input(&args.target, config)?
            {
                args.target = addr;
            }
            compliance::handle_compliance_report(args, out).await
        }
    };
    result.map_err(|e| ScopeError::Compliance(e.to_string()))
}

/// Creates the HTTP transport based on Ghola configuration.
///
/// When `config.ghola.enabled` is `true`, attempts to create a Ghola
/// sidecar client. Falls back to native `reqwest` if Ghola fails to start
/// or is not installed, and says so on the error channel.
///
/// # Errors
///
/// Returns the output's error when the fallback notice cannot be written.
pub fn create_http_client(config: &Config, out: &Output) -> std::io::Result<Arc<dyn HttpClient>> {
    if config.ghola.enabled {
        match scope::http::GholaHttpClient::new(config.ghola.stealth, config.ghola.buffer_size) {
            Ok(client) => {
                tracing::info!("Using Ghola sidecar for HTTP transport");
                return Ok(Arc::new(client));
            }
            Err(e) => {
                errln!(out, "  ⚠ Ghola sidecar enabled but unavailable: {}", e)?;
                errln!(
                    out,
                    "    Install: go install github.com/robot-accomplice/ghola@latest"
                )?;
                errln!(out, "    Falling back to native HTTP transport")?;
            }
        }
    }

    Ok(Arc::new(
        scope::http::NativeHttpClient::new().expect("Failed to create HTTP client"),
    ))
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn quiet() -> Output {
        Output::capture().0
    }

    #[test]
    fn test_create_http_client_default_config() {
        let config = Config::default();
        let client = create_http_client(&config, &quiet()).unwrap();
        // Default config has ghola.enabled = false, so should get NativeHttpClient
        let _: &dyn HttpClient = &*client;
    }

    #[test]
    fn test_create_http_client_ghola_enabled() {
        let mut config = Config::default();
        config.ghola.enabled = true;
        config.ghola.stealth = true;
        let client = create_http_client(&config, &quiet()).unwrap();
        // Should still succeed (falls back to native if ghola not installed)
        let _: &dyn HttpClient = &*client;
    }

    #[test]
    fn test_create_http_client_ghola_no_stealth() {
        let mut config = Config::default();
        config.ghola.enabled = true;
        config.ghola.stealth = false;
        let client = create_http_client(&config, &quiet()).unwrap();
        let _: &dyn HttpClient = &*client;
    }

    #[tokio::test]
    async fn test_dispatch_writes_completions_to_output() {
        // The TUI shows dispatch output in a pane: completions must go to
        // the injected writer, not straight to the process stdout.
        let cli = Cli::try_parse_from(["scope", "completions", "bash"]).unwrap();
        let (o, cap) = Output::capture();
        dispatch(cli.command, &Config::default(), &o).await.unwrap();
        assert!(cap.out().contains("_scope()"));
    }

    #[tokio::test]
    async fn test_dispatch_fails_when_output_cannot_be_written() {
        // `scope completions zsh > /full/disk` must exit non-zero, not
        // report success with no file content.
        let cli = Cli::try_parse_from(["scope", "completions", "zsh"]).unwrap();
        let o = Output::from_writers(
            crate::cli::output::tests::ClosedPipe,
            crate::cli::output::tests::ClosedPipe,
        );
        let err = dispatch(cli.command, &Config::default(), &o)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ScopeError::IoError(ref e) if e.kind() == std::io::ErrorKind::BrokenPipe)
        );
    }

    #[tokio::test]
    async fn test_dispatch_rejects_web() {
        // `web` needs the scope-bca crate; dispatch must fail, not panic.
        let cli = Cli::try_parse_from(["scope", "web"]).unwrap();
        let err = dispatch(cli.command, &Config::default(), &quiet())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("scope web"));
    }
}
