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
    let declined = scope::paths::state_dir().join(DECLINED_FILE);
    if declined.exists() {
        return None;
    }
    note!(
        out,
        "Ghola is enabled (ghola.enabled) but ghola is not installed."
    );
    let asked = with_local_installer(|installer| {
        offer_install(&mut crate::cli::prompt::stdin(), out, installer)
    });
    match asked {
        Ok(Ok(Some(path))) => Some(path),
        Ok(Ok(None)) => {
            let _ = std::fs::create_dir_all(scope::paths::state_dir())
                .and_then(|()| std::fs::write(&declined, b""));
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
    let Some(pid) = ghola::spawned_sidecar() else {
        return;
    };
    let keep = ask && {
        let question = format!(
            "Stop the Ghola sidecar that scope started (pid {pid})? [Y/n] (stops in {}s): ",
            STOP_QUESTION_TIMEOUT.as_secs()
        );
        match ask_timeout(out, &question, STOP_QUESTION_TIMEOUT) {
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
        let (out, cap) = Output::capture();
        finish_sidecar(&out, true);
        assert!(cap.err().is_empty());
    }
}
