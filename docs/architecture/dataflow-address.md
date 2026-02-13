# Address Command Dataflow

Dataflow for `scope address [ADDRESS] [OPTIONS]`.

```mermaid
flowchart TB
    subgraph Input
        A[CLI: address + chain + --include-txs --include-tokens --report]
        A --> B[Auto-detect chain if default]
        B --> C[validate_address]
    end

    subgraph Factory
        D[ChainClientFactory]
        D --> E[create_chain_client]
        E --> F[EthereumClient | SolanaClient | TronClient]
    end

    subgraph Fetch
        C --> G[ChainClient.get_balance]
        G --> H[ChainClient.enrich_balance_usd]
        H --> I{Flags}
        I -->|include_txs| J[get_transactions]
        I -->|include_tokens| K[get_token_balances]
        J --> L[AddressReport]
        K --> L
        H --> L
    end

    subgraph Output
        L --> M{format}
        M -->|json| N[println JSON]
        M -->|csv| O[println CSV]
        M -->|table| P[println table]
        L --> Q{--report?}
        Q -->|Some path| R[address_report::generate_address_report]
        R --> S[save_report]
    end

    subgraph External
        F --> T[EVM RPC / Solana RPC / TronGrid]
        H --> U[DexScreener: native token price]
    end

    D -.-> F
    F -.-> G
```

## Data Types

| Stage | Type | Description |
|-------|------|-------------|
| Input | `AddressArgs` | address, chain, format, include_txs, include_tokens, limit, report |
| Fetch | `Balance` | raw, formatted, usd_value |
| Fetch | `Transaction[]` | hash, block, timestamp, from, to, value, status |
| Fetch | `TokenBalance[]` | contract, symbol, name, balance |
| Output | `AddressReport` | address, chain, balance, transaction_count, transactions?, tokens? |
| Report | `String` | Markdown with header, balance, transactions, tokens, footer |
