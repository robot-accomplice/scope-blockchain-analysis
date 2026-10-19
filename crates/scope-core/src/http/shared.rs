//! # Shared HTTP transport
//!
//! The one place that selects the HTTP transport. `build` makes it from
//! config and the environment; the caller passes it to every client it
//! creates (see `DefaultClientFactory`). Nothing is installed for the
//! process (#59).

use super::replay::ReplayHttpClient;
use super::{GholaHttpClient, HttpClient, NativeHttpClient, ghola, ghola_install};
use crate::config::Config;
use crate::error::ScopeError;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    /// Ghola is enabled, no sidecar answers and no `ghola` binary was
    /// found. A caller that may ask the user can offer to install it
    /// (`ghola_install`) and build again.
    pub ghola_missing: bool,
}

/// The local Ghola sidecar, as the build sees it.
trait Sidecar {
    /// True when a sidecar answers on `ghola::SIDECAR_ADDR`.
    fn up(&self) -> bool;
    /// The `ghola` binary, when one is installed.
    fn binary(&self) -> Option<PathBuf>;
    /// Starts `binary --serve` and waits until it answers.
    fn start(&self, binary: &Path) -> Result<(), ScopeError>;
}

/// The real sidecar: the probe, `ghola_install::find` and `ghola::start_sidecar`.
struct LocalSidecar;

impl Sidecar for LocalSidecar {
    fn up(&self) -> bool {
        ghola::is_bridge_running()
    }
    fn binary(&self) -> Option<PathBuf> {
        ghola_install::find()
    }
    fn start(&self, binary: &Path) -> Result<(), ScopeError> {
        ghola::start_sidecar(binary, ghola::SPAWN_TIMEOUT).map(|_| ())
    }
}

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
///
/// With Ghola enabled and no sidecar answering, it starts an installed
/// `ghola --serve` (from PATH or `~/.scope-bca/bin`) and waits for it.
/// That sidecar keeps running until `ghola::stop_spawned_sidecar`.
pub fn build_with(
    config: &Config,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Built, ScopeError> {
    build_probed(config, env, &LocalSidecar)
}

/// [`build_with`] with the Ghola sidecar injected, so tests do not depend
/// on a sidecar or a binary on the test machine. Ghola is chosen only when
/// the sidecar answers, at once or after a start (#64).
fn build_probed(
    config: &Config,
    env: &dyn Fn(&str) -> Option<String>,
    sidecar: &dyn Sidecar,
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
                ghola_missing: false,
            }),
            "record" => {
                // The normal transport (Ghola or native) does the real requests.
                // The empty env lookup stops it from selecting record again.
                let inner = build_probed(config, &|_| None, sidecar)?;
                Ok(Built {
                    client: Arc::new(ReplayHttpClient::record(
                        dir,
                        inner.client,
                        secret_values(config),
                    )),
                    kind: Kind::Record,
                    notices: inner.notices,
                    ghola_missing: inner.ghola_missing,
                })
            }
            other => Err(ScopeError::Network(format!(
                "unknown SCOPE_HTTP_MODE \"{other}\" (use replay or record)"
            ))),
        };
    }
    let mut notices = Vec::new();
    let mut ghola_missing = false;
    if config.ghola.enabled {
        let client = match connect_sidecar(sidecar, &mut notices) {
            Ok(()) => GholaHttpClient::new(config.ghola.stealth, config.ghola.buffer_size),
            Err(e) => Err(e),
        };
        match client {
            Ok(client) => {
                tracing::info!("Using Ghola sidecar for HTTP transport");
                return Ok(Built {
                    client: Arc::new(client),
                    kind: Kind::Ghola,
                    notices,
                    ghola_missing,
                });
            }
            Err(e) => {
                notices.push(format!("⚠ Ghola sidecar enabled but unavailable: {e}"));
                if sidecar.binary().is_none() {
                    ghola_missing = true;
                    notices.extend(
                        ghola_install::manual_steps()
                            .iter()
                            .map(|s| format!("  {s}")),
                    );
                }
                notices.push("  Falling back to native HTTP transport".into());
            }
        }
    }
    Ok(Built {
        client: Arc::new(NativeHttpClient::new()?),
        kind: Kind::Native,
        notices,
        ghola_missing,
    })
}

