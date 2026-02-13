# Token Crawl Dataflow

Dataflow for `scope crawl [TOKEN] [OPTIONS]` — fetches DEX and block-explorer data, aggregates into TokenAnalytics.

```mermaid
flowchart TB
    subgraph Input
        A[CLI: token or address]
        A --> B{Address format?}
        B -->|Yes| F[infer_chain_from_address]
        B -->|No| C{Saved alias?}
        C -->|Yes| G[Resolved: address + chain]
        C -->|No| D[DexDataSource.search_tokens]
        D --> E[Token search results]
        E --> H[User pick or --yes first]
        H --> G
        F --> G
    end

    subgraph DexFetch
        G --> I1[DexClient.get_token_data]
        I1 --> I{DEX result}
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
        S -->|markdown| V2[report::generate_report + println]
        P --> W{--report?}
        R --> W
        W -->|Some path| X[report::generate_report]
        X --> Y[report::save_report]
    end

    subgraph External
        I1 --> Z[DexScreener API]
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

## Architecture Note

Token resolution uses `DexDataSource` from the factory for search, enabling dependency injection and testability. Both `crawl::run` and `crawl::fetch_analytics_for_input` pass the factory's dex client to `resolve_token_input`.

## TokenAnalytics Fields

- token (symbol, name, address), chain
- holders, total_holders, top_*_concentration
- volume_24h, volume_7d, liquidity_usd
- price_usd, price_change_*, price_history
- dex_pairs, market_cap, fdv
- total_buys/sells_*h, token_age_hours
- socials, websites, image_url
