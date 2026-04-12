//! Shared formatting utilities for consistent presentation across all chains.

/// Formats a USD value with K, M, B suffixes.
///
/// # Examples
///
/// ```
/// use scope::display::format::format_usd;
///
/// assert_eq!(format_usd(1_500_000_000.0), "$1.50B");
/// assert_eq!(format_usd(1_500_000.0), "$1.50M");
/// assert_eq!(format_usd(1_500.0), "$1.50K");
/// assert_eq!(format_usd(15.50), "$15.50");
/// ```
pub fn format_usd(value: f64) -> String {
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

/// Formats a large number with K, M, B suffixes (no currency prefix).
///
/// # Examples
///
/// ```
/// use scope::display::format::format_large_number;
///
/// assert_eq!(format_large_number(1_500_000_000.0), "1.50B");
/// assert_eq!(format_large_number(1_500_000.0), "1.50M");
/// assert_eq!(format_large_number(1_500.0), "1.50K");
/// assert_eq!(format_large_number(500.0), "500.00");
/// ```
pub fn format_large_number(value: f64) -> String {
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

/// Formats a raw token balance string with proper decimals and K/M/B suffixes.
///
/// # Arguments
///
/// * `balance` - Raw balance in smallest units (e.g., wei, lamports)
/// * `decimals` - Token decimal places
///
/// # Examples
///
/// ```
/// use scope::display::format::format_token_balance;
///
/// assert!(format_token_balance("1000000000000000000", 18).starts_with("1.00"));
/// assert!(format_token_balance("5000000000000000000000000", 18).contains("M"));
/// ```
pub fn format_token_balance(balance: &str, decimals: u8) -> String {
    let balance_f64: f64 = balance.parse().unwrap_or(0.0);
    let divisor = 10_f64.powi(decimals as i32);
    let formatted = balance_f64 / divisor;

    if formatted >= 1_000_000_000.0 {
        format!("{:.2}B", formatted / 1_000_000_000.0)
    } else if formatted >= 1_000_000.0 {
        format!("{:.2}M", formatted / 1_000_000.0)
    } else if formatted >= 1_000.0 {
        format!("{:.2}K", formatted / 1_000.0)
    } else {
        format!("{:.4}", formatted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_usd() {
        assert_eq!(format_usd(1_500_000_000.0), "$1.50B");
        assert_eq!(format_usd(1_500_000.0), "$1.50M");
        assert_eq!(format_usd(1_500.0), "$1.50K");
        assert_eq!(format_usd(15.50), "$15.50");
        assert_eq!(format_usd(0.0), "$0.00");
    }

    #[test]
    fn test_format_large_number() {
        assert_eq!(format_large_number(500.0), "500.00");
        assert_eq!(format_large_number(1_500.0), "1.50K");
        assert_eq!(format_large_number(1_500_000.0), "1.50M");
        assert_eq!(format_large_number(1_500_000_000.0), "1.50B");
    }

    #[test]
    fn test_format_token_balance() {
        assert!(format_token_balance("1000000000000000000", 18).starts_with("1.00"));
        assert!(format_token_balance("5000000000000000000", 18).contains("5.00"));
        assert!(format_token_balance("0", 18).starts_with("0.00"));
    }
}
