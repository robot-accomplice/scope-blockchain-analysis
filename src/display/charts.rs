//! # ASCII Chart Rendering
//!
//! This module provides ASCII chart rendering for terminal display,
//! similar to the visualization style used by `btm` (bottom).
//!
//! ## Features
//!
//! - Line charts for price history
//! - Bar charts for volume data
//! - Distribution charts for holder concentration
//!
//! ## Usage
//!
//! ```rust
//! use scope::display::charts::{render_price_chart, ChartConfig};
//! use scope::chains::PricePoint;
//!
//! let history = vec![
//!     PricePoint { timestamp: 0, price: 100.0 },
//!     PricePoint { timestamp: 3600, price: 105.0 },
//! ];
//!
//! let chart = render_price_chart(&history, 60, 10);
//! println!("{}", chart);
//! ```

use crate::chains::{PricePoint, TokenHolder, VolumePoint};
use textplots::{Chart, Plot, Shape};

/// Configuration for chart rendering.
#[derive(Debug, Clone)]
pub struct ChartConfig {
    /// Width of the chart in characters.
    pub width: u32,
    /// Height of the chart in characters.
    pub height: u32,
    /// Title for the chart.
    pub title: Option<String>,
    /// Whether to show axis labels.
    pub show_labels: bool,
}

impl Default for ChartConfig {
    fn default() -> Self {
        Self {
            width: 60,
            height: 15,
            title: None,
            show_labels: true,
        }
    }
}

/// Renders a price chart as ASCII art.
///
/// # Arguments
///
/// * `price_history` - Vector of price points over time
/// * `width` - Chart width in characters
/// * `height` - Chart height in characters
///
/// # Returns
///
/// Returns a string containing the ASCII chart.
pub fn render_price_chart(price_history: &[PricePoint], width: u32, height: u32) -> String {
    if price_history.is_empty() {
        return "No price data available".to_string();
    }

    let mut output = String::new();

    // Calculate price range for labels
    let min_price = price_history
        .iter()
        .map(|p| p.price)
        .fold(f64::INFINITY, f64::min);
    let max_price = price_history
        .iter()
        .map(|p| p.price)
        .fold(f64::NEG_INFINITY, f64::max);

    // Calculate time range
    let min_time = price_history.iter().map(|p| p.timestamp).min().unwrap_or(0);
    let max_time = price_history.iter().map(|p| p.timestamp).max().unwrap_or(0);

    // Convert to textplots format (f32 points)
    let points: Vec<(f32, f32)> = price_history
        .iter()
        .map(|p| {
            let x = (p.timestamp - min_time) as f32;
            let y = p.price as f32;
            (x, y)
        })
        .collect();

    if points.is_empty() {
        return "No price data available".to_string();
    }

    // Render chart to string
    let x_max = (max_time - min_time) as f32;
    let x_min = 0.0_f32;

    // Capture chart output
    let chart_str = Chart::new(width, height, x_min, x_max)
        .lineplot(&Shape::Lines(&points))
        .to_string();

    // Format the output with title and labels
    output.push_str(&format!("Price (${:.4} - ${:.4})\n", min_price, max_price));
    output.push_str(&chart_str);

    // Add time labels
    let time_range_hours = (max_time - min_time) as f64 / 3600.0;
    if time_range_hours <= 24.0 {
        output.push_str(&format!(
            " {:>width$}\n",
            format!("{:.0}h ago -> now", time_range_hours),
            width = width as usize - 5
        ));
    } else {
        let days = time_range_hours / 24.0;
        output.push_str(&format!(
            " {:>width$}\n",
            format!("{:.0}d ago -> now", days),
            width = width as usize - 5
        ));
    }

    output
}

