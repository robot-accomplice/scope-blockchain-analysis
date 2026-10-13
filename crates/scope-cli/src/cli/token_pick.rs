//! # Token pick list
//!
//! The list a user picks a token from when a search by name or symbol
//! finds more than one (crawl, monitor). It goes through
//! [`prompt::pick`](crate::cli::prompt::pick), so it is not data and no
//! progress bar draws into it.

use crate::cli::output::Output;
use crate::cli::progress::{self, Spinner};
use crate::cli::textfit::{cols, cut_with_ellipsis};
use scope::chains::dex::TokenSearchResult;
use scope::error::Result;
use scope::market::{ExchangeClient, VenueRegistry};
use std::io::BufRead;

/// The venue a symbol is looked up on when DexScreener does not know it.
const CEX_FALLBACK_VENUE: &str = "binance";

/// The quote a fallback pair uses.
const CEX_FALLBACK_QUOTE: &str = "USDT";

/// The longest token name the list shows whole.
const NAME_COLS: usize = 20;

/// Shows `results` as a numbered table and returns the one the user picks.
///
/// # Errors
///
/// The write or read error, or an answer that is not a number in range.
pub(crate) fn pick_token<'a>(
    reader: &mut impl BufRead,
    spinner: Option<&Spinner>,
    out: &Output,
    results: &'a [TokenSearchResult],
) -> Result<&'a TokenSearchResult> {
    let index = crate::cli::prompt::pick(reader, out, &table(results), "token", results.len())?;
    let picked = &results[index];
    announce(spinner, out, picked);
    Ok(picked)
}

/// Tells the user which token the command uses (above the spinner when
/// one draws).
pub(crate) fn announce(spinner: Option<&Spinner>, out: &Output, token: &TokenSearchResult) {
    progress::notice(spinner, out, &selected_line(token));
}

/// Looks `symbol` up on a centralized exchange when DexScreener does not
/// know it, and tells the user when it finds it. The result has no
/// on-chain address; price and volume come from the venue ticker.
pub(crate) async fn cex_fallback(
    symbol: &str,
    chain: &str,
    spinner: Option<&Spinner>,
    out: &Output,
) -> Option<TokenSearchResult> {
    let registry = VenueRegistry::load().ok()?;
    let client = ExchangeClient::from_descriptor(registry.get(CEX_FALLBACK_VENUE)?);
    let symbol = symbol.to_uppercase();
    let pair = client.format_pair(&format!("{symbol}{CEX_FALLBACK_QUOTE}"));
    let ticker = client.fetch_ticker(&pair).await.ok()?;
    let found = TokenSearchResult {
        address: String::new(),
        symbol: symbol.clone(),
        name: symbol,
        chain: chain.to_string(),
        price_usd: Some(ticker.last_price.unwrap_or(0.0)),
        volume_24h: ticker.volume_24h.unwrap_or(0.0),
        liquidity_usd: 0.0,
        market_cap: None,
    };
    let line = format!(
        "Not found on DexScreener; found {} on {} (CEX)",
        found.symbol, found.chain
    );
    progress::notice(spinner, out, &line);
    Some(found)
}

/// The one line that names the token a command uses: symbol, name, chain,
/// price and, when there is one, the address.
pub(crate) fn selected_line(token: &TokenSearchResult) -> String {
    let at = if token.address.is_empty() {
        String::new()
    } else {
        format!(" at {}", token.address)
    };
    format!(
        "Selected: {} ({}) on {} - ${:.6}{}",
        token.symbol,
        token.name,
        token.chain,
        token.price_usd.unwrap_or(0.0),
        at
    )
}

/// The header, a rule and one row per result, each line ending in a
/// newline.
fn table(results: &[TokenSearchResult]) -> String {
    let mut list = format!("\nFound {} matching tokens:\n\n", results.len());
    list.push_str(&row(
        "#",
        "Symbol",
        &pad_end("Name"),
        "Address",
        "Chain",
        "Price",
        "Liquidity",
    ));
    list.push_str(&format!("{}\n", "─".repeat(98)));
    for (i, token) in results.iter().enumerate() {
        let price = token
            .price_usd
            .map(|p| format!("${:.6}", p))
            .unwrap_or_else(|| "N/A".to_string());
        list.push_str(&row(
            &(i + 1).to_string(),
            &token.symbol,
            &pad_end(&cut_with_ellipsis(&token.name, NAME_COLS)),
            &abbreviate_address(&token.address),
            &token.chain,
            &price,
            &scope::display::format_large_number(token.liquidity_usd),
        ));
    }
    list
}

/// One table row. `name` comes padded ([`pad_end`]).
fn row(
    n: &str,
    symbol: &str,
    name: &str,
    addr: &str,
    chain: &str,
    price: &str,
    liq: &str,
) -> String {
    format!("{n:>3}  {symbol:>8}  {name}  {addr:<16}  {chain:<12}  {price:>12}  {liq:>12}\n")
}

