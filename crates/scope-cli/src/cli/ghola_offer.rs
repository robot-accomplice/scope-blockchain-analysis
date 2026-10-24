//! # Ghola install offer
//!
//! Asks the user before installing the Ghola sidecar, and before leaving
//! a sidecar that scope started running.
//!
//! Scope never installs on its own. It asks:
//! - when a command builds its transport with `ghola.enabled`, no sidecar
//!   answers and no `ghola` binary is found (once: a "no" is remembered);
//! - in the `setup` wizard.
//!
//! `setup --status` only suggests it. A run that cannot answer never gets
//! the question: `--yes`, JSON / CSV / Markdown output, a pipe, the web
//! API and the TUI pane keep the native fallback and print how to install.
//!
//! The install itself is `scope::http::ghola_install`: the latest GitHub
//! release, its SHA-256 checked against `checksums.txt` before anything
//! is extracted, into `~/.scope-bca/bin`.

use crate::cli::output::Output;
use crate::cli::prompt::{ask, ask_timeout};
use crate::note;
use scope::config::{Config, OutputFormat};
use scope::error::{Result, ScopeError};
use scope::http::ghola;
use scope::http::ghola_install::{self, GithubReleases, Outcome, ReleaseSource, Target};
use std::io::{BufRead, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the exit question waits before it stops the sidecar.
const STOP_QUESTION_TIMEOUT: Duration = Duration::from_secs(15);

/// The file that records a "no" to the transport's install question.
const DECLINED_FILE: &str = "ghola-install-declined";

/// What an install needs.
pub struct Installer<'a> {
    /// Where the release comes from.
    pub source: &'a dyn ReleaseSource,
    /// This OS and CPU; `None` when Ghola publishes no archive for it.
    pub target: Option<Target>,
    /// Where the binary goes.
    pub dir: PathBuf,
    /// `go` on PATH, for building Ghola when no archive fits.
    pub go: Option<PathBuf>,
    /// Runs `go install` into a folder (`ghola_install::go_install`).
    pub go_install: &'a dyn Fn(&Path, &Path) -> Result<PathBuf>,
}

