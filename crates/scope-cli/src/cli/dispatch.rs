//! # Command Dispatch
//!
//! Routes a parsed [`Commands`] value to its handler. The `scope` binary and
//! the TUI both call [`dispatch`], so a command gives the same result in
//! both places. Process concerns (the version line, error display, the exit
//! code) stay with the caller.

use crate::cli::output::Output;
use crate::cli::tui::Settings;
use crate::cli::{Cli, Commands};
use crate::{note, out};
use clap::CommandFactory;
use clap_complete::generate;
use scope::Config;
use scope::chains::{ChainClientFactory, DefaultClientFactory};
use scope::config::OutputFormat;
use scope::error::{Result, ScopeError};
use scope::http::HttpClient;
use std::path::PathBuf;
use std::sync::Arc;

/// Where the config comes from: the binary reads it this way at start,
/// and the TUI reads it the same way again after `setup` (#99).
#[derive(Debug, Clone, Default)]
pub struct ConfigSource {
    /// `--config`, when given; else `SCOPE_CONFIG` or the default file.
    pub path: Option<PathBuf>,
    /// `--ai`: Markdown output for an agent to read.
    pub ai: bool,
}

impl ConfigSource {
    /// Reads the config and applies the flags.
    ///
    /// # Errors
    ///
    /// Returns the config file's read or parse error.
    pub fn load(&self) -> Result<Config> {
        let mut config = Config::load(self.path.as_deref())?;
        self.apply(&mut config);
        Ok(config)
    }

    /// Applies the flags that override the file (`--ai`).
    pub fn apply(&self, config: &mut Config) {
        if self.ai {
            config.output.format = OutputFormat::Markdown;
        }
    }
}

/// Routes one command to its handler.
///
/// Creates the HTTP transport and client factory, then runs the handler
/// with `out` as its output. Returns the process exit code: 0, or the
/// TUI's code (128+N after signal N ended it). `source` says how to read
/// `config` again; the TUI does so after `setup`.
///
/// # Errors
///
/// Returns the handler's error. `Commands::Web` is an error here: the web
/// server lives in the `scope-bca` crate, and its binary starts it before
/// dispatch.
pub async fn dispatch(
    command: Commands,
    source: &ConfigSource,
    config: &Config,
    out: &Output,
) -> Result<i32> {
    // A command run on a terminal may offer to install a missing Ghola.
    // The TUI, the wizard, the web server and scripted runs never do.
    use crate::cli::ghola_offer;
    let offer = !command.is_interactive()
        && ghola_offer::may_ask(out)
        && !ghola_offer::scripted(&ghola_offer::process_args(), config);
    let env = |k: &str| std::env::var(k).ok();
    let http: Arc<dyn HttpClient> = create_http_client_offering(config, out, &env, offer)?;
    // Only the TUI can end with a code other than 0, so it starts here.
    // `scope monitor` opens it too, with the monitor wizard (#47).
    let opens_tui = match &command {
        Commands::Interactive(_) => true,
        Commands::Monitor(args) => !args.watch.dashboard,
        _ => false,
    };
    if opens_tui {
        let settings = |config: Config, http: Arc<dyn HttpClient>| Settings {
            clients: Box::new(DefaultClientFactory {
                chains_config: config.chains.clone(),
                http,
            }),
            config,
        };
        // A reload after `setup` builds the transport again, so transport
        // settings (proxy, Ghola) apply as well as the chain keys.
        let reload = || {
            let config = source.load()?;
            let http = transport_for_reload(&config, &http, out, &|k| std::env::var(k).ok())?;
            Ok(settings(config, http))
        };
        let first = settings(config.clone(), http.clone());
        let exit = match command {
            Commands::Monitor(args) => {
                crate::cli::interactive::run_monitor(args, first, &reload).await?
            }
            Commands::Interactive(args) => {
                crate::cli::interactive::run(args, first, &reload).await?
            }
            _ => unreachable!("opens_tui is true only for these two"),
        };
        return Ok(exit.code);
    }
    let factory = DefaultClientFactory {
        chains_config: config.chains.clone(),
        http,
    };
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
    if needs_venues(&command) {
        crate::cli::venues::ensure_catalog(config, clients.http().as_ref(), out).await;
    }
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
        Commands::Monitor(args) => {
            crate::cli::monitor::run_direct(args, config, clients, out).await
        }
        Commands::Setup(args) => crate::cli::setup::run(args, config, out).await,
        Commands::Compliance(compliance_cmd) => {
            dispatch_compliance(compliance_cmd, config, clients.http(), out).await
        }
        Commands::Market(cmd) => crate::cli::market::run(cmd, config, clients, out).await,
        Commands::TokenHealth(args) => {
            crate::cli::token_health::run(args, config, clients, out).await
        }
        Commands::Venues(cmd) => crate::cli::venues::run(cmd, config, clients.http(), out).await,
        Commands::Report(cmd) => crate::cli::report::run(cmd, config, clients, out).await,
        Commands::Discover(args) => {
            crate::cli::discover::run(args, config.output.format, clients.http(), out)
                .await
                .map_err(|e| match e {
                    // An I/O error keeps its kind (a closed pipe exits quietly).
                    ScopeError::IoError(_) => e,
                    other => ScopeError::Discovery(other.to_string()),
                })
        }
        Commands::Insights(args) => crate::cli::insights::run(args, config, clients, out).await,
        Commands::Contract(ref args) => crate::cli::contract::run(args, config, clients, out).await,
        Commands::Web(_) => Err(ScopeError::Other(
            "the web server cannot run from here; start it with `scope web`".into(),
        )),
    }
}

