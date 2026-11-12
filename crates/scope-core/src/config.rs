//! # Configuration Management
//!
//! This module handles loading, merging, and validating configuration
//! from multiple sources with the following priority (highest to lowest):
//!
//! 1. CLI arguments
//! 2. Environment variables (`SCOPE_*` prefix)
//! 3. User config file (`~/.scope-bca/config.yaml`)
//! 4. Built-in defaults
//!
//! ## Configuration File Format
//!
//! ```yaml
//! chains:
//!   # EVM-compatible chains
//!   ethereum_rpc: "https://mainnet.infura.io/v3/YOUR_KEY"
//!   bsc_rpc: "https://bsc-dataseed.binance.org"
//!
//!   # Non-EVM chains
//!   solana_rpc: "https://api.mainnet-beta.solana.com"
//!   tron_api: "https://api.trongrid.io"
//!
//!   api_keys:
//!     etherscan: "YOUR_API_KEY"
//!     bscscan: "YOUR_API_KEY"
//!     solscan: "YOUR_API_KEY"
//!     tronscan: "YOUR_API_KEY"
//!
//! output:
//!   format: table  # table, json, csv, markdown
//!   color: true
//!
//! address_book:
//!   data_dir: "/Users/me/scope-address-book"  # optional; absolute path
//!
//! ghola:
//!   enabled: false        # route HTTP through Ghola sidecar
//!   stealth: false        # apply temporal drift + ghost signing
//!   buffer_size: 4096     # read buffer for large response headers
//!
//! market:
//!   health:               # order book health thresholds; omitted keys keep defaults
//!     min_levels: 10      # valid levels (price > 0, qty > 0) per side
//!     max_spread_pct: 3.0 # best bid/ask spread, % of mid
//!     min_top3_depth: 300
//!     min_top10_depth: 2000
//!     min_depth: 3000     # total in-band depth per side
//!     peg_target: 1.0
//!     peg_range: 0.001
//!     min_bid_ask_ratio: 0.2
//!     max_bid_ask_ratio: 5.0
//!     depth_window_pct: 2.0       # depth/volume rule: depth within ±2% of mid
//!     min_depth_volume_pct: 5.0   # below 5% of 24h volume fails
//!     max_depth_volume_pct: 10.0  # above 10% of 24h volume warns
//! ```
//!
//! ## Error Handling
//!
//! Configuration errors are wrapped in [`ScopeError::Config`] and include
//! context about which source caused the failure.

use crate::error::{ConfigError, Result, ScopeError};
use crate::market::HealthThresholds;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Application configuration.
///
/// Contains all settings for blockchain clients, output formatting,
/// and address book management. Use [`Config::load`] to load from file
/// or [`Config::default`] for sensible defaults.
///
/// # Examples
///
/// ```rust
/// use scope::Config;
///
/// // Load from default location or custom path
/// let config = Config::load(None).unwrap_or_default();
/// println!("Output format: {:?}", config.output.format);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// Blockchain chain client configuration.
    pub chains: ChainsConfig,

    /// Output formatting configuration.
    pub output: OutputConfig,

    /// Address book configuration (label → address mapping storage).
    #[serde(alias = "portfolio")]
    pub address_book: AddressBookConfig,

    /// Monitor configuration (refresh rate, alerts, venue).
    pub monitor: MonitorConfig,

    /// Ghola sidecar configuration for stealth HTTP transport.
    pub ghola: GholaConfig,

    /// Web server configuration.
    pub web: WebConfig,

    /// Market analysis configuration (order book health thresholds).
    pub market: MarketConfig,

    /// The file this config was loaded from. Commands that save the config
    /// (`setup`) write here, so `--config` and `SCOPE_CONFIG` reach them.
    /// Not part of the YAML.
    #[serde(skip)]
    pub loaded_from: Option<PathBuf>,
}

