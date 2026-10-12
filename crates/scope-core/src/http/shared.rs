//! # Shared HTTP transport
//!
//! The one place that selects the HTTP transport. `init` builds it from
//! config and installs it for the process. Clients that do not get a
//! transport passed in call `transport()`. See the TUI test harness
//! design, section 2.2.

use super::replay::ReplayHttpClient;
use super::{GholaHttpClient, HttpClient, NativeHttpClient};
use crate::config::Config;
use crate::error::ScopeError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

/// An RPC URL path segment this long or longer is treated as a secret.
const MIN_SECRET_SEGMENT_LEN: usize = 16;

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

/// The process-wide transport. `fell_back` is set in the same write as the
/// install, so a concurrent `init` always sees it.
struct Installed {
    client: Arc<dyn HttpClient>,
    kind: Kind,
    /// True when `transport()` installed native because nothing was installed.
    fell_back: bool,
}

static INSTALLED: OnceLock<Installed> = OnceLock::new();
/// Set once the "not applied" notice has been returned, so it is shown once.
static NOTICE_SENT: AtomicBool = AtomicBool::new(false);

/// The kind the config and environment select, derived without building a client.
pub fn configured_kind(config: &Config) -> Kind {
    configured_kind_with(config, &|k| std::env::var(k).ok())
}

/// Like `configured_kind`, reading environment variables through `env`.
pub fn configured_kind_with(config: &Config, env: &dyn Fn(&str) -> Option<String>) -> Kind {
    match env("SCOPE_HTTP_MODE").as_deref() {
        Some("replay") => Kind::Replay,
        Some("record") => Kind::Record,
        _ if config.ghola.enabled => Kind::Ghola,
        _ => Kind::Native,
    }
}

/// Builds the transport the config and environment select.
pub fn build(config: &Config) -> Result<Built, ScopeError> {
    build_with(config, &|k| std::env::var(k).ok())
}

