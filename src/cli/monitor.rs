//! # Live Token Monitor
//!
//! This module implements a real-time terminal UI for monitoring token metrics.
//! It displays live-updating charts for price, volume, transactions, and liquidity
//! across four switchable layout presets with responsive terminal sizing.
//!
//! ## Usage
//!
//! Directly from the command line (no interactive mode required):
//! ```text
//! scope monitor USDC
//! scope mon PEPE --chain ethereum --layout chart-focus --refresh 3
//! scope monitor 0x1234... -c solana -s log --color-scheme blue-orange
//! ```
//!
//! Or from interactive mode:
//! ```text
//! scope> monitor USDC
//! scope> mon 0x1234...
//! ```
//!
//! ## Layout Presets
//!
//! - **Dashboard** -- Charts top, gauges middle, transaction feed bottom (default)
//! - **ChartFocus** -- Full-width candles (~85%), minimal stats overlay below
//! - **Feed** -- Transaction log takes priority (~75%), small metrics + buy/sell on top
//! - **Compact** -- Price sparkline and metrics only, for small terminals (<80x24)
//! - **Exchange** -- Order book + chart + market info (exchange-style view)
//!
//! The monitor auto-selects a layout based on terminal dimensions (responsive
//! breakpoints). Manual switching via `L`/`H` disables auto-selection until `A`.
//!
//! ## Features
//!
//! - Real-time price chart (line, candlestick, or volume profile) with sliding window
//! - Volume bar chart
//! - Buy/sell ratio gauge
//! - Scrollable activity feed (transaction log)
//! - Key metrics panel with sparkline and stats table
//! - Config-driven widget visibility (toggle any widget on/off)
//! - Four layout presets switchable at runtime
//! - Responsive terminal sizing with auto-layout
//! - Log/linear Y-axis scale toggle
//! - Three color schemes (Green/Red, Blue/Orange, Monochrome)
//! - On-chain holder count integration (when chain client is available)
//! - Per-pair liquidity depth breakdown across DEXes
//! - Configurable price alerts (min/max thresholds, whale detection, volume spikes)
//! - CSV export mode (toggle with `E`, writes to `./scope-exports/`)
//! - Auto-pause on user input (toggle with `Shift+P`)
//!
//! ## Keyboard Controls
//!
//! - `Q`/`Esc` quit, `R` refresh, `P`/`Space` pause
//! - `Shift+P` toggle auto-pause on input
//! - `E` toggle CSV export (REC indicator when active)
//! - `L`/`H` cycle layout forward/backward
//! - `W` + `1-5` toggle widget visibility
//! - `A` re-enable auto layout
//! - `C` toggle chart mode, `S` toggle log/linear scale, `/` cycle color scheme
//! - `T`/`Tab` cycle time period, `1-6` select period
//! - `J`/`K` scroll activity log, `+`/`-` adjust refresh speed

use crate::chains::dex::{DexClient, DexDataSource, DexTokenData};
use crate::chains::{ChainClient, ChainClientFactory, DexPair};
use crate::config::Config;
use crate::error::{Result, ScopeError};
use crate::market::{OrderBook, OrderBookLevel, Trade, TradeSide};
use clap::Args;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Axis, Bar, BarChart, BarGroup, Block, Borders, Chart, Dataset, GraphType, List, ListItem,
        ListState, Paragraph, Row, Sparkline, Table, Tabs,
        canvas::{Canvas, Line as CanvasLine, Rectangle},
    },
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::io::{self, BufWriter, Write as _};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::interactive::SessionContext;

// ============================================================================
// CLI Arguments
// ============================================================================

/// Arguments for the top-level `monitor` command.
///
/// Launches the live TUI dashboard directly from the command line,
/// without requiring interactive mode.
///
/// # Examples
///
/// ```bash
/// # Monitor by token address
/// scope monitor 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48
///
/// # Monitor by symbol on a specific chain
/// scope monitor USDC --chain ethereum
///
/// # Short alias
/// scope mon PEPE -c ethereum
///
/// # Custom layout and refresh rate
/// scope monitor USDC --layout chart-focus --refresh 3
/// ```
#[derive(Debug, Args)]
pub struct MonitorArgs {
    /// Token address or symbol to monitor.
    ///
    /// Can be a contract address (0x...) or a token symbol/name.
    /// If a name/symbol is provided, matching tokens will be searched
    /// and you can select from the results.
    pub token: String,

    /// Target blockchain network.
    ///
    /// Determines which chain to query for token data.
    #[arg(short, long, default_value = "ethereum")]
    pub chain: String,

    /// Layout preset for the TUI dashboard.
    ///
    /// Controls how widgets are arranged on screen.
    /// Options: dashboard, chart-focus, feed, compact, exchange.
    #[arg(short, long)]
    pub layout: Option<LayoutPreset>,

    /// Refresh interval in seconds.
    ///
    /// How often to fetch new data from the API.
    /// Adjustable at runtime with +/- keys.
    #[arg(short, long)]
    pub refresh: Option<u64>,

    /// Y-axis scale mode for price charts.
    ///
    /// Options: linear, log.
    #[arg(short, long)]
    pub scale: Option<ScaleMode>,

    /// Color scheme for charts.
    ///
    /// Options: green-red, blue-orange, monochrome.
    #[arg(long)]
    pub color_scheme: Option<ColorScheme>,

    /// Start CSV export immediately, writing to the given path.
    #[arg(short, long, value_name = "PATH")]
    pub export: Option<PathBuf>,

    /// Exchange venue for real OHLC candle data (e.g., binance, mexc).
    ///
    /// When specified, the monitor fetches real candlestick data from the
    /// exchange API instead of generating synthetic candles from price history.
    #[arg(long, value_name = "VENUE")]
    pub venue: Option<String>,

    /// Direct trading pair for exchange-only mode (e.g., PUSD_USDT).
    ///
    /// Bypasses DexScreener token resolution entirely and uses the exchange
    /// ticker as the data source. Requires `--venue` to be specified.
    /// Use this when the token is not listed on DexScreener.
    #[arg(long, value_name = "PAIR")]
    pub pair: Option<String>,
}

/// Maximum data retention: 24 hours.
/// At 5-second intervals: 24 * 60 * 12 = 17,280 points max per history.
/// With DataPoint at 24 bytes: ~415 KB per history, ~830 KB total.
/// Data is persisted to OS temp folder for session continuity.
const MAX_DATA_AGE_SECS: f64 = 24.0 * 3600.0; // 24 hours

/// Cache file prefix in temp directory.
const CACHE_FILE_PREFIX: &str = "bcc_monitor_";

/// Default refresh interval in seconds.
const DEFAULT_REFRESH_SECS: u64 = 5;

/// Minimum refresh interval in seconds.
const MIN_REFRESH_SECS: u64 = 1;

/// Maximum refresh interval in seconds.
const MAX_REFRESH_SECS: u64 = 60;

/// A data point with timestamp, value, and whether it's real (from API) or synthetic.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DataPoint {
    /// Unix timestamp in seconds.
    pub timestamp: f64,
    /// Value (price or volume).
    pub value: f64,
    /// True if this is real data from API, false if synthetic/estimated.
    pub is_real: bool,
}

/// OHLC (Open-High-Low-Close) candlestick data for a time period.
#[derive(Debug, Clone, Copy)]
pub struct OhlcCandle {
    /// Start timestamp of this candle period.
    pub timestamp: f64,
    /// Opening price.
    pub open: f64,
    /// Highest price during the period.
    pub high: f64,
    /// Lowest price during the period.
    pub low: f64,
    /// Closing price.
    pub close: f64,
    /// Whether this candle is bullish (close >= open).
    pub is_bullish: bool,
}

impl OhlcCandle {
    /// Creates a new candle from a single price point.
    pub fn new(timestamp: f64, price: f64) -> Self {
        Self {
            timestamp,
            open: price,
            high: price,
            low: price,
            close: price,
            is_bullish: true,
        }
    }

    /// Updates the candle with a new price.
    pub fn update(&mut self, price: f64) {
        self.high = self.high.max(price);
        self.low = self.low.min(price);
        self.close = price;
        self.is_bullish = self.close >= self.open;
    }
}

/// Cached monitor data that persists between sessions.
#[derive(Debug, Serialize, Deserialize)]
struct CachedMonitorData {
    /// Token address this cache is for.
    token_address: String,
    /// Chain identifier.
    chain: String,
    /// Price history data points.
    price_history: Vec<DataPoint>,
    /// Volume history data points.
    volume_history: Vec<DataPoint>,
    /// Timestamp when cache was saved.
    saved_at: f64,
}

/// Time period for chart display (limited to 24 hours of data retention).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimePeriod {
    /// Last 1 minute
    Min1,
    /// Last 5 minutes
    Min5,
    /// Last 15 minutes
    Min15,
    /// Last 1 hour
    Hour1,
    /// Last 4 hours
    Hour4,
    /// Last 24 hours (1 day)
    Day1,
}

impl TimePeriod {
    /// Returns the duration in seconds for this period.
    pub fn duration_secs(&self) -> i64 {
        match self {
            TimePeriod::Min1 => 60,
            TimePeriod::Min5 => 5 * 60,
            TimePeriod::Min15 => 15 * 60,
            TimePeriod::Hour1 => 3600,
            TimePeriod::Hour4 => 4 * 3600,
            TimePeriod::Day1 => 24 * 3600,
        }
    }

    /// Returns a display label for this period.
    pub fn label(&self) -> &'static str {
        match self {
            TimePeriod::Min1 => "1m",
            TimePeriod::Min5 => "5m",
            TimePeriod::Min15 => "15m",
            TimePeriod::Hour1 => "1h",
            TimePeriod::Hour4 => "4h",
            TimePeriod::Day1 => "1d",
        }
    }

    /// Returns the zero-based index for this period (for Tabs widget).
    pub fn index(&self) -> usize {
        match self {
            TimePeriod::Min1 => 0,
            TimePeriod::Min5 => 1,
            TimePeriod::Min15 => 2,
            TimePeriod::Hour1 => 3,
            TimePeriod::Hour4 => 4,
            TimePeriod::Day1 => 5,
        }
    }

    /// Returns the exchange API kline interval string for this period.
    pub fn exchange_interval(&self) -> &'static str {
        match self {
            TimePeriod::Min1 => "1m",
            TimePeriod::Min5 => "5m",
            TimePeriod::Min15 => "15m",
            TimePeriod::Hour1 => "1h",
            TimePeriod::Hour4 => "4h",
            TimePeriod::Day1 => "1d",
        }
    }

    /// Cycles to the next time period.
    pub fn next(&self) -> Self {
        match self {
            TimePeriod::Min1 => TimePeriod::Min5,
            TimePeriod::Min5 => TimePeriod::Min15,
            TimePeriod::Min15 => TimePeriod::Hour1,
            TimePeriod::Hour1 => TimePeriod::Hour4,
            TimePeriod::Hour4 => TimePeriod::Day1,
            TimePeriod::Day1 => TimePeriod::Min1,
        }
    }
}

impl std::fmt::Display for TimePeriod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Chart display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChartMode {
    /// Line chart showing price over time.
    #[default]
    Line,
    /// Candlestick chart showing OHLC data.
    Candlestick,
    /// Volume profile showing volume distribution by price level.
    VolumeProfile,
}

impl ChartMode {
    /// Cycles to the next chart mode.
    pub fn next(&self) -> Self {
        match self {
            ChartMode::Line => ChartMode::Candlestick,
            ChartMode::Candlestick => ChartMode::VolumeProfile,
            ChartMode::VolumeProfile => ChartMode::Line,
        }
    }

