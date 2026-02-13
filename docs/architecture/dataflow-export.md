# Export Dataflow

Dataflow for `scope export --address ADDR` and `scope export --portfolio`.

```mermaid
flowchart TB
    subgraph Input
        A[CLI: address? | portfolio?, output, format, from?, to?, limit]
        A --> B{source}
        B -->|--address| C[export_address]
        B -->|--portfolio| D[export_portfolio]
    end

    subgraph AddressExport
        C --> E[ChainClient.get_transactions]
        E --> F[Apply date filter: from, to]
        F --> G[Map to TransactionExport]
        G --> H{format}
        H -->|json| I[ExportData: address, chain, transactions]
        H -->|csv| J[CSV: hash, block, timestamp, from, to, value, gas_used, status]
        I --> K[std::fs::write]
        J --> K
    end

    subgraph PortfolioExport
        D --> L[Portfolio::load]
        L --> M[addresses]
        M --> N{format}
        N -->|json| O[serde_json::to_string_pretty]
        N -->|csv| P[CSV: address, label, chain, tags, added_at]
        N -->|markdown| P2[Markdown table]
        O --> K
        P --> K
        P2 --> K
    end

    subgraph External
        E --> Q[Chain RPC / Explorer]
    end
```
