# Market Summary Dataflow

Dataflow for `scope market summary [SYMBOL] [OPTIONS]` — peg, order book health, volume, and execution checks.

**Venues:** Binance (default), Biconomy, Ethereum DEX, Solana DEX. CEX uses REST depth APIs; DEX synthesizes from DexScreener via `crawl::fetch_analytics_for_input`.

```mermaid
flowchart TB
    subgraph Input
        A[CLI: pair, market_venue, chain, peg, thresholds, format, every?, duration?, report?, csv?]
        A --> B{venue.is_cex?}
    end

    subgraph CEX["CEX (Binance, Biconomy)"]
        B -->|Yes| C1[venue.create_client / BiconomyClient with custom URL]
        C1 --> C2[fetch_order_book]
        C2 --> C3{Binance?}
        C3 -->|Yes| C4[fetch_24h_volume]
        C3 -->|No| C5[volume = None]
        C4 --> C6[OrderBook + volume_24h]
        C5 --> C6
    end

    subgraph DEX["DEX (Ethereum, Solana)"]
        B -->|No| D1[crawl::fetch_analytics_for_input]
        D1 --> D2[order_book_from_analytics]
        D2 --> D3[best_pair.volume_24h]
        D3 --> D4[OrderBook + volume_24h]
    end

    subgraph Summary
        C6 --> E[MarketSummary::from_order_book]
        D4 --> E
        E --> F[peg, depth, execution_10k_buy, execution_10k_sell]
        F --> G[MarketSummary]
    end

    subgraph Output
        G --> H{report?}
        H -->|Yes| I[market_summary_to_markdown]
        I --> J[std::fs::write]
        H -->|No| K[format_text or JSON]
        K --> L[println]
    end

    subgraph RepeatMode["Repeat Mode (--every + --duration)"]
        G --> M[Append to CSV row]
        M --> N[timestamp, best_bid, best_ask, mid_price, spread, volume_24h?, bid_depth, ask_depth, healthy]
        N --> O[csv_path append]
        G --> P[last_summary = summary]
        P --> Q[After loop: write final report]
    end
```

## Venues

| Venue   | Symbol format | Volume 24h        | Execution check                  |
|---------|---------------|-------------------|----------------------------------|
| Binance | USDCUSDT      | Ticker 24hr       | Order book walk (10k USDT)       |
| Biconomy| USDC_USDT     | —                 | Order book walk                  |
| Ethereum| DEX           | DexPair.volume_24h | Synthesized book walk            |
| Solana  | DEX           | DexPair.volume_24h | Synthesized book walk            |

## Health Checks

| Check | Description |
|-------|-------------|
| No sells below peg | Ask levels below peg_target flagged |
| Bid/ask ratio | Depth ratio within min/max |
| Min levels | At least N levels per side |
| Min depth | Total depth ≥ threshold |

## Volume & Execution

- **Volume 24h:** From Binance ticker (`quoteVolume`) or DEX pair analytics. Omitted for Biconomy.
- **Execution 10k:** Simulates buying/selling 10k USDT by walking the order book; reports slippage in bps or "insufficient liquidity".
