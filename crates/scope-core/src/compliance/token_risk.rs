//! # Token risk classes (M7)
//!
//! The one place that decides when liquidity is low and when trading looks
//! like a honeypot. The crawl report, the compliance engine and the
//! insights summary read these classes, so they never disagree on whether
//! to flag a token. Each keeps its own scoring scale.

/// Liquidity below this (USD) is very low: high slippage on any trade.
pub const VERY_LOW_LIQUIDITY_USD: f64 = 10_000.0;
/// Liquidity below this (USD) is low: slippage on larger trades.
pub const LOW_LIQUIDITY_USD: f64 = 100_000.0;
/// Liquidity above this (USD) is good: enough depth for most trades.
pub const GOOD_LIQUIDITY_USD: f64 = 1_000_000.0;

/// A buy/sell ratio above this is a high honeypot risk.
pub const HONEYPOT_HIGH_RATIO: f64 = 10.0;
/// A buy/sell ratio above this is a medium honeypot risk.
pub const HONEYPOT_MEDIUM_RATIO: f64 = 3.0;

/// How deep a token's pools are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiquidityTier {
    /// Below [`VERY_LOW_LIQUIDITY_USD`], including none.
    VeryLow,
    /// Below [`LOW_LIQUIDITY_USD`].
    Low,
    /// Between low and good.
    Moderate,
    /// Above [`GOOD_LIQUIDITY_USD`].
    Good,
}

/// The tier of `liquidity_usd`. No liquidity is very low, not unknown.
pub fn liquidity_tier(liquidity_usd: f64) -> LiquidityTier {
    if liquidity_usd < VERY_LOW_LIQUIDITY_USD {
        LiquidityTier::VeryLow
    } else if liquidity_usd < LOW_LIQUIDITY_USD {
        LiquidityTier::Low
    } else if liquidity_usd > GOOD_LIQUIDITY_USD {
        LiquidityTier::Good
    } else {
        LiquidityTier::Moderate
    }
}

/// What 24h trading says about a honeypot (a token that cannot be sold).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Honeypot {
    /// No buys and no sells: nothing to judge.
    Unknown,
    /// Buys and no sells: the strongest sign.
    NoSells { buys: u64 },
    /// Buys per sell.
    Ratio(f64),
}

/// How strong a honeypot sign is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HoneypotLevel {
    Unknown,
    Low,
    Medium,
    High,
}

/// The honeypot sign of `buys` and `sells` in 24h.
pub fn honeypot(buys: u64, sells: u64) -> Honeypot {
    match (buys, sells) {
        (0, 0) => Honeypot::Unknown,
        (b, 0) => Honeypot::NoSells { buys: b },
        (b, s) => Honeypot::Ratio(b as f64 / s as f64),
    }
}

impl Honeypot {
    /// The one level rule: no sells, or a ratio above
    /// [`HONEYPOT_HIGH_RATIO`], is high; above [`HONEYPOT_MEDIUM_RATIO`] is
    /// medium.
    pub fn level(self) -> HoneypotLevel {
        match self {
            Honeypot::Unknown => HoneypotLevel::Unknown,
            Honeypot::NoSells { .. } => HoneypotLevel::High,
            Honeypot::Ratio(r) if r > HONEYPOT_HIGH_RATIO => HoneypotLevel::High,
            Honeypot::Ratio(r) if r > HONEYPOT_MEDIUM_RATIO => HoneypotLevel::Medium,
            Honeypot::Ratio(_) => HoneypotLevel::Low,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_liquidity_tiers_and_their_bounds() {
        assert_eq!(liquidity_tier(0.0), LiquidityTier::VeryLow);
        assert_eq!(liquidity_tier(9_999.0), LiquidityTier::VeryLow);
        assert_eq!(liquidity_tier(75_000.0), LiquidityTier::Low);
        assert_eq!(liquidity_tier(500_000.0), LiquidityTier::Moderate);
        assert_eq!(liquidity_tier(1_000_000.0), LiquidityTier::Moderate);
        assert_eq!(liquidity_tier(1_000_001.0), LiquidityTier::Good);
    }

    #[test]
    fn test_any_buys_with_no_sells_is_a_high_honeypot_sign() {
        // The audit case: 5 buys and 0 sells must be flagged everywhere.
        assert_eq!(honeypot(5, 0), Honeypot::NoSells { buys: 5 });
        assert_eq!(honeypot(5, 0).level(), HoneypotLevel::High);
        assert_eq!(honeypot(0, 0).level(), HoneypotLevel::Unknown);
        assert_eq!(honeypot(11, 1).level(), HoneypotLevel::High);
        assert_eq!(honeypot(4, 1).level(), HoneypotLevel::Medium);
        assert_eq!(honeypot(3, 1).level(), HoneypotLevel::Low);
    }
}
