# Scope — C4 Context Diagram (Level 1)

System context: Scope and its external dependencies.

```mermaid
C4Context
    title System Context - Scope Blockchain Analysis

    Person(analyst, "Analyst", "Compliance officer, researcher, or trader")

    System(scope, "Scope", "CLI tool for blockchain data analysis, compliance, and reporting")

    System_Ext(etherscan, "Etherscan API", "EVM chain data: balances, txs, token holders")
    System_Ext(dexscreener, "DexScreener API", "Token prices, volume, liquidity, DEX pairs")
    System_Ext(rpc_evm, "EVM RPC", "Ethereum, Polygon, Arbitrum, Base, BSC, etc.")
    System_Ext(rpc_solana, "Solana RPC", "Solana chain data")
    System_Ext(trongrid, "TronGrid API", "Tron chain data")
    System_Ext(biconomy, "Biconomy API", "CEX-style order book depth")
    System_Ext(config, "Config File", "~/.config/scope/config.yaml")

    Rel(analyst, scope, "Runs commands")
    Rel(scope, etherscan, "Balance, txs, holders (API key)")
    Rel(scope, dexscreener, "Token data, prices, discovery")
    Rel(scope, rpc_evm, "Balance, txs, tokens")
    Rel(scope, rpc_solana, "Balance, txs, SPL tokens")
    Rel(scope, trongrid, "Balance, txs, TRC-20")
    Rel(scope, biconomy, "Order book depth")
    Rel(scope, config, "Reads/writes")
```

## External Systems

| System | Purpose |
|--------|---------|
| **Etherscan API** | EVM block explorer data; requires API key for compliance features |
| **DexScreener** | DEX token data, prices, search, trending/boosted discovery; no key required |
| **EVM RPC** | Direct chain queries (Infura, Alchemy, public RPC) |
| **Solana RPC** | Solana chain queries |
| **TronGrid** | Tron chain queries |
| **Biconomy** | CEX-style order book for stablecoin markets |
| **Config File** | API keys, RPC URLs, preferences |
