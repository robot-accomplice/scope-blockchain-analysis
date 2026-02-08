//! # Live Token Monitor
//!
//! This module implements a real-time terminal UI for monitoring token metrics.
//! It displays live-updating charts for price, volume, transactions, and liquidity
//! across four switchable layout presets with responsive terminal sizing.
//!
//! ## Usage
//!
//! From interactive mode:
//! ```text
//! scope> monitor USDC
//! scope> mon 0x1234...
//! ```
//!
//! ## Layout Presets
//!
//! - **Dashboard** -- Balanced 2x2 grid with all widgets (default)
//! - **ChartFocus** -- Price chart takes ~80% of screen; minimal stats below
//! - **Feed** -- Activity log prioritized; metrics/volume on top
//! - **Compact** -- Minimal single-column for small terminals (<80 cols or <24 rows)
//!
//! The monitor auto-selects a layout based on terminal dimensions (responsive
//! breakpoints). Manual switching via `L`/`H` disables auto-selection until `A`.
//!
//! ## Features
//!
//! - Real-time price chart (line or candlestick) with sliding window
//! - Volume bar chart
//! - Buy/sell ratio gauge with activity log
//! - Key metrics panel with sparkline and stats table
//! - Config-driven widget visibility (toggle any widget on/off)
//! - Four layout presets switchable at runtime
//! - Responsive terminal sizing with auto-layout
//!
//! ## Keyboard Controls
//!
//! - `Q`/`Esc` quit, `R` refresh, `P`/`Space` pause
//! - `L`/`H` cycle layout forward/backward
//! - `W` + `1-5` toggle widget visibility
//! - `A` re-enable auto layout
//! - `C` toggle chart mode, `T`/`Tab` cycle time period, `1-4` select period
//! - `J`/`K` scroll activity log, `+`/`-` adjust refresh speed

use crate::chains::ChainClientFactory;
use crate::chains::dex::{DexClient, DexDataSource, DexTokenData};
use crate::config::Config;
use crate::error::{Result, ScopeError};
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
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::interactive::SessionContext;

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
    /// Last 15 minutes
    Min15,
    /// Last 1 hour
    Hour1,
    /// Last 6 hours
    Hour6,
    /// Last 24 hours
    Hour24,
}

impl TimePeriod {
    /// Returns the duration in seconds for this period.
    pub fn duration_secs(&self) -> i64 {
        match self {
            TimePeriod::Min15 => 15 * 60,
            TimePeriod::Hour1 => 3600,
            TimePeriod::Hour6 => 6 * 3600,
            TimePeriod::Hour24 => 24 * 3600,
        }
    }

    /// Returns a display label for this period.
    pub fn label(&self) -> &'static str {
        match self {
            TimePeriod::Min15 => "15m",
            TimePeriod::Hour1 => "1h",
            TimePeriod::Hour6 => "6h",
            TimePeriod::Hour24 => "24h",
        }
    }

    /// Returns the zero-based index for this period (for Tabs widget).
    pub fn index(&self) -> usize {
        match self {
            TimePeriod::Min15 => 0,
            TimePeriod::Hour1 => 1,
            TimePeriod::Hour6 => 2,
            TimePeriod::Hour24 => 3,
        }
    }

    /// Cycles to the next time period.
    pub fn next(&self) -> Self {
        match self {
            TimePeriod::Min15 => TimePeriod::Hour1,
            TimePeriod::Hour1 => TimePeriod::Hour6,
            TimePeriod::Hour6 => TimePeriod::Hour24,
            TimePeriod::Hour24 => TimePeriod::Min15,
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
}

impl ChartMode {
    /// Cycles to the next chart mode.
    pub fn next(&self) -> Self {
        match self {
            ChartMode::Line => ChartMode::Candlestick,
            ChartMode::Candlestick => ChartMode::Line,
        }
    }

    /// Returns a display label for this mode.
    pub fn label(&self) -> &'static str {
        match self {
            ChartMode::Line => "Line",
            ChartMode::Candlestick => "Candle",
        }
    }
}

