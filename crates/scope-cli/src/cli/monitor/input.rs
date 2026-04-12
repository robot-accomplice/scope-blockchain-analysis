//! MonitorApp struct and event handling for the monitor TUI.

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
};
use scope::chains::ChainClient;
use scope::chains::dex::{DexClient, DexDataSource};
use scope::error::{Result, ScopeError};
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
    /// Creates a new monitor application with a real terminal and live DEX client.
    pub fn new(
        initial_data: DexTokenData,
        chain: &str,
        monitor_config: &MonitorConfig,
        chain_client: Option<Box<dyn ChainClient>>,
        exchange_client: Option<scope::market::ExchangeClient>,
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
    pub(crate) fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) {
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
    pub(crate) async fn fetch_data(&mut self) {
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
pub(crate) fn handle_key_event_on_state(
    key: crossterm::event::KeyEvent,
    state: &mut MonitorState,
) -> bool {
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