/// Runs `f` with an installer for this machine: GitHub releases, into
/// `~/.scope-bca/bin`.
///
/// # Errors
///
/// Returns `Network` when the download client cannot be built.
pub fn with_local_installer<T>(f: impl FnOnce(&Installer<'_>) -> T) -> Result<T> {
    let source = GithubReleases::new()?;
    let installer = Installer {
        source: &source,
        target: Target::current(),
        dir: ghola_install::install_dir(),
        go: ghola_install::find_go(),
        go_install: &ghola_install::go_install,
    };
    Ok(f(&installer))
}

/// True when this run may ask the user: a terminal on stdin, stdout and
/// stderr, outside the TUI and the web server.
pub fn may_ask(out: &Output) -> bool {
    out.prompts_allowed()
        && !out.is_tui()
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal()
}

/// True when the run is scripted: `--yes`, `--ai`, or JSON, CSV or
/// Markdown output (from the flags or the config). Such a run never asks.
pub fn scripted(args: &[String], config: &Config) -> bool {
    if config.output.format != OutputFormat::Table {
        return true;
    }
    const MACHINE: &[&str] = &["json", "csv", "markdown"];
    args.iter().enumerate().any(|(i, a)| {
        let next = args.get(i + 1).map(String::as_str);
        match a.as_str() {
            "--yes" | "--json" | "--ai" => true,
            "--format" | "-f" => next.is_some_and(|v| MACHINE.contains(&v)),
            _ => a
                .strip_prefix("--format=")
                .is_some_and(|v| MACHINE.contains(&v)),
        }
    })
}

/// The process arguments, for [`scripted`].
pub fn process_args() -> Vec<String> {
    std::env::args().skip(1).collect()
}

/// Asks whether to install Ghola and installs it on a yes. Returns the
/// installed binary.
///
/// An install that fails is reported with the manual steps, and the run
/// goes on without Ghola.
///
/// # Errors
///
/// Returns the prompt's read or write error.
pub fn offer_install(
    reader: &mut impl BufRead,
    out: &Output,
    installer: &Installer<'_>,
) -> std::io::Result<Option<PathBuf>> {
    let answer = ask(
        reader,
        out,
        &format!(
            "Download ghola from its GitHub releases (robot-accomplice/ghola) into {}? [y/N]: ",
            installer.dir.display()
        ),
    )?;
    if !is_yes(&answer) {
        return Ok(None);
    }
    let outcome = match installer.target {
        Some(target) => run_install(installer.source, target, &installer.dir),
        None => Ok(Outcome::NoAsset {
            version: "latest".into(),
            asset: format!(
                "an archive for {}/{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        }),
    };
    match outcome {
        Ok(Outcome::Installed { path, version }) => {
            note!(
                out,
                "✓ Installed ghola v{version} at {} (SHA-256 matched the release's checksums.txt)",
                path.display()
            );
            Ok(Some(path))
        }
        Ok(Outcome::NoAsset { version, asset }) => {
            note!(out, "The ghola {version} release has no {asset}.");
            offer_go_install(reader, out, installer)
        }
        Err(e) => {
            note!(out, "⚠ Could not install ghola: {e}");
            print_manual_steps(out);
            Ok(None)
        }
    }
}

/// With no release archive for this machine: offers `go install` when
/// `go` is on PATH, else prints the manual steps.
fn offer_go_install(
    reader: &mut impl BufRead,
    out: &Output,
    installer: &Installer<'_>,
) -> std::io::Result<Option<PathBuf>> {
    let Some(go) = &installer.go else {
        print_manual_steps(out);
        return Ok(None);
    };
    let answer = ask(
        reader,
        out,
        &format!(
            "Build it with `{}` into {}? [y/N]: ",
            ghola_install::GO_INSTALL,
            installer.dir.display()
        ),
    )?;
    if !is_yes(&answer) {
        print_manual_steps(out);
        return Ok(None);
    }
    match (installer.go_install)(go, &installer.dir) {
        Ok(path) => {
            note!(out, "✓ Built ghola at {}", path.display());
            Ok(Some(path))
        }
        Err(e) => {
            note!(out, "⚠ Could not build ghola: {e}");
            print_manual_steps(out);
            Ok(None)
        }
    }
}

/// Runs the async install to its end on a thread of its own, so a
/// synchronous prompt flow (the wizard) can wait for it from inside or
/// outside a runtime.
fn run_install(source: &dyn ReleaseSource, target: Target, dir: &Path) -> Result<Outcome> {
    std::thread::scope(|s| {
        s.spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(ScopeError::IoError)?
                .block_on(ghola_install::install_latest(source, target, dir))
        })
        .join()
        .unwrap_or_else(|_| Err(ScopeError::Other("the ghola install panicked".into())))
    })
}

fn print_manual_steps(out: &Output) {
    note!(out, "To install ghola yourself:");
    for step in ghola_install::manual_steps() {
        note!(out, "- {step}");
    }
}

fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
}

/// The transport's offer: asks once (a "no" is remembered in the state
/// folder) and installs on a yes. Returns the installed binary.
pub fn offer_for_transport(out: &Output) -> Option<PathBuf> {
    offer_once(out, &scope::paths::state_dir().join(DECLINED_FILE), || {
        with_local_installer(|installer| {
            offer_install(&mut crate::cli::prompt::stdin(), out, installer)
        })
    })
}

/// [`offer_for_transport`] with the "no" recorded in `declined` and the
/// question asked by `ask`.
fn offer_once(
    out: &Output,
    declined: &Path,
    ask: impl FnOnce() -> Result<std::io::Result<Option<PathBuf>>>,
) -> Option<PathBuf> {
    if declined.exists() {
        return None;
    }
    note!(
        out,
        "Ghola is enabled (ghola.enabled) but ghola is not installed."
    );
    match ask() {
        Ok(Ok(Some(path))) => Some(path),
        Ok(Ok(None)) => {
            let _ = declined
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(declined, b""));
            note!(
                out,
                "Not asking again. Run `scope setup` to install ghola later."
            );
            None
        }
        Ok(Err(e)) => {
            tracing::debug!("ghola install question failed: {e}");
            None
        }
        Err(e) => {
            note!(out, "⚠ Could not prepare the ghola download: {e}");
            None
        }
    }
}

/// Stops the sidecar this run started, or leaves it running when the user
/// says so. With `ask`, the user has 15 seconds to answer;
/// no answer stops it. Without `ask` it is stopped.
///
/// Call it last: an unanswered question leaves a thread reading stdin.
pub fn finish_sidecar(out: &Output, ask: bool) {
    finish_with(out, ask, |question| {
        ask_timeout(out, question, STOP_QUESTION_TIMEOUT)
    });
}

