//! # Terminal Display Helpers
//!
//! Styled output utilities for rich, user-friendly terminal presentation.
//! Uses `owo-colors` for color and `unicode` box-drawing characters for structure.
//! All helpers respect non-TTY contexts (piped output) by falling back to plain text.

use owo_colors::OwoColorize;
use std::io::IsTerminal;

// ============================================================================
// Color-aware helpers
// ============================================================================

/// Returns `true` when stdout is an interactive terminal (not piped).
pub fn is_tty() -> bool {
    std::io::stdout().is_terminal()
}

/// Section header with colored title and box-drawing underline.
///
/// ```text
/// ┌─ Token Health ─────────────────────────────
/// ```
pub fn section_header(title: &str) -> String {
    let width: usize = 50;
    let pad = width.saturating_sub(title.len() + 4);
    if is_tty() {
        format!(
            "{}",
            format!("\n┌─ {} {}", title.bold(), "─".repeat(pad)).cyan()
        )
    } else {
        format!("\n┌─ {} {}", title, "─".repeat(pad))
    }
}

/// Sub-section header (lighter weight).
///
/// ```text
/// │
/// ├── DEX Analytics
/// ```
pub fn subsection_header(title: &str) -> String {
    if is_tty() {
        format!("{}\n{}", "│".cyan(), format!("├── {}", title.bold()).cyan())
    } else {
        format!("│\n├── {}", title)
    }
}

/// A key-value row inside a section, with aligned values.
///
/// ```text
/// │  Price            $0.9999
/// ```
pub fn kv_row(key: &str, value: &str) -> String {
    if is_tty() {
        format!("{}  {:<18}{}", "│".cyan(), key.dimmed(), value)
    } else {
        format!("│  {:<18}{}", key, value)
    }
}

/// A key-value row where the value is colored based on positive/negative.
pub fn kv_row_delta(key: &str, value: f64, formatted: &str) -> String {
    if is_tty() {
        let colored_val = if value > 0.0 {
            format!("{}", formatted.green())
        } else if value < 0.0 {
            format!("{}", formatted.red())
        } else {
            format!("{}", formatted.dimmed())
        };
        format!("{}  {:<18}{}", "│".cyan(), key.dimmed(), colored_val)
    } else {
        format!("│  {:<18}{}", key, formatted)
    }
}

/// Health check pass line.
///
/// ```text
/// │  ✓ No sells below peg
/// ```
pub fn check_pass(msg: &str) -> String {
    if is_tty() {
        format!("{}  {} {}", "│".cyan(), "✓".green(), msg)
    } else {
        format!("│  ✓ {}", msg)
    }
}

/// Health check fail line.
///
/// ```text
/// │  ✗ Bid depth: 0 USDT < 3000 USDT minimum
/// ```
pub fn check_fail(msg: &str) -> String {
    if is_tty() {
        format!("{}  {} {}", "│".cyan(), "✗".red(), msg)
    } else {
        format!("│  ✗ {}", msg)
    }
}

/// Overall status line (healthy / unhealthy).
pub fn status_line(healthy: bool) -> String {
    if is_tty() {
        if healthy {
            format!("{}  {}", "│".cyan(), "HEALTHY".green().bold())
        } else {
            format!("{}  {}", "│".cyan(), "UNHEALTHY".red().bold())
        }
    } else if healthy {
        "│  HEALTHY".to_string()
    } else {
        "│  UNHEALTHY".to_string()
    }
}

/// Section footer (closing box line).
///
/// ```text
/// └──────────────────────────────────────────────
/// ```
pub fn section_footer() -> String {
    let line = "─".repeat(50);
    if is_tty() {
        format!("{}", format!("└{}", line).cyan())
    } else {
        format!("└{}", line)
    }
}

/// A separator row inside a section.
///
/// ```text
/// ├──────────────────────────────────────────────
/// ```
pub fn separator() -> String {
    let line = "─".repeat(50);
    if is_tty() {
        format!("{}", format!("├{}", line).cyan())
    } else {
        format!("├{}", line)
    }
}

