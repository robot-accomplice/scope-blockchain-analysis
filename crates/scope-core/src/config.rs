//! # Configuration Management
//!
//! This module handles loading, merging, and validating configuration
//! from multiple sources with the following priority (highest to lowest):
//!
//! 1. CLI arguments
//! 2. Environment variables (`SCOPE_*` prefix)
//! 3. User config file (`~/.config/scope/config.yaml`)
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
//!   data_dir: "~/.local/share/scope"
//!
//! ghola:
//!   enabled: false        # route HTTP through Ghola sidecar
//!   stealth: false        # apply temporal drift + ghost signing
//!   buffer_size: 4096     # read buffer for large response headers
//! ```
//!
//! ## Error Handling
//!
//! Configuration errors are wrapped in [`ScopeError::Config`] and include
//! context about which source caused the failure.

use crate::error::{ConfigError, Result, ScopeError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

    /// Monitor TUI configuration (layout, widgets, refresh rate).
    pub monitor: MonitorConfig,

    /// Ghola sidecar configuration for stealth HTTP transport.
    pub ghola: GholaConfig,

    /// Web server configuration.
    pub web: WebConfig,
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
    /// Defaults to `~/.local/share/scope` on Linux/macOS.
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
    ///   for config at `~/.config/scope/config.yaml`.
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
        // Determine config path: CLI arg > env var > default location
        let config_path = path
            .map(PathBuf::from)
            .or_else(|| std::env::var("SCOPE_CONFIG").ok().map(PathBuf::from))
            .unwrap_or_else(Self::default_path);

        // Return defaults if no config file exists
        // This allows first-run without manual setup
        if !config_path.exists() {
            tracing::debug!(
                path = %config_path.display(),
                "No config file found, using defaults"
            );
            return Ok(Self::default());
        }

        tracing::debug!(path = %config_path.display(), "Loading configuration");

        let contents = std::fs::read_to_string(&config_path).map_err(|e| {
            ScopeError::Config(ConfigError::Read {
                path: config_path.clone(),
                source: e,
            })
        })?;

        let config: Config =
            serde_yaml::from_str(&contents).map_err(|e| ConfigError::Parse { source: e })?;

        Ok(config)
    }

    /// Returns the default configuration file path.
    ///
    /// Checks multiple locations in order:
    /// 1. `~/.config/scope/config.yaml` (XDG-style, preferred for CLI tools)
    /// 2. Platform-specific config dir (e.g., `~/Library/Application Support/` on macOS)
    ///
    /// Returns the first location that exists, or the XDG-style path if neither exists.
    pub fn default_path() -> PathBuf {
        // Prefer XDG-style ~/.config/scope/ which is common for CLI tools
        if let Some(home) = dirs::home_dir() {
            let xdg_path = home.join(".config").join("scope").join("config.yaml");
            if xdg_path.exists() {
                return xdg_path;
            }
        }

        // Check platform-specific config dir
        if let Some(config_dir) = dirs::config_dir() {
            let platform_path = config_dir.join("scope").join("config.yaml");
            if platform_path.exists() {
                return platform_path;
            }
        }

        // Default to XDG-style path for new configs
        dirs::home_dir()
            .map(|h| h.join(".config").join("scope").join("config.yaml"))
            .unwrap_or_else(|| PathBuf::from(".").join("scope").join("config.yaml"))
    }

    /// Returns the configuration file path, if it can be determined.
    ///
    /// Returns `Some(path)` for the config file location, or `None` if
    /// the path cannot be determined (e.g., no home directory).
    /// Prefers the XDG-style `~/.config/scope/` location.
    pub fn config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".config").join("scope").join("config.yaml"))
    }

    /// Returns the default data directory for address book storage.
    ///
    /// Uses `~/.config/scope/data` on all platforms for consistency.
    pub fn default_data_dir() -> PathBuf {
        dirs::home_dir()
            .map(|h| h.join(".config").join("scope").join("data"))
            .unwrap_or_else(|| PathBuf::from(".config").join("scope").join("data"))
    }

    /// Returns the effective data directory, using config or default.
    pub fn data_dir(&self) -> PathBuf {
        self.address_book
            .data_dir
            .clone()
            .unwrap_or_else(Self::default_data_dir)
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
// live in scope-core.  The TUI-only types (DataPoint, OhlcCandle, ChartMode,
// TimePeriod, ColorPalette, etc.) remain in scope-cli's `cli::monitor::config`.

/// Default refresh interval in seconds.
pub const DEFAULT_MONITOR_REFRESH_SECS: u64 = 5;

/// Layout preset for the monitor TUI.
///
/// Controls which widgets are shown and how they are arranged.
/// Can be switched at runtime with keybindings or set via config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutPreset {
    /// Balanced 2x2 grid with all widgets visible.
    #[default]
    Dashboard,
    /// Price chart takes ~85% of the screen; minimal stats overlay.
    ChartFocus,
    /// Transaction/activity feed prioritized; small price ticker.
    Feed,
    /// Minimal single-column sparkline view for small terminals.
    Compact,
    /// Exchange-style view: order book + chart + market info.
    Exchange,
}

