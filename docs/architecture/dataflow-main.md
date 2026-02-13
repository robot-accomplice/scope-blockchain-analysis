# Main Application Dataflow

High-level flow from CLI entry to command execution.

```mermaid
flowchart TB
    subgraph Entry
        A[User: scope &lt;cmd&gt;] --> B[main.rs]
        B --> C[Clap parse]
        C --> D[Config.load]
        D --> E{Config exists?}
        E -->|No| F[Setup wizard]
        F --> D
        E -->|Yes| G[DefaultClientFactory.new]
    end

    subgraph Dispatch
        G --> H[Commands match]
        H --> I1[address::run]
        H --> I2[tx::run]
        H --> I3[crawl::run]
        H --> I4[portfolio::run]
        H --> I5[export::run]
        H --> I6[monitor::run_direct]
        H --> I7[compliance::handle_*]
        H --> I8[market::run]
        H --> I9[report::run]
        H --> I10[interactive::run]
        H --> I11[token_health::run]
        H --> I12[discover::run]
        H --> I13[setup::run]
    end

    subgraph Factory
        G -.-> J[create_chain_client]
        G -.-> K[create_dex_client]
    end

    style A fill:#e1f5fe
    style H fill:#fff3e0
```

## Flow Summary

1. **Parse** — CLI args → `Commands` enum
2. **Config** — Load `~/.config/scope/config.yaml`, merge with env
3. **Factory** — Build `DefaultClientFactory` with chain config
4. **Dispatch** — Match command, call handler with config + factory
5. **Handler** — Uses factory for chain/DEX clients, config for output preferences
