//! The market summary use case: venue + pair -> order book, 24h volume and
//! [`MarketSummary`]. `scope market summary`, `token-health --with-market`,
//! `insights` and the web endpoints all call it, so the same input gives
//! the same request everywhere.

use super::MarketSummary;
use super::analytics::order_book_from_analytics;
use super::exchange::ExchangeClient;
use super::health::HealthThresholds;
use super::pair::Pair;
use super::types::OrderBook;
use super::venue::{DexVenue, MarketVenue};
use crate::chains::{DexPair, TokenAnalytics};
use crate::error::{Result, ScopeError};
use async_trait::async_trait;

/// Port: token analytics with DEX pools, for a DEX venue's synthetic book.
/// Adapters resolve the token (symbol, address or alias) on the chain.
#[async_trait]
pub trait DexAnalyticsSource: Send + Sync {
    /// Analytics for `token` on `chain`.
    async fn token_analytics(&self, token: &str, chain: &str) -> Result<TokenAnalytics>;
}

/// A [`DexAnalyticsSource`] that serves analytics the caller already has.
struct KnownAnalytics<'a>(&'a TokenAnalytics);

#[async_trait]
impl DexAnalyticsSource for KnownAnalytics<'_> {
    async fn token_analytics(&self, _token: &str, _chain: &str) -> Result<TokenAnalytics> {
        Ok(self.0.clone())
    }
}

/// Fetches the book and 24h quote volume for `pair` on `venue`.
///
/// - DEX: the most liquid pool of `pair.base` on the venue's chain; the
///   volume is that pool's 24h volume.
/// - CEX: the venue's order book for the pair; the volume is the ticker's
///   quote volume, when the venue has a ticker.
pub async fn fetch_book_and_volume(
    venue: &MarketVenue,
    pair: &Pair,
    thresholds: &HealthThresholds,
    dex: &dyn DexAnalyticsSource,
) -> Result<(OrderBook, Option<f64>)> {
    match venue {
        MarketVenue::Dex(dex_venue) => {
            let analytics = dex.token_analytics(&pair.base, dex_venue.chain()).await?;
            dex_book_and_volume(*dex_venue, &analytics, thresholds)
        }
        MarketVenue::Cex(exchange) => cex_book_and_volume(exchange, pair).await,
    }
}

/// Fetches the book and volume, then judges it against `thresholds`.
pub async fn fetch_summary(
    venue: &MarketVenue,
    pair: &Pair,
    thresholds: &HealthThresholds,
    dex: &dyn DexAnalyticsSource,
) -> Result<MarketSummary> {
    let (book, volume_24h) = fetch_book_and_volume(venue, pair, thresholds, dex).await?;
    Ok(MarketSummary::from_order_book(
        &book, thresholds, volume_24h,
    ))
}

/// The market summary for a token the caller has already analyzed. The pair
/// is the token symbol with the venue's default quote: a token symbol is
/// never split (`WETH` is not `W/ETH`). A DEX venue reads these analytics,
/// so they must be on its chain.
pub async fn fetch_token_summary(
    venue: &MarketVenue,
    analytics: &TokenAnalytics,
    thresholds: &HealthThresholds,
) -> Result<MarketSummary> {
    let pair = Pair::base_only(&analytics.token.symbol);
    fetch_summary(venue, &pair, thresholds, &KnownAnalytics(analytics)).await
}

/// [`fetch_token_summary`] for a venue name: resolves it with
/// [`MarketVenue::from_name`] (the one unknown-venue error), then fetches.
#[deprecated(
    since = "0.8.0",
    note = "pass the transport: use `fetch_token_summary_by_name_with_http`"
)]
pub async fn fetch_token_summary_by_name(
    venue: &str,
    analytics: &TokenAnalytics,
    thresholds: &HealthThresholds,
) -> Result<MarketSummary> {
    fetch_token_summary_by_name_with_http(
        venue,
        analytics,
        thresholds,
        crate::http::shared::fallback(),
    )
    .await
}

/// [`fetch_token_summary_by_name`], sending a CEX venue's requests through
/// `http`.
pub async fn fetch_token_summary_by_name_with_http(
    venue: &str,
    analytics: &TokenAnalytics,
    thresholds: &HealthThresholds,
    http: std::sync::Arc<dyn crate::http::HttpClient>,
) -> Result<MarketSummary> {
    let venue = MarketVenue::from_name_with_http(venue, http)?;
    fetch_token_summary(&venue, analytics, thresholds).await
}

fn dex_book_and_volume(
    venue: DexVenue,
    analytics: &TokenAnalytics,
    thresholds: &HealthThresholds,
) -> Result<(OrderBook, Option<f64>)> {
    venue.require_chain(&analytics.chain)?;
    let best = most_liquid(&analytics.dex_pairs).ok_or_else(|| {
        ScopeError::NotFound(format!(
            "No DEX pairs found for {} on {}",
            analytics.token.symbol,
            venue.chain()
        ))
    })?;
    let book = order_book_from_analytics(venue.chain(), best, &analytics.token.symbol, thresholds);
    Ok((book, Some(best.volume_24h)))
}

