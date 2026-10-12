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
/// with `out` as its output. Returns the process exit code: 0, or the
/// TUI's code (128+N after signal N ended it).
///
/// # Errors
///
/// Returns the handler's error. `Commands::Web` is an error here: the web
/// server lives in the `scope-bca` crate, and its binary starts it before
/// dispatch.
pub async fn dispatch(command: Commands, config: &Config, out: &Output) -> Result<i32> {
    let http: Arc<dyn HttpClient> = create_http_client(config, out)?;
    let factory = DefaultClientFactory {
        chains_config: config.chains.clone(),
        http,
    };
    // Only the TUI can end with a code other than 0, so it starts here.
    if let Commands::Interactive(args) = command {
        let exit = crate::cli::interactive::run(args, config, &factory).await?;
        return Ok(exit.code);
    }
    dispatch_with(command, config, &factory, out)
        .await
        .map(|()| 0)
}

/// Routes one command to its handler, using the given client factory.
///
/// The TUI and the tests call this with their own factory (for example a
/// mock). [`dispatch`] calls it with the default factory.
///
/// # Errors
///
/// Returns the handler's error. See [`dispatch`]. `Commands::Interactive`
/// is an error here: the TUI starts only from [`dispatch`], which returns
/// its exit code.
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
        // The TUI calls dispatch_with and refuses a nested `interactive`
        // before it gets here (exec::route).
        Commands::Interactive(_) => Err(ScopeError::Other(
            "the TUI cannot start from here; start it with `scope tui`".into(),
        )),
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

/// Installs the process transport on the first call (see `scope::http::shared`)
/// and prints the notices. Later calls return the same transport.
///
/// # Errors
///
/// Returns an error when the transport cannot be built, or when the output's
/// error channel cannot be written.
pub fn create_http_client(config: &Config, out: &Output) -> std::io::Result<Arc<dyn HttpClient>> {
    let built = scope::http::init(config).map_err(|e| std::io::Error::other(e.to_string()))?;
    for notice in &built.notices {
        errln!(out, "  {notice}")?;
    }
    Ok(built.client)
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use scope::http::shared::Kind;

    fn quiet() -> Output {
        Output::capture().0
    }

    #[test]
    fn test_create_http_client_returns_the_installed_transport() {
        // Two calls must give the same transport: one process, one transport.
        // Default config only: `init` installs for the whole test binary, and
        // a non-native transport would reach the mockito tests too.
        let config = Config::default();
        let (out, captured) = Output::capture();
        let a = create_http_client(&config, &out).unwrap();
        let b = create_http_client(&config, &out).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        // Default config selects native, which has nothing to report.
        assert!(captured.err().is_empty(), "{}", captured.err());
    }

    /// Selection for a Ghola config, without installing it for the process.
    /// The test machine may or may not run a Ghola sidecar: the result must
    /// be Ghola with no notice, or native with notices that say so.
    fn assert_ghola_selection(stealth: bool) {
        let mut config = Config::default();
        config.ghola.enabled = true;
        config.ghola.stealth = stealth;
        let built = scope::http::shared::build_with(&config, &|_| None).unwrap();
        match built.kind {
            Kind::Ghola => assert!(built.notices.is_empty(), "{:?}", built.notices),
            Kind::Native => {
                let all = built.notices.join("\n");
                assert!(all.contains("Ghola"), "{all}");
                assert!(all.contains("Falling back"), "{all}");
            }
            other => panic!("unexpected kind {other:?}"),
        }
    }

    #[test]
    fn test_transport_selection_default_config_is_native() {
        let built = scope::http::shared::build_with(&Config::default(), &|_| None).unwrap();
        assert_eq!(built.kind, Kind::Native);
        assert!(built.notices.is_empty(), "{:?}", built.notices);
    }

    #[test]
    fn test_transport_selection_ghola_enabled() {
        assert_ghola_selection(true);
    }

    #[test]
    fn test_transport_selection_ghola_no_stealth() {
        assert_ghola_selection(false);
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

    // ---- dispatch_with routes to the real handlers (offline, mocks) ----

    const ADDR: &str = "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2";

    async fn run_line(args: &[&str]) -> (Result<()>, String) {
        let mut argv = vec!["scope"];
        argv.extend_from_slice(args);
        let cli = Cli::try_parse_from(argv).unwrap();
        let (o, cap) = Output::capture_merged();
        let factory = scope::chains::mocks::MockClientFactory::new();
        let res = dispatch_with(cli.command, &Config::default(), &factory, &o).await;
        (res, cap.out())
    }

    #[tokio::test]
    async fn test_dispatch_with_crawl_by_address() {
        let (res, out) = run_line(&["crawl", ADDR, "--yes", "--no-charts"]).await;
        res.unwrap();
        assert!(!out.trim().is_empty());
    }

    #[tokio::test]
    async fn test_dispatch_with_token_health() {
        let (res, out) = run_line(&["token-health", ADDR]).await;
        res.unwrap();
        assert!(!out.trim().is_empty());
    }

    #[tokio::test]
    async fn test_dispatch_with_report_batch_writes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("batch.md");
        let (res, _) = run_line(&[
            "report",
            "batch",
            "--addresses",
            ADDR,
            "--output",
            path.to_str().unwrap(),
        ])
        .await;
        res.unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains(ADDR));
    }

    #[tokio::test]
    async fn test_dispatch_with_refuses_the_tui() {
        // Only `dispatch` carries the TUI's exit code, so dispatch_with
        // (which the TUI itself calls) must refuse `interactive`.
        let (res, _) = run_line(&["interactive"]).await;
        assert!(res.unwrap_err().to_string().contains("scope tui"));
    }
}
