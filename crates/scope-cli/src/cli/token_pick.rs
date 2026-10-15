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
use scope::display::text::abbreviate;
use scope::error::Result;
use scope::market::{ExchangeClient, Pair, VenueRegistry};
use std::io::BufRead;

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
    let client = ExchangeClient::from_descriptor(registry.get(scope::market::DEFAULT_VENUE)?);
    let symbol = symbol.to_uppercase();
    let pair = fallback_pair(&client, &symbol);
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

/// The venue symbol a CEX fallback asks for: `symbol` quoted in
/// [`CEX_FALLBACK_QUOTE`].
fn fallback_pair(client: &ExchangeClient, symbol: &str) -> String {
    client.venue_symbol(&Pair {
        base: symbol.to_string(),
        quote: Some(CEX_FALLBACK_QUOTE.to_string()),
    })
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
        "Selected: {} ({}) on {} - {}{}",
        token.symbol,
        token.name,
        token.chain,
        scope::display::format::format_price_usd(token.price_usd.unwrap_or(0.0)),
        at
    )
}

/// How a column lines up its cells.
#[derive(Clone, Copy)]
enum Align {
    Left,
    Right,
}

/// The table columns: header, width in display columns, alignment.
const COLUMNS: [(&str, usize, Align); 7] = [
    ("#", 3, Align::Right),
    ("Symbol", 8, Align::Right),
    ("Name", 22, Align::Left),
    ("Address", 16, Align::Left),
    ("Chain", 12, Align::Left),
    ("Price", 12, Align::Right),
    ("Liquidity", 12, Align::Right),
];

/// The blank between two columns.
const GAP: &str = "  ";

/// The width of the rule under the header: the columns and the gaps.
const RULE_COLS: usize = {
    let mut total = GAP.len() * (COLUMNS.len() - 1);
    let mut i = 0;
    while i < COLUMNS.len() {
        total += COLUMNS[i].1;
        i += 1;
    }
    total
};

/// The header, a rule and one row per result, each line ending in a
/// newline.
fn table(results: &[TokenSearchResult]) -> String {
    let candidates: Vec<_> = results
        .iter()
        .map(crate::cli::resolve::dex_candidate)
        .collect();
    candidate_table(&candidates)
}

/// The numbered table the resolver's terminal picker shows.
pub(crate) fn candidate_table(candidates: &[crate::cli::resolve::Candidate]) -> String {
    let mut list = format!("\nFound {} matching tokens:\n\n", candidates.len());
    list.push_str(&row(COLUMNS.map(|(header, _, _)| header.to_string())));
    list.push_str(&format!("{}\n", "─".repeat(RULE_COLS)));
    let na = || "N/A".to_string();
    for (i, c) in candidates.iter().enumerate() {
        list.push_str(&row([
            (i + 1).to_string(),
            c.symbol.clone(),
            cut_with_ellipsis(&c.name, NAME_COLS),
            c.address.as_deref().map(abbreviate).unwrap_or_default(),
            c.chain.clone(),
            c.price_usd.map(|p| format!("${:.6}", p)).unwrap_or_else(na),
            c.liquidity_usd
                .map(scope::display::format_large_number)
                .unwrap_or_else(na),
        ]));
    }
    list
}

/// One table row: each cell padded to its column by display width
/// (`format!` pads by characters, which is wrong for wide characters).
fn row(cells: [String; 7]) -> String {
    let padded: Vec<String> = cells
        .iter()
        .zip(COLUMNS)
        .map(|(cell, (_, width, align))| {
            let fill = " ".repeat(width.saturating_sub(cols(cell)));
            match align {
                Align::Left => format!("{cell}{fill}"),
                Align::Right => format!("{fill}{cell}"),
            }
        })
        .collect();
    format!("{}\n", padded.join(GAP).trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The fallback asked Binance for USDCUSDTUSDT: the quote was glued onto
    // the symbol and the venue added its default quote again.
    #[test]
    fn test_fallback_pair_quotes_the_symbol_once() {
        let registry = VenueRegistry::load().unwrap();
        let client =
            ExchangeClient::from_descriptor(registry.get(scope::market::DEFAULT_VENUE).unwrap());
        assert_eq!(fallback_pair(&client, "USDC"), "USDCUSDT");
        assert_eq!(fallback_pair(&client, "OP"), "OPUSDT");
    }

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
        assert!(shown.contains("0xA0b8…eB48") && shown.contains("A Very Long Token N…"));
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
}
