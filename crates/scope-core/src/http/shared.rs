//! # Shared HTTP transport
//!
//! The one place that selects the HTTP transport. `init` builds it from
//! config and installs it for the process. Clients that do not get a
//! transport passed in call `transport()`. See the TUI test harness
//! design, section 2.2.

use super::replay::ReplayHttpClient;
use super::{GholaHttpClient, HttpClient, NativeHttpClient, ghola};
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
    build_probed(config, env, &ghola::is_bridge_running)
}

/// [`build_with`] with the Ghola sidecar check injected, so tests do not
/// depend on a sidecar on the test machine. Ghola is chosen only when
/// `sidecar_up` says the sidecar answers (#64).
fn build_probed(
    config: &Config,
    env: &dyn Fn(&str) -> Option<String>,
    sidecar_up: &dyn Fn() -> bool,
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
                let inner = build_probed(config, &|_| None, sidecar_up)?;
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
        let client = if sidecar_up() {
            GholaHttpClient::new(config.ghola.stealth, config.ghola.buffer_size)
        } else {
            Err(ScopeError::Network(format!(
                "no sidecar answers on {}",
                ghola::SIDECAR_ADDR
            )))
        };
        match client {
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
///
/// # Errors
///
/// Returns an error when the transport cannot be built, or when
/// `SCOPE_HTTP_MODE` selects replay or record but `transport()` already
/// installed a different transport (the run must not use the network).
pub fn init(config: &Config) -> Result<Built, ScopeError> {
    init_with(config, &|k| std::env::var(k).ok())
}

/// Like `init`, reading environment variables through `env`.
pub fn init_with(
    config: &Config,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Built, ScopeError> {
    if let Some(installed) = INSTALLED.get() {
        let mut notices = Vec::new();
        let wanted = configured_kind_with(config, env);
        let mismatch = installed.fell_back && wanted != installed.kind;
        // A fallen-back recorder was built from the default config, so it
        // scanned for no configured secrets. Refuse it even when record is
        // also the wanted mode.
        if installed.fell_back && wanted == Kind::Record && installed.kind == Kind::Record {
            return Err(ScopeError::Network(
                "record mode started before the config was loaded, so secrets from the \
                 config could not be scanned for; the run stops"
                    .into(),
            ));
        }
        if mismatch && matches!(wanted, Kind::Replay | Kind::Record) {
            return Err(ScopeError::Network(format!(
                "HTTP was used before startup with the {} transport; SCOPE_HTTP_MODE={} \
                 was not applied, so the run stops",
                installed.kind.name(),
                wanted.name()
            )));
        }
        if mismatch && !NOTICE_SENT.swap(true, Ordering::SeqCst) {
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
    let built = build_with(config, env)?;
    let installed = Installed {
        client: built.client.clone(),
        kind: built.kind,
        fell_back: false,
    };
    match INSTALLED.set(installed) {
        Ok(()) => Ok(built),
        // Another thread installed first: use its transport.
        Err(_) => init_with(config, env),
    }
}

/// The installed transport. If nothing is installed, installs the
/// transport that the default config and the environment select, so
/// `SCOPE_HTTP_MODE` applies even before `init`.
///
/// # Panics
///
/// Panics when that transport cannot be built, for example
/// `SCOPE_HTTP_MODE=replay` with no `SCOPE_HTTP_FIXTURES`. A replay run must
/// not go to the network in its place.
pub fn transport() -> Arc<dyn HttpClient> {
    transport_with(&|k| std::env::var(k).ok())
}

/// Like `transport`, reading environment variables through `env`.
pub fn transport_with(env: &dyn Fn(&str) -> Option<String>) -> Arc<dyn HttpClient> {
    if let Some(installed) = INSTALLED.get() {
        return installed.client.clone();
    }
    let built = build_with(&Config::default(), env).unwrap_or_else(|e| {
        panic!("HTTP used before startup, and the transport could not be built: {e}")
    });
    tracing::debug!(
        "no HTTP transport installed; using {} from the environment and default config",
        built.kind.name()
    );
    // If another thread won the race, `get` below returns its transport.
    let _ = INSTALLED.set(Installed {
        client: built.client,
        kind: built.kind,
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

    fn ghola_config() -> Config {
        let mut config = Config::default();
        config.ghola.enabled = true;
        config
    }

    // #64: an enabled Ghola with no sidecar must not be selected, or every
    // request fails at send time. Native takes over, and the user is told why.
    #[test]
    fn test_build_ghola_enabled_sidecar_down_falls_back_to_native() {
        let built = build_probed(&ghola_config(), &no_env, &|| false).unwrap();
        assert_eq!(built.kind, Kind::Native);
        assert!(
            built
                .notices
                .iter()
                .any(|n| n.contains(ghola::SIDECAR_ADDR))
        );
        assert!(built.notices.iter().any(|n| n.contains("native")));
    }

    #[test]
    fn test_build_ghola_enabled_sidecar_up_selects_ghola() {
        let built = build_probed(&ghola_config(), &no_env, &|| true).unwrap();
        assert_eq!(built.kind, Kind::Ghola);
        assert!(built.notices.is_empty());
    }

    #[test]
    fn test_build_ghola_disabled_never_probes() {
        let built = build_probed(&Config::default(), &no_env, &|| {
            panic!("probed the sidecar with Ghola disabled")
        })
        .unwrap();
        assert_eq!(built.kind, Kind::Native);
    }

    // Record mode wraps the normal transport, so it must use the same probe.
    #[test]
    fn test_record_mode_falls_back_when_sidecar_down() {
        let env = env_of(&[
            ("SCOPE_HTTP_MODE", "record"),
            ("SCOPE_HTTP_FIXTURES", "/tmp/x"),
        ]);
        let built = build_probed(&ghola_config(), &env, &|| false).unwrap();
        assert_eq!(built.kind, Kind::Record);
        assert!(
            built
                .notices
                .iter()
                .any(|n| n.contains(ghola::SIDECAR_ADDR))
        );
    }
}
