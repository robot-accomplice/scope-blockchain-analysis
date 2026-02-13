# Address Command Dataflow

Dataflow for `scope address [ADDRESS] [OPTIONS]`.

```mermaid
flowchart TB
    subgraph Input
        A[CLI: address + chain + --include-txs --include-tokens --report --dossier]
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
        I -->|dossier| M[RiskEngine.assess_address]
        M --> N[RiskAssessment]
    end

    subgraph Output
        L --> O{format}
        N -.->|dossier| O
        O -->|json| P[println JSON]
        O -->|csv| Q[println CSV]
        O -->|table| R[println table]
        O -->|markdown| S[generate_address_report or generate_dossier_report]
        S --> T[println]
        L --> U{--report?}
        U -->|Some path| V[address_report::generate_address_report]
        V --> W[save_report]
    end

    subgraph External
        F --> X1[EVM RPC / Solana RPC / TronGrid]
        H --> X2[DexScreener: native token price]
    end

    D -.-> F
    F -.-> G
```

## Data Types

| Stage | Type | Description |
|-------|------|-------------|
| Input | `AddressArgs` | address, chain, format, include_txs, include_tokens, limit, report, dossier |
| Fetch | `Balance` | raw, formatted, usd_value |
| Fetch | `Transaction[]` | hash, block, timestamp, from, to, value, status |
| Fetch | `TokenBalance[]` | contract, symbol, name, balance |
| Output | `AddressReport` | address, chain, balance, transaction_count, transactions?, tokens? |
| Report | `String` | Markdown with header, balance, transactions, tokens, footer |