/// Layout preset for the monitor TUI.
///
/// Controls which widgets are shown and how they are arranged.
/// Can be switched at runtime with keybindings or set via config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
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
}

impl LayoutPreset {
    /// Cycles to the next layout preset.
    pub fn next(&self) -> Self {
        match self {
            LayoutPreset::Dashboard => LayoutPreset::ChartFocus,
            LayoutPreset::ChartFocus => LayoutPreset::Feed,
            LayoutPreset::Feed => LayoutPreset::Compact,
            LayoutPreset::Compact => LayoutPreset::Dashboard,
        }
    }

    /// Cycles to the previous layout preset.
    pub fn prev(&self) -> Self {
        match self {
            LayoutPreset::Dashboard => LayoutPreset::Compact,
            LayoutPreset::ChartFocus => LayoutPreset::Dashboard,
            LayoutPreset::Feed => LayoutPreset::ChartFocus,
            LayoutPreset::Compact => LayoutPreset::Feed,
        }
    }

    /// Returns a display label for this preset.
    pub fn label(&self) -> &'static str {
        match self {
            LayoutPreset::Dashboard => "Dashboard",
            LayoutPreset::ChartFocus => "Chart",
            LayoutPreset::Feed => "Feed",
            LayoutPreset::Compact => "Compact",
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
}

impl Default for WidgetVisibility {
    fn default() -> Self {
        Self {
            price_chart: true,
            volume_chart: true,
            buy_sell_pressure: true,
            metrics_panel: true,
            activity_log: true,
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
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            layout: LayoutPreset::Dashboard,
            refresh_seconds: DEFAULT_REFRESH_SECS,
            widgets: WidgetVisibility::default(),
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
            start_timestamp: now_ts as i64,
            layout: LayoutPreset::Dashboard,
            widgets: WidgetVisibility::default(),
            auto_layout: true,
            widget_toggle_mode: false,
        }
    }

    /// Applies monitor config settings to this state.
    pub fn apply_config(&mut self, config: &MonitorConfig) {
        self.layout = config.layout;
        self.widgets = config.widgets.clone();
        self.refresh_rate = Duration::from_secs(config.refresh_seconds);
    }

    /// Toggles between line and candlestick chart modes.
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

        self.last_update = Instant::now();
        self.error_message = None;

        self.log(format!("Updated: ${:.6}", token_data.price_usd));