impl LayoutPreset {
    /// Cycles to the next layout preset.
    pub fn next(&self) -> Self {
        match self {
            LayoutPreset::Dashboard => LayoutPreset::ChartFocus,
            LayoutPreset::ChartFocus => LayoutPreset::Feed,
            LayoutPreset::Feed => LayoutPreset::Compact,
            LayoutPreset::Compact => LayoutPreset::Exchange,
            LayoutPreset::Exchange => LayoutPreset::Dashboard,
        }
    }

    /// Cycles to the previous layout preset.
    pub fn prev(&self) -> Self {
        match self {
            LayoutPreset::Dashboard => LayoutPreset::Exchange,
            LayoutPreset::ChartFocus => LayoutPreset::Dashboard,
            LayoutPreset::Feed => LayoutPreset::ChartFocus,
            LayoutPreset::Compact => LayoutPreset::Feed,
            LayoutPreset::Exchange => LayoutPreset::Compact,
        }
    }

    /// Returns a display label for this preset.
    pub fn label(&self) -> &'static str {
        match self {
            LayoutPreset::Dashboard => "Dashboard",
            LayoutPreset::ChartFocus => "Chart",
            LayoutPreset::Feed => "Feed",
            LayoutPreset::Compact => "Compact",
            LayoutPreset::Exchange => "Exchange",
        }
    }
}

/// Controls which widgets are visible in the monitor.
///
/// Individual widgets can be toggled on/off via keybindings or config.
/// The layout functions use these flags to decide which `Rect` areas to allocate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WidgetVisibility {
    /// Show the price chart (line or candlestick).
    pub price_chart: bool,
    /// Show the volume bar chart.
    pub volume_chart: bool,
    /// Show the buy/sell pressure gauge and activity log.
    pub buy_sell_pressure: bool,
    /// Show the metrics panel (sparkline + key metrics table).
    pub metrics_panel: bool,
    /// Show the activity log feed.
    pub activity_log: bool,
    /// Show the holder count in the metrics panel.
    pub holder_count: bool,
    /// Show the per-pair liquidity depth.
    pub liquidity_depth: bool,
}

impl Default for WidgetVisibility {
    fn default() -> Self {
        Self {
            price_chart: true,
            volume_chart: true,
            buy_sell_pressure: true,
            metrics_panel: true,
            activity_log: true,
            holder_count: true,
            liquidity_depth: true,
        }
    }
}

impl WidgetVisibility {
    /// Returns the number of visible widgets.
    pub fn visible_count(&self) -> usize {
        [
            self.price_chart,
            self.volume_chart,
            self.buy_sell_pressure,
            self.metrics_panel,
            self.activity_log,
        ]
        .iter()
        .filter(|&&v| v)
        .count()
    }