/// Renders a volume chart as ASCII art using bar representation.
///
/// # Arguments
///
/// * `volume_history` - Vector of volume points over time
/// * `width` - Chart width in characters
/// * `height` - Chart height in characters
///
/// # Returns
///
/// Returns a string containing the ASCII chart.
pub fn render_volume_chart(volume_history: &[VolumePoint], width: u32, height: u32) -> String {
    if volume_history.is_empty() {
        return "No volume data available".to_string();
    }

    let mut output = String::new();

    // Calculate volume range
    let max_volume = volume_history
        .iter()
        .map(|v| v.volume)
        .fold(f64::NEG_INFINITY, f64::max);

    let total_volume: f64 = volume_history.iter().map(|v| v.volume).sum();

    // Format max volume for display
    let max_vol_formatted = format_large_number(max_volume);
    let total_vol_formatted = format_large_number(total_volume);

    output.push_str(&format!(
        "Volume (max: ${}, total: ${})\n",
        max_vol_formatted, total_vol_formatted
    ));

    // Calculate time range
    let min_time = volume_history
        .iter()
        .map(|v| v.timestamp)
        .min()
        .unwrap_or(0);
    let max_time = volume_history
        .iter()
        .map(|v| v.timestamp)
        .max()
        .unwrap_or(0);

    // Convert to textplots format
    let points: Vec<(f32, f32)> = volume_history
        .iter()
        .map(|v| {
            let x = (v.timestamp - min_time) as f32;
            let y = v.volume as f32;
            (x, y)
        })
        .collect();

    let x_max = (max_time - min_time) as f32;
    let x_min = 0.0_f32;

    // Render as a bar-like chart using points
    let chart_str = Chart::new(width, height, x_min, x_max)
        .lineplot(&Shape::Bars(&points))
        .to_string();

    output.push_str(&chart_str);

    output
}

/// Renders a holder distribution chart as ASCII bars.
///
/// This displays the top holders with horizontal bar representation
/// of their percentage ownership.
///
/// # Arguments
///
/// * `holders` - Vector of token holders sorted by balance
///
/// # Returns
///
/// Returns a string containing the ASCII distribution chart.
pub fn render_holder_distribution(holders: &[TokenHolder]) -> String {
    if holders.is_empty() {
        return "No holder data available".to_string();
    }

    let mut output = String::new();
    output.push_str("Top Holders\n");
    output.push_str(&"=".repeat(50));
    output.push('\n');

    let max_bar_width = 20;

    for holder in holders.iter().take(10) {
        // Truncate address for display (terminal only)
        let addr_display = truncate_address(&holder.address);

        // Calculate bar width based on percentage
        let bar_width = ((holder.percentage / 100.0) * max_bar_width as f64).round() as usize;
        let bar_width = bar_width.min(max_bar_width);

        let filled = "█".repeat(bar_width);
        let empty = "░".repeat(max_bar_width - bar_width);

        output.push_str(&format!(
            "{:>2}. {}  {:>6.2}%  {}{}\n",
            holder.rank, addr_display, holder.percentage, filled, empty
        ));
    }

    // Add concentration summary if we have enough holders
    if holders.len() >= 10 {
        let top_10_total: f64 = holders.iter().take(10).map(|h| h.percentage).sum();
        output.push_str(&"-".repeat(50));
        output.push('\n');
        output.push_str(&format!("Top 10 control: {:.1}% of supply\n", top_10_total));
    }

    output
}

/// Renders a combined analytics dashboard with price, volume, and holder charts.
///
/// # Arguments
///
/// * `price_history` - Price data points
/// * `volume_history` - Volume data points
/// * `holders` - Top token holders
/// * `token_symbol` - Token symbol for the title
/// * `chain` - Chain name for the title
///
/// # Returns
///
/// Returns a formatted string with all charts.
pub fn render_analytics_dashboard(
    price_history: &[PricePoint],
    volume_history: &[VolumePoint],
    holders: &[TokenHolder],
    token_symbol: &str,
    chain: &str,
) -> String {
    let mut output = String::new();

    // Header
    output.push_str(&format!("Token Analytics: {} on {}\n", token_symbol, chain));
    output.push_str(&"=".repeat(60));
    output.push_str("\n\n");

    // Price chart
    if !price_history.is_empty() {
        output.push_str(&render_price_chart(price_history, 60, 12));
        output.push('\n');
    }

    // Volume chart
    if !volume_history.is_empty() {
        output.push_str(&render_volume_chart(volume_history, 60, 8));
        output.push('\n');
    }

    // Holder distribution
    if !holders.is_empty() {
        output.push_str(&render_holder_distribution(holders));
    }

    output
}

/// Truncates an address for terminal display.
fn truncate_address(address: &str) -> String {
    if address.len() <= 13 {
        address.to_string()
    } else {
        format!("{}...{}", &address[..6], &address[address.len() - 4..])
    }
}