/// Makes sure a sidecar answers: one already running, or an installed
/// binary started now.
fn connect_sidecar(sidecar: &dyn Sidecar, notices: &mut Vec<String>) -> Result<(), ScopeError> {
    if sidecar.up() {
        return Ok(());
    }
    let binary = sidecar.binary().ok_or_else(|| {
        ScopeError::Network(format!(
            "no sidecar answers on {} and ghola is not installed",
            ghola::SIDECAR_ADDR
        ))
    })?;
    sidecar.start(&binary)?;
    notices.push(format!(
        "Started the Ghola sidecar ({} --serve)",
        binary.display()
    ));
    Ok(())
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

/// A transport for the deprecated constructors that take none: built
/// fresh from the environment and the default config on each call, so
/// `SCOPE_HTTP_MODE` still applies. Nothing is installed or shared.
///
/// # Panics
///
/// Panics when that transport cannot be built, for example
/// `SCOPE_HTTP_MODE=replay` with no `SCOPE_HTTP_FIXTURES`. A replay run must
/// not go to the network in its place.
pub(crate) fn fallback() -> Arc<dyn HttpClient> {
    let built = build(&Config::default()).unwrap_or_else(|e| {
        panic!("a client was built without a transport, and none could be built: {e}")
    });
    tracing::debug!(
        "client built without a transport; using {} from the environment and default config",
        built.kind.name()
    );
    built.client
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

    /// A sidecar for tests: `up` answers, `binary` is installed, and a
    /// start succeeds when `starts`. It counts the starts.
    struct FakeSidecar {
        up: bool,
        binary: Option<&'static str>,
        starts: bool,
        started: std::cell::Cell<u32>,
    }

    impl FakeSidecar {
        fn new(up: bool, binary: Option<&'static str>, starts: bool) -> Self {
            Self {
                up,
                binary,
                starts,
                started: std::cell::Cell::new(0),
            }
        }
    }

    impl Sidecar for FakeSidecar {
        fn up(&self) -> bool {
            self.up
        }
        fn binary(&self) -> Option<PathBuf> {
            self.binary.map(PathBuf::from)
        }
        fn start(&self, _: &Path) -> Result<(), ScopeError> {
            self.started.set(self.started.get() + 1);
            if self.starts {
                Ok(())
            } else {
                Err(ScopeError::Network("exited".into()))
            }
        }
    }

    /// Ghola must never be probed or started when it is disabled.
    struct NoSidecar;

    impl Sidecar for NoSidecar {
        fn up(&self) -> bool {
            panic!("probed the sidecar with Ghola disabled")
        }
        fn binary(&self) -> Option<PathBuf> {
            panic!("looked for ghola with Ghola disabled")
        }
        fn start(&self, _: &Path) -> Result<(), ScopeError> {
            panic!("started ghola with Ghola disabled")
        }
    }

    // #64: an enabled Ghola with no sidecar must not be selected, or every
    // request fails at send time. Native takes over, and the user is told why.
    #[test]
    fn test_build_ghola_enabled_sidecar_down_falls_back_to_native() {
        let sidecar = FakeSidecar::new(false, None, false);
        let built = build_probed(&ghola_config(), &no_env, &sidecar).unwrap();
        assert_eq!(built.kind, Kind::Native);
        assert!(built.ghola_missing);
        assert_eq!(sidecar.started.get(), 0);
        assert!(
            built
                .notices
                .iter()
                .any(|n| n.contains(ghola::SIDECAR_ADDR))
        );
        assert!(built.notices.iter().any(|n| n.contains("native")));
        assert!(
            built
                .notices
                .iter()
                .any(|n| n.contains("/cmd/ghola@latest")),
            "{:?}",
            built.notices
        );
    }

    #[test]
    fn test_build_ghola_enabled_sidecar_up_selects_ghola() {
        let sidecar = FakeSidecar::new(true, None, false);
        let built = build_probed(&ghola_config(), &no_env, &sidecar).unwrap();
        assert_eq!(built.kind, Kind::Ghola);
        assert!(built.notices.is_empty());
        assert!(!built.ghola_missing);
        assert_eq!(sidecar.started.get(), 0);
    }

    #[test]
    fn test_build_starts_an_installed_sidecar() {
        let sidecar = FakeSidecar::new(false, Some("/home/u/.scope-bca/bin/ghola"), true);
        let built = build_probed(&ghola_config(), &no_env, &sidecar).unwrap();
        assert_eq!(built.kind, Kind::Ghola);
        assert_eq!(sidecar.started.get(), 1);
        assert!(!built.ghola_missing);
        assert!(
            built.notices.iter().any(|n| n.contains("Started")),
            "{:?}",
            built.notices
        );
    }

    #[test]
    fn test_build_falls_back_when_the_installed_sidecar_fails() {
        let sidecar = FakeSidecar::new(false, Some("/opt/ghola"), false);
        let built = build_probed(&ghola_config(), &no_env, &sidecar).unwrap();
        assert_eq!(built.kind, Kind::Native);
        // It is installed: nothing to offer.
        assert!(!built.ghola_missing);
        assert!(built.notices.iter().any(|n| n.contains("exited")));
    }

    #[test]
    fn test_build_ghola_disabled_never_probes() {
        let built = build_probed(&Config::default(), &no_env, &NoSidecar).unwrap();
        assert_eq!(built.kind, Kind::Native);
        assert!(!built.ghola_missing);
    }

    // Record mode wraps the normal transport, so it must use the same probe.
    #[test]
    fn test_record_mode_falls_back_when_sidecar_down() {
        let env = env_of(&[
            ("SCOPE_HTTP_MODE", "record"),
            ("SCOPE_HTTP_FIXTURES", "/tmp/x"),
        ]);
        let sidecar = FakeSidecar::new(false, None, false);
        let built = build_probed(&ghola_config(), &env, &sidecar).unwrap();
        assert_eq!(built.kind, Kind::Record);
        assert!(built.ghola_missing);
        assert!(
            built
                .notices
                .iter()
                .any(|n| n.contains(ghola::SIDECAR_ADDR))
        );
    }
}