    /// Toggles a widget by index (1-based: 1=price_chart, 2=volume, 3=buy_sell, 4=metrics, 5=log).
    pub fn toggle_by_index(&mut self, index: usize) {
        match index {
            1 => self.price_chart = !self.price_chart,
            2 => self.volume_chart = !self.volume_chart,
            3 => self.buy_sell_pressure = !self.buy_sell_pressure,
            4 => self.metrics_panel = !self.metrics_panel,
            5 => self.activity_log = !self.activity_log,
            _ => {}
        }
    }
}

/// Y-axis scaling mode for price charts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ScaleMode {
    /// Linear scale (default).
    #[default]
    Linear,
    /// Logarithmic scale -- useful for tokens with very wide price ranges.
    Log,
}

impl ScaleMode {
    /// Toggles between Linear and Log.
    pub fn toggle(&self) -> Self {
        match self {
            ScaleMode::Linear => ScaleMode::Log,
            ScaleMode::Log => ScaleMode::Linear,
        }
    }

    /// Returns a short display label.
    pub fn label(&self) -> &'static str {
        match self {
            ScaleMode::Linear => "Lin",
            ScaleMode::Log => "Log",
        }
    }
}

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
            ColorScheme::GreenRed => "G/R",
            ColorScheme::BlueOrange => "B/O",
            ColorScheme::Monochrome => "Mono",
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

/// CSV export configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExportConfig {
    /// Base directory for exports (default: `./scope-exports/`).
    pub path: Option<String>,
}

impl Default for ExportConfig {
    #[allow(clippy::derivable_impls)]
    fn default() -> Self {
        Self { path: None }
    }
}

/// Monitor-specific configuration.
///
/// Loaded from the `monitor:` section of `~/.config/scope/config.yaml`.
/// All fields have sensible defaults so the section is entirely optional.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MonitorConfig {
    /// Layout preset to use on startup.
    pub layout: LayoutPreset,
    /// Refresh interval in seconds.
    pub refresh_seconds: u64,
    /// Widget visibility toggles.
    pub widgets: WidgetVisibility,
    /// Y-axis scale mode for price charts.
    pub scale: ScaleMode,
    /// Color scheme.
    pub color_scheme: ColorScheme,
    /// Alert thresholds (price min/max, whale detection).
    pub alerts: AlertConfig,
    /// CSV export settings.
    pub export: ExportConfig,
    /// Whether to auto-pause data fetching when the user is interacting.
    pub auto_pause_on_input: bool,
    /// Exchange venue for real OHLC candle data (e.g., "binance").
    #[serde(default)]
    pub venue: Option<String>,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            layout: LayoutPreset::Dashboard,
            refresh_seconds: DEFAULT_MONITOR_REFRESH_SECS,
            widgets: WidgetVisibility::default(),
            scale: ScaleMode::Linear,
            color_scheme: ColorScheme::GreenRed,
            alerts: AlertConfig::default(),
            export: ExportConfig::default(),
            auto_pause_on_input: false,
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
    }

    #[test]
    fn test_load_missing_file_returns_defaults() {
        let config = Config::load(Some(Path::new("/nonexistent/path/config.yaml"))).unwrap();
        assert_eq!(config, Config::default());
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
        assert_eq!(config, Config::default());
    }

    #[test]
    fn test_output_format_serialization() {
        let json_format = OutputFormat::Json;
        let serialized = serde_yaml::to_string(&json_format).unwrap();
        assert!(serialized.contains("json"));

        let deserialized: OutputFormat = serde_yaml::from_str("csv").unwrap();
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
        let path = Config::default_path();
        // Should end with expected structure
        assert!(path.ends_with("scope/config.yaml") || path.ends_with("scope\\config.yaml"));
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
    fn test_config_path_returns_some() {
        let path = Config::config_path();
        // Should return Some on systems with a home dir
        assert!(path.is_some());
        assert!(path.unwrap().to_string_lossy().contains("scope"));
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

        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: Config = serde_yaml::from_str(&yaml).unwrap();
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

        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(config.ghola, deserialized.ghola);
    }
}