/// Formats a large number with K, M, B suffixes.
fn format_large_number(value: f64) -> String {
    if value >= 1_000_000_000.0 {
        format!("{:.2}B", value / 1_000_000_000.0)
    } else if value >= 1_000_000.0 {
        format!("{:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.2}K", value / 1_000.0)
    } else {
        format!("{:.2}", value)
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_price_chart_empty() {
        let result = render_price_chart(&[], 60, 10);
        assert!(result.contains("No price data"));
    }

    #[test]
    fn test_render_price_chart_with_data() {
        let history = vec![
            PricePoint {
                timestamp: 0,
                price: 100.0,
            },
            PricePoint {
                timestamp: 3600,
                price: 105.0,
            },
            PricePoint {
                timestamp: 7200,
                price: 102.0,
            },
        ];

        let result = render_price_chart(&history, 60, 10);
        assert!(!result.is_empty());
        assert!(result.contains("Price"));
    }

    #[test]
    fn test_render_volume_chart_empty() {
        let result = render_volume_chart(&[], 60, 10);
        assert!(result.contains("No volume data"));
    }

    #[test]
    fn test_render_volume_chart_with_data() {
        let history = vec![
            VolumePoint {
                timestamp: 0,
                volume: 1000000.0,
            },
            VolumePoint {
                timestamp: 3600,
                volume: 1500000.0,
            },
        ];

        let result = render_volume_chart(&history, 60, 10);
        assert!(!result.is_empty());
        assert!(result.contains("Volume"));
    }

    #[test]
    fn test_render_holder_distribution_empty() {
        let result = render_holder_distribution(&[]);
        assert!(result.contains("No holder data"));
    }

    #[test]
    fn test_render_holder_distribution_with_data() {
        let holders = vec![
            TokenHolder {
                address: "0x1234567890123456789012345678901234567890".to_string(),
                balance: "1000000".to_string(),
                formatted_balance: "1M".to_string(),
                percentage: 25.0,
                rank: 1,
            },
            TokenHolder {
                address: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd".to_string(),
                balance: "500000".to_string(),
                formatted_balance: "500K".to_string(),
                percentage: 12.5,
                rank: 2,
            },
        ];

        let result = render_holder_distribution(&holders);
        assert!(result.contains("Top Holders"));
        assert!(result.contains("25.00%"));
        assert!(result.contains("12.50%"));
        assert!(result.contains("█")); // Has bar characters
    }

    #[test]
    fn test_truncate_address() {
        let addr = "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2";
        let truncated = truncate_address(addr);
        assert_eq!(truncated, "0x742d...b3c2");

        // Short addresses stay the same
        let short = "0x123";
        assert_eq!(truncate_address(short), "0x123");
    }

    #[test]
    fn test_format_large_number() {
        assert_eq!(format_large_number(1500.0), "1.50K");
        assert_eq!(format_large_number(1500000.0), "1.50M");
        assert_eq!(format_large_number(1500000000.0), "1.50B");
        assert_eq!(format_large_number(500.0), "500.00");
    }

    #[test]
    fn test_chart_config_default() {
        let config = ChartConfig::default();
        assert_eq!(config.width, 60);
        assert_eq!(config.height, 15);
        assert!(config.show_labels);
    }

    #[test]
    fn test_render_analytics_dashboard() {
        let prices = vec![PricePoint {
            timestamp: 0,
            price: 1.0,
        }];
        let volumes = vec![VolumePoint {
            timestamp: 0,
            volume: 1000.0,
        }];
        let holders = vec![TokenHolder {
            address: "0x1234567890123456789012345678901234567890".to_string(),
            balance: "1000".to_string(),
            formatted_balance: "1K".to_string(),
            percentage: 50.0,
            rank: 1,
        }];

        let result = render_analytics_dashboard(&prices, &volumes, &holders, "TEST", "ethereum");
        assert!(result.contains("Token Analytics: TEST on ethereum"));
    }

    #[test]
    fn test_price_chart_multiday_range() {
        // Time range > 24h to trigger the "Xd ago" branch
        let prices: Vec<PricePoint> = (0..50)
            .map(|i| PricePoint {
                timestamp: i * 7200, // every 2 hours, spanning ~4 days
                price: 1.0 + (i as f64) * 0.01,
            })
            .collect();
        let chart = render_price_chart(&prices, 60, 15);
        assert!(chart.contains("d ago -> now"));
    }

    #[test]
    fn test_holder_distribution_with_10_holders() {
        // >= 10 holders triggers concentration summary
        let holders: Vec<TokenHolder> = (1..=12)
            .map(|i| TokenHolder {
                address: format!("0x{:040}", i),
                balance: format!("{}", 1000 - i * 50),
                formatted_balance: format!("{}K", (1000 - i * 50) / 1000),
                percentage: 10.0 - (i as f64) * 0.5,
                rank: i as u32,
            })
            .collect();
        let chart = render_holder_distribution(&holders);
        assert!(chart.contains("Top 10 control:"));
        assert!(chart.contains("% of supply"));
    }
}