        // Periodically save to cache (every 60 updates, ~5 minutes at 5s refresh)
        if self.real_data_count.is_multiple_of(60) {
            self.save_cache();
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
    /// - 15m view: 1-minute candles
    /// - 1h view: 5-minute candles
    /// - 6h view: 15-minute candles  
    /// - 24h view: 1-hour candles
    pub fn get_ohlc_candles(&self) -> Vec<OhlcCandle> {
        let (data, _) = self.get_price_data_for_period();

        if data.is_empty() {
            return vec![];
        }

        // Determine candle duration based on time period
        let candle_duration_secs = match self.time_period {
            TimePeriod::Min15 => 60.0,    // 1-minute candles
            TimePeriod::Hour1 => 300.0,   // 5-minute candles
            TimePeriod::Hour6 => 900.0,   // 15-minute candles
            TimePeriod::Hour24 => 3600.0, // 1-hour candles
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
    pub fn should_refresh(&self) -> bool {
        !self.paused && self.last_update.elapsed() >= self.refresh_rate
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

/// Main monitor application.
pub struct MonitorApp {
    /// Terminal backend.
    terminal: ratatui::DefaultTerminal,

    /// Monitor state.
    state: MonitorState,

    /// DEX client for fetching data.
    dex_client: DexClient,

    /// Whether to exit the application.
    should_exit: bool,
}

impl MonitorApp {
    /// Creates a new monitor application.
    pub fn new(
        initial_data: DexTokenData,
        chain: &str,
        monitor_config: &MonitorConfig,
    ) -> Result<Self> {
        // Setup terminal using ratatui's simplified init
        let terminal = ratatui::init();
        // Enable mouse capture (not handled by ratatui::init)
        execute!(io::stdout(), EnableMouseCapture)
            .map_err(|e| ScopeError::Chain(format!("Failed to enable mouse capture: {}", e)))?;

        let mut state = MonitorState::new(&initial_data, chain);
        state.apply_config(monitor_config);

        Ok(Self {
            terminal,
            state,
            dex_client: DexClient::new(),
            should_exit: false,
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

    /// Handles a single key event, updating state accordingly.
    /// Extracted from the event loop for testability.
    fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) {
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
                self.should_exit = true;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_exit = true;
            }
            KeyCode::Char('r') => {
                self.state.force_refresh();
            }
            KeyCode::Char('p') | KeyCode::Char(' ') => {
                self.state.toggle_pause();
            }
            // Increase refresh interval (slower)
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char(']') => {
                self.state.slower_refresh();
            }
            // Decrease refresh interval (faster)
            KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Char('[') => {
                self.state.faster_refresh();
            }
            // Time period selection (1=15m, 2=1h, 3=6h, 4=24h)
            KeyCode::Char('1') => {
                self.state.set_time_period(TimePeriod::Min15);
            }
            KeyCode::Char('2') => {
                self.state.set_time_period(TimePeriod::Hour1);
            }
            KeyCode::Char('3') => {
                self.state.set_time_period(TimePeriod::Hour6);
            }
            KeyCode::Char('4') => {
                self.state.set_time_period(TimePeriod::Hour24);
            }
            KeyCode::Char('t') | KeyCode::Tab => {
                self.state.cycle_time_period();
            }
            // Toggle chart mode (line/candlestick)
            KeyCode::Char('c') => {
                self.state.toggle_chart_mode();
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
    }

    /// Cleans up terminal state.
    pub fn cleanup(&mut self) -> Result<()> {
        // Save cache before exiting
        self.state.save_cache();

        // Disable mouse capture (not handled by ratatui::restore)
        let _ = execute!(io::stdout(), DisableMouseCapture);
        // Restore terminal using ratatui's simplified cleanup
        ratatui::restore();
        Ok(())
    }
}

impl Drop for MonitorApp {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableMouseCapture);
        ratatui::restore();
    }
}

/// Handles a key event by mutating state. Standalone version for testability.
/// Returns true if the application should exit.
#[cfg(test)]
fn handle_key_event_on_state(key: crossterm::event::KeyEvent, state: &mut MonitorState) -> bool {
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
            return true;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return true;
        }
        KeyCode::Char('r') => {
            state.force_refresh();
        }
        KeyCode::Char('p') | KeyCode::Char(' ') => {
            state.toggle_pause();
        }
        KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char(']') => {
            state.slower_refresh();
        }
        KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Char('[') => {
            state.faster_refresh();
        }
        KeyCode::Char('1') => {
            state.set_time_period(TimePeriod::Min15);
        }
        KeyCode::Char('2') => {
            state.set_time_period(TimePeriod::Hour1);
        }
        KeyCode::Char('3') => {
            state.set_time_period(TimePeriod::Hour6);
        }
        KeyCode::Char('4') => {
            state.set_time_period(TimePeriod::Hour24);
        }
        KeyCode::Char('t') | KeyCode::Tab => {
            state.cycle_time_period();
        }
        KeyCode::Char('c') => {
            state.toggle_chart_mode();
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
}

/// Dashboard layout: balanced 2x2 grid (closest to the original layout).
///
/// ```text
/// ┌──────────────────────┬──────────────────────┐
/// │  Price Chart (60%)   │  Volume Chart (60%)  │
/// ├──────────────────────┼──────────────────────┤
/// │  Buy/Sell (40%)      │  Metrics (40%)       │
/// └──────────────────────┴──────────────────────┘
/// ```
fn layout_dashboard(area: Rect, widgets: &WidgetVisibility) -> LayoutAreas {
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(content_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(content_chunks[1]);

    LayoutAreas {
        price_chart: if widgets.price_chart {
            Some(left_chunks[0])
        } else {
            None
        },
        buy_sell_gauge: if widgets.buy_sell_pressure {
            Some(left_chunks[1])
        } else {
            None
        },
        volume_chart: if widgets.volume_chart {
            Some(right_chunks[0])
        } else {
            None
        },
        metrics_panel: if widgets.metrics_panel {
            Some(right_chunks[1])
        } else {
            None
        },
    }
}

/// Chart-focus layout: price chart dominates ~80% of screen.
///
/// ```text
/// ┌────────────────────────────────────────────┐
/// │                                            │
/// │            Price Chart (~80%)               │
/// │                                            │
/// ├──────────────────────┬─────────────────────┤
/// │  Buy/Sell (50%)      │  Metrics (50%)      │
/// └──────────────────────┴─────────────────────┘
/// ```
fn layout_chart_focus(area: Rect, widgets: &WidgetVisibility) -> LayoutAreas {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(80), Constraint::Percentage(20)])
        .split(area);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(vertical[1]);

    LayoutAreas {
        price_chart: if widgets.price_chart {
            Some(vertical[0])
        } else {
            None
        },
        volume_chart: None, // Hidden in chart-focus
        buy_sell_gauge: if widgets.buy_sell_pressure {
            Some(bottom[0])
        } else {
            None
        },
        metrics_panel: if widgets.metrics_panel {
            Some(bottom[1])
        } else {
            None
        },
    }
}

/// Feed layout: activity log/buy-sell panel dominates; price ticker + metrics on top.
///
/// ```text
/// ┌──────────────────────┬─────────────────────┐
/// │  Metrics (50%)       │  Volume (50%)        │
/// ├──────────────────────┴─────────────────────┤
/// │                                            │
/// │            Buy/Sell + Activity Log (~75%)   │
/// │                                            │
/// └────────────────────────────────────────────┘
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
        price_chart: None, // Hidden in feed mode
        metrics_panel: if widgets.metrics_panel {
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
            Some(vertical[1])
        } else {
            None
        },
    }
}

/// Compact layout: minimal single-column view for small terminals.
///
/// ```text
/// ┌────────────────────────────────────────────┐
/// │  Metrics Panel (top half)                  │
/// ├────────────────────────────────────────────┤
/// │  Buy/Sell + Activity Log (bottom half)     │
/// └────────────────────────────────────────────┘
/// ```
fn layout_compact(area: Rect, widgets: &WidgetVisibility) -> LayoutAreas {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    LayoutAreas {
        price_chart: None,  // Hidden in compact
        volume_chart: None, // Hidden in compact
        metrics_panel: if widgets.metrics_panel {
            Some(vertical[0])
        } else {
            None
        },
        buy_sell_gauge: if widgets.buy_sell_pressure {
            Some(vertical[1])
        } else {
            None
        },
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
    };

    // Render each widget if its area is allocated
    if let Some(area) = areas.price_chart {
        match state.chart_mode {
            ChartMode::Line => render_price_chart(f, area, state),
            ChartMode::Candlestick => render_candlestick_chart(f, area, state),
        }
    }
    if let Some(area) = areas.volume_chart {
        render_volume_chart(f, area, &*state);
    }
    if let Some(area) = areas.buy_sell_gauge {
        render_buy_sell_gauge(f, area, state);
    }
    if let Some(area) = areas.metrics_panel {
        render_metrics_panel(f, area, &*state);
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
    let tab_titles = vec!["15m", "1h", "6h", "24h"];
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
    let is_price_up = price_change >= 0.0;
    let trend_color = if is_price_up {
        Color::Green
    } else {
        Color::Red
    };
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

    // Split data into synthetic and real datasets for visual differentiation
    let synthetic_data: Vec<(f64, f64)> = data
        .iter()
        .zip(&is_real)
        .filter(|(_, real)| !**real)
        .map(|(point, _)| *point)
        .collect();

    let real_data: Vec<(f64, f64)> = data
        .iter()
        .zip(&is_real)
        .filter(|(_, real)| **real)
        .map(|(point, _)| *point)
        .collect();

    // Create reference line at first price (horizontal line for comparison)
    let reference_line: Vec<(f64, f64)> = vec![(x_min, first_price), (x_max, first_price)];

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
    let mid_price = (y_min + y_max) / 2.0;

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
                .title(Span::styled("USD", Style::new().gray()))
                .style(Style::new().gray())
                .bounds([y_min, y_max])
                .labels(vec![
                    Span::raw(format_price_usd(y_min)),
                    Span::raw(format_price_usd(mid_price)),
                    Span::raw(format_price_usd(y_max)),
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

    let is_price_up = price_change >= 0.0;
    let trend_color = if is_price_up {
        Color::Green
    } else {
        Color::Red
    };
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

    // Clone candles for the closure
    let candles_clone = candles.clone();

    let canvas = Canvas::default()
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::new().fg(trend_color)),
        )
        .x_bounds([x_min - candle_spacing, x_max])
        .y_bounds([y_min, y_max])
        .paint(move |ctx| {
            for candle in &candles_clone {
                let color = if candle.is_bullish {
                    Color::Green
                } else {
                    Color::Red
                };

                // Draw the wick (high-low line)
                ctx.draw(&CanvasLine {
                    x1: candle.timestamp,
                    y1: candle.low,
                    x2: candle.timestamp,
                    y2: candle.high,
                    color,
                });

                // Draw the body (open-close rectangle)
                let body_top = candle.open.max(candle.close);
                let body_bottom = candle.open.min(candle.close);
                let body_height = (body_top - body_bottom).max(price_range * 0.002); // Minimum visible height

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

/// Renders the volume chart with visual differentiation between real and synthetic data.
fn render_volume_chart(f: &mut Frame, area: Rect, state: &MonitorState) {
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
    let volume_str = format_usd(current_volume);

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
            Style::new().fg(Color::Blue).bold(),
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
                Color::Blue
            } else {
                Color::LightBlue
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

/// Renders the buy/sell ratio gauge and recent activity.
fn render_buy_sell_gauge(f: &mut Frame, area: Rect, state: &mut MonitorState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    // Buy/Sell ratio bar — green for buys, red for sells
    let ratio = state.buy_ratio();
    let border_color = if ratio > 0.5 {
        Color::Green
    } else {
        Color::Red
    };

    let block = Block::default()
        .title(" ◐ Buy/Sell Ratio (24h) ")
        .borders(Borders::ALL)
        .border_style(Style::new().fg(border_color));

    let inner = block.inner(chunks[0]);
    f.render_widget(block, chunks[0]);

    // Build a two-tone bar: green portion for buys, red portion for sells
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

        // Render green buy blocks and red sell blocks
        let buy_bar = "█".repeat(buy_width as usize);
        let sell_bar = "█".repeat(sell_width as usize);
        let bar_line = Line::from(vec![
            Span::styled(buy_bar, Style::new().green()),
            Span::styled(sell_bar, Style::new().red()),
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

    // Activity log — scrollable with j/k keys
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

    f.render_stateful_widget(log_list, chunks[1], &mut state.log_list_state);
}

/// Renders the key metrics panel.
fn render_metrics_panel(f: &mut Frame, area: Rect, state: &MonitorState) {
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
        Color::Green
    } else {
        Color::Red
    };

    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .title(" ◉ Price Trend ")
                .borders(Borders::ALL)
                .border_style(Style::new().magenta()),
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
        Color::Green
    } else if state.price_change_5m < 0.0 {
        Color::Red
    } else {
        Color::Gray
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
        Color::Green
    } else {
        Color::Yellow
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
        .map(format_usd)
        .unwrap_or_else(|| "N/A".to_string());

    let rows = vec![
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
            Span::raw(format_usd(state.liquidity_usd)),
        ]),
        Row::new(vec![
            Span::styled("Vol 24h", Style::new().gray()),
            Span::raw(format_usd(state.volume_24h)),
        ]),
        Row::new(vec![
            Span::styled("Mkt Cap", Style::new().gray()),
            Span::raw(market_cap_str),
        ]),
        Row::new(vec![
            Span::styled("Buys", Style::new().gray()),
            Span::styled(format!("{}", state.buys_24h), Style::new().green()),
        ]),
        Row::new(vec![
            Span::styled("Sells", Style::new().gray()),
            Span::styled(format!("{}", state.sells_24h), Style::new().red()),
        ]),
    ];

    let table = Table::new(rows, [Constraint::Length(8), Constraint::Min(10)]).block(
        Block::default()
            .title(" ◉ Key Metrics ")
            .borders(Borders::ALL)
            .border_style(Style::new().magenta()),
    );

    f.render_widget(table, chunks[1]);
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

    let spans = vec![
        status,
        Span::raw(" ║ "),
        Span::styled("Q", Style::new().red().bold()),
        Span::raw("uit "),
        Span::styled("R", Style::new().fg(Color::Green).bold()),
        Span::raw("efresh "),
        Span::styled("P", Style::new().fg(Color::Yellow).bold()),
        Span::raw("ause "),
        Span::styled("L", Style::new().fg(Color::Cyan).bold()),
        Span::raw(format!(":{} ", state.layout.label())),
        widget_hint,
        Span::raw("idget "),
        Span::styled("C", Style::new().fg(Color::LightBlue).bold()),
        Span::raw(format!("hart:{} ", state.chart_mode.label())),
        Span::styled("T", Style::new().fg(Color::Magenta).bold()),
        Span::raw("ime "),
    ];

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

/// Formats a USD amount with appropriate suffix.
fn format_usd(n: f64) -> String {
    if n >= 1_000_000_000.0 {
        format!("${:.2}B", n / 1_000_000_000.0)
    } else if n >= 1_000_000.0 {
        format!("${:.2}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("${:.2}K", n / 1_000.0)
    } else {
        format!("${:.2}", n)
    }
}

/// Entry point for the monitor command from interactive mode.
pub async fn run(
    token: Option<String>,
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

    println!("Starting live monitor for {}...", token_input);
    println!("Fetching initial data...");

    // Resolve token address
    let dex_client = clients.create_dex_client();
    let token_address =
        resolve_token_address(&token_input, &ctx.chain, config, dex_client.as_ref()).await?;

    // Fetch initial data
    let initial_data = dex_client
        .get_token_data(&ctx.chain, &token_address)
        .await?;

    println!(
        "Monitoring {} ({}) on {}",
        initial_data.symbol, initial_data.name, ctx.chain
    );
    println!("Press Q to quit, R to refresh, P to pause...\n");

    // Small delay to let user read the message
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Create and run the app
    let mut app = MonitorApp::new(initial_data, &ctx.chain, &config.monitor)?;
    let result = app.run().await;

    // Cleanup is handled by Drop, but we do it explicitly for error handling
    if let Err(e) = app.cleanup() {
        eprintln!("Warning: Failed to cleanup terminal: {}", e);
    }

    result
}

/// Resolves a token input (address or symbol) to an address.
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

    // Check saved aliases
    let aliases = crate::tokens::TokenAliases::load();
    if let Some(alias) = aliases.get(input, Some(chain)) {
        return Ok(alias.address.clone());
    }

    // Search by name/symbol
    let results = dex_client.search_tokens(input, Some(chain)).await?;

    if results.is_empty() {
        return Err(ScopeError::NotFound(format!(
            "No token found matching '{}' on {}",
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
        assert_eq!(format_usd(500.0), "$500.00");
        assert_eq!(format_usd(1_500.0), "$1.50K");
        assert_eq!(format_usd(1_500_000.0), "$1.50M");
        assert_eq!(format_usd(1_500_000_000.0), "$1.50B");
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
        assert_eq!(TimePeriod::Min15.label(), "15m");
        assert_eq!(TimePeriod::Hour1.label(), "1h");
        assert_eq!(TimePeriod::Hour6.label(), "6h");
        assert_eq!(TimePeriod::Hour24.label(), "24h");

        assert_eq!(TimePeriod::Min15.duration_secs(), 15 * 60);
        assert_eq!(TimePeriod::Hour1.duration_secs(), 3600);
        assert_eq!(TimePeriod::Hour6.duration_secs(), 6 * 3600);
        assert_eq!(TimePeriod::Hour24.duration_secs(), 24 * 3600);

        // Test cycling
        assert_eq!(TimePeriod::Min15.next(), TimePeriod::Hour1);
        assert_eq!(TimePeriod::Hour1.next(), TimePeriod::Hour6);
        assert_eq!(TimePeriod::Hour6.next(), TimePeriod::Hour24);
        assert_eq!(TimePeriod::Hour24.next(), TimePeriod::Min15);
    }

    #[test]
    fn test_monitor_state_time_period() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");

        // Default is 1 hour
        assert_eq!(state.time_period, TimePeriod::Hour1);

        // Cycle through periods
        state.cycle_time_period();
        assert_eq!(state.time_period, TimePeriod::Hour6);

        state.set_time_period(TimePeriod::Hour24);
        assert_eq!(state.time_period, TimePeriod::Hour24);
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

    // ========================================================================
    // ChartMode tests
    // ========================================================================

    #[test]
    fn test_chart_mode_cycle() {
        let mode = ChartMode::Line;
        assert_eq!(mode.next(), ChartMode::Candlestick);
        assert_eq!(ChartMode::Candlestick.next(), ChartMode::Line);
    }

    #[test]
    fn test_chart_mode_label() {
        assert_eq!(ChartMode::Line.label(), "Line");
        assert_eq!(ChartMode::Candlestick.label(), "Candle");
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
            TimePeriod::Min15,
            TimePeriod::Hour1,
            TimePeriod::Hour6,
            TimePeriod::Hour24,
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
        assert_eq!(state.chart_mode, ChartMode::Line);
    }

    #[test]
    fn test_cycle_all_time_periods() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        assert_eq!(state.time_period, TimePeriod::Hour1);
        state.cycle_time_period();
        assert_eq!(state.time_period, TimePeriod::Hour6);
        state.cycle_time_period();
        assert_eq!(state.time_period, TimePeriod::Hour24);
        state.cycle_time_period();
        assert_eq!(state.time_period, TimePeriod::Min15);
        state.cycle_time_period();
        assert_eq!(state.time_period, TimePeriod::Hour1);
    }

    #[test]
    fn test_set_specific_time_period() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        state.set_time_period(TimePeriod::Hour24);
        assert_eq!(state.time_period, TimePeriod::Hour24);
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
        assert_eq!(format!("{}", TimePeriod::Min15), "15m");
        assert_eq!(format!("{}", TimePeriod::Hour24), "24h");
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
        assert!(!format_usd(0.0).is_empty());
        assert!(!format_usd(999.0).is_empty());
        assert!(!format_usd(1500.0).is_empty());
        assert!(!format_usd(1_500_000.0).is_empty());
        assert!(!format_usd(1_500_000_000.0).is_empty());
        assert!(!format_usd(1_500_000_000_000.0).is_empty());
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
        state.set_time_period(TimePeriod::Hour6);
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
            TimePeriod::Min15,
            TimePeriod::Hour1,
            TimePeriod::Hour6,
            TimePeriod::Hour24,
        ] {
            for mode in &[ChartMode::Line, ChartMode::Candlestick] {
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
        assert!(matches!(state.time_period, TimePeriod::Min15));

        handle_key_event_on_state(make_key_event(KeyCode::Char('2')), &mut state);
        assert!(matches!(state.time_period, TimePeriod::Hour1));

        handle_key_event_on_state(make_key_event(KeyCode::Char('3')), &mut state);
        assert!(matches!(state.time_period, TimePeriod::Hour6));

        handle_key_event_on_state(make_key_event(KeyCode::Char('4')), &mut state);
        assert!(matches!(state.time_period, TimePeriod::Hour24));
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
            TimePeriod::Min15,
            TimePeriod::Hour1,
            TimePeriod::Hour6,
            TimePeriod::Hour24,
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
            TimePeriod::Min15,
            TimePeriod::Hour1,
            TimePeriod::Hour6,
            TimePeriod::Hour24,
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
            TimePeriod::Min15,
            TimePeriod::Hour1,
            TimePeriod::Hour6,
            TimePeriod::Hour24,
        ] {
            state.set_time_period(period);
            terminal
                .draw(|f| render_header(f, f.area(), &state))
                .unwrap();
        }
    }

    #[test]
    fn test_time_period_index() {
        assert_eq!(TimePeriod::Min15.index(), 0);
        assert_eq!(TimePeriod::Hour1.index(), 1);
        assert_eq!(TimePeriod::Hour6.index(), 2);
        assert_eq!(TimePeriod::Hour24.index(), 3);
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
        assert_eq!(LayoutPreset::Compact.next(), LayoutPreset::Dashboard);
    }

    #[test]
    fn test_layout_preset_prev_cycles() {
        assert_eq!(LayoutPreset::Dashboard.prev(), LayoutPreset::Compact);
        assert_eq!(LayoutPreset::Compact.prev(), LayoutPreset::Feed);
        assert_eq!(LayoutPreset::Feed.prev(), LayoutPreset::ChartFocus);
        assert_eq!(LayoutPreset::ChartFocus.prev(), LayoutPreset::Dashboard);
    }

    #[test]
    fn test_layout_preset_full_cycle() {
        let start = LayoutPreset::Dashboard;
        let mut preset = start;
        for _ in 0..4 {
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
    fn test_layout_chart_focus_no_volume() {
        let area = Rect::new(0, 0, 120, 40);
        let vis = WidgetVisibility::default();
        let areas = layout_chart_focus(area, &vis);
        assert!(areas.price_chart.is_some());
        assert!(areas.volume_chart.is_none()); // Always hidden in chart-focus
        assert!(areas.buy_sell_gauge.is_some());
        assert!(areas.metrics_panel.is_some());
    }

    #[test]
    fn test_layout_feed_no_price_chart() {
        let area = Rect::new(0, 0, 120, 40);
        let vis = WidgetVisibility::default();
        let areas = layout_feed(area, &vis);
        assert!(areas.price_chart.is_none()); // Always hidden in feed
        assert!(areas.volume_chart.is_some());
        assert!(areas.buy_sell_gauge.is_some());
        assert!(areas.metrics_panel.is_some());
    }

    #[test]
    fn test_layout_compact_minimal() {
        let area = Rect::new(0, 0, 60, 20);
        let vis = WidgetVisibility::default();
        let areas = layout_compact(area, &vis);
        assert!(areas.price_chart.is_none()); // Always hidden
        assert!(areas.volume_chart.is_none()); // Always hidden
        assert!(areas.metrics_panel.is_some());
        assert!(areas.buy_sell_gauge.is_some());
    }

    #[test]
    fn test_ui_render_all_layouts_no_panic() {
        let presets = [
            LayoutPreset::Dashboard,
            LayoutPreset::ChartFocus,
            LayoutPreset::Feed,
            LayoutPreset::Compact,
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
        assert_eq!(state.layout, LayoutPreset::Compact);
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
            },
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
            },
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
        };
        let areas = layout_dashboard(area, &vis);
        assert!(areas.price_chart.is_none());
        assert!(areas.volume_chart.is_none());
        assert!(areas.buy_sell_gauge.is_none());
        assert!(areas.metrics_panel.is_none());
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
}