/// Format a price with color for peg deviation.
/// Green if within 0.1% of target, yellow if within 0.5%, red otherwise.
pub fn format_price_peg(price: f64, target: f64) -> String {
    let deviation = ((price - target) / target).abs();
    let text = format!("{:.4}", price);
    if !is_tty() {
        return text;
    }
    if deviation < 0.001 {
        format!("{}", text.green())
    } else if deviation < 0.005 {
        format!("{}", text.yellow())
    } else {
        format!("{}", text.red())
    }
}

/// Empty line with continuation bar.
pub fn blank_row() -> String {
    if is_tty() {
        format!("{}", "│".cyan())
    } else {
        "│".to_string()
    }
}

/// An order book level row with price coloring relative to peg.
pub fn orderbook_level(price: f64, quantity: f64, base: &str, value: f64, peg: f64) -> String {
    let price_str = format_price_peg(price, peg);
    if is_tty() {
        format!(
            "{}    {}  {:>10.2} {}  {:>10.2} USDT",
            "│".cyan(),
            price_str,
            quantity,
            base.dimmed(),
            value
        )
    } else {
        format!(
            "│    {:.4}  {:>10.2} {}  {:>10.2} USDT",
            price, quantity, base, value
        )
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_section_header_contains_title() {
        let header = section_header("Token Health");
        assert!(header.contains("Token Health"));
        assert!(header.contains("┌─"));
    }

    #[test]
    fn test_subsection_header_contains_title() {
        let header = subsection_header("DEX Analytics");
        assert!(header.contains("DEX Analytics"));
        assert!(header.contains("├──"));
    }

    #[test]
    fn test_kv_row_contains_key_value() {
        let row = kv_row("Price", "$1.00");
        assert!(row.contains("Price"));
        assert!(row.contains("$1.00"));
        assert!(row.contains("│"));
    }

    #[test]
    fn test_kv_row_delta_positive() {
        let row = kv_row_delta("24h Change", 5.0, "+5.00%");
        assert!(row.contains("+5.00%"));
    }

    #[test]
    fn test_kv_row_delta_negative() {
        let row = kv_row_delta("24h Change", -3.0, "-3.00%");
        assert!(row.contains("-3.00%"));
    }

    #[test]
    fn test_check_pass() {
        let line = check_pass("No sells below peg");
        assert!(line.contains("✓"));
        assert!(line.contains("No sells below peg"));
    }

    #[test]
    fn test_check_fail() {
        let line = check_fail("Bid depth too low");
        assert!(line.contains("✗"));
        assert!(line.contains("Bid depth too low"));
    }

    #[test]
    fn test_status_line_healthy() {
        let line = status_line(true);
        assert!(line.contains("HEALTHY"));
    }

    #[test]
    fn test_status_line_unhealthy() {
        let line = status_line(false);
        assert!(line.contains("UNHEALTHY"));
    }

    #[test]
    fn test_section_footer() {
        let footer = section_footer();
        assert!(footer.contains("└"));
    }

    #[test]
    fn test_separator() {
        let sep = separator();
        assert!(sep.contains("├"));
    }

    #[test]
    fn test_format_price_peg_near() {
        let s = format_price_peg(1.0001, 1.0);
        assert!(s.contains("1.0001"));
    }

    #[test]
    fn test_format_price_peg_far() {
        let s = format_price_peg(0.95, 1.0);
        assert!(s.contains("0.9500"));
    }

    #[test]
    fn test_blank_row() {
        let row = blank_row();
        assert!(row.contains("│"));
    }

    #[test]
    fn test_orderbook_level() {
        let row = orderbook_level(1.0001, 500.0, "PUSD", 500.05, 1.0);
        assert!(row.contains("PUSD"));
        assert!(row.contains("USDT"));
    }
}
