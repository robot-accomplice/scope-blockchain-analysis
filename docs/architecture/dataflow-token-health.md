# Token Health Dataflow

Dataflow for `scope token-health [TOKEN]` and `scope health [TOKEN]` — composite DEX analytics + optional market/order book.

**Venues:** CEX (Binance, Biconomy) or DEX (Ethereum, Solana) synthesized from DexScreener liquidity.

```mermaid
flowchart TB
    subgraph Input
        A[CLI: token, chain, --with-market, --venue, format]
        A --> B[crawl::fetch_analytics_for_input]
        B --> C[ChainClientFactory]
    end

    subgraph DEX
        C --> D[create_dex_client + create_chain_client]
        D --> E[fetch_token_analytics via crawl]
        E --> F[TokenAnalytics: price, volume, dex_pairs, liquidity]
    end

    subgraph Market
        F --> G{--with-market?}
        G -->|No| H[market_summary = None]
        G -->|Yes| I{venue.is_cex?}
        I -->|Yes| J[MarketVenue.create_client]
        J --> K[fetch_order_book: Binance/Biconomy REST]
        I -->|No| L{DEX pairs + chain match?}
        L -->|Yes| M[order_book_from_analytics]
        M --> N[OrderBook]
        K --> N
        L -->|No| H
        N --> O[MarketSummary::from_order_book]
    end

    subgraph Output
        F --> P{format}
        O -.-> P
        H -.-> P
        P -->|markdown| Q[token_health_to_markdown]
        P -->|json| R[token_health_to_json]
        P -->|table| S[output_token_health_table]
        Q --> T[println]
        R --> T
        S --> T
    end

    subgraph External
        E --> U[DexScreener API]
        K --> V[Binance / Biconomy REST]
    end
```

## Venue Routing

| Venue | Type | Source | Pair format |
|-------|------|--------|-------------|
| binance | CEX | Binance Spot REST | USDCUSDT |
| biconomy | CEX | Biconomy REST | USDC_USDT |
| eth | DEX | DexScreener liquidity | Synthesized from best pair |
| solana | DEX | DexScreener liquidity | Synthesized from best pair |

## Config Override

When `--ai` is set (or `config.output.format == Markdown`), output format is forced to markdown for agent/LLM consumption.