/// Blockchain client configuration.
///
/// Contains RPC endpoints and API keys for various blockchain networks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ChainsConfig {
    // =========================================================================
    // EVM-Compatible Chains
    // =========================================================================
    /// Ethereum JSON-RPC endpoint URL.
    ///
    /// Example: `https://mainnet.infura.io/v3/YOUR_PROJECT_ID`
    pub ethereum_rpc: Option<String>,

    /// BSC (BNB Smart Chain) JSON-RPC endpoint URL.
    ///
    /// Example: `https://bsc-dataseed.binance.org`
    pub bsc_rpc: Option<String>,

    /// Custom EVM chain JSON-RPC endpoint (reserved for internal use).
    #[doc(hidden)]
    pub aegis_rpc: Option<String>,

    // =========================================================================
    // Non-EVM Chains
    // =========================================================================
    /// Solana JSON-RPC endpoint URL.
    ///
    /// Example: `https://api.mainnet-beta.solana.com`
    pub solana_rpc: Option<String>,

    /// Tron API endpoint URL (TronGrid).
    ///
    /// Example: `https://api.trongrid.io`
    pub tron_api: Option<String>,

    // =========================================================================
    // API Keys
    // =========================================================================
    /// API keys for block explorer services.
    ///
    /// Keys are service names (e.g., "etherscan", "polygonscan", "bscscan",
    /// "solscan", "tronscan").
    pub api_keys: HashMap<String, String>,
}

/// Output formatting configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    /// Output format for analysis results.
    pub format: OutputFormat,

    /// Whether to use colored output in the terminal.
    pub color: bool,
}

/// Address book configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AddressBookConfig {
    /// Directory for storing address book data.
    ///
    /// Defaults to `~/.scope-bca/data` (see [`crate::paths`]). Use an absolute
    /// path: `~` is not expanded.
    pub data_dir: Option<PathBuf>,
}

/// Ghola sidecar configuration.
///
/// Controls whether HTTP requests are routed through the
/// [Ghola](https://github.com/robot-accomplice/ghola) sidecar for
/// stealth analysis (temporal drift, ghost signing, chain-aware headers).
///
/// Ghola is an external Go binary; all fields default to `false` / sensible
/// values so existing installations see no behavior change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GholaConfig {
    /// When `true`, route HTTP requests through the Ghola sidecar
    /// (`127.0.0.1:18789`) instead of using `reqwest` directly.
    pub enabled: bool,

    /// When `true` (and `enabled` is `true`), the sidecar applies
    /// temporal drift and ghost signing to every outgoing request.
    pub stealth: bool,

    /// Read buffer size (in bytes) for handling large response headers.
    /// Passed to the Ghola sidecar which uses it as `fasthttp.Client.ReadBufferSize`.
    /// Default: 4096. Increase for APIs that return very large headers.
    pub buffer_size: u32,
}

impl Default for GholaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            stealth: false,
            buffer_size: 4096,
        }
    }
}

/// Market analysis configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MarketConfig {
    /// Order book health thresholds. Keys left out keep the built-in default.
    pub health: HealthThresholds,
    /// Where `scope venues update` fetches the venue catalog from (a folder
    /// URL holding `index.json`). Unset: the scope repository's catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub venue_catalog_url: Option<String>,
}

impl MarketConfig {
    /// The venue catalog's URL: the configured one, else the default.
    pub fn catalog_url(&self) -> &str {
        self.venue_catalog_url
            .as_deref()
            .filter(|u| !u.trim().is_empty())
            .unwrap_or(crate::market::catalog::DEFAULT_CATALOG_URL)
    }
}

/// Web server configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WebConfig {
    /// Optional bearer token for API authentication.
    /// When set, all `/api/*` endpoints require `Authorization: Bearer <token>`.
    /// When not set, no authentication is required (localhost-only use case).
    pub auth_token: Option<String>,
}

/// Available output formats for analysis results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Human-readable table format (default).
    #[default]
    Table,

    /// JSON format for programmatic consumption.
    Json,

    /// CSV format for spreadsheet import.
    Csv,

    /// Markdown format for agent parsing (console output).
    #[value(name = "markdown")]
    Markdown,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            format: OutputFormat::Table,
            color: true,
        }
    }
}

impl Config {
    /// Loads configuration from a file path or the default location.
    ///
    /// # Arguments
    ///
    /// * `path` - Optional path to a configuration file. If `None`, looks
    ///   for config at `~/.scope-bca/config.yaml`.
    ///
    /// # Returns
    ///
    /// Returns the loaded configuration, or defaults if no config file exists.
    ///
    /// # Errors
    ///
    /// Returns [`ScopeError::Config`] if the file exists but cannot be read
    /// or contains invalid YAML.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use scope::Config;
    /// use std::path::Path;
    ///
    /// // Load from default location
    /// let config = Config::load(None).unwrap_or_default();
    ///
    /// // Load from custom path
    /// let config = Config::load(Some(Path::new("/custom/config.yaml")));
    /// ```
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let config_path = Self::resolve_path(path);