/// The width of the name column, in display columns.
const NAME_FIELD_COLS: usize = 22;

/// `name` followed by spaces up to [`NAME_FIELD_COLS`] display columns.
/// `format!` pads by characters, which is wrong for wide characters.
fn pad_end(name: &str) -> String {
    let fill = NAME_FIELD_COLS.saturating_sub(cols(name));
    format!("{name}{}", " ".repeat(fill))
}

/// Abbreviates a blockchain address for display (e.g. "0x1234...abcd").
pub(crate) fn abbreviate_address(addr: &str) -> String {
    if addr.len() > 16 {
        format!("{}...{}", &addr[..8], &addr[addr.len() - 6..])
    } else {
        addr.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(symbol: &str, name: &str, address: &str) -> TokenSearchResult {
        TokenSearchResult {
            address: address.into(),
            symbol: symbol.into(),
            name: name.into(),
            chain: "ethereum".into(),
            price_usd: Some(1.0),
            volume_24h: 0.0,
            liquidity_usd: 5_000_000.0,
            market_cap: None,
        }
    }

    #[test]
    fn test_pick_token_shows_the_table_on_the_diagnostic_channel() {
        let results = [
            result(
                "USDC",
                "USD Coin",
                "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
            ),
            result("XYZ", "A Very Long Token Name Indeed", "0x1234abcd"),
        ];
        let (out, cap) = Output::capture();
        let picked = pick_token(
            &mut std::io::Cursor::new(b"2\n".to_vec()),
            None,
            &out,
            &results,
        )
        .unwrap();
        assert_eq!(picked.symbol, "XYZ");
        let shown = cap.err();
        assert!(
            shown.starts_with("\nFound 2 matching tokens:\n\n"),
            "{shown}"
        );
        assert!(shown.contains("0xA0b869...06eB48") && shown.contains("A Very Long Token N…"));
        assert!(
            shown.ends_with("\nSelect token (1-2): Selected: XYZ (A Very Long Token Name Indeed) on ethereum - $1.000000 at 0x1234abcd\n"),
            "{shown}"
        );
        assert_eq!(cap.out(), "", "the list is not data");
    }

    #[test]
    fn test_long_wide_names_cut_by_display_width_and_line_up() {
        // Byte slicing panicked inside a multi-byte character, and byte
        // padding pushed the columns after a wide name out of line.
        let names = [
            "USD Coin",
            "日本円ステーブルコイン・テスト用の長い名前",
            "🚀🚀🚀 Rocket Moon Token Supreme Edition",
        ];
        let results: Vec<_> = names.iter().map(|n| result("X", n, "0xabc")).collect();
        let list = table(&results);
        let rows: Vec<&str> = list.lines().skip(5).collect();
        assert_eq!(rows.len(), 3, "{list}");
        let address_col = |row: &str| cols(&row[..row.find("0xabc").unwrap()]);
        let first = address_col(rows[0]);
        for row in &rows {
            assert_eq!(address_col(row), first, "{list}");
            assert!(row.contains('…') || row.contains("USD Coin"), "{row}");
        }
    }

    #[test]
    fn test_selected_line_names_the_address_only_when_there_is_one() {
        let mut t = result("USDC", "USD Coin", "0xabc");
        assert_eq!(
            selected_line(&t),
            "Selected: USDC (USD Coin) on ethereum - $1.000000 at 0xabc"
        );
        t.address.clear();
        assert_eq!(
            selected_line(&t),
            "Selected: USDC (USD Coin) on ethereum - $1.000000"
        );
    }

    #[test]
    fn test_abbreviate_address() {
        assert_eq!(
            abbreviate_address("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            "0xA0b869...06eB48"
        );
        // Short address is not abbreviated
        assert_eq!(abbreviate_address("0x1234abcd"), "0x1234abcd");
    }

    #[test]
    fn test_abbreviate_address_boundary_16_chars() {
        // Exactly 16 chars - not abbreviated (len > 16 is false)
        let addr = "0x1234567890abcd";
        assert_eq!(abbreviate_address(addr), addr);
    }

    #[test]
    fn test_abbreviate_address_boundary_17_chars() {
        // 17 chars - should be abbreviated (8 + "..." + 6)
        let addr = "0x1234567890abcdef1";
        let result = abbreviate_address(addr);
        assert!(result.contains("..."));
        assert_eq!(&result[..8], "0x123456");
        // Last 6 chars of original "0x1234567890abcdef1" are "bcdef1"
        assert_eq!(&result[result.len() - 6..], "bcdef1");
    }

    #[test]
    fn test_abbreviate_address_empty() {
        assert_eq!(abbreviate_address(""), "");
    }
}
