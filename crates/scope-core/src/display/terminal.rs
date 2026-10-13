//! # Terminal Display Helpers
//!
//! Styled output utilities for rich, user-friendly terminal presentation.
//! Text is painted by semantic role through [`super::theme`], with
//! `unicode` box-drawing characters for structure.
//! All helpers respect non-TTY contexts (piped output) by falling back to plain text.
//! Long text is automatically word-wrapped to the detected terminal width.

use super::theme::{Painter, Role};
use std::sync::atomic::{AtomicUsize, Ordering};
use unicode_width::UnicodeWidthStr;

// ============================================================================
// Terminal detection
// ============================================================================

/// The color decision lives in the theme; these paths stay for callers.
pub use super::theme::{
    color_enabled, color_enabled_stderr, color_enabled_with, set_color_override,
};

/// Width override in columns: 0 = none. The TUI sets it to the width of
/// its output pane, so command output fits the pane.
static WIDTH_OVERRIDE: AtomicUsize = AtomicUsize::new(0);

/// Sets the width that [`terminal_width`] returns for the whole process,
/// or removes the override (`None`). `Some(0)` also removes it. The TUI
/// removes it each time it gives the terminal back.
pub fn set_width_override(width: Option<usize>) {
    WIDTH_OVERRIDE.store(width.unwrap_or(0), Ordering::Relaxed);
}

/// The width override, if one is set.
pub fn width_override() -> Option<usize> {
    match WIDTH_OVERRIDE.load(Ordering::Relaxed) {
        0 => None,
        w => Some(w),
    }
}

/// Returns the current terminal width in columns.
///
/// Uses the width override when one is set (the TUI pane). Otherwise uses
/// `crossterm::terminal::size()` when available, and falls back to 80
/// columns.
pub fn terminal_width() -> usize {
    width_or_real(width_override())
}

/// `over`, or the real terminal width when it is `None`.
fn width_or_real(over: Option<usize>) -> usize {
    over.unwrap_or_else(|| {
        crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(80)
    })
}

/// Serialises the tests that set the width override or depend on
/// `terminal_width()`.
#[cfg(test)]
pub(crate) fn width_lock() -> std::sync::MutexGuard<'static, ()> {
    static WIDTH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    WIDTH_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// The widest rule, in columns, that the section helpers draw.
const RULE_WIDTH: usize = 50;

/// The `─` run of a rule of `len` columns that starts after `prefix_cols`
/// columns. Only the width override (`cap`, the TUI pane) shortens it:
/// with no override a rule keeps its fixed width, so CLI and piped output
/// stay the same at every terminal width.
fn rule_at(len: usize, prefix_cols: usize, cap: Option<usize>) -> String {
    let len = match cap {
        Some(w) => len.min(w.saturating_sub(prefix_cols)),
        None => len,
    };
    "─".repeat(len)
}

// ============================================================================
// Word-wrapping engine
// ============================================================================

