# Transaction Command Dataflow

Dataflow for `scope tx [HASH]`.

```mermaid
flowchart TB
    subgraph Input
        A[CLI: hash, chain?]
        A --> B[infer_chain_from_hash]
        B --> C[ChainClient from factory]
    end

    subgraph Fetch
        C --> D[get_transaction]
        D --> E{Chain type}
        E -->|EVM| F[Etherscan proxy / RPC]
        E -->|Solana| G[getTransaction RPC]
        E -->|Tron| H[TronGrid getTransaction]
        F --> I[Transaction]
        G --> I
        H --> I
    end

    subgraph Output
        I --> J{format}
        J -->|json| K[println JSON]
        J -->|table| L[println: hash, from, to, value, status, gas]
        J -->|csv| M[println CSV]
    end
```

## Transaction Fields

- hash, block_number, timestamp
- from, to, value
- gas_limit, gas_used, gas_price
- status (success/failure)
- input (calldata)
