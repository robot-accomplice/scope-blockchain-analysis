# Token Crawl Dataflow

Dataflow for `scope crawl [TOKEN] [OPTIONS]` — fetches DEX and block-explorer data, aggregates into TokenAnalytics.

```mermaid
flowchart TB
    subgraph Input
        A[CLI: token or address]
        A --> B{Address format?}
        B -->|No| C[DexClient.search_tokens]
        C --> D[Token search results]
        D --> E[User pick or --yes first]
        B -->|Yes| F[infer_chain_from_address]
        E --> G[Resolved: address + chain]
        F --> G
    end

    subgraph DexFetch
        G --> H[DexClient.get_token_data]
        H --> I{DEX result}
        I -->|OK| J[fetch_analytics_with_dex]
        I -->|NotFound| K[fetch_analytics_from_explorer]
        I -->|Err| L[Return error]
    end

    subgraph WithDex
        J --> M[dex_data: DexTokenData]
        M --> N[ChainClient.get_token_holders]
        N --> O[Aggregate: pairs, volume, holders, concentration]
        O --> P[TokenAnalytics]
    end

    subgraph ExplorerOnly
        K --> Q[ChainClient.get_token_info + get_token_holders]
        Q --> R[TokenAnalytics with limited data]
    end

    subgraph Output
        P --> S{format}
        R --> S
        S -->|json| T[println]
        S -->|csv| U[output_csv]
        S -->|table| V[output_table]
        P --> W{--report?}
        R --> W
        W -->|Some path| X[report::generate_report]
        X --> Y[report::save_report]
    end

    subgraph External
        H --> Z[DexScreener API]
        N --> AA[Etherscan/Polygonscan/etc]
        Q --> AA
    end
```

## Data Sources

| Source | Data |
|--------|------|
| **DexScreener** | price, volume (1h/6h/24h), liquidity, pairs, price_history, volume_history |
| **Block Explorer** | token holders, holder percentages |
| **Fallback** | When no DEX pairs: explorer-only TokenAnalytics |

## TokenAnalytics Fields

- token (symbol, name, address), chain
- holders, total_holders, top_*_concentration
- volume_24h, volume_7d, liquidity_usd
- price_usd, price_change_*, price_history
- dex_pairs, market_cap, fdv
- total_buys/sells_*h, token_age_hours
- socials, websites, image_url
