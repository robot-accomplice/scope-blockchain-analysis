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
    section_header_styled(title, is_tty())
}

fn section_header_styled(title: &str, tty: bool) -> String {
    let width: usize = 50;
    let pad = width.saturating_sub(title.len() + 4);
    if tty {
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
    subsection_header_styled(title, is_tty())
}

fn subsection_header_styled(title: &str, tty: bool) -> String {
    if tty {
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
    kv_row_styled(key, value, is_tty())
}

fn kv_row_styled(key: &str, value: &str, tty: bool) -> String {
    if tty {
        format!("{}  {:<18}{}", "│".cyan(), key.dimmed(), value)
    } else {
        format!("│  {:<18}{}", key, value)
    }
}

/// A key-value row where the value is colored based on positive/negative.
pub fn kv_row_delta(key: &str, value: f64, formatted: &str) -> String {
    kv_row_delta_styled(key, value, formatted, is_tty())
}

fn kv_row_delta_styled(key: &str, value: f64, formatted: &str, tty: bool) -> String {
    if tty {
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
    check_pass_styled(msg, is_tty())
}

fn check_pass_styled(msg: &str, tty: bool) -> String {
    if tty {
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
    check_fail_styled(msg, is_tty())
}

fn check_fail_styled(msg: &str, tty: bool) -> String {
    if tty {
        format!("{}  {} {}", "│".cyan(), "✗".red(), msg)
    } else {
        format!("│  ✗ {}", msg)
    }
}

/// Overall status line (healthy / unhealthy).
pub fn status_line(healthy: bool) -> String {
    status_line_styled(healthy, is_tty())
}

fn status_line_styled(healthy: bool, tty: bool) -> String {
    if tty {
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
    section_footer_styled(is_tty())
}

fn section_footer_styled(tty: bool) -> String {
    let line = "─".repeat(50);
    if tty {
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
    separator_styled(is_tty())
}

fn separator_styled(tty: bool) -> String {
    let line = "─".repeat(50);
    if tty {
        format!("{}", format!("├{}", line).cyan())
    } else {
        format!("├{}", line)
    }
}

/// Format a price with color for peg deviation.
/// Green if within 0.1% of target, yellow if within 0.5%, red otherwise.
pub fn format_price_peg(price: f64, target: f64) -> String {
    format_price_peg_styled(price, target, is_tty())
}

fn format_price_peg_styled(price: f64, target: f64, tty: bool) -> String {
    let deviation = ((price - target) / target).abs();
    let text = format!("{:.4}", price);
    if !tty {
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
    blank_row_styled(is_tty())
}

fn blank_row_styled(tty: bool) -> String {
    if tty {
        format!("{}", "│".cyan())
    } else {
        "│".to_string()
    }
}

/// An order book level row with price coloring relative to peg.
pub fn orderbook_level(price: f64, quantity: f64, base: &str, value: f64, peg: f64) -> String {
    orderbook_level_styled(price, quantity, base, value, peg, is_tty())
}

fn orderbook_level_styled(
    price: f64,
    quantity: f64,
    base: &str,
    value: f64,
    peg: f64,
    tty: bool,
) -> String {
    let price_str = format_price_peg_styled(price, peg, tty);
    if tty {
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

    #[test]
    fn test_kv_row_delta_zero_value() {
        // Non-TTY: no color, but zero value (else/dimmed branch) still returns formatted string
        let row = kv_row_delta("Change", 0.0, "0.00%");
        assert!(row.contains("Change"));
        assert!(row.contains("0.00%"));
        assert!(row.contains("│"));
    }

    #[test]
    fn test_format_price_peg_moderate_deviation() {
        // 0.2% deviation: price 1.002, target 1.0 -> in non-TTY returns plain price string
        let s = format_price_peg(1.002, 1.0);
        assert!(s.contains("1.0020"));
    }

    #[test]
    fn test_orderbook_level_various_prices() {
        let row_low = orderbook_level(0.9990, 100.0, "PUSD", 99.90, 1.0);
        let row_mid = orderbook_level(1.0000, 100.0, "PUSD", 100.0, 1.0);
        let row_high = orderbook_level(1.0015, 100.0, "PUSD", 100.15, 1.0);
        assert!(row_low.contains("0.9990"));
        assert!(row_mid.contains("1.0000"));
        assert!(row_high.contains("1.0015"));
        assert!(row_low.contains("│"));
        assert!(row_mid.contains("│"));
        assert!(row_high.contains("│"));
    }

    #[test]
    fn test_non_tty_returns_unicode_box_characters() {
        // In CI (non-TTY), all helpers still emit Unicode box-drawing chars
        let header = section_header("Test");
        let sub = subsection_header("Sub");
        let kv = kv_row("Key", "Val");
        let pass = check_pass("ok");
        let fail = check_fail("err");
        let footer = section_footer();
        let sep = separator();
        let blank = blank_row();
        let status_healthy = status_line(true);
        let status_unhealthy = status_line(false);

        assert!(header.contains('┌'), "section_header should contain ┌");
        assert!(header.contains('─'), "section_header should contain ─");
        assert!(sub.contains('│'), "subsection_header should contain │");
        assert!(sub.contains('├'), "subsection_header should contain ├");
        assert!(kv.contains('│'), "kv_row should contain │");
        assert!(pass.contains('✓'), "check_pass should contain ✓");
        assert!(fail.contains('✗'), "check_fail should contain ✗");
        assert!(footer.contains('└'), "section_footer should contain └");
        assert!(sep.contains('├'), "separator should contain ├");
        assert!(blank.contains('│'), "blank_row should contain │");
        assert!(status_healthy.contains("HEALTHY"));
        assert!(status_unhealthy.contains("UNHEALTHY"));
    }

    // ============================================================
    // TTY-branch tests (via _styled variants with tty=true)
    // ============================================================

    #[test]
    fn test_section_header_tty() {
        let header = section_header_styled("Token Health", true);
        assert!(header.contains("Token Health"));
        assert!(header.contains("┌─"));
    }

    #[test]
    fn test_subsection_header_tty() {
        let header = subsection_header_styled("DEX", true);
        assert!(header.contains("DEX"));
        assert!(header.contains("├──"));
    }

    #[test]
    fn test_kv_row_tty() {
        let row = kv_row_styled("Price", "$1.00", true);
        assert!(row.contains("Price"));
        assert!(row.contains("$1.00"));
    }

    #[test]
    fn test_kv_row_delta_positive_tty() {
        let row = kv_row_delta_styled("Change", 5.0, "+5%", true);
        assert!(row.contains("+5%"));
    }

    #[test]
    fn test_kv_row_delta_negative_tty() {
        let row = kv_row_delta_styled("Change", -3.0, "-3%", true);
        assert!(row.contains("-3%"));
    }

    #[test]
    fn test_kv_row_delta_zero_tty() {
        let row = kv_row_delta_styled("Change", 0.0, "0.00%", true);
        assert!(row.contains("0.00%"));
    }

    #[test]
    fn test_check_pass_tty() {
        let line = check_pass_styled("ok", true);
        assert!(line.contains("✓"));
        assert!(line.contains("ok"));
    }

    #[test]
    fn test_check_fail_tty() {
        let line = check_fail_styled("err", true);
        assert!(line.contains("✗"));
        assert!(line.contains("err"));
    }

    #[test]
    fn test_status_line_healthy_tty() {
        let line = status_line_styled(true, true);
        assert!(line.contains("HEALTHY"));
    }

    #[test]
    fn test_status_line_unhealthy_tty() {
        let line = status_line_styled(false, true);
        assert!(line.contains("UNHEALTHY"));
    }

    #[test]
    fn test_section_footer_tty() {
        let footer = section_footer_styled(true);
        assert!(footer.contains("└"));
    }

    #[test]
    fn test_separator_tty() {
        let sep = separator_styled(true);
        assert!(sep.contains("├"));
    }

    #[test]
    fn test_format_price_peg_tty_near() {
        let s = format_price_peg_styled(1.0001, 1.0, true);
        assert!(s.contains("1.0001"));
    }

    #[test]
    fn test_format_price_peg_tty_moderate() {
        let s = format_price_peg_styled(1.003, 1.0, true);
        assert!(s.contains("1.0030"));
    }

    #[test]
    fn test_format_price_peg_tty_far() {
        let s = format_price_peg_styled(0.95, 1.0, true);
        assert!(s.contains("0.9500"));
    }

    #[test]
    fn test_blank_row_tty() {
        let row = blank_row_styled(true);
        assert!(row.contains("│"));
    }

    #[test]
    fn test_orderbook_level_tty() {
        let row = orderbook_level_styled(1.0001, 500.0, "PUSD", 500.05, 1.0, true);
        assert!(row.contains("PUSD"));
        assert!(row.contains("USDT"));
    }
}
