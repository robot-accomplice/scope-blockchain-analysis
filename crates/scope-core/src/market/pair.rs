//! Trading pairs: one parser and one display format for every surface.
//!
//! Users write a pair as `BASE_QUOTE`, `BASE/QUOTE`, `BASE-QUOTE`,
//! `BASEQUOTE` (for a quote in [`QUOTE_CURRENCIES`]) or a bare `BASE`. Venue
//! responses use the same forms. [`Pair::parse`] reads all of them, and
//! `Display` writes `BASE/QUOTE`.

use std::fmt;

/// Quote currencies recognized at the end of a concatenated symbol such as
/// `BTCUSDT`. A longer code comes before a code it starts with (`USDT` and
/// `USDC` before `USD`), so the longest match wins.
pub const QUOTE_CURRENCIES: [&str; 8] = ["USDT", "USDC", "USD", "BTC", "ETH", "EUR", "GBP", "TRY"];

/// Separators accepted between base and quote.
const SEPARATORS: [char; 3] = ['_', '/', '-'];

/// A base asset and, when known, its quote asset. With no quote, a venue
/// uses its default quote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    /// Base asset symbol, as written (case is kept).
    pub base: String,
    /// Quote asset symbol, as written; `None` for a bare base.
    pub quote: Option<String>,
}

impl Pair {
    /// A pair of `base` with the venue's default quote. Use this for a token
    /// symbol, which must not be split (`WETH` is not `W/ETH`).
    pub fn base_only(base: &str) -> Self {
        Self {
            base: base.trim().to_string(),
            quote: None,
        }
    }

    /// Parses user input or a venue symbol. Accepts `key=value` input
    /// (`pair=DAI_USDC`), a separated pair, a base followed by a known quote
    /// currency, or a bare base.
    pub fn parse(input: &str) -> Self {
        let p = strip_key(input.trim());
        if let Some((base, quote)) = split_on_separator(p).or_else(|| split_on_quote_suffix(p)) {
            return Self {
                base: base.to_string(),
                quote: Some(quote.to_string()),
            };
        }
        Self::base_only(p)
    }
}

impl fmt::Display for Pair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.quote {
            Some(quote) => write!(f, "{}/{}", self.base, quote),
            None => f.write_str(&self.base),
        }
    }
}

/// `pair=USDC_USDT` -> `USDC_USDT`; other input is returned unchanged.
fn strip_key(p: &str) -> &str {
    match p.split_once('=') {
        Some((key, value))
            if !value.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') =>
        {
            value
        }
        _ => p,
    }
}

fn split_on_separator(p: &str) -> Option<(&str, &str)> {
    SEPARATORS.iter().find_map(|&sep| {
        p.split_once(sep)
            .filter(|(base, quote)| !base.is_empty() && !quote.is_empty())
    })
}

fn split_on_quote_suffix(p: &str) -> Option<(&str, &str)> {
    let upper = p.to_ascii_uppercase();
    QUOTE_CURRENCIES
        .iter()
        .find(|quote| p.len() > quote.len() && upper.ends_with(*quote))
        .map(|quote| p.split_at(p.len() - quote.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(input: &str) -> (String, Option<String>) {
        let pair = Pair::parse(input);
        (pair.base, pair.quote)
    }

    fn pair(base: &str, quote: Option<&str>) -> (String, Option<String>) {
        (base.to_string(), quote.map(str::to_string))
    }

    #[test]
    fn separated_forms_split_on_the_separator() {
        assert_eq!(parts("USDT_PUSD"), pair("USDT", Some("PUSD")));
        assert_eq!(parts("ETH/USDC"), pair("ETH", Some("USDC")));
        assert_eq!(parts("BTC-USDT"), pair("BTC", Some("USDT")));
        assert_eq!(parts("DAI_USDC"), pair("DAI", Some("USDC")));
    }

    #[test]
    fn case_is_kept() {
        assert_eq!(parts("dai_usdt"), pair("dai", Some("usdt")));
        assert_eq!(parts("btcusdt"), pair("btc", Some("usdt")));
    }

    #[test]
    fn concatenated_forms_split_on_a_known_quote() {
        assert_eq!(parts("BTCUSDT"), pair("BTC", Some("USDT")));
        assert_eq!(parts("ETHUSD"), pair("ETH", Some("USD")));
        assert_eq!(parts("ETHUSDC"), pair("ETH", Some("USDC")));
        assert_eq!(parts("USDCUSDT"), pair("USDC", Some("USDT")));
        assert_eq!(parts("XBTEUR"), pair("XBT", Some("EUR")));
    }

    /// The audit's inputs: before one list, the CLI knew TRY but not GBP and
    /// the venue display knew GBP but not TRY.
    #[test]
    fn gbp_and_try_parse_and_display() {
        assert_eq!(parts("BTCGBP"), pair("BTC", Some("GBP")));
        assert_eq!(parts("BTCTRY"), pair("BTC", Some("TRY")));
        assert_eq!(Pair::parse("BTCGBP").to_string(), "BTC/GBP");
        assert_eq!(Pair::parse("BTCTRY").to_string(), "BTC/TRY");
        assert_eq!(Pair::parse("XBTGBP").to_string(), "XBT/GBP");
    }

    #[test]
    fn a_bare_base_has_no_quote() {
        assert_eq!(parts("ARB"), pair("ARB", None));
        assert_eq!(parts("USDC"), pair("USDC", None));
        assert_eq!(parts("USDT"), pair("USDT", None));
        assert_eq!(parts("X"), pair("X", None));
        assert_eq!(parts("XYZABC"), pair("XYZABC", None));
        assert_eq!(parts(""), pair("", None));
        assert_eq!(Pair::parse("USDT").to_string(), "USDT");
        assert_eq!(Pair::parse("A").to_string(), "A");
    }

    #[test]
    fn whitespace_and_key_value_input_are_accepted() {
        assert_eq!(parts("  DAI_USDT  "), pair("DAI", Some("USDT")));
        assert_eq!(parts("pair=USDT_PUSD"), pair("USDT", Some("PUSD")));
        assert_eq!(parts("pair_symbol=USDT_PUSD"), pair("USDT", Some("PUSD")));
    }

    #[test]
    fn venue_symbols_display_with_a_slash() {
        assert_eq!(Pair::parse("BTC_USDT").to_string(), "BTC/USDT");
        assert_eq!(Pair::parse("BTC-USDT").to_string(), "BTC/USDT");
        assert_eq!(Pair::parse("BTCUSDT").to_string(), "BTC/USDT");
        assert_eq!(Pair::parse("ETHUSD").to_string(), "ETH/USD");
    }

    #[test]
    fn base_only_never_splits_a_token_symbol() {
        assert_eq!(
            Pair::base_only("WETH"),
            Pair {
                base: "WETH".into(),
                quote: None
            }
        );
        assert_eq!(Pair::parse("WETH").quote.as_deref(), Some("ETH"));
    }
}