/// [`finish_sidecar`] with the exit question answered by `answer`
/// (`None`: no answer in time).
fn finish_with(
    out: &Output,
    ask: bool,
    answer: impl FnOnce(&str) -> std::io::Result<Option<String>>,
) {
    let Some(pid) = ghola::spawned_sidecar() else {
        return;
    };
    let keep = ask && {
        let question = format!(
            "Stop the Ghola sidecar that scope started (pid {pid})? [Y/n] (stops in {}s): ",
            STOP_QUESTION_TIMEOUT.as_secs()
        );
        match answer(&question) {
            Ok(Some(answer)) => matches!(answer.trim().to_lowercase().as_str(), "n" | "no"),
            Ok(None) => {
                note!(out);
                false
            }
            Err(_) => false,
        }
    };
    if keep {
        note!(
            out,
            "The Ghola sidecar keeps running (pid {pid}); later runs use it."
        );
    } else if ghola::stop_spawned_sidecar().is_some() {
        note!(out, "Stopped the Ghola sidecar (pid {pid}).");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// A source that has no files and records what was fetched.
    #[derive(Default)]
    struct EmptySource {
        fetched: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ReleaseSource for EmptySource {
        async fn fetch(&self, url: &str) -> Result<Vec<u8>> {
            self.fetched.lock().unwrap().push(url.to_string());
            Err(ScopeError::Network(format!("HTTP 404 for {url}")))
        }
    }

    fn no_go(_: &Path, _: &Path) -> Result<PathBuf> {
        panic!("ran go install")
    }

    fn installer<'a>(source: &'a EmptySource, dir: &Path) -> Installer<'a> {
        Installer {
            source,
            target: Target::from_rust("linux", "x86_64"),
            dir: dir.to_path_buf(),
            go: None,
            go_install: &no_go,
        }
    }

    #[test]
    fn test_no_answer_downloads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let source = EmptySource::default();
        let (out, cap) = Output::capture();
        for answer in ["", "n\n", "no\n"] {
            let mut reader = answer.as_bytes();
            let got = offer_install(&mut reader, &out, &installer(&source, dir.path())).unwrap();
            assert_eq!(got, None);
        }
        assert!(source.fetched.lock().unwrap().is_empty());
        assert!(cap.err().contains("[y/N]"));
    }

    #[test]
    fn test_a_failed_download_prints_the_manual_steps() {
        let dir = tempfile::tempdir().unwrap();
        let source = EmptySource::default();
        let (out, cap) = Output::capture();
        let got =
            offer_install(&mut "y\n".as_bytes(), &out, &installer(&source, dir.path())).unwrap();
        assert_eq!(got, None);
        assert_eq!(
            source.fetched.lock().unwrap().as_slice(),
            [ghola_install::LATEST_RELEASE_URL]
        );
        let err = cap.err();
        assert!(err.contains("Could not install ghola"), "{err}");
        assert!(err.contains("/cmd/ghola@latest"), "{err}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn test_no_archive_for_the_target_offers_go_install() {
        let dir = tempfile::tempdir().unwrap();
        let source = EmptySource::default();
        let built = dir.path().join("ghola");
        let go_install = |go: &Path, into: &Path| -> Result<PathBuf> {
            assert_eq!(go, Path::new("/usr/bin/go"));
            Ok(into.join("ghola"))
        };
        let installer = Installer {
            source: &source,
            target: None,
            dir: dir.path().to_path_buf(),
            go: Some(PathBuf::from("/usr/bin/go")),
            go_install: &go_install,
        };
        let (out, cap) = Output::capture();
        let got = offer_install(&mut "y\ny\n".as_bytes(), &out, &installer).unwrap();
        assert_eq!(got, Some(built));
        assert!(cap.err().contains("go install"), "{}", cap.err());
        // Nothing was downloaded: there is no archive to fetch.
        assert!(source.fetched.lock().unwrap().is_empty());
    }

    #[test]
    fn test_no_archive_and_no_go_prints_the_steps() {
        let dir = tempfile::tempdir().unwrap();
        let source = EmptySource::default();
        let mut installer = installer(&source, dir.path());
        installer.target = None;
        let (out, cap) = Output::capture();
        let got = offer_install(&mut "y\n".as_bytes(), &out, &installer).unwrap();
        assert_eq!(got, None);
        assert!(cap.err().contains("To install ghola yourself"));
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_scripted_runs_are_recognised() {
        let table = Config::default();
        assert!(!scripted(&args(&["crawl", "USDC"]), &table));
        assert!(scripted(&args(&["crawl", "USDC", "--yes"]), &table));
        assert!(scripted(
            &args(&["address", "0x1", "--format", "json"]),
            &table
        ));
        assert!(scripted(&args(&["address", "0x1", "-f", "csv"]), &table));
        assert!(scripted(&args(&["tx", "0x1", "--format=json"]), &table));
        assert!(scripted(&args(&["contract", "0x1", "--json"]), &table));
        assert!(!scripted(
            &args(&["address", "0x1", "--format", "table"]),
            &table
        ));
        let mut json = Config::default();
        json.output.format = OutputFormat::Json;
        assert!(scripted(&args(&["crawl", "USDC"]), &json));
    }

    #[test]
    fn test_captured_output_never_asks() {
        let (out, _) = Output::capture();
        assert!(!may_ask(&out));
    }

    #[test]
    fn test_finish_without_a_spawned_sidecar_says_nothing() {
        let _lock = sidecar_lock();
        let (out, cap) = Output::capture();
        finish_sidecar(&out, true);
        assert!(cap.err().is_empty());
    }

    #[test]
    fn test_the_local_installer_uses_this_machine() {
        let (dir, target, go) =
            with_local_installer(|i| (i.dir.clone(), i.target, i.go.clone())).unwrap();
        assert_eq!(dir, ghola_install::install_dir());
        assert_eq!(target, Target::current());
        assert_eq!(go, ghola_install::find_go());
    }

    #[test]
    fn test_may_ask_needs_prompts_and_a_terminal() {
        let out = Output::from_writers(std::io::sink(), std::io::sink());
        let terminal = std::io::stdin().is_terminal()
            && std::io::stdout().is_terminal()
            && std::io::stderr().is_terminal();
        assert_eq!(may_ask(&out), terminal);
        assert!(!may_ask(&out.without_prompts()));
    }

    #[test]
    fn test_process_args_leave_out_the_program() {
        assert_eq!(process_args().len(), std::env::args().count() - 1);
    }

    #[test]
    fn test_a_yes_installs_the_checked_release() {
        use ghola_install::test_support::{BINARY, release, tar_gz};
        let dir = tempfile::tempdir().unwrap();
        let target = Target::from_rust("linux", "x86_64").unwrap();
        let source = release(target, tar_gz(&[("ghola", BINARY)]), None);
        let installer = Installer {
            source: &source,
            target: Some(target),
            dir: dir.path().to_path_buf(),
            go: None,
            go_install: &no_go,
        };
        let (out, cap) = Output::capture();
        let got = offer_install(&mut "yes\n".as_bytes(), &out, &installer).unwrap();
        let path = dir.path().join("ghola");
        assert_eq!(got, Some(path.clone()));
        assert_eq!(std::fs::read(&path).unwrap(), BINARY);
        let err = cap.err();
        assert!(err.contains("Installed ghola v0.6.2"), "{err}");
        assert!(err.contains("SHA-256 matched"), "{err}");
    }

    /// An installer with no archive for this machine and `go` on PATH.
    fn go_only<'a>(
        source: &'a EmptySource,
        dir: &Path,
        go_install: &'a dyn Fn(&Path, &Path) -> Result<PathBuf>,
    ) -> Installer<'a> {
        Installer {
            source,
            target: None,
            dir: dir.to_path_buf(),
            go: Some(PathBuf::from("/usr/bin/go")),
            go_install,
        }
    }

    #[test]
    fn test_a_no_to_go_install_builds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let source = EmptySource::default();
        let installer = go_only(&source, dir.path(), &no_go);
        let (out, cap) = Output::capture();
        let got = offer_install(&mut "y\nn\n".as_bytes(), &out, &installer).unwrap();
        assert_eq!(got, None);
        assert!(cap.err().contains("To install ghola yourself"));
    }

    #[test]
    fn test_a_failed_go_install_prints_the_steps() {
        let dir = tempfile::tempdir().unwrap();
        let source = EmptySource::default();
        let fails = |_: &Path, _: &Path| -> Result<PathBuf> {
            Err(ScopeError::Other("go: module not found".into()))
        };
        let installer = go_only(&source, dir.path(), &fails);
        let (out, cap) = Output::capture();
        let got = offer_install(&mut "y\ny\n".as_bytes(), &out, &installer).unwrap();
        assert_eq!(got, None);
        let err = cap.err();
        assert!(err.contains("Could not build ghola: "), "{err}");
        assert!(err.contains("go: module not found"), "{err}");
        assert!(err.contains("To install ghola yourself"), "{err}");
    }

    #[test]
    fn test_the_transport_offer_respects_a_recorded_no() {
        let dir = tempfile::tempdir().unwrap();
        let declined = dir.path().join(DECLINED_FILE);
        std::fs::write(&declined, b"").unwrap();
        let (out, cap) = Output::capture();
        let got = offer_once(&out, &declined, || panic!("asked again"));
        assert_eq!(got, None);
        assert!(cap.err().is_empty());
    }

    #[test]
    fn test_a_no_to_the_transport_offer_is_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let declined = dir.path().join("state").join(DECLINED_FILE);
        let (out, cap) = Output::capture();
        assert_eq!(offer_once(&out, &declined, || Ok(Ok(None))), None);
        assert!(declined.exists());
        let err = cap.err();
        assert!(err.contains("ghola is not installed"), "{err}");
        assert!(err.contains("Not asking again"), "{err}");
    }

    #[test]
    fn test_a_yes_to_the_transport_offer_returns_the_binary() {
        let dir = tempfile::tempdir().unwrap();
        let declined = dir.path().join(DECLINED_FILE);
        let binary = dir.path().join("ghola");
        let (out, _) = Output::capture();
        let got = offer_once(&out, &declined, || Ok(Ok(Some(binary.clone()))));
        assert_eq!(got, Some(binary));
        assert!(!declined.exists());
    }

    #[test]
    fn test_a_failed_transport_offer_is_not_taken_as_a_no() {
        let dir = tempfile::tempdir().unwrap();
        let declined = dir.path().join(DECLINED_FILE);
        let (out, cap) = Output::capture();
        let read_failed = || Ok(Err(std::io::Error::other("stdin closed")));
        assert_eq!(offer_once(&out, &declined, read_failed), None);
        let no_client = || Err(ScopeError::Network("no TLS".into()));
        assert_eq!(offer_once(&out, &declined, no_client), None);
        assert!(!declined.exists());
        let err = cap.err();
        assert!(
            err.contains("Could not prepare the ghola download: "),
            "{err}"
        );
    }

    #[test]
    fn test_the_transport_offer_reads_the_state_folder() {
        let declined = scope::paths::state_dir().join(DECLINED_FILE);
        std::fs::create_dir_all(scope::paths::state_dir()).unwrap();
        std::fs::write(&declined, b"").unwrap();
        let (out, cap) = Output::capture();
        assert_eq!(offer_for_transport(&out), None);
        assert!(cap.err().is_empty());
        std::fs::remove_file(&declined).unwrap();
    }

    /// Serializes the tests that use the spawned sidecar.
    static SIDECAR: Mutex<()> = Mutex::new(());

    fn sidecar_lock() -> std::sync::MutexGuard<'static, ()> {
        SIDECAR.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adopts a process that runs until it is stopped as the spawned
    /// sidecar, and returns its PID.
    fn adopt_running_sidecar() -> u32 {
        let mut command = if cfg!(windows) {
            let mut c = std::process::Command::new("ping");
            c.args(["-n", "60", "127.0.0.1"]);
            c
        } else {
            let mut c = std::process::Command::new("sleep");
            c.arg("60");
            c
        };
        let child = command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        ghola::adopt_sidecar(child);
        pid
    }

    #[test]
    fn test_finish_without_asking_stops_the_sidecar() {
        let _lock = sidecar_lock();
        let pid = adopt_running_sidecar();
        let (out, cap) = Output::capture();
        finish_sidecar(&out, false);
        assert_eq!(ghola::spawned_sidecar(), None);
        assert!(
            cap.err()
                .contains(&format!("Stopped the Ghola sidecar (pid {pid})")),
            "{}",
            cap.err()
        );
    }

    #[test]
    fn test_a_no_at_exit_keeps_the_sidecar() {
        let _lock = sidecar_lock();
        let pid = adopt_running_sidecar();
        let (out, cap) = Output::capture();
        let mut asked = String::new();
        finish_with(&out, true, |question| {
            asked = question.to_string();
            Ok(Some(" No\n".into()))
        });
        assert!(asked.contains(&format!("(pid {pid})")), "{asked}");
        assert!(asked.contains("stops in 15s"), "{asked}");
        assert_eq!(ghola::spawned_sidecar(), Some(pid));
        assert!(cap.err().contains("keeps running"), "{}", cap.err());
        assert_eq!(ghola::stop_spawned_sidecar(), Some(pid));
    }

    #[test]
    fn test_any_other_exit_answer_stops_the_sidecar() {
        let _lock = sidecar_lock();
        let answers: [fn() -> std::io::Result<Option<String>>; 4] = [
            || Ok(Some("\n".into())),
            || Ok(Some("y\n".into())),
            || Ok(None),
            || Err(std::io::Error::other("stdin closed")),
        ];
        for answer in answers {
            let pid = adopt_running_sidecar();
            let (out, cap) = Output::capture();
            finish_with(&out, true, |_| answer());
            assert_eq!(ghola::spawned_sidecar(), None);
            assert!(
                cap.err()
                    .contains(&format!("Stopped the Ghola sidecar (pid {pid})")),
                "{}",
                cap.err()
            );
        }
    }
}
