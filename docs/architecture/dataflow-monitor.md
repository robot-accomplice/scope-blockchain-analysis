# Live Monitor Dataflow

Dataflow for `scope monitor [TOKEN]` — real-time TUI dashboard.

```mermaid
flowchart TB
    subgraph Init
        A[CLI: token, chain, layout, refresh, etc.] --> B[Resolve token to address]
        B --> C[ChainClientFactory.create_chain_client]
        B --> D[ChainClientFactory.create_dex_client]
    end

    subgraph PollLoop["Poll loop (every refresh_seconds)"]
        E[DexClient.get_token_data] --> F[Price, volume, pairs]
        G[ChainClient.get_token_holder_count] --> H[Holder count]
        F --> I[MonitorState]
        H --> I
    end

    subgraph TUI
        I --> J[ratatui::terminal.draw]
        J --> K[Layout: Dashboard | Chart | Feed | Compact]
        K --> L[Widgets: price chart, volume, buy/sell gauge, metrics, activity log]
        L --> M[Key handling: Q=quit, E=export CSV, P=pause, etc.]
    end

    subgraph DataFlow
        N[DexScreener API] --> E
        O[Etherscan/Explorer] --> G
        I --> P{Export enabled?}
        P -->|Yes| Q[Append row to scope-exports/*.csv]
    end
```

## Monitor State Updates

| Field | Source | Refresh |
|-------|--------|---------|
| price_usd, volume_24h | DexScreener | Every N seconds |
| price_history | DexScreener pairs | For candlestick |
| holder_count | Block explorer | When API key set |
| recent_txs | DexScreener / derived | Per poll |