/// Word-wrap `text` so each line fits within `content_width` columns.
///
/// Returns a `Vec<String>` where the first element is the first line of text
/// and subsequent elements are continuation lines. Words are split on whitespace;
/// a word longer than the content width is kept on its own line unbroken (e.g. URLs).
///
/// This function operates on the raw text — callers apply the box-drawing prefix
/// and styling to each returned line.
pub fn wrap_lines(text: &str, content_width: usize) -> Vec<String> {
    if content_width == 0 {
        return vec![text.to_string()];
    }

    let mut lines: Vec<String> = Vec::new();
    let mut current_line = String::new();
    let mut current_len = 0usize;

    for word in text.split_whitespace() {
        let word_len = word.width();

        if current_len == 0 {
            // First word on the line — always accept it even if it exceeds width
            current_line.push_str(word);
            current_len = word_len;
        } else if current_len + 1 + word_len <= content_width {
            // Fits on current line with a space
            current_line.push(' ');
            current_line.push_str(word);
            current_len += 1 + word_len;
        } else {
            // Doesn't fit — wrap to next line
            lines.push(current_line);
            current_line = word.to_string();
            current_len = word_len;
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

/// Compute the available content width for a given prefix.
///
/// `prefix_width` is the number of visible columns consumed by the prefix
/// (e.g. "│  " = 3, "│      " = 7, "│    • " = 6).
/// Returns the remaining columns for text content.
fn content_width_for(prefix_width: usize) -> usize {
    terminal_width().saturating_sub(prefix_width)
}

// ============================================================================
// Color-aware helpers
// ============================================================================

/// The visible columns of a marked row's prefix: `"│  ✓ "`.
const MARK_PREFIX_COLS: usize = 5;

/// The visible width of a key or label column.
const LABEL_COLS: usize = 18;

/// The vertical rule that opens every row inside a section.
fn rail(p: Painter) -> String {
    p.paint(Role::Accent, "│")
}

/// A key or label padded to [`LABEL_COLS`], in the secondary tone. The
/// padding sits inside the paint, so colored and plain rows align alike.
fn label(p: Painter, text: &str) -> String {
    p.paint(Role::Secondary, &format!("{:<LABEL_COLS$}", text))
}

/// A row that starts with a glyph (`✓`, `✗`, `!`, `–`, `⚠`, `ℹ`), its
/// text wrapped under the glyph in `text_role`.
fn marked_row(p: Painter, glyph: String, text_role: Role, msg: &str) -> String {
    let wrapped = wrap_lines(msg, content_width_for(MARK_PREFIX_COLS));
    let mut out = format!("{}  {} {}", rail(p), glyph, p.paint(text_role, &wrapped[0]));
    for line in &wrapped[1..] {
        out.push_str(&format!("\n{}    {}", rail(p), p.paint(text_role, line)));
    }
    out
}

/// Section header with colored title and box-drawing underline.
///
/// ```text
/// ┌─ Token Health ─────────────────────────────
/// ```
pub fn section_header(title: &str) -> String {
    section_header_styled(title, color_enabled())
}

fn section_header_styled(title: &str, tty: bool) -> String {
    section_header_at(title, tty, width_override())
}

/// `section_header_styled` with the width cap given.
fn section_header_at(title: &str, tty: bool, cap: Option<usize>) -> String {
    let p = Painter::new(tty);
    // "┌─ " + title + " " come before the rule.
    let prefix = title.width() + 4;
    let pad = rule_at(RULE_WIDTH.saturating_sub(prefix), prefix, cap);
    format!(
        "\n{} {} {}",
        p.paint(Role::Accent, "┌─"),
        p.paint_strong(Role::Accent, title),
        p.paint(Role::Accent, &pad)
    )
}

/// Sub-section header (lighter weight).
///
/// ```text
/// │
/// ├── DEX Analytics
/// ```
pub fn subsection_header(title: &str) -> String {
    subsection_header_styled(title, color_enabled())
}

fn subsection_header_styled(title: &str, tty: bool) -> String {
    let p = Painter::new(tty);
    format!(
        "{}\n{}{}",
        rail(p),
        p.paint(Role::Accent, "├── "),
        p.paint_strong(Role::Accent, title)
    )
}

/// A key-value row inside a section, with aligned values.
///
/// ```text
/// │  Price            $0.9999
/// ```
pub fn kv_row(key: &str, value: &str) -> String {
    kv_row_styled(key, value, color_enabled())
}

fn kv_row_styled(key: &str, value: &str, tty: bool) -> String {
    let p = Painter::new(tty);
    // Prefix: "│  " (3) + key pad (18) = 21 visible columns before value starts
    let prefix_cols = 3 + LABEL_COLS;
    let avail = terminal_width().saturating_sub(prefix_cols);
    let wrapped = wrap_lines(value, avail);

    let mut out = format!("{}  {}{}", rail(p), label(p, key), wrapped[0]);
    // Continuation lines align under the value column
    for line in &wrapped[1..] {
        out.push_str(&format!("\n{}  {:<LABEL_COLS$}{}", rail(p), "", line));
    }
    out
}

/// A key-value row where the value is colored based on positive/negative.
pub fn kv_row_delta(key: &str, value: f64, formatted: &str) -> String {
    kv_row_delta_styled(key, value, formatted, color_enabled())
}

fn kv_row_delta_styled(key: &str, value: f64, formatted: &str, tty: bool) -> String {
    let p = Painter::new(tty);
    let role = if value > 0.0 {
        Role::Ok
    } else if value < 0.0 {
        Role::Err
    } else {
        Role::Secondary
    };
    format!("{}  {}{}", rail(p), label(p, key), p.paint(role, formatted))
}

/// Health check pass line.
///
/// ```text
/// │  ✓ No sells below peg
/// ```
pub fn check_pass(msg: &str) -> String {
    check_pass_styled(msg, color_enabled())
}

fn check_pass_styled(msg: &str, tty: bool) -> String {
    let p = Painter::new(tty);
    marked_row(p, p.paint(Role::Ok, "✓"), Role::Neutral, msg)
}

/// Health check fail line.
///
/// ```text
/// │  ✗ Bid depth: 0 USDT < 3000 USDT minimum
/// ```
pub fn check_fail(msg: &str) -> String {
    check_fail_styled(msg, color_enabled())
}

fn check_fail_styled(msg: &str, tty: bool) -> String {
    let p = Painter::new(tty);
    marked_row(p, p.paint(Role::Err, "✗"), Role::Neutral, msg)
}

/// Formats a check that does not apply (dim dash, message in grey).
///
/// ```text
/// │  – Spread: n/a (synthetic AMM book: the real spread is the pool fee)
/// ```
pub fn check_na(msg: &str) -> String {
    check_na_styled(msg, color_enabled())
}

fn check_na_styled(msg: &str, tty: bool) -> String {
    let p = Painter::new(tty);
    marked_row(p, p.paint(Role::Secondary, "–"), Role::Secondary, msg)
}

/// A check row for a warning: outside its target, but not a failure.
///
/// ```text
/// │  ! Depth/volume: 27.8% (13344 / 47929 USDT within ±2%; target 5–10%)
/// ```
pub fn check_warn(msg: &str) -> String {
    check_warn_styled(msg, color_enabled())
}

fn check_warn_styled(msg: &str, tty: bool) -> String {
    let p = Painter::new(tty);
    marked_row(p, p.paint_strong(Role::Warn, "!"), Role::Warn, msg)
}

/// Overall status line (healthy / unhealthy).
pub fn status_line(healthy: bool) -> String {
    status_line_styled(healthy, color_enabled())
}

fn status_line_styled(healthy: bool, tty: bool) -> String {
    let p = Painter::new(tty);
    let verdict = if healthy {
        p.paint_strong(Role::Ok, "HEALTHY")
    } else {
        p.paint_strong(Role::Err, "UNHEALTHY")
    };
    format!("{}  {}", rail(p), verdict)
}

/// Section footer (closing box line).
///
/// ```text
/// └──────────────────────────────────────────────
/// ```
pub fn section_footer() -> String {
    section_footer_styled(color_enabled())
}

fn section_footer_styled(tty: bool) -> String {
    section_footer_at(tty, width_override())
}

/// `section_footer_styled` with the width cap given.
fn section_footer_at(tty: bool, cap: Option<usize>) -> String {
    let line = rule_at(RULE_WIDTH, 1, cap);
    Painter::new(tty).paint(Role::Accent, &format!("└{}", line))
}

/// A separator row inside a section.
///
/// ```text
/// ├──────────────────────────────────────────────
/// ```
pub fn separator() -> String {
    separator_styled(color_enabled())
}

fn separator_styled(tty: bool) -> String {
    separator_at(tty, width_override())
}

/// `separator_styled` with the width cap given.
fn separator_at(tty: bool, cap: Option<usize>) -> String {
    let line = rule_at(RULE_WIDTH, 1, cap);
    Painter::new(tty).paint(Role::Accent, &format!("├{}", line))
}

/// Format a price with color for peg deviation.
/// Green if within 0.1% of target, yellow if within 0.5%, red otherwise.
pub fn format_price_peg(price: f64, target: f64) -> String {
    format_price_peg_styled(price, target, color_enabled())
}

fn format_price_peg_styled(price: f64, target: f64, tty: bool) -> String {
    let deviation = ((price - target) / target).abs();
    let role = if deviation < 0.001 {
        Role::Ok
    } else if deviation < 0.005 {
        Role::Warn
    } else {
        Role::Err
    };
    Painter::new(tty).paint(role, &format!("{:.4}", price))
}

/// Empty line with continuation bar.
pub fn blank_row() -> String {
    blank_row_styled(color_enabled())
}

fn blank_row_styled(tty: bool) -> String {
    rail(Painter::new(tty))
}

/// An order book level row with price coloring relative to peg.
pub fn orderbook_level(price: f64, quantity: f64, base: &str, value: f64, peg: f64) -> String {
    orderbook_level_styled(price, quantity, base, value, peg, color_enabled())
}

fn orderbook_level_styled(
    price: f64,
    quantity: f64,
    base: &str,
    value: f64,
    peg: f64,
    tty: bool,
) -> String {
    let p = Painter::new(tty);
    format!(
        "{}    {}  {:>10.2} {}  {:>10.2} USDT",
        rail(p),
        format_price_peg_styled(price, peg, tty),
        quantity,
        p.paint(Role::Secondary, base),
        value
    )
}

// ============================================================================
// Score / severity / link helpers
// ============================================================================

/// A visual score bar with color coding.
///
/// ```text
/// │  Security Score   [████████████────────] 60/100
/// ```
pub fn score_bar(label: &str, score: u32, max: u32) -> String {
    score_bar_styled(label, score, max, color_enabled())
}

fn score_bar_styled(label_text: &str, score: u32, max: u32, tty: bool) -> String {
    let p = Painter::new(tty);
    let width = 20usize;
    let filled = ((score as f64 / max as f64) * width as f64).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    let bar = format!(
        "[{}{}] {}/{}",
        "█".repeat(filled),
        "─".repeat(empty),
        score,
        max
    );
    let role = if score >= 80 {
        Role::Ok
    } else if score >= 50 {
        Role::Warn
    } else {
        Role::Err
    };
    format!(
        "{}  {}{}",
        rail(p),
        label(p, label_text),
        p.paint(role, &bar)
    )
}

/// Severity label with color coding.
///
/// Returns a colored severity string: Critical (red bold), High (red),
/// Medium (yellow), Low (cyan), Informational (dimmed).
pub fn severity_label(severity: &str) -> String {
    severity_label_styled(severity, color_enabled())
}

fn severity_label_styled(severity: &str, tty: bool) -> String {
    let p = Painter::new(tty);
    match severity.to_lowercase().as_str() {
        "critical" => p.paint_strong(Role::Err, severity),
        "high" => p.paint(Role::Err, severity),
        "medium" => p.paint(Role::Warn, severity),
        "low" => p.paint(Role::Accent, severity),
        _ => p.paint(Role::Secondary, severity),
    }
}

/// A warning banner inside a section box.
///
/// ```text
/// │  ⚠ WARNING: Source code is NOT verified
/// ```
pub fn warning_row(msg: &str) -> String {
    warning_row_styled(msg, color_enabled())
}

fn warning_row_styled(msg: &str, tty: bool) -> String {
    let p = Painter::new(tty);
    marked_row(p, p.paint_strong(Role::Warn, "⚠"), Role::Warn, msg)
}

/// An informational note row inside a section box.
///
/// ```text
/// │  ℹ No heuristic findings triggered.
/// ```
pub fn info_row(msg: &str) -> String {
    info_row_styled(msg, color_enabled())
}

fn info_row_styled(msg: &str, tty: bool) -> String {
    let p = Painter::new(tty);
    marked_row(p, p.paint(Role::Info, "ℹ"), Role::Secondary, msg)
}

/// A link row inside a section box.
///
/// ```text
/// │  Explorer          https://etherscan.io/address/0x...
/// ```
pub fn link_row(label: &str, url: &str) -> String {
    link_row_styled(label, url, color_enabled())
}

fn link_row_styled(label_text: &str, url: &str, tty: bool) -> String {
    let p = Painter::new(tty);
    format!(
        "{}  {}{}",
        rail(p),
        label(p, label_text),
        p.paint(Role::Link, url)
    )
}

/// An indented detail line inside a section (for sub-items like vulnerability descriptions).
///
/// ```text
/// │      Contract source code is not verified.
/// ```
pub fn detail_row(msg: &str) -> String {
    detail_row_styled(msg, color_enabled())
}

fn detail_row_styled(msg: &str, tty: bool) -> String {
    let p = Painter::new(tty);
    // Prefix: "│      " = 7 visible columns
    let avail = content_width_for(7);
    let wrapped = wrap_lines(msg, avail);

    let mut out = String::new();
    for (i, line) in wrapped.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "{}      {}",
            rail(p),
            p.paint(Role::Secondary, line)
        ));
    }
    out
}