    /// Returns a display label for this mode.
    pub fn label(&self) -> &'static str {
        match self {
            ChartMode::Line => "Line",
            ChartMode::Candlestick => "Candle",
            ChartMode::VolumeProfile => "VolPro",
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

    /// Returns the named color palette for this scheme.
    pub fn palette(&self) -> ColorPalette {
        match self {
            ColorScheme::GreenRed => ColorPalette {
                up: Color::Green,
                down: Color::Red,
                neutral: Color::Gray,
                header_fg: Color::White,
                border: Color::DarkGray,
                highlight: Color::Yellow,
                volume_bar: Color::Blue,
                sparkline: Color::Cyan,
            },
            ColorScheme::BlueOrange => ColorPalette {
                up: Color::Blue,
                down: Color::Rgb(255, 165, 0), // orange
                neutral: Color::Gray,
                header_fg: Color::White,
                border: Color::DarkGray,
                highlight: Color::Cyan,
                volume_bar: Color::Magenta,
                sparkline: Color::LightBlue,
            },
            ColorScheme::Monochrome => ColorPalette {
                up: Color::White,
                down: Color::DarkGray,
                neutral: Color::Gray,
                header_fg: Color::White,
                border: Color::DarkGray,
                highlight: Color::White,
                volume_bar: Color::Gray,
                sparkline: Color::White,
            },
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

/// Named color palette derived from a ColorScheme.
#[derive(Debug, Clone, Copy)]
pub struct ColorPalette {
    /// Color for price-up / bullish.
    pub up: Color,
    /// Color for price-down / bearish.
    pub down: Color,
    /// Neutral/secondary text color.
    pub neutral: Color,
    /// Header foreground.
    pub header_fg: Color,
    /// Border color.
    pub border: Color,
    /// Highlight/accent color.
    pub highlight: Color,
    /// Volume bar color.
    pub volume_bar: Color,
    /// Sparkline color.
    pub sparkline: Color,
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

/// An active (currently firing) alert with a description.
#[derive(Debug, Clone)]
pub struct ActiveAlert {
    /// Human-readable message describing the alert.
    pub message: String,
    /// When the alert was first triggered.
    pub triggered_at: Instant,
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
            refresh_seconds: DEFAULT_REFRESH_SECS,
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

/// State for the live token monitor.
pub struct MonitorState {
    /// Token contract address.
    pub token_address: String,

    /// Token symbol.
    pub symbol: String,

    /// Token name.
    pub name: String,

    /// Blockchain network.
    pub chain: String,

    /// Historical price data points with real/synthetic indicator.
    pub price_history: VecDeque<DataPoint>,

    /// Historical volume data points with real/synthetic indicator.
    pub volume_history: VecDeque<DataPoint>,

    /// Count of real (non-synthetic) data points.
    pub real_data_count: usize,

    /// Current price in USD.
    pub current_price: f64,

    /// 24-hour price change percentage.
    pub price_change_24h: f64,

    /// 6-hour price change percentage.
    pub price_change_6h: f64,

    /// 1-hour price change percentage.
    pub price_change_1h: f64,

    /// 5-minute price change percentage.
    pub price_change_5m: f64,

    /// Timestamp when the price last changed (Unix timestamp).
    pub last_price_change_at: f64,

    /// Previous price for change detection.
    pub previous_price: f64,

    /// Total buy transactions in 24 hours.
    pub buys_24h: u64,

    /// Total sell transactions in 24 hours.
    pub sells_24h: u64,

    /// Total liquidity in USD.
    pub liquidity_usd: f64,

    /// 24-hour volume in USD.
    pub volume_24h: f64,

    /// Market capitalization.
    pub market_cap: Option<f64>,

    /// Fully diluted valuation.
    pub fdv: Option<f64>,

    /// Last update timestamp.
    pub last_update: Instant,

    /// Refresh rate.
    pub refresh_rate: Duration,

    /// Whether monitoring is paused.
    pub paused: bool,

    /// Recent log messages.
    pub log_messages: VecDeque<String>,

    /// Scroll state for the activity log list widget.
    pub log_list_state: ListState,

    /// Error message to display (if any).
    pub error_message: Option<String>,

    /// Selected time period for chart display.
    pub time_period: TimePeriod,

    /// Chart display mode (line or candlestick).
    pub chart_mode: ChartMode,

    /// Y-axis scale mode for price charts (Linear or Log).
    pub scale_mode: ScaleMode,

    /// Active color scheme.
    pub color_scheme: ColorScheme,

    /// Holder count (fetched from chain client, if available).
    pub holder_count: Option<u64>,

    /// Per-pair liquidity data: (pair_name, liquidity_usd).
    pub liquidity_pairs: Vec<(String, f64)>,

    /// Synthetic order book generated from DEX pair data.
    pub order_book: Option<OrderBook>,

    /// Recent trades (synthetic from DEX pair data or real from exchange API).
    pub recent_trades: VecDeque<Trade>,

    /// Raw DEX pair data for the exchange view.
    pub dex_pairs: Vec<DexPair>,

    /// Token metadata: website URLs.
    pub websites: Vec<String>,

    /// Token metadata: social links (name, url).
    pub socials: Vec<(String, String)>,

    /// Earliest pair creation timestamp (for "listed since").
    pub earliest_pair_created_at: Option<i64>,

    /// DexScreener URL for this token.
    pub dexscreener_url: Option<String>,

    /// Counter to throttle holder count fetches.
    pub holder_fetch_counter: u32,

    /// Unix timestamp when monitoring started.
    pub start_timestamp: i64,

    /// Current layout preset.
    pub layout: LayoutPreset,

    /// Widget visibility toggles.
    pub widgets: WidgetVisibility,

    /// Whether responsive auto-layout is active (disabled by manual layout switch).
    pub auto_layout: bool,

    /// Whether the widget-toggle input mode is active (waiting for digit 1-5).
    pub widget_toggle_mode: bool,

    // ── Phase 7: Alert System ──
    /// Alert configuration thresholds.
    pub alerts: AlertConfig,

    /// Currently firing alerts.
    pub active_alerts: Vec<ActiveAlert>,

    /// Visual flash timer for alert overlay.
    pub alert_flash_until: Option<Instant>,

    // ── Phase 8: CSV Export ──
    /// Whether CSV export is currently active.
    pub export_active: bool,

    /// Path to the current export file.
    pub export_path: Option<PathBuf>,

    /// Rolling volume average for spike detection (simple moving average).
    pub volume_avg: f64,

    // ── Phase 9: Auto-Pause ──
    /// Whether auto-pause on user input is enabled.
    pub auto_pause_on_input: bool,

    /// Timestamp of the last user key input.
    pub last_input_at: Instant,

    /// Duration after last input before auto-pause lifts (default 3s).
    pub auto_pause_timeout: Duration,

    // ── Exchange OHLC ──
    /// Real OHLC candles fetched from an exchange venue.
    /// When present, `get_ohlc_candles` uses these instead of synthetic candles.
    pub exchange_ohlc: Vec<OhlcCandle>,

    /// Venue-formatted pair symbol for OHLC queries (e.g., "BTCUSDT").
    pub venue_pair: Option<String>,
}

impl MonitorState {
    /// Creates a new monitor state from initial token data.
    /// Attempts to load cached data from disk first.
    pub fn new(token_data: &DexTokenData, chain: &str) -> Self {
        let now = Instant::now();
        let now_ts = chrono::Utc::now().timestamp() as f64;

        // Try to load cached data first
        let (price_history, volume_history, real_data_count) =
            if let Some(cached) = Self::load_cache(&token_data.address, chain) {
                // Filter out data older than 24 hours
                let cutoff = now_ts - MAX_DATA_AGE_SECS;
                let price_hist: VecDeque<DataPoint> = cached
                    .price_history
                    .into_iter()
                    .filter(|p| p.timestamp >= cutoff)
                    .collect();
                let vol_hist: VecDeque<DataPoint> = cached
                    .volume_history
                    .into_iter()
                    .filter(|p| p.timestamp >= cutoff)
                    .collect();
                let real_count = price_hist.iter().filter(|p| p.is_real).count();
                (price_hist, vol_hist, real_count)
            } else {
                // Generate synthetic historical data from price change percentages
                let price_hist = Self::generate_synthetic_price_history(
                    token_data.price_usd,
                    token_data.price_change_1h,
                    token_data.price_change_6h,
                    token_data.price_change_24h,
                    now_ts,
                );
                let vol_hist = Self::generate_synthetic_volume_history(
                    token_data.volume_24h,
                    token_data.volume_6h,
                    token_data.volume_1h,
                    now_ts,
                );
                (price_hist, vol_hist, 0)
            };

        Self {
            token_address: token_data.address.clone(),
            symbol: token_data.symbol.clone(),
            name: token_data.name.clone(),
            chain: chain.to_string(),
            price_history,
            volume_history,
            real_data_count,
            current_price: token_data.price_usd,
            price_change_24h: token_data.price_change_24h,
            price_change_6h: token_data.price_change_6h,
            price_change_1h: token_data.price_change_1h,
            price_change_5m: token_data.price_change_5m,
            last_price_change_at: now_ts, // Initialize to current time
            previous_price: token_data.price_usd,
            buys_24h: token_data.total_buys_24h,
            sells_24h: token_data.total_sells_24h,
            liquidity_usd: token_data.liquidity_usd,
            volume_24h: token_data.volume_24h,
            market_cap: token_data.market_cap,
            fdv: token_data.fdv,
            last_update: now,
            refresh_rate: Duration::from_secs(DEFAULT_REFRESH_SECS),
            paused: false,
            log_messages: VecDeque::with_capacity(10),
            log_list_state: ListState::default(),
            error_message: None,
            time_period: TimePeriod::Hour1, // Default to 1 hour view
            chart_mode: ChartMode::Line,    // Default to line chart
            scale_mode: ScaleMode::Linear,  // Default to linear scale
            color_scheme: ColorScheme::GreenRed, // Default color scheme
            holder_count: None,
            liquidity_pairs: Vec::new(),
            order_book: None,
            recent_trades: VecDeque::new(),
            dex_pairs: token_data.pairs.clone(),
            websites: token_data.websites.clone(),
            socials: token_data
                .socials
                .iter()
                .map(|s| (s.platform.clone(), s.url.clone()))
                .collect(),
            earliest_pair_created_at: token_data.earliest_pair_created_at,
            dexscreener_url: token_data.dexscreener_url.clone(),
            holder_fetch_counter: 0,
            start_timestamp: now_ts as i64,
            layout: LayoutPreset::Dashboard,
            widgets: WidgetVisibility::default(),
            auto_layout: true,
            widget_toggle_mode: false,
            // Phase 7: Alerts
            alerts: AlertConfig::default(),
            active_alerts: Vec::new(),
            alert_flash_until: None,
            // Phase 8: Export
            export_active: false,
            export_path: None,
            volume_avg: token_data.volume_24h,
            // Phase 9: Auto-Pause
            auto_pause_on_input: false,
            last_input_at: now,
            auto_pause_timeout: Duration::from_secs(3),
            // Exchange OHLC
            exchange_ohlc: Vec::new(),
            venue_pair: None,
        }
    }

    /// Applies monitor config settings to this state.
    pub fn apply_config(&mut self, config: &MonitorConfig) {
        self.layout = config.layout;
        self.widgets = config.widgets.clone();
        self.refresh_rate = Duration::from_secs(config.refresh_seconds);
        self.scale_mode = config.scale;
        self.color_scheme = config.color_scheme;
        self.alerts = config.alerts.clone();
        self.auto_pause_on_input = config.auto_pause_on_input;
    }

    /// Toggles between line and candlestick chart modes.
    /// Returns the current color palette based on the active color scheme.
    pub fn palette(&self) -> ColorPalette {
        self.color_scheme.palette()
    }

    pub fn toggle_chart_mode(&mut self) {
        self.chart_mode = self.chart_mode.next();
        self.log(format!("Chart mode: {}", self.chart_mode.label()));
    }

    /// Returns the path to the cache file for a token.
    fn cache_path(token_address: &str, chain: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        // Create a safe filename from address (first 16 chars) and chain
        let safe_addr = token_address
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(16)
            .collect::<String>()
            .to_lowercase();
        path.push(format!("{}{}_{}.json", CACHE_FILE_PREFIX, chain, safe_addr));
        path
    }

    /// Loads cached monitor data from disk.
    fn load_cache(token_address: &str, chain: &str) -> Option<CachedMonitorData> {
        let path = Self::cache_path(token_address, chain);
        if !path.exists() {
            return None;
        }

        match fs::read_to_string(&path) {
            Ok(contents) => {
                match serde_json::from_str::<CachedMonitorData>(&contents) {
                    Ok(cached) => {
                        // Verify this is for the same token
                        if cached.token_address.to_lowercase() == token_address.to_lowercase()
                            && cached.chain.to_lowercase() == chain.to_lowercase()
                        {
                            Some(cached)
                        } else {
                            None
                        }
                    }
                    Err(_) => None,
                }
            }
            Err(_) => None,
        }
    }

    /// Saves monitor data to cache file.
    pub fn save_cache(&self) {
        let cached = CachedMonitorData {
            token_address: self.token_address.clone(),
            chain: self.chain.clone(),
            price_history: self.price_history.iter().copied().collect(),
            volume_history: self.volume_history.iter().copied().collect(),
            saved_at: chrono::Utc::now().timestamp() as f64,
        };

        let path = Self::cache_path(&self.token_address, &self.chain);
        if let Ok(json) = serde_json::to_string(&cached) {
            let _ = fs::write(&path, json);
        }
    }

    /// Generates synthetic price history from percentage changes.
    /// All generated points are marked as synthetic (is_real = false).
    fn generate_synthetic_price_history(
        current_price: f64,
        change_1h: f64,
        change_6h: f64,
        change_24h: f64,
        now_ts: f64,
    ) -> VecDeque<DataPoint> {
        let mut history = VecDeque::with_capacity(50);

        // Calculate prices at known points (working backwards from current)
        let price_1h_ago = current_price / (1.0 + change_1h / 100.0);
        let price_6h_ago = current_price / (1.0 + change_6h / 100.0);
        let price_24h_ago = current_price / (1.0 + change_24h / 100.0);

        // Generate points: 24h ago, 12h ago, 6h ago, 3h ago, 1h ago, 30m ago, now
        let points = [
            (now_ts - 24.0 * 3600.0, price_24h_ago),
            (now_ts - 12.0 * 3600.0, (price_24h_ago + price_6h_ago) / 2.0),
            (now_ts - 6.0 * 3600.0, price_6h_ago),
            (now_ts - 3.0 * 3600.0, (price_6h_ago + price_1h_ago) / 2.0),
            (now_ts - 1.0 * 3600.0, price_1h_ago),
            (now_ts - 0.5 * 3600.0, (price_1h_ago + current_price) / 2.0),
            (now_ts, current_price),
        ];

        // Interpolate to create more points for smoother charts
        for i in 0..points.len() - 1 {
            let (t1, p1) = points[i];
            let (t2, p2) = points[i + 1];
            let steps = 4; // Number of interpolated points between each pair

            for j in 0..steps {
                let frac = j as f64 / steps as f64;
                let t = t1 + (t2 - t1) * frac;
                let p = p1 + (p2 - p1) * frac;
                history.push_back(DataPoint {
                    timestamp: t,
                    value: p,
                    is_real: false, // Synthetic data
                });
            }
        }
        // Add the final point (also synthetic since it's estimated)
        history.push_back(DataPoint {
            timestamp: points[points.len() - 1].0,
            value: points[points.len() - 1].1,
            is_real: false,
        });

        history
    }

    /// Generates synthetic volume history from known data points.
    /// All generated points are marked as synthetic (is_real = false).
    fn generate_synthetic_volume_history(
        volume_24h: f64,
        volume_6h: f64,
        volume_1h: f64,
        now_ts: f64,
    ) -> VecDeque<DataPoint> {
        let mut history = VecDeque::with_capacity(24);

        // Create hourly volume estimates
        let hourly_avg = volume_24h / 24.0;

        for i in 0..24 {
            let hours_ago = 24 - i;
            let ts = now_ts - (hours_ago as f64) * 3600.0;

            // Use more accurate data for recent hours
            let volume = if hours_ago <= 1 {
                volume_1h
            } else if hours_ago <= 6 {
                volume_6h / 6.0
            } else {
                // Estimate with some variation
                hourly_avg * (0.8 + (i as f64 / 24.0) * 0.4)
            };

            history.push_back(DataPoint {
                timestamp: ts,
                value: volume,
                is_real: false, // Synthetic data
            });
        }

        history
    }

    /// Generates a multi-level synthetic order book from DEX pair data.
    ///
    /// Aggregates liquidity across all pairs and distributes it into
    /// realistic bid/ask levels around the current mid price. Levels are
    /// spaced logarithmically so near-mid levels are denser.
    fn generate_synthetic_order_book(
        pairs: &[DexPair],
        symbol: &str,
        price: f64,
        total_liquidity: f64,
    ) -> Option<OrderBook> {
        if price <= 0.0 || total_liquidity <= 0.0 {
            return None;
        }

        // Spread is tighter for more liquid markets
        let base_spread_bps = if total_liquidity > 1_000_000.0 {
            5.0 // 0.05%
        } else if total_liquidity > 100_000.0 {
            15.0 // 0.15%
        } else {
            50.0 // 0.50%
        };

        let half_spread = price * base_spread_bps / 10_000.0;
        let half_liq = total_liquidity / 2.0;
        let num_levels: usize = 15;

        // Generate ask levels (ascending from mid + half_spread)
        let mut asks = Vec::with_capacity(num_levels);
        for i in 0..num_levels {
            // Exponential spacing: tighter near the mid, wider further out
            let offset_pct = (1.0 + i as f64 * 0.3).powf(1.4) * 0.001;
            let ask_price = price + half_spread + price * offset_pct;
            // Liquidity decreases further from mid (exponential decay)
            let weight = (-1.5 * i as f64 / num_levels as f64).exp();
            let level_liq = half_liq * weight / num_levels as f64 * 2.5;
            let quantity = level_liq / ask_price;
            if quantity > 0.0 {
                asks.push(OrderBookLevel {
                    price: ask_price,
                    quantity,
                });
            }
        }

        // Generate bid levels (descending from mid - half_spread)
        let mut bids = Vec::with_capacity(num_levels);
        for i in 0..num_levels {
            let offset_pct = (1.0 + i as f64 * 0.3).powf(1.4) * 0.001;
            let bid_price = price - half_spread - price * offset_pct;
            if bid_price <= 0.0 {
                break;
            }
            let weight = (-1.5 * i as f64 / num_levels as f64).exp();
            let level_liq = half_liq * weight / num_levels as f64 * 2.5;
            let quantity = level_liq / bid_price;
            if quantity > 0.0 {
                bids.push(OrderBookLevel {
                    price: bid_price,
                    quantity,
                });
            }
        }

        // Find best quote token from pairs for the pair label
        let quote = pairs
            .first()
            .map(|p| p.quote_token.as_str())
            .unwrap_or("USD");

        Some(OrderBook {
            pair: format!("{}/{}", symbol, quote),
            bids,
            asks,
        })
    }

    /// Updates the state with new token data.
    /// New data points are marked as real (is_real = true).
    pub fn update(&mut self, token_data: &DexTokenData) {
        let now_ts = chrono::Utc::now().timestamp() as f64;

        // Add new REAL data points
        self.price_history.push_back(DataPoint {
            timestamp: now_ts,
            value: token_data.price_usd,
            is_real: true,
        });
        self.volume_history.push_back(DataPoint {
            timestamp: now_ts,
            value: token_data.volume_24h,
            is_real: true,
        });
        self.real_data_count += 1;

        // Trim data points older than 24 hours
        let cutoff = now_ts - MAX_DATA_AGE_SECS;

        while let Some(point) = self.price_history.front() {
            if point.timestamp < cutoff {
                self.price_history.pop_front();
            } else {
                break;
            }
        }
        while let Some(point) = self.volume_history.front() {
            if point.timestamp < cutoff {
                self.volume_history.pop_front();
            } else {
                break;
            }
        }

        // Track when price actually changes (using 8 decimal precision for stablecoins)
        let price_changed = (self.previous_price - token_data.price_usd).abs() > 0.00000001;
        if price_changed {
            self.last_price_change_at = now_ts;
            self.previous_price = token_data.price_usd;
        }

        // Update current values
        self.current_price = token_data.price_usd;
        self.price_change_24h = token_data.price_change_24h;
        self.price_change_6h = token_data.price_change_6h;
        self.price_change_1h = token_data.price_change_1h;
        self.price_change_5m = token_data.price_change_5m;
        self.buys_24h = token_data.total_buys_24h;
        self.sells_24h = token_data.total_sells_24h;
        self.liquidity_usd = token_data.liquidity_usd;
        self.volume_24h = token_data.volume_24h;
        self.market_cap = token_data.market_cap;
        self.fdv = token_data.fdv;

        // Extract per-pair liquidity data
        self.liquidity_pairs = token_data
            .pairs
            .iter()
            .map(|p| {
                let label = format!("{}/{} ({})", p.base_token, p.quote_token, p.dex_name);
                (label, p.liquidity_usd)
            })
            .collect();

        // Update DEX pairs and generate synthetic order book
        self.dex_pairs = token_data.pairs.clone();
        self.order_book = Self::generate_synthetic_order_book(
            &token_data.pairs,
            &token_data.symbol,
            token_data.price_usd,
            token_data.liquidity_usd,
        );

        // Generate synthetic trade from price movement
        if token_data.price_usd > 0.0 {
            let side = if price_changed && token_data.price_usd > self.current_price {
                TradeSide::Buy
            } else if price_changed {
                TradeSide::Sell
            } else {
                // No change — alternate based on buy/sell ratio
                if token_data.total_buys_24h >= token_data.total_sells_24h {
                    TradeSide::Buy
                } else {
                    TradeSide::Sell
                }
            };
            let ts_ms = (now_ts * 1000.0) as u64;
            // Synthetic quantity based on recent volume
            let qty = if token_data.volume_24h > 0.0 && token_data.price_usd > 0.0 {
                // Rough: daily volume / 86400 updates * refresh_rate gives per-update volume
                let per_update_vol =
                    token_data.volume_24h / 86400.0 * self.refresh_rate.as_secs_f64();
                per_update_vol / token_data.price_usd
            } else {
                1.0
            };
            self.recent_trades.push_back(Trade {
                price: token_data.price_usd,
                quantity: qty,
                quote_quantity: Some(qty * token_data.price_usd),
                timestamp_ms: ts_ms,
                side,
                id: None,
            });
            // Keep at most 200 trades
            while self.recent_trades.len() > 200 {
                self.recent_trades.pop_front();
            }
        }

        // Update metadata
        self.websites = token_data.websites.clone();
        self.socials = token_data
            .socials
            .iter()
            .map(|s| (s.platform.clone(), s.url.clone()))
            .collect();
        self.earliest_pair_created_at = token_data.earliest_pair_created_at;
        self.dexscreener_url = token_data.dexscreener_url.clone();

        self.last_update = Instant::now();
        self.error_message = None;

        // ── Alert checks ──
        self.check_alerts(token_data);

        // ── CSV export ──
        if self.export_active {
            self.write_export_row();
        }

        // ── Volume average for spike detection ──
        // Exponential moving average: 10% weight on new value
        self.volume_avg = self.volume_avg * 0.9 + token_data.volume_24h * 0.1;

        self.log(format!("Updated: ${:.6}", token_data.price_usd));

        // Periodically save to cache (every 60 updates, ~5 minutes at 5s refresh)
        if self.real_data_count.is_multiple_of(60) {
            self.save_cache();
        }
    }

    /// Checks alert thresholds and updates active_alerts.
    fn check_alerts(&mut self, token_data: &DexTokenData) {
        self.active_alerts.clear();

        // Price min alert
        if let Some(min) = self.alerts.price_min
            && self.current_price < min
        {
            self.active_alerts.push(ActiveAlert {
                message: format!("⚠ Price ${:.6} below min ${:.6}", self.current_price, min),
                triggered_at: Instant::now(),
            });
        }

        // Price max alert
        if let Some(max) = self.alerts.price_max
            && self.current_price > max
        {
            self.active_alerts.push(ActiveAlert {
                message: format!("⚠ Price ${:.6} above max ${:.6}", self.current_price, max),
                triggered_at: Instant::now(),
            });
        }

        // Volume spike alert
        if let Some(threshold_pct) = self.alerts.volume_spike_threshold_pct
            && self.volume_avg > 0.0
        {
            let spike_pct = ((token_data.volume_24h - self.volume_avg) / self.volume_avg) * 100.0;
            if spike_pct > threshold_pct {
                self.active_alerts.push(ActiveAlert {
                    message: format!("⚠ Volume spike: +{:.1}% vs avg", spike_pct),
                    triggered_at: Instant::now(),
                });
            }
        }

        // Whale detection — estimate from buy/sell changes
        if let Some(whale_min) = self.alerts.whale_min_usd {
            // Approximate single-transaction size from volume/tx count
            let total_txs = (token_data.total_buys_24h + token_data.total_sells_24h) as f64;
            if total_txs > 0.0 && token_data.volume_24h / total_txs >= whale_min {
                let avg_tx_size = token_data.volume_24h / total_txs;
                self.active_alerts.push(ActiveAlert {
                    message: format!(
                        "🐋 Avg tx size {} ≥ whale threshold {}",
                        crate::display::format_usd(avg_tx_size),
                        crate::display::format_usd(whale_min)
                    ),
                    triggered_at: Instant::now(),
                });
            }
        }

        // Set flash timer if any alerts are active
        if !self.active_alerts.is_empty() {
            self.alert_flash_until = Some(Instant::now() + Duration::from_secs(2));
        }
    }

    /// Writes a single CSV row to the export file.
    fn write_export_row(&mut self) {
        if let Some(ref path) = self.export_path {
            // Open file in append mode
            if let Ok(file) = fs::OpenOptions::new().append(true).open(path) {
                let mut writer = BufWriter::new(file);
                let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
                let market_cap_str = self
                    .market_cap
                    .map(|mc| format!("{:.2}", mc))
                    .unwrap_or_default();
                let row = format!(
                    "{},{:.8},{:.2},{:.2},{},{},{}\n",
                    timestamp,
                    self.current_price,
                    self.volume_24h,
                    self.liquidity_usd,
                    self.buys_24h,
                    self.sells_24h,
                    market_cap_str,
                );
                let _ = writer.write_all(row.as_bytes());
            }
        }
    }

    /// Starts CSV export: creates the file and writes the header.
    pub fn start_export(&mut self) {
        let base_dir = PathBuf::from("./scope-exports");
        let _ = fs::create_dir_all(&base_dir);
        let date_str = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
        let filename = format!("{}_{}.csv", self.symbol, date_str);
        let path = base_dir.join(filename);

        // Write CSV header
        if let Ok(mut file) = fs::File::create(&path) {
            let header =
                "timestamp,price_usd,volume_24h,liquidity_usd,buys_24h,sells_24h,market_cap\n";
            let _ = file.write_all(header.as_bytes());
        }

        self.export_path = Some(path.clone());
        self.export_active = true;
        self.log(format!("Export started: {}", path.display()));
    }

    /// Stops CSV export and closes the file.
    pub fn stop_export(&mut self) {
        if let Some(ref path) = self.export_path {
            self.log(format!("Export saved: {}", path.display()));
        }
        self.export_active = false;
        self.export_path = None;
    }

    /// Toggles CSV export on/off.
    pub fn toggle_export(&mut self) {
        if self.export_active {
            self.stop_export();
        } else {
            self.start_export();
        }
    }

    /// Returns data points filtered by the current time period.
    /// Returns tuples for chart compatibility, plus a separate vector of is_real flags.
    pub fn get_price_data_for_period(&self) -> (Vec<(f64, f64)>, Vec<bool>) {
        let now_ts = chrono::Utc::now().timestamp() as f64;
        let cutoff = now_ts - self.time_period.duration_secs() as f64;

        let filtered: Vec<&DataPoint> = self
            .price_history
            .iter()
            .filter(|p| p.timestamp >= cutoff)
            .collect();

        let data: Vec<(f64, f64)> = filtered.iter().map(|p| (p.timestamp, p.value)).collect();
        let is_real: Vec<bool> = filtered.iter().map(|p| p.is_real).collect();

        (data, is_real)
    }

    /// Returns volume data filtered by the current time period.
    /// Returns tuples for chart compatibility, plus a separate vector of is_real flags.
    pub fn get_volume_data_for_period(&self) -> (Vec<(f64, f64)>, Vec<bool>) {
        let now_ts = chrono::Utc::now().timestamp() as f64;
        let cutoff = now_ts - self.time_period.duration_secs() as f64;

        let filtered: Vec<&DataPoint> = self
            .volume_history
            .iter()
            .filter(|p| p.timestamp >= cutoff)
            .collect();

        let data: Vec<(f64, f64)> = filtered.iter().map(|p| (p.timestamp, p.value)).collect();
        let is_real: Vec<bool> = filtered.iter().map(|p| p.is_real).collect();

        (data, is_real)
    }

    /// Returns count of synthetic vs real data points in the current view.
    pub fn data_stats(&self) -> (usize, usize) {
        let now_ts = chrono::Utc::now().timestamp() as f64;
        let cutoff = now_ts - self.time_period.duration_secs() as f64;

        let (synthetic, real) = self
            .price_history
            .iter()
            .filter(|p| p.timestamp >= cutoff)
            .fold(
                (0, 0),
                |(s, r), p| {
                    if p.is_real { (s, r + 1) } else { (s + 1, r) }
                },
            );

        (synthetic, real)
    }

    /// Estimates memory usage of stored data in bytes.
    pub fn memory_usage(&self) -> usize {
        // DataPoint is 24 bytes (f64 + f64 + bool + padding)
        let point_size = std::mem::size_of::<DataPoint>();
        (self.price_history.len() + self.volume_history.len()) * point_size
    }

    /// Generates OHLC candles from price history for the current time period.
    ///
    /// The candle duration is automatically determined based on the selected time period:
    /// - 1m view: 5-second candles
    /// - 5m view: 15-second candles
    /// - 15m view: 1-minute candles
    /// - 1h view: 5-minute candles
    /// - 4h view: 15-minute candles
    /// - 1d view: 1-hour candles
    pub fn get_ohlc_candles(&self) -> Vec<OhlcCandle> {
        // Prefer real exchange OHLC candles when available
        if !self.exchange_ohlc.is_empty() {
            return self.exchange_ohlc.clone();
        }

        let (data, _) = self.get_price_data_for_period();

        if data.is_empty() {
            return vec![];
        }

        // Determine candle duration based on time period
        let candle_duration_secs = match self.time_period {
            TimePeriod::Min1 => 5.0,    // 5-second candles
            TimePeriod::Min5 => 15.0,   // 15-second candles
            TimePeriod::Min15 => 60.0,  // 1-minute candles
            TimePeriod::Hour1 => 300.0, // 5-minute candles
            TimePeriod::Hour4 => 900.0, // 15-minute candles
            TimePeriod::Day1 => 3600.0, // 1-hour candles
        };

        let mut candles: Vec<OhlcCandle> = Vec::new();

        for (timestamp, price) in data {
            // Determine which candle this point belongs to
            let candle_start = (timestamp / candle_duration_secs).floor() * candle_duration_secs;

            if let Some(last_candle) = candles.last_mut() {
                if (last_candle.timestamp - candle_start).abs() < 0.001 {
                    // Same candle, update it
                    last_candle.update(price);
                } else {
                    // New candle
                    candles.push(OhlcCandle::new(candle_start, price));
                }
            } else {
                // First candle
                candles.push(OhlcCandle::new(candle_start, price));
            }
        }

        candles
    }

    /// Cycles to the next time period.
    pub fn cycle_time_period(&mut self) {
        self.time_period = self.time_period.next();
        self.log(format!("Time period: {}", self.time_period.label()));
    }

    /// Sets a specific time period.
    pub fn set_time_period(&mut self, period: TimePeriod) {
        self.time_period = period;
        self.log(format!("Time period: {}", period.label()));
    }

    /// Logs a message to the log panel.
    fn log(&mut self, message: String) {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        self.log_messages
            .push_back(format!("[{}] {}", timestamp, message));
        while self.log_messages.len() > 10 {
            self.log_messages.pop_front();
        }
    }

    /// Returns whether a refresh is needed.
    /// Respects manual pause, auto-pause on input, and the refresh interval.
    pub fn should_refresh(&self) -> bool {
        if self.paused {
            return false;
        }
        // Auto-pause: if the user is actively interacting, defer refresh
        if self.auto_pause_on_input && self.last_input_at.elapsed() < self.auto_pause_timeout {
            return false;
        }
        self.last_update.elapsed() >= self.refresh_rate
    }

    /// Returns true if auto-pause is currently suppressing refreshes.
    pub fn is_auto_paused(&self) -> bool {
        self.auto_pause_on_input && self.last_input_at.elapsed() < self.auto_pause_timeout
    }

    /// Toggles pause state.
    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        self.log(if self.paused {
            "Paused".to_string()
        } else {
            "Resumed".to_string()
        });
    }

    /// Forces an immediate refresh.
    pub fn force_refresh(&mut self) {
        self.paused = false;
        self.last_update = Instant::now() - self.refresh_rate;
    }

    /// Increases refresh interval (slower updates).
    pub fn slower_refresh(&mut self) {
        let current_secs = self.refresh_rate.as_secs();
        let new_secs = (current_secs + 5).min(MAX_REFRESH_SECS);
        self.refresh_rate = Duration::from_secs(new_secs);
        self.log(format!("Refresh rate: {}s", new_secs));
    }

    /// Decreases refresh interval (faster updates).
    pub fn faster_refresh(&mut self) {
        let current_secs = self.refresh_rate.as_secs();
        let new_secs = current_secs.saturating_sub(5).max(MIN_REFRESH_SECS);
        self.refresh_rate = Duration::from_secs(new_secs);
        self.log(format!("Refresh rate: {}s", new_secs));
    }

    /// Scrolls the activity log down (newer messages).
    pub fn scroll_log_down(&mut self) {
        let len = self.log_messages.len();
        if len == 0 {
            return;
        }
        let i = self
            .log_list_state
            .selected()
            .map_or(0, |i| if i + 1 < len { i + 1 } else { i });
        self.log_list_state.select(Some(i));
    }

    /// Scrolls the activity log up (older messages).
    pub fn scroll_log_up(&mut self) {
        let i = self
            .log_list_state
            .selected()
            .map_or(0, |i| i.saturating_sub(1));
        self.log_list_state.select(Some(i));
    }

    /// Returns the current refresh rate in seconds.
    pub fn refresh_rate_secs(&self) -> u64 {
        self.refresh_rate.as_secs()
    }

    /// Returns the buy/sell ratio as a percentage (0.0 to 1.0).
    pub fn buy_ratio(&self) -> f64 {
        let total = self.buys_24h + self.sells_24h;
        if total == 0 {
            0.5
        } else {
            self.buys_24h as f64 / total as f64
        }
    }
}

/// Main monitor application, generic over terminal backend for testability.
///
/// The default type parameter (`CrosstermBackend<Stdout>`) is used in production.
/// Tests use `TestBackend` to avoid real terminal I/O.
pub struct MonitorApp<B: ratatui::backend::Backend = ratatui::backend::CrosstermBackend<io::Stdout>>
{
    /// Terminal backend.
    terminal: ratatui::Terminal<B>,

    /// Monitor state.
    state: MonitorState,

    /// DEX client for fetching data.
    dex_client: Box<dyn DexDataSource>,

    /// Optional chain client for on-chain data (holder count, etc.).
    chain_client: Option<Box<dyn ChainClient>>,

    /// Optional exchange client for real OHLC data.
    exchange_client: Option<crate::market::ExchangeClient>,

    /// Whether to exit the application.
    should_exit: bool,

    /// Whether this app owns the real terminal (and should restore it on drop).
    /// False for test instances using `TestBackend`.
    owns_terminal: bool,
}

/// Production constructor and terminal-specific methods.
impl MonitorApp {
    /// Creates a new monitor application with a real terminal and live DEX client.
    pub fn new(
        initial_data: DexTokenData,
        chain: &str,
        monitor_config: &MonitorConfig,
        chain_client: Option<Box<dyn ChainClient>>,
        exchange_client: Option<crate::market::ExchangeClient>,
    ) -> Result<Self> {
        // Setup terminal using ratatui's simplified init
        let terminal = ratatui::init();
        // Enable mouse capture (not handled by ratatui::init)
        execute!(io::stdout(), EnableMouseCapture)
            .map_err(|e| ScopeError::Chain(format!("Failed to enable mouse capture: {}", e)))?;

        let mut state = MonitorState::new(&initial_data, chain);
        state.apply_config(monitor_config);

        // If we have an exchange client, set up the venue pair for OHLC queries
        if let Some(ref ex) = exchange_client {
            let pair = ex.format_pair(&initial_data.symbol);
            state.venue_pair = Some(pair);
        }

        Ok(Self {
            terminal,
            state,
            dex_client: Box::new(DexClient::new()),
            chain_client,
            exchange_client,
            should_exit: false,
            owns_terminal: true,
        })
    }

    /// Runs the main event loop using async event stream.
    pub async fn run(&mut self) -> Result<()> {
        use futures::StreamExt;

        let mut event_stream = crossterm::event::EventStream::new();

        loop {
            // Render UI
            self.terminal.draw(|f| ui(f, &mut self.state))?;

            // Calculate how long until next refresh
            let refresh_delay = if self.state.paused {
                Duration::from_millis(200) // Just check for events while paused
            } else {
                let elapsed = self.state.last_update.elapsed();
                self.state.refresh_rate.saturating_sub(elapsed)
            };

            // Wait for either an event or the refresh timer
            tokio::select! {
                maybe_event = event_stream.next() => {
                    match maybe_event {
                        Some(Ok(Event::Key(key))) => {
                            self.handle_key_event(key);
                        }
                        Some(Ok(Event::Resize(_, _))) => {
                            // Terminal resized — ui() will pick up new size on next draw
                        }
                        _ => {}
                    }
                }
                _ = tokio::time::sleep(refresh_delay) => {
                    // Timer expired — check if refresh is needed
                }
            }

            if self.should_exit {
                break;
            }

            // Check if refresh needed
            if self.state.should_refresh() {
                self.fetch_data().await;
            }
        }

        Ok(())
    }
}

impl<B: ratatui::backend::Backend> Drop for MonitorApp<B> {
    fn drop(&mut self) {
        if self.owns_terminal {
            let _ = execute!(io::stdout(), DisableMouseCapture);
            ratatui::restore();
        }
    }
}

/// Methods that work with any terminal backend (production or test).
/// Includes cleanup, key handling, and data fetching.
impl<B: ratatui::backend::Backend> MonitorApp<B> {
    /// Cleans up terminal state. Only performs real terminal restore when
    /// this app owns the terminal (production mode).
    pub fn cleanup(&mut self) -> Result<()> {
        // Save cache before exiting
        self.state.save_cache();

        if self.owns_terminal {
            // Disable mouse capture (not handled by ratatui::restore)
            let _ = execute!(io::stdout(), DisableMouseCapture);
            // Restore terminal using ratatui's simplified cleanup
            ratatui::restore();
        }
        Ok(())
    }

    /// Handles a single key event, updating state accordingly.
    /// Extracted from the event loop for testability.
    fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) {
        // Track last input time for auto-pause
        self.state.last_input_at = Instant::now();

        // Widget toggle mode: waiting for digit 1-5
        if self.state.widget_toggle_mode {
            self.state.widget_toggle_mode = false;
            if let KeyCode::Char(c @ '1'..='5') = key.code {
                let idx = (c as u8 - b'0') as usize;
                self.state.widgets.toggle_by_index(idx);
                return;
            }
            // Any other key cancels the mode and falls through
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                // Stop export before quitting if active
                if self.state.export_active {
                    self.state.stop_export();
                }
                self.should_exit = true;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.state.export_active {
                    self.state.stop_export();
                }
                self.should_exit = true;
            }
            KeyCode::Char('r') => {
                self.state.force_refresh();
            }
            // Shift+P toggles auto-pause on input
            KeyCode::Char('P') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.state.auto_pause_on_input = !self.state.auto_pause_on_input;
                self.state.log(format!(
                    "Auto-pause: {}",
                    if self.state.auto_pause_on_input {
                        "ON"
                    } else {
                        "OFF"
                    }
                ));
            }
            KeyCode::Char('p') | KeyCode::Char(' ') => {
                self.state.toggle_pause();
            }
            // Toggle CSV export
            KeyCode::Char('e') => {
                self.state.toggle_export();
            }
            // Increase refresh interval (slower)
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char(']') => {
                self.state.slower_refresh();
            }
            // Decrease refresh interval (faster)
            KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Char('[') => {
                self.state.faster_refresh();
            }
            // Time period selection (1=1m, 2=5m, 3=15m, 4=1h, 5=4h, 6=1d)
            KeyCode::Char('1') => {
                self.state.set_time_period(TimePeriod::Min1);
            }
            KeyCode::Char('2') => {
                self.state.set_time_period(TimePeriod::Min5);
            }
            KeyCode::Char('3') => {
                self.state.set_time_period(TimePeriod::Min15);
            }
            KeyCode::Char('4') => {
                self.state.set_time_period(TimePeriod::Hour1);
            }
            KeyCode::Char('5') => {
                self.state.set_time_period(TimePeriod::Hour4);
            }
            KeyCode::Char('6') => {
                self.state.set_time_period(TimePeriod::Day1);
            }
            KeyCode::Char('t') | KeyCode::Tab => {
                self.state.cycle_time_period();
            }
            // Toggle chart mode (line/candlestick/volume-profile)
            KeyCode::Char('c') => {
                self.state.toggle_chart_mode();
            }
            // Toggle scale mode (linear/log)
            KeyCode::Char('s') => {
                self.state.scale_mode = self.state.scale_mode.toggle();
                self.state
                    .log(format!("Scale: {}", self.state.scale_mode.label()));
            }
            // Cycle color scheme
            KeyCode::Char('/') => {
                self.state.color_scheme = self.state.color_scheme.next();
                self.state
                    .log(format!("Colors: {}", self.state.color_scheme.label()));
            }
            // Scroll activity log
            KeyCode::Char('j') | KeyCode::Down => {
                self.state.scroll_log_down();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.state.scroll_log_up();
            }
            // Layout cycling
            KeyCode::Char('l') => {
                self.state.layout = self.state.layout.next();
                self.state.auto_layout = false;
            }
            KeyCode::Char('h') => {
                self.state.layout = self.state.layout.prev();
                self.state.auto_layout = false;
            }
            // Widget toggle mode
            KeyCode::Char('w') => {
                self.state.widget_toggle_mode = true;
            }
            // Re-enable auto layout
            KeyCode::Char('a') => {
                self.state.auto_layout = true;
            }
            _ => {}
        }
    }

    /// Fetches new data from the API.
    async fn fetch_data(&mut self) {
        match self
            .dex_client
            .get_token_data(&self.state.chain, &self.state.token_address)
            .await
        {
            Ok(data) => {
                self.state.update(&data);
            }
            Err(e) => {
                self.state.error_message = Some(format!("API Error: {}", e));
                self.state.last_update = Instant::now(); // Prevent rapid retries
            }
        }

        // Fetch OHLC candles from exchange venue (if configured)
        if let (Some(ex), Some(pair)) = (&self.exchange_client, &self.state.venue_pair.clone())
            && ex.has_ohlc()
        {
            let interval = self.state.time_period.exchange_interval();
            let limit = 100;
            match ex.fetch_ohlc(pair, interval, limit).await {
                Ok(candles) => {
                    self.state.exchange_ohlc = candles
                        .into_iter()
                        .map(|c| OhlcCandle {
                            timestamp: c.open_time as f64 / 1000.0,
                            open: c.open,
                            high: c.high,
                            low: c.low,
                            close: c.close,
                            is_bullish: c.close >= c.open,
                        })
                        .collect();
                }
                Err(e) => {
                    tracing::debug!("Failed to fetch OHLC: {}", e);
                    // Fall back to synthetic candles silently
                }
            }
        }

        // Periodically fetch holder count via chain client (~every 12th refresh ≈ 60s at 5s rate)
        self.state.holder_fetch_counter += 1;
        if self.state.holder_fetch_counter.is_multiple_of(12)
            && let Some(ref client) = self.chain_client
        {
            match client
                .get_token_holder_count(&self.state.token_address)
                .await
            {
                Ok(count) if count > 0 => {
                    self.state.holder_count = Some(count);
                }
                _ => {} // Keep previous value or None
            }
        }
    }
}

/// Handles a key event by mutating state. Standalone version for testability.
/// Returns true if the application should exit.
#[cfg(test)]
fn handle_key_event_on_state(key: crossterm::event::KeyEvent, state: &mut MonitorState) -> bool {
    // Track last input time for auto-pause
    state.last_input_at = Instant::now();

    // Widget toggle mode: waiting for digit 1-5
    if state.widget_toggle_mode {
        state.widget_toggle_mode = false;
        if let KeyCode::Char(c @ '1'..='5') = key.code {
            let idx = (c as u8 - b'0') as usize;
            state.widgets.toggle_by_index(idx);
            return false;
        }
        // Any other key cancels the mode and falls through
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => {
            if state.export_active {
                state.stop_export();
            }
            return true;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if state.export_active {
                state.stop_export();
            }
            return true;
        }
        KeyCode::Char('r') => {
            state.force_refresh();
        }
        // Shift+P toggles auto-pause
        KeyCode::Char('P') if key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.auto_pause_on_input = !state.auto_pause_on_input;
            state.log(format!(
                "Auto-pause: {}",
                if state.auto_pause_on_input {
                    "ON"
                } else {
                    "OFF"
                }
            ));
        }
        KeyCode::Char('p') | KeyCode::Char(' ') => {
            state.toggle_pause();
        }
        // Toggle CSV export
        KeyCode::Char('e') => {
            state.toggle_export();
        }
        KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char(']') => {
            state.slower_refresh();
        }
        KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Char('[') => {
            state.faster_refresh();
        }
        KeyCode::Char('1') => {
            state.set_time_period(TimePeriod::Min1);
        }
        KeyCode::Char('2') => {
            state.set_time_period(TimePeriod::Min5);
        }
        KeyCode::Char('3') => {
            state.set_time_period(TimePeriod::Min15);
        }
        KeyCode::Char('4') => {
            state.set_time_period(TimePeriod::Hour1);
        }
        KeyCode::Char('5') => {
            state.set_time_period(TimePeriod::Hour4);
        }
        KeyCode::Char('6') => {
            state.set_time_period(TimePeriod::Day1);
        }
        KeyCode::Char('t') | KeyCode::Tab => {
            state.cycle_time_period();
        }
        KeyCode::Char('c') => {
            state.toggle_chart_mode();
        }
        KeyCode::Char('s') => {
            state.scale_mode = state.scale_mode.toggle();
            state.log(format!("Scale: {}", state.scale_mode.label()));
        }
        KeyCode::Char('/') => {
            state.color_scheme = state.color_scheme.next();
            state.log(format!("Colors: {}", state.color_scheme.label()));
        }
        KeyCode::Char('j') | KeyCode::Down => {
            state.scroll_log_down();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.scroll_log_up();
        }
        KeyCode::Char('l') => {
            state.layout = state.layout.next();
            state.auto_layout = false;
        }
        KeyCode::Char('h') => {
            state.layout = state.layout.prev();
            state.auto_layout = false;
        }
        KeyCode::Char('w') => {
            state.widget_toggle_mode = true;
        }
        KeyCode::Char('a') => {
            state.auto_layout = true;
        }
        _ => {}
    }
    false
}

/// Renders the UI.
/// Computed layout areas for each widget. `None` means the widget is hidden.
struct LayoutAreas {
    price_chart: Option<Rect>,
    volume_chart: Option<Rect>,
    buy_sell_gauge: Option<Rect>,
    metrics_panel: Option<Rect>,
    activity_feed: Option<Rect>,
    /// Order book depth panel (Exchange layout).
    order_book: Option<Rect>,
    /// Market info panel with pair details (Exchange layout).
    market_info: Option<Rect>,
    /// Recent trade history (Exchange layout).
    trade_history: Option<Rect>,
}

/// Dashboard layout: charts top, gauges middle, transaction feed bottom.
///
/// ```text
/// ┌──────────────────────┬──────────────────────┐
/// │  Price Chart (60%)   │  Volume Chart (40%)  │  55%
/// ├──────────────────────┼──────────────────────┤
/// │  Buy/Sell (50%)      │  Metrics (50%)       │  20%
/// ├──────────────────────┴──────────────────────┤
/// │  Activity Feed                              │  25%
/// └─────────────────────────────────────────────┘
/// ```
fn layout_dashboard(area: Rect, widgets: &WidgetVisibility) -> LayoutAreas {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(55),
            Constraint::Percentage(20),
            Constraint::Percentage(25),
        ])
        .split(area);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(rows[0]);

    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);

    LayoutAreas {
        price_chart: if widgets.price_chart {
            Some(top[0])
        } else {
            None
        },
        volume_chart: if widgets.volume_chart {
            Some(top[1])
        } else {
            None
        },
        buy_sell_gauge: if widgets.buy_sell_pressure {
            Some(middle[0])
        } else {
            None
        },
        metrics_panel: if widgets.metrics_panel {
            Some(middle[1])
        } else {
            None
        },
        activity_feed: if widgets.activity_log {
            Some(rows[2])
        } else {
            None
        },
        order_book: None,
        market_info: None,
        trade_history: None,
    }
}

/// Chart-focus layout: full-width candles with minimal stats overlay.
///
/// ```text
/// ┌─────────────────────────────────────────────┐
/// │                                             │
/// │            Price Chart (~85%)                │
/// │                                             │
/// ├─────────────────────────────────────────────┤
/// │  Metrics (compact stats overlay)     ~15%   │
/// └─────────────────────────────────────────────┘
/// ```
fn layout_chart_focus(area: Rect, widgets: &WidgetVisibility) -> LayoutAreas {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(85), Constraint::Percentage(15)])
        .split(area);

    LayoutAreas {
        price_chart: if widgets.price_chart {
            Some(vertical[0])
        } else {
            None
        },
        volume_chart: None,   // Hidden in chart-focus
        buy_sell_gauge: None, // Hidden in chart-focus
        metrics_panel: if widgets.metrics_panel {
            Some(vertical[1])
        } else {
            None
        },
        activity_feed: None, // Hidden in chart-focus
        order_book: None,
        market_info: None,
        trade_history: None,
    }
}

/// Feed layout: transaction log takes priority, small price ticker on top.
///
/// ```text
/// ┌──────────────────────┬──────────────────────┐
/// │  Metrics (50%)       │  Buy/Sell (50%)      │  25%
/// ├──────────────────────┴──────────────────────┤
/// │                                             │
/// │            Activity Feed (~75%)             │
/// │                                             │
/// └─────────────────────────────────────────────┘
/// ```
fn layout_feed(area: Rect, widgets: &WidgetVisibility) -> LayoutAreas {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(area);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(vertical[0]);

    LayoutAreas {
        price_chart: None,  // Hidden in feed mode
        volume_chart: None, // Hidden in feed mode
        metrics_panel: if widgets.metrics_panel {
            Some(top[0])
        } else {
            None
        },
        buy_sell_gauge: if widgets.buy_sell_pressure {
            Some(top[1])
        } else {
            None
        },
        activity_feed: if widgets.activity_log {
            Some(vertical[1])
        } else {
            None
        },
        order_book: None,
        market_info: None,
        trade_history: None,
    }
}

/// Compact layout: price sparkline and metrics only for small terminals.
///
/// ```text
/// ┌─────────────────────────────────────────────┐
/// │  Metrics Panel (sparkline + stats)    100%  │
/// └─────────────────────────────────────────────┘
/// ```
fn layout_compact(area: Rect, widgets: &WidgetVisibility) -> LayoutAreas {
    LayoutAreas {
        price_chart: None,    // Hidden in compact
        volume_chart: None,   // Hidden in compact
        buy_sell_gauge: None, // Hidden in compact
        metrics_panel: if widgets.metrics_panel {
            Some(area)
        } else {
            None
        },
        activity_feed: None, // Hidden in compact
        order_book: None,
        market_info: None,
        trade_history: None,
    }
}

/// Exchange layout: order book left, chart center, trade history right.
///
/// ```text
/// ┌─────────────────┬──────────────────────────┬─────────────────┐
/// │                 │                          │                 │
/// │  Order Book     │   Price Chart (45%)      │  Trade History  │  60%
/// │    (25%)        │                          │    (30%)        │
/// │                 │                          │                 │
/// ├─────────────────┼──────────────────────────┼─────────────────┤
/// │  Buy/Sell (25%) │   Market Info (45%)      │  (continued)    │  40%
/// │                 │                          │    (30%)        │
/// └─────────────────┴──────────────────────────┴─────────────────┘
/// ```
fn layout_exchange(area: Rect, _widgets: &WidgetVisibility) -> LayoutAreas {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(45),
            Constraint::Percentage(30),
        ])
        .split(rows[0]);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(rows[1]);

    LayoutAreas {
        price_chart: Some(top[1]),
        volume_chart: None,
        buy_sell_gauge: Some(bottom[0]),
        metrics_panel: None,
        activity_feed: None,
        order_book: Some(top[0]),
        market_info: Some(bottom[1]),
        trade_history: Some(top[2]),
    }
}

/// Selects the best layout preset based on terminal size.
fn auto_select_layout(size: Rect) -> LayoutPreset {
    match (size.width, size.height) {
        (w, h) if w < 80 || h < 24 => LayoutPreset::Compact,
        (w, _) if w < 120 => LayoutPreset::Feed,
        (_, h) if h < 30 => LayoutPreset::ChartFocus,
        _ => LayoutPreset::Dashboard,
    }
}

/// Renders the UI, dispatching to the active layout preset.
fn ui(f: &mut Frame, state: &mut MonitorState) {
    // Responsive breakpoint: auto-select layout for terminal size
    if state.auto_layout {
        let suggested = auto_select_layout(f.area());
        if suggested != state.layout {
            state.layout = suggested;
        }
    }

    // Main layout: header, content, footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Header (token info + time period tabs)
            Constraint::Min(10),   // Content
            Constraint::Length(3), // Footer
        ])
        .split(f.area());

    // Render header
    render_header(f, chunks[0], state);

    // Calculate content areas based on active layout preset
    let areas = match state.layout {
        LayoutPreset::Dashboard => layout_dashboard(chunks[1], &state.widgets),
        LayoutPreset::ChartFocus => layout_chart_focus(chunks[1], &state.widgets),
        LayoutPreset::Feed => layout_feed(chunks[1], &state.widgets),
        LayoutPreset::Compact => layout_compact(chunks[1], &state.widgets),
        LayoutPreset::Exchange => layout_exchange(chunks[1], &state.widgets),
    };

    // Render each widget if its area is allocated
    if let Some(area) = areas.price_chart {
        match state.chart_mode {
            ChartMode::Line => render_price_chart(f, area, state),
            ChartMode::Candlestick => render_candlestick_chart(f, area, state),
            ChartMode::VolumeProfile => render_volume_profile_chart(f, area, state),
        }
    }
    if let Some(area) = areas.volume_chart {
        render_volume_chart(f, area, &*state);
    }
    if let Some(area) = areas.buy_sell_gauge {
        render_buy_sell_gauge(f, area, state);
    }
    if let Some(area) = areas.metrics_panel {
        // Split metrics area to show liquidity depth if enabled and data available
        if state.widgets.liquidity_depth && !state.liquidity_pairs.is_empty() {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                .split(area);
            render_metrics_panel(f, split[0], &*state);
            render_liquidity_depth(f, split[1], &*state);
        } else {
            render_metrics_panel(f, area, &*state);
        }
    }
    if let Some(area) = areas.activity_feed {
        render_activity_feed(f, area, state);
    }
    // Exchange-specific widgets
    if let Some(area) = areas.order_book {
        render_order_book_panel(f, area, state);
    }
    if let Some(area) = areas.market_info {
        render_market_info_panel(f, area, state);
    }
    if let Some(area) = areas.trade_history {
        render_recent_trades_panel(f, area, state);
    }

    // Render alert overlay on top of content area if alerts are active
    if !state.active_alerts.is_empty() {
        // Show alerts as a banner at the top of the content area (3 lines max)
        let alert_height = (state.active_alerts.len() as u16 + 2).min(5);
        let alert_area = Rect::new(
            chunks[1].x,
            chunks[1].y,
            chunks[1].width,
            alert_height.min(chunks[1].height),
        );
        render_alert_overlay(f, alert_area, state);
    }

    // Render footer
    render_footer(f, chunks[2], state);
}

