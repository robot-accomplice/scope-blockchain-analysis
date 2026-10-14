//! The venue a command uses when the user names none.

/// The default exchange venue: CLI `--venue` defaults, web market requests,
/// insights, and the fallback lookup for a symbol DexScreener does not know.
pub const DEFAULT_VENUE: &str = "binance";
