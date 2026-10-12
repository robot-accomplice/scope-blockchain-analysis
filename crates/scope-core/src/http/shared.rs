//! # Shared HTTP transport
//!
//! The one place that selects the HTTP transport. `init` builds it from
//! config and installs it for the process. Clients that do not get a
//! transport passed in call `transport()`. See the TUI test harness
//! design, section 2.2.

use super::{GholaHttpClient, HttpClient, NativeHttpClient};
use crate::config::Config;
use crate::error::ScopeError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

/// Which transport was built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Native,
    Ghola,
    Replay,
    Record,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Native => "native",
            Kind::Ghola => "ghola",
            Kind::Replay => "replay",
            Kind::Record => "record",
        }
    }
}

/// A built transport and the notices the caller must show the user.
#[derive(Clone)]
pub struct Built {
    pub client: Arc<dyn HttpClient>,
    pub kind: Kind,
    pub notices: Vec<String>,
}

static INSTALLED: OnceLock<(Arc<dyn HttpClient>, Kind)> = OnceLock::new();
/// True when `transport()` installed native because nothing was installed.
static FELL_BACK: AtomicBool = AtomicBool::new(false);

/// Builds the transport the config and environment select.
pub fn build(config: &Config) -> Result<Built, ScopeError> {
    build_with(config, &|k| std::env::var(k).ok())
}

/// Builds the transport, reading environment variables through `env`.
///
/// Order: Ghola when enabled and available, else native.
pub fn build_with(
    config: &Config,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Built, ScopeError> {
    let _ = env; // replay and record selection are added in Task 9
    let mut notices = Vec::new();
    if config.ghola.enabled {
        match GholaHttpClient::new(config.ghola.stealth, config.ghola.buffer_size) {
            Ok(client) => {
                tracing::info!("Using Ghola sidecar for HTTP transport");
                return Ok(Built {
                    client: Arc::new(client),
                    kind: Kind::Ghola,
                    notices,
                });
            }
            Err(e) => {
                notices.push(format!("⚠ Ghola sidecar enabled but unavailable: {e}"));
                notices
                    .push("  Install: go install github.com/robot-accomplice/ghola@latest".into());
                notices.push("  Falling back to native HTTP transport".into());
            }
        }
    }
    Ok(Built {
        client: Arc::new(NativeHttpClient::new()?),
        kind: Kind::Native,
        notices,
    })
}

/// Builds and installs the transport the first time; later calls return
/// the installed transport with no notices.
pub fn init(config: &Config) -> Result<Built, ScopeError> {
    if let Some((client, kind)) = INSTALLED.get() {
        let mut notices = Vec::new();
        if FELL_BACK.swap(false, Ordering::SeqCst) {
            let msg = format!(
                "⚠ HTTP was used before startup; the {} transport from config was not applied",
                build(config).map_or("configured", |b| b.kind.name())
            );
            tracing::warn!("{msg}");
            notices.push(msg);
        }
        return Ok(Built {
            client: client.clone(),
            kind: *kind,
            notices,
        });
    }
    let built = build(config)?;
    match INSTALLED.set((built.client.clone(), built.kind)) {
        Ok(()) => Ok(built),
        // Another thread installed first: use its transport.
        Err(_) => init(config),
    }
}

/// The installed transport. Installs native if nothing is installed.
pub fn transport() -> Arc<dyn HttpClient> {
    if let Some((client, _)) = INSTALLED.get() {
        return client.clone();
    }
    tracing::debug!("no HTTP transport installed; using native");
    let native: Arc<dyn HttpClient> =
        Arc::new(NativeHttpClient::new().expect("failed to build HTTP client"));
    if INSTALLED.set((native, Kind::Native)).is_ok() {
        FELL_BACK.store(true, Ordering::SeqCst);
    }
    INSTALLED.get().expect("installed above").0.clone()
}

/// The installed transport kind, or `"none"`.
pub fn describe() -> &'static str {
    INSTALLED.get().map_or("none", |(_, k)| k.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn test_build_default_is_native_with_no_notices() {
        let built = build_with(&Config::default(), &no_env).unwrap();
        assert!(built.notices.is_empty());
        assert_eq!(built.kind, Kind::Native);
    }

    #[test]
    fn test_build_ghola_enabled_reports_its_choice() {
        // The test machine may or may not run a Ghola sidecar. Either way,
        // the result must say which transport it chose: Ghola with no
        // notice, or native with a notice that names Ghola.
        let mut config = Config::default();
        config.ghola.enabled = true;
        let built = build_with(&config, &no_env).unwrap();
        match built.kind {
            Kind::Ghola => assert!(built.notices.is_empty()),
            Kind::Native => assert!(built.notices.iter().any(|n| n.contains("Ghola"))),
            other => panic!("unexpected kind {other:?}"),
        }
    }
}
