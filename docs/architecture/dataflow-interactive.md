# Interactive Mode Dataflow

Dataflow for `scope interactive` — REPL with persistent context.

```mermaid
flowchart TB
    subgraph Init
        A[scope interactive] --> B[ rustyline::DefaultEditor]
        B --> C[SessionContext: chain, format, limit, include_txs, include_tokens]
    end

    subgraph Loop["REPL Loop"]
        D[Read line] --> E[Parse command]
        E --> F{Command}
        F -->|address| G[address::run]
        F -->|tx| H[tx::run]
        F -->|crawl| I[crawl::run]
        F -->|monitor| J[monitor::run_direct]
        F -->|portfolio| K[portfolio::run]
        F -->|chain| L[Update context.chain]
        F -->|format| M[Update context.format]
        F -->|limit| N[Update context.limit]
        F -->|+tokens| O[Toggle context.include_tokens]
        F -->|+txs| P[Toggle context.include_txs]
    end

    subgraph Context
        C -.-> Q[Injected into address/tx/crawl args]
        L --> C
        M --> C
        N --> C
        O --> C
        P --> C
    end

    subgraph Dispatch
        G --> R[Uses context for chain, format, etc.]
        H --> R
        I --> R
    end
```

## Context Persistence

| Setting | Default | Commands that use it |
|---------|---------|----------------------|
| chain | ethereum | address, tx, crawl, monitor |
| format | table | address, tx, crawl, portfolio |
| limit | 100 | address (tx count) |
| include_txs | false | address |
| include_tokens | false | address |
| last_address | — | address (when no arg) |
| last_tx | — | tx (when no arg) |