/// A bullet-point row inside a section.
///
/// ```text
/// │    • mint (Critical): Can mint tokens
/// ```
pub fn bullet_row(msg: &str) -> String {
    bullet_row_styled(msg, color_enabled())
}

fn bullet_row_styled(msg: &str, tty: bool) -> String {
    let p = Painter::new(tty);
    // Prefix: "│    • " = 7 visible columns
    let avail = content_width_for(7);
    let wrapped = wrap_lines(msg, avail);

    let mut out = format!(
        "{}    {} {}",
        rail(p),
        p.paint(Role::Secondary, "•"),
        wrapped[0]
    );
    for line in &wrapped[1..] {
        out.push_str(&format!("\n{}      {}", rail(p), line));
    }
    out
}

// ============================================================================
// Table helpers (for columnar data inside sections)
// ============================================================================

/// Column specification for table formatting.
pub struct Col<'a> {
    /// Column header text.
    pub label: &'a str,
    /// Minimum width in characters.
    pub width: usize,
    /// Alignment: `'<'` for left, `'>'` for right.
    pub align: char,
}

/// Format a table header row inside a section box.
///
/// ```text
/// │    Rank      Percent                Balance  Address
/// │  ─────────────────────────────────────────────────────
/// ```
pub fn table_header(cols: &[Col]) -> String {
    table_header_styled(cols, color_enabled())
}