/// Builds the transport, reading environment variables through `env`.
///
/// Order: replay or record when `SCOPE_HTTP_MODE` is set (with
/// `SCOPE_HTTP_FIXTURES`), else Ghola when enabled and available, else native.
pub fn build_with(
    config: &Config,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Built, ScopeError> {
    if let Some(mode) = env("SCOPE_HTTP_MODE") {
        let dir = env("SCOPE_HTTP_FIXTURES")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| {
                ScopeError::Network("SCOPE_HTTP_MODE is set but SCOPE_HTTP_FIXTURES is not".into())
            })?;
        return match mode.as_str() {
            "replay" => Ok(Built {
                client: Arc::new(ReplayHttpClient::replay(dir)),
                kind: Kind::Replay,
                notices: vec![],
            }),
            "record" => {
                // The normal transport (Ghola or native) does the real requests.
                // The empty env lookup stops it from selecting record again.
                let inner = build_with(config, &|_| None)?;
                Ok(Built {
                    client: Arc::new(ReplayHttpClient::record(
                        dir,
                        inner.client,
                        secret_values(config),
                    )),
                    kind: Kind::Record,
                    notices: inner.notices,
                })
            }
            other => Err(ScopeError::Network(format!(
                "unknown SCOPE_HTTP_MODE \"{other}\" (use replay or record)"
            ))),
        };
    }
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

/// Values that must never be written to a fixture.
pub fn secret_values(config: &Config) -> Vec<String> {
    let mut out: Vec<String> = config.chains.api_keys.values().cloned().collect();
    if let Some(t) = &config.web.auth_token {
        out.push(t.clone());
    }
    let rpcs = [
        &config.chains.ethereum_rpc,
        &config.chains.bsc_rpc,
        &config.chains.aegis_rpc,
        &config.chains.solana_rpc,
        &config.chains.tron_api,
    ];
    for url in rpcs.into_iter().flatten() {
        if let Ok(u) = reqwest::Url::parse(url) {
            if let Some(p) = u.password() {
                out.push(p.to_string());
            }
            out.extend(
                u.path_segments()
                    .into_iter()
                    .flatten()
                    .filter(|s| s.len() >= MIN_SECRET_SEGMENT_LEN)
                    .map(str::to_string),
            );
        }
    }
    out
}

/// Builds and installs the transport the first time; later calls return
/// the installed transport with no notices.
pub fn init(config: &Config) -> Result<Built, ScopeError> {
    if let Some(installed) = INSTALLED.get() {
        let mut notices = Vec::new();
        let wanted = configured_kind(config);
        if installed.fell_back
            && wanted != installed.kind
            && !NOTICE_SENT.swap(true, Ordering::SeqCst)
        {
            let msg = format!(
                "⚠ HTTP was used before startup; the {} transport from config was not applied",
                wanted.name()
            );
            tracing::warn!("{msg}");
            notices.push(msg);
        }
        return Ok(Built {
            client: installed.client.clone(),
            kind: installed.kind,
            notices,
        });
    }
    let built = build(config)?;
    let installed = Installed {
        client: built.client.clone(),
        kind: built.kind,
        fell_back: false,
    };
    match INSTALLED.set(installed) {
        Ok(()) => Ok(built),
        // Another thread installed first: use its transport.
        Err(_) => init(config),
    }
}

/// The installed transport. Installs native if nothing is installed.
pub fn transport() -> Arc<dyn HttpClient> {
    if let Some(installed) = INSTALLED.get() {
        return installed.client.clone();
    }
    tracing::debug!("no HTTP transport installed; using native");
    let native: Arc<dyn HttpClient> =
        Arc::new(NativeHttpClient::new().expect("failed to build HTTP client"));
    // If another thread won the race, `get` below returns its transport.
    let _ = INSTALLED.set(Installed {
        client: native,
        kind: Kind::Native,
        fell_back: true,
    });
    INSTALLED.get().expect("installed above").client.clone()
}

/// The installed transport kind, or `"none"`.
pub fn describe() -> &'static str {
    INSTALLED.get().map_or("none", |i| i.kind.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn test_configured_kind_follows_ghola_enabled() {
        assert_eq!(configured_kind(&Config::default()), Kind::Native);
        let mut config = Config::default();
        config.ghola.enabled = true;
        assert_eq!(configured_kind(&config), Kind::Ghola);
    }

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn test_configured_kind_with_env_selects_replay_and_record() {
        let c = Config::default();
        let replay = env_of(&[("SCOPE_HTTP_MODE", "replay")]);
        let record = env_of(&[("SCOPE_HTTP_MODE", "record")]);
        assert_eq!(configured_kind_with(&c, &replay), Kind::Replay);
        assert_eq!(configured_kind_with(&c, &record), Kind::Record);
        // The mode wins over ghola.
        let mut g = Config::default();
        g.ghola.enabled = true;
        assert_eq!(configured_kind_with(&g, &record), Kind::Record);
        assert_eq!(configured_kind_with(&g, &no_env), Kind::Ghola);
    }

    #[test]
    fn test_replay_mode_needs_a_fixture_dir() {
        let err = build_with(
            &Config::default(),
            &env_of(&[("SCOPE_HTTP_MODE", "replay")]),
        )
        .err()
        .unwrap();
        assert!(err.to_string().contains("SCOPE_HTTP_FIXTURES"), "{err}");
    }

    #[test]
    fn test_replay_mode_selects_replay() {
        let env = env_of(&[
            ("SCOPE_HTTP_MODE", "replay"),
            ("SCOPE_HTTP_FIXTURES", "/tmp/x"),
        ]);
        let built = build_with(&Config::default(), &env).unwrap();
        assert_eq!(built.kind, Kind::Replay);
    }

    #[test]
    fn test_record_mode_selects_record() {
        let env = env_of(&[
            ("SCOPE_HTTP_MODE", "record"),
            ("SCOPE_HTTP_FIXTURES", "/tmp/x"),
        ]);
        let built = build_with(&Config::default(), &env).unwrap();
        assert_eq!(built.kind, Kind::Record);
    }

    #[test]
    fn test_unknown_mode_is_an_error() {
        let env = env_of(&[
            ("SCOPE_HTTP_MODE", "playback"),
            ("SCOPE_HTTP_FIXTURES", "/tmp/x"),
        ]);
        let err = build_with(&Config::default(), &env).err().unwrap();
        assert!(err.to_string().contains("playback"), "{err}");
    }

    #[test]
    fn test_secret_values_include_keys_token_and_rpc_path() {
        let mut c = Config::default();
        c.chains
            .api_keys
            .insert("etherscan".into(), "ABCDEF123456".into());
        c.web.auth_token = Some("tok-secret".into());
        c.chains.ethereum_rpc = Some("https://mainnet.infura.io/v3/0123456789abcdef0123".into());
        let s = secret_values(&c);
        assert!(s.contains(&"ABCDEF123456".to_string()));
        assert!(s.contains(&"tok-secret".to_string()));
        assert!(s.contains(&"0123456789abcdef0123".to_string()));
        assert!(!s.contains(&"v3".to_string()));
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