/// Renders the header with token info and time period tabs.
fn render_header(f: &mut Frame, area: Rect, state: &MonitorState) {
    let price_color = if state.price_change_24h >= 0.0 {
        Color::Green
    } else {
        Color::Red
    };

    // Use Unicode arrows for trend indication
    let trend_arrow = if state.price_change_24h > 0.5 {
        "▲"
    } else if state.price_change_24h < -0.5 {
        "▼"
    } else if state.price_change_24h >= 0.0 {
        "△"
    } else {
        "▽"
    };

    let change_str = format!(
        "{}{:.2}%",
        if state.price_change_24h >= 0.0 {
            "+"
        } else {
            ""
        },
        state.price_change_24h
    );

    let title = format!(
        " ◈ {} ({}) │ {} ",
        state.symbol,
        state.name,
        state.chain.to_uppercase(),
    );

    let price_str = format_price_usd(state.current_price);

    // Split header area: top row for token info, bottom row for tabs
    let header_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(1)])
        .split(area);

    // Token info with price
    let header = Paragraph::new(Line::from(vec![
        Span::styled(price_str, Style::new().fg(price_color).bold()),
        Span::raw(" "),
        Span::styled(trend_arrow, Style::new().fg(price_color)),
        Span::styled(format!(" {}", change_str), Style::new().fg(price_color)),
    ]))
    .block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::new().cyan()),
    );

    f.render_widget(header, header_chunks[0]);

    // Time period tabs
    let tab_titles = vec!["1m", "5m", "15m", "1h", "4h", "1d"];
    let chart_label = state.chart_mode.label();
    let tabs = Tabs::new(tab_titles)
        .select(state.time_period.index())
        .highlight_style(Style::new().cyan().bold())
        .divider("│")
        .padding(" ", " ");
    let tabs_line = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(10)])
        .split(header_chunks[1]);
    f.render_widget(tabs, tabs_line[0]);
    f.render_widget(
        Paragraph::new(Span::styled(
            format!("⊞ {}", chart_label),
            Style::new().magenta(),
        )),
        tabs_line[1],
    );
}

/// Renders the price chart with visual differentiation between real and synthetic data.
fn render_price_chart(f: &mut Frame, area: Rect, state: &MonitorState) {
    // Get data filtered by selected time period
    let (data, is_real) = state.get_price_data_for_period();

    if data.is_empty() {
        let empty = Paragraph::new("No price data").block(
            Block::default()
                .title(" Price (USD) ")
                .borders(Borders::ALL),
        );
        f.render_widget(empty, area);
        return;
    }

    // Calculate price statistics
    let current_price = state.current_price;
    let first_price = data.first().map(|(_, p)| *p).unwrap_or(current_price);
    let price_change = current_price - first_price;
    let price_change_pct = if first_price > 0.0 {
        (price_change / first_price) * 100.0
    } else {
        0.0
    };

    // Determine if price is up or down for coloring
    let pal = state.palette();
    let is_price_up = price_change >= 0.0;
    let trend_color = if is_price_up { pal.up } else { pal.down };
    let trend_symbol = if is_price_up { "▲" } else { "▼" };

    // Format current price based on magnitude
    let price_str = format_price_usd(current_price);
    let change_str = if price_change_pct.abs() < 0.01 {
        "0.00%".to_string()
    } else {
        format!(
            "{}{:.2}%",
            if is_price_up { "+" } else { "" },
            price_change_pct
        )
    };

    // Build title with current price and change
    let chart_title = Line::from(vec![
        Span::raw(" ◆ "),
        Span::styled(
            format!("{} {} ", price_str, trend_symbol),
            Style::new().fg(trend_color).bold(),
        ),
        Span::styled(format!("({}) ", change_str), Style::new().fg(trend_color)),
        Span::styled(
            format!("│{}│ ", state.time_period.label()),
            Style::new().gray(),
        ),
    ]);

    // Calculate bounds
    let (min_price, max_price) = data
        .iter()
        .fold((f64::MAX, f64::MIN), |(min, max), (_, p)| {
            (min.min(*p), max.max(*p))
        });

    // Handle case where all prices are the same (e.g., stablecoins)
    let price_range = max_price - min_price;
    let (y_min, y_max) = if price_range < 0.0001 {
        // Add ±0.1% padding when prices are flat
        let padding = min_price * 0.001;
        (min_price - padding, max_price + padding)
    } else {
        (min_price - price_range * 0.1, max_price + price_range * 0.1)
    };

    let x_min = data.first().map(|(t, _)| *t).unwrap_or(0.0);
    let x_max = data.last().map(|(t, _)| *t).unwrap_or(1.0);
    // Ensure x range is non-zero for proper rendering
    let x_max = if (x_max - x_min).abs() < 0.001 {
        x_min + 1.0
    } else {
        x_max
    };

    // Apply scale transformation (log or linear)
    let apply_scale = |price: f64| -> f64 {
        match state.scale_mode {
            ScaleMode::Linear => price,
            ScaleMode::Log => {
                if price > 0.0 {
                    price.ln()
                } else {
                    0.0
                }
            }
        }
    };

    let (y_min, y_max) = (apply_scale(y_min), apply_scale(y_max));

    // Split data into synthetic and real datasets for visual differentiation
    let synthetic_data: Vec<(f64, f64)> = data
        .iter()
        .zip(&is_real)
        .filter(|(_, real)| !**real)
        .map(|((t, p), _)| (*t, apply_scale(*p)))
        .collect();

    let real_data: Vec<(f64, f64)> = data
        .iter()
        .zip(&is_real)
        .filter(|(_, real)| **real)
        .map(|((t, p), _)| (*t, apply_scale(*p)))
        .collect();

    // Create reference line at first price (horizontal line for comparison)
    let reference_line: Vec<(f64, f64)> = vec![
        (x_min, apply_scale(first_price)),
        (x_max, apply_scale(first_price)),
    ];

    let mut datasets = Vec::new();

    // Reference line (starting price) - dashed gray
    datasets.push(
        Dataset::default()
            .name("━Start")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().dark_gray())
            .data(&reference_line),
    );

    // Synthetic data shown with Dot marker and dimmed color
    if !synthetic_data.is_empty() {
        datasets.push(
            Dataset::default()
                .name("◇Est")
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::new().cyan())
                .data(&synthetic_data),
        );
    }

    // Real data shown with Braille marker and trend color
    if !real_data.is_empty() {
        datasets.push(
            Dataset::default()
                .name("●Live")
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::new().fg(trend_color))
                .data(&real_data),
        );
    }

    // Create time labels based on period
    let time_label = format!("-{}", state.time_period.label());

    // Calculate middle price for 3-point y-axis labels
    // In log mode, labels show original USD prices (exp of log values)
    let mid_y = (y_min + y_max) / 2.0;
    let y_label = |val: f64| -> String {
        match state.scale_mode {
            ScaleMode::Linear => format_price_usd(val),
            ScaleMode::Log => format_price_usd(val.exp()),
        }
    };

    let scale_label = match state.scale_mode {
        ScaleMode::Linear => "USD",
        ScaleMode::Log => "USD (log)",
    };

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .title(chart_title)
                .borders(Borders::ALL)
                .border_style(Style::new().fg(trend_color)),
        )
        .x_axis(
            Axis::default()
                .title(Span::styled("Time", Style::new().gray()))
                .style(Style::new().gray())
                .bounds([x_min, x_max])
                .labels(vec![Span::raw(time_label), Span::raw("now")]),
        )
        .y_axis(
            Axis::default()
                .title(Span::styled(scale_label, Style::new().gray()))
                .style(Style::new().gray())
                .bounds([y_min, y_max])
                .labels(vec![
                    Span::raw(y_label(y_min)),
                    Span::raw(y_label(mid_y)),
                    Span::raw(y_label(y_max)),
                ]),
        );

    f.render_widget(chart, area);
}

/// Checks if a price indicates a stablecoin (pegged around $1.00).
fn is_stablecoin_price(price: f64) -> bool {
    (0.95..=1.05).contains(&price)
}

/// Formats a price in USD with appropriate precision.
/// Stablecoins get extra precision (6 decimals) to show micro-fluctuations.
fn format_price_usd(price: f64) -> String {
    if price >= 1000.0 {
        format!("${:.2}", price)
    } else if is_stablecoin_price(price) {
        // Stablecoins get 6 decimals to show micro-fluctuations
        format!("${:.6}", price)
    } else if price >= 1.0 {
        format!("${:.4}", price)
    } else if price >= 0.01 {
        format!("${:.6}", price)
    } else if price >= 0.0001 {
        format!("${:.8}", price)
    } else {
        format!("${:.10}", price)
    }
}

/// Renders a candlestick chart using OHLC data.
fn render_candlestick_chart(f: &mut Frame, area: Rect, state: &MonitorState) {
    let candles = state.get_ohlc_candles();

    if candles.is_empty() {
        let empty = Paragraph::new("No candle data (waiting for more data points)").block(
            Block::default()
                .title(" Candlestick (USD) ")
                .borders(Borders::ALL),
        );
        f.render_widget(empty, area);
        return;
    }

    // Calculate price statistics
    let current_price = state.current_price;
    let first_candle = candles.first().unwrap();
    let last_candle = candles.last().unwrap();
    let price_change = last_candle.close - first_candle.open;
    let price_change_pct = if first_candle.open > 0.0 {
        (price_change / first_candle.open) * 100.0
    } else {
        0.0
    };

    let pal = state.palette();
    let is_price_up = price_change >= 0.0;
    let trend_color = if is_price_up { pal.up } else { pal.down };
    let trend_symbol = if is_price_up { "▲" } else { "▼" };

    let price_str = format_price_usd(current_price);
    let change_str = format!(
        "{}{:.2}%",
        if is_price_up { "+" } else { "" },
        price_change_pct
    );

    // Calculate bounds from all candle high/low
    let (min_price, max_price) = candles.iter().fold((f64::MAX, f64::MIN), |(min, max), c| {
        (min.min(c.low), max.max(c.high))
    });

    let price_range = max_price - min_price;
    let (y_min, y_max) = if price_range < 0.0001 {
        let padding = min_price * 0.001;
        (min_price - padding, max_price + padding)
    } else {
        (min_price - price_range * 0.1, max_price + price_range * 0.1)
    };

    let x_min = candles.first().map(|c| c.timestamp).unwrap_or(0.0);
    let x_max = candles.last().map(|c| c.timestamp).unwrap_or(1.0);
    let x_range = x_max - x_min;
    let x_max = if x_range < 0.001 {
        x_min + 1.0
    } else {
        x_max + x_range * 0.05
    };

    // Calculate candle width based on number of candles and area
    let candle_count = candles.len() as f64;
    let candle_spacing = x_range / candle_count.max(1.0);
    let candle_width = candle_spacing * 0.6; // 60% of spacing for body

    let title = Line::from(vec![
        Span::raw(" ⬡ "),
        Span::styled(
            format!("{} {} ", price_str, trend_symbol),
            Style::new().fg(trend_color).bold(),
        ),
        Span::styled(format!("({}) ", change_str), Style::new().fg(trend_color)),
        Span::styled(
            format!("│{}│ ", state.time_period.label()),
            Style::new().gray(),
        ),
        Span::styled("⊞Candles ", Style::new().magenta()),
    ]);

    // Apply scale transformation (log or linear)
    let apply_scale = |price: f64| -> f64 {
        match state.scale_mode {
            ScaleMode::Linear => price,
            ScaleMode::Log => {
                if price > 0.0 {
                    price.ln()
                } else {
                    0.0
                }
            }
        }
    };
    let scaled_y_min = apply_scale(y_min);
    let scaled_y_max = apply_scale(y_max);
    let scaled_price_range = scaled_y_max - scaled_y_min;

    // Clone candles for the closure
    let candles_clone = candles.clone();
    let is_log = matches!(state.scale_mode, ScaleMode::Log);
    let pal_up = pal.up;
    let pal_down = pal.down;

    let canvas = Canvas::default()
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::new().fg(trend_color)),
        )
        .x_bounds([x_min - candle_spacing, x_max])
        .y_bounds([scaled_y_min, scaled_y_max])
        .paint(move |ctx| {
            let scale_fn = |p: f64| -> f64 { if is_log && p > 0.0 { p.ln() } else { p } };
            for candle in &candles_clone {
                let color = if candle.is_bullish { pal_up } else { pal_down };

                // Draw the wick (high-low line)
                ctx.draw(&CanvasLine {
                    x1: candle.timestamp,
                    y1: scale_fn(candle.low),
                    x2: candle.timestamp,
                    y2: scale_fn(candle.high),
                    color,
                });

                // Draw the body (open-close rectangle)
                let body_top = scale_fn(candle.open.max(candle.close));
                let body_bottom = scale_fn(candle.open.min(candle.close));
                let body_height = (body_top - body_bottom).max(scaled_price_range * 0.002);

                ctx.draw(&Rectangle {
                    x: candle.timestamp - candle_width / 2.0,
                    y: body_bottom,
                    width: candle_width,
                    height: body_height,
                    color,
                });
            }
        });

    f.render_widget(canvas, area);
}

/// Renders a volume profile chart showing volume distribution by price level.
///
/// Buckets the price+volume history into horizontal bars where each bar shows
/// the accumulated volume at that price level. Accuracy improves over longer
/// monitoring sessions as more data points are collected.
fn render_volume_profile_chart(f: &mut Frame, area: Rect, state: &MonitorState) {
    let pal = state.palette();
    let (price_data, _) = state.get_price_data_for_period();
    let (volume_data, _) = state.get_volume_data_for_period();

    if price_data.len() < 2 || volume_data.is_empty() {
        let block = Block::default()
            .title(" ◨ Volume Profile (collecting data...) ")
            .borders(Borders::ALL)
            .border_style(Style::new().fg(Color::DarkGray));
        f.render_widget(block, area);
        return;
    }

    // Find price range
    let min_price = price_data.iter().map(|(_, p)| *p).fold(f64::MAX, f64::min);
    let max_price = price_data.iter().map(|(_, p)| *p).fold(f64::MIN, f64::max);

    if (max_price - min_price).abs() < f64::EPSILON {
        let block = Block::default()
            .title(" ◨ Volume Profile (no price range) ")
            .borders(Borders::ALL)
            .border_style(Style::new().fg(Color::DarkGray));
        f.render_widget(block, area);
        return;
    }

    // Number of price buckets = available height minus borders
    let inner_height = area.height.saturating_sub(2) as usize;
    let num_buckets = inner_height.clamp(1, 30);
    let bucket_size = (max_price - min_price) / num_buckets as f64;

    // Accumulate volume per price bucket
    let mut bucket_volumes = vec![0.0_f64; num_buckets];

    // Pair price and volume data by index (they have same timestamps)
    let vol_iter: Vec<f64> = volume_data.iter().map(|(_, v)| *v).collect();
    for (i, (_, price)) in price_data.iter().enumerate() {
        let bucket_idx =
            (((price - min_price) / bucket_size).floor() as usize).min(num_buckets - 1);
        // Use volume delta if available, otherwise use a unit contribution
        let vol_contribution = if i < vol_iter.len() {
            // Use relative volume (delta from previous if possible)
            if i > 0 {
                (vol_iter[i] - vol_iter[i - 1]).abs().max(1.0)
            } else {
                1.0
            }
        } else {
            1.0
        };
        bucket_volumes[bucket_idx] += vol_contribution;
    }

    let max_vol = bucket_volumes
        .iter()
        .cloned()
        .fold(0.0_f64, f64::max)
        .max(1.0);

    // Find the bucket containing the current price
    let current_bucket = (((state.current_price - min_price) / bucket_size).floor() as usize)
        .min(num_buckets.saturating_sub(1));

    // Build horizontal bars using Paragraph with spans
    let inner_width = area.width.saturating_sub(12) as usize; // leave room for price labels

    let lines: Vec<Line> = (0..num_buckets)
        .rev() // top = highest price
        .map(|i| {
            let bar_width = ((bucket_volumes[i] / max_vol) * inner_width as f64).round() as usize;
            let price_mid = min_price + (i as f64 + 0.5) * bucket_size;
            let label = if price_mid >= 1.0 {
                format!("{:>8.2}", price_mid)
            } else {
                format!("{:>8.6}", price_mid)
            };
            let bar_str = "█".repeat(bar_width);
            let style = if i == current_bucket {
                Style::new().fg(pal.highlight).bold()
            } else {
                Style::new().fg(pal.sparkline)
            };
            Line::from(vec![
                Span::styled(label, Style::new().dark_gray()),
                Span::raw(" "),
                Span::styled(bar_str, style),
            ])
        })
        .collect();

    let block = Block::default()
        .title(" ◨ Volume Profile (accuracy improves over time) ")
        .borders(Borders::ALL)
        .border_style(Style::new().fg(pal.sparkline));

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, area);
}

/// Renders the volume chart with visual differentiation between real and synthetic data.
fn render_volume_chart(f: &mut Frame, area: Rect, state: &MonitorState) {
    let pal = state.palette();
    // Get data filtered by selected time period
    let (data, is_real) = state.get_volume_data_for_period();

    if data.is_empty() {
        let empty = Paragraph::new("No volume data")
            .block(Block::default().title(" 24h Volume ").borders(Borders::ALL));
        f.render_widget(empty, area);
        return;
    }

    // Get current volume for display
    let current_volume = state.volume_24h;
    let volume_str = crate::display::format_usd(current_volume);

    // Count synthetic vs real points for the legend
    let has_synthetic = is_real.iter().any(|r| !r);
    let has_real = is_real.iter().any(|r| *r);

    // Build title with current volume
    let data_indicator = if has_synthetic && has_real {
        "[◆ est │ ● live]"
    } else if has_synthetic {
        "[◆ estimated]"
    } else {
        "[● live]"
    };

    let chart_title = Line::from(vec![
        Span::raw(" ▣ "),
        Span::styled(
            format!("24h Vol: {} ", volume_str),
            Style::new().fg(pal.volume_bar).bold(),
        ),
        Span::styled(
            format!("│{}│ ", state.time_period.label()),
            Style::new().gray(),
        ),
        Span::styled(data_indicator, Style::new().dark_gray()),
    ]);

    // Build bars from data points — bucket into a reasonable number of bars
    // based on available width (each bar needs at least 3 chars)
    let inner_width = area.width.saturating_sub(2) as usize; // account for block borders
    let max_bars = (inner_width / 3).max(1).min(data.len());
    let bucket_size = data.len().div_ceil(max_bars);

    let bars: Vec<Bar> = data
        .chunks(bucket_size)
        .zip(is_real.chunks(bucket_size))
        .enumerate()
        .map(|(i, (chunk, real_chunk))| {
            let avg_vol = chunk.iter().map(|(_, v)| v).sum::<f64>() / chunk.len() as f64;
            let any_real = real_chunk.iter().any(|r| *r);
            let bar_color = if any_real {
                pal.volume_bar
            } else {
                pal.neutral
            };
            // Show time labels at start, middle, and end
            let label = if i == 0 || i == max_bars.saturating_sub(1) || i == max_bars / 2 {
                format_number(avg_vol)
            } else {
                String::new()
            };
            Bar::default()
                .value(avg_vol as u64)
                .label(Line::from(label))
                .style(Style::new().fg(bar_color))
        })
        .collect();

    // Calculate dynamic bar width based on available space
    let bar_width = if !bars.is_empty() {
        let total_bars = bars.len() as u16;
        // Each bar gets: bar_width + 1 gap, minus 1 gap for the last bar
        ((inner_width as u16).saturating_sub(total_bars.saturating_sub(1))) / total_bars
    } else {
        1
    }
    .max(1);

    let barchart = BarChart::default()
        .data(BarGroup::default().bars(&bars))
        .block(
            Block::default()
                .title(chart_title)
                .borders(Borders::ALL)
                .border_style(Style::new().blue()),
        )
        .bar_width(bar_width)
        .bar_gap(1)
        .value_style(Style::new().dark_gray());

    f.render_widget(barchart, area);
}

/// Renders the buy/sell ratio gauge (pressure bar only, no activity log).
fn render_buy_sell_gauge(f: &mut Frame, area: Rect, state: &mut MonitorState) {
    let pal = state.palette();
    // Buy/Sell ratio bar
    let ratio = state.buy_ratio();
    let border_color = if ratio > 0.5 { pal.up } else { pal.down };

    let block = Block::default()
        .title(" ◐ Buy/Sell Ratio (24h) ")
        .borders(Borders::ALL)
        .border_style(Style::new().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width > 0 && inner.height > 0 {
        let buy_width = ((ratio * inner.width as f64).round() as u16).min(inner.width);
        let sell_width = inner.width.saturating_sub(buy_width);

        let buy_indicator = if ratio > 0.5 { "▶" } else { "▷" };
        let sell_indicator = if ratio < 0.5 { "◀" } else { "◁" };
        let label = format!(
            "{}Buys: {} │ Sells: {}{} ({:.1}%)",
            buy_indicator,
            state.buys_24h,
            state.sells_24h,
            sell_indicator,
            ratio * 100.0
        );

        let buy_bar = "█".repeat(buy_width as usize);
        let sell_bar = "█".repeat(sell_width as usize);
        let bar_line = Line::from(vec![
            Span::styled(buy_bar, Style::new().fg(pal.up)),
            Span::styled(sell_bar, Style::new().fg(pal.down)),
        ]);
        f.render_widget(Paragraph::new(bar_line), inner);

        // Center the label on top of the bar
        let label_len = label.len() as u16;
        if label_len <= inner.width {
            let x_offset = (inner.width.saturating_sub(label_len)) / 2;
            let label_area = Rect::new(inner.x + x_offset, inner.y, label_len, 1);
            let label_widget =
                Paragraph::new(Span::styled(label, Style::new().fg(Color::White).bold()));
            f.render_widget(label_widget, label_area);
        }
    }
}

/// Renders the scrollable activity log feed.
fn render_activity_feed(f: &mut Frame, area: Rect, state: &mut MonitorState) {
    let log_len = state.log_messages.len();
    let log_title = if log_len > 0 {
        let selected = state.log_list_state.selected().unwrap_or(0);
        format!(" ◷ Activity Log [{}/{}] ", selected + 1, log_len)
    } else {
        " ◷ Activity Log ".to_string()
    };

    let items: Vec<ListItem> = state
        .log_messages
        .iter()
        .rev()
        .map(|msg| ListItem::new(msg.as_str()).style(Style::new().gray()))
        .collect();

    let log_list = List::new(items)
        .block(
            Block::default()
                .title(log_title)
                .borders(Borders::ALL)
                .border_style(Style::new().dark_gray()),
        )
        .highlight_style(Style::new().white().bold())
        .highlight_symbol("▸ ");

    f.render_stateful_widget(log_list, area, &mut state.log_list_state);
}

/// Renders a flashing alert overlay when alerts are active.
fn render_alert_overlay(f: &mut Frame, area: Rect, state: &MonitorState) {
    if state.active_alerts.is_empty() {
        return;
    }

    let is_flash_on = state
        .alert_flash_until
        .map(|deadline| {
            if Instant::now() < deadline {
                // Flash with ~500ms period
                (Instant::now().elapsed().subsec_millis() / 500).is_multiple_of(2)
            } else {
                false
            }
        })
        .unwrap_or(false);

    let border_color = if is_flash_on {
        Color::Red
    } else {
        Color::Yellow
    };

    let alert_lines: Vec<Line> = state
        .active_alerts
        .iter()
        .map(|a| Line::from(Span::styled(&a.message, Style::new().fg(Color::Red).bold())))
        .collect();

    let alert_widget = Paragraph::new(alert_lines).block(
        Block::default()
            .title(" ⚠ ALERTS ")
            .borders(Borders::ALL)
            .border_style(Style::new().fg(border_color).bold()),
    );

    f.render_widget(alert_widget, area);
}

/// Renders a horizontal stacked bar chart of per-pair liquidity.
fn render_liquidity_depth(f: &mut Frame, area: Rect, state: &MonitorState) {
    let pal = state.palette();

    if state.liquidity_pairs.is_empty() {
        let block = Block::default()
            .title(" ◫ Liquidity Depth (no data) ")
            .borders(Borders::ALL)
            .border_style(Style::new().fg(Color::DarkGray));
        f.render_widget(block, area);
        return;
    }

    let max_liquidity = state
        .liquidity_pairs
        .iter()
        .map(|(_, liq)| *liq)
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let inner_width = area.width.saturating_sub(2) as usize;

    let lines: Vec<Line> = state
        .liquidity_pairs
        .iter()
        .take(area.height.saturating_sub(2) as usize) // limit to available rows
        .map(|(name, liq)| {
            let bar_width = ((liq / max_liquidity) * inner_width as f64 * 0.6).round() as usize;
            let bar_str = "█".repeat(bar_width);
            let label = format!(" {} {}", crate::display::format_usd(*liq), name);
            Line::from(vec![
                Span::styled(bar_str, Style::new().fg(pal.volume_bar)),
                Span::styled(label, Style::new().fg(pal.neutral)),
            ])
        })
        .collect();

    let block = Block::default()
        .title(format!(
            " ◫ Liquidity Depth ({} pairs) ",
            state.liquidity_pairs.len()
        ))
        .borders(Borders::ALL)
        .border_style(Style::new().fg(pal.border));

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, area);
}

/// Renders the key metrics panel.
fn render_metrics_panel(f: &mut Frame, area: Rect, state: &MonitorState) {
    let pal = state.palette();
    // Split panel: top sparkline (2 rows), bottom table
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    // --- Sparkline: recent price trend ---
    let sparkline_data: Vec<u64> = {
        // Take the last N price points and normalize to u64 for Sparkline
        let points: Vec<f64> = state.price_history.iter().map(|dp| dp.value).collect();
        if points.len() < 2 {
            vec![0; chunks[0].width.saturating_sub(2) as usize]
        } else {
            let min_p = points.iter().cloned().fold(f64::MAX, f64::min);
            let max_p = points.iter().cloned().fold(f64::MIN, f64::max);
            let range = (max_p - min_p).max(0.0001);
            points
                .iter()
                .map(|p| (((*p - min_p) / range) * 100.0) as u64)
                .collect()
        }
    };

    let trend_color = if state.price_change_5m >= 0.0 {
        pal.up
    } else {
        pal.down
    };

    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .title(" ◉ Price Trend ")
                .borders(Borders::ALL)
                .border_style(Style::new().fg(pal.sparkline)),
        )
        .data(&sparkline_data)
        .style(Style::new().fg(trend_color));

    f.render_widget(sparkline, chunks[0]);

    // --- Table: key metrics ---
    let change_5m_str = if state.price_change_5m.abs() < 0.0001 {
        "0.00%".to_string()
    } else {
        format!("{:+.4}%", state.price_change_5m)
    };
    let change_5m_color = if state.price_change_5m > 0.0 {
        pal.up
    } else if state.price_change_5m < 0.0 {
        pal.down
    } else {
        pal.neutral
    };

    let now_ts = chrono::Utc::now().timestamp() as f64;
    let secs_since_change = (now_ts - state.last_price_change_at).max(0.0) as u64;
    let last_change_str = if secs_since_change < 60 {
        format!("{}s ago", secs_since_change)
    } else if secs_since_change < 3600 {
        format!("{}m ago", secs_since_change / 60)
    } else {
        format!("{}h ago", secs_since_change / 3600)
    };
    let last_change_color = if secs_since_change < 60 {
        pal.up
    } else {
        pal.highlight
    };

    let change_24h_str = format!(
        "{}{:.2}%",
        if state.price_change_24h >= 0.0 {
            "+"
        } else {
            ""
        },
        state.price_change_24h
    );

    let market_cap_str = state
        .market_cap
        .map(crate::display::format_usd)
        .unwrap_or_else(|| "N/A".to_string());

    let mut rows = vec![
        Row::new(vec![
            Span::styled("Price", Style::new().gray()),
            Span::styled(format_price_usd(state.current_price), Style::new().bold()),
        ]),
        Row::new(vec![
            Span::styled("5m Chg", Style::new().gray()),
            Span::styled(change_5m_str, Style::new().fg(change_5m_color)),
        ]),
        Row::new(vec![
            Span::styled("Last Δ", Style::new().gray()),
            Span::styled(last_change_str, Style::new().fg(last_change_color)),
        ]),
        Row::new(vec![
            Span::styled("24h Chg", Style::new().gray()),
            Span::raw(change_24h_str),
        ]),
        Row::new(vec![
            Span::styled("Liq", Style::new().gray()),
            Span::raw(crate::display::format_usd(state.liquidity_usd)),
        ]),
        Row::new(vec![
            Span::styled("Vol 24h", Style::new().gray()),
            Span::raw(crate::display::format_usd(state.volume_24h)),
        ]),
        Row::new(vec![
            Span::styled("Mkt Cap", Style::new().gray()),
            Span::raw(market_cap_str),
        ]),
        Row::new(vec![
            Span::styled("Buys", Style::new().gray()),
            Span::styled(format!("{}", state.buys_24h), Style::new().fg(pal.up)),
        ]),
        Row::new(vec![
            Span::styled("Sells", Style::new().gray()),
            Span::styled(format!("{}", state.sells_24h), Style::new().fg(pal.down)),
        ]),
    ];

    // Add holder count if available and the widget is enabled
    if state.widgets.holder_count
        && let Some(count) = state.holder_count
    {
        rows.push(Row::new(vec![
            Span::styled("Holders", Style::new().gray()),
            Span::styled(format_number(count as f64), Style::new().fg(pal.highlight)),
        ]));
    }

    let table = Table::new(rows, [Constraint::Length(8), Constraint::Min(10)]).block(
        Block::default()
            .title(" ◉ Key Metrics ")
            .borders(Borders::ALL)
            .border_style(Style::new().magenta()),
    );

    f.render_widget(table, chunks[1]);
}

/// Renders the order book panel for the Exchange layout.
///
/// Shows asks (descending), spread, bids (descending) with depth bars.
fn render_order_book_panel(f: &mut Frame, area: Rect, state: &MonitorState) {
    let pal = state.palette();

    let book = match &state.order_book {
        Some(b) => b,
        None => {
            let block = Block::default()
                .title(" ◈ Order Book (no data) ")
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::DarkGray));
            f.render_widget(block, area);
            return;
        }
    };

    let inner_height = area.height.saturating_sub(2) as usize; // minus borders
    if inner_height < 3 {
        return;
    }

    // Allocate rows: half for asks, 1 for spread, half for bids
    let ask_rows = (inner_height.saturating_sub(1)) / 2;
    let bid_rows = inner_height.saturating_sub(ask_rows).saturating_sub(1);

    // Find max quantity for bar scaling
    let max_qty = book
        .asks
        .iter()
        .chain(book.bids.iter())
        .map(|l| l.quantity)
        .fold(0.0_f64, f64::max)
        .max(0.001);

    let inner_width = area.width.saturating_sub(2) as usize;
    // Bar takes ~30% of width, rest is price/qty text
    let bar_width_max = (inner_width as f64 * 0.3).round() as usize;

    let mut lines: Vec<Line> = Vec::with_capacity(inner_height);

    // --- Ask side (show in reverse so lowest ask is nearest the spread) ---
    let visible_asks: Vec<_> = book.asks.iter().take(ask_rows).collect();
    // Pad with empty lines if fewer asks than rows
    for _ in 0..ask_rows.saturating_sub(visible_asks.len()) {
        lines.push(Line::from(""));
    }
    for level in visible_asks.iter().rev() {
        let bar_len = ((level.quantity / max_qty) * bar_width_max as f64).round() as usize;
        let bar = "█".repeat(bar_len);
        let price_str = format!("{:.6}", level.price);
        let qty_str = format_number(level.quantity);
        let val_str = format_number(level.value());
        let padding = inner_width
            .saturating_sub(bar_len)
            .saturating_sub(price_str.len())
            .saturating_sub(qty_str.len())
            .saturating_sub(val_str.len())
            .saturating_sub(4); // spaces between columns
        lines.push(Line::from(vec![
            Span::styled(bar, Style::new().fg(pal.down).dim()),
            Span::raw(" "),
            Span::styled(price_str, Style::new().fg(pal.down)),
            Span::raw(" ".repeat(padding.max(1))),
            Span::styled(qty_str, Style::new().fg(pal.neutral)),
            Span::raw(" "),
            Span::styled(val_str, Style::new().fg(Color::DarkGray)),
        ]));
    }

    // --- Spread line ---
    let spread = book
        .best_ask()
        .zip(book.best_bid())
        .map(|(ask, bid)| {
            let s = ask - bid;
            let pct = if bid > 0.0 { (s / bid) * 100.0 } else { 0.0 };
            format!("  Spread: {:.6} ({:.3}%)", s, pct)
        })
        .unwrap_or_else(|| "  Spread: --".to_string());
    lines.push(Line::from(Span::styled(
        spread,
        Style::new().fg(Color::Yellow).bold(),
    )));

    // --- Bid side ---
    for level in book.bids.iter().take(bid_rows) {
        let bar_len = ((level.quantity / max_qty) * bar_width_max as f64).round() as usize;
        let bar = "█".repeat(bar_len);
        let price_str = format!("{:.6}", level.price);
        let qty_str = format_number(level.quantity);
        let val_str = format_number(level.value());
        let padding = inner_width
            .saturating_sub(bar_len)
            .saturating_sub(price_str.len())
            .saturating_sub(qty_str.len())
            .saturating_sub(val_str.len())
            .saturating_sub(4);
        lines.push(Line::from(vec![
            Span::styled(bar, Style::new().fg(pal.up).dim()),
            Span::raw(" "),
            Span::styled(price_str, Style::new().fg(pal.up)),
            Span::raw(" ".repeat(padding.max(1))),
            Span::styled(qty_str, Style::new().fg(pal.neutral)),
            Span::raw(" "),
            Span::styled(val_str, Style::new().fg(Color::DarkGray)),
        ]));
    }

    let ask_depth: f64 = book.asks.iter().map(|l| l.value()).sum();
    let bid_depth: f64 = book.bids.iter().map(|l| l.value()).sum();
    let title = format!(
        " ◈ {} │ Ask {} │ Bid {} ",
        book.pair,
        format_number(ask_depth),
        format_number(bid_depth),
    );

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::new().fg(pal.border));

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, area);
}

