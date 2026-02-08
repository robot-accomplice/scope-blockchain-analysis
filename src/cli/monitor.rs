//! # Live Token Monitor
//!
//! This module implements a real-time terminal UI for monitoring token metrics.
//! It displays live-updating charts for price, volume, transactions, and liquidity.
//!
//! ## Usage
//!
//! From interactive mode:
//! ```text
//! bcc> monitor USDC
//! bcc> mon 0x1234...
//! ```
//!
//! ## Features
//!
//! - Real-time price chart with sliding window
//! - Volume bar chart
//! - Buy/sell ratio gauge
//! - Key metrics panel (price, liquidity, market cap, 24h volume)
//! - Keyboard controls: Q=quit, R=refresh, P=pause

use crate::chains::ChainClientFactory;
use crate::chains::dex::{DexClient, DexDataSource, DexTokenData};
use crate::config::Config;
use crate::error::{BccError, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Axis, Block, Borders, Chart, Dataset, Gauge, GraphType, List, ListItem, Paragraph,
        canvas::{Canvas, Line as CanvasLine, Rectangle},
    },
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::io::{self, Stdout};
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

    /// Error message to display (if any).
    pub error_message: Option<String>,

    /// Selected time period for chart display.
    pub time_period: TimePeriod,

    /// Chart display mode (line or candlestick).
    pub chart_mode: ChartMode,

    /// Unix timestamp when monitoring started.
    pub start_timestamp: i64,
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
            error_message: None,
            time_period: TimePeriod::Hour1, // Default to 1 hour view
            chart_mode: ChartMode::Line,    // Default to line chart
            start_timestamp: now_ts as i64,
        }
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
    terminal: Terminal<CrosstermBackend<Stdout>>,

    /// Monitor state.
    state: MonitorState,

    /// DEX client for fetching data.
    dex_client: DexClient,

    /// Whether to exit the application.
    should_exit: bool,
}

impl MonitorApp {
    /// Creates a new monitor application.
    pub fn new(initial_data: DexTokenData, chain: &str) -> Result<Self> {
        // Setup terminal
        enable_raw_mode()
            .map_err(|e| BccError::Chain(format!("Failed to enable raw mode: {}", e)))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .map_err(|e| BccError::Chain(format!("Failed to enter alternate screen: {}", e)))?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)
            .map_err(|e| BccError::Chain(format!("Failed to create terminal: {}", e)))?;

