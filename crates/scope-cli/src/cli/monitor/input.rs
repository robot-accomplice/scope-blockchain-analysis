//! MonitorApp struct and event handling for the monitor TUI.

use crate::cli::screen;
use crossterm::event::{Event, KeyCode, KeyModifiers};
use scope::chains::ChainClient;
use scope::chains::dex::DexDataSource;
use scope::error::Result;
use std::io;
use std::time::{Duration, Instant};

use super::config::{MonitorConfig, OhlcCandle, TimePeriod};
use super::state::MonitorState;
use super::widgets::ui;

use scope::chains::dex::DexTokenData;

/// Main monitor application, generic over terminal backend for testability.
///
/// The default type parameter (`CrosstermBackend<Stdout>`) is used in production.
/// Tests use `TestBackend` to avoid real terminal I/O.
pub struct MonitorApp<B: ratatui::backend::Backend = ratatui::backend::CrosstermBackend<io::Stdout>>
{
    /// Terminal backend.
    pub(crate) terminal: ratatui::Terminal<B>,

    /// Monitor state.
    pub(crate) state: MonitorState,

    /// DEX client for fetching data.
    pub(crate) dex_client: Box<dyn DexDataSource>,

    /// Optional chain client for on-chain data (holder count, etc.).
    pub(crate) chain_client: Option<Box<dyn ChainClient>>,

    /// Optional exchange client for real OHLC data.
    pub(crate) exchange_client: Option<scope::market::ExchangeClient>,

    /// Whether to exit the application.
    pub(crate) should_exit: bool,

    /// Whether this app owns the real terminal (and should restore it on drop).
    /// False for test instances using `TestBackend`.
    pub(crate) owns_terminal: bool,
}

/// Production constructor and terminal-specific methods.
impl MonitorApp {
    /// Creates a new monitor application with a real terminal.
    ///
    /// `dex_client` comes from the caller's client factory, so the monitor
    /// uses the configured HTTP transport (and Ghola) for every refresh.
    pub fn new(
        initial_data: DexTokenData,
        chain: &str,
        monitor_config: &MonitorConfig,
        dex_client: Box<dyn DexDataSource>,
        chain_client: Option<Box<dyn ChainClient>>,
        exchange_client: Option<scope::market::ExchangeClient>,
    ) -> Result<Self> {
        let terminal = screen::enter(screen::Mouse::Capture)?;

        let mut state = MonitorState::new(&initial_data, chain);
        state.apply_config(monitor_config);

        // If we have an exchange client, set up the venue pair for OHLC queries
        if let Some(ref ex) = exchange_client {
            let pair = ex.venue_symbol(&scope::market::Pair::parse(&initial_data.symbol));
            state.venue_pair = Some(pair);
            state.venue = Some(ex.venue_id().to_string());
        }

        Ok(Self {
            terminal,
            state,
            dex_client,
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
            self.terminal.draw(|f| {
                ui(f, &mut self.state);
                crate::cli::plain::finish_frame(f);
            })?;

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
            screen::restore();
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
            screen::restore();
        }
        Ok(())
    }

    /// Handles a single key event with [`handle_key_event_on_state`], the
    /// one key handler (production and tests).
    pub(crate) fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) {
        if handle_key_event_on_state(key, &mut self.state) {
            self.should_exit = true;
        }
    }

    /// Fetches new data from the API.
    pub(crate) async fn fetch_data(&mut self) {
        if self.state.venue_only {
            // DexScreener does not know an `exchange:<pair>` token.
            self.fetch_venue_ticker().await;
        } else {
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

        self.fetch_venue_book_and_trades().await;

        // Periodically fetch holder count via chain client (~every 12th refresh ≈ 60s at 5s rate)
        self.state.holder_fetch_counter += 1;
        if self.state.holder_fetch_counter.is_multiple_of(12)
            && !self.state.venue_only
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

impl<B: ratatui::backend::Backend> MonitorApp<B> {
    /// With a venue, its real order book and recent trades replace the ones
    /// built from DEX data (#78). A failed fetch keeps the last real data
    /// and shows the error.
    async fn fetch_venue_book_and_trades(&mut self) {
        let (Some(ex), Some(pair)) = (&self.exchange_client, self.state.venue_pair.clone()) else {
            return;
        };
        let venue = ex.venue_id().to_string();
        if ex.has_order_book() {
            match ex.fetch_order_book(&pair).await {
                Ok(book) => self.state.set_venue_book(&venue, book),
                Err(e) => self.state.error_message = Some(format!("{venue} order book: {e}")),
            }
        }
        if ex.has_trade_history() {
            let limit = super::state::MAX_RECENT_TRADES as u32;
            match ex.fetch_recent_trades(&pair, limit).await {
                Ok(trades) => self.state.set_venue_trades(&venue, trades),
                Err(e) => self.state.error_message = Some(format!("{venue} trades: {e}")),
            }
        }
    }

    /// Venue-only refresh: prices the token from the venue ticker.
    async fn fetch_venue_ticker(&mut self) {
        let (Some(ex), Some(pair)) = (&self.exchange_client, self.state.venue_pair.clone()) else {
            // Venue-only mode is set only with a venue and a pair. Say so
            // rather than freeze the price without a word.
            self.state.error_message =
                Some("Venue-only mode has no venue or pair to refresh from".to_string());
            self.state.last_update = Instant::now(); // Prevent rapid retries
            return;
        };
        match ex.fetch_ticker(&pair).await {
            Ok(ticker) => {
                let data = super::build_exchange_token_data(&self.state.symbol, &pair, &ticker);
                self.state.set_venue_ticker(&ticker);
                self.state.update(&data);
            }
            Err(e) => {
                self.state.error_message = Some(e.to_string());
                self.state.last_update = Instant::now(); // Prevent rapid retries
            }
        }
    }
}

/// The one monitor key handler: mutates `state` and returns true when the
/// monitor should exit. `?` shows the key overlay; with it open, any key
/// closes it (#68).
pub(crate) fn handle_key_event_on_state(
    key: crossterm::event::KeyEvent,
    state: &mut MonitorState,
) -> bool {
    // Track last input time for auto-pause
    state.last_input_at = Instant::now();

    if state.show_keys {
        state.show_keys = false;
        return false;
    }
    if key.code == KeyCode::Char('?') {
        state.show_keys = true;
        return false;
    }

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
            // The key says it was heard (#68).
            state.log("Refresh requested".to_string());
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