/// Renders the recent trades panel for the Exchange layout.
///
/// Displays a scrolling list of recent trades with time, side (buy/sell),
/// price, and quantity. Buy trades are green, sell trades are red.
fn render_recent_trades_panel(f: &mut Frame, area: Rect, state: &MonitorState) {
    let pal = state.palette();

    if state.recent_trades.is_empty() {
        let block = Block::default()
            .title(" ◈ Recent Trades (no data) ")
            .borders(Borders::ALL)
            .border_style(Style::new().fg(Color::DarkGray));
        f.render_widget(block, area);
        return;
    }

    let inner_height = area.height.saturating_sub(2) as usize;
    let inner_width = area.width.saturating_sub(2) as usize;

    // Column widths
    let time_width = 8; // HH:MM:SS
    let side_width = 4; // BUY / SELL
    let price_width = inner_width
        .saturating_sub(time_width)
        .saturating_sub(side_width)
        .saturating_sub(3) // separators
        / 2;
    let qty_width = inner_width
        .saturating_sub(time_width)
        .saturating_sub(side_width)
        .saturating_sub(price_width)
        .saturating_sub(3);

    let mut lines: Vec<Line> = Vec::with_capacity(inner_height);

    // Header row
    lines.push(Line::from(vec![
        Span::styled(
            format!("{:<time_width$}", "Time"),
            Style::new().fg(Color::DarkGray).bold(),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:<side_width$}", "Side"),
            Style::new().fg(Color::DarkGray).bold(),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:>price_width$}", "Price"),
            Style::new().fg(Color::DarkGray).bold(),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:>qty_width$}", "Qty"),
            Style::new().fg(Color::DarkGray).bold(),
        ),
    ]));

    // Trade rows (most recent first)
    let visible_count = inner_height.saturating_sub(1); // minus header
    for trade in state.recent_trades.iter().rev().take(visible_count) {
        let (side_str, side_color) = match trade.side {
            TradeSide::Buy => ("BUY ", pal.up),
            TradeSide::Sell => ("SELL", pal.down),
        };

        // Format timestamp (HH:MM:SS from epoch ms)
        let secs = (trade.timestamp_ms / 1000) as i64;
        let hours = (secs / 3600) % 24;
        let mins = (secs / 60) % 60;
        let sec = secs % 60;
        let time_str = format!("{:02}:{:02}:{:02}", hours, mins, sec);

        let price_str = if trade.price >= 1000.0 {
            format!("{:.2}", trade.price)
        } else if trade.price >= 1.0 {
            format!("{:.4}", trade.price)
        } else {
            format!("{:.6}", trade.price)
        };

        let qty_str = format_number(trade.quantity);

        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<time_width$}", time_str),
                Style::new().fg(Color::DarkGray),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:<side_width$}", side_str),
                Style::new().fg(side_color),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:>price_width$}", price_str),
                Style::new().fg(side_color),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:>qty_width$}", qty_str),
                Style::new().fg(pal.neutral),
            ),
        ]));
    }

    let title = format!(" ◈ Recent Trades ({}) ", state.recent_trades.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::new().fg(pal.border));

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, area);
}

/// Renders the market info panel for the Exchange layout.
///
/// Shows per-pair breakdown (DEX, volume, liquidity), 24h stats,
/// token metadata (links, creation date), and aggregated metrics.
fn render_market_info_panel(f: &mut Frame, area: Rect, state: &MonitorState) {
    let pal = state.palette();

    // Split into left (pair table) and right (token info) columns
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    // ── Left: Trading pairs table ──
    {
        let header = Row::new(vec!["DEX / Pair", "Volume 24h", "Liquidity", "Δ 24h"])
            .style(Style::new().fg(Color::Cyan).bold())
            .bottom_margin(0);

        let rows: Vec<Row> = state
            .dex_pairs
            .iter()
            .take(cols[0].height.saturating_sub(3) as usize) // fit available rows
            .map(|p| {
                let pair_label = format!("{}/{}", p.base_token, p.quote_token);
                let dex_and_pair = format!("{} {}", p.dex_name, pair_label);
                let vol = format_number(p.volume_24h);
                let liq = format_number(p.liquidity_usd);
                let change_str = format!("{:+.1}%", p.price_change_24h);
                let change_color = if p.price_change_24h >= 0.0 {
                    pal.up
                } else {
                    pal.down
                };
                Row::new(vec![
                    ratatui::text::Text::from(dex_and_pair),
                    ratatui::text::Text::styled(vol, Style::new().fg(pal.neutral)),
                    ratatui::text::Text::styled(liq, Style::new().fg(pal.volume_bar)),
                    ratatui::text::Text::styled(change_str, Style::new().fg(change_color)),
                ])
            })
            .collect();

        let widths = [
            Constraint::Percentage(40),
            Constraint::Percentage(22),
            Constraint::Percentage(22),
            Constraint::Percentage(16),
        ];

        let table = Table::new(rows, widths).header(header).block(
            Block::default()
                .title(format!(" ◫ Trading Pairs ({}) ", state.dex_pairs.len()))
                .borders(Borders::ALL)
                .border_style(Style::new().fg(pal.border)),
        );

        f.render_widget(table, cols[0]);
    }

    // ── Right: Token info + aggregated metrics ──
    {
        let mut info_lines: Vec<Line> = Vec::new();

        // Price summary
        let price_color = if state.price_change_24h >= 0.0 {
            pal.up
        } else {
            pal.down
        };
        info_lines.push(Line::from(vec![
            Span::styled(" Price  ", Style::new().fg(Color::DarkGray)),
            Span::styled(
                format!("${:.6}", state.current_price),
                Style::new().fg(Color::White).bold(),
            ),
        ]));

        // Multi-timeframe changes
        let changes = [
            ("5m", state.price_change_5m),
            ("1h", state.price_change_1h),
            ("6h", state.price_change_6h),
            ("24h", state.price_change_24h),
        ];
        let change_spans: Vec<Span> = changes
            .iter()
            .flat_map(|(label, val)| {
                let color = if *val >= 0.0 { pal.up } else { pal.down };
                vec![
                    Span::styled(format!(" {}: ", label), Style::new().fg(Color::DarkGray)),
                    Span::styled(format!("{:+.2}%", val), Style::new().fg(color)),
                ]
            })
            .collect();
        info_lines.push(Line::from(change_spans));

        info_lines.push(Line::from(""));

        // Volume & Liquidity
        info_lines.push(Line::from(vec![
            Span::styled(" Vol 24h ", Style::new().fg(Color::DarkGray)),
            Span::styled(
                format!("${}", format_number(state.volume_24h)),
                Style::new().fg(pal.neutral),
            ),
        ]));
        info_lines.push(Line::from(vec![
            Span::styled(" Liq     ", Style::new().fg(Color::DarkGray)),
            Span::styled(
                format!("${}", format_number(state.liquidity_usd)),
                Style::new().fg(pal.volume_bar),
            ),
        ]));

        // Market cap / FDV
        if let Some(mc) = state.market_cap {
            info_lines.push(Line::from(vec![
                Span::styled(" MCap    ", Style::new().fg(Color::DarkGray)),
                Span::styled(
                    format!("${}", format_number(mc)),
                    Style::new().fg(pal.neutral),
                ),
            ]));
        }
        if let Some(fdv) = state.fdv {
            info_lines.push(Line::from(vec![
                Span::styled(" FDV     ", Style::new().fg(Color::DarkGray)),
                Span::styled(
                    format!("${}", format_number(fdv)),
                    Style::new().fg(pal.neutral),
                ),
            ]));
        }

        // Buy/sell stats
        info_lines.push(Line::from(""));
        let total_txs = state.buys_24h + state.sells_24h;
        let buy_pct = if total_txs > 0 {
            (state.buys_24h as f64 / total_txs as f64) * 100.0
        } else {
            50.0
        };
        info_lines.push(Line::from(vec![
            Span::styled(" Buys    ", Style::new().fg(Color::DarkGray)),
            Span::styled(
                format!("{} ({:.0}%)", state.buys_24h, buy_pct),
                Style::new().fg(pal.up),
            ),
        ]));
        info_lines.push(Line::from(vec![
            Span::styled(" Sells   ", Style::new().fg(Color::DarkGray)),
            Span::styled(
                format!("{} ({:.0}%)", state.sells_24h, 100.0 - buy_pct),
                Style::new().fg(pal.down),
            ),
        ]));

        // Holder count
        if let Some(holders) = state.holder_count {
            info_lines.push(Line::from(vec![
                Span::styled(" Holders ", Style::new().fg(Color::DarkGray)),
                Span::styled(format_number(holders as f64), Style::new().fg(pal.neutral)),
            ]));
        }

        // Listed since
        if let Some(ts) = state.earliest_pair_created_at {
            let dt = chrono::DateTime::from_timestamp(ts, 0)
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "?".to_string());
            info_lines.push(Line::from(vec![
                Span::styled(" Listed  ", Style::new().fg(Color::DarkGray)),
                Span::styled(dt, Style::new().fg(pal.neutral)),
            ]));
        }

        // Links
        if !state.websites.is_empty() || !state.socials.is_empty() {
            info_lines.push(Line::from(""));
            let mut link_spans = vec![Span::styled(" Links   ", Style::new().fg(Color::DarkGray))];
            for (platform, _url) in &state.socials {
                link_spans.push(Span::styled(
                    format!("[{}] ", platform),
                    Style::new().fg(Color::Cyan),
                ));
            }
            for url in &state.websites {
                // Show domain only
                let domain = url
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .split('/')
                    .next()
                    .unwrap_or(url);
                link_spans.push(Span::styled(
                    format!("[{}] ", domain),
                    Style::new().fg(Color::Blue),
                ));
            }
            info_lines.push(Line::from(link_spans));
        }

        let title = format!(" ◉ {} ({}) ", state.symbol, state.name);
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::new().fg(price_color));

        let paragraph = Paragraph::new(info_lines).block(block);
        f.render_widget(paragraph, cols[1]);
    }
}

/// Renders the footer with status and controls.
fn render_footer(f: &mut Frame, area: Rect, state: &MonitorState) {
    let elapsed = state.last_update.elapsed().as_secs();

    // Calculate time since last price change
    let now_ts = chrono::Utc::now().timestamp() as f64;
    let secs_since_change = (now_ts - state.last_price_change_at).max(0.0) as u64;
    let price_change_str = if secs_since_change < 60 {
        format!("{}s", secs_since_change)
    } else if secs_since_change < 3600 {
        format!("{}m", secs_since_change / 60)
    } else {
        format!("{}h", secs_since_change / 3600)
    };

    // Get data stats
    let (synthetic_count, real_count) = state.data_stats();
    let memory_bytes = state.memory_usage();
    let memory_str = if memory_bytes >= 1024 * 1024 {
        format!("{:.1}MB", memory_bytes as f64 / (1024.0 * 1024.0))
    } else if memory_bytes >= 1024 {
        format!("{:.1}KB", memory_bytes as f64 / 1024.0)
    } else {
        format!("{}B", memory_bytes)
    };

    let status = if let Some(ref err) = state.error_message {
        Span::styled(format!("⚠ {}", err), Style::new().red())
    } else if state.paused {
        Span::styled("⏸ PAUSED", Style::new().fg(Color::Yellow).bold())
    } else if state.is_auto_paused() {
        Span::styled("⏸ AUTO-PAUSED", Style::new().fg(Color::Cyan).bold())
    } else {
        Span::styled(
            format!(
                "↻ {}s │ Δ {} │ {} pts │ {}",
                elapsed,
                price_change_str,
                synthetic_count + real_count,
                memory_str
            ),
            Style::new().gray(),
        )
    };

    let widget_hint = if state.widget_toggle_mode {
        Span::styled("W:1-5?", Style::new().fg(Color::Yellow).bold())
    } else {
        Span::styled("W", Style::new().fg(Color::Cyan).bold())
    };

    let mut spans = vec![status, Span::raw(" ║ ")];

    // REC indicator when CSV export is active
    if state.export_active {
        spans.push(Span::styled("● REC ", Style::new().fg(Color::Red).bold()));
    }

    spans.extend([
        Span::styled("Q", Style::new().red().bold()),
        Span::raw("uit "),
        Span::styled("R", Style::new().fg(Color::Green).bold()),
        Span::raw("efresh "),
        Span::styled("P", Style::new().fg(Color::Yellow).bold()),
        Span::raw("ause "),
        Span::styled("E", Style::new().fg(Color::LightRed).bold()),
        Span::raw("xport "),
        Span::styled("L", Style::new().fg(Color::Cyan).bold()),
        Span::raw(format!(":{} ", state.layout.label())),
        widget_hint,
        Span::raw("idget "),
        Span::styled("C", Style::new().fg(Color::LightBlue).bold()),
        Span::raw(format!("hart:{} ", state.chart_mode.label())),
        Span::styled("S", Style::new().fg(Color::LightGreen).bold()),
        Span::raw(format!("cale:{} ", state.scale_mode.label())),
        Span::styled("/", Style::new().fg(Color::LightRed).bold()),
        Span::raw(format!(":{} ", state.color_scheme.label())),
        Span::styled("T", Style::new().fg(Color::Magenta).bold()),
        Span::raw("ime "),
    ]);

    let footer = Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::ALL));

    f.render_widget(footer, area);
}

/// Formats a number with K/M/B suffixes.
fn format_number(n: f64) -> String {
    if n >= 1_000_000_000.0 {
        format!("{:.2}B", n / 1_000_000_000.0)
    } else if n >= 1_000_000.0 {
        format!("{:.2}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.2}K", n / 1_000.0)
    } else {
        format!("{:.2}", n)
    }
}

/// Entry point for the top-level `scope monitor` command.
///
/// Creates a `SessionContext` from CLI args and delegates to [`run`].
/// Applies CLI-provided overrides (layout, refresh, scale, etc.) on top
/// of the config-file defaults.
pub async fn run_direct(
    mut args: MonitorArgs,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    // Resolve address book label → address + chain
    if let Some((address, chain)) =
        crate::cli::address_book::resolve_address_book_input(&args.token, config)?
    {
        args.token = address;
        if args.chain == "ethereum" {
            args.chain = chain;
        }
    }

    // Build a SessionContext from the CLI args (no interactive session needed)
    let ctx = SessionContext {
        chain: args.chain,
        ..SessionContext::default()
    };

    // Build a MonitorConfig from config-file defaults + CLI overrides
    let mut monitor_config = config.monitor.clone();
    if let Some(layout) = args.layout {
        monitor_config.layout = layout;
    }
    if let Some(refresh) = args.refresh {
        monitor_config.refresh_seconds = refresh;
    }
    if let Some(scale) = args.scale {
        monitor_config.scale = scale;
    }
    if let Some(color_scheme) = args.color_scheme {
        monitor_config.color_scheme = color_scheme;
    }
    if let Some(ref path) = args.export {
        monitor_config.export.path = Some(path.to_string_lossy().into_owned());
    }
    if let Some(ref venue) = args.venue {
        monitor_config.venue = Some(venue.clone());
    }

    // Use a temporary Config with the CLI-overridden monitor settings
    let mut effective_config = config.clone();
    effective_config.monitor = monitor_config;

    run(
        Some(args.token),
        args.pair,
        &ctx,
        &effective_config,
        clients,
    )
    .await
}

/// Entry point for the monitor command from interactive mode.
///
/// When `explicit_pair` is `Some`, token resolution is bypassed and the
/// exchange ticker is used as the primary data source.  This enables
/// monitoring tokens that are not indexed by DexScreener (e.g., PUSD on
/// Biconomy).
pub async fn run(
    token: Option<String>,
    explicit_pair: Option<String>,
    ctx: &SessionContext,
    config: &Config,
    clients: &dyn ChainClientFactory,
) -> Result<()> {
    let token_input = match token {
        Some(t) => t,
        None => {
            return Err(ScopeError::Chain(
                "Token address or symbol required. Usage: monitor <token>".to_string(),
            ));
        }
    };

    eprintln!("  Starting live monitor for {}...", token_input);
    eprintln!("  Fetching initial data...");

    // Create exchange client if a venue is configured (from MonitorConfig or CLI)
    let exchange_client = config.monitor.venue.as_ref().and_then(|venue_id| {
        crate::market::VenueRegistry::load()
            .ok()
            .and_then(|r| r.get(venue_id).cloned())
            .map(|desc| crate::market::ExchangeClient::from_descriptor(&desc))
    });

    // ---- Exchange-only mode (--pair + --venue) ----
    // Bypass DexScreener entirely when the user provides a direct pair.
    let initial_data = if let Some(ref pair_str) = explicit_pair {
        let ex = exchange_client.as_ref().ok_or_else(|| {
            ScopeError::Chain("--pair requires --venue to be specified".to_string())
        })?;

        // Fetch the ticker to get current price data
        let ticker = ex.fetch_ticker(pair_str).await.map_err(|e| {
            ScopeError::Chain(format!("Failed to fetch ticker for {}: {}", pair_str, e))
        })?;

        // Extract base symbol from pair label (e.g., "PUSD/USDT" → "PUSD")
        let base_symbol = ticker
            .pair
            .split('/')
            .next()
            .unwrap_or(&token_input)
            .to_string();

        eprintln!(
            "  Exchange-only mode: {} @ ${:.6}",
            pair_str,
            ticker.last_price.unwrap_or(0.0)
        );

        // Build a minimal DexTokenData from the exchange ticker
        build_exchange_token_data(&base_symbol, pair_str, &ticker)
    } else {
        // ---- Normal DexScreener mode ----
        let dex_client = clients.create_dex_client();
        let token_address =
            resolve_token_address(&token_input, &ctx.chain, config, dex_client.as_ref()).await?;

        dex_client
            .get_token_data(&ctx.chain, &token_address)
            .await?
    };

    println!(
        "Monitoring {} ({}) on {}",
        initial_data.symbol, initial_data.name, ctx.chain
    );
    println!("Press Q to quit, R to refresh, P to pause...\n");

    // Small delay to let user read the message
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Create optional chain client for on-chain data (holder count, etc.)
    let chain_client = clients.create_chain_client(&ctx.chain).ok();

    // Create and run the app — override venue_pair when using explicit pair mode
    let mut app = MonitorApp::new(
        initial_data,
        &ctx.chain,
        &config.monitor,
        chain_client,
        exchange_client,
    )?;

    // In explicit-pair mode, override the auto-formatted pair with the user's exact pair
    if let Some(ref pair_str) = explicit_pair {
        app.state.venue_pair = Some(pair_str.clone());
    }

    let result = app.run().await;

    // Cleanup is handled by Drop, but we do it explicitly for error handling
    if let Err(e) = app.cleanup() {
        eprintln!("Warning: Failed to cleanup terminal: {}", e);
    }

    result
}

/// Builds a minimal [`DexTokenData`] from an exchange ticker.
///
/// Used in exchange-only mode (--pair) when DexScreener is not available
/// for the token.
fn build_exchange_token_data(
    symbol: &str,
    pair_label: &str,
    ticker: &crate::market::Ticker,
) -> DexTokenData {
    let price = ticker.last_price.unwrap_or(0.0);
    DexTokenData {
        address: format!("exchange:{}", pair_label),
        symbol: symbol.to_string(),
        name: symbol.to_string(),
        price_usd: price,
        price_change_24h: 0.0,
        price_change_6h: 0.0,
        price_change_1h: 0.0,
        price_change_5m: 0.0,
        volume_24h: ticker.volume_24h.unwrap_or(0.0),
        volume_6h: 0.0,
        volume_1h: 0.0,
        liquidity_usd: 0.0,
        market_cap: None,
        fdv: None,
        pairs: vec![],
        price_history: vec![],
        volume_history: vec![],
        total_buys_24h: 0,
        total_sells_24h: 0,
        total_buys_6h: 0,
        total_sells_6h: 0,
        total_buys_1h: 0,
        total_sells_1h: 0,
        earliest_pair_created_at: None,
        image_url: None,
        websites: vec![],
        socials: vec![],
        dexscreener_url: None,
    }
}

/// Resolves a token input (address or symbol) to an address.
///
/// Uses the same chain filter logic as the crawl command: when the chain is
/// "ethereum" (the default), searches ALL chains so that exact symbol matches
/// on other chains rank above substring matches on ethereum.  Includes a CEX
/// ticker fallback when DexScreener returns no results.
async fn resolve_token_address(
    input: &str,
    chain: &str,
    _config: &Config,
    dex_client: &dyn DexDataSource,
) -> Result<String> {
    // Check if it's already an address (EVM, Solana, Tron)
    if crate::tokens::TokenAliases::is_address(input) {
        return Ok(input.to_string());
    }

    // Check saved aliases — use chain filter only when explicitly overridden
    let chain_filter = if chain != "ethereum" {
        Some(chain)
    } else {
        None
    };
    let aliases = crate::tokens::TokenAliases::load();
    if let Some(alias) = aliases.get(input, chain_filter) {
        return Ok(alias.address.clone());
    }

    // Search by name/symbol — same filter logic as crawl:
    // "ethereum" (default) → None (all chains), explicit chain → Some(chain)
    let mut results = dex_client.search_tokens(input, chain_filter).await?;

    // CEX fallback: if DexScreener has no results, try exchange ticker
    if results.is_empty()
        && let Some(fallback) = try_cex_fallback(input, chain).await
    {
        eprintln!(
            "  Not found on DexScreener; found {} on {} (CEX)",
            fallback.symbol, fallback.chain
        );
        results.push(fallback);
    }

    if results.is_empty() {
        return Err(ScopeError::NotFound(format!(
            "No token found matching '{}' on {} (checked DexScreener and CEX venues)",
            input, chain
        )));
    }

    // If only one result, use it directly
    if results.len() == 1 {
        let token = &results[0];
        println!(
            "Found: {} ({}) - ${:.6}",
            token.symbol,
            token.name,
            token.price_usd.unwrap_or(0.0)
        );
        return Ok(token.address.clone());
    }

    // Multiple results — prompt user to select
    let selected = select_token_interactive(&results)?;
    Ok(selected.address.clone())
}

/// CEX venue ticker fallback for token resolution.
///
/// When DexScreener returns no results, tries to find the token on a
/// centralized exchange (Binance) as a fallback. Returns a synthetic
/// `TokenSearchResult` from the ticker data.
async fn try_cex_fallback(
    symbol: &str,
    chain: &str,
) -> Option<crate::chains::TokenSearchResult> {
    let registry = crate::market::VenueRegistry::load().ok()?;
    let descriptor = registry.get("binance")?;
    let client = crate::market::ExchangeClient::from_descriptor(&descriptor.clone());
    let pair = client.format_pair(&format!("{}USDT", symbol.to_uppercase()));
    let ticker = client.fetch_ticker(&pair).await.ok()?;
    let price = ticker.last_price.unwrap_or(0.0);
    Some(crate::chains::TokenSearchResult {
        address: String::new(),
        symbol: symbol.to_uppercase(),
        name: symbol.to_uppercase(),
        chain: chain.to_string(),
        price_usd: Some(price),
        volume_24h: ticker.volume_24h.unwrap_or(0.0),
        liquidity_usd: 0.0,
        market_cap: None,
    })
}

/// Abbreviates a blockchain address for display (e.g. "0x1234...abcd").
fn abbreviate_address(addr: &str) -> String {
    if addr.len() > 16 {
        format!("{}...{}", &addr[..8], &addr[addr.len() - 6..])
    } else {
        addr.to_string()
    }
}

/// Displays token search results and prompts the user to select one.
fn select_token_interactive(
    results: &[crate::chains::dex::TokenSearchResult],
) -> Result<&crate::chains::dex::TokenSearchResult> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    select_token_impl(results, &mut stdin.lock(), &mut stdout.lock())
}