/// The pool with the most USD liquidity: the deepest book, not the first.
pub fn most_liquid(pairs: &[DexPair]) -> Option<&DexPair> {
    pairs
        .iter()
        .max_by(|a, b| a.liquidity_usd.total_cmp(&b.liquidity_usd))
}

async fn cex_book_and_volume(
    exchange: &ExchangeClient,
    pair: &Pair,
) -> Result<(OrderBook, Option<f64>)> {
    let symbol = exchange.venue_symbol(pair);
    let book = exchange.fetch_order_book(&symbol).await?;
    let volume = if exchange.has_ticker() {
        exchange
            .fetch_ticker(&symbol)
            .await
            .ok()
            .and_then(|t| t.quote_volume(book.mid_price()))
    } else {
        None
    };
    Ok((book, volume))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::Token;

    fn dex_pair(liquidity_usd: f64, volume_24h: f64) -> DexPair {
        DexPair {
            dex_name: "DEX".into(),
            pair_address: "0xpair".into(),
            base_token: "USDC".into(),
            quote_token: "USDT".into(),
            price_usd: 1.0,
            volume_24h,
            liquidity_usd,
            liquidity_base: None,
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

    fn analytics(chain: &str, pairs: Vec<DexPair>) -> TokenAnalytics {
        TokenAnalytics {
            token: Token {
                contract_address: "0x".into(),
                symbol: "USDC".into(),
                name: "USD Coin".into(),
                decimals: 6,
            },
            chain: chain.into(),
            holders: vec![],
            total_holders: 0,
            volume_24h: 0.0,
            volume_7d: 0.0,
            price_usd: 1.0,
            price_change_24h: 0.0,
            price_change_7d: 0.0,
            liquidity_usd: 0.0,
            market_cap: None,
            fdv: None,
            total_supply: None,
            circulating_supply: None,
            price_history: vec![],
            volume_history: vec![],
            holder_history: vec![],
            dex_pairs: pairs,
            fetched_at: 0,
            top_10_concentration: None,
            top_50_concentration: None,
            top_100_concentration: None,
            price_change_6h: 0.0,
            price_change_1h: 0.0,
            total_buys_24h: 0,
            total_sells_24h: 0,
            total_buys_6h: 0,
            total_sells_6h: 0,
            total_buys_1h: 0,
            total_sells_1h: 0,
            token_age_hours: None,
            image_url: None,
            websites: vec![],
            socials: vec![],
            dexscreener_url: None,
        }
    }

    #[tokio::test]
    async fn dex_book_comes_from_the_most_liquid_pool() {
        let a = analytics(
            "ethereum",
            vec![
                dex_pair(1_000.0, 10.0),
                dex_pair(50_000.0, 777.0),
                dex_pair(2_000.0, 20.0),
            ],
        );
        let (book, volume) = fetch_book_and_volume(
            &MarketVenue::Dex(DexVenue::Ethereum),
            &Pair::parse("USDC"),
            &HealthThresholds::default(),
            &KnownAnalytics(&a),
        )
        .await
        .unwrap();
        assert_eq!(volume, Some(777.0));
        assert_eq!(book.pair, "USDC/USD");
    }

    #[tokio::test]
    async fn dex_without_pools_is_an_error() {
        let a = analytics("solana", vec![]);
        let err = fetch_summary(
            &MarketVenue::Dex(DexVenue::Solana),
            &Pair::parse("USDC"),
            &HealthThresholds::default(),
            &KnownAnalytics(&a),
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("No DEX pairs found for USDC on solana"),
            "{err}"
        );
        // Not found, so the web answers 404 (see scope-web `status_for`).
        assert!(matches!(err, ScopeError::NotFound(_)), "{err:?}");
    }

    /// One call resolves and fetches: a DEX alias reads the analytics'
    /// chain, and an unknown name is the one unknown-venue error.
    #[tokio::test]
    async fn fetch_token_summary_by_name_resolves_then_fetches() {
        let a = analytics("ethereum", vec![dex_pair(1_000.0, 10.0)]);
        let t = HealthThresholds::default();
        assert!(
            fetch_token_summary_by_name_with_http("eth", &a, &t, crate::http::test_transport())
                .await
                .is_ok()
        );
        let err =
            fetch_token_summary_by_name_with_http("polygon", &a, &t, crate::http::test_transport())
                .await
                .unwrap_err();
        assert!(matches!(err, ScopeError::InvalidInput(_)), "{err:?}");
        assert!(err.to_string().contains("Unknown venue"), "{err}");
    }

    /// One chain rule for every caller: analytics from another chain never
    /// build a DEX venue's book, and the check ignores case.
    #[tokio::test]
    async fn dex_analytics_from_another_chain_are_an_error() {
        let a = analytics("Ethereum", vec![dex_pair(1_000.0, 1.0)]);
        let venue = MarketVenue::Dex(DexVenue::Solana);
        let pair = Pair::parse("USDC");
        let t = HealthThresholds::default();
        let err = fetch_summary(&venue, &pair, &t, &KnownAnalytics(&a))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("reads chain solana"), "{err}");

        let same_chain = MarketVenue::Dex(DexVenue::Ethereum);
        assert!(
            fetch_summary(&same_chain, &pair, &t, &KnownAnalytics(&a))
                .await
                .is_ok()
        );
    }
}
