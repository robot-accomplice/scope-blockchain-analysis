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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::DexPair;

    fn make_pair(price: f64, liquidity: f64) -> DexPair {
        DexPair {
            dex_name: "TestDex".into(),
            pair_address: "0x0".into(),
            base_token: "USDC".into(),
            quote_token: "WETH".into(),
            price_usd: price,
            volume_24h: 0.0,
            liquidity_usd: liquidity,
            price_change_24h: 0.0,
            buys_24h: 0,
            sells_24h: 0,
            buys_6h: 0,
            sells_6h: 0,
            buys_1h: 0,
            sells_1h: 0,
            pair_created_at: None,
            url: None,
        }
    }

    #[test]
    fn test_order_book_from_analytics_normal() {
        let pair = make_pair(1.0, 100_000.0);
        let book = order_book_from_analytics("ethereum", &pair, "USDC");
        assert_eq!(book.pair, "USDC/USDT");
        assert_eq!(book.bids.len(), 1);
        assert_eq!(book.asks.len(), 1);
        assert!(book.bids[0].price > 0.0);
        assert!(book.asks[0].price > 0.0);
        assert!(book.bids[0].quantity > 0.0);
        assert!(book.asks[0].quantity > 0.0);
    }

    #[test]
    fn test_order_book_from_analytics_zero_price() {
        // price_usd = 0.0 -> bid_price = 0 and ask_price = 0
        // -> both qty branches hit the else { 0.0 }
        let pair = make_pair(0.0, 100_000.0);
        let book = order_book_from_analytics("ethereum", &pair, "TOKEN");
        assert_eq!(book.bids[0].quantity, 0.0);
        assert_eq!(book.asks[0].quantity, 0.0);
    }

    #[test]
    fn test_order_book_from_analytics_zero_liquidity() {
        let pair = make_pair(1.0, 0.0);
        let book = order_book_from_analytics("solana", &pair, "SOL");
        assert_eq!(book.bids[0].quantity, 0.0);
        assert_eq!(book.asks[0].quantity, 0.0);
    }
}
