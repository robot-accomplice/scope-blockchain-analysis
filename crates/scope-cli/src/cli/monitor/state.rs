//! Application state management for the monitor TUI.

use ratatui::widgets::ListState;
use scope::chains::DexPair;
use scope::chains::dex::DexTokenData;
use scope::market::{OrderBook, OrderBookLevel, Trade, TradeSide};
use std::collections::VecDeque;
use std::fs;
use std::io::{BufWriter, Write as _};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::config::{
    ActiveAlert, AlertConfig, CACHE_FILE_PREFIX, CachedMonitorData, ChartMode, ColorPalette,
    ColorScheme, ColorSchemeExt, DEFAULT_REFRESH_SECS, DataPoint, LayoutPreset, MAX_DATA_AGE_SECS,
    MAX_REFRESH_SECS, MIN_REFRESH_SECS, MonitorConfig, OhlcCandle, ScaleMode, TimePeriod,
    WidgetVisibility,
};

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
    pub(crate) fn cache_path(token_address: &str, chain: &str) -> PathBuf {
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
    pub(crate) fn load_cache(token_address: &str, chain: &str) -> Option<CachedMonitorData> {
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
    pub(crate) fn generate_synthetic_price_history(
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
    pub(crate) fn generate_synthetic_volume_history(
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
    pub(crate) fn generate_synthetic_order_book(
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
    pub(crate) fn check_alerts(&mut self, token_data: &DexTokenData) {
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
                        scope::display::format_usd(avg_tx_size),
                        scope::display::format_usd(whale_min)
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
    pub(crate) fn write_export_row(&mut self) {
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
    pub(crate) fn log(&mut self, message: String) {
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
