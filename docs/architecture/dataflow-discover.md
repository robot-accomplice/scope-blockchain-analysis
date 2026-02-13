# Discover Dataflow

Dataflow for `scope discover [OPTIONS]` — browse trending and boosted tokens from DexScreener.

## Overview

The discover command fetches curated token lists from DexScreener's discovery APIs. No API key required.

```mermaid
flowchart TB
    subgraph Input
        A[CLI: scope discover] --> B[--source: profiles|boosts|top-boosts]
        B --> C[--chain optional filter]
        C --> D[--limit default 15]
    end

    subgraph DexClient
        D --> E{source?}
        E -->|profiles| F[get_token_profiles]
        E -->|boosts| G[get_token_boosts]
        E -->|top-boosts| H[get_token_boosts_top]
    end

    subgraph DexScreener
        F --> J[GET /token-profiles/latest/v1]
        G --> K[GET /token-boosts/latest/v1]
        H --> L[GET /token-boosts/top/v1]
    end

    subgraph Filter
        J --> M[Vec DiscoverToken]
        K --> M
        L --> M
        M --> N{--chain?}
        N -->|Some| O[Filter by chain_id]
        N -->|None| P[Pass through]
        O --> Q[Take --limit]
        P --> Q
    end

    subgraph Output
        Q --> R{format}
        R -->|table| S[Pretty print: chain, address, description, url]
        R -->|json| T[Serialize DiscoverRow[]]
        R -->|csv| U[chain,address,description,url]
    end

    style A fill:#e1f5fe
    style E fill:#fff3e0
```

## Discover Sources

| Source | DexScreener endpoint | Description |
|--------|----------------------|-------------|
| **profiles** | `/token-profiles/latest/v1` | Featured token profiles |
| **boosts** | `/token-boosts/latest/v1` | Recently boosted tokens |
| **top-boosts** | `/token-boosts/top/v1` | Tokens with most active boosts |

## DiscoverToken Structure

```text
DiscoverToken {
  chain_id: String,
  token_address: String,
  url: String,
  description: Option<String>,
  links: Vec<DiscoverLink>,
}
```

## Data Source

DexScreener discovery APIs (rate limit 60 req/min). No authentication required.
