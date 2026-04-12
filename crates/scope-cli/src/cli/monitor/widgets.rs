//! Layout and rendering functions for the monitor TUI.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Axis, Bar, BarChart, BarGroup, Block, Borders, Chart, Dataset, GraphType, List, ListItem,
        Paragraph, Row, Sparkline, Table, Tabs,
        canvas::{Canvas, Line as CanvasLine, Rectangle},
    },
};
use std::time::Instant;

use scope::market::TradeSide;

use super::config::{ChartMode, LayoutPreset, ScaleMode, WidgetVisibility};
use super::state::MonitorState;

/// Renders the UI.
/// Computed layout areas for each widget. `None` means the widget is hidden.
pub(crate) struct LayoutAreas {
    pub(crate) price_chart: Option<Rect>,
    pub(crate) volume_chart: Option<Rect>,
    pub(crate) buy_sell_gauge: Option<Rect>,
    pub(crate) metrics_panel: Option<Rect>,
    pub(crate) activity_feed: Option<Rect>,
    /// Order book depth panel (Exchange layout).
    pub(crate) order_book: Option<Rect>,
    /// Market info panel with pair details (Exchange layout).
    pub(crate) market_info: Option<Rect>,
    /// Recent trade history (Exchange layout).
    pub(crate) trade_history: Option<Rect>,
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
pub(crate) fn layout_dashboard(area: Rect, widgets: &WidgetVisibility) -> LayoutAreas {
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
pub(crate) fn layout_chart_focus(area: Rect, widgets: &WidgetVisibility) -> LayoutAreas {
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
pub(crate) fn layout_feed(area: Rect, widgets: &WidgetVisibility) -> LayoutAreas {
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
pub(crate) fn layout_compact(area: Rect, widgets: &WidgetVisibility) -> LayoutAreas {
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
pub(crate) fn layout_exchange(area: Rect, _widgets: &WidgetVisibility) -> LayoutAreas {
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
pub(crate) fn auto_select_layout(size: Rect) -> LayoutPreset {
    match (size.width, size.height) {
        (w, h) if w < 80 || h < 24 => LayoutPreset::Compact,
        (w, _) if w < 120 => LayoutPreset::Feed,
        (_, h) if h < 30 => LayoutPreset::ChartFocus,
        _ => LayoutPreset::Dashboard,
    }
}

/// Renders the UI, dispatching to the active layout preset.
pub(crate) fn ui(f: &mut Frame, state: &mut MonitorState) {
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
pub(crate) fn render_header(f: &mut Frame, area: Rect, state: &MonitorState) {
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
pub(crate) fn render_price_chart(f: &mut Frame, area: Rect, state: &MonitorState) {
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
pub(crate) fn is_stablecoin_price(price: f64) -> bool {
    (0.95..=1.05).contains(&price)
}

/// Formats a price in USD with appropriate precision.
/// Stablecoins get extra precision (6 decimals) to show micro-fluctuations.
pub(crate) fn format_price_usd(price: f64) -> String {
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
pub(crate) fn render_candlestick_chart(f: &mut Frame, area: Rect, state: &MonitorState) {
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
pub(crate) fn render_volume_profile_chart(f: &mut Frame, area: Rect, state: &MonitorState) {
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
pub(crate) fn render_volume_chart(f: &mut Frame, area: Rect, state: &MonitorState) {
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
    let volume_str = scope::display::format_usd(current_volume);

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
pub(crate) fn render_buy_sell_gauge(f: &mut Frame, area: Rect, state: &mut MonitorState) {
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
pub(crate) fn render_activity_feed(f: &mut Frame, area: Rect, state: &mut MonitorState) {
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
pub(crate) fn render_alert_overlay(f: &mut Frame, area: Rect, state: &MonitorState) {
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
pub(crate) fn render_liquidity_depth(f: &mut Frame, area: Rect, state: &MonitorState) {
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
            let label = format!(" {} {}", scope::display::format_usd(*liq), name);
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
pub(crate) fn render_metrics_panel(f: &mut Frame, area: Rect, state: &MonitorState) {
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
        .map(scope::display::format_usd)
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
            Span::raw(scope::display::format_usd(state.liquidity_usd)),
        ]),
        Row::new(vec![
            Span::styled("Vol 24h", Style::new().gray()),
            Span::raw(scope::display::format_usd(state.volume_24h)),
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
pub(crate) fn render_order_book_panel(f: &mut Frame, area: Rect, state: &MonitorState) {
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
pub(crate) fn render_recent_trades_panel(f: &mut Frame, area: Rect, state: &MonitorState) {
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
pub(crate) fn render_market_info_panel(f: &mut Frame, area: Rect, state: &MonitorState) {
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
pub(crate) fn render_footer(f: &mut Frame, area: Rect, state: &MonitorState) {
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
pub(crate) fn format_number(n: f64) -> String {
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