fn table_header_styled(cols: &[Col], tty: bool) -> String {
    table_header_at(cols, tty, width_override())
}

/// `table_header_styled` with the width cap given.
fn table_header_at(cols: &[Col], tty: bool, cap: Option<usize>) -> String {
    let p = Painter::new(tty);
    let mut header = String::new();
    for col in cols {
        if col.align == '>' {
            header.push_str(&format!("{:>width$}  ", col.label, width = col.width));
        } else {
            header.push_str(&format!("{:<width$}  ", col.label, width = col.width));
        }
    }
    let header = header.trim_end().to_string();
    let rule_len = cols.iter().map(|c| c.width + 2).sum::<usize>();
    let line = rule_at(rule_len, 3, cap);

    format!(
        "{}    {}\n{}  {}",
        rail(p),
        p.paint_strong(Role::Accent, &header),
        rail(p),
        p.paint(Role::Accent, &line)
    )
}

/// Format a table data row inside a section box.
///
/// Each value is aligned according to the column specification.
///
/// ```text
/// │       1     12.50%         1,000,000  0xdAC1...1ec7
/// ```
pub fn table_row(cols: &[Col], values: &[&str]) -> String {
    table_row_styled(cols, values, color_enabled())
}

fn table_row_styled(cols: &[Col], values: &[&str], tty: bool) -> String {
    let mut row = String::new();
    for (i, col) in cols.iter().enumerate() {
        let val = values.get(i).copied().unwrap_or("");
        if col.align == '>' {
            row.push_str(&format!("{:>width$}  ", val, width = col.width));
        } else {
            row.push_str(&format!("{:<width$}  ", val, width = col.width));
        }
    }
    let row = row.trim_end().to_string();
    format!("{}    {}", rail(Painter::new(tty)), row)
}

/// Format an enumerated list item inside a section box.
///
/// ```text
/// │  1. Uniswap ETH/USDC - $1.2M ($500K liq)
/// ```
pub fn numbered_row(index: usize, msg: &str) -> String {
    numbered_row_styled(index, msg, color_enabled())
}