        Ok(Self {
            terminal,
            state: MonitorState::new(&initial_data, chain),
            dex_client: DexClient::new(),
            should_exit: false,
        })
    }

    /// Runs the main event loop.
    pub async fn run(&mut self) -> Result<()> {
        loop {
            // Render UI
            self.terminal.draw(|f| ui(f, &self.state))?;

            // Handle events with timeout
            if crossterm::event::poll(Duration::from_millis(100))
                .map_err(|e| BccError::Chain(format!("Event poll error: {}", e)))?
                && let Event::Key(key) = event::read()
                    .map_err(|e| BccError::Chain(format!("Event read error: {}", e)))?
            {
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
                    _ => {}
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

        disable_raw_mode()
            .map_err(|e| BccError::Chain(format!("Failed to disable raw mode: {}", e)))?;
        execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .map_err(|e| BccError::Chain(format!("Failed to leave alternate screen: {}", e)))?;
        self.terminal
            .show_cursor()
            .map_err(|e| BccError::Chain(format!("Failed to show cursor: {}", e)))?;
        Ok(())
    }
}

impl Drop for MonitorApp {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

/// Renders the UI.
fn ui(f: &mut Frame, state: &MonitorState) {
    // Main layout: header, content, footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(10),   // Content
            Constraint::Length(3), // Footer
        ])
        .split(f.area());

    // Render header
    render_header(f, chunks[0], state);

    // Content layout: 2x2 grid
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(content_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(content_chunks[1]);

    // Render panels - dispatch to appropriate chart type
    match state.chart_mode {
        ChartMode::Line => render_price_chart(f, left_chunks[0], state),
        ChartMode::Candlestick => render_candlestick_chart(f, left_chunks[0], state),
    }
    render_buy_sell_gauge(f, left_chunks[1], state);
    render_volume_chart(f, right_chunks[0], state);
    render_metrics_panel(f, right_chunks[1], state);

    // Render footer
    render_footer(f, chunks[2], state);
}

/// Renders the header with token info.
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
        " ◈ {} ({}) │ {} │ {} ",
        state.symbol,
        state.name,
        state.chain.to_uppercase(),
        state.time_period.label()
    );

    let price_str = format_price_usd(state.current_price);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            price_str,
            Style::default()
                .fg(price_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(trend_arrow, Style::default().fg(price_color)),
        Span::styled(format!(" {}", change_str), Style::default().fg(price_color)),
    ]))
    .block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );

    f.render_widget(header, area);
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
            Style::default()
                .fg(trend_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("({}) ", change_str),
            Style::default().fg(trend_color),
        ),
        Span::styled(
            format!("│{}│ ", state.time_period.label()),
            Style::default().fg(Color::Gray),
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
            .style(Style::default().fg(Color::DarkGray))
            .data(&reference_line),
    );

    // Synthetic data shown with Dot marker and dimmed color
    if !synthetic_data.is_empty() {
        datasets.push(
            Dataset::default()
                .name("◇Est")
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Cyan))
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
                .style(Style::default().fg(trend_color))
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
                .border_style(Style::default().fg(trend_color)),
        )
        .x_axis(
            Axis::default()
                .title(Span::styled("Time", Style::default().fg(Color::Gray)))
                .style(Style::default().fg(Color::Gray))
                .bounds([x_min, x_max])
                .labels(vec![Span::raw(time_label), Span::raw("now")]),
        )
        .y_axis(
            Axis::default()
                .title(Span::styled("USD", Style::default().fg(Color::Gray)))
                .style(Style::default().fg(Color::Gray))
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
            Style::default()
                .fg(trend_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("({}) ", change_str),
            Style::default().fg(trend_color),
        ),
        Span::styled(
            format!("│{}│ ", state.time_period.label()),
            Style::default().fg(Color::Gray),
        ),
        Span::styled("⊞Candles ", Style::default().fg(Color::Magenta)),
    ]);

    // Clone candles for the closure
    let candles_clone = candles.clone();

    let canvas = Canvas::default()
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(trend_color)),
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
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("│{}│ ", state.time_period.label()),
            Style::default().fg(Color::Gray),
        ),
        Span::styled(data_indicator, Style::default().fg(Color::DarkGray)),
    ]);

    // Calculate bounds
    let max_volume = data.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max);
    let min_volume = data.iter().map(|(_, v)| *v).fold(f64::MAX, f64::min);

    // Handle case where volumes are similar (cumulative 24h volume doesn't change much)
    let vol_range = max_volume - min_volume;
    let (y_min, y_max) = if vol_range < max_volume * 0.01 {
        // Less than 1% variation - center the data with ±5% padding
        let padding = max_volume * 0.05;
        (min_volume - padding, max_volume + padding)
    } else {
        // Normal variation - show from 0 to max
        (0.0, max_volume * 1.1)
    };

    // Ensure y_min is not negative
    let y_min = y_min.max(0.0);

    let x_min = data.first().map(|(t, _)| *t).unwrap_or(0.0);
    let x_max = data.last().map(|(t, _)| *t).unwrap_or(1.0);
    // Ensure x range is non-zero
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

    let mut datasets = Vec::new();

    // Synthetic data shown with Dot marker and light blue color
    if !synthetic_data.is_empty() {
        datasets.push(
            Dataset::default()
                .name("◇Est")
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::LightBlue))
                .data(&synthetic_data),
        );
    }

    // Real data shown with Braille marker and blue color
    if !real_data.is_empty() {
        datasets.push(
            Dataset::default()
                .name("●Live")
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Blue))
                .data(&real_data),
        );
    }

    // Create time labels based on period
    let time_label = format!("-{}", state.time_period.label());

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .title(chart_title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue)),
        )
        .x_axis(
            Axis::default()
                .title("Time")
                .style(Style::default().fg(Color::Gray))
                .bounds([x_min, x_max])
                .labels(vec![Span::raw(time_label), Span::raw("now")]),
        )
        .y_axis(
            Axis::default()
                .title("USD")
                .style(Style::default().fg(Color::Gray))
                .bounds([y_min, y_max])
                .labels(vec![
                    Span::raw(format_number(y_min)),
                    Span::raw(format_number((y_min + y_max) / 2.0)),
                    Span::raw(format_number(y_max)),
                ]),
        );

    f.render_widget(chart, area);
}