        // Return defaults if no config file exists
        // This allows first-run without manual setup
        if !config_path.exists() {
            tracing::debug!(
                path = %config_path.display(),
                "No config file found, using defaults"
            );
            return Ok(Self {
                loaded_from: Some(config_path),
                ..Self::default()
            });
        }

        tracing::debug!(path = %config_path.display(), "Loading configuration");

        let contents = std::fs::read_to_string(&config_path).map_err(|e| {
            ScopeError::Config(ConfigError::Read {
                path: config_path.clone(),
                source: e,
            })
        })?;

        let mut config: Config =
            serde_saphyr::from_str(&contents).map_err(|e| ConfigError::Parse { source: e })?;
        config.loaded_from = Some(config_path);

        report_dropped_monitor_keys(
            &mut crate::diag::Stderr,
            &dropped_monitor_keys(&contents),
            path,
            &DROPPED_KEYS_REPORTED,
        );

        Ok(config)
    }

    /// The one rule for which config file to use: `path` (the `--config`
    /// flag), else the `SCOPE_CONFIG` variable, else `<home>/config.yaml`.
    pub fn resolve_path(path: Option<&Path>) -> PathBuf {
        Self::resolve_path_with(path, std::env::var_os("SCOPE_CONFIG"))
    }

    /// [`Config::resolve_path`] with the `SCOPE_CONFIG` value given, so a
    /// test needs no process-wide variable. An empty value is no value.
    pub fn resolve_path_with(path: Option<&Path>, scope_config: Option<OsString>) -> PathBuf {
        path.map(PathBuf::from)
            .or_else(|| scope_config.filter(|v| !v.is_empty()).map(PathBuf::from))
            .unwrap_or_else(Self::default_path)
    }

    /// The file this config was loaded from. A config that did not come
    /// from [`Config::load`] gets the file `load(None)` would read.
    pub fn file(&self) -> PathBuf {
        self.loaded_from
            .clone()
            .unwrap_or_else(|| Self::resolve_path(None))
    }

    /// Returns the default configuration file path: `<home>/config.yaml`
    /// (see [`crate::paths`]).
    pub fn default_path() -> PathBuf {
        crate::paths::config_file()
    }

    /// Returns the default data directory for address book storage:
    /// `<home>/data`.
    pub fn default_data_dir() -> PathBuf {
        crate::paths::data_dir()
    }

    /// Returns the effective data directory, using config or default. A
    /// leading `~` in the config is the user's home folder.
    pub fn data_dir(&self) -> PathBuf {
        match &self.address_book.data_dir {
            Some(dir) => expand_home(dir, crate::paths::user_home()),
            None => Self::default_data_dir(),
        }
    }
}

/// `dir` with a leading `~` as `home`. YAML has no shell to expand it, so
/// `data_dir: "~/x"` made a folder named `~` (#106).
fn expand_home(dir: &Path, home: Option<PathBuf>) -> PathBuf {
    match (dir.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => dir.to_path_buf(),
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputFormat::Table => write!(f, "table"),
            OutputFormat::Json => write!(f, "json"),
            OutputFormat::Csv => write!(f, "csv"),
            OutputFormat::Markdown => write!(f, "markdown"),
        }
    }
}

// ============================================================================
// Monitor Configuration Types
// ============================================================================
//
// These types are serialized as part of the main Config struct, so they must
// live in scope-core.

/// Default refresh interval in seconds.
pub const DEFAULT_MONITOR_REFRESH_SECS: u64 = 5;

/// Color scheme for the monitor TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ColorScheme {
    /// Classic green/red (default).
    #[default]
    GreenRed,
    /// Blue/orange, better for certain color blindness.
    BlueOrange,
    /// Monochrome -- fully accessible grayscale.
    Monochrome,
}

impl ColorScheme {
    /// Cycles to the next color scheme.
    pub fn next(&self) -> Self {
        match self {
            ColorScheme::GreenRed => ColorScheme::BlueOrange,
            ColorScheme::BlueOrange => ColorScheme::Monochrome,
            ColorScheme::Monochrome => ColorScheme::GreenRed,
        }
    }

    /// Returns a short display label.
    pub fn label(&self) -> &'static str {
        match self {
            ColorScheme::GreenRed => "green-red",
            ColorScheme::BlueOrange => "blue-orange",
            ColorScheme::Monochrome => "mono",
        }
    }
}

