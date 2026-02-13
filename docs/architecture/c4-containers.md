# Scope — C4 Container Diagram (Level 2)

High-level containers (modules) and their interactions.

```mermaid
C4Container
    title Container Diagram - Scope

    Person(analyst, "Analyst")
    Container_Boundary(scope, "Scope CLI") {
        Container(cli, "CLI", "Rust", "Command parsing and dispatch")
        Container(web, "Web", "Rust/Axum", "HTTP server, REST API, WebSocket, SPA UI")
        Container(chains, "Chains", "Rust", "ChainClient, DexClient, ChainClientFactory")
        Container(compliance, "Compliance", "Rust", "RiskEngine, pattern detection, taint trace")
        Container(market, "Market", "Rust", "Order book fetch, peg health, Biconomy client")
        Container(display, "Display", "Rust", "Charts, reports, format outputs")
        Container(config, "Config", "Rust", "Load/merge config from file and env")
        Container(tokens, "Tokens", "Rust", "Token alias storage")
    }

    Container_Ext(etherscan, "Etherscan")
    Container_Ext(dexscreener, "DexScreener")
    Container_Ext(rpc, "RPCs")
    Container_Ext(biconomy, "Biconomy")

    Rel(analyst, cli, "Commands")
    Rel(analyst, web, "Browser")
    Rel(cli, config, "Load config")
    Rel(web, config, "Load config")
    Rel(web, chains, "Create clients")
    Rel(cli, chains, "Create clients")
    Rel(cli, compliance, "Risk, trace, analyze")
    Rel(cli, market, "Summary")
    Rel(cli, display, "Generate reports")
    Rel(cli, tokens, "Token aliases")
    Rel(chains, etherscan, "EVM data")
    Rel(chains, dexscreener, "Token prices")
    Rel(chains, rpc, "Chain data")
    Rel(compliance, etherscan, "Transactions")
    Rel(market, biconomy, "Order book")
```

## Container Responsibilities

| Container | Description |
|-----------|-------------|
| **CLI** | `clap` argument parsing, `Commands` dispatch, command handlers (address, tx, crawl, discover, monitor, token-health, portfolio, export, compliance, market, report, interactive, setup, web) |
| **Web** | Axum HTTP server, REST API handlers (`/api/*`), WebSocket live monitor (`/ws/monitor`), embedded SPA UI, daemon mode |
| **Chains** | `ChainClient` (Ethereum, Solana, Tron), `DexClient`, `ChainClientFactory`, balance/tx/token fetch |
| **Compliance** | `RiskEngine`, `BlockchainDataClient`, pattern analysis, transaction tracing |
| **Market** | `BiconomyClient`, `OrderBook`, `MarketSummary`, health checks |
| **Display** | ASCII charts, markdown reports (`report`, `address_report`), `format_risk_report` |
| **Config** | YAML load, env overrides, portfolio/monitor settings |
| **Tokens** | Token alias store (symbol → address) |
