//! # Token pick list
//!
//! The list a user picks a token from when a search by name or symbol
//! finds more than one (crawl, monitor). It goes through
//! [`prompt::pick`](crate::cli::prompt::pick), so it is not data and no
//! progress bar draws into it.

use crate::cli::output::Output;
use scope::chains::dex::TokenSearchResult;
use scope::error::Result;
use std::io::BufRead;

/// The longest token name the list shows whole.
const NAME_COLS: usize = 20;

/// Shows `results` as a numbered table and returns the one the user picks.
///
/// # Errors
///
/// The write or read error, or an answer that is not a number in range.
pub(crate) fn pick_token<'a>(
    reader: &mut impl BufRead,
    out: &Output,
    results: &'a [TokenSearchResult],
) -> Result<&'a TokenSearchResult> {
    let index = crate::cli::prompt::pick(reader, out, &table(results), "token", results.len())?;
    Ok(&results[index])
}

/// The header, a rule and one row per result, each line ending in a
/// newline.
fn table(results: &[TokenSearchResult]) -> String {
    let mut list = format!("\nFound {} matching tokens:\n\n", results.len());
    list.push_str(&row(
        "#",
        "Symbol",
        "Name",
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
            &short_name(&token.name),
            &abbreviate_address(&token.address),
            &token.chain,
            &price,
            &scope::display::format_large_number(token.liquidity_usd),
        ));
    }
    list
}

fn row(
    n: &str,
    symbol: &str,
    name: &str,
    addr: &str,
    chain: &str,
    price: &str,
    liq: &str,
) -> String {
    format!("{n:>3}  {symbol:>8}  {name:<22}  {addr:<16}  {chain:<12}  {price:>12}  {liq:>12}\n")
}

/// `name`, or its start and `...` when it is longer than [`NAME_COLS`].
fn short_name(name: &str) -> String {
    if name.len() > NAME_COLS {
        format!("{}...", &name[..NAME_COLS - 3])
    } else {
        name.to_string()
    }
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
        let picked =
            pick_token(&mut std::io::Cursor::new(b"2\n".to_vec()), &out, &results).unwrap();
        assert_eq!(picked.symbol, "XYZ");
        let shown = cap.err();
        assert!(
            shown.starts_with("\nFound 2 matching tokens:\n\n"),
            "{shown}"
        );
        assert!(shown.contains("0xA0b869...06eB48") && shown.contains("A Very Long Token..."));
        assert!(shown.ends_with("\nSelect token (1-2): "), "{shown}");
        assert_eq!(cap.out(), "", "the list is not data");
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
