//! Configuration types, constants, and data structures for the monitor TUI.

use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use std::time::Instant;

// Re-export config types from scope-core so existing `crate::cli::monitor::X` paths keep working.
pub use scope::config::{
    AlertConfig, ColorScheme, DEFAULT_MONITOR_REFRESH_SECS, ExportConfig, LayoutPreset,
    MonitorConfig, ScaleMode, WidgetVisibility,
};

/// Maximum data retention: 24 hours.
/// At 5-second intervals: 24 * 60 * 12 = 17,280 points max per history.
/// With DataPoint at 24 bytes: ~415 KB per history, ~830 KB total.
/// Data is persisted to OS temp folder for session continuity.
pub(crate) const MAX_DATA_AGE_SECS: f64 = 24.0 * 3600.0; // 24 hours

/// Cache file prefix in temp directory.
pub(crate) const CACHE_FILE_PREFIX: &str = "bcc_monitor_";

/// Default refresh interval in seconds.
pub(crate) const DEFAULT_REFRESH_SECS: u64 = DEFAULT_MONITOR_REFRESH_SECS;

/// Minimum refresh interval in seconds.
pub(crate) const MIN_REFRESH_SECS: u64 = 1;

/// Maximum refresh interval in seconds.
pub(crate) const MAX_REFRESH_SECS: u64 = 60;

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
pub(crate) struct CachedMonitorData {
    /// Token address this cache is for.
    pub(crate) token_address: String,
    /// Chain identifier.
    pub(crate) chain: String,
    /// Price history data points.
    pub(crate) price_history: Vec<DataPoint>,
    /// Volume history data points.
    pub(crate) volume_history: Vec<DataPoint>,
    /// Timestamp when cache was saved.
    pub(crate) saved_at: f64,
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

/// Extension trait for `ColorScheme` that provides TUI-specific palette
/// rendering (depends on `ratatui::style::Color` which is not in scope-core).
pub trait ColorSchemeExt {
    fn palette(&self) -> ColorPalette;
}

impl ColorSchemeExt for ColorScheme {
    /// Returns the named color palette for this scheme.
    fn palette(&self) -> ColorPalette {
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

/// An active (currently firing) alert with a description.
#[derive(Debug, Clone)]
pub struct ActiveAlert {
    /// Human-readable message describing the alert.
    pub message: String,
    /// When the alert was first triggered.
    pub triggered_at: Instant,
}