/// Renders the buy/sell ratio gauge and recent activity.
fn render_buy_sell_gauge(f: &mut Frame, area: Rect, state: &MonitorState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    // Buy/Sell gauge
    let ratio = state.buy_ratio();
    let color = if ratio > 0.5 {
        Color::Green
    } else {
        Color::Red
    };

    // Create a visual bar using Unicode block characters
    let buy_indicator = if ratio > 0.5 { "▶" } else { "▷" };
    let sell_indicator = if ratio < 0.5 { "◀" } else { "◁" };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(" ◐ Buy/Sell Ratio (24h) ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color)),
        )
        .gauge_style(Style::default().fg(color))
        .ratio(ratio)
        .label(format!(
            "{}Buys: {} │ Sells: {}{} ({:.1}%)",
            buy_indicator,
            state.buys_24h,
            state.sells_24h,
            sell_indicator,
            ratio * 100.0
        ));

    f.render_widget(gauge, chunks[0]);

    // Activity log
    let items: Vec<ListItem> = state
        .log_messages
        .iter()
        .rev()
        .take(5)
        .map(|msg| ListItem::new(msg.as_str()).style(Style::default().fg(Color::Gray)))
        .collect();

    let log_list = List::new(items).block(
        Block::default()
            .title(" ◷ Activity Log ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    f.render_widget(log_list, chunks[1]);
}

/// Renders the key metrics panel.
fn render_metrics_panel(f: &mut Frame, area: Rect, state: &MonitorState) {
    // Format 5m change with appropriate color
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

    // Calculate time since last price change
    let now_ts = chrono::Utc::now().timestamp() as f64;
    let secs_since_change = (now_ts - state.last_price_change_at).max(0.0) as u64;
    let last_change_str = if secs_since_change < 60 {
        format!("{}s ago", secs_since_change)
    } else if secs_since_change < 3600 {
        format!("{}m ago", secs_since_change / 60)
    } else {
        format!("{}h ago", secs_since_change / 3600)
    };

    // Build metrics as styled lines
    let text: Vec<Line> = vec![
        Line::from(vec![
            Span::raw("Price:      "),
            Span::styled(
                format_price_usd(state.current_price),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("5m Change:  "),
            Span::styled(change_5m_str, Style::default().fg(change_5m_color)),
        ]),
        Line::from(vec![
            Span::raw("Last Δ:     "),
            Span::styled(
                last_change_str,
                Style::default().fg(if secs_since_change < 60 {
                    Color::Green
                } else {
                    Color::Yellow
                }),
            ),
        ]),
        Line::from(format!(
            "24h Change: {}{:.2}%",
            if state.price_change_24h >= 0.0 {
                "+"
            } else {
                ""
            },
            state.price_change_24h
        )),
        Line::from(format!("Liquidity:  {}", format_usd(state.liquidity_usd))),
        Line::from(format!("24h Volume: {}", format_usd(state.volume_24h))),
        Line::from(format!(
            "Market Cap: {}",
            state
                .market_cap
                .map(format_usd)
                .unwrap_or_else(|| "N/A".to_string())
        )),
        Line::from(String::new()),
        Line::from(format!("24h Buys:   {}", state.buys_24h)),
        Line::from(format!("24h Sells:  {}", state.sells_24h)),
    ];

    let panel = Paragraph::new(text).block(
        Block::default()
            .title(" ◉ Key Metrics ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta)),
    );

    f.render_widget(panel, area);
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
        Span::styled(format!("⚠ {}", err), Style::default().fg(Color::Red))
    } else if state.paused {
        Span::styled(
            "⏸ PAUSED",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!(
                "↻ {}s │ Δ {} │ {} pts │ {}",
                elapsed,
                price_change_str,
                synthetic_count + real_count,
                memory_str
            ),
            Style::default().fg(Color::Gray),
        )
    };

    let spans = vec![
        status,
        Span::raw(" ║ "),
        Span::styled(
            "Q",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw("uit "),
        Span::styled(
            "R",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("efresh "),
        Span::styled(
            "P",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("ause "),
        Span::styled(
            "1-4",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("/"),
        Span::styled(
            "T",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("ime "),
        Span::styled(
            "C",
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("hart:{} ", state.chart_mode.label())),
        Span::styled(
            "±",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("Speed"),
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
            return Err(BccError::Chain(
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
    let mut app = MonitorApp::new(initial_data, &ctx.chain)?;
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
    // Check if it's already an address
    if input.starts_with("0x") && input.len() == 42 {
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
        return Err(BccError::NotFound(format!(
            "No token found matching '{}' on {}",
            input, chain
        )));
    }

    // Use the first result (highest liquidity)
    let token = &results[0];
    println!(
        "Found: {} ({}) - ${:.6}",
        token.symbol,
        token.name,
        token.price_usd.unwrap_or(0.0)
    );

    Ok(token.address.clone())
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
        state.refresh_rate = Duration::from_millis(10);

        // Just created, should not need refresh
        assert!(!state.should_refresh());

        // Simulate time passing
        state.last_update = Instant::now() - Duration::from_secs(10);
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

        // Should have generated synthetic history
        assert!(state.price_history.len() > 1);
        assert!(state.volume_history.len() > 1);

        // All initial data should be synthetic (is_real = false)
        assert!(state.price_history.iter().all(|p| !p.is_real));
        assert!(state.volume_history.iter().all(|p| !p.is_real));
        assert_eq!(state.real_data_count, 0);

        // Price history should span approximately 24 hours
        if let (Some(first), Some(last)) = (state.price_history.front(), state.price_history.back())
        {
            let span = last.timestamp - first.timestamp;
            assert!(span > 20.0 * 3600.0); // At least ~20 hours
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

        // Get initial synthetic data
        let (data, is_real) = state.get_price_data_for_period();
        assert_eq!(data.len(), is_real.len());
        assert!(is_real.iter().all(|r| !r)); // All synthetic initially

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
        let state = create_populated_state();
        terminal
            .draw(|f| render_buy_sell_gauge(f, f.area(), &state))
            .unwrap();
    }

    #[test]
    fn test_render_buy_sell_gauge_balanced() {
        let mut terminal = create_test_terminal();
        let mut token_data = create_test_token_data();
        token_data.total_buys_24h = 100;
        token_data.total_sells_24h = 100;
        let state = MonitorState::new(&token_data, "ethereum");
        terminal
            .draw(|f| render_buy_sell_gauge(f, f.area(), &state))
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
        let state = create_populated_state();
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
                render_buy_sell_gauge(f, chunks[3], &state);
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
}