fn numbered_row_styled(index: usize, msg: &str, tty: bool) -> String {
    let p = Painter::new(tty);
    // Prefix: "│  N. " = ~6 visible columns (varies with digit count)
    let prefix_len = 4 + format!("{}.", index).len();
    let avail = terminal_width().saturating_sub(prefix_len);
    let wrapped = wrap_lines(msg, avail);

    let num = format!("{}.", index);
    let mut out = format!(
        "{}  {} {}",
        rail(p),
        p.paint(Role::Secondary, &num),
        wrapped[0]
    );
    let cont_pad = " ".repeat(num.len() + 1);
    for line in &wrapped[1..] {
        out.push_str(&format!("\n{}  {}{}", rail(p), cont_pad, line));
    }
    out
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod golden_tests;

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
        let row = orderbook_level(1.0001, 500.0, "DAI", 500.05, 1.0);
        assert!(row.contains("DAI"));
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
        let row_low = orderbook_level(0.9990, 100.0, "DAI", 99.90, 1.0);
        let row_mid = orderbook_level(1.0000, 100.0, "DAI", 100.0, 1.0);
        let row_high = orderbook_level(1.0015, 100.0, "DAI", 100.15, 1.0);
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
        let row = orderbook_level_styled(1.0001, 500.0, "DAI", 500.05, 1.0, true);
        assert!(row.contains("DAI"));
        assert!(row.contains("USDT"));
    }

    // ============================================================
    // Score / severity / link helper tests (non-TTY)
    // ============================================================

    #[test]
    fn test_score_bar_high() {
        let bar = score_bar("Security Score", 85, 100);
        assert!(bar.contains("85/100"));
        assert!(bar.contains("│"));
        assert!(bar.contains("█"));
    }

    #[test]
    fn test_score_bar_low() {
        let bar = score_bar("Security Score", 20, 100);
        assert!(bar.contains("20/100"));
    }

    #[test]
    fn test_score_bar_zero() {
        let bar = score_bar("Score", 0, 100);
        assert!(bar.contains("0/100"));
        assert!(bar.contains("│"));
    }

    #[test]
    fn test_score_bar_max() {
        let bar = score_bar("Score", 100, 100);
        assert!(bar.contains("100/100"));
    }

    #[test]
    fn test_severity_label_critical() {
        let label = severity_label("Critical");
        assert!(label.contains("Critical"));
    }

    #[test]
    fn test_severity_label_high() {
        let label = severity_label("High");
        assert!(label.contains("High"));
    }

    #[test]
    fn test_severity_label_medium() {
        let label = severity_label("Medium");
        assert!(label.contains("Medium"));
    }

    #[test]
    fn test_severity_label_low() {
        let label = severity_label("Low");
        assert!(label.contains("Low"));
    }

    #[test]
    fn test_severity_label_informational() {
        let label = severity_label("Informational");
        assert!(label.contains("Informational"));
    }

    #[test]
    fn test_warning_row() {
        let row = warning_row("Source code is NOT verified");
        assert!(row.contains("⚠"));
        assert!(row.contains("Source code is NOT verified"));
        assert!(row.contains("│"));
    }

    #[test]
    fn test_info_row() {
        let row = info_row("No findings triggered");
        assert!(row.contains("ℹ"));
        assert!(row.contains("No findings triggered"));
        assert!(row.contains("│"));
    }

    #[test]
    fn test_link_row() {
        let row = link_row("Explorer", "https://etherscan.io");
        assert!(row.contains("Explorer"));
        assert!(row.contains("https://etherscan.io"));
        assert!(row.contains("│"));
    }

    #[test]
    fn test_detail_row() {
        let row = detail_row("Contract is not verified");
        assert!(row.contains("Contract is not verified"));
        assert!(row.contains("│"));
    }

    #[test]
    fn test_bullet_row() {
        let row = bullet_row("mint (Critical): Can mint tokens");
        assert!(row.contains("•"));
        assert!(row.contains("mint (Critical): Can mint tokens"));
        assert!(row.contains("│"));
    }

    // ============================================================
    // Score / severity / link helper tests (TTY)
    // ============================================================

    #[test]
    fn test_score_bar_tty_high() {
        let bar = score_bar_styled("Security Score", 85, 100, true);
        assert!(bar.contains("85/100"));
        assert!(bar.contains("█"));
    }

    #[test]
    fn test_score_bar_tty_medium() {
        let bar = score_bar_styled("Security Score", 55, 100, true);
        assert!(bar.contains("55/100"));
    }

    #[test]
    fn test_score_bar_tty_low() {
        let bar = score_bar_styled("Security Score", 20, 100, true);
        assert!(bar.contains("20/100"));
    }

    #[test]
    fn test_severity_label_tty_critical() {
        let label = severity_label_styled("Critical", true);
        assert!(label.contains("Critical"));
    }

    #[test]
    fn test_severity_label_tty_low() {
        let label = severity_label_styled("Low", true);
        assert!(label.contains("Low"));
    }

    #[test]
    fn test_severity_label_tty_unknown() {
        let label = severity_label_styled("Unknown", true);
        assert!(label.contains("Unknown"));
    }

    #[test]
    fn test_warning_row_tty() {
        let row = warning_row_styled("Alert!", true);
        assert!(row.contains("⚠"));
        assert!(row.contains("Alert!"));
    }

    #[test]
    fn test_info_row_tty() {
        let row = info_row_styled("Note", true);
        assert!(row.contains("ℹ"));
        assert!(row.contains("Note"));
    }

    #[test]
    fn test_link_row_tty() {
        let row = link_row_styled("Explorer", "https://example.com", true);
        assert!(row.contains("Explorer"));
        assert!(row.contains("https://example.com"));
    }

    #[test]
    fn test_detail_row_tty() {
        let row = detail_row_styled("Some detail", true);
        assert!(row.contains("Some detail"));
        assert!(row.contains("│"));
    }

    #[test]
    fn test_bullet_row_tty() {
        let row = bullet_row_styled("Item one", true);
        assert!(row.contains("•"));
        assert!(row.contains("Item one"));
    }

    // ============================================================
    // Word-wrapping engine tests
    // ============================================================

    #[test]
    fn test_wrap_lines_short_text() {
        let lines = wrap_lines("hello world", 80);
        assert_eq!(lines, vec!["hello world"]);
    }

    #[test]
    fn test_wrap_lines_exact_fit() {
        let lines = wrap_lines("abcde fghij", 11);
        assert_eq!(lines, vec!["abcde fghij"]);
    }

    #[test]
    fn test_wrap_lines_wraps_at_word_boundary() {
        let lines = wrap_lines("hello world foo", 11);
        assert_eq!(lines, vec!["hello world", "foo"]);
    }

    #[test]
    fn test_wrap_lines_multiple_wraps() {
        let lines = wrap_lines("a b c d e f g", 3);
        assert_eq!(lines, vec!["a b", "c d", "e f", "g"]);
    }

    #[test]
    fn test_wrap_lines_long_word_exceeds_width() {
        // A single word longer than the width stays on its own line unbroken
        let lines = wrap_lines(
            "https://etherscan.io/address/0xdAC17F958D2ee523a2206206994597C13D831ec7",
            40,
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("https://"));
    }

    #[test]
    fn test_wrap_lines_long_word_after_short() {
        let lines = wrap_lines(
            "Explorer: https://etherscan.io/address/0xdAC17F958D2ee523a2206206994597C13D831ec7",
            30,
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "Explorer:");
        assert!(lines[1].starts_with("https://"));
    }

    #[test]
    fn test_wrap_lines_empty_string() {
        let lines = wrap_lines("", 80);
        assert_eq!(lines, vec![""]);
    }

    #[test]
    fn test_wrap_lines_zero_width() {
        let lines = wrap_lines("hello", 0);
        assert_eq!(lines, vec!["hello"]);
    }

    #[test]
    fn test_wrap_lines_single_word() {
        let lines = wrap_lines("hello", 80);
        assert_eq!(lines, vec!["hello"]);
    }

    #[test]
    fn test_wrap_lines_preserves_all_words() {
        let input = "Contract source code is not verified. Full vulnerability analysis requires verified source code.";
        let lines = wrap_lines(input, 40);
        let reassembled: String = lines.join(" ");
        assert_eq!(reassembled, input);
    }

    #[test]
    fn test_terminal_width_returns_positive() {
        let _lock = width_lock();
        let w = terminal_width();
        assert!(w > 0);
    }

    #[test]
    fn test_content_width_for_reasonable_prefix() {
        let _lock = width_lock();
        let w = content_width_for(7);
        // Should be terminal_width - 7; at minimum > 0 even in CI
        assert!(w > 0 || terminal_width() <= 7);
    }

    // ============================================================
    // Word-wrapping integration tests (helpers produce multiline)
    // ============================================================

    #[test]
    fn test_detail_row_wraps_long_text() {
        let _lock = width_lock();
        // detail_row prefix is 7 cols ("│      "); with width=40 that gives 33 cols for content
        let long = "Contract source code is not verified and full vulnerability analysis requires verified source code for accurate results";
        let row = detail_row_styled(long, false);
        // Should contain continuation lines with │ prefix
        let line_count = row.lines().count();
        // At a typical terminal width this will wrap; in narrow CI it will certainly wrap
        assert!(row.contains("│"), "should contain box-drawing prefix");
        // All lines should start with │
        for line in row.lines() {
            assert!(
                line.starts_with('│'),
                "continuation line should start with │: {}",
                line
            );
        }
        // Verify no content is lost
        assert!(row.contains("Contract"));
        assert!(row.contains("results"));
        // If terminal is wide enough, it might not wrap; otherwise verify multiple lines
        if terminal_width() < 80 {
            assert!(line_count > 1, "should wrap on narrow terminal");
        }
    }

    #[test]
    fn test_check_pass_wraps_long_text() {
        let _lock = width_lock();
        let long = "No sells detected below the configured peg target during the monitoring window across all tracked pairs";
        let row = check_pass_styled(long, false);
        assert!(row.contains("✓"));
        assert!(row.contains("No sells"));
        assert!(row.contains("pairs"));
        for line in row.lines() {
            assert!(line.starts_with('│'));
        }
    }

    #[test]
    fn test_check_fail_wraps_long_text() {
        let _lock = width_lock();
        let long = "Bid depth is significantly below the minimum threshold required for healthy market conditions on this trading pair";
        let row = check_fail_styled(long, false);
        assert!(row.contains("✗"));
        for line in row.lines() {
            assert!(line.starts_with('│'));
        }
    }

    #[test]
    fn test_warning_row_wraps_long_text() {
        let _lock = width_lock();
        let long = "Source code is NOT verified — unable to perform source-level analysis. Consider requesting verification.";
        let row = warning_row_styled(long, false);
        assert!(row.contains("⚠"));
        for line in row.lines() {
            assert!(line.starts_with('│'));
        }
    }

    #[test]
    fn test_info_row_wraps_long_text() {
        let _lock = width_lock();
        let long = "No audit reports found. Check block explorer and auditor databases manually for third-party audit information.";
        let row = info_row_styled(long, false);
        assert!(row.contains("ℹ"));
        for line in row.lines() {
            assert!(line.starts_with('│'));
        }
    }

    #[test]
    fn test_bullet_row_wraps_long_text() {
        let _lock = width_lock();
        let long = "Uniswap V3 integration detected with slippage protection enabled and deadline protection enabled for all swap calls";
        let row = bullet_row_styled(long, false);
        assert!(row.contains("•"));
        for line in row.lines() {
            assert!(line.starts_with('│'));
        }
    }

    #[test]
    fn test_kv_row_wraps_long_value() {
        let _lock = width_lock();
        let long_val = "Verified contract with comprehensive access controls and multiple security features including role-based permissions";
        let row = kv_row_styled("Summary", long_val, false);
        assert!(row.contains("Summary"));
        assert!(row.contains("Verified"));
        assert!(row.contains("permissions"));
        for line in row.lines() {
            assert!(line.starts_with('│'));
        }
    }

    #[test]
    fn test_kv_row_short_value_no_wrap() {
        let _lock = width_lock();
        let row = kv_row_styled("Chain", "ethereum", false);
        assert_eq!(row.lines().count(), 1);
        assert!(row.contains("Chain"));
        assert!(row.contains("ethereum"));
    }

    #[test]
    fn test_detail_row_wraps_tty() {
        let _lock = width_lock();
        let long = "Contract source code is not verified and full vulnerability analysis requires verified source code for accurate results";
        let row = detail_row_styled(long, true);
        assert!(row.contains("│"));
        assert!(row.contains("Contract"));
        assert!(row.contains("results"));
    }

    #[test]
    fn test_bullet_row_wraps_tty() {
        let _lock = width_lock();
        let long = "Uniswap V3 integration detected with slippage protection enabled and deadline protection enabled for all swap calls";
        let row = bullet_row_styled(long, true);
        assert!(row.contains("•"));
        assert!(row.contains("Uniswap"));
        assert!(row.contains("calls"));
    }

    #[test]
    fn test_kv_row_wraps_tty() {
        let _lock = width_lock();
        let long_val = "Verified contract with comprehensive access controls and multiple security features including role-based permissions";
        let row = kv_row_styled("Summary", long_val, true);
        assert!(row.contains("Summary"));
        assert!(row.contains("Verified"));
        assert!(row.contains("permissions"));
    }

    // ============================================================
    // Table / numbered row helper tests
    // ============================================================

    #[test]
    fn test_table_header_contains_labels() {
        let cols = &[
            Col {
                label: "Rank",
                width: 6,
                align: '>',
            },
            Col {
                label: "Name",
                width: 20,
                align: '<',
            },
        ];
        let header = table_header(cols);
        assert!(header.contains("Rank"));
        assert!(header.contains("Name"));
        assert!(header.contains("│"));
        assert!(header.contains("─"));
    }

    #[test]
    fn test_table_row_contains_values() {
        let cols = &[
            Col {
                label: "Rank",
                width: 6,
                align: '>',
            },
            Col {
                label: "Name",
                width: 20,
                align: '<',
            },
        ];
        let row = table_row(cols, &["1", "TestToken"]);
        assert!(row.contains("1"));
        assert!(row.contains("TestToken"));
        assert!(row.contains("│"));
    }

    #[test]
    fn test_table_row_missing_values() {
        let cols = &[
            Col {
                label: "A",
                width: 5,
                align: '<',
            },
            Col {
                label: "B",
                width: 5,
                align: '<',
            },
        ];
        let row = table_row(cols, &["only"]);
        assert!(row.contains("only"));
        assert!(row.contains("│"));
    }

    #[test]
    fn test_table_header_tty() {
        let cols = &[Col {
            label: "Price",
            width: 10,
            align: '>',
        }];
        let header = table_header_styled(cols, true);
        assert!(header.contains("Price"));
        assert!(header.contains("│"));
        // The header text is bold cyan: cyan, then bold, then the label.
        assert!(header.contains("\x1b[36m\x1b[1m"), "{header:?}");
        let plain = table_header_styled(cols, false);
        assert!(!plain.contains('\x1b'), "plain output has no escapes");
    }

    #[test]
    fn test_table_row_tty() {
        let cols = &[Col {
            label: "Price",
            width: 10,
            align: '>',
        }];
        let row = table_row_styled(cols, &["$1.00"], true);
        assert!(row.contains("$1.00"));
        assert!(row.contains("│"));
    }

    #[test]
    fn test_numbered_row_basic() {
        let row = numbered_row(1, "First item");
        assert!(row.contains("1."));
        assert!(row.contains("First item"));
        assert!(row.contains("│"));
    }

    #[test]
    fn test_numbered_row_wraps() {
        let _lock = width_lock();
        let long = "This is a very long description that should eventually wrap to the next line when the terminal width is narrow enough";
        let row = numbered_row(1, long);
        assert!(row.contains("1."));
        assert!(row.contains("This"));
        assert!(row.contains("enough"));
        for line in row.lines() {
            assert!(line.contains('│'));
        }
    }

    #[test]
    fn test_numbered_row_tty() {
        let row = numbered_row_styled(5, "Fifth item", true);
        assert!(row.contains("5."));
        assert!(row.contains("Fifth item"));
    }

    #[test]
    fn test_numbered_row_double_digits() {
        let row = numbered_row(12, "Twelfth item");
        assert!(row.contains("12."));
        assert!(row.contains("Twelfth item"));
    }

    // ── TTY wrapping continuation tests ──
    // Exercise the tty=true branch for multi-line wrapping in each helper.

    #[test]
    fn test_check_pass_wraps_tty() {
        let _lock = width_lock();
        // Long enough to wrap on any real terminal (up to ~500 columns).
        let long = &format!("{} ", "No sells detected below the configured peg target during the monitoring window across all tracked pairs and venues in scope").repeat(4);
        let row = check_pass_styled(long, true);
        assert!(row.contains("✓"));
        assert!(row.lines().count() > 1, "should wrap to multiple lines");
    }

    #[test]
    fn test_check_fail_wraps_tty() {
        let _lock = width_lock();
        // Long enough to wrap on any real terminal (up to ~500 columns).
        let long = &format!("{} ", "Bid depth is significantly below the minimum threshold required for healthy market conditions on this particular trading pair").repeat(4);
        let row = check_fail_styled(long, true);
        assert!(row.contains("✗"));
        assert!(row.lines().count() > 1, "should wrap to multiple lines");
    }

    #[test]
    fn test_check_na_plain_and_tty_wrap() {
        let _lock = width_lock();
        // Long enough to wrap on any real terminal (up to ~500 columns).
        // n/a rows (#37) must read as neither pass nor fail.
        let long = &format!("{} ", "Spread: n/a (synthetic AMM book; the real spread is the pool fee, which the data source does not report)").repeat(4);
        for tty in [false, true] {
            let row = check_na_styled(long, tty);
            assert!(row.contains("–"), "tty={}", tty);
            assert!(!row.contains("✓") && !row.contains("✗"), "tty={}", tty);
            assert!(row.lines().count() > 1, "should wrap, tty={}", tty);
        }
        assert!(check_na_styled("short", false).starts_with("│  – short"));
    }

    #[test]
    fn test_warning_row_wraps_tty() {
        let _lock = width_lock();
        // Long enough to wrap on any real terminal (up to ~500 columns).
        let long = &format!("{} ", "Source code is NOT verified — unable to perform full source-level analysis on this contract. Consider requesting verification from the deployer.").repeat(4);
        let row = warning_row_styled(long, true);
        assert!(row.contains("⚠"));
        assert!(row.lines().count() > 1, "should wrap to multiple lines");
    }

    #[test]
    fn test_info_row_wraps_tty() {
        let _lock = width_lock();
        // Long enough to wrap on any real terminal (up to ~500 columns).
        let long = &format!("{} ", "No audit reports found in any public database. Check block explorer and auditor databases manually for third-party audit information and verification.").repeat(4);
        let row = info_row_styled(long, true);
        assert!(row.contains("ℹ"));
        assert!(row.lines().count() > 1, "should wrap to multiple lines");
    }

    #[test]
    fn test_numbered_row_wraps_tty() {
        let _lock = width_lock();
        // Long enough to wrap on any real terminal (up to ~500 columns).
        let long = &format!("{} ", "This is a very long description that should eventually wrap to the next line when the terminal width is narrow enough to force it").repeat(4);
        let row = numbered_row_styled(1, long, true);
        assert!(row.contains("1."));
        assert!(row.lines().count() > 1, "should wrap to multiple lines");
    }

    #[test]
    fn test_check_warn_styled_keeps_text_and_continuation_bar() {
        let msg = "word ".repeat(60);
        let plain = check_warn_styled(&msg, false);
        let styled = check_warn_styled(&msg, true);
        assert!(plain.starts_with("│  ! "));
        assert!(!plain.contains('\x1b'));
        assert!(styled.contains('\x1b'), "color on a color-enabled stream");
        // Wrapped lines keep the bar, and no text is lost in either mode.
        assert!(plain.lines().count() > 1);
        assert_eq!(styled.lines().count(), plain.lines().count());
        let words = |s: &str| s.matches("word").count();
        assert_eq!(words(&plain), 60);
        assert_eq!(words(&styled), 60);
    }

    // ============================================================
    // Width override (R11): the TUI sets it to the pane width
    // ============================================================

    fn widest(text: &str) -> usize {
        text.lines().map(UnicodeWidthStr::width).max().unwrap_or(0)
    }

    fn two_cols() -> [Col<'static>; 2] {
        [
            Col {
                label: "Rank",
                width: 150,
                align: '<',
            },
            Col {
                label: "Value",
                width: 150,
                align: '>',
            },
        ]
    }

    #[test]
    fn test_rules_fit_a_narrow_pane() {
        // A rule wider than the pane wraps onto the next row in the TUI.
        let cap = Some(40);
        assert!(widest(&section_footer_at(false, cap)) <= 40);
        assert!(widest(&separator_at(false, cap)) <= 40);
        assert!(widest(&section_header_at("Available Venues", false, cap)) <= 40);
        let header = table_header_at(&two_cols(), false, cap);
        let rule = header.lines().nth(1).unwrap_or_default();
        assert_eq!(rule.width(), 40, "{header}");
    }

    #[test]
    fn test_a_wide_pane_keeps_the_fixed_rules() {
        // The override only shortens rules; it never makes them longer.
        let cap = Some(200);
        assert_eq!(section_footer_at(false, cap).width(), 51);
        assert_eq!(separator_at(false, cap).width(), 51);
        let header = section_header_at("Available Venues", false, cap);
        assert_eq!(header.trim().width(), 50);
    }

    #[test]
    fn test_without_an_override_rules_keep_their_fixed_widths() {
        // CLI and piped output must not change with the terminal width:
        // only the TUI pane (the override) shortens a rule.
        let _lock = width_lock();
        assert_eq!(width_override(), None);
        assert_eq!(section_footer_styled(false).width(), 51);
        assert_eq!(separator_styled(false).width(), 51);
        let header = section_header_styled("Available Venues", false);
        assert_eq!(header.trim().width(), 50);
        // Two 150-column cols, wider than any terminal: (150 + 2) * 2 = 304
        // `─` after "│  ".
        let table = table_header_styled(&two_cols(), false);
        assert_eq!(table.lines().nth(1).unwrap_or_default().width(), 3 + 304);
        assert_eq!(section_footer_at(false, None), section_footer_styled(false));
    }

    #[test]
    fn test_terminal_width_uses_the_override_first() {
        assert_eq!(width_or_real(Some(40)), 40);
        let real = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(80);
        assert_eq!(width_or_real(None), real);
    }

    #[test]
    fn test_width_override_set_and_clear() {
        // The only test that sets the global. It sets the real width, so
        // tests that read terminal_width() without the lock see no change.
        let _lock = width_lock();
        let real = terminal_width();
        set_width_override(Some(real));
        assert_eq!(width_override(), Some(real));
        set_width_override(Some(0));
        assert_eq!(width_override(), None, "Some(0) removes it");
        set_width_override(Some(real));
        set_width_override(None);
        assert_eq!(width_override(), None);
        assert_eq!(terminal_width(), real);
    }

    #[test]
    fn test_wrap_lines_counts_display_width_of_cjk() {
        // Each CJK char is two columns. Bytes (3 per char) wrap too early;
        // chars (1 per char) would put all five words on one 12-column row.
        let lines = wrap_lines("日本 語 です", 7);
        assert_eq!(lines, vec!["日本 語", "です"]);
        for l in &lines {
            assert!(l.width() <= 7, "{l}");
        }
    }
}