/// Alert configuration for price and whale detection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AlertConfig {
    /// Minimum price threshold; alert fires when price drops below this.
    pub price_min: Option<f64>,
    /// Maximum price threshold; alert fires when price exceeds this.
    pub price_max: Option<f64>,
    /// Minimum USD value for whale transaction detection.
    pub whale_min_usd: Option<f64>,
    /// Volume spike threshold as a percentage increase from the rolling average.
    pub volume_spike_threshold_pct: Option<f64>,
}

impl Default for AlertConfig {
    #[allow(clippy::derivable_impls)]
    fn default() -> Self {
        Self {
            price_min: None,
            price_max: None,
            whale_min_usd: None,
            volume_spike_threshold_pct: None,
        }
    }
}

/// The `monitor:` keys of the removed dashboard. A config that has them
/// still loads; [`dropped_monitor_keys`] names them for the notice, and
/// `scope setup --clean` ([`crate::config_clean`]) removes them. This is the
/// one list.
pub const DROPPED_MONITOR_KEYS: [&str; 6] = [
    "layout",
    "widgets",
    "scale",
    "color_scheme",
    "export",
    "auto_pause_on_input",
];

/// The dropped dashboard keys found in the `monitor:` section of a config
/// file's text, in the order layout, widgets, scale, color_scheme, export,
/// auto_pause_on_input. Empty when there is no `monitor:` section, none of them, or no parse.
pub fn dropped_monitor_keys(contents: &str) -> Vec<&'static str> {
    #[derive(Deserialize)]
    struct Probe {
        monitor: Option<std::collections::BTreeMap<String, serde::de::IgnoredAny>>,
    }
    let Ok(Probe {
        monitor: Some(keys),
    }) = serde_saphyr::from_str::<Probe>(contents)
    else {
        return Vec::new();
    };
    DROPPED_MONITOR_KEYS
        .into_iter()
        .filter(|k| keys.contains_key(*k))
        .collect()
}

/// Set when the process has told the user about dropped monitor keys.
static DROPPED_KEYS_REPORTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The command that removes the dropped keys from the config file. With
/// `--config`, it names that file.
pub fn clean_command(flag: Option<&Path>) -> String {
    match flag {
        Some(path) => format!("scope --config \"{}\" setup --clean", path.display()),
        None => "scope setup --clean".to_string(),
    }
}

/// The text of the notice for the dropped monitor `keys`. `flag` is the
/// `--config` path, when there is one.
pub fn dropped_keys_notice(keys: &[&str], flag: Option<&Path>) -> String {
    format!(
        "scope: ignoring config keys of the removed monitor dashboard under `monitor:`: {}. Run `{}` to remove them.",
        keys.join(", "),
        clean_command(flag)
    )
}

/// Writes one line to `out` that names the dropped monitor `keys`, the first
/// time it is called with some keys for `reported`. Later calls write
/// nothing, so a config that loads again (after `setup`) does not repeat
/// it. The line goes to the diagnostics writer, never to stdout.
fn report_dropped_monitor_keys(
    out: &mut impl std::io::Write,
    keys: &[&str],
    flag: Option<&Path>,
    reported: &std::sync::atomic::AtomicBool,
) {
    if keys.is_empty() || reported.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let _ = writeln!(out, "{}", dropped_keys_notice(keys, flag));
}