/// True for a command that reads the venues, which fetches the catalog on
/// first use. `venues update` and `venues init` fetch it themselves.
fn needs_venues(command: &Commands) -> bool {
    use crate::cli::venues::VenuesCommands;
    match command {
        Commands::Market(_)
        | Commands::Monitor(_)
        | Commands::TokenHealth(_)
        | Commands::Insights(_) => true,
        Commands::Venues(cmd) => matches!(cmd, VenuesCommands::List(_)),
        _ => false,
    }
}

/// Routes a compliance subcommand. `@label` inputs resolve through the
/// address book first.
async fn dispatch_compliance(
    cmd: crate::cli::compliance::ComplianceCommands,
    config: &Config,
    http: Arc<dyn HttpClient>,
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
            compliance::handle_risk(args, http, out).await
        }
        ComplianceCommands::Trace(args) => compliance::handle_trace(args, http, out).await,
        ComplianceCommands::Analyze(mut args) => {
            if let Some((addr, _chain)) = resolve_address_book_input(&args.address, config)? {
                args.address = addr;
            }
            compliance::handle_analyze(args, http, out).await
        }
        ComplianceCommands::ComplianceReport(mut args) => {
            if !std::path::Path::new(&args.target).exists()
                && let Some((addr, _chain)) = resolve_address_book_input(&args.target, config)?
            {
                args.target = addr;
            }
            compliance::handle_compliance_report(args, http, out).await
        }
    };
    result.map_err(|e| from_anyhow(e, ScopeError::Compliance))
}

/// A command error from an `anyhow` handler. An I/O error keeps its kind
/// (a closed pipe stays a closed pipe) and a `ScopeError` stays itself;
/// anything else is wrapped by `kind` with its message.
fn from_anyhow(e: anyhow::Error, kind: fn(String) -> ScopeError) -> ScopeError {
    match e.downcast::<std::io::Error>() {
        Ok(io) => ScopeError::IoError(io),
        Err(e) => match e.downcast::<ScopeError>() {
            Ok(scope) => scope,
            Err(e) => kind(e.to_string()),
        },
    }
}

/// Builds the transport that `config` and the environment select (see
/// `scope::http::shared`) and prints its notices. Each call builds a new one.
///
/// # Errors
///
/// Returns the error when the transport cannot be built.
pub fn create_http_client(config: &Config, out: &Output) -> Result<Arc<dyn HttpClient>> {
    create_http_client_with(config, out, &|k| std::env::var(k).ok())
}

