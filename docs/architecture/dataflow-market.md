# Market Summary Dataflow

Dataflow for `scope market summary [PAIR] [OPTIONS]` — peg and order book health.

```mermaid
flowchart TB
    subgraph Input
        A[CLI: pair, chain, peg, thresholds, format, every?, duration?, report?, csv?]
        A --> B{BiconomyClient.new}
    end

    subgraph Fetch
        B --> C[fetch_order_book]
        C --> D[OrderBook: bids, asks]
        D --> E[MarketSummary::from_order_book]
        E --> F[HealthThresholds: peg_range, min_levels, min_depth, bid/ask ratio]
        F --> G[MarketSummary]
    end

    subgraph OneShot
        G --> H{report?}
        H -->|Yes| I[market_summary_to_markdown]
        I --> J[std::fs::write]
        H -->|No| K[format_text or JSON]
        K --> L[println]
    end

    subgraph RepeatMode["Repeat Mode (--every + --duration)"]
        G --> M[Append to CSV row]
        M --> N[timestamp, best_bid, best_ask, mid_price, spread, bid_depth, ask_depth, healthy]
        N --> O[csv_path append]
        G --> P[last_summary = summary]
        P --> Q[After loop: write final report]
    end

    subgraph External
        C --> R[Biconomy API: /api/v1/depth?symbol=PAIR]
    end
```

## Health Checks

| Check | Description |
|-------|-------------|
| No sells below peg | Ask levels below peg_target flagged |
| Bid/ask ratio | Depth ratio within min/max |
| Min levels | At least N levels per side |
| Min depth | Total depth ≥ threshold |
