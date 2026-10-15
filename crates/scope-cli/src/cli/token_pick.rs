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
use scope::market::{ExchangeClient, Pair, VenueRegistry};

/// The quote a fallback pair uses.
const CEX_FALLBACK_QUOTE: &str = "USDT";

/// The longest token name the list shows whole.
const NAME_COLS: usize = 20;

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

    fn table(results: &[TokenSearchResult]) -> String {
        let cs: Vec<_> = results
            .iter()
            .map(crate::cli::resolve::dex_candidate)
            .collect();
        candidate_table(&cs)
    }

    #[test]
    fn test_the_candidate_table_abbreviates_addresses_and_cuts_names() {
        let list = table(&[
            result(
                "USDC",
                "USD Coin",
                "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
            ),
            result("XYZ", "A Very Long Token Name Indeed", "0x1234abcd"),
        ]);
        assert!(list.starts_with("\nFound 2 matching tokens:\n\n"), "{list}");
        assert!(list.contains("0xA0b8…eB48") && list.contains("A Very Long Token N…"));
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
}