/// [`create_http_client`], reading environment variables through `env`.
fn create_http_client_with(
    config: &Config,
    out: &Output,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Arc<dyn HttpClient>> {
    create_http_client_offering(config, out, env, false)
}

/// Builds the transport. With `offer`, when Ghola is enabled but not
/// installed, it asks whether to install it and, on a yes, builds again
/// with the new binary (which starts the sidecar).
fn create_http_client_offering(
    config: &Config,
    out: &Output,
    env: &dyn Fn(&str) -> Option<String>,
    offer: bool,
) -> Result<Arc<dyn HttpClient>> {
    let mut built = scope::http::shared::build_with(config, env)?;
    if offer && built.ghola_missing && crate::cli::ghola_offer::offer_for_transport(out).is_some() {
        built = scope::http::shared::build_with(config, env)?;
    }
    for notice in &built.notices {
        note!(out, "{notice}");
    }
    Ok(built.client)
}

/// The transport for the TUI after `setup` saved `config`: a new one, so
/// the saved transport settings apply. Replay and record keep `current`:
/// it counts the responses served and the fixtures written in this run,
/// and `setup` cannot change the mode.
fn transport_for_reload(
    config: &Config,
    current: &Arc<dyn HttpClient>,
    out: &Output,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Arc<dyn HttpClient>> {
    use scope::http::shared::{Kind, configured_kind_with};
    match configured_kind_with(config, env) {
        Kind::Replay | Kind::Record => Ok(current.clone()),
        Kind::Native | Kind::Ghola => create_http_client_with(config, out, env),
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
pub(crate) mod tests {
    /// The mock factory with a transport that never reaches the network:
    /// every request it is sent misses (a replay of an empty folder). The
    /// mock's own transport is a real one, which handlers that call
    /// `clients.http()` (Etherscan with a configured key, venues, discover)
    /// would use.
    pub(crate) struct OfflineFactory {
        pub(crate) mock: scope::chains::mocks::MockClientFactory,
        http: std::sync::Arc<dyn scope::http::HttpClient>,
        _fixtures: tempfile::TempDir,
    }

    impl OfflineFactory {
        pub(crate) fn new() -> Self {
            let fixtures = tempfile::tempdir().unwrap();
            Self {
                mock: scope::chains::mocks::MockClientFactory::new(),
                http: std::sync::Arc::new(scope::http::replay::ReplayHttpClient::replay(
                    fixtures.path().to_path_buf(),
                )),
                _fixtures: fixtures,
            }
        }
    }

    impl scope::chains::ChainClientFactory for OfflineFactory {
        fn create_chain_client(
            &self,
            chain: &str,
        ) -> scope::error::Result<Box<dyn scope::chains::ChainClient>> {
            self.mock.create_chain_client(chain)
        }

        fn create_dex_client(&self) -> Box<dyn scope::chains::DexDataSource> {
            self.mock.create_dex_client()
        }

        fn http(&self) -> std::sync::Arc<dyn scope::http::HttpClient> {
            self.http.clone()
        }
    }

    #[test]
    fn test_a_config_source_reads_its_file_and_applies_ai() {
        use super::ConfigSource;
        use scope::config::OutputFormat;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.yaml");
        std::fs::write(&path, "output:\n  format: json\n").unwrap();
        let plain = ConfigSource {
            path: Some(path.clone()),
            ai: false,
        };
        assert_eq!(plain.load().unwrap().output.format, OutputFormat::Json);
        let ai = ConfigSource {
            path: Some(path),
            ai: true,
        };
        assert_eq!(ai.load().unwrap().output.format, OutputFormat::Markdown);
    }

    #[test]
    fn test_anyhow_io_errors_keep_their_kind() {
        // A closed pipe in `compliance … | head` must still read as a
        // closed pipe, so the CLI exits quietly.
        let pipe = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed");
        let e = super::from_anyhow(pipe.into(), ScopeError::Compliance);
        assert!(
            matches!(&e, ScopeError::IoError(io) if io.kind() == std::io::ErrorKind::BrokenPipe)
        );
        let scope = super::from_anyhow(
            ScopeError::NotFound("x".into()).into(),
            ScopeError::Compliance,
        );
        assert!(matches!(scope, ScopeError::NotFound(_)));
        let other = super::from_anyhow(anyhow::anyhow!("bad"), ScopeError::Compliance);
        assert_eq!(
            other.to_string(),
            ScopeError::Compliance("bad".into()).to_string()
        );
    }

    use super::*;
    use clap::Parser;
    use scope::http::shared::Kind;

    fn quiet() -> Output {
        Output::capture().0
    }

    #[test]
    fn test_create_http_client_builds_a_new_transport_each_call() {
        // Nothing is installed for the process (#59): each caller gets its own.
        let config = Config::default();
        let (out, captured) = Output::capture();
        let a = create_http_client_with(&config, &out, &|_| None).unwrap();
        let b = create_http_client_with(&config, &out, &|_| None).unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        // Default config selects native, which has nothing to report.
        assert!(captured.err().is_empty(), "{}", captured.err());
    }

    #[test]
    fn test_a_reload_builds_the_transport_again() {
        // After `setup`, transport settings apply without a restart.
        let (out, _) = Output::capture();
        let current = create_http_client_with(&Config::default(), &out, &|_| None).unwrap();
        let next = transport_for_reload(&Config::default(), &current, &out, &|_| None).unwrap();
        assert!(!Arc::ptr_eq(&current, &next));
    }

    #[test]
    fn test_a_reload_keeps_the_replay_and_record_transport() {
        // A new recorder would overwrite this run's fixtures, and a new
        // replayer would serve each fixture from its first response again.
        let (out, _) = Output::capture();
        let current = create_http_client_with(&Config::default(), &out, &|_| None).unwrap();
        for mode in ["replay", "record"] {
            let env = move |k: &str| match k {
                "SCOPE_HTTP_MODE" => Some(mode.to_string()),
                "SCOPE_HTTP_FIXTURES" => Some("unused".to_string()),
                _ => None,
            };
            let next = transport_for_reload(&Config::default(), &current, &out, &env).unwrap();
            assert!(Arc::ptr_eq(&current, &next), "{mode}");
        }
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
        dispatch(
            cli.command,
            &ConfigSource::default(),
            &Config::default(),
            &o,
        )
        .await
        .unwrap();
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
        let err = dispatch(
            cli.command,
            &ConfigSource::default(),
            &Config::default(),
            &o,
        )
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
        let err = dispatch(
            cli.command,
            &ConfigSource::default(),
            &Config::default(),
            &quiet(),
        )
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

    #[tokio::test]
    async fn test_compliance_errors_are_labelled_as_compliance_errors() {
        // Scripts match on the error kind, so a handler failure must come
        // back as ScopeError::Compliance, not as the handler's own error.
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("none.txt");
        std::fs::write(&empty, "# no addresses\n").unwrap();
        let out = dir.path().join("report.txt");
        let (res, _) = run_line(&[
            "compliance",
            "compliance-report",
            empty.to_str().unwrap(),
            "--jurisdiction",
            "us",
            "--output",
            out.to_str().unwrap(),
        ])
        .await;
        let err = res.unwrap_err();
        assert!(matches!(err, ScopeError::Compliance(_)), "{err:?}");
        assert!(err.to_string().contains("No addresses to analyze"));

        let (res, _) = run_line(&["compliance", "risk", "not-an-address"]).await;
        assert!(matches!(res.unwrap_err(), ScopeError::Compliance(_)));
    }

    /// Runs `scope <args>` through `dispatch_with` on an offline factory.
    /// Returns the result, stdout and stderr (where notes go).
    async fn run_offline(args: &[&str], config: &Config) -> (Result<()>, String, String) {
        let mut argv = vec!["scope"];
        argv.extend_from_slice(args);
        let cli = Cli::try_parse_from(argv).unwrap();
        let (o, cap) = Output::capture();
        let factory = OfflineFactory::new();
        let res = dispatch_with(cli.command, config, &factory, &o).await;
        (res, cap.out(), cap.err())
    }

    #[tokio::test]
    async fn test_compliance_resolves_a_saved_label_to_its_address() {
        // `@label` reads the address book for risk, analyze and the report,
        // so they look up the saved address, not the label text.
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.address_book.data_dir = Some(dir.path().to_path_buf());
        let (res, _, _) = run_offline(
            &["address-book", "add", ADDR, "--label", "treasury"],
            &config,
        )
        .await;
        res.unwrap();
        let (res, out, _) = run_offline(&["address-book", "list"], &config).await;
        res.unwrap();
        assert!(out.contains("treasury"), "{out}");

        let (res, out, _) = run_offline(
            &["compliance", "risk", "@treasury", "--format", "json"],
            &config,
        )
        .await;
        res.unwrap();
        let v: serde_json::Value = serde_json::from_str(out.trim()).expect(&out);
        assert_eq!(v["address"], ADDR, "{out}");
        assert_eq!(v["chain"], "ethereum", "the saved chain: {out}");

        let (res, _, err) = run_offline(&["compliance", "analyze", "@treasury"], &config).await;
        res.unwrap();
        assert!(
            err.contains(&format!("Analyzing patterns for {ADDR}")),
            "{err}"
        );

        let report = dir.path().join("report.json");
        let (res, _, _) = run_offline(
            &[
                "compliance",
                "compliance-report",
                "@treasury",
                "--jurisdiction",
                "us",
                "--output",
                report.to_str().unwrap(),
            ],
            &config,
        )
        .await;
        res.unwrap();
        let text = std::fs::read_to_string(&report).unwrap();
        assert!(text.contains(ADDR), "{text}");
    }

    #[tokio::test]
    async fn test_compliance_trace_runs_its_handler() {
        let hash = "0x0000000000000000000000000000000000000000000000000000000000000001";
        let (res, _, err) = run_offline(&["compliance", "trace", hash], &Config::default()).await;
        // With an Etherscan key on the test machine the lookup runs and,
        // with no fixture to serve, fails as a compliance error; with none
        // the handler says how to add one.
        match res {
            Ok(()) => assert!(
                err.contains(&format!("Tracing transaction {hash}")),
                "{err}"
            ),
            Err(e) => {
                assert!(matches!(e, ScopeError::Compliance(_)), "{e:?}");
                assert!(e.to_string().contains("cannot trace transaction"), "{e}");
            }
        }
    }

    #[tokio::test]
    async fn test_discover_failures_are_labelled_as_discovery_errors() {
        // The offline transport serves nothing, so the listing fails; the
        // error must say it came from discover.
        let (res, _, _) = run_offline(&["discover"], &Config::default()).await;
        let err = res.unwrap_err();
        assert!(matches!(err, ScopeError::Discovery(_)), "{err:?}");
    }

    #[tokio::test]
    async fn test_dispatch_with_insights_and_contract() {
        let (res, out, _) = run_offline(&["insights", ADDR], &Config::default()).await;
        res.unwrap();
        assert!(!out.trim().is_empty());
        // The mock chain client reads no code; contract's error comes back
        // as the chain error it is, not relabelled.
        let (res, out, _) = run_offline(&["contract", ADDR, "--json"], &Config::default()).await;
        let err = res.unwrap_err();
        assert!(matches!(err, ScopeError::Chain(_)), "{err:?}");
        assert!(out.is_empty(), "no report on a failed analysis: {out}");
    }

    #[test]
    fn test_create_http_client_selects_the_configured_transport() {
        let (out, captured) = Output::capture();
        let client = create_http_client(&Config::default(), &out);
        assert!(client.is_ok());
        assert!(captured.err().is_empty(), "{}", captured.err());
    }

    #[tokio::test]
    async fn test_compliance_unknown_label_fails_before_any_lookup() {
        // A typo in `@label` must stop at the address book (a NotFound
        // error that lists the labels), never reach a handler.
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.address_book.data_dir = Some(dir.path().to_path_buf());
        let factory = scope::chains::mocks::MockClientFactory::new();
        for args in [
            vec!["compliance", "risk", "@nobody"],
            vec!["compliance", "analyze", "@nobody"],
            vec![
                "compliance",
                "compliance-report",
                "@nobody",
                "--jurisdiction",
                "eu",
                "--output",
                "unused.txt",
            ],
        ] {
            let mut argv = vec!["scope"];
            argv.extend_from_slice(&args);
            let cli = Cli::try_parse_from(argv).unwrap();
            let (o, cap) = Output::capture_merged();
            let err = dispatch_with(cli.command, &config, &factory, &o)
                .await
                .unwrap_err();
            assert!(matches!(err, ScopeError::NotFound(_)), "{args:?}: {err:?}");
            assert!(err.to_string().contains("@nobody"), "{args:?}");
            assert!(!cap.out().contains("Analyzing"), "handler must not run");
        }
    }
}