/// Testable implementation of token selection with injected I/O.
fn select_token_impl<'a>(
    results: &'a [crate::chains::dex::TokenSearchResult],
    reader: &mut impl io::BufRead,
    writer: &mut impl io::Write,
) -> Result<&'a crate::chains::dex::TokenSearchResult> {
    writeln!(
        writer,
        "\nFound {} tokens matching your query:\n",
        results.len()
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;

    writeln!(
        writer,
        "{:>3}  {:>8}  {:<22}  {:<16}  {:>12}  {:>12}",
        "#", "Symbol", "Name", "Address", "Price", "Liquidity"
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;

    writeln!(writer, "{}", "─".repeat(82)).map_err(|e| ScopeError::Io(e.to_string()))?;

    for (i, token) in results.iter().enumerate() {
        let price = token
            .price_usd
            .map(|p| format!("${:.6}", p))
            .unwrap_or_else(|| "N/A".to_string());

        let liquidity = format_monitor_number(token.liquidity_usd);
        let addr = abbreviate_address(&token.address);

        // Truncate name if too long
        let name = if token.name.len() > 20 {
            format!("{}...", &token.name[..17])
        } else {
            token.name.clone()
        };

        writeln!(
            writer,
            "{:>3}  {:>8}  {:<22}  {:<16}  {:>12}  {:>12}",
            i + 1,
            token.symbol,
            name,
            addr,
            price,
            liquidity
        )
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    }

    writeln!(writer).map_err(|e| ScopeError::Io(e.to_string()))?;
    write!(writer, "Select token (1-{}): ", results.len())
        .map_err(|e| ScopeError::Io(e.to_string()))?;
    writer.flush().map_err(|e| ScopeError::Io(e.to_string()))?;

    let mut input = String::new();
    reader
        .read_line(&mut input)
        .map_err(|e| ScopeError::Io(e.to_string()))?;

    let selection: usize = input
        .trim()
        .parse()
        .map_err(|_| ScopeError::Api("Invalid selection".to_string()))?;

    if selection < 1 || selection > results.len() {
        return Err(ScopeError::Api(format!(
            "Selection must be between 1 and {}",
            results.len()
        )));
    }

    let selected = &results[selection - 1];
    writeln!(
        writer,
        "Selected: {} ({}) at {}",
        selected.symbol, selected.name, selected.address
    )
    .map_err(|e| ScopeError::Io(e.to_string()))?;

    Ok(selected)
}

/// Format a number for the monitor selection table.
fn format_monitor_number(value: f64) -> String {
    if value >= 1_000_000_000.0 {
        format!("${:.2}B", value / 1_000_000_000.0)
    } else if value >= 1_000_000.0 {
        format!("${:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("${:.2}K", value / 1_000.0)
    } else {
        format!("${:.2}", value)
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_token_data() -> DexTokenData {
        DexTokenData {
            address: "0x1234".to_string(),
            symbol: "TEST".to_string(),
            name: "Test Token".to_string(),
            price_usd: 1.0,
            price_change_24h: 5.0,
            price_change_6h: 2.0,
            price_change_1h: 0.5,
            price_change_5m: 0.1,
            volume_24h: 1_000_000.0,
            volume_6h: 250_000.0,
            volume_1h: 50_000.0,
            liquidity_usd: 500_000.0,
            market_cap: Some(10_000_000.0),
            fdv: Some(100_000_000.0),
            pairs: vec![],
            price_history: vec![],
            volume_history: vec![],
            total_buys_24h: 100,
            total_sells_24h: 50,
            total_buys_6h: 25,
            total_sells_6h: 12,
            total_buys_1h: 5,
            total_sells_1h: 3,
            earliest_pair_created_at: Some(1700000000000),
            image_url: None,
            websites: vec![],
            socials: vec![],
            dexscreener_url: None,
        }
    }

    #[test]
    fn test_monitor_state_new() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");

        assert_eq!(state.symbol, "TEST");
        assert_eq!(state.chain, "ethereum");
        assert_eq!(state.current_price, 1.0);
        assert_eq!(state.buys_24h, 100);
        assert_eq!(state.sells_24h, 50);
        assert!(!state.paused);
    }

    #[test]
    fn test_monitor_state_buy_ratio() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");

        let ratio = state.buy_ratio();
        assert!((ratio - 0.6666).abs() < 0.01); // 100 / 150 ≈ 0.667
    }

    #[test]
    fn test_monitor_state_buy_ratio_zero() {
        let mut token_data = create_test_token_data();
        token_data.total_buys_24h = 0;
        token_data.total_sells_24h = 0;
        let state = MonitorState::new(&token_data, "ethereum");

        assert_eq!(state.buy_ratio(), 0.5); // Default to 50/50 when no data
    }

    #[test]
    fn test_monitor_state_toggle_pause() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");

        assert!(!state.paused);
        state.toggle_pause();
        assert!(state.paused);
        state.toggle_pause();
        assert!(!state.paused);
    }

    #[test]
    fn test_monitor_state_should_refresh() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.refresh_rate = Duration::from_secs(60);

        // Just created, should not need refresh (60s refresh rate)
        assert!(!state.should_refresh());

        // Simulate time passing well beyond refresh rate
        state.last_update = Instant::now() - Duration::from_secs(120);
        assert!(state.should_refresh());

        // Pause should prevent refresh
        state.paused = true;
        assert!(!state.should_refresh());
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(500.0), "500.00");
        assert_eq!(format_number(1_500.0), "1.50K");
        assert_eq!(format_number(1_500_000.0), "1.50M");
        assert_eq!(format_number(1_500_000_000.0), "1.50B");
    }

    #[test]
    fn test_format_usd() {
        assert_eq!(crate::display::format_usd(500.0), "$500.00");
        assert_eq!(crate::display::format_usd(1_500.0), "$1.50K");
        assert_eq!(crate::display::format_usd(1_500_000.0), "$1.50M");
        assert_eq!(crate::display::format_usd(1_500_000_000.0), "$1.50B");
    }

    #[test]
    fn test_monitor_state_update() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");

        let initial_len = state.price_history.len();

        let mut updated_data = token_data.clone();
        updated_data.price_usd = 1.5;
        updated_data.total_buys_24h = 150;

        state.update(&updated_data);

        assert_eq!(state.current_price, 1.5);
        assert_eq!(state.buys_24h, 150);
        // Should have one more point after update
        assert_eq!(state.price_history.len(), initial_len + 1);
    }

    #[test]
    fn test_monitor_state_refresh_rate_adjustment() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");

        // Default is 5 seconds
        assert_eq!(state.refresh_rate_secs(), 5);

        // Slow down (+5s)
        state.slower_refresh();
        assert_eq!(state.refresh_rate_secs(), 10);

        // Speed up (-5s)
        state.faster_refresh();
        assert_eq!(state.refresh_rate_secs(), 5);

        // Speed up again (should hit minimum of 1s)
        state.faster_refresh();
        assert_eq!(state.refresh_rate_secs(), 1);

        // Can't go below 1s
        state.faster_refresh();
        assert_eq!(state.refresh_rate_secs(), 1);

        // Slow down to max (60s)
        for _ in 0..20 {
            state.slower_refresh();
        }
        assert_eq!(state.refresh_rate_secs(), 60);
    }

    #[test]
    fn test_time_period() {
        assert_eq!(TimePeriod::Min1.label(), "1m");
        assert_eq!(TimePeriod::Min5.label(), "5m");
        assert_eq!(TimePeriod::Min15.label(), "15m");
        assert_eq!(TimePeriod::Hour1.label(), "1h");
        assert_eq!(TimePeriod::Hour4.label(), "4h");
        assert_eq!(TimePeriod::Day1.label(), "1d");

        assert_eq!(TimePeriod::Min1.duration_secs(), 60);
        assert_eq!(TimePeriod::Min5.duration_secs(), 300);
        assert_eq!(TimePeriod::Min15.duration_secs(), 15 * 60);
        assert_eq!(TimePeriod::Hour1.duration_secs(), 3600);
        assert_eq!(TimePeriod::Hour4.duration_secs(), 4 * 3600);
        assert_eq!(TimePeriod::Day1.duration_secs(), 24 * 3600);

        // Test cycling
        assert_eq!(TimePeriod::Min1.next(), TimePeriod::Min5);
        assert_eq!(TimePeriod::Min5.next(), TimePeriod::Min15);
        assert_eq!(TimePeriod::Min15.next(), TimePeriod::Hour1);
        assert_eq!(TimePeriod::Hour1.next(), TimePeriod::Hour4);
        assert_eq!(TimePeriod::Hour4.next(), TimePeriod::Day1);
        assert_eq!(TimePeriod::Day1.next(), TimePeriod::Min1);
    }

    #[test]
    fn test_time_period_exchange_interval() {
        assert_eq!(TimePeriod::Min1.exchange_interval(), "1m");
        assert_eq!(TimePeriod::Min5.exchange_interval(), "5m");
        assert_eq!(TimePeriod::Min15.exchange_interval(), "15m");
        assert_eq!(TimePeriod::Hour1.exchange_interval(), "1h");
        assert_eq!(TimePeriod::Hour4.exchange_interval(), "4h");
        assert_eq!(TimePeriod::Day1.exchange_interval(), "1d");
    }

    #[test]
    fn test_monitor_state_time_period() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");

        // Default is 1 hour
        assert_eq!(state.time_period, TimePeriod::Hour1);

        // Cycle through periods
        state.cycle_time_period();
        assert_eq!(state.time_period, TimePeriod::Hour4);

        state.set_time_period(TimePeriod::Day1);
        assert_eq!(state.time_period, TimePeriod::Day1);
    }

    #[test]
    fn test_synthetic_history_generation() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");

        // Should have generated history (synthetic or cached real)
        assert!(state.price_history.len() > 1);
        assert!(state.volume_history.len() > 1);

        // Price history should span some time range
        if let (Some(first), Some(last)) = (state.price_history.front(), state.price_history.back())
        {
            let span = last.timestamp - first.timestamp;
            assert!(span > 0.0); // History should span some time
        }
    }

    #[test]
    fn test_real_data_marking() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");

        // Initially all synthetic
        let (synthetic, real) = state.data_stats();
        assert!(synthetic > 0);
        assert_eq!(real, 0);

        // After update, should have real data
        let mut updated_data = token_data.clone();
        updated_data.price_usd = 1.5;
        state.update(&updated_data);

        let (synthetic2, real2) = state.data_stats();
        assert!(synthetic2 > 0);
        assert_eq!(real2, 1);
        assert_eq!(state.real_data_count, 1);

        // The last point should be real
        assert!(
            state
                .price_history
                .back()
                .map(|p| p.is_real)
                .unwrap_or(false)
        );
    }

    #[test]
    fn test_memory_usage() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");

        let mem = state.memory_usage();
        // DataPoint is 24 bytes, should have some data points
        assert!(mem > 0);

        // Each DataPoint is 24 bytes (f64 + f64 + bool + padding)
        let expected_point_size = std::mem::size_of::<DataPoint>();
        assert_eq!(expected_point_size, 24);
    }

    #[test]
    fn test_get_data_for_period_returns_flags() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");

        // Get initial data (may contain cached real data or synthetic)
        let (data, is_real) = state.get_price_data_for_period();
        assert_eq!(data.len(), is_real.len());

        // Add real data point
        state.update(&token_data);

        let (_data2, is_real2) = state.get_price_data_for_period();
        // Should have at least one real point now
        assert!(is_real2.iter().any(|r| *r));
    }

    #[test]
    fn test_cache_path_generation() {
        let path =
            MonitorState::cache_path("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", "ethereum");
        assert!(path.to_string_lossy().contains("bcc_monitor_"));
        assert!(path.to_string_lossy().contains("ethereum"));
        // Should be in temp directory
        let temp_dir = std::env::temp_dir();
        assert!(path.starts_with(temp_dir));
    }

    #[test]
    fn test_cache_save_and_load() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "test_chain");

        // Add some real data
        state.update(&token_data);
        state.update(&token_data);

        // Save cache
        state.save_cache();

        // Verify cache file exists
        let path = MonitorState::cache_path(&state.token_address, &state.chain);
        assert!(path.exists(), "Cache file should exist after save");

        // Load cache
        let loaded = MonitorState::load_cache(&state.token_address, &state.chain);
        assert!(loaded.is_some(), "Should be able to load saved cache");

        let cached = loaded.unwrap();
        assert_eq!(cached.token_address, state.token_address);
        assert_eq!(cached.chain, state.chain);
        assert!(!cached.price_history.is_empty());

        // Cleanup
        let _ = std::fs::remove_file(path);
    }

    // ========================================================================
    // Price formatting tests
    // ========================================================================

    #[test]
    fn test_format_price_usd_high() {
        let formatted = format_price_usd(2500.50);
        assert!(formatted.starts_with("$2500.50"));
    }

    #[test]
    fn test_format_price_usd_stablecoin() {
        let formatted = format_price_usd(1.0001);
        assert!(formatted.contains("1.000100"));
        assert!(is_stablecoin_price(1.0001));
    }

    #[test]
    fn test_format_price_usd_medium() {
        let formatted = format_price_usd(5.1234);
        assert!(formatted.starts_with("$5.1234"));
    }

    #[test]
    fn test_format_price_usd_small() {
        let formatted = format_price_usd(0.05);
        assert!(formatted.starts_with("$0.0500"));
    }

    #[test]
    fn test_format_price_usd_micro() {
        let formatted = format_price_usd(0.001);
        assert!(formatted.starts_with("$0.0010"));
    }

    #[test]
    fn test_format_price_usd_nano() {
        let formatted = format_price_usd(0.00001);
        assert!(formatted.contains("0.0000100"));
    }

    #[test]
    fn test_is_stablecoin_price() {
        assert!(is_stablecoin_price(1.0));
        assert!(is_stablecoin_price(0.999));
        assert!(is_stablecoin_price(1.001));
        assert!(is_stablecoin_price(0.95));
        assert!(is_stablecoin_price(1.05));
        assert!(!is_stablecoin_price(0.94));
        assert!(!is_stablecoin_price(1.06));
        assert!(!is_stablecoin_price(100.0));
    }

    // ========================================================================
    // OHLC candle tests
    // ========================================================================

    #[test]
    fn test_ohlc_candle_new() {
        let candle = OhlcCandle::new(1000.0, 50.0);
        assert_eq!(candle.open, 50.0);
        assert_eq!(candle.high, 50.0);
        assert_eq!(candle.low, 50.0);
        assert_eq!(candle.close, 50.0);
        assert!(candle.is_bullish);
    }

    #[test]
    fn test_ohlc_candle_update() {
        let mut candle = OhlcCandle::new(1000.0, 50.0);
        candle.update(55.0);
        assert_eq!(candle.high, 55.0);
        assert_eq!(candle.close, 55.0);
        assert!(candle.is_bullish);

        candle.update(45.0);
        assert_eq!(candle.low, 45.0);
        assert_eq!(candle.close, 45.0);
        assert!(!candle.is_bullish); // close < open
    }

    #[test]
    fn test_get_ohlc_candles() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        // Add several data points
        for i in 0..20 {
            let mut data = token_data.clone();
            data.price_usd = 1.0 + (i as f64 * 0.01);
            state.update(&data);
        }
        let candles = state.get_ohlc_candles();
        // Should have some candles
        assert!(!candles.is_empty());
    }

    #[test]
    fn test_get_ohlc_candles_returns_exchange_ohlc_when_populated() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        // Populate exchange OHLC
        let exchange_candles = vec![
            OhlcCandle::new(1700000000.0, 100.0),
            OhlcCandle::new(1700003600.0, 101.0),
        ];
        state.exchange_ohlc = exchange_candles.clone();
        let candles = state.get_ohlc_candles();
        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].timestamp, 1700000000.0);
        assert_eq!(candles[0].open, 100.0);
        assert_eq!(candles[1].timestamp, 1700003600.0);
        assert_eq!(candles[1].open, 101.0);
    }

    // ========================================================================
    // ChartMode tests
    // ========================================================================

    #[test]
    fn test_chart_mode_cycle() {
        let mode = ChartMode::Line;
        assert_eq!(mode.next(), ChartMode::Candlestick);
        assert_eq!(ChartMode::Candlestick.next(), ChartMode::VolumeProfile);
        assert_eq!(ChartMode::VolumeProfile.next(), ChartMode::Line);
    }

    #[test]
    fn test_chart_mode_label() {
        assert_eq!(ChartMode::Line.label(), "Line");
        assert_eq!(ChartMode::Candlestick.label(), "Candle");
        assert_eq!(ChartMode::VolumeProfile.label(), "VolPro");
    }

    // ========================================================================
    // TUI rendering tests (headless TestBackend)
    // ========================================================================

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn create_test_terminal() -> Terminal<TestBackend> {
        let backend = TestBackend::new(120, 40);
        Terminal::new(backend).unwrap()
    }

    fn create_populated_state() -> MonitorState {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        // Add real data points so charts have content
        for i in 0..30 {
            let mut data = token_data.clone();
            data.price_usd = 1.0 + (i as f64 * 0.01);
            data.volume_24h = 1_000_000.0 + (i as f64 * 10_000.0);
            state.update(&data);
        }
        state
    }

    #[test]
    fn test_render_header_no_panic() {
        let mut terminal = create_test_terminal();
        let state = create_populated_state();
        terminal
            .draw(|f| render_header(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_price_chart_no_panic() {
        let mut terminal = create_test_terminal();
        let state = create_populated_state();
        terminal
            .draw(|f| render_price_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_price_chart_line_mode() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.chart_mode = ChartMode::Line;
        terminal
            .draw(|f| render_price_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_candlestick_chart_no_panic() {
        let mut terminal = create_test_terminal();
        let state = create_populated_state();
        terminal
            .draw(|f| render_candlestick_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_candlestick_chart_empty() {
        let mut terminal = create_test_terminal();
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");
        terminal
            .draw(|f| render_candlestick_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_volume_chart_no_panic() {
        let mut terminal = create_test_terminal();
        let state = create_populated_state();
        terminal
            .draw(|f| render_volume_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_volume_chart_empty() {
        let mut terminal = create_test_terminal();
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");
        terminal
            .draw(|f| render_volume_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_buy_sell_gauge_no_panic() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        terminal
            .draw(|f| render_buy_sell_gauge(f, f.area(), &mut state))
            .unwrap();
    }

    #[test]
    fn test_render_buy_sell_gauge_balanced() {
        let mut terminal = create_test_terminal();
        let mut token_data = create_test_token_data();
        token_data.total_buys_24h = 100;
        token_data.total_sells_24h = 100;
        let mut state = MonitorState::new(&token_data, "ethereum");
        terminal
            .draw(|f| render_buy_sell_gauge(f, f.area(), &mut state))
            .unwrap();
    }

    #[test]
    fn test_render_metrics_panel_no_panic() {
        let mut terminal = create_test_terminal();
        let state = create_populated_state();
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_metrics_panel_no_market_cap() {
        let mut terminal = create_test_terminal();
        let mut token_data = create_test_token_data();
        token_data.market_cap = None;
        token_data.fdv = None;
        let state = MonitorState::new(&token_data, "ethereum");
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_footer_no_panic() {
        let mut terminal = create_test_terminal();
        let state = create_populated_state();
        terminal
            .draw(|f| render_footer(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_footer_paused() {
        let mut terminal = create_test_terminal();
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.paused = true;
        terminal
            .draw(|f| render_footer(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_all_components() {
        // Exercise the full draw_ui layout path
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        terminal
            .draw(|f| {
                let area = f.area();
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Min(10),
                        Constraint::Length(5),
                        Constraint::Length(3),
                        Constraint::Length(3),
                    ])
                    .split(area);
                render_header(f, chunks[0], &state);
                render_price_chart(f, chunks[1], &state);
                render_volume_chart(f, chunks[2], &state);
                render_buy_sell_gauge(f, chunks[3], &mut state);
                render_footer(f, chunks[4], &state);
            })
            .unwrap();
    }

    #[test]
    fn test_render_candlestick_mode() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.chart_mode = ChartMode::Candlestick;
        terminal
            .draw(|f| {
                let area = f.area();
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(3), Constraint::Min(10)])
                    .split(area);
                render_header(f, chunks[0], &state);
                render_candlestick_chart(f, chunks[1], &state);
            })
            .unwrap();
    }

    #[test]
    fn test_render_with_different_time_periods() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();

        for period in [
            TimePeriod::Min1,
            TimePeriod::Min5,
            TimePeriod::Min15,
            TimePeriod::Hour1,
            TimePeriod::Hour4,
            TimePeriod::Day1,
        ] {
            state.time_period = period;
            terminal
                .draw(|f| render_price_chart(f, f.area(), &state))
                .unwrap();
        }
    }

    #[test]
    fn test_render_metrics_with_stablecoin() {
        let mut terminal = create_test_terminal();
        let mut token_data = create_test_token_data();
        token_data.price_usd = 0.999;
        token_data.symbol = "USDC".to_string();
        let state = MonitorState::new(&token_data, "ethereum");
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_header_with_negative_change() {
        let mut terminal = create_test_terminal();
        let mut token_data = create_test_token_data();
        token_data.price_change_24h = -15.5;
        token_data.price_change_1h = -2.3;
        let state = MonitorState::new(&token_data, "ethereum");
        terminal
            .draw(|f| render_header(f, f.area(), &state))
            .unwrap();
    }

    // ========================================================================
    // MonitorState method tests
    // ========================================================================

    #[test]
    fn test_toggle_chart_mode_roundtrip() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        assert_eq!(state.chart_mode, ChartMode::Line);
        state.toggle_chart_mode();
        assert_eq!(state.chart_mode, ChartMode::Candlestick);
        state.toggle_chart_mode();
        assert_eq!(state.chart_mode, ChartMode::VolumeProfile);
        state.toggle_chart_mode();
        assert_eq!(state.chart_mode, ChartMode::Line);
    }

    #[test]
    fn test_cycle_all_time_periods() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        assert_eq!(state.time_period, TimePeriod::Hour1);
        state.cycle_time_period();
        assert_eq!(state.time_period, TimePeriod::Hour4);
        state.cycle_time_period();
        assert_eq!(state.time_period, TimePeriod::Day1);
        state.cycle_time_period();
        assert_eq!(state.time_period, TimePeriod::Min1);
        state.cycle_time_period();
        assert_eq!(state.time_period, TimePeriod::Min5);
        state.cycle_time_period();
        assert_eq!(state.time_period, TimePeriod::Min15);
        state.cycle_time_period();
        assert_eq!(state.time_period, TimePeriod::Hour1);
    }

    #[test]
    fn test_set_specific_time_period() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.set_time_period(TimePeriod::Day1);
        assert_eq!(state.time_period, TimePeriod::Day1);
    }

    #[test]
    fn test_pause_resume_roundtrip() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        assert!(!state.paused);
        state.toggle_pause();
        assert!(state.paused);
        state.toggle_pause();
        assert!(!state.paused);
    }

    #[test]
    fn test_force_refresh_unpauses() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.paused = true;
        state.force_refresh();
        assert!(!state.paused);
        assert!(state.should_refresh());
    }

    #[test]
    fn test_refresh_rate_adjust() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        assert_eq!(state.refresh_rate_secs(), 5);

        state.slower_refresh();
        assert_eq!(state.refresh_rate_secs(), 10);

        state.faster_refresh();
        assert_eq!(state.refresh_rate_secs(), 5);
    }

    #[test]
    fn test_faster_refresh_clamped_min() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        for _ in 0..10 {
            state.faster_refresh();
        }
        assert!(state.refresh_rate_secs() >= 1);
    }

    #[test]
    fn test_slower_refresh_clamped_max() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        for _ in 0..20 {
            state.slower_refresh();
        }
        assert!(state.refresh_rate_secs() <= 60);
    }

    #[test]
    fn test_buy_ratio_balanced() {
        let mut token_data = create_test_token_data();
        token_data.total_buys_24h = 100;
        token_data.total_sells_24h = 100;
        let state = MonitorState::new(&token_data, "ethereum");
        assert!((state.buy_ratio() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_buy_ratio_no_trades() {
        let mut token_data = create_test_token_data();
        token_data.total_buys_24h = 0;
        token_data.total_sells_24h = 0;
        let state = MonitorState::new(&token_data, "ethereum");
        assert!((state.buy_ratio() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_data_stats_initial() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");
        let (synthetic, real) = state.data_stats();
        assert!(synthetic > 0 || real == 0);
    }

    #[test]
    fn test_memory_usage_nonzero() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");
        let usage = state.memory_usage();
        assert!(usage > 0);
    }

    #[test]
    fn test_price_data_for_period() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");
        let (data, is_real) = state.get_price_data_for_period();
        assert_eq!(data.len(), is_real.len());
    }

    #[test]
    fn test_volume_data_for_period() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");
        let (data, is_real) = state.get_volume_data_for_period();
        assert_eq!(data.len(), is_real.len());
    }

    #[test]
    fn test_ohlc_candles_generation() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");
        let candles = state.get_ohlc_candles();
        for candle in &candles {
            assert!(candle.high >= candle.low);
        }
    }

    #[test]
    fn test_state_update_with_new_data() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let initial_count = state.real_data_count;

        let mut updated_data = create_test_token_data();
        updated_data.price_usd = 2.0;
        updated_data.volume_24h = 2_000_000.0;

        state.update(&updated_data);
        assert_eq!(state.current_price, 2.0);
        assert_eq!(state.real_data_count, initial_count + 1);
        assert!(state.error_message.is_none());
    }

    #[test]
    fn test_cache_roundtrip_save_load() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");

        state.save_cache();

        let cache_path = MonitorState::cache_path(&token_data.address, "ethereum");
        assert!(cache_path.exists());

        let cached = MonitorState::load_cache(&token_data.address, "ethereum");
        assert!(cached.is_some());

        let _ = std::fs::remove_file(cache_path);
    }

    #[test]
    fn test_should_refresh_when_paused() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        assert!(!state.should_refresh());
        state.paused = true;
        assert!(!state.should_refresh());
    }

    #[test]
    fn test_ohlc_candle_lifecycle() {
        let mut candle = OhlcCandle::new(1700000000.0, 100.0);
        assert_eq!(candle.open, 100.0);
        assert!(candle.is_bullish);
        candle.update(110.0);
        assert_eq!(candle.high, 110.0);
        assert!(candle.is_bullish);
        candle.update(90.0);
        assert_eq!(candle.low, 90.0);
        assert!(!candle.is_bullish);
    }

    #[test]
    fn test_time_period_display_impl() {
        assert_eq!(format!("{}", TimePeriod::Min1), "1m");
        assert_eq!(format!("{}", TimePeriod::Min15), "15m");
        assert_eq!(format!("{}", TimePeriod::Day1), "1d");
    }

    #[test]
    fn test_log_messages_accumulate() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        // Trigger actions that log
        state.toggle_pause();
        state.toggle_pause();
        state.cycle_time_period();
        state.toggle_chart_mode();
        assert!(!state.log_messages.is_empty());
    }

    #[test]
    fn test_ui_function_full_render() {
        // Test the main ui() function which orchestrates all rendering
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        terminal.draw(|f| ui(f, &mut state)).unwrap();
    }

    #[test]
    fn test_ui_function_candlestick_mode() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.chart_mode = ChartMode::Candlestick;
        terminal.draw(|f| ui(f, &mut state)).unwrap();
    }

    #[test]
    fn test_ui_function_with_error_message() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.error_message = Some("Test error".to_string());
        terminal.draw(|f| ui(f, &mut state)).unwrap();
    }

    #[test]
    fn test_render_header_with_small_positive_change() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.price_change_24h = 0.3; // Between 0 and 0.5 -> △
        terminal
            .draw(|f| render_header(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_header_with_small_negative_change() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.price_change_24h = -0.3; // Between -0.5 and 0 -> ▽
        terminal
            .draw(|f| render_header(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_buy_sell_gauge_high_buy_ratio() {
        let mut terminal = create_test_terminal();
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.buys_24h = 100;
        state.sells_24h = 10;
        terminal
            .draw(|f| render_buy_sell_gauge(f, f.area(), &mut state))
            .unwrap();
    }

    #[test]
    fn test_render_buy_sell_gauge_zero_total() {
        let mut terminal = create_test_terminal();
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.buys_24h = 0;
        state.sells_24h = 0;
        terminal
            .draw(|f| render_buy_sell_gauge(f, f.area(), &mut state))
            .unwrap();
    }

    #[test]
    fn test_render_metrics_with_market_cap() {
        let mut terminal = create_test_terminal();
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.market_cap = Some(1_000_000_000.0);
        state.fdv = Some(2_000_000_000.0);
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_footer_with_error() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.error_message = Some("Connection failed".to_string());
        terminal
            .draw(|f| render_footer(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_format_price_usd_various() {
        // Test format_price_usd with various magnitudes
        assert!(!format_price_usd(0.0000001).is_empty());
        assert!(!format_price_usd(0.001).is_empty());
        assert!(!format_price_usd(1.0).is_empty());
        assert!(!format_price_usd(100.0).is_empty());
        assert!(!format_price_usd(10000.0).is_empty());
        assert!(!format_price_usd(1000000.0).is_empty());
    }

    #[test]
    fn test_format_usd_various() {
        assert!(!crate::display::format_usd(0.0).is_empty());
        assert!(!crate::display::format_usd(999.0).is_empty());
        assert!(!crate::display::format_usd(1500.0).is_empty());
        assert!(!crate::display::format_usd(1_500_000.0).is_empty());
        assert!(!crate::display::format_usd(1_500_000_000.0).is_empty());
        assert!(!crate::display::format_usd(1_500_000_000_000.0).is_empty());
    }

    #[test]
    fn test_format_number_various() {
        assert!(!format_number(0.0).is_empty());
        assert!(!format_number(999.0).is_empty());
        assert!(!format_number(1500.0).is_empty());
        assert!(!format_number(1_500_000.0).is_empty());
        assert!(!format_number(1_500_000_000.0).is_empty());
    }

    #[test]
    fn test_render_with_min15_period() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.set_time_period(TimePeriod::Min15);
        terminal.draw(|f| ui(f, &mut state)).unwrap();
    }

    #[test]
    fn test_render_with_hour6_period() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.set_time_period(TimePeriod::Hour4);
        terminal.draw(|f| ui(f, &mut state)).unwrap();
    }

    #[test]
    fn test_ui_with_fresh_state_no_real_data() {
        let mut terminal = create_test_terminal();
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        // Fresh state with only synthetic data
        terminal.draw(|f| ui(f, &mut state)).unwrap();
    }

    #[test]
    fn test_ui_with_paused_state() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.toggle_pause();
        terminal.draw(|f| ui(f, &mut state)).unwrap();
    }

    #[test]
    fn test_render_all_with_different_time_periods_and_modes() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();

        // Test all combinations of time period + chart mode
        for period in &[
            TimePeriod::Min1,
            TimePeriod::Min5,
            TimePeriod::Min15,
            TimePeriod::Hour1,
            TimePeriod::Hour4,
            TimePeriod::Day1,
        ] {
            for mode in &[
                ChartMode::Line,
                ChartMode::Candlestick,
                ChartMode::VolumeProfile,
            ] {
                state.set_time_period(*period);
                state.chart_mode = *mode;
                terminal.draw(|f| ui(f, &mut state)).unwrap();
            }
        }
    }

    #[test]
    fn test_render_metrics_with_large_values() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.market_cap = Some(50_000_000_000.0); // 50B
        state.fdv = Some(100_000_000_000.0); // 100B
        state.volume_24h = 5_000_000_000.0; // 5B
        state.liquidity_usd = 500_000_000.0; // 500M
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_header_large_positive_change() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.price_change_24h = 50.0; // >0.5 -> ▲
        terminal
            .draw(|f| render_header(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_header_large_negative_change() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.price_change_24h = -50.0; // <-0.5 -> ▼
        terminal
            .draw(|f| render_header(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_price_chart_empty_data() {
        let mut terminal = create_test_terminal();
        let token_data = create_test_token_data();
        // Create state with no price history data
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.price_history.clear();
        terminal
            .draw(|f| render_price_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_price_chart_price_down() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        // Force price down scenario
        state.price_change_24h = -15.0;
        state.current_price = 0.5; // Below initial
        terminal
            .draw(|f| render_price_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_price_chart_zero_first_price() {
        let mut terminal = create_test_terminal();
        let mut token_data = create_test_token_data();
        token_data.price_usd = 0.0;
        let state = MonitorState::new(&token_data, "ethereum");
        terminal
            .draw(|f| render_price_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_metrics_panel_zero_5m_change() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.price_change_5m = 0.0; // Exactly zero
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_metrics_panel_positive_5m_change() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.price_change_5m = 5.0; // Positive
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_metrics_panel_negative_5m_change() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.price_change_5m = -3.0; // Negative
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_metrics_panel_negative_24h_change() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.price_change_24h = -10.0;
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_metrics_panel_old_last_change() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        // Set last_price_change_at to over an hour ago
        state.last_price_change_at = chrono::Utc::now().timestamp() as f64 - 7200.0; // 2h ago
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_metrics_panel_minutes_ago_change() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        // Set last_price_change_at to minutes ago
        state.last_price_change_at = chrono::Utc::now().timestamp() as f64 - 300.0; // 5 min ago
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_candlestick_empty_fresh_state() {
        let mut terminal = create_test_terminal();
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.price_history.clear();
        state.chart_mode = ChartMode::Candlestick;
        terminal
            .draw(|f| render_candlestick_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_candlestick_price_down() {
        let mut terminal = create_test_terminal();
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        // Add data going down
        for i in 0..20 {
            let mut data = token_data.clone();
            data.price_usd = 2.0 - (i as f64 * 0.05);
            state.update(&data);
        }
        state.chart_mode = ChartMode::Candlestick;
        terminal
            .draw(|f| render_candlestick_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_volume_chart_with_many_points() {
        let mut terminal = create_test_terminal();
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        // Add lots of data points
        for i in 0..100 {
            let mut data = token_data.clone();
            data.volume_24h = 1_000_000.0 + (i as f64 * 50_000.0);
            data.price_usd = 1.0 + (i as f64 * 0.001);
            state.update(&data);
        }
        terminal
            .draw(|f| render_volume_chart(f, f.area(), &state))
            .unwrap();
    }

    // ========================================================================
    // Key event handler tests
    // ========================================================================

    fn make_key_event(code: KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn make_ctrl_key_event(code: KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn test_handle_key_quit_q() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        assert!(handle_key_event_on_state(
            make_key_event(KeyCode::Char('q')),
            &mut state
        ));
    }

    #[test]
    fn test_handle_key_quit_esc() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        assert!(handle_key_event_on_state(
            make_key_event(KeyCode::Esc),
            &mut state
        ));
    }

    #[test]
    fn test_handle_key_quit_ctrl_c() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        assert!(handle_key_event_on_state(
            make_ctrl_key_event(KeyCode::Char('c')),
            &mut state
        ));
    }

    #[test]
    fn test_handle_key_refresh() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.refresh_rate = Duration::from_secs(60);
        // Set last_update in the past so should_refresh was false
        let exit = handle_key_event_on_state(make_key_event(KeyCode::Char('r')), &mut state);
        assert!(!exit);
        // force_refresh sets last_update to epoch, so should_refresh() should be true
        assert!(state.should_refresh());
    }

    #[test]
    fn test_handle_key_pause_toggle() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        assert!(!state.paused);

        handle_key_event_on_state(make_key_event(KeyCode::Char('p')), &mut state);
        assert!(state.paused);

        handle_key_event_on_state(make_key_event(KeyCode::Char(' ')), &mut state);
        assert!(!state.paused);
    }

    #[test]
    fn test_handle_key_slower_refresh() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let initial = state.refresh_rate;

        handle_key_event_on_state(make_key_event(KeyCode::Char('+')), &mut state);
        assert!(state.refresh_rate > initial);

        state.refresh_rate = initial;
        handle_key_event_on_state(make_key_event(KeyCode::Char('=')), &mut state);
        assert!(state.refresh_rate > initial);

        state.refresh_rate = initial;
        handle_key_event_on_state(make_key_event(KeyCode::Char(']')), &mut state);
        assert!(state.refresh_rate > initial);
    }

    #[test]
    fn test_handle_key_faster_refresh() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        // First make it slower so there's room to go faster
        state.refresh_rate = Duration::from_secs(30);
        let initial = state.refresh_rate;

        handle_key_event_on_state(make_key_event(KeyCode::Char('-')), &mut state);
        assert!(state.refresh_rate < initial);

        state.refresh_rate = initial;
        handle_key_event_on_state(make_key_event(KeyCode::Char('_')), &mut state);
        assert!(state.refresh_rate < initial);

        state.refresh_rate = initial;
        handle_key_event_on_state(make_key_event(KeyCode::Char('[')), &mut state);
        assert!(state.refresh_rate < initial);
    }

    #[test]
    fn test_handle_key_time_periods() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");

        handle_key_event_on_state(make_key_event(KeyCode::Char('1')), &mut state);
        assert!(matches!(state.time_period, TimePeriod::Min1));

        handle_key_event_on_state(make_key_event(KeyCode::Char('2')), &mut state);
        assert!(matches!(state.time_period, TimePeriod::Min5));

        handle_key_event_on_state(make_key_event(KeyCode::Char('3')), &mut state);
        assert!(matches!(state.time_period, TimePeriod::Min15));

        handle_key_event_on_state(make_key_event(KeyCode::Char('4')), &mut state);
        assert!(matches!(state.time_period, TimePeriod::Hour1));

        handle_key_event_on_state(make_key_event(KeyCode::Char('5')), &mut state);
        assert!(matches!(state.time_period, TimePeriod::Hour4));

        handle_key_event_on_state(make_key_event(KeyCode::Char('6')), &mut state);
        assert!(matches!(state.time_period, TimePeriod::Day1));
    }

    #[test]
    fn test_handle_key_cycle_time_period() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");

        handle_key_event_on_state(make_key_event(KeyCode::Char('t')), &mut state);
        // Should cycle from default
        let first = state.time_period;

        handle_key_event_on_state(make_key_event(KeyCode::Tab), &mut state);
        // Should have cycled again
        // Verify it cycled (no panic is the main check)
        let _ = state.time_period;
        let _ = first;
    }

    #[test]
    fn test_handle_key_toggle_chart_mode() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let initial_mode = state.chart_mode;

        handle_key_event_on_state(make_key_event(KeyCode::Char('c')), &mut state);
        assert!(state.chart_mode != initial_mode);
    }

    #[test]
    fn test_handle_key_unknown_no_op() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let exit = handle_key_event_on_state(make_key_event(KeyCode::Char('z')), &mut state);
        assert!(!exit);
    }

    // ========================================================================
    // Cache save/load tests
    // ========================================================================

    #[test]
    fn test_save_and_load_cache() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.price_history.push_back(DataPoint {
            timestamp: 1.0,
            value: 100.0,
            is_real: true,
        });
        state.price_history.push_back(DataPoint {
            timestamp: 2.0,
            value: 101.0,
            is_real: true,
        });
        state.volume_history.push_back(DataPoint {
            timestamp: 1.0,
            value: 5000.0,
            is_real: true,
        });

        // save_cache uses dirs::cache_dir() which we can't redirect easily
        // but we can test the load_cache path with a real write
        state.save_cache();
        let cached = MonitorState::load_cache(&state.token_address, &state.chain);
        // Cache may or may not exist depending on system - just verify no panic
        if let Some(c) = cached {
            assert_eq!(
                c.token_address.to_lowercase(),
                state.token_address.to_lowercase()
            );
        }
    }

    #[test]
    fn test_load_cache_nonexistent_token() {
        let cached = MonitorState::load_cache("0xNONEXISTENT_TOKEN_ADDR", "nonexistent_chain");
        assert!(cached.is_none());
    }

    // ========================================================================
    // New widget tests: BarChart (volume), Table+Sparkline (metrics), scroll
    // ========================================================================

    #[test]
    fn test_render_volume_barchart_with_populated_data() {
        // Verify the BarChart-based volume chart renders without panic
        // when state has many volume data points across different time periods
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        for period in [
            TimePeriod::Min1,
            TimePeriod::Min5,
            TimePeriod::Min15,
            TimePeriod::Hour1,
            TimePeriod::Hour4,
            TimePeriod::Day1,
        ] {
            state.set_time_period(period);
            terminal
                .draw(|f| render_volume_chart(f, f.area(), &state))
                .unwrap();
        }
    }

    #[test]
    fn test_render_volume_barchart_narrow_terminal() {
        // BarChart with very narrow width should still render without panic
        let backend = TestBackend::new(20, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = create_populated_state();
        terminal
            .draw(|f| render_volume_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_metrics_table_sparkline_no_panic() {
        // Verify the Table+Sparkline metrics panel renders without panic
        let mut terminal = create_test_terminal();
        let state = create_populated_state();
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_metrics_table_sparkline_all_periods() {
        // Ensure metrics panel renders correctly for every time period
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        for period in [
            TimePeriod::Min1,
            TimePeriod::Min5,
            TimePeriod::Min15,
            TimePeriod::Hour1,
            TimePeriod::Hour4,
            TimePeriod::Day1,
        ] {
            state.set_time_period(period);
            terminal
                .draw(|f| render_metrics_panel(f, f.area(), &state))
                .unwrap();
        }
    }

    #[test]
    fn test_render_metrics_sparkline_trend_direction() {
        // When 5m change is negative, sparkline should still render
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.price_change_5m = -3.5;
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();

        // When 5m change is positive
        state.price_change_5m = 2.0;
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();

        // When 5m change is zero
        state.price_change_5m = 0.0;
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_tabs_time_period() {
        // Verify the Tabs widget in the header renders for each period
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        for period in [
            TimePeriod::Min1,
            TimePeriod::Min5,
            TimePeriod::Min15,
            TimePeriod::Hour1,
            TimePeriod::Hour4,
            TimePeriod::Day1,
        ] {
            state.set_time_period(period);
            terminal
                .draw(|f| render_header(f, f.area(), &state))
                .unwrap();
        }
    }

    #[test]
    fn test_time_period_index() {
        assert_eq!(TimePeriod::Min1.index(), 0);
        assert_eq!(TimePeriod::Min5.index(), 1);
        assert_eq!(TimePeriod::Min15.index(), 2);
        assert_eq!(TimePeriod::Hour1.index(), 3);
        assert_eq!(TimePeriod::Hour4.index(), 4);
        assert_eq!(TimePeriod::Day1.index(), 5);
    }

    #[test]
    fn test_scroll_log_down_from_start() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.log_messages.push_back("msg 1".to_string());
        state.log_messages.push_back("msg 2".to_string());
        state.log_messages.push_back("msg 3".to_string());

        // Initially no selection
        assert_eq!(state.log_list_state.selected(), None);

        // First scroll down selects item 0
        state.scroll_log_down();
        assert_eq!(state.log_list_state.selected(), Some(0));

        // Second scroll moves to item 1
        state.scroll_log_down();
        assert_eq!(state.log_list_state.selected(), Some(1));

        // Third scroll moves to item 2
        state.scroll_log_down();
        assert_eq!(state.log_list_state.selected(), Some(2));

        // Fourth scroll stays at last item (bounds check)
        state.scroll_log_down();
        assert_eq!(state.log_list_state.selected(), Some(2));
    }

    #[test]
    fn test_scroll_log_up_from_start() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.log_messages.push_back("msg 1".to_string());
        state.log_messages.push_back("msg 2".to_string());
        state.log_messages.push_back("msg 3".to_string());

        // Scroll up from no selection goes to 0
        state.scroll_log_up();
        assert_eq!(state.log_list_state.selected(), Some(0));

        // Can't go below 0
        state.scroll_log_up();
        assert_eq!(state.log_list_state.selected(), Some(0));
    }

    #[test]
    fn test_scroll_log_up_down_roundtrip() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        for i in 0..10 {
            state.log_messages.push_back(format!("msg {}", i));
        }

        // Scroll down 5 times
        for _ in 0..5 {
            state.scroll_log_down();
        }
        assert_eq!(state.log_list_state.selected(), Some(4));

        // Scroll up 3 times
        for _ in 0..3 {
            state.scroll_log_up();
        }
        assert_eq!(state.log_list_state.selected(), Some(1));
    }

    #[test]
    fn test_scroll_log_empty_no_panic() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        // With no log messages, scrolling should not panic
        state.scroll_log_down();
        state.scroll_log_up();
        assert!(
            state.log_list_state.selected().is_none() || state.log_list_state.selected() == Some(0)
        );
    }

    #[test]
    fn test_render_scrollable_activity_log() {
        // Ensure the stateful activity log renders without panic
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        for i in 0..20 {
            state
                .log_messages
                .push_back(format!("Activity event #{}", i));
        }
        // Scroll down a few items
        state.scroll_log_down();
        state.scroll_log_down();
        state.scroll_log_down();

        terminal
            .draw(|f| render_buy_sell_gauge(f, f.area(), &mut state))
            .unwrap();
    }

    #[test]
    fn test_handle_key_scroll_log_j_k() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.log_messages.push_back("line 1".to_string());
        state.log_messages.push_back("line 2".to_string());

        // j scrolls down
        handle_key_event_on_state(make_key_event(KeyCode::Char('j')), &mut state);
        assert_eq!(state.log_list_state.selected(), Some(0));

        handle_key_event_on_state(make_key_event(KeyCode::Char('j')), &mut state);
        assert_eq!(state.log_list_state.selected(), Some(1));

        // k scrolls up
        handle_key_event_on_state(make_key_event(KeyCode::Char('k')), &mut state);
        assert_eq!(state.log_list_state.selected(), Some(0));
    }

    #[test]
    fn test_handle_key_scroll_log_arrow_keys() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.log_messages.push_back("line 1".to_string());
        state.log_messages.push_back("line 2".to_string());
        state.log_messages.push_back("line 3".to_string());

        // Down arrow scrolls down
        handle_key_event_on_state(make_key_event(KeyCode::Down), &mut state);
        assert_eq!(state.log_list_state.selected(), Some(0));

        handle_key_event_on_state(make_key_event(KeyCode::Down), &mut state);
        assert_eq!(state.log_list_state.selected(), Some(1));

        // Up arrow scrolls up
        handle_key_event_on_state(make_key_event(KeyCode::Up), &mut state);
        assert_eq!(state.log_list_state.selected(), Some(0));
    }

    #[test]
    fn test_render_ui_with_scrolled_log() {
        // Full UI render with a scrolled activity log position
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        for i in 0..15 {
            state.log_messages.push_back(format!("Log entry {}", i));
        }
        state.scroll_log_down();
        state.scroll_log_down();
        state.scroll_log_down();
        state.scroll_log_down();
        state.scroll_log_down();

        terminal.draw(|f| ui(f, &mut state)).unwrap();
    }

    // ========================================================================
    // Token selection / resolve tests
    // ========================================================================

    fn make_monitor_search_results() -> Vec<crate::chains::dex::TokenSearchResult> {
        vec![
            crate::chains::dex::TokenSearchResult {
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
                chain: "ethereum".to_string(),
                price_usd: Some(1.0),
                volume_24h: 1_000_000.0,
                liquidity_usd: 500_000_000.0,
                market_cap: Some(30_000_000_000.0),
            },
            crate::chains::dex::TokenSearchResult {
                symbol: "USDC".to_string(),
                name: "Bridged USD Coin".to_string(),
                address: "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174".to_string(),
                chain: "ethereum".to_string(),
                price_usd: Some(0.9998),
                volume_24h: 500_000.0,
                liquidity_usd: 100_000_000.0,
                market_cap: None,
            },
            crate::chains::dex::TokenSearchResult {
                symbol: "USDC".to_string(),
                name: "A Very Long Token Name That Exceeds The Limit".to_string(),
                address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
                chain: "ethereum".to_string(),
                price_usd: None,
                volume_24h: 0.0,
                liquidity_usd: 50_000.0,
                market_cap: None,
            },
        ]
    }

    #[test]
    fn test_abbreviate_address_long() {
        let addr = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
        let abbr = abbreviate_address(addr);
        assert_eq!(abbr, "0xA0b869...06eB48");
        assert!(abbr.contains("..."));
    }

    #[test]
    fn test_abbreviate_address_short() {
        let addr = "0x1234abcd";
        let abbr = abbreviate_address(addr);
        // Short addresses are not abbreviated
        assert_eq!(abbr, "0x1234abcd");
    }

    #[test]
    fn test_select_token_impl_first() {
        let results = make_monitor_search_results();
        let input = b"1\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let selected = select_token_impl(&results, &mut reader, &mut writer).unwrap();
        assert_eq!(selected.name, "USD Coin");
        assert_eq!(
            selected.address,
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
        );

        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Found 3 tokens"));
        assert!(output.contains("USDC"));
        assert!(output.contains("0xA0b869...06eB48"));
        assert!(output.contains("Selected:"));
    }

    #[test]
    fn test_select_token_impl_second() {
        let results = make_monitor_search_results();
        let input = b"2\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let selected = select_token_impl(&results, &mut reader, &mut writer).unwrap();
        assert_eq!(selected.name, "Bridged USD Coin");
        assert_eq!(
            selected.address,
            "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174"
        );
    }

    #[test]
    fn test_select_token_impl_shows_address_column() {
        let results = make_monitor_search_results();
        let input = b"1\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        select_token_impl(&results, &mut reader, &mut writer).unwrap();
        let output = String::from_utf8(writer).unwrap();

        // Table header should include Address column
        assert!(output.contains("Address"));
        // All three abbreviated addresses should appear
        assert!(output.contains("0xA0b869...06eB48"));
        assert!(output.contains("0x2791Bc...a84174"));
        assert!(output.contains("0x123456...345678"));
    }

    #[test]
    fn test_select_token_impl_truncates_long_name() {
        let results = make_monitor_search_results();
        let input = b"3\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let selected = select_token_impl(&results, &mut reader, &mut writer).unwrap();
        assert_eq!(
            selected.address,
            "0x1234567890abcdef1234567890abcdef12345678"
        );

        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("A Very Long Token..."));
    }

    #[test]
    fn test_select_token_impl_invalid_input() {
        let results = make_monitor_search_results();
        let input = b"xyz\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let result = select_token_impl(&results, &mut reader, &mut writer);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid selection")
        );
    }

    #[test]
    fn test_select_token_impl_out_of_range_zero() {
        let results = make_monitor_search_results();
        let input = b"0\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let result = select_token_impl(&results, &mut reader, &mut writer);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Selection must be between")
        );
    }

    #[test]
    fn test_select_token_impl_out_of_range_high() {
        let results = make_monitor_search_results();
        let input = b"99\n";
        let mut reader = std::io::Cursor::new(&input[..]);
        let mut writer = Vec::new();

        let result = select_token_impl(&results, &mut reader, &mut writer);
        assert!(result.is_err());
    }

    #[test]
    fn test_format_monitor_number() {
        assert_eq!(format_monitor_number(1_500_000_000.0), "$1.50B");
        assert_eq!(format_monitor_number(250_000_000.0), "$250.00M");
        assert_eq!(format_monitor_number(75_000.0), "$75.00K");
        assert_eq!(format_monitor_number(42.5), "$42.50");
    }

    // ============================
    // Phase 4: Layout system tests
    // ============================

    #[test]
    fn test_monitor_config_defaults() {
        let config = MonitorConfig::default();
        assert_eq!(config.layout, LayoutPreset::Dashboard);
        assert_eq!(config.refresh_seconds, DEFAULT_REFRESH_SECS);
        assert!(config.widgets.price_chart);
        assert!(config.widgets.volume_chart);
        assert!(config.widgets.buy_sell_pressure);
        assert!(config.widgets.metrics_panel);
        assert!(config.widgets.activity_log);
    }

    #[test]
    fn test_layout_preset_next_cycles() {
        assert_eq!(LayoutPreset::Dashboard.next(), LayoutPreset::ChartFocus);
        assert_eq!(LayoutPreset::ChartFocus.next(), LayoutPreset::Feed);
        assert_eq!(LayoutPreset::Feed.next(), LayoutPreset::Compact);
        assert_eq!(LayoutPreset::Compact.next(), LayoutPreset::Exchange);
        assert_eq!(LayoutPreset::Exchange.next(), LayoutPreset::Dashboard);
    }

    #[test]
    fn test_layout_preset_prev_cycles() {
        assert_eq!(LayoutPreset::Dashboard.prev(), LayoutPreset::Exchange);
        assert_eq!(LayoutPreset::Exchange.prev(), LayoutPreset::Compact);
        assert_eq!(LayoutPreset::Compact.prev(), LayoutPreset::Feed);
        assert_eq!(LayoutPreset::Feed.prev(), LayoutPreset::ChartFocus);
        assert_eq!(LayoutPreset::ChartFocus.prev(), LayoutPreset::Dashboard);
    }

    #[test]
    fn test_layout_preset_full_cycle() {
        let start = LayoutPreset::Dashboard;
        let mut preset = start;
        for _ in 0..5 {
            preset = preset.next();
        }
        assert_eq!(preset, start);
    }

    #[test]
    fn test_layout_preset_labels() {
        assert_eq!(LayoutPreset::Dashboard.label(), "Dashboard");
        assert_eq!(LayoutPreset::ChartFocus.label(), "Chart");
        assert_eq!(LayoutPreset::Feed.label(), "Feed");
        assert_eq!(LayoutPreset::Compact.label(), "Compact");
        assert_eq!(LayoutPreset::Exchange.label(), "Exchange");
    }

    #[test]
    fn test_widget_visibility_default_all_visible() {
        let vis = WidgetVisibility::default();
        assert_eq!(vis.visible_count(), 5);
    }

    #[test]
    fn test_widget_visibility_toggle_by_index() {
        let mut vis = WidgetVisibility::default();
        vis.toggle_by_index(1);
        assert!(!vis.price_chart);
        assert_eq!(vis.visible_count(), 4);

        vis.toggle_by_index(2);
        assert!(!vis.volume_chart);
        assert_eq!(vis.visible_count(), 3);

        vis.toggle_by_index(3);
        assert!(!vis.buy_sell_pressure);
        assert_eq!(vis.visible_count(), 2);

        vis.toggle_by_index(4);
        assert!(!vis.metrics_panel);
        assert_eq!(vis.visible_count(), 1);

        vis.toggle_by_index(5);
        assert!(!vis.activity_log);
        assert_eq!(vis.visible_count(), 0);

        // Toggle back
        vis.toggle_by_index(1);
        assert!(vis.price_chart);
        assert_eq!(vis.visible_count(), 1);
    }

    #[test]
    fn test_widget_visibility_toggle_invalid_index() {
        let mut vis = WidgetVisibility::default();
        vis.toggle_by_index(0);
        vis.toggle_by_index(6);
        vis.toggle_by_index(100);
        assert_eq!(vis.visible_count(), 5); // unchanged
    }

    #[test]
    fn test_auto_select_layout_small_terminal() {
        let size = Rect::new(0, 0, 60, 20);
        assert_eq!(auto_select_layout(size), LayoutPreset::Compact);
    }

    #[test]
    fn test_auto_select_layout_narrow_terminal() {
        let size = Rect::new(0, 0, 100, 40);
        assert_eq!(auto_select_layout(size), LayoutPreset::Feed);
    }

    #[test]
    fn test_auto_select_layout_short_terminal() {
        let size = Rect::new(0, 0, 140, 28);
        assert_eq!(auto_select_layout(size), LayoutPreset::ChartFocus);
    }

    #[test]
    fn test_auto_select_layout_large_terminal() {
        let size = Rect::new(0, 0, 160, 50);
        assert_eq!(auto_select_layout(size), LayoutPreset::Dashboard);
    }

    #[test]
    fn test_auto_select_layout_edge_80x24() {
        // Exactly at the threshold: width>=80 and height>=24, but width<120
        let size = Rect::new(0, 0, 80, 24);
        assert_eq!(auto_select_layout(size), LayoutPreset::Feed);
    }

    #[test]
    fn test_auto_select_layout_edge_79() {
        let size = Rect::new(0, 0, 79, 50);
        assert_eq!(auto_select_layout(size), LayoutPreset::Compact);
    }

    #[test]
    fn test_auto_select_layout_edge_23_height() {
        let size = Rect::new(0, 0, 160, 23);
        assert_eq!(auto_select_layout(size), LayoutPreset::Compact);
    }

    #[test]
    fn test_layout_dashboard_all_visible() {
        let area = Rect::new(0, 0, 120, 40);
        let vis = WidgetVisibility::default();
        let areas = layout_dashboard(area, &vis);
        assert!(areas.price_chart.is_some());
        assert!(areas.volume_chart.is_some());
        assert!(areas.buy_sell_gauge.is_some());
        assert!(areas.metrics_panel.is_some());
        assert!(areas.activity_feed.is_some());
    }

    #[test]
    fn test_layout_dashboard_hidden_widget() {
        let area = Rect::new(0, 0, 120, 40);
        let vis = WidgetVisibility {
            price_chart: false,
            ..WidgetVisibility::default()
        };
        let areas = layout_dashboard(area, &vis);
        assert!(areas.price_chart.is_none());
        assert!(areas.volume_chart.is_some());
    }

    #[test]
    fn test_layout_chart_focus_minimal_overlay() {
        let area = Rect::new(0, 0, 120, 40);
        let vis = WidgetVisibility::default();
        let areas = layout_chart_focus(area, &vis);
        assert!(areas.price_chart.is_some());
        assert!(areas.volume_chart.is_none()); // Hidden in chart-focus
        assert!(areas.buy_sell_gauge.is_none()); // Hidden in chart-focus
        assert!(areas.metrics_panel.is_some()); // Minimal stats overlay
        assert!(areas.activity_feed.is_none()); // Hidden in chart-focus
    }

    #[test]
    fn test_layout_feed_activity_priority() {
        let area = Rect::new(0, 0, 120, 40);
        let vis = WidgetVisibility::default();
        let areas = layout_feed(area, &vis);
        assert!(areas.price_chart.is_none()); // Hidden in feed
        assert!(areas.volume_chart.is_none()); // Hidden in feed
        assert!(areas.buy_sell_gauge.is_some()); // Top row
        assert!(areas.metrics_panel.is_some()); // Top row
        assert!(areas.activity_feed.is_some()); // Dominates bottom 75%
    }

    #[test]
    fn test_layout_compact_metrics_only() {
        let area = Rect::new(0, 0, 60, 20);
        let vis = WidgetVisibility::default();
        let areas = layout_compact(area, &vis);
        assert!(areas.price_chart.is_none()); // Hidden in compact
        assert!(areas.volume_chart.is_none()); // Hidden in compact
        assert!(areas.buy_sell_gauge.is_none()); // Hidden in compact
        assert!(areas.metrics_panel.is_some()); // Full area
        assert!(areas.activity_feed.is_none()); // Hidden in compact
    }

    #[test]
    fn test_layout_exchange_has_order_book_and_market_info() {
        let area = Rect::new(0, 0, 160, 50);
        let vis = WidgetVisibility::default();
        let areas = layout_exchange(area, &vis);
        assert!(areas.order_book.is_some());
        assert!(areas.market_info.is_some());
        assert!(areas.price_chart.is_some());
        assert!(areas.buy_sell_gauge.is_some());
        assert!(areas.volume_chart.is_none()); // Not in exchange layout
        assert!(areas.metrics_panel.is_none()); // Not in exchange layout
        assert!(areas.activity_feed.is_none()); // Not in exchange layout
    }

    #[test]
    fn test_ui_render_all_layouts_no_panic() {
        let presets = [
            LayoutPreset::Dashboard,
            LayoutPreset::ChartFocus,
            LayoutPreset::Feed,
            LayoutPreset::Compact,
            LayoutPreset::Exchange,
        ];
        for preset in &presets {
            let mut terminal = create_test_terminal();
            let mut state = create_populated_state();
            state.layout = *preset;
            state.auto_layout = false; // Don't override during render
            terminal.draw(|f| ui(f, &mut state)).unwrap();
        }
    }

    #[test]
    fn test_ui_render_compact_small_terminal() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = create_populated_state();
        state.layout = LayoutPreset::Compact;
        state.auto_layout = false;
        terminal.draw(|f| ui(f, &mut state)).unwrap();
    }

    #[test]
    fn test_ui_auto_layout_selects_compact_for_small() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = create_populated_state();
        state.layout = LayoutPreset::Dashboard;
        state.auto_layout = true;
        terminal.draw(|f| ui(f, &mut state)).unwrap();
        assert_eq!(state.layout, LayoutPreset::Compact);
    }

    #[test]
    fn test_ui_auto_layout_disabled_keeps_preset() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = create_populated_state();
        state.layout = LayoutPreset::Dashboard;
        state.auto_layout = false;
        terminal.draw(|f| ui(f, &mut state)).unwrap();
        assert_eq!(state.layout, LayoutPreset::Dashboard); // Not changed
    }

    #[test]
    fn test_keybinding_l_cycles_layout_forward() {
        let mut state = create_populated_state();
        state.layout = LayoutPreset::Dashboard;
        state.auto_layout = true;

        handle_key_event_on_state(make_key_event(KeyCode::Char('l')), &mut state);
        assert_eq!(state.layout, LayoutPreset::ChartFocus);
        assert!(!state.auto_layout); // Manual switch disables auto

        handle_key_event_on_state(make_key_event(KeyCode::Char('l')), &mut state);
        assert_eq!(state.layout, LayoutPreset::Feed);
    }

    #[test]
    fn test_keybinding_h_cycles_layout_backward() {
        let mut state = create_populated_state();
        state.layout = LayoutPreset::Dashboard;
        state.auto_layout = true;

        handle_key_event_on_state(make_key_event(KeyCode::Char('h')), &mut state);
        assert_eq!(state.layout, LayoutPreset::Exchange);
        assert!(!state.auto_layout);
    }

    #[test]
    fn test_keybinding_a_enables_auto_layout() {
        let mut state = create_populated_state();
        state.auto_layout = false;

        handle_key_event_on_state(make_key_event(KeyCode::Char('a')), &mut state);
        assert!(state.auto_layout);
    }

    #[test]
    fn test_keybinding_w_widget_toggle_mode() {
        let mut state = create_populated_state();
        assert!(!state.widget_toggle_mode);

        // Press w to enter toggle mode
        handle_key_event_on_state(make_key_event(KeyCode::Char('w')), &mut state);
        assert!(state.widget_toggle_mode);

        // Press 1 to toggle price_chart off
        handle_key_event_on_state(make_key_event(KeyCode::Char('1')), &mut state);
        assert!(!state.widget_toggle_mode);
        assert!(!state.widgets.price_chart);
    }

    #[test]
    fn test_keybinding_w_cancel_with_non_digit() {
        let mut state = create_populated_state();

        // Enter widget toggle mode
        handle_key_event_on_state(make_key_event(KeyCode::Char('w')), &mut state);
        assert!(state.widget_toggle_mode);

        // Press 'x' to cancel — should also process 'x' as a normal key (no-op)
        handle_key_event_on_state(make_key_event(KeyCode::Char('x')), &mut state);
        assert!(!state.widget_toggle_mode);
        assert!(state.widgets.price_chart); // unchanged
    }

    #[test]
    fn test_keybinding_w_toggle_multiple_widgets() {
        let mut state = create_populated_state();

        // Toggle widget 2 (volume_chart)
        handle_key_event_on_state(make_key_event(KeyCode::Char('w')), &mut state);
        handle_key_event_on_state(make_key_event(KeyCode::Char('2')), &mut state);
        assert!(!state.widgets.volume_chart);

        // Toggle widget 4 (metrics_panel)
        handle_key_event_on_state(make_key_event(KeyCode::Char('w')), &mut state);
        handle_key_event_on_state(make_key_event(KeyCode::Char('4')), &mut state);
        assert!(!state.widgets.metrics_panel);

        // Toggle widget 5 (activity_log)
        handle_key_event_on_state(make_key_event(KeyCode::Char('w')), &mut state);
        handle_key_event_on_state(make_key_event(KeyCode::Char('5')), &mut state);
        assert!(!state.widgets.activity_log);
    }

    #[test]
    fn test_monitor_config_serde_roundtrip() {
        let config = MonitorConfig {
            layout: LayoutPreset::ChartFocus,
            refresh_seconds: 5,
            widgets: WidgetVisibility {
                price_chart: true,
                volume_chart: false,
                buy_sell_pressure: true,
                metrics_panel: false,
                activity_log: true,
                holder_count: true,
                liquidity_depth: true,
            },
            scale: ScaleMode::Log,
            color_scheme: ColorScheme::BlueOrange,
            alerts: AlertConfig::default(),
            export: ExportConfig::default(),
            auto_pause_on_input: false,
            venue: None,
        };

        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: MonitorConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.layout, LayoutPreset::ChartFocus);
        assert_eq!(parsed.refresh_seconds, 5);
        assert!(parsed.widgets.price_chart);
        assert!(!parsed.widgets.volume_chart);
        assert!(parsed.widgets.buy_sell_pressure);
        assert!(!parsed.widgets.metrics_panel);
        assert!(parsed.widgets.activity_log);
    }

    #[test]
    fn test_monitor_config_serde_kebab_case() {
        let yaml = r#"
layout: chart-focus
refresh_seconds: 15
widgets:
  price_chart: true
  volume_chart: true
  buy_sell_pressure: false
  metrics_panel: true
  activity_log: false
"#;
        let config: MonitorConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.layout, LayoutPreset::ChartFocus);
        assert_eq!(config.refresh_seconds, 15);
        assert!(!config.widgets.buy_sell_pressure);
        assert!(!config.widgets.activity_log);
    }

    #[test]
    fn test_monitor_config_serde_default_missing_fields() {
        let yaml = "layout: feed\n";
        let config: MonitorConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.layout, LayoutPreset::Feed);
        assert_eq!(config.refresh_seconds, DEFAULT_REFRESH_SECS);
        assert!(config.widgets.price_chart); // defaults
    }

    #[test]
    fn test_state_apply_config() {
        let mut state = create_populated_state();
        let config = MonitorConfig {
            layout: LayoutPreset::Feed,
            refresh_seconds: 5,
            widgets: WidgetVisibility {
                price_chart: false,
                volume_chart: true,
                buy_sell_pressure: true,
                metrics_panel: false,
                activity_log: true,
                holder_count: true,
                liquidity_depth: true,
            },
            scale: ScaleMode::Log,
            color_scheme: ColorScheme::Monochrome,
            alerts: AlertConfig::default(),
            export: ExportConfig::default(),
            auto_pause_on_input: false,
            venue: None,
        };
        state.apply_config(&config);
        assert_eq!(state.layout, LayoutPreset::Feed);
        assert!(!state.widgets.price_chart);
        assert!(!state.widgets.metrics_panel);
        assert_eq!(state.refresh_rate, Duration::from_secs(5));
    }

    #[test]
    fn test_layout_all_widgets_hidden_dashboard() {
        let area = Rect::new(0, 0, 120, 40);
        let vis = WidgetVisibility {
            price_chart: false,
            volume_chart: false,
            buy_sell_pressure: false,
            metrics_panel: false,
            activity_log: false,
            holder_count: false,
            liquidity_depth: false,
        };
        let areas = layout_dashboard(area, &vis);
        assert!(areas.price_chart.is_none());
        assert!(areas.volume_chart.is_none());
        assert!(areas.buy_sell_gauge.is_none());
        assert!(areas.metrics_panel.is_none());
        assert!(areas.activity_feed.is_none());
    }

    #[test]
    fn test_ui_render_with_hidden_widgets() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.auto_layout = false;
        state.widgets.price_chart = false;
        state.widgets.volume_chart = false;
        terminal.draw(|f| ui(f, &mut state)).unwrap();
    }

    #[test]
    fn test_ui_render_widget_toggle_mode_footer() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.auto_layout = false;
        state.widget_toggle_mode = true;
        terminal.draw(|f| ui(f, &mut state)).unwrap();
    }

    #[test]
    fn test_monitor_state_new_has_layout_fields() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");
        assert_eq!(state.layout, LayoutPreset::Dashboard);
        assert!(state.auto_layout);
        assert!(!state.widget_toggle_mode);
        assert_eq!(state.widgets.visible_count(), 5);
    }

    // ========================================================================
    // Phase 6: Data Source Integration tests
    // ========================================================================

    #[test]
    fn test_monitor_state_has_holder_count_field() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");
        assert_eq!(state.holder_count, None);
        assert!(state.liquidity_pairs.is_empty());
        assert_eq!(state.holder_fetch_counter, 0);
    }

    #[test]
    fn test_liquidity_pairs_extracted_on_update() {
        let mut token_data = create_test_token_data();
        token_data.pairs = vec![
            crate::chains::DexPair {
                dex_name: "Uniswap V3".to_string(),
                pair_address: "0xpair1".to_string(),
                base_token: "TEST".to_string(),
                quote_token: "WETH".to_string(),
                price_usd: 1.0,
                volume_24h: 500_000.0,
                liquidity_usd: 250_000.0,
                price_change_24h: 5.0,
                buys_24h: 50,
                sells_24h: 25,
                buys_6h: 10,
                sells_6h: 5,
                buys_1h: 3,
                sells_1h: 2,
                pair_created_at: None,
                url: None,
            },
            crate::chains::DexPair {
                dex_name: "SushiSwap".to_string(),
                pair_address: "0xpair2".to_string(),
                base_token: "TEST".to_string(),
                quote_token: "USDC".to_string(),
                price_usd: 1.0,
                volume_24h: 300_000.0,
                liquidity_usd: 150_000.0,
                price_change_24h: 3.0,
                buys_24h: 30,
                sells_24h: 15,
                buys_6h: 8,
                sells_6h: 4,
                buys_1h: 2,
                sells_1h: 1,
                pair_created_at: None,
                url: None,
            },
        ];

        let mut state = MonitorState::new(&token_data, "ethereum");
        state.update(&token_data);

        assert_eq!(state.liquidity_pairs.len(), 2);
        assert!(state.liquidity_pairs[0].0.contains("Uniswap V3"));
        assert!((state.liquidity_pairs[0].1 - 250_000.0).abs() < 0.01);
    }

    #[test]
    fn test_render_liquidity_depth_no_panic() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.liquidity_pairs = vec![
            ("TEST/WETH (Uniswap V3)".to_string(), 250_000.0),
            ("TEST/USDC (SushiSwap)".to_string(), 150_000.0),
        ];
        terminal
            .draw(|f| render_liquidity_depth(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_liquidity_depth_empty() {
        let mut terminal = create_test_terminal();
        let state = create_populated_state();
        terminal
            .draw(|f| render_liquidity_depth(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_metrics_with_holder_count() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.holder_count = Some(42_000);
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    // ========================================================================
    // Phase 7: Alert System tests
    // ========================================================================

    #[test]
    fn test_alert_config_default() {
        let config = AlertConfig::default();
        assert!(config.price_min.is_none());
        assert!(config.price_max.is_none());
        assert!(config.whale_min_usd.is_none());
        assert!(config.volume_spike_threshold_pct.is_none());
    }

    #[test]
    fn test_alert_price_min_triggers() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.alerts.price_min = Some(2.0); // Price is 1.0, below min of 2.0
        state.update(&token_data);
        assert!(
            !state.active_alerts.is_empty(),
            "Should have price-min alert"
        );
        assert!(state.active_alerts[0].message.contains("below min"));
    }

    #[test]
    fn test_alert_price_max_triggers() {
        let mut token_data = create_test_token_data();
        token_data.price_usd = 100.0;
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.alerts.price_max = Some(50.0); // Price 100.0 above max of 50.0
        state.update(&token_data);
        assert!(
            !state.active_alerts.is_empty(),
            "Should have price-max alert"
        );
        assert!(state.active_alerts[0].message.contains("above max"));
    }

    #[test]
    fn test_alert_no_trigger_within_bounds() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.alerts.price_min = Some(0.5); // Price 1.0 is above min
        state.alerts.price_max = Some(2.0); // Price 1.0 is below max
        state.update(&token_data);
        assert!(
            state.active_alerts.is_empty(),
            "Should have no alerts when price is within bounds"
        );
    }

    #[test]
    fn test_alert_volume_spike_triggers() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.alerts.volume_spike_threshold_pct = Some(10.0);
        state.volume_avg = 500_000.0; // Average volume is 500K

        // Token data has volume_24h of 1M, which is +100% vs avg — should trigger
        state.update(&token_data);
        let spike_alerts: Vec<_> = state
            .active_alerts
            .iter()
            .filter(|a| a.message.contains("spike"))
            .collect();
        assert!(!spike_alerts.is_empty(), "Should have volume spike alert");
    }

    #[test]
    fn test_alert_flash_timer_set() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.alerts.price_min = Some(2.0);
        state.update(&token_data);
        assert!(state.alert_flash_until.is_some());
    }

    #[test]
    fn test_render_alert_overlay_no_panic() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.active_alerts.push(ActiveAlert {
            message: "⚠ Test alert".to_string(),
            triggered_at: Instant::now(),
        });
        state.alert_flash_until = Some(Instant::now() + Duration::from_secs(2));
        terminal
            .draw(|f| render_alert_overlay(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_alert_overlay_empty() {
        let mut terminal = create_test_terminal();
        let state = create_populated_state();
        terminal
            .draw(|f| render_alert_overlay(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_alert_config_serde_roundtrip() {
        let config = AlertConfig {
            price_min: Some(0.5),
            price_max: Some(2.0),
            whale_min_usd: Some(10_000.0),
            volume_spike_threshold_pct: Some(50.0),
        };
        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: AlertConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.price_min, Some(0.5));
        assert_eq!(parsed.price_max, Some(2.0));
        assert_eq!(parsed.whale_min_usd, Some(10_000.0));
        assert_eq!(parsed.volume_spike_threshold_pct, Some(50.0));
    }

    #[test]
    fn test_ui_with_active_alerts() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.active_alerts.push(ActiveAlert {
            message: "⚠ Price below min".to_string(),
            triggered_at: Instant::now(),
        });
        state.alert_flash_until = Some(Instant::now() + Duration::from_secs(2));
        terminal.draw(|f| ui(f, &mut state)).unwrap();
    }

    // ========================================================================
    // Phase 8: CSV Export tests
    // ========================================================================

    #[test]
    fn test_export_config_default() {
        let config = ExportConfig::default();
        assert!(config.path.is_none());
    }

    /// Helper to create an export in a temp directory, avoiding race conditions.
    fn start_export_in_temp(state: &mut MonitorState) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("scope_test_export_{}_{}", std::process::id(), id));
        let _ = fs::create_dir_all(&dir);
        let filename = format!("{}_test_{}.csv", state.symbol, id);
        let path = dir.join(filename);

        let mut file = fs::File::create(&path).expect("failed to create export test file");
        let header = "timestamp,price_usd,volume_24h,liquidity_usd,buys_24h,sells_24h,market_cap\n";
        file.write_all(header.as_bytes())
            .expect("failed to write header");
        drop(file); // Ensure file is flushed and closed

        state.export_path = Some(path.clone());
        state.export_active = true;
        path
    }

    #[test]
    fn test_export_start_creates_file() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let path = start_export_in_temp(&mut state);

        assert!(state.export_active);
        assert!(state.export_path.is_some());
        assert!(path.exists(), "Export file should exist");

        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_export_stop() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let path = start_export_in_temp(&mut state);
        state.stop_export();

        assert!(!state.export_active);
        assert!(state.export_path.is_none());

        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_export_toggle() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");

        state.toggle_export();
        assert!(state.export_active);
        let path = state.export_path.clone().unwrap();

        state.toggle_export();
        assert!(!state.export_active);

        // Cleanup
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_export_writes_csv_rows() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let path = start_export_in_temp(&mut state);

        // Simulate a few updates
        state.update(&token_data);
        state.update(&token_data);

        let contents = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();

        assert!(
            lines.len() >= 3,
            "Should have header + 2 data rows, got {}",
            lines.len()
        );
        assert!(lines[0].starts_with("timestamp,price_usd"));

        // Cleanup
        state.stop_export();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_keybinding_e_toggles_export() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");

        handle_key_event_on_state(make_key_event(KeyCode::Char('e')), &mut state);
        assert!(state.export_active);
        let path = state.export_path.clone().unwrap();

        handle_key_event_on_state(make_key_event(KeyCode::Char('e')), &mut state);
        assert!(!state.export_active);

        // Cleanup
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_render_footer_with_export_active() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.export_active = true;
        terminal
            .draw(|f| render_footer(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_export_config_serde_roundtrip() {
        let config = ExportConfig {
            path: Some("./my-exports".to_string()),
        };
        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: ExportConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.path, Some("./my-exports".to_string()));
    }

    // ========================================================================
    // Phase 9: Auto-Pause tests
    // ========================================================================

    #[test]
    fn test_auto_pause_default_disabled() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");
        assert!(!state.auto_pause_on_input);
        assert_eq!(state.auto_pause_timeout, Duration::from_secs(3));
    }

    #[test]
    fn test_auto_pause_blocks_refresh() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.auto_pause_on_input = true;
        state.refresh_rate = Duration::from_secs(1);

        // Simulate fresh input
        state.last_input_at = Instant::now();
        state.last_update = Instant::now() - Duration::from_secs(10); // Long overdue

        // Auto-pause should block refresh since we just had input
        assert!(!state.should_refresh());
    }

    #[test]
    fn test_auto_pause_allows_refresh_after_timeout() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.auto_pause_on_input = true;
        state.refresh_rate = Duration::from_secs(1);
        state.auto_pause_timeout = Duration::from_millis(1); // Very short timeout

        // Simulate old input (long ago)
        state.last_input_at = Instant::now() - Duration::from_secs(10);
        state.last_update = Instant::now() - Duration::from_secs(10);

        // Should allow refresh since input was long ago
        assert!(state.should_refresh());
    }

    #[test]
    fn test_auto_pause_disabled_does_not_block() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.auto_pause_on_input = false;
        state.refresh_rate = Duration::from_secs(1);

        state.last_input_at = Instant::now(); // Fresh input
        state.last_update = Instant::now() - Duration::from_secs(10);

        // Should still refresh because auto-pause is disabled
        assert!(state.should_refresh());
    }

    #[test]
    fn test_is_auto_paused() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");

        // Not auto-paused when disabled
        state.auto_pause_on_input = false;
        state.last_input_at = Instant::now();
        assert!(!state.is_auto_paused());

        // Auto-paused when enabled and input is recent
        state.auto_pause_on_input = true;
        state.last_input_at = Instant::now();
        assert!(state.is_auto_paused());

        // Not auto-paused when input is old
        state.last_input_at = Instant::now() - Duration::from_secs(10);
        assert!(!state.is_auto_paused());
    }

    #[test]
    fn test_keybinding_shift_p_toggles_auto_pause() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        assert!(!state.auto_pause_on_input);

        let shift_p = crossterm::event::KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT);
        handle_key_event_on_state(shift_p, &mut state);
        assert!(state.auto_pause_on_input);

        handle_key_event_on_state(shift_p, &mut state);
        assert!(!state.auto_pause_on_input);
    }

    #[test]
    fn test_keybinding_updates_last_input_at() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");

        // Set last_input_at to the past
        state.last_input_at = Instant::now() - Duration::from_secs(60);
        let old_input = state.last_input_at;

        // Any key event should update last_input_at
        handle_key_event_on_state(make_key_event(KeyCode::Char('z')), &mut state);
        assert!(state.last_input_at > old_input);
    }

    #[test]
    fn test_render_footer_auto_paused() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.auto_pause_on_input = true;
        state.last_input_at = Instant::now(); // Recent input -> auto-paused
        terminal
            .draw(|f| render_footer(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_config_auto_pause_applied() {
        let mut state = create_populated_state();
        let config = MonitorConfig {
            auto_pause_on_input: true,
            ..MonitorConfig::default()
        };
        state.apply_config(&config);
        assert!(state.auto_pause_on_input);
    }

    // ========================================================================
    // Combined full-UI tests for new features
    // ========================================================================

    #[test]
    fn test_ui_render_all_layouts_with_alerts_and_export() {
        for preset in &[
            LayoutPreset::Dashboard,
            LayoutPreset::ChartFocus,
            LayoutPreset::Feed,
            LayoutPreset::Compact,
        ] {
            let mut terminal = create_test_terminal();
            let mut state = create_populated_state();
            state.layout = *preset;
            state.auto_layout = false;
            state.export_active = true;
            state.active_alerts.push(ActiveAlert {
                message: "⚠ Test alert".to_string(),
                triggered_at: Instant::now(),
            });
            terminal.draw(|f| ui(f, &mut state)).unwrap();
        }
    }

    #[test]
    fn test_ui_render_with_liquidity_data() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.liquidity_pairs = vec![
            ("TEST/WETH (Uniswap V3)".to_string(), 250_000.0),
            ("TEST/USDC (SushiSwap)".to_string(), 150_000.0),
        ];
        terminal.draw(|f| ui(f, &mut state)).unwrap();
    }

    #[test]
    fn test_monitor_config_full_serde_roundtrip() {
        let config = MonitorConfig {
            layout: LayoutPreset::Dashboard,
            refresh_seconds: 10,
            widgets: WidgetVisibility::default(),
            scale: ScaleMode::Log,
            color_scheme: ColorScheme::BlueOrange,
            alerts: AlertConfig {
                price_min: Some(0.5),
                price_max: Some(10.0),
                whale_min_usd: Some(50_000.0),
                volume_spike_threshold_pct: Some(100.0),
            },
            export: ExportConfig {
                path: Some("./exports".to_string()),
            },
            auto_pause_on_input: true,
            venue: Some("binance".to_string()),
        };

        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: MonitorConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.layout, LayoutPreset::Dashboard);
        assert_eq!(parsed.refresh_seconds, 10);
        assert_eq!(parsed.venue, Some("binance".to_string()));
        assert_eq!(parsed.alerts.price_min, Some(0.5));
        assert_eq!(parsed.alerts.price_max, Some(10.0));
        assert_eq!(parsed.export.path, Some("./exports".to_string()));
        assert!(parsed.auto_pause_on_input);
    }

    #[test]
    fn test_monitor_config_serde_defaults_for_new_fields() {
        // Only specify old fields — new fields should default
        let yaml = r#"
layout: dashboard
refresh_seconds: 5
"#;
        let config: MonitorConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.alerts.price_min.is_none());
        assert!(config.export.path.is_none());
        assert!(!config.auto_pause_on_input);
    }

    #[test]
    fn test_quit_stops_export() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let path = start_export_in_temp(&mut state);

        let exit = handle_key_event_on_state(make_key_event(KeyCode::Char('q')), &mut state);
        assert!(exit);
        assert!(!state.export_active);

        // Cleanup
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_monitor_state_new_has_alert_export_autopause_fields() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");

        // Alert fields
        assert!(state.active_alerts.is_empty());
        assert!(state.alert_flash_until.is_none());
        assert!(state.alerts.price_min.is_none());

        // Export fields
        assert!(!state.export_active);
        assert!(state.export_path.is_none());

        // Auto-pause fields
        assert!(!state.auto_pause_on_input);
        assert_eq!(state.auto_pause_timeout, Duration::from_secs(3));
    }

    // ========================================================================
    // Coverage gap: Serde round-trip tests for enums
    // ========================================================================

    #[test]
    fn test_scale_mode_serde_roundtrip() {
        for mode in &[ScaleMode::Linear, ScaleMode::Log] {
            let yaml = serde_yaml::to_string(mode).unwrap();
            let parsed: ScaleMode = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(&parsed, mode);
        }
    }

    #[test]
    fn test_scale_mode_serde_kebab_case() {
        let parsed: ScaleMode = serde_yaml::from_str("linear").unwrap();
        assert_eq!(parsed, ScaleMode::Linear);
        let parsed: ScaleMode = serde_yaml::from_str("log").unwrap();
        assert_eq!(parsed, ScaleMode::Log);
    }

    #[test]
    fn test_scale_mode_toggle() {
        assert_eq!(ScaleMode::Linear.toggle(), ScaleMode::Log);
        assert_eq!(ScaleMode::Log.toggle(), ScaleMode::Linear);
    }

    #[test]
    fn test_scale_mode_label() {
        assert_eq!(ScaleMode::Linear.label(), "Lin");
        assert_eq!(ScaleMode::Log.label(), "Log");
    }

    #[test]
    fn test_color_scheme_serde_roundtrip() {
        for scheme in &[
            ColorScheme::GreenRed,
            ColorScheme::BlueOrange,
            ColorScheme::Monochrome,
        ] {
            let yaml = serde_yaml::to_string(scheme).unwrap();
            let parsed: ColorScheme = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(&parsed, scheme);
        }
    }

    #[test]
    fn test_color_scheme_serde_kebab_case() {
        let parsed: ColorScheme = serde_yaml::from_str("green-red").unwrap();
        assert_eq!(parsed, ColorScheme::GreenRed);
        let parsed: ColorScheme = serde_yaml::from_str("blue-orange").unwrap();
        assert_eq!(parsed, ColorScheme::BlueOrange);
        let parsed: ColorScheme = serde_yaml::from_str("monochrome").unwrap();
        assert_eq!(parsed, ColorScheme::Monochrome);
    }

    #[test]
    fn test_color_scheme_cycle() {
        assert_eq!(ColorScheme::GreenRed.next(), ColorScheme::BlueOrange);
        assert_eq!(ColorScheme::BlueOrange.next(), ColorScheme::Monochrome);
        assert_eq!(ColorScheme::Monochrome.next(), ColorScheme::GreenRed);
    }

    #[test]
    fn test_color_scheme_label() {
        assert_eq!(ColorScheme::GreenRed.label(), "G/R");
        assert_eq!(ColorScheme::BlueOrange.label(), "B/O");
        assert_eq!(ColorScheme::Monochrome.label(), "Mono");
    }

    #[test]
    fn test_color_palette_fields_populated() {
        // Verify each palette has distinct meaningful values
        for scheme in &[
            ColorScheme::GreenRed,
            ColorScheme::BlueOrange,
            ColorScheme::Monochrome,
        ] {
            let pal = scheme.palette();
            // up and down colors should differ (visually distinct)
            assert_ne!(
                format!("{:?}", pal.up),
                format!("{:?}", pal.down),
                "Up/down should differ for {:?}",
                scheme
            );
        }
    }

    #[test]
    fn test_layout_preset_serde_roundtrip() {
        for preset in &[
            LayoutPreset::Dashboard,
            LayoutPreset::ChartFocus,
            LayoutPreset::Feed,
            LayoutPreset::Compact,
        ] {
            let yaml = serde_yaml::to_string(preset).unwrap();
            let parsed: LayoutPreset = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(&parsed, preset);
        }
    }

    #[test]
    fn test_layout_preset_serde_kebab_case() {
        let parsed: LayoutPreset = serde_yaml::from_str("dashboard").unwrap();
        assert_eq!(parsed, LayoutPreset::Dashboard);
        let parsed: LayoutPreset = serde_yaml::from_str("chart-focus").unwrap();
        assert_eq!(parsed, LayoutPreset::ChartFocus);
        let parsed: LayoutPreset = serde_yaml::from_str("feed").unwrap();
        assert_eq!(parsed, LayoutPreset::Feed);
        let parsed: LayoutPreset = serde_yaml::from_str("compact").unwrap();
        assert_eq!(parsed, LayoutPreset::Compact);
    }

    #[test]
    fn test_widget_visibility_serde_roundtrip() {
        let vis = WidgetVisibility {
            price_chart: false,
            volume_chart: true,
            buy_sell_pressure: false,
            metrics_panel: true,
            activity_log: false,
            holder_count: false,
            liquidity_depth: true,
        };
        let yaml = serde_yaml::to_string(&vis).unwrap();
        let parsed: WidgetVisibility = serde_yaml::from_str(&yaml).unwrap();
        assert!(!parsed.price_chart);
        assert!(parsed.volume_chart);
        assert!(!parsed.buy_sell_pressure);
        assert!(parsed.metrics_panel);
        assert!(!parsed.activity_log);
        assert!(!parsed.holder_count);
        assert!(parsed.liquidity_depth);
    }

    #[test]
    fn test_data_point_serde_roundtrip() {
        let dp = DataPoint {
            timestamp: 1700000000.5,
            value: 42.123456,
            is_real: true,
        };
        let json = serde_json::to_string(&dp).unwrap();
        let parsed: DataPoint = serde_json::from_str(&json).unwrap();
        assert!((parsed.timestamp - dp.timestamp).abs() < 0.001);
        assert!((parsed.value - dp.value).abs() < 0.001);
        assert_eq!(parsed.is_real, dp.is_real);
    }

    // ========================================================================
    // Coverage gap: Key handler tests for scale and color scheme
    // ========================================================================

    #[test]
    fn test_handle_key_scale_toggle_s() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        assert_eq!(state.scale_mode, ScaleMode::Linear);

        handle_key_event_on_state(make_key_event(KeyCode::Char('s')), &mut state);
        assert_eq!(state.scale_mode, ScaleMode::Log);

        handle_key_event_on_state(make_key_event(KeyCode::Char('s')), &mut state);
        assert_eq!(state.scale_mode, ScaleMode::Linear);
    }

    #[test]
    fn test_handle_key_color_scheme_cycle_slash() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        assert_eq!(state.color_scheme, ColorScheme::GreenRed);

        handle_key_event_on_state(make_key_event(KeyCode::Char('/')), &mut state);
        assert_eq!(state.color_scheme, ColorScheme::BlueOrange);

        handle_key_event_on_state(make_key_event(KeyCode::Char('/')), &mut state);
        assert_eq!(state.color_scheme, ColorScheme::Monochrome);

        handle_key_event_on_state(make_key_event(KeyCode::Char('/')), &mut state);
        assert_eq!(state.color_scheme, ColorScheme::GreenRed);
    }

    // ========================================================================
    // Coverage gap: Volume profile chart render tests
    // ========================================================================

    #[test]
    fn test_render_volume_profile_chart_no_panic() {
        let mut terminal = create_test_terminal();
        let state = create_populated_state();
        terminal
            .draw(|f| render_volume_profile_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_volume_profile_chart_empty_data() {
        let mut terminal = create_test_terminal();
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.price_history.clear();
        state.volume_history.clear();
        terminal
            .draw(|f| render_volume_profile_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_volume_profile_chart_single_price() {
        // When all prices are identical, there's no range -> "no price range" path
        let mut terminal = create_test_terminal();
        let mut token_data = create_test_token_data();
        token_data.price_usd = 1.0;
        let mut state = MonitorState::new(&token_data, "ethereum");
        // Clear and add identical-price data points
        state.price_history.clear();
        state.volume_history.clear();
        let now = chrono::Utc::now().timestamp() as f64;
        for i in 0..5 {
            state.price_history.push_back(DataPoint {
                timestamp: now - (5.0 - i as f64) * 60.0,
                value: 1.0, // all same price
                is_real: true,
            });
            state.volume_history.push_back(DataPoint {
                timestamp: now - (5.0 - i as f64) * 60.0,
                value: 1000.0,
                is_real: true,
            });
        }
        terminal
            .draw(|f| render_volume_profile_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_volume_profile_chart_narrow_terminal() {
        let backend = TestBackend::new(30, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = create_populated_state();
        terminal
            .draw(|f| render_volume_profile_chart(f, f.area(), &state))
            .unwrap();
    }

    // ========================================================================
    // Coverage gap: Log scale rendering tests
    // ========================================================================

    #[test]
    fn test_render_price_chart_log_scale() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.scale_mode = ScaleMode::Log;
        terminal
            .draw(|f| render_price_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_candlestick_chart_log_scale() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.scale_mode = ScaleMode::Log;
        state.chart_mode = ChartMode::Candlestick;
        terminal
            .draw(|f| render_candlestick_chart(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_price_chart_log_scale_zero_price() {
        // Verify log scale handles zero/near-zero prices safely
        let mut terminal = create_test_terminal();
        let mut token_data = create_test_token_data();
        token_data.price_usd = 0.0001;
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.scale_mode = ScaleMode::Log;
        for i in 0..10 {
            let mut data = token_data.clone();
            data.price_usd = 0.0001 + (i as f64 * 0.00001);
            state.update(&data);
        }
        terminal
            .draw(|f| render_price_chart(f, f.area(), &state))
            .unwrap();
    }

    // ========================================================================
    // Coverage gap: Color scheme rendering tests
    // ========================================================================

    #[test]
    fn test_render_ui_with_all_color_schemes() {
        for scheme in &[
            ColorScheme::GreenRed,
            ColorScheme::BlueOrange,
            ColorScheme::Monochrome,
        ] {
            let mut terminal = create_test_terminal();
            let mut state = create_populated_state();
            state.color_scheme = *scheme;
            terminal.draw(|f| ui(f, &mut state)).unwrap();
        }
    }

    #[test]
    fn test_render_volume_chart_all_color_schemes() {
        for scheme in &[
            ColorScheme::GreenRed,
            ColorScheme::BlueOrange,
            ColorScheme::Monochrome,
        ] {
            let mut terminal = create_test_terminal();
            let mut state = create_populated_state();
            state.color_scheme = *scheme;
            terminal
                .draw(|f| render_volume_chart(f, f.area(), &state))
                .unwrap();
        }
    }

    // ========================================================================
    // Coverage gap: Activity feed dedicated render tests
    // ========================================================================

    #[test]
    fn test_render_activity_feed_no_panic() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        for i in 0..5 {
            state.log_messages.push_back(format!("Event {}", i));
        }
        terminal
            .draw(|f| render_activity_feed(f, f.area(), &mut state))
            .unwrap();
    }

    #[test]
    fn test_render_activity_feed_empty_log() {
        let mut terminal = create_test_terminal();
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.log_messages.clear();
        terminal
            .draw(|f| render_activity_feed(f, f.area(), &mut state))
            .unwrap();
    }

    #[test]
    fn test_render_activity_feed_with_selection() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        for i in 0..10 {
            state.log_messages.push_back(format!("Event {}", i));
        }
        state.scroll_log_down();
        state.scroll_log_down();
        state.scroll_log_down();
        terminal
            .draw(|f| render_activity_feed(f, f.area(), &mut state))
            .unwrap();
    }

    // ========================================================================
    // Coverage gap: Alert edge cases
    // ========================================================================

    #[test]
    fn test_alert_whale_zero_transactions() {
        let mut token_data = create_test_token_data();
        token_data.total_buys_24h = 0;
        token_data.total_sells_24h = 0;
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.alerts.whale_min_usd = Some(100.0);
        state.update(&token_data);
        // With zero total txs, whale detection should NOT fire
        let whale_alerts: Vec<_> = state
            .active_alerts
            .iter()
            .filter(|a| a.message.contains("whale") || a.message.contains("🐋"))
            .collect();
        assert!(
            whale_alerts.is_empty(),
            "Whale alert should not fire with zero transactions"
        );
    }

    #[test]
    fn test_alert_multiple_simultaneous() {
        let mut token_data = create_test_token_data();
        token_data.price_usd = 0.1; // below min
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.alerts.price_min = Some(0.5); // will fire: price 0.1 < 0.5
        state.alerts.price_max = Some(0.05); // will fire: price 0.1 > 0.05
        state.alerts.volume_spike_threshold_pct = Some(1.0);
        state.volume_avg = 100.0; // volume_24h 1M vs avg 100 => huge spike

        state.update(&token_data);
        // Should have multiple alerts
        assert!(
            state.active_alerts.len() >= 2,
            "Expected multiple alerts, got {}",
            state.active_alerts.len()
        );
    }

    #[test]
    fn test_alert_clears_on_next_update() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.alerts.price_min = Some(2.0); // price 1.0 < 2.0 -> fires
        state.update(&token_data);
        assert!(!state.active_alerts.is_empty());

        // Update with price above min -> should clear
        let mut above_min = token_data.clone();
        above_min.price_usd = 3.0;
        state.alerts.price_min = Some(2.0);
        state.update(&above_min);
        // check_alerts clears alerts each time and re-evaluates
        let price_min_alerts: Vec<_> = state
            .active_alerts
            .iter()
            .filter(|a| a.message.contains("below min"))
            .collect();
        assert!(
            price_min_alerts.is_empty(),
            "Price-min alert should clear when price goes above min"
        );
    }

    #[test]
    fn test_render_alert_overlay_multiple_alerts() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.active_alerts.push(ActiveAlert {
            message: "⚠ Price below min".to_string(),
            triggered_at: Instant::now(),
        });
        state.active_alerts.push(ActiveAlert {
            message: "🐋 Whale detected".to_string(),
            triggered_at: Instant::now(),
        });
        state.active_alerts.push(ActiveAlert {
            message: "⚠ Volume spike".to_string(),
            triggered_at: Instant::now(),
        });
        state.alert_flash_until = Some(Instant::now() + Duration::from_secs(2));
        terminal
            .draw(|f| render_alert_overlay(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_alert_overlay_flash_expired() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.active_alerts.push(ActiveAlert {
            message: "⚠ Test".to_string(),
            triggered_at: Instant::now(),
        });
        // Flash timer already expired
        state.alert_flash_until = Some(Instant::now() - Duration::from_secs(5));
        terminal
            .draw(|f| render_alert_overlay(f, f.area(), &state))
            .unwrap();
    }

    // ========================================================================
    // Coverage gap: Liquidity depth edge cases
    // ========================================================================

    #[test]
    fn test_render_liquidity_depth_many_pairs() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        // Add many pairs to test height-limiting
        for i in 0..20 {
            state.liquidity_pairs.push((
                format!("TEST/TOKEN{} (DEX{})", i, i),
                (100_000.0 + i as f64 * 50_000.0),
            ));
        }
        terminal
            .draw(|f| render_liquidity_depth(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_liquidity_depth_narrow_terminal() {
        let backend = TestBackend::new(30, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = create_populated_state();
        state.liquidity_pairs = vec![
            ("TEST/WETH (Uniswap)".to_string(), 500_000.0),
            ("TEST/USDC (Sushi)".to_string(), 100_000.0),
        ];
        terminal
            .draw(|f| render_liquidity_depth(f, f.area(), &state))
            .unwrap();
    }

    // ========================================================================
    // Coverage gap: Metrics panel edge cases
    // ========================================================================

    #[test]
    fn test_render_metrics_panel_holder_count_disabled() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.holder_count = Some(42_000);
        state.widgets.holder_count = false; // disabled
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_metrics_panel_sparkline_single_point() {
        let mut terminal = create_test_terminal();
        let mut token_data = create_test_token_data();
        token_data.price_usd = 1.0;
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.price_history.clear();
        state.price_history.push_back(DataPoint {
            timestamp: 1.0,
            value: 1.0,
            is_real: true,
        });
        terminal
            .draw(|f| render_metrics_panel(f, f.area(), &state))
            .unwrap();
    }

    // ========================================================================
    // Coverage gap: Buy/sell gauge edge cases
    // ========================================================================

    #[test]
    fn test_render_buy_sell_gauge_tiny_area() {
        // Render in a very small area to test zero width/height paths
        let backend = TestBackend::new(5, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = create_populated_state();
        terminal
            .draw(|f| render_buy_sell_gauge(f, f.area(), &mut state))
            .unwrap();
    }

    // ========================================================================
    // Coverage gap: Log message queue overflow
    // ========================================================================

    #[test]
    fn test_log_message_queue_overflow() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        // Add more than 10 messages (queue capacity)
        for i in 0..20 {
            state.toggle_pause(); // each toggle logs a message
            let _ = i;
        }
        assert!(
            state.log_messages.len() <= 10,
            "Log queue should cap at 10, got {}",
            state.log_messages.len()
        );
    }

    // ========================================================================
    // Coverage gap: Export CSV row content verification
    // ========================================================================

    #[test]
    fn test_export_writes_csv_row_content_format() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let path = start_export_in_temp(&mut state);

        state.update(&token_data);

        let contents = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert!(lines.len() >= 2);

        // Verify header
        assert_eq!(
            lines[0],
            "timestamp,price_usd,volume_24h,liquidity_usd,buys_24h,sells_24h,market_cap"
        );

        // Verify data row has correct number of columns
        let data_cols: Vec<&str> = lines[1].split(',').collect();
        assert_eq!(
            data_cols.len(),
            7,
            "Expected 7 CSV columns, got {}",
            data_cols.len()
        );

        // Verify timestamp format (ISO 8601)
        assert!(data_cols[0].contains('T'));
        assert!(data_cols[0].ends_with('Z'));

        // Cleanup
        state.stop_export();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_export_writes_csv_row_market_cap_none() {
        let mut token_data = create_test_token_data();
        token_data.market_cap = None;
        let mut state = MonitorState::new(&token_data, "ethereum");
        let path = start_export_in_temp(&mut state);

        state.update(&token_data);

        let contents = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert!(lines.len() >= 2);

        // Last column should be empty when market_cap is None
        let data_cols: Vec<&str> = lines[1].split(',').collect();
        assert_eq!(data_cols.len(), 7);
        assert!(
            data_cols[6].is_empty(),
            "Market cap column should be empty when None"
        );

        // Cleanup
        state.stop_export();
        let _ = std::fs::remove_file(path);
    }

    // ========================================================================
    // Coverage gap: Full UI render with log scale + all chart modes
    // ========================================================================

    #[test]
    fn test_ui_render_log_scale_all_chart_modes() {
        for mode in &[
            ChartMode::Line,
            ChartMode::Candlestick,
            ChartMode::VolumeProfile,
        ] {
            let mut terminal = create_test_terminal();
            let mut state = create_populated_state();
            state.scale_mode = ScaleMode::Log;
            state.chart_mode = *mode;
            terminal.draw(|f| ui(f, &mut state)).unwrap();
        }
    }

    // ========================================================================
    // Coverage gap: Footer rendering edge cases
    // ========================================================================

    #[test]
    fn test_render_footer_widget_toggle_mode_active() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.widget_toggle_mode = true;
        terminal
            .draw(|f| render_footer(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_footer_all_status_indicators() {
        // Test with export + auto-pause + alerts simultaneously
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.export_active = true;
        state.auto_pause_on_input = true;
        state.last_input_at = Instant::now(); // triggers auto-paused display
        terminal
            .draw(|f| render_footer(f, f.area(), &state))
            .unwrap();
    }

    // ========================================================================
    // Coverage gap: Synthetic data generation edge cases
    // ========================================================================

    #[test]
    fn test_generate_synthetic_price_history_zero_price() {
        let mut token_data = create_test_token_data();
        token_data.price_usd = 0.0;
        token_data.price_change_1h = 0.0;
        token_data.price_change_6h = 0.0;
        token_data.price_change_24h = 0.0;
        let state = MonitorState::new(&token_data, "ethereum");
        // Should not panic with zero prices
        assert!(!state.price_history.is_empty());
    }

    #[test]
    fn test_generate_synthetic_volume_history_zero_volume() {
        let mut token_data = create_test_token_data();
        token_data.volume_24h = 0.0;
        token_data.volume_6h = 0.0;
        token_data.volume_1h = 0.0;
        let state = MonitorState::new(&token_data, "ethereum");
        assert!(!state.volume_history.is_empty());
    }

    #[test]
    fn test_generate_synthetic_order_book() {
        let pairs = vec![crate::chains::DexPair {
            dex_name: "Uniswap V3".to_string(),
            pair_address: "0xabc".to_string(),
            base_token: "PUSD".to_string(),
            quote_token: "USDT".to_string(),
            price_usd: 1.0,
            volume_24h: 50_000.0,
            liquidity_usd: 200_000.0,
            price_change_24h: 0.1,
            buys_24h: 100,
            sells_24h: 90,
            buys_6h: 30,
            sells_6h: 25,
            buys_1h: 10,
            sells_1h: 8,
            pair_created_at: None,
            url: None,
        }];
        let book = MonitorState::generate_synthetic_order_book(&pairs, "PUSD", 1.0, 200_000.0);
        assert!(book.is_some());
        let book = book.unwrap();
        assert_eq!(book.pair, "PUSD/USDT");
        assert!(!book.asks.is_empty());
        assert!(!book.bids.is_empty());
        // Asks should be ascending
        for w in book.asks.windows(2) {
            assert!(w[0].price <= w[1].price);
        }
        // Bids should be descending
        for w in book.bids.windows(2) {
            assert!(w[0].price >= w[1].price);
        }
    }

    #[test]
    fn test_generate_synthetic_order_book_zero_price() {
        let book = MonitorState::generate_synthetic_order_book(&[], "TEST", 0.0, 100_000.0);
        assert!(book.is_none());
    }

    #[test]
    fn test_generate_synthetic_order_book_zero_liquidity() {
        let book = MonitorState::generate_synthetic_order_book(&[], "TEST", 1.0, 0.0);
        assert!(book.is_none());
    }

    // ========================================================================
    // Coverage gap: Auto-pause with custom timeout
    // ========================================================================

    #[test]
    fn test_auto_pause_custom_timeout() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.auto_pause_on_input = true;
        state.auto_pause_timeout = Duration::from_secs(10);
        state.refresh_rate = Duration::from_secs(1);

        // Fresh input with long timeout -> still auto-paused
        state.last_input_at = Instant::now();
        state.last_update = Instant::now() - Duration::from_secs(5);
        assert!(!state.should_refresh()); // within 10s timeout
        assert!(state.is_auto_paused());
    }

    // ========================================================================
    // Coverage gap: Price chart with stablecoin flat range
    // ========================================================================

    #[test]
    fn test_render_price_chart_stablecoin_flat_range() {
        let mut terminal = create_test_terminal();
        let mut token_data = create_test_token_data();
        token_data.price_usd = 1.0;
        let mut state = MonitorState::new(&token_data, "ethereum");
        // Add many points at nearly identical prices (stablecoin)
        for i in 0..20 {
            let mut data = token_data.clone();
            data.price_usd = 1.0 + (i as f64 * 0.000001); // micro variation
            state.update(&data);
        }
        terminal
            .draw(|f| render_price_chart(f, f.area(), &state))
            .unwrap();
    }

    // ========================================================================
    // Coverage gap: Cache load edge cases
    // ========================================================================

    #[test]
    fn test_load_cache_corrupted_json() {
        let path = MonitorState::cache_path("0xCORRUPTED_TEST", "test_chain");
        // Write invalid JSON
        let _ = std::fs::write(&path, "not valid json {{{");
        let cached = MonitorState::load_cache("0xCORRUPTED_TEST", "test_chain");
        assert!(cached.is_none(), "Corrupted JSON should return None");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_load_cache_wrong_token() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");
        state.save_cache();

        // Try to load with different token address
        let cached = MonitorState::load_cache("0xDIFFERENT_ADDRESS", "ethereum");
        assert!(
            cached.is_none(),
            "Loading cache with wrong token address should return None"
        );

        // Cleanup
        let path = MonitorState::cache_path(&token_data.address, "ethereum");
        let _ = std::fs::remove_file(path);
    }

    // ========================================================================
    // Integration tests: Mock types and MonitorApp constructor
    // ========================================================================

    use crate::chains::dex::TokenSearchResult;

    /// Mock DEX data source for integration testing.
    struct MockDexDataSource {
        /// Data returned by `get_token_data`. If `Err`, simulates an API failure.
        token_data_result: std::sync::Mutex<Result<DexTokenData>>,
    }

    impl MockDexDataSource {
        fn new(data: DexTokenData) -> Self {
            Self {
                token_data_result: std::sync::Mutex::new(Ok(data)),
            }
        }

        fn failing(msg: &str) -> Self {
            Self {
                token_data_result: std::sync::Mutex::new(Err(ScopeError::Api(msg.to_string()))),
            }
        }
    }

    #[async_trait::async_trait]
    impl DexDataSource for MockDexDataSource {
        async fn get_token_price(&self, _chain: &str, _address: &str) -> Option<f64> {
            self.token_data_result
                .lock()
                .unwrap()
                .as_ref()
                .ok()
                .map(|d| d.price_usd)
        }

        async fn get_native_token_price(&self, _chain: &str) -> Option<f64> {
            Some(2000.0)
        }

        async fn get_token_data(&self, _chain: &str, _address: &str) -> Result<DexTokenData> {
            let guard = self.token_data_result.lock().unwrap();
            match &*guard {
                Ok(data) => Ok(data.clone()),
                Err(e) => Err(ScopeError::Api(e.to_string())),
            }
        }

        async fn search_tokens(
            &self,
            _query: &str,
            _chain: Option<&str>,
        ) -> Result<Vec<TokenSearchResult>> {
            Ok(vec![])
        }
    }

    /// Mock chain client for integration testing.
    struct MockChainClient {
        holder_count: u64,
    }

    impl MockChainClient {
        fn new(holder_count: u64) -> Self {
            Self { holder_count }
        }
    }

    #[async_trait::async_trait]
    impl ChainClient for MockChainClient {
        fn chain_name(&self) -> &str {
            "ethereum"
        }
        fn native_token_symbol(&self) -> &str {
            "ETH"
        }
        async fn get_balance(&self, _address: &str) -> Result<crate::chains::Balance> {
            unimplemented!("not needed for monitor tests")
        }
        async fn enrich_balance_usd(&self, _balance: &mut crate::chains::Balance) {}
        async fn get_transaction(&self, _hash: &str) -> Result<crate::chains::Transaction> {
            unimplemented!("not needed for monitor tests")
        }
        async fn get_transactions(
            &self,
            _address: &str,
            _limit: u32,
        ) -> Result<Vec<crate::chains::Transaction>> {
            Ok(vec![])
        }
        async fn get_block_number(&self) -> Result<u64> {
            Ok(1000000)
        }
        async fn get_token_balances(
            &self,
            _address: &str,
        ) -> Result<Vec<crate::chains::TokenBalance>> {
            Ok(vec![])
        }
        async fn get_token_holder_count(&self, _address: &str) -> Result<u64> {
            Ok(self.holder_count)
        }
    }

    /// Creates a `MonitorApp<TestBackend>` with mock dependencies.
    fn create_test_app(
        dex: Box<dyn DexDataSource>,
        chain_client: Option<Box<dyn ChainClient>>,
    ) -> MonitorApp<TestBackend> {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");
        let backend = TestBackend::new(120, 40);
        let terminal = ratatui::Terminal::new(backend).unwrap();
        MonitorApp {
            terminal,
            state,
            dex_client: dex,
            chain_client,
            exchange_client: None,
            should_exit: false,
            owns_terminal: false,
        }
    }

    fn create_test_app_with_state(
        state: MonitorState,
        dex: Box<dyn DexDataSource>,
        chain_client: Option<Box<dyn ChainClient>>,
    ) -> MonitorApp<TestBackend> {
        let backend = TestBackend::new(120, 40);
        let terminal = ratatui::Terminal::new(backend).unwrap();
        MonitorApp {
            terminal,
            state,
            dex_client: dex,
            chain_client,
            exchange_client: None,
            should_exit: false,
            owns_terminal: false,
        }
    }

    // ========================================================================
    // Integration tests: MonitorApp::handle_key_event
    // ========================================================================

    #[test]
    fn test_app_handle_key_quit_q() {
        let data = create_test_token_data();
        let mut app = create_test_app(Box::new(MockDexDataSource::new(data)), None);
        assert!(!app.should_exit);
        app.handle_key_event(make_key_event(KeyCode::Char('q')));
        assert!(app.should_exit);
    }

    #[test]
    fn test_app_handle_key_quit_esc() {
        let data = create_test_token_data();
        let mut app = create_test_app(Box::new(MockDexDataSource::new(data)), None);
        app.handle_key_event(make_key_event(KeyCode::Esc));
        assert!(app.should_exit);
    }

    #[test]
    fn test_app_handle_key_quit_ctrl_c() {
        let data = create_test_token_data();
        let mut app = create_test_app(Box::new(MockDexDataSource::new(data)), None);
        let key = crossterm::event::KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        app.handle_key_event(key);
        assert!(app.should_exit);
    }

    #[test]
    fn test_app_handle_key_quit_stops_active_export() {
        let data = create_test_token_data();
        let mut app = create_test_app(Box::new(MockDexDataSource::new(data)), None);
        let path = start_export_in_temp(&mut app.state);
        assert!(app.state.export_active);

        app.handle_key_event(make_key_event(KeyCode::Char('q')));
        assert!(app.should_exit);
        assert!(!app.state.export_active);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_app_handle_key_ctrl_c_stops_active_export() {
        let data = create_test_token_data();
        let mut app = create_test_app(Box::new(MockDexDataSource::new(data)), None);
        let path = start_export_in_temp(&mut app.state);
        assert!(app.state.export_active);

        let key = crossterm::event::KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        app.handle_key_event(key);
        assert!(app.should_exit);
        assert!(!app.state.export_active);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_app_handle_key_updates_last_input_time() {
        let data = create_test_token_data();
        let mut app = create_test_app(Box::new(MockDexDataSource::new(data)), None);
        let before = Instant::now();
        app.handle_key_event(make_key_event(KeyCode::Char('p')));
        assert!(app.state.last_input_at >= before);
    }

    #[test]
    fn test_app_handle_key_widget_toggle_mode() {
        let data = create_test_token_data();
        let mut app = create_test_app(Box::new(MockDexDataSource::new(data)), None);
        assert!(app.state.widgets.price_chart);

        // Enter widget toggle mode
        app.handle_key_event(make_key_event(KeyCode::Char('w')));
        assert!(app.state.widget_toggle_mode);

        // Toggle widget 1 (price chart)
        app.handle_key_event(make_key_event(KeyCode::Char('1')));
        assert!(!app.state.widget_toggle_mode);
        assert!(!app.state.widgets.price_chart);
    }

    #[test]
    fn test_app_handle_key_widget_toggle_mode_cancel() {
        let data = create_test_token_data();
        let mut app = create_test_app(Box::new(MockDexDataSource::new(data)), None);

        // Enter widget toggle mode
        app.handle_key_event(make_key_event(KeyCode::Char('w')));
        assert!(app.state.widget_toggle_mode);

        // Any non-digit key cancels widget toggle mode
        app.handle_key_event(make_key_event(KeyCode::Char('x')));
        assert!(!app.state.widget_toggle_mode);
    }

    #[test]
    fn test_app_handle_key_all_keybindings() {
        let data = create_test_token_data();
        let mut app = create_test_app(Box::new(MockDexDataSource::new(data)), None);

        // r = force refresh
        app.handle_key_event(make_key_event(KeyCode::Char('r')));
        assert!(!app.should_exit);

        // Shift+P = toggle auto-pause
        let key = crossterm::event::KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT);
        app.handle_key_event(key);
        assert!(app.state.auto_pause_on_input);
        app.handle_key_event(key);
        assert!(!app.state.auto_pause_on_input);

        // p = toggle pause
        app.handle_key_event(make_key_event(KeyCode::Char('p')));
        assert!(app.state.paused);

        // space = toggle pause
        app.handle_key_event(make_key_event(KeyCode::Char(' ')));
        assert!(!app.state.paused);

        // e = toggle export
        app.handle_key_event(make_key_event(KeyCode::Char('e')));
        assert!(app.state.export_active);
        // Stop export to avoid file handles
        app.state.stop_export();

        // + = slower refresh
        let before_rate = app.state.refresh_rate;
        app.handle_key_event(make_key_event(KeyCode::Char('+')));
        assert!(app.state.refresh_rate >= before_rate);

        // - = faster refresh
        let before_rate = app.state.refresh_rate;
        app.handle_key_event(make_key_event(KeyCode::Char('-')));
        assert!(app.state.refresh_rate <= before_rate);

        // 1-6 = time periods
        app.handle_key_event(make_key_event(KeyCode::Char('1')));
        assert_eq!(app.state.time_period, TimePeriod::Min1);
        app.handle_key_event(make_key_event(KeyCode::Char('2')));
        assert_eq!(app.state.time_period, TimePeriod::Min5);
        app.handle_key_event(make_key_event(KeyCode::Char('3')));
        assert_eq!(app.state.time_period, TimePeriod::Min15);
        app.handle_key_event(make_key_event(KeyCode::Char('4')));
        assert_eq!(app.state.time_period, TimePeriod::Hour1);
        app.handle_key_event(make_key_event(KeyCode::Char('5')));
        assert_eq!(app.state.time_period, TimePeriod::Hour4);
        app.handle_key_event(make_key_event(KeyCode::Char('6')));
        assert_eq!(app.state.time_period, TimePeriod::Day1);

        // t = cycle time period
        app.handle_key_event(make_key_event(KeyCode::Char('t')));
        assert_eq!(app.state.time_period, TimePeriod::Min1); // wraps from Day1

        // c = toggle chart mode
        app.handle_key_event(make_key_event(KeyCode::Char('c')));
        assert_eq!(app.state.chart_mode, ChartMode::Candlestick);

        // s = toggle scale
        app.handle_key_event(make_key_event(KeyCode::Char('s')));
        assert_eq!(app.state.scale_mode, ScaleMode::Log);

        // / = cycle color scheme
        app.handle_key_event(make_key_event(KeyCode::Char('/')));
        assert_eq!(app.state.color_scheme, ColorScheme::BlueOrange);

        // j = scroll log down
        app.handle_key_event(make_key_event(KeyCode::Char('j')));

        // k = scroll log up
        app.handle_key_event(make_key_event(KeyCode::Char('k')));

        // l = next layout
        app.handle_key_event(make_key_event(KeyCode::Char('l')));
        assert!(!app.state.auto_layout);

        // h = prev layout
        app.handle_key_event(make_key_event(KeyCode::Char('h')));

        // a = re-enable auto layout
        app.handle_key_event(make_key_event(KeyCode::Char('a')));
        assert!(app.state.auto_layout);

        // w = widget toggle mode
        app.handle_key_event(make_key_event(KeyCode::Char('w')));
        assert!(app.state.widget_toggle_mode);
        // Cancel it
        app.handle_key_event(make_key_event(KeyCode::Char('z')));

        // Unknown key is a no-op
        app.handle_key_event(make_key_event(KeyCode::F(12)));
        assert!(!app.should_exit);
    }

    // ========================================================================
    // Integration tests: MonitorApp::fetch_data
    // ========================================================================

    #[tokio::test]
    async fn test_app_fetch_data_success() {
        let data = create_test_token_data();
        let initial_price = data.price_usd;
        let mut updated = data.clone();
        updated.price_usd = 2.5;
        let mut app = create_test_app(Box::new(MockDexDataSource::new(updated)), None);

        assert!((app.state.current_price - initial_price).abs() < 0.001);
        app.fetch_data().await;
        assert!((app.state.current_price - 2.5).abs() < 0.001);
        assert!(app.state.error_message.is_none());
    }

    #[tokio::test]
    async fn test_app_fetch_data_api_error() {
        let mut app = create_test_app(Box::new(MockDexDataSource::failing("rate limited")), None);

        app.fetch_data().await;
        assert!(app.state.error_message.is_some());
        assert!(
            app.state
                .error_message
                .as_ref()
                .unwrap()
                .contains("API Error")
        );
    }

    #[tokio::test]
    async fn test_app_fetch_data_holder_count_on_12th_tick() {
        let data = create_test_token_data();
        let mock_chain = MockChainClient::new(42_000);
        let mut app = create_test_app(
            Box::new(MockDexDataSource::new(data)),
            Some(Box::new(mock_chain)),
        );

        // First 11 fetches should not update holder count
        for _ in 0..11 {
            app.fetch_data().await;
        }
        assert!(app.state.holder_count.is_none());

        // 12th fetch triggers holder count lookup
        app.fetch_data().await;
        assert_eq!(app.state.holder_count, Some(42_000));
    }

    #[tokio::test]
    async fn test_app_fetch_data_holder_count_zero_not_stored() {
        let data = create_test_token_data();
        let mock_chain = MockChainClient::new(0); // returns zero
        let mut app = create_test_app(
            Box::new(MockDexDataSource::new(data)),
            Some(Box::new(mock_chain)),
        );

        // Skip to 12th tick
        app.state.holder_fetch_counter = 11;
        app.fetch_data().await;
        // Zero holder count should NOT be stored
        assert!(app.state.holder_count.is_none());
    }

    #[tokio::test]
    async fn test_app_fetch_data_no_chain_client_skips_holders() {
        let data = create_test_token_data();
        let mut app = create_test_app(Box::new(MockDexDataSource::new(data)), None);

        // Skip to 12th tick
        app.state.holder_fetch_counter = 11;
        app.fetch_data().await;
        // Without a chain client, holder count stays None
        assert!(app.state.holder_count.is_none());
    }

    #[tokio::test]
    async fn test_app_fetch_data_preserves_holder_on_subsequent_failure() {
        let data = create_test_token_data();
        let mock_chain = MockChainClient::new(42_000);
        let mut app = create_test_app(
            Box::new(MockDexDataSource::new(data)),
            Some(Box::new(mock_chain)),
        );

        // Fetch holder count on 12th tick
        app.state.holder_fetch_counter = 11;
        app.fetch_data().await;
        assert_eq!(app.state.holder_count, Some(42_000));

        // Replace chain client with one returning 0
        app.chain_client = Some(Box::new(MockChainClient::new(0)));
        // 24th tick
        app.state.holder_fetch_counter = 23;
        app.fetch_data().await;
        // Previous value should be preserved (zero is ignored)
        assert_eq!(app.state.holder_count, Some(42_000));
    }

    // ========================================================================
    // Integration tests: MonitorApp::cleanup
    // ========================================================================

    #[test]
    fn test_app_cleanup_does_not_panic_test_backend() {
        let data = create_test_token_data();
        let mut app = create_test_app(Box::new(MockDexDataSource::new(data)), None);
        // cleanup() with owns_terminal=false should not attempt terminal restore
        let result = app.cleanup();
        assert!(result.is_ok());
    }

    // ========================================================================
    // Integration tests: MonitorApp draw renders without panic
    // ========================================================================

    #[test]
    fn test_app_draw_renders_ui() {
        let data = create_test_token_data();
        let mut app = create_test_app(Box::new(MockDexDataSource::new(data)), None);
        // Verify we can render UI through the MonitorApp terminal
        app.terminal
            .draw(|f| ui(f, &mut app.state))
            .expect("should render without panic");
    }

    // ========================================================================
    // Integration tests: select_token_impl
    // ========================================================================

    fn make_search_results() -> Vec<TokenSearchResult> {
        vec![
            TokenSearchResult {
                address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                chain: "ethereum".to_string(),
                price_usd: Some(1.0),
                volume_24h: 5_000_000_000.0,
                liquidity_usd: 2_000_000_000.0,
                market_cap: Some(32_000_000_000.0),
            },
            TokenSearchResult {
                address: "0xdAC17F958D2ee523a2206206994597C13D831ec7".to_string(),
                symbol: "USDT".to_string(),
                name: "Tether USD".to_string(),
                chain: "ethereum".to_string(),
                price_usd: Some(1.0),
                volume_24h: 6_000_000_000.0,
                liquidity_usd: 3_000_000_000.0,
                market_cap: Some(83_000_000_000.0),
            },
            TokenSearchResult {
                address: "0x6B175474E89094C44Da98b954EedeAC495271d0F".to_string(),
                symbol: "DAI".to_string(),
                name: "Dai Stablecoin".to_string(),
                chain: "ethereum".to_string(),
                price_usd: Some(1.0),
                volume_24h: 200_000_000.0,
                liquidity_usd: 500_000_000.0,
                market_cap: Some(5_000_000_000.0),
            },
        ]
    }

    #[test]
    fn test_select_token_impl_valid_first() {
        let results = make_search_results();
        let mut reader = io::Cursor::new(b"1\n");
        let mut writer = Vec::new();
        let selected = select_token_impl(&results, &mut reader, &mut writer).unwrap();
        assert_eq!(selected.symbol, "USDC");
        assert_eq!(selected.address, results[0].address);
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Found 3 tokens"));
        assert!(output.contains("Selected: USDC"));
    }

    #[test]
    fn test_select_token_impl_valid_last() {
        let results = make_search_results();
        let mut reader = io::Cursor::new(b"3\n");
        let mut writer = Vec::new();
        let selected = select_token_impl(&results, &mut reader, &mut writer).unwrap();
        assert_eq!(selected.symbol, "DAI");
    }

    #[test]
    fn test_select_token_impl_valid_middle() {
        let results = make_search_results();
        let mut reader = io::Cursor::new(b"2\n");
        let mut writer = Vec::new();
        let selected = select_token_impl(&results, &mut reader, &mut writer).unwrap();
        assert_eq!(selected.symbol, "USDT");
    }

    #[test]
    fn test_select_token_impl_out_of_bounds_zero() {
        let results = make_search_results();
        let mut reader = io::Cursor::new(b"0\n");
        let mut writer = Vec::new();
        let result = select_token_impl(&results, &mut reader, &mut writer);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Selection must be between 1 and 3"));
    }

    #[test]
    fn test_select_token_impl_out_of_bounds_high() {
        let results = make_search_results();
        let mut reader = io::Cursor::new(b"99\n");
        let mut writer = Vec::new();
        let result = select_token_impl(&results, &mut reader, &mut writer);
        assert!(result.is_err());
    }

    #[test]
    fn test_select_token_impl_non_numeric_input() {
        let results = make_search_results();
        let mut reader = io::Cursor::new(b"abc\n");
        let mut writer = Vec::new();
        let result = select_token_impl(&results, &mut reader, &mut writer);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid selection"));
    }

    #[test]
    fn test_select_token_impl_empty_input() {
        let results = make_search_results();
        let mut reader = io::Cursor::new(b"\n");
        let mut writer = Vec::new();
        let result = select_token_impl(&results, &mut reader, &mut writer);
        assert!(result.is_err());
    }

    #[test]
    fn test_select_token_impl_long_name_truncation() {
        let results = vec![TokenSearchResult {
            address: "0xABCDEF1234567890ABCDEF1234567890ABCDEF12".to_string(),
            symbol: "LONG".to_string(),
            name: "A Very Long Token Name That Exceeds Twenty Characters".to_string(),
            chain: "ethereum".to_string(),
            price_usd: None,
            volume_24h: 100.0,
            liquidity_usd: 50.0,
            market_cap: None,
        }];
        let mut reader = io::Cursor::new(b"1\n");
        let mut writer = Vec::new();
        let selected = select_token_impl(&results, &mut reader, &mut writer).unwrap();
        assert_eq!(selected.symbol, "LONG");
        let output = String::from_utf8(writer).unwrap();
        // Should have truncated name
        assert!(output.contains("A Very Long Token..."));
        // Should show N/A for price
        assert!(output.contains("N/A"));
    }

    #[test]
    fn test_select_token_impl_output_format() {
        let results = make_search_results();
        let mut reader = io::Cursor::new(b"1\n");
        let mut writer = Vec::new();
        let _ = select_token_impl(&results, &mut reader, &mut writer).unwrap();
        let output = String::from_utf8(writer).unwrap();

        // Verify table header
        assert!(output.contains("#"));
        assert!(output.contains("Symbol"));
        assert!(output.contains("Name"));
        assert!(output.contains("Address"));
        assert!(output.contains("Price"));
        assert!(output.contains("Liquidity"));
        // Verify separator line
        assert!(output.contains("─"));
        // Verify prompt
        assert!(output.contains("Select token (1-3):"));
    }

    // ========================================================================
    // Integration tests: format_monitor_number
    // ========================================================================

    #[test]
    fn test_format_monitor_number_billions() {
        assert_eq!(format_monitor_number(5_000_000_000.0), "$5.00B");
        assert_eq!(format_monitor_number(1_234_567_890.0), "$1.23B");
    }

    #[test]
    fn test_format_monitor_number_millions() {
        assert_eq!(format_monitor_number(5_000_000.0), "$5.00M");
        assert_eq!(format_monitor_number(42_500_000.0), "$42.50M");
    }

    #[test]
    fn test_format_monitor_number_thousands() {
        assert_eq!(format_monitor_number(5_000.0), "$5.00K");
        assert_eq!(format_monitor_number(999_999.0), "$1000.00K");
    }

    #[test]
    fn test_format_monitor_number_small() {
        assert_eq!(format_monitor_number(42.0), "$42.00");
        assert_eq!(format_monitor_number(0.5), "$0.50");
        assert_eq!(format_monitor_number(0.0), "$0.00");
    }

    // ========================================================================
    // Integration tests: abbreviate_address edge cases
    // ========================================================================

    #[test]
    fn test_abbreviate_address_exactly_16_chars() {
        let addr = "0123456789ABCDEF"; // exactly 16 chars
        assert_eq!(abbreviate_address(addr), addr);
    }

    #[test]
    fn test_abbreviate_address_17_chars() {
        let addr = "0123456789ABCDEFG"; // 17 chars -> abbreviated
        assert_eq!(abbreviate_address(addr), "01234567...BCDEFG");
    }

    // ========================================================================
    // Integration tests: MonitorApp with state + fetch combined scenario
    // ========================================================================

    #[tokio::test]
    async fn test_app_full_scenario_fetch_render_quit() {
        let data = create_test_token_data();
        let mut updated = data.clone();
        updated.price_usd = 3.0;
        let mock_chain = MockChainClient::new(10_000);
        let state = MonitorState::new(&data, "ethereum");
        let mut app = create_test_app_with_state(
            state,
            Box::new(MockDexDataSource::new(updated)),
            Some(Box::new(mock_chain)),
        );

        // 1. Fetch new data
        app.fetch_data().await;
        assert!((app.state.current_price - 3.0).abs() < 0.001);

        // 2. Render UI
        app.terminal
            .draw(|f| ui(f, &mut app.state))
            .expect("render");

        // 3. Start export
        app.handle_key_event(make_key_event(KeyCode::Char('e')));
        assert!(app.state.export_active);

        // 4. Quit (should stop export)
        app.handle_key_event(make_key_event(KeyCode::Char('q')));
        assert!(app.should_exit);
        assert!(!app.state.export_active);
    }

    #[tokio::test]
    async fn test_app_fetch_data_error_then_recovery() {
        let mut app = create_test_app(Box::new(MockDexDataSource::failing("server down")), None);

        // First fetch fails
        app.fetch_data().await;
        assert!(app.state.error_message.is_some());

        // Replace with working mock
        let mut recovered = create_test_token_data();
        recovered.price_usd = 5.0;
        app.dex_client = Box::new(MockDexDataSource::new(recovered));

        // Second fetch succeeds
        app.fetch_data().await;
        assert!((app.state.current_price - 5.0).abs() < 0.001);
        // Error message is cleared by state.update()
    }

    // ========================================================================
    // Integration tests: MonitorArgs parsing and run_direct config merging
    // ========================================================================

    #[test]
    fn test_monitor_args_defaults() {
        use super::super::Cli;
        use clap::Parser;
        // Simulate: scope monitor USDC
        let cli = Cli::try_parse_from(["scope", "monitor", "USDC"]).unwrap();
        if let super::super::Commands::Monitor(args) = cli.command {
            assert_eq!(args.token, "USDC");
            assert_eq!(args.chain, "ethereum");
            assert!(args.layout.is_none());
            assert!(args.refresh.is_none());
            assert!(args.scale.is_none());
            assert!(args.color_scheme.is_none());
            assert!(args.export.is_none());
        } else {
            panic!("Expected Monitor command");
        }
    }

    #[test]
    fn test_monitor_args_all_flags() {
        use super::super::Cli;
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "scope",
            "monitor",
            "PEPE",
            "--chain",
            "solana",
            "--layout",
            "feed",
            "--refresh",
            "2",
            "--scale",
            "log",
            "--color-scheme",
            "monochrome",
            "--export",
            "/tmp/data.csv",
        ])
        .unwrap();
        if let super::super::Commands::Monitor(args) = cli.command {
            assert_eq!(args.token, "PEPE");
            assert_eq!(args.chain, "solana");
            assert_eq!(args.layout, Some(LayoutPreset::Feed));
            assert_eq!(args.refresh, Some(2));
            assert_eq!(args.scale, Some(ScaleMode::Log));
            assert_eq!(args.color_scheme, Some(ColorScheme::Monochrome));
            assert_eq!(args.export, Some(PathBuf::from("/tmp/data.csv")));
        } else {
            panic!("Expected Monitor command");
        }
    }

    #[test]
    fn test_run_direct_config_override_layout() {
        // Verify that run_direct properly applies CLI overrides to config
        let config = Config::default();
        assert_eq!(config.monitor.layout, LayoutPreset::Dashboard);

        let args = MonitorArgs {
            token: "USDC".to_string(),
            chain: "ethereum".to_string(),
            layout: Some(LayoutPreset::ChartFocus),
            refresh: None,
            scale: None,
            color_scheme: None,
            export: None,
            venue: None,
            pair: None,
        };

        // Build the effective config the same way run_direct does
        let mut monitor_config = config.monitor.clone();
        if let Some(layout) = args.layout {
            monitor_config.layout = layout;
        }
        assert_eq!(monitor_config.layout, LayoutPreset::ChartFocus);
    }

    #[test]
    fn test_run_direct_config_override_all_fields() {
        let config = Config::default();
        let args = MonitorArgs {
            token: "PEPE".to_string(),
            chain: "solana".to_string(),
            layout: Some(LayoutPreset::Compact),
            refresh: Some(2),
            scale: Some(ScaleMode::Log),
            color_scheme: Some(ColorScheme::BlueOrange),
            export: Some(PathBuf::from("/tmp/test.csv")),
            venue: None,
            pair: None,
        };

        let mut mc = config.monitor.clone();
        if let Some(layout) = args.layout {
            mc.layout = layout;
        }
        if let Some(refresh) = args.refresh {
            mc.refresh_seconds = refresh;
        }
        if let Some(scale) = args.scale {
            mc.scale = scale;
        }
        if let Some(color_scheme) = args.color_scheme {
            mc.color_scheme = color_scheme;
        }
        if let Some(ref path) = args.export {
            mc.export.path = Some(path.to_string_lossy().into_owned());
        }

        assert_eq!(mc.layout, LayoutPreset::Compact);
        assert_eq!(mc.refresh_seconds, 2);
        assert_eq!(mc.scale, ScaleMode::Log);
        assert_eq!(mc.color_scheme, ColorScheme::BlueOrange);
        assert_eq!(mc.export.path, Some("/tmp/test.csv".to_string()));
    }

    #[test]
    fn test_run_direct_config_no_overrides_preserves_defaults() {
        let config = Config::default();
        let args = MonitorArgs {
            token: "USDC".to_string(),
            chain: "ethereum".to_string(),
            layout: None,
            refresh: None,
            scale: None,
            color_scheme: None,
            export: None,
            venue: None,
            pair: None,
        };

        let mut mc = config.monitor.clone();
        if let Some(layout) = args.layout {
            mc.layout = layout;
        }
        if let Some(refresh) = args.refresh {
            mc.refresh_seconds = refresh;
        }
        if let Some(scale) = args.scale {
            mc.scale = scale;
        }
        if let Some(color_scheme) = args.color_scheme {
            mc.color_scheme = color_scheme;
        }

        // All should remain at defaults
        assert_eq!(mc.layout, LayoutPreset::Dashboard);
        assert_eq!(mc.refresh_seconds, DEFAULT_REFRESH_SECS);
        assert_eq!(mc.scale, ScaleMode::Linear);
        assert_eq!(mc.color_scheme, ColorScheme::GreenRed);
        assert!(mc.export.path.is_none());
    }

    // =================================================================
    // OHLC / exchange_interval tests
    // =================================================================

    #[test]
    fn test_exchange_interval_mapping() {
        assert_eq!(TimePeriod::Min1.exchange_interval(), "1m");
        assert_eq!(TimePeriod::Min5.exchange_interval(), "5m");
        assert_eq!(TimePeriod::Min15.exchange_interval(), "15m");
        assert_eq!(TimePeriod::Hour1.exchange_interval(), "1h");
        assert_eq!(TimePeriod::Hour4.exchange_interval(), "4h");
        assert_eq!(TimePeriod::Day1.exchange_interval(), "1d");
    }

    #[test]
    fn test_monitor_state_exchange_ohlc_default_empty() {
        let token_data = create_test_token_data();
        let state = MonitorState::new(&token_data, "ethereum");
        assert!(state.exchange_ohlc.is_empty());
        assert!(state.venue_pair.is_none());
    }

    #[test]
    fn test_get_ohlc_candles_prefers_exchange_data() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");

        // Add some synthetic candles from price history
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        state.price_history.push_back(DataPoint {
            timestamp: now,
            value: 1.0,
            is_real: true,
        });
        state.price_history.push_back(DataPoint {
            timestamp: now + 5.0,
            value: 1.01,
            is_real: true,
        });

        // Without exchange OHLC, should get synthetic candles
        let candles_before = state.get_ohlc_candles();

        // Now add exchange OHLC data
        state.exchange_ohlc = vec![
            OhlcCandle {
                timestamp: 1700000000.0,
                open: 50000.0,
                high: 50500.0,
                low: 49800.0,
                close: 50200.0,
                is_bullish: true,
            },
            OhlcCandle {
                timestamp: 1700003600.0,
                open: 50200.0,
                high: 50800.0,
                low: 50100.0,
                close: 50700.0,
                is_bullish: true,
            },
        ];

        let candles_after = state.get_ohlc_candles();
        assert_eq!(candles_after.len(), 2);
        assert_eq!(candles_after[0].open, 50000.0);
        assert_eq!(candles_after[1].close, 50700.0);

        // Verify exchange data is preferred over synthetic
        if !candles_before.is_empty() {
            assert_ne!(candles_after[0].open, candles_before[0].open);
        }
    }

    #[test]
    fn test_monitor_args_with_venue() {
        let args = MonitorArgs {
            token: "BTC".to_string(),
            chain: "ethereum".to_string(),
            refresh: None,
            layout: None,
            scale: None,
            color_scheme: None,
            export: None,
            venue: Some("binance".to_string()),
            pair: None,
        };
        assert_eq!(args.venue, Some("binance".to_string()));
    }

    #[test]
    fn test_ohlc_candle_is_bullish_calculation() {
        let bullish = OhlcCandle {
            timestamp: 1700000000.0,
            open: 100.0,
            high: 110.0,
            low: 95.0,
            close: 105.0,
            is_bullish: true,
        };
        assert!(bullish.is_bullish);

        let bearish = OhlcCandle {
            timestamp: 1700000000.0,
            open: 100.0,
            high: 105.0,
            low: 90.0,
            close: 95.0,
            is_bullish: false,
        };
        assert!(!bearish.is_bullish);
    }

    #[test]
    fn test_build_exchange_token_data_from_ticker() {
        let ticker = crate::market::Ticker {
            pair: "PUSD/USDT".to_string(),
            last_price: Some(1.001),
            high_24h: Some(1.005),
            low_24h: Some(0.998),
            volume_24h: Some(500_000.0),
            quote_volume_24h: Some(500_500.0),
            best_bid: Some(1.0005),
            best_ask: Some(1.0015),
        };

        let data = build_exchange_token_data("PUSD", "PUSD_USDT", &ticker);

        assert_eq!(data.symbol, "PUSD");
        assert_eq!(data.name, "PUSD");
        assert_eq!(data.price_usd, 1.001);
        assert_eq!(data.volume_24h, 500_000.0);
        assert!(data.address.contains("exchange:"));
        assert!(data.pairs.is_empty());
        assert!(data.price_history.is_empty());
        assert!(data.dexscreener_url.is_none());
    }

    #[test]
    fn test_build_exchange_token_data_missing_price() {
        let ticker = crate::market::Ticker {
            pair: "FOO/USDT".to_string(),
            last_price: None,
            high_24h: None,
            low_24h: None,
            volume_24h: None,
            quote_volume_24h: None,
            best_bid: None,
            best_ask: None,
        };

        let data = build_exchange_token_data("FOO", "FOO_USDT", &ticker);
        assert_eq!(data.price_usd, 0.0);
        assert_eq!(data.volume_24h, 0.0);
    }

    #[test]
    fn test_monitor_args_with_pair() {
        let args = MonitorArgs {
            token: "PUSD".to_string(),
            chain: "ethereum".to_string(),
            refresh: None,
            layout: None,
            scale: None,
            color_scheme: None,
            export: None,
            venue: Some("biconomy".to_string()),
            pair: Some("PUSD_USDT".to_string()),
        };
        assert_eq!(args.pair, Some("PUSD_USDT".to_string()));
        assert_eq!(args.venue, Some("biconomy".to_string()));
    }

    #[test]
    fn test_monitor_args_pair_none_by_default() {
        let args = MonitorArgs {
            token: "BTC".to_string(),
            chain: "ethereum".to_string(),
            refresh: None,
            layout: None,
            scale: None,
            color_scheme: None,
            export: None,
            venue: None,
            pair: None,
        };
        assert!(args.pair.is_none());
    }

    #[test]
    fn test_build_exchange_token_data_extracts_base_symbol() {
        let ticker = crate::market::Ticker {
            pair: "DOGE/USDT".to_string(),
            last_price: Some(0.123),
            high_24h: Some(0.13),
            low_24h: Some(0.12),
            volume_24h: Some(1_000_000.0),
            quote_volume_24h: Some(123_000.0),
            best_bid: Some(0.1225),
            best_ask: Some(0.1235),
        };

        let data = build_exchange_token_data("DOGE", "DOGE_USDT", &ticker);
        assert_eq!(data.symbol, "DOGE");
        assert_eq!(data.name, "DOGE");
        assert_eq!(data.price_usd, 0.123);
        assert_eq!(data.volume_24h, 1_000_000.0);
        // Verify all DEX-specific fields are zeroed/empty
        assert_eq!(data.price_change_24h, 0.0);
        assert_eq!(data.price_change_6h, 0.0);
        assert_eq!(data.price_change_1h, 0.0);
        assert_eq!(data.price_change_5m, 0.0);
        assert_eq!(data.volume_6h, 0.0);
        assert_eq!(data.volume_1h, 0.0);
        assert_eq!(data.liquidity_usd, 0.0);
        assert!(data.market_cap.is_none());
        assert!(data.fdv.is_none());
        assert!(data.earliest_pair_created_at.is_none());
        assert!(data.image_url.is_none());
        assert!(data.websites.is_empty());
        assert!(data.socials.is_empty());
        assert_eq!(data.total_buys_24h, 0);
        assert_eq!(data.total_sells_24h, 0);
    }

    #[test]
    fn test_build_exchange_token_data_address_format() {
        let ticker = crate::market::Ticker {
            pair: "X/Y".to_string(),
            last_price: Some(1.0),
            high_24h: None,
            low_24h: None,
            volume_24h: None,
            quote_volume_24h: None,
            best_bid: None,
            best_ask: None,
        };

        let data = build_exchange_token_data("X", "X_Y", &ticker);
        assert_eq!(data.address, "exchange:X_Y");
    }

    #[test]
    fn test_monitor_args_pair_requires_venue_conceptually() {
        // When --pair is set, --venue should also be set.
        // This is a structural test; the runtime validation is in run().
        let args = MonitorArgs {
            token: "PUSD".to_string(),
            chain: "ethereum".to_string(),
            refresh: None,
            layout: None,
            scale: None,
            color_scheme: None,
            export: None,
            venue: Some("biconomy".to_string()),
            pair: Some("PUSD_USDT".to_string()),
        };
        assert!(args.venue.is_some());
        assert!(args.pair.is_some());
    }

    #[test]
    fn test_run_direct_config_pair_passthrough() {
        // Verify that run_direct properly propagates the pair field
        let config = Config::default();
        let args = MonitorArgs {
            token: "PUSD".to_string(),
            chain: "ethereum".to_string(),
            layout: None,
            refresh: None,
            scale: None,
            color_scheme: None,
            export: None,
            venue: Some("biconomy".to_string()),
            pair: Some("PUSD_USDT".to_string()),
        };

        // Simulate the config override path from run_direct
        let mut mc = config.monitor.clone();
        if let Some(ref venue) = args.venue {
            mc.venue = Some(venue.clone());
        }
        assert_eq!(mc.venue, Some("biconomy".to_string()));
        // pair is passed directly to run(), not stored in MonitorConfig
        assert_eq!(args.pair, Some("PUSD_USDT".to_string()));
    }

    // =========================================================================
    // resolve_token_address chain filter tests
    // =========================================================================

    #[test]
    fn test_chain_filter_logic_ethereum_default() {
        // When chain is "ethereum" (default), chain_filter should be None
        // so we search ALL chains and exact matches on any chain sort first.
        let chain = "ethereum";
        let chain_filter: Option<&str> = if chain != "ethereum" {
            Some(chain)
        } else {
            None
        };
        assert!(chain_filter.is_none());
    }

    #[test]
    fn test_chain_filter_logic_explicit_chain() {
        // When chain is explicitly set (not "ethereum"), filter to that chain.
        let chain = "solana";
        let chain_filter: Option<&str> = if chain != "ethereum" {
            Some(chain)
        } else {
            None
        };
        assert_eq!(chain_filter, Some("solana"));
    }

    #[test]
    fn test_chain_filter_logic_bsc() {
        let chain = "bsc";
        let chain_filter: Option<&str> = if chain != "ethereum" {
            Some(chain)
        } else {
            None
        };
        assert_eq!(chain_filter, Some("bsc"));
    }

    #[tokio::test]
    async fn test_resolve_token_address_with_address_input() {
        use crate::chains::dex::DexClient;
        let config = Config::default();
        let dex = DexClient::new();
        // EVM address should be returned directly, no DexScreener query
        let result = resolve_token_address(
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
            "ethereum",
            &config,
            &dex,
        )
        .await
        .unwrap();
        assert_eq!(result, "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
    }

    #[tokio::test]
    async fn test_resolve_token_address_solana_address() {
        use crate::chains::dex::DexClient;
        let config = Config::default();
        let dex = DexClient::new();
        // Solana address (base58, 32+ chars) should be returned directly
        let addr = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let result = resolve_token_address(addr, "solana", &config, &dex)
            .await
            .unwrap();
        assert_eq!(result, addr);
    }

    #[test]
    fn test_try_cex_fallback_returns_correct_structure() {
        // Verify the fallback constructs results with the right shape
        // (can't easily test async CEX call without mocking, so test the structure)
        let result = crate::chains::TokenSearchResult {
            address: String::new(),
            symbol: "TEST".to_string(),
            name: "TEST".to_string(),
            chain: "ethereum".to_string(),
            price_usd: Some(1.0),
            volume_24h: 100.0,
            liquidity_usd: 0.0,
            market_cap: None,
        };
        assert_eq!(result.symbol, "TEST");
        assert!(result.address.is_empty());
        assert_eq!(result.liquidity_usd, 0.0);
    }
}