/// Monitor-specific configuration.
///
/// Loaded from the `monitor:` section of `~/.scope-bca/config.yaml`.
/// All fields have sensible defaults so the section is entirely optional.
/// The keys of the removed dashboard (`layout`, `widgets`, `scale`,
/// `color_scheme`, `export`, `auto_pause_on_input`) are ignored. Run
/// `scope setup --clean` to remove them from the file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MonitorConfig {
    /// Refresh interval in seconds.
    pub refresh_seconds: u64,
    /// Alert thresholds (price min/max, whale detection). The monitor wizard
    /// turns them into watchlist rules.
    pub alerts: AlertConfig,
    /// Exchange venue for the monitor (e.g., "binance").
    #[serde(default)]
    pub venue: Option<String>,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            refresh_seconds: DEFAULT_MONITOR_REFRESH_SECS,
            alerts: AlertConfig::default(),
            venue: None,
        }
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_a_leading_tilde_in_data_dir_is_the_home_folder() {
        let home = Some(PathBuf::from("/home/u"));
        assert_eq!(
            expand_home(Path::new("~/.scope-bca/data"), home.clone()),
            PathBuf::from("/home/u").join(".scope-bca/data")
        );
        assert_eq!(
            expand_home(Path::new("~"), home.clone()),
            PathBuf::from("/home/u")
        );
        // Only a whole `~` component, and only at the start.
        assert_eq!(
            expand_home(Path::new("~x/d"), home.clone()),
            PathBuf::from("~x/d")
        );
        assert_eq!(
            expand_home(Path::new("/a/~/d"), home),
            PathBuf::from("/a/~/d")
        );
        // No home folder known: the path stays as written.
        assert_eq!(expand_home(Path::new("~/d"), None), PathBuf::from("~/d"));
    }

    #[test]
    fn test_a_config_with_the_dashboard_keys_loads_and_names_them() {
        // Configs written for 0.7 and 0.8 have these keys. The removal
        // must not turn them into a parse error.
        let yaml = "monitor:
  layout: feed
  refresh_seconds: 7
  scale: log
  color_scheme: monochrome
  auto_pause_on_input: true
  widgets:
    price_chart: false
  export:
    path: out
  venue: binance
";
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();
        let config = Config::load(Some(file.path())).unwrap();
        assert_eq!(config.monitor.refresh_seconds, 7);
        assert_eq!(config.monitor.venue.as_deref(), Some("binance"));
        assert_eq!(
            dropped_monitor_keys(yaml),
            [
                "layout",
                "widgets",
                "scale",
                "color_scheme",
                "export",
                "auto_pause_on_input"
            ]
        );
    }

    #[test]
    fn test_the_dropped_keys_are_named_once_on_the_diagnostics_writer() {
        use std::sync::atomic::AtomicBool;
        let reported = AtomicBool::new(false);
        let mut out = Vec::new();
        // No keys: nothing is said, and the one report is not used up.
        report_dropped_monitor_keys(&mut out, &[], None, &reported);
        assert!(out.is_empty());
        report_dropped_monitor_keys(&mut out, &["layout", "scale"], None, &reported);
        report_dropped_monitor_keys(&mut out, &["layout", "scale"], None, &reported);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.contains("layout, scale"), "{text}");
        assert!(text.contains("ignoring"), "{text}");
        // The notice is the only way users find the command.
        assert!(
            text.ends_with(
                "Run `scope setup --clean` to remove them.
"
            ),
            "{text}"
        );
    }

    #[test]
    fn test_the_notice_names_the_config_flag_file() {
        // With `--config X`, a bare `scope setup --clean` would clean
        // another file. The hint must carry the path.
        let text = dropped_keys_notice(&["layout"], Some(Path::new("/etc/my scope.yaml")));
        assert!(
            text.contains("`scope --config \"/etc/my scope.yaml\" setup --clean`"),
            "{text}"
        );
    }

    #[test]
    fn test_a_config_without_dashboard_keys_has_nothing_to_warn_about() {
        assert!(
            dropped_monitor_keys(
                "monitor:
  refresh_seconds: 7
"
            )
            .is_empty()
        );
        assert!(
            dropped_monitor_keys(
                "output:
  color: true
"
            )
            .is_empty()
        );
        assert!(dropped_monitor_keys("not: [valid").is_empty());
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();

        assert!(config.chains.api_keys.is_empty());
        assert!(config.chains.ethereum_rpc.is_none());
        assert!(config.chains.bsc_rpc.is_none());
        assert!(config.chains.aegis_rpc.is_none());
        assert!(config.chains.solana_rpc.is_none());
        assert!(config.chains.tron_api.is_none());
        assert_eq!(config.output.format, OutputFormat::Table);
        assert!(config.output.color);
        assert!(config.address_book.data_dir.is_none());
    }

    #[test]
    fn test_load_from_yaml_full() {
        let yaml = r#"
chains:
  ethereum_rpc: "https://example.com/rpc"
  bsc_rpc: "https://bsc-dataseed.binance.org"
  solana_rpc: "https://api.mainnet-beta.solana.com"
  tron_api: "https://api.trongrid.io"
  api_keys:
    etherscan: "test-api-key"
    polygonscan: "another-key"
    bscscan: "bsc-key"
    solscan: "sol-key"
    tronscan: "tron-key"

output:
  format: json
  color: false

address_book:
  data_dir: "/custom/data"
"#;

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let config = Config::load(Some(file.path())).unwrap();

        // EVM chains
        assert_eq!(
            config.chains.ethereum_rpc,
            Some("https://example.com/rpc".into())
        );
        assert_eq!(
            config.chains.bsc_rpc,
            Some("https://bsc-dataseed.binance.org".into())
        );

        // Non-EVM chains
        assert_eq!(
            config.chains.solana_rpc,
            Some("https://api.mainnet-beta.solana.com".into())
        );
        assert_eq!(
            config.chains.tron_api,
            Some("https://api.trongrid.io".into())
        );

        // API keys
        assert_eq!(
            config.chains.api_keys.get("etherscan"),
            Some(&"test-api-key".into())
        );
        assert_eq!(
            config.chains.api_keys.get("polygonscan"),
            Some(&"another-key".into())
        );
        assert_eq!(
            config.chains.api_keys.get("bscscan"),
            Some(&"bsc-key".into())
        );
        assert_eq!(
            config.chains.api_keys.get("solscan"),
            Some(&"sol-key".into())
        );
        assert_eq!(
            config.chains.api_keys.get("tronscan"),
            Some(&"tron-key".into())
        );

        assert_eq!(config.output.format, OutputFormat::Json);
        assert!(!config.output.color);
        assert_eq!(
            config.address_book.data_dir,
            Some(PathBuf::from("/custom/data"))
        );
    }

    #[test]
    fn test_load_partial_yaml_uses_defaults() {
        let yaml = r#"
chains:
  ethereum_rpc: "https://partial.example.com"
"#;

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let config = Config::load(Some(file.path())).unwrap();

        // Specified value
        assert_eq!(
            config.chains.ethereum_rpc,
            Some("https://partial.example.com".into())
        );

        // Defaults
        assert!(config.chains.api_keys.is_empty());
        assert_eq!(config.output.format, OutputFormat::Table);
        assert!(config.output.color);
        assert_eq!(config.market.health, HealthThresholds::default());
    }

    #[test]
    fn test_load_market_health_overrides_only_given_keys() {
        let yaml = r#"
market:
  health:
    min_levels: 20
    max_spread_pct: 1.5
"#;

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let config = Config::load(Some(file.path())).unwrap();
        let health = &config.market.health;

        assert_eq!(health.min_levels, 20);
        assert_eq!(health.max_spread_pct, 1.5);
        // Keys not in the file keep the built-in default.
        let defaults = HealthThresholds::default();
        assert_eq!(health.min_top3_depth, defaults.min_top3_depth);
        assert_eq!(health.min_top10_depth, defaults.min_top10_depth);
        assert_eq!(health.min_depth, defaults.min_depth);
    }

    #[test]
    fn test_path_rule_is_flag_then_variable_then_home() {
        // One rule decides the file for load, setup and clean. If the
        // variable beat the flag, `--config` would be ignored; an empty
        // variable must not become the path "".
        let flag = Path::new("flag.yaml");
        let var = || Some(OsString::from("var.yaml"));
        assert_eq!(Config::resolve_path_with(Some(flag), var()), flag);
        assert_eq!(
            Config::resolve_path_with(None, var()),
            PathBuf::from("var.yaml")
        );
        assert_eq!(
            Config::resolve_path_with(None, Some(OsString::new())),
            Config::default_path()
        );
        assert_eq!(
            Config::resolve_path_with(None, None),
            Config::default_path()
        );
    }

    #[test]
    fn test_load_remembers_its_file_even_when_the_file_is_missing() {
        // `setup` saves to `config.file()`. If a missing --config file fell
        // back to the home path, the first save would go to the wrong file.
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope.yaml");
        let config = Config::load(Some(&missing)).unwrap();
        assert_eq!(config.file(), missing);
        let present = tmp.path().join("here.yaml");
        std::fs::write(
            &present,
            "output:
  color: false
",
        )
        .unwrap();
        assert_eq!(Config::load(Some(&present)).unwrap().file(), present);
    }

    #[test]
    fn test_load_missing_file_returns_defaults() {
        let path = Path::new("/nonexistent/path/config.yaml");
        let config = Config::load(Some(path)).unwrap();
        let defaults = Config {
            loaded_from: Some(path.to_path_buf()),
            ..Config::default()
        };
        assert_eq!(config, defaults);
    }

    #[test]
    fn test_load_invalid_yaml_returns_error() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"invalid: yaml: : content: [").unwrap();

        let result = Config::load(Some(file.path()));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ScopeError::Config(_)));
    }

    #[test]
    fn test_load_empty_file_returns_defaults() {
        let file = NamedTempFile::new().unwrap();
        // Empty file

        let config = Config::load(Some(file.path())).unwrap();
        let defaults = Config {
            loaded_from: Some(file.path().to_path_buf()),
            ..Config::default()
        };
        assert_eq!(config, defaults);
    }

    #[test]
    fn test_output_format_serialization() {
        let json_format = OutputFormat::Json;
        let serialized = serde_saphyr::to_string(&json_format).unwrap();
        assert!(serialized.contains("json"));

        let deserialized: OutputFormat = serde_saphyr::from_str("csv").unwrap();
        assert_eq!(deserialized, OutputFormat::Csv);
    }

    #[test]
    fn test_output_format_display() {
        assert_eq!(OutputFormat::Table.to_string(), "table");
        assert_eq!(OutputFormat::Json.to_string(), "json");
        assert_eq!(OutputFormat::Csv.to_string(), "csv");
        assert_eq!(OutputFormat::Markdown.to_string(), "markdown");
    }

    #[test]
    fn test_default_path_is_absolute_or_relative() {
        // The config file is in the one scope home.
        assert_eq!(Config::default_path(), crate::paths::config_file());
        assert!(Config::default_path().ends_with(".scope-bca/config.yaml"));
    }

    #[test]
    fn test_default_data_dir() {
        let data_dir = Config::default_data_dir();
        assert!(data_dir.ends_with("scope") || data_dir.to_string_lossy().contains("scope"));
    }

    #[test]
    fn test_data_dir_uses_config_value() {
        let config = Config {
            address_book: AddressBookConfig {
                data_dir: Some(PathBuf::from("/custom/path")),
            },
            ..Default::default()
        };

        assert_eq!(config.data_dir(), PathBuf::from("/custom/path"));
    }

    #[test]
    fn test_data_dir_falls_back_to_default() {
        let config = Config::default();
        assert_eq!(config.data_dir(), Config::default_data_dir());
    }

    #[test]
    fn test_config_clone_and_eq() {
        let config1 = Config::default();
        let config2 = config1.clone();
        assert_eq!(config1, config2);
    }

    #[test]
    fn test_config_debug() {
        let config = Config::default();
        let debug = format!("{:?}", config);
        assert!(debug.contains("Config"));
        assert!(debug.contains("ChainsConfig"));
    }

    #[test]
    fn test_output_config_default() {
        let output = OutputConfig::default();
        assert_eq!(output.format, OutputFormat::Table);
        assert!(output.color);
    }

    #[test]
    fn test_config_serialization_roundtrip() {
        let mut config = Config::default();
        config
            .chains
            .api_keys
            .insert("etherscan".to_string(), "test_key".to_string());
        config.output.format = OutputFormat::Json;
        config.output.color = false;
        config.address_book.data_dir = Some(PathBuf::from("/custom"));

        let yaml = serde_saphyr::to_string(&config).unwrap();
        let deserialized: Config = serde_saphyr::from_str(&yaml).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_chains_config_with_multiple_api_keys() {
        let mut api_keys = HashMap::new();
        api_keys.insert("etherscan".into(), "key1".into());
        api_keys.insert("polygonscan".into(), "key2".into());
        api_keys.insert("bscscan".into(), "key3".into());

        let chains = ChainsConfig {
            ethereum_rpc: Some("https://rpc.example.com".into()),
            api_keys,
            ..Default::default()
        };

        assert_eq!(chains.api_keys.len(), 3);
        assert!(chains.api_keys.contains_key("etherscan"));
    }

    #[test]
    fn test_load_via_scope_config_env_var() {
        let yaml = r#"
chains:
  ethereum_rpc: "https://env-test.example.com"
output:
  format: csv
"#;
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let path_str = file.path().to_string_lossy().to_string();
        unsafe { std::env::set_var("SCOPE_CONFIG", &path_str) };

        // Load with None path — should pick up the env var
        let config = Config::load(None).unwrap();
        assert_eq!(
            config.chains.ethereum_rpc,
            Some("https://env-test.example.com".into())
        );
        assert_eq!(config.output.format, OutputFormat::Csv);

        unsafe { std::env::remove_var("SCOPE_CONFIG") };
    }

    #[test]
    fn test_output_format_default() {
        let fmt = OutputFormat::default();
        assert_eq!(fmt, OutputFormat::Table);
    }

    #[test]
    fn test_address_book_config_default() {
        let port = AddressBookConfig::default();
        assert!(port.data_dir.is_none());
    }

    #[test]
    fn test_chains_config_default() {
        let chains = ChainsConfig::default();
        assert!(chains.ethereum_rpc.is_none());
        assert!(chains.bsc_rpc.is_none());
        assert!(chains.aegis_rpc.is_none());
        assert!(chains.solana_rpc.is_none());
        assert!(chains.tron_api.is_none());
        assert!(chains.api_keys.is_empty());
    }

    #[test]
    fn test_load_unreadable_file_returns_error() {
        // Create a directory where a file is expected to trigger read error
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        // Create it as a directory so reading it as a file fails
        std::fs::create_dir_all(&config_path).unwrap();
        let result = Config::load(Some(&config_path));
        assert!(result.is_err());
    }

    #[test]
    fn test_default_path_returns_valid_path() {
        let path = Config::default_path();
        // Should always return some path (may or may not exist)
        assert!(path.to_str().unwrap().contains("scope"));
    }

    // ========================================================================
    // Ghola Configuration Tests
    // ========================================================================

    #[test]
    fn test_ghola_config_default() {
        let ghola = GholaConfig::default();
        assert!(!ghola.enabled);
        assert!(!ghola.stealth);
        assert_eq!(ghola.buffer_size, 4096);
    }

    #[test]
    fn test_config_default_ghola_disabled() {
        let config = Config::default();
        assert!(!config.ghola.enabled);
        assert!(!config.ghola.stealth);
        assert_eq!(config.ghola.buffer_size, 4096);
    }

    #[test]
    fn test_load_ghola_enabled() {
        let yaml = r#"
ghola:
  enabled: true
  stealth: true
  buffer_size: 8192
"#;
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let config = Config::load(Some(file.path())).unwrap();
        assert!(config.ghola.enabled);
        assert!(config.ghola.stealth);
        assert_eq!(config.ghola.buffer_size, 8192);
    }

    #[test]
    fn test_load_ghola_partial() {
        let yaml = r#"
ghola:
  enabled: true
"#;
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let config = Config::load(Some(file.path())).unwrap();
        assert!(config.ghola.enabled);
        assert!(!config.ghola.stealth); // defaults to false
        assert_eq!(config.ghola.buffer_size, 4096); // defaults to 4096
    }

    #[test]
    fn test_load_ghola_absent_uses_defaults() {
        let yaml = r#"
chains:
  ethereum_rpc: "https://example.com"
"#;
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let config = Config::load(Some(file.path())).unwrap();
        assert!(!config.ghola.enabled);
        assert!(!config.ghola.stealth);
        assert_eq!(config.ghola.buffer_size, 4096);
    }

    #[test]
    fn test_load_ghola_custom_buffer_size() {
        let yaml = r#"
ghola:
  enabled: false
  buffer_size: 16384
"#;
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let config = Config::load(Some(file.path())).unwrap();
        assert!(!config.ghola.enabled);
        assert_eq!(config.ghola.buffer_size, 16384);
    }

    #[test]
    fn test_ghola_config_serialization_roundtrip() {
        let mut config = Config::default();
        config.ghola.enabled = true;
        config.ghola.stealth = true;
        config.ghola.buffer_size = 8192;

        let yaml = serde_saphyr::to_string(&config).unwrap();
        let deserialized: Config = serde_saphyr::from_str(&yaml).unwrap();
        assert_eq!(config.ghola, deserialized.ghola);
    }

    #[test]
    fn test_the_venue_catalog_url_defaults_unless_set() {
        let mut market = MarketConfig::default();
        assert_eq!(
            market.catalog_url(),
            crate::market::catalog::DEFAULT_CATALOG_URL
        );
        market.venue_catalog_url = Some("  ".into());
        assert_eq!(
            market.catalog_url(),
            crate::market::catalog::DEFAULT_CATALOG_URL
        );
        market.venue_catalog_url = Some("https://example.com/cat".into());
        assert_eq!(market.catalog_url(), "https://example.com/cat");
        let read: Config =
            serde_saphyr::from_str("market:\n  venue_catalog_url: https://x/c\n").unwrap();
        assert_eq!(read.market.catalog_url(), "https://x/c");
    }
}
