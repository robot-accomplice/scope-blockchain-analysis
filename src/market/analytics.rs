//! DEX analytics utilities.
//!
//! Builds synthetic order books from DexScreener liquidity data for
//! Ethereum and Solana DEX venues.

use super::types::{OrderBook, OrderBookLevel};

/// Builds a synthetic order book from DEX analytics (used for Ethereum/Solana venues).
pub fn order_book_from_analytics(
    _chain: &str,
    pair: &crate::chains::DexPair,
    symbol: &str,
) -> OrderBook {
    let price = pair.price_usd;
    let liquidity = pair.liquidity_usd;
    // Synthetic bid/ask spread ±0.1% around mid
    let bid_price = price * 0.999;
    let ask_price = price * 1.001;
    let half_liq = liquidity / 2.0;
    let bid_qty = if bid_price > 0.0 {
        half_liq / bid_price
    } else {
        0.0
    };
    let ask_qty = if ask_price > 0.0 {
        half_liq / ask_price
    } else {
        0.0
    };

    OrderBook {
        pair: format!("{}/USDT", symbol),
        bids: vec![OrderBookLevel {
            price: bid_price,
            quantity: bid_qty,
        }],
        asks: vec![OrderBookLevel {
            price: ask_price,
            quantity: ask_qty,
        }],
    }
}
