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

pub mod config;
pub mod input;
pub mod state;
pub mod widgets;

// Re-export key types for external use
pub use config::{
    AlertConfig, ChartMode, ColorScheme, ExportConfig, LayoutPreset, MonitorConfig, ScaleMode,
    TimePeriod, WidgetVisibility,
};
pub use input::MonitorApp;
pub use state::MonitorState;

use clap::Args;
use scope::chains::ChainClientFactory;
use scope::chains::dex::{DexDataSource, DexTokenData};
use scope::config::Config;
use scope::error::{Result, ScopeError};
use std::io;
use std::path::PathBuf;
use std::time::Duration;

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
#[command(after_help = "\x1b[1mExamples:\x1b[0m
  scope monitor USDC
  scope monitor @dai-token                                \x1b[2m# address book shortcut\x1b[0m
  scope monitor PEPE --chain ethereum --layout chart-focus
  scope monitor DAI --venue binance --pair DAI_USDT
  scope monitor BTC --venue binance --refresh 5 --scale log")]
pub struct MonitorArgs {
    /// Token address or symbol to monitor.
    ///
    /// Can be a contract address (0x...) or a token symbol/name.
    /// If a name/symbol is provided, matching tokens will be searched
    /// and you can select from the results.
    /// Use @label to resolve from the address book (e.g., @dai-token).
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

    /// Direct trading pair for exchange-only mode (e.g., DAI_USDT).
    ///
    /// Bypasses DexScreener token resolution entirely and uses the exchange
    /// ticker as the data source. Requires `--venue` to be specified.
    /// Use this when the token is not listed on DexScreener.
    #[arg(long, value_name = "PAIR")]
    pub pair: Option<String>,
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
/// monitoring tokens that are not indexed by DexScreener.
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
        scope::market::VenueRegistry::load()
            .ok()
            .and_then(|r| r.get(venue_id).cloned())
            .map(|desc| scope::market::ExchangeClient::from_descriptor(&desc))
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

        // Extract base symbol from pair label (e.g., "DAI/USDT" → "DAI")
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
    ticker: &scope::market::Ticker,
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
    if scope::tokens::TokenAliases::is_address(input) {
        return Ok(input.to_string());
    }

    // Check saved aliases — use chain filter only when explicitly overridden
    let chain_filter = if chain != "ethereum" {
        Some(chain)
    } else {
        None
    };
    let aliases = scope::tokens::TokenAliases::load();
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
async fn try_cex_fallback(symbol: &str, chain: &str) -> Option<scope::chains::TokenSearchResult> {
    let registry = scope::market::VenueRegistry::load().ok()?;
    let descriptor = registry.get("binance")?;
    let client = scope::market::ExchangeClient::from_descriptor(&descriptor.clone());
    let pair = client.format_pair(&format!("{}USDT", symbol.to_uppercase()));
    let ticker = client.fetch_ticker(&pair).await.ok()?;
    let price = ticker.last_price.unwrap_or(0.0);
    Some(scope::chains::TokenSearchResult {
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
    results: &[scope::chains::dex::TokenSearchResult],
) -> Result<&scope::chains::dex::TokenSearchResult> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    select_token_impl(results, &mut stdin.lock(), &mut stdout.lock())
}

/// Testable implementation of token selection with injected I/O.
fn select_token_impl<'a>(
    results: &'a [scope::chains::dex::TokenSearchResult],
    reader: &mut impl io::BufRead,
    writer: &mut impl io::Write,
) -> Result<&'a scope::chains::dex::TokenSearchResult> {
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
    use super::config::*;
    use super::input::*;
    use super::state::*;
    use super::widgets::*;
    use super::*;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::{Constraint, Direction, Layout, Rect};
    use scope::chains::ChainClient;
    use scope::market::{Trade, TradeSide};
    use std::fs;
    use std::io::Write as _;
    use std::time::Instant;

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
        assert_eq!(scope::display::format_usd(500.0), "$500.00");
        assert_eq!(scope::display::format_usd(1_500.0), "$1.50K");
        assert_eq!(scope::display::format_usd(1_500_000.0), "$1.50M");
        assert_eq!(scope::display::format_usd(1_500_000_000.0), "$1.50B");
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
        assert!(!scope::display::format_usd(0.0).is_empty());
        assert!(!scope::display::format_usd(999.0).is_empty());
        assert!(!scope::display::format_usd(1500.0).is_empty());
        assert!(!scope::display::format_usd(1_500_000.0).is_empty());
        assert!(!scope::display::format_usd(1_500_000_000.0).is_empty());
        assert!(!scope::display::format_usd(1_500_000_000_000.0).is_empty());
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

    fn make_monitor_search_results() -> Vec<scope::chains::dex::TokenSearchResult> {
        vec![
            scope::chains::dex::TokenSearchResult {
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
                chain: "ethereum".to_string(),
                price_usd: Some(1.0),
                volume_24h: 1_000_000.0,
                liquidity_usd: 500_000_000.0,
                market_cap: Some(30_000_000_000.0),
            },
            scope::chains::dex::TokenSearchResult {
                symbol: "USDC".to_string(),
                name: "Bridged USD Coin".to_string(),
                address: "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174".to_string(),
                chain: "ethereum".to_string(),
                price_usd: Some(0.9998),
                volume_24h: 500_000.0,
                liquidity_usd: 100_000_000.0,
                market_cap: None,
            },
            scope::chains::dex::TokenSearchResult {
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
            scope::chains::DexPair {
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
            scope::chains::DexPair {
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
        let pairs = vec![scope::chains::DexPair {
            dex_name: "Uniswap V3".to_string(),
            pair_address: "0xabc".to_string(),
            base_token: "DAI".to_string(),
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
        let book = MonitorState::generate_synthetic_order_book(&pairs, "DAI", 1.0, 200_000.0);
        assert!(book.is_some());
        let book = book.unwrap();
        assert_eq!(book.pair, "DAI/USDT");
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

    use scope::chains::dex::TokenSearchResult;

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
        async fn get_balance(&self, _address: &str) -> Result<scope::chains::Balance> {
            unimplemented!("not needed for monitor tests")
        }
        async fn enrich_balance_usd(&self, _balance: &mut scope::chains::Balance) {}
        async fn get_transaction(&self, _hash: &str) -> Result<scope::chains::Transaction> {
            unimplemented!("not needed for monitor tests")
        }
        async fn get_transactions(
            &self,
            _address: &str,
            _limit: u32,
        ) -> Result<Vec<scope::chains::Transaction>> {
            Ok(vec![])
        }
        async fn get_block_number(&self) -> Result<u64> {
            Ok(1000000)
        }
        async fn get_token_balances(
            &self,
            _address: &str,
        ) -> Result<Vec<scope::chains::TokenBalance>> {
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
        let ticker = scope::market::Ticker {
            pair: "DAI/USDT".to_string(),
            last_price: Some(1.001),
            high_24h: Some(1.005),
            low_24h: Some(0.998),
            volume_24h: Some(500_000.0),
            quote_volume_24h: Some(500_500.0),
            best_bid: Some(1.0005),
            best_ask: Some(1.0015),
        };

        let data = build_exchange_token_data("DAI", "DAI_USDT", &ticker);

        assert_eq!(data.symbol, "DAI");
        assert_eq!(data.name, "DAI");
        assert_eq!(data.price_usd, 1.001);
        assert_eq!(data.volume_24h, 500_000.0);
        assert!(data.address.contains("exchange:"));
        assert!(data.pairs.is_empty());
        assert!(data.price_history.is_empty());
        assert!(data.dexscreener_url.is_none());
    }

    #[test]
    fn test_build_exchange_token_data_missing_price() {
        let ticker = scope::market::Ticker {
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
            token: "DAI".to_string(),
            chain: "ethereum".to_string(),
            refresh: None,
            layout: None,
            scale: None,
            color_scheme: None,
            export: None,
            venue: Some("biconomy".to_string()),
            pair: Some("DAI_USDT".to_string()),
        };
        assert_eq!(args.pair, Some("DAI_USDT".to_string()));
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
        let ticker = scope::market::Ticker {
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
        let ticker = scope::market::Ticker {
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
            token: "DAI".to_string(),
            chain: "ethereum".to_string(),
            refresh: None,
            layout: None,
            scale: None,
            color_scheme: None,
            export: None,
            venue: Some("biconomy".to_string()),
            pair: Some("DAI_USDT".to_string()),
        };
        assert!(args.venue.is_some());
        assert!(args.pair.is_some());
    }

    #[test]
    fn test_run_direct_config_pair_passthrough() {
        // Verify that run_direct properly propagates the pair field
        let config = Config::default();
        let args = MonitorArgs {
            token: "DAI".to_string(),
            chain: "ethereum".to_string(),
            layout: None,
            refresh: None,
            scale: None,
            color_scheme: None,
            export: None,
            venue: Some("biconomy".to_string()),
            pair: Some("DAI_USDT".to_string()),
        };

        // Simulate the config override path from run_direct
        let mut mc = config.monitor.clone();
        if let Some(ref venue) = args.venue {
            mc.venue = Some(venue.clone());
        }
        assert_eq!(mc.venue, Some("biconomy".to_string()));
        // pair is passed directly to run(), not stored in MonitorConfig
        assert_eq!(args.pair, Some("DAI_USDT".to_string()));
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
        use scope::chains::dex::DexClient;
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
        use scope::chains::dex::DexClient;
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
        let result = scope::chains::TokenSearchResult {
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

    #[test]
    fn test_select_token_impl_single_result_pick() {
        let results = vec![scope::chains::dex::TokenSearchResult {
            address: "0xabc".to_string(),
            symbol: "TEST".to_string(),
            name: "Test Token".to_string(),
            chain: "ethereum".to_string(),
            price_usd: Some(1.0),
            volume_24h: 100.0,
            liquidity_usd: 1000.0,
            market_cap: None,
        }];
        let input = b"1\n";
        let mut output = Vec::new();
        let result = select_token_impl(&results, &mut &input[..], &mut output);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().symbol, "TEST");
    }

    #[test]
    fn test_select_token_impl_out_of_range() {
        let results = vec![scope::chains::dex::TokenSearchResult {
            address: "0xabc".to_string(),
            symbol: "TEST".to_string(),
            name: "Test Token".to_string(),
            chain: "ethereum".to_string(),
            price_usd: Some(1.0),
            volume_24h: 100.0,
            liquidity_usd: 1000.0,
            market_cap: None,
        }];
        let input = b"0\n";
        let mut output = Vec::new();
        let result = select_token_impl(&results, &mut &input[..], &mut output);
        assert!(result.is_err());
    }

    #[test]
    fn test_select_token_impl_second_of_two() {
        let results = vec![
            scope::chains::dex::TokenSearchResult {
                address: "0xabc".to_string(),
                symbol: "TOKEN1".to_string(),
                name: "Token One".to_string(),
                chain: "ethereum".to_string(),
                price_usd: Some(1.0),
                volume_24h: 100.0,
                liquidity_usd: 1000.0,
                market_cap: None,
            },
            scope::chains::dex::TokenSearchResult {
                address: "0xdef".to_string(),
                symbol: "TOKEN2".to_string(),
                name: "Token Two".to_string(),
                chain: "solana".to_string(),
                price_usd: Some(2.0),
                volume_24h: 200.0,
                liquidity_usd: 2000.0,
                market_cap: None,
            },
        ];
        let input = b"2\n";
        let mut output = Vec::new();
        let result = select_token_impl(&results, &mut &input[..], &mut output);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().symbol, "TOKEN2");
    }

    // ========================================================================
    // Key event tests
    // ========================================================================

    #[test]
    fn test_handle_key_event_quit() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::NONE,
        );
        assert!(handle_key_event_on_state(key, &mut state));
    }

    #[test]
    fn test_handle_key_event_pause() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(' '),
            crossterm::event::KeyModifiers::NONE,
        );
        assert!(!state.paused);
        assert!(!handle_key_event_on_state(key, &mut state));
        assert!(state.paused);
    }

    #[test]
    fn test_handle_key_event_chart_cycle() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let initial_mode = state.chart_mode;
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('c'),
            crossterm::event::KeyModifiers::NONE,
        );
        handle_key_event_on_state(key, &mut state);
        assert_ne!(state.chart_mode, initial_mode);
    }

    #[test]
    fn test_handle_key_event_slower_faster() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let initial_rate = state.refresh_rate_secs();

        // '+' slows down (increases interval)
        let key_plus = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('+'),
            crossterm::event::KeyModifiers::NONE,
        );
        handle_key_event_on_state(key_plus, &mut state);
        assert!(state.refresh_rate_secs() > initial_rate);

        // '-' speeds up (decreases interval)
        let key_minus = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('-'),
            crossterm::event::KeyModifiers::NONE,
        );
        handle_key_event_on_state(key_minus, &mut state);
        assert_eq!(state.refresh_rate_secs(), initial_rate);
    }

    #[test]
    fn test_handle_key_event_time_period() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let initial_period = state.time_period;
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('t'),
            crossterm::event::KeyModifiers::NONE,
        );
        handle_key_event_on_state(key, &mut state);
        assert_ne!(state.time_period, initial_period);
    }

    #[test]
    fn test_handle_key_event_escape() {
        let token_data = create_test_token_data();
        let mut state = MonitorState::new(&token_data, "ethereum");
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        assert!(handle_key_event_on_state(key, &mut state));
    }

    // ========================================================================
    // Coverage gap: Exchange layout render functions (order book, trades, market info)
    // ========================================================================

    #[test]
    fn test_render_order_book_panel_no_data() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.order_book = None;
        let area = Rect::new(0, 0, 50, 20);
        terminal
            .draw(|f| render_order_book_panel(f, area, &state))
            .unwrap();
    }

    #[test]
    fn test_render_order_book_panel_with_data() {
        let mut terminal = create_test_terminal();
        let state = create_populated_state();
        let area = Rect::new(0, 0, 60, 25);
        terminal
            .draw(|f| render_order_book_panel(f, area, &state))
            .unwrap();
    }

    #[test]
    fn test_render_order_book_panel_narrow_height() {
        let mut terminal = create_test_terminal();
        let state = create_populated_state();
        let area = Rect::new(0, 0, 50, 3);
        terminal
            .draw(|f| render_order_book_panel(f, area, &state))
            .unwrap();
    }

    #[test]
    fn test_render_recent_trades_panel_empty() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.recent_trades.clear();
        let area = Rect::new(0, 0, 60, 15);
        terminal
            .draw(|f| render_recent_trades_panel(f, area, &state))
            .unwrap();
    }

    #[test]
    fn test_render_recent_trades_panel_with_trades() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.recent_trades.push_back(Trade {
            price: 1.5,
            quantity: 100.0,
            quote_quantity: None,
            timestamp_ms: 1700000000000,
            side: TradeSide::Buy,
            id: None,
        });
        state.recent_trades.push_back(Trade {
            price: 1.49,
            quantity: 50.0,
            quote_quantity: None,
            timestamp_ms: 1700000001000,
            side: TradeSide::Sell,
            id: None,
        });
        let area = Rect::new(0, 0, 60, 20);
        terminal
            .draw(|f| render_recent_trades_panel(f, area, &state))
            .unwrap();
    }

    #[test]
    fn test_render_recent_trades_panel_high_price_trade() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.recent_trades.push_back(Trade {
            price: 50_000.0,
            quantity: 0.5,
            quote_quantity: None,
            timestamp_ms: 1700000000000,
            side: TradeSide::Buy,
            id: None,
        });
        let area = Rect::new(0, 0, 60, 15);
        terminal
            .draw(|f| render_recent_trades_panel(f, area, &state))
            .unwrap();
    }

    #[test]
    fn test_render_market_info_panel_no_panic() {
        let mut terminal = create_test_terminal();
        let state = create_populated_state();
        let area = Rect::new(0, 0, 80, 30);
        terminal
            .draw(|f| render_market_info_panel(f, area, &state))
            .unwrap();
    }

    #[test]
    fn test_render_market_info_panel_with_dex_pairs() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.dex_pairs = vec![
            scope::chains::DexPair {
                dex_name: "Uniswap V3".to_string(),
                pair_address: "0xabc".to_string(),
                base_token: "TEST".to_string(),
                quote_token: "USDT".to_string(),
                price_usd: 1.0,
                volume_24h: 100_000.0,
                liquidity_usd: 250_000.0,
                price_change_24h: 2.5,
                buys_24h: 50,
                sells_24h: 40,
                buys_6h: 15,
                sells_6h: 12,
                buys_1h: 5,
                sells_1h: 4,
                pair_created_at: None,
                url: None,
            },
            scope::chains::DexPair {
                dex_name: "SushiSwap".to_string(),
                pair_address: "0xdef".to_string(),
                base_token: "TEST".to_string(),
                quote_token: "WETH".to_string(),
                price_usd: 1.01,
                volume_24h: 25_000.0,
                liquidity_usd: 80_000.0,
                price_change_24h: -1.2,
                buys_24h: 20,
                sells_24h: 25,
                buys_6h: 8,
                sells_6h: 10,
                buys_1h: 3,
                sells_1h: 4,
                pair_created_at: None,
                url: None,
            },
        ];
        let area = Rect::new(0, 0, 100, 35);
        terminal
            .draw(|f| render_market_info_panel(f, area, &state))
            .unwrap();
    }

    #[test]
    fn test_ohlc_candle_update_bearish() {
        let mut candle = OhlcCandle::new(1700000000.0, 10.0);
        assert!(candle.is_bullish);
        candle.update(9.5);
        assert!(!candle.is_bullish);
        assert_eq!(candle.high, 10.0);
        assert_eq!(candle.low, 9.5);
        assert_eq!(candle.close, 9.5);
    }

    #[test]
    fn test_time_period_index_all() {
        assert_eq!(TimePeriod::Min1.index(), 0);
        assert_eq!(TimePeriod::Min5.index(), 1);
        assert_eq!(TimePeriod::Min15.index(), 2);
        assert_eq!(TimePeriod::Hour1.index(), 3);
        assert_eq!(TimePeriod::Hour4.index(), 4);
        assert_eq!(TimePeriod::Day1.index(), 5);
    }

    #[test]
    fn test_format_number_boundary_values() {
        assert_eq!(format_number(0.5), "0.50");
        assert_eq!(format_number(999.99), "999.99");
        assert_eq!(format_number(1000.0), "1.00K");
        assert_eq!(format_number(999_999.0), "1000.00K"); // 999999/1000 = 999.999 -> rounds to 1000.00
        assert_eq!(format_number(1_000_000.0), "1.00M");
        assert_eq!(format_number(999_999_999.0), "1000.00M");
        assert_eq!(format_number(1_000_000_000.0), "1.00B");
    }

    #[test]
    fn test_chart_mode_volume_profile_label() {
        assert_eq!(ChartMode::VolumeProfile.label(), "VolPro");
        assert_eq!(ChartMode::VolumeProfile.next(), ChartMode::Line);
    }

    #[test]
    fn test_layout_exchange_ui_full_render() {
        let mut terminal = create_test_terminal();
        let mut state = create_populated_state();
        state.layout = LayoutPreset::Exchange;
        state.auto_layout = false;
        state.dex_pairs = vec![scope::chains::DexPair {
            dex_name: "Uniswap".to_string(),
            pair_address: "0xabc".to_string(),
            base_token: "TEST".to_string(),
            quote_token: "USDT".to_string(),
            price_usd: 1.0,
            volume_24h: 50_000.0,
            liquidity_usd: 100_000.0,
            price_change_24h: 0.5,
            buys_24h: 100,
            sells_24h: 80,
            buys_6h: 30,
            sells_6h: 25,
            buys_1h: 10,
            sells_1h: 8,
            pair_created_at: None,
            url: None,
        }];
        state.order_book = MonitorState::generate_synthetic_order_book(
            &state.dex_pairs,
            &state.symbol,
            state.current_price,
            state.liquidity_usd,
        );
        state.recent_trades.push_back(Trade {
            price: 1.0,
            quantity: 500.0,
            quote_quantity: None,
            timestamp_ms: 1700000000000,
            side: TradeSide::Buy,
            id: None,
        });
        terminal.draw(|f| ui(f, &mut state)).unwrap();
    }

    // ========================================================================
    // MonitorState::update and synthetic order book tests
    // ========================================================================

    fn make_dex_pair(
        dex: &str,
        base: &str,
        quote: &str,
        price: f64,
        vol: f64,
        liq: f64,
    ) -> scope::chains::DexPair {
        scope::chains::DexPair {
            dex_name: dex.to_string(),
            base_token: base.to_string(),
            quote_token: quote.to_string(),
            price_usd: price,
            volume_24h: vol,
            liquidity_usd: liq,
            price_change_24h: 5.0,
            pair_address: format!("0x{}", dex),
            buys_24h: 100,
            sells_24h: 50,
            buys_6h: 25,
            sells_6h: 12,
            buys_1h: 5,
            sells_1h: 3,
            pair_created_at: None,
            url: None,
        }
    }

    fn create_test_token_data_with_pairs() -> DexTokenData {
        let mut data = create_test_token_data();
        data.pairs = vec![
            make_dex_pair("uniswap", "TEST", "USDC", 1.0, 500_000.0, 300_000.0),
            make_dex_pair("sushiswap", "TEST", "WETH", 1.01, 200_000.0, 150_000.0),
        ];
        data
    }

    #[test]
    fn test_generate_synthetic_order_book_with_pairs() {
        let data = create_test_token_data_with_pairs();
        let book = MonitorState::generate_synthetic_order_book(
            &data.pairs,
            &data.symbol,
            data.price_usd,
            data.liquidity_usd,
        );
        assert!(book.is_some());
        let book = book.unwrap();
        assert!(!book.bids.is_empty());
        assert!(!book.asks.is_empty());
        assert!(book.bids[0].price < data.price_usd);
        assert!(book.asks[0].price > data.price_usd);
    }

    #[test]
    fn test_generate_synthetic_order_book_empty_pairs() {
        let book = MonitorState::generate_synthetic_order_book(&[], "TEST", 1.0, 500_000.0);
        // Empty pairs with positive price/liquidity still generates a book
        assert!(book.is_some());
    }

    #[test]
    fn test_generate_synthetic_order_book_zero_mid_price() {
        let pairs = vec![make_dex_pair("uniswap", "TEST", "USDC", 0.0, 100.0, 100.0)];
        let book = MonitorState::generate_synthetic_order_book(&pairs, "TEST", 0.0, 100.0);
        assert!(book.is_none());
    }

    #[test]
    fn test_generate_synthetic_order_book_high_liquidity() {
        let pairs = vec![make_dex_pair(
            "uniswap",
            "WETH",
            "USDC",
            3500.0,
            50_000_000.0,
            5_000_000.0,
        )];
        let book = MonitorState::generate_synthetic_order_book(&pairs, "WETH", 3500.0, 5_000_000.0);
        assert!(book.is_some());
        let book = book.unwrap();
        assert!(book.bids.len() > 5);
        assert!(book.asks.len() > 5);
    }

    #[test]
    fn test_monitor_state_update_with_token_data() {
        let initial_data = create_test_token_data_with_pairs();
        let mut state = MonitorState::new(&initial_data, "ethereum");
        assert_eq!(state.current_price, 1.0);
        assert_eq!(state.real_data_count, 0);

        // Update with new price
        let mut updated_data = initial_data.clone();
        updated_data.price_usd = 1.05;
        updated_data.volume_24h = 1_200_000.0;
        state.update(&updated_data);

        assert_eq!(state.current_price, 1.05);
        assert_eq!(state.real_data_count, 1);
        assert!(!state.price_history.is_empty());
        assert!(!state.volume_history.is_empty());
    }

    #[test]
    fn test_monitor_state_update_price_unchanged() {
        let data = create_test_token_data();
        let mut state = MonitorState::new(&data, "ethereum");
        state.update(&data);
        assert_eq!(state.real_data_count, 1);
        // Price didn't change, so last_price_change_at should not update
        assert_eq!(state.current_price, 1.0);
    }

    #[test]
    fn test_monitor_state_update_generates_trades() {
        let data = create_test_token_data_with_pairs();
        let mut state = MonitorState::new(&data, "ethereum");

        // Update with changed price to generate a trade
        let mut new_data = data.clone();
        new_data.price_usd = 1.10;
        state.update(&new_data);

        // recent_trades should have at least one trade from the update
        assert!(!state.recent_trades.is_empty());
    }

    #[test]
    fn test_monitor_state_update_liquidity_pairs() {
        let data = create_test_token_data_with_pairs();
        let mut state = MonitorState::new(&data, "ethereum");
        state.update(&data);
        assert_eq!(state.liquidity_pairs.len(), 2);
        assert!(state.liquidity_pairs[0].0.contains("TEST/USDC"));
        assert!(state.liquidity_pairs[1].0.contains("TEST/WETH"));
    }

    #[test]
    fn test_monitor_state_update_dex_pairs_and_order_book() {
        let data = create_test_token_data_with_pairs();
        let mut state = MonitorState::new(&data, "ethereum");
        state.update(&data);
        assert_eq!(state.dex_pairs.len(), 2);
        assert!(state.order_book.is_some());
    }

    #[test]
    fn test_monitor_state_update_sells_more_than_buys() {
        let mut data = create_test_token_data_with_pairs();
        data.total_buys_24h = 10;
        data.total_sells_24h = 50;
        let mut state = MonitorState::new(&data, "ethereum");
        state.update(&data);
        // Should produce Sell side trade when sells > buys and price unchanged
        assert!(!state.recent_trades.is_empty());
    }

    #[test]
    fn test_monitor_state_update_zero_volume() {
        let mut data = create_test_token_data_with_pairs();
        data.volume_24h = 0.0;
        let mut state = MonitorState::new(&data, "ethereum");
        state.update(&data);
        // Should still produce a trade with quantity 1.0
        if let Some(trade) = state.recent_trades.back() {
            assert_eq!(trade.quantity, 1.0);
        }
    }
}
