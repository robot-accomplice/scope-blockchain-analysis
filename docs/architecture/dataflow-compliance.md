# Compliance Dataflow

Dataflow for `scope compliance risk`, `analyze`, `trace`, and `compliance-report`.

## Risk Assessment (`scope compliance risk`)

```mermaid
flowchart TB
    subgraph Input
        A[CLI: address, chain?, format, detailed, output?]
        A --> B[detect_chain or use --chain]
    end

    subgraph Engine
        B --> C{ETHERSCAN_API_KEY?}
        C -->|Yes| D[RiskEngine.with_data_client]
        C -->|No| E[RiskEngine.new]
        D --> F[assess_address]
        E --> F
    end

    subgraph Assess
        F --> G[BlockchainDataClient.get_transactions]
        G --> H[analyze_patterns]
        H --> I[Risk factors: behavioral, association, etc.]
        I --> J[RiskAssessment]
    end

    subgraph Output
        J --> K[format_risk_report: table|json|yaml|markdown]
        K --> L[println]
        J --> M{--output?}
        M -->|Some| N[Write to file by extension]
    end

    subgraph External
        G --> O[Etherscan API]
    end
```

## Pattern Analysis (`scope compliance analyze`)

```mermaid
flowchart LR
    A[address + patterns + range] --> B[BlockchainDataClient.get_transactions]
    B --> C[analyze_patterns]
    C --> D[PatternAnalysis]
    D --> E[velocity_score, structuring_detected, round_number_pattern, unusual_hours]
    E --> F[println]
```

## Transaction Trace (`scope compliance trace`)

```mermaid
flowchart LR
    A[tx_hash + depth] --> B[BlockchainDataClient.trace_transaction]
    B --> C[TraceResult: root_hash + hops]
    C --> D[For each hop: address, amount, depth]
    D --> E[println]
```

## Unified Compliance Report (`scope compliance compliance-report`)

```mermaid
flowchart TB
    subgraph Resolve
        A[target: address or file path] --> B{path.exists?}
        B -->|Yes| C[Read lines: address,chain]
        B -->|No| D[Single address]
        C --> E[Vec of address,chain]
        D --> E
    end

    subgraph PerAddress
        E --> F[RiskEngine.assess_address]
        F --> G[RiskAssessment]
        E --> H[BlockchainDataClient.get_transactions]
        H --> I[analyze_patterns]
        I --> J[PatternAnalysis]
    end

    subgraph Report
        G --> K[format_compliance_report]
        J --> K
        K --> L[Markdown: risk + patterns per address]
        L --> M[std::fs::write]
    end
```
