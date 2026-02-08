# BCC - Blockchain Crawler CLI

[![CI](https://github.com/robot-accomplice/bcc/actions/workflows/ci.yml/badge.svg)](https://github.com/robot-accomplice/bcc/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/)

```
  ██████╗  ██████╗ ██████╗             ______ 
  ██╔══██╗██╔════╝██╔════╝    /^|^|^\_/_o__o_\_/^|^|^\
  ██████╔╝██║     ██║         \ \ \ \ \_v__v_/ / / / /
  ██╔══██╗██║     ██║
  ██████╔╝╚██████╗╚██████╗    Blockchain Crawler CLI
  ╚═════╝  ╚═════╝ ╚═════╝
```

A production-grade command-line tool for blockchain data analysis, portfolio tracking, transaction investigation, and **compliance-grade risk assessment**.

## Features

- **Address Analysis**: Query balances (with USD valuation), transaction history, and token holdings for blockchain addresses
- **Transaction Analysis**: Look up and decode blockchain transactions across all supported chains
- **Token Crawling**: Crawl DEX data for any token -- price, volume, liquidity, holder analysis, and risk scoring with markdown report generation
- **Live Monitoring**: Real-time TUI dashboard with price/volume/candlestick charts, buy/sell gauges, and activity logs
- **Portfolio Management**: Track multiple addresses across chains with labels, tags, and aggregated balance views
- **Compliance & Risk Assessment**: Risk scoring, transaction pattern detection, taint analysis, and compliance reporting (NEW)
- **Interactive Mode**: REPL with preserved context between commands for faster workflow
- **USD Valuation**: Native token balances enriched with real-time USD prices via DexScreener
- **Multi-Chain Support**: 
  - EVM chains: Ethereum, Polygon, Arbitrum, Optimism, Base, BSC, Aegis
  - Non-EVM chains: Solana, Tron

## Installation

```bash
# Clone the repository
git clone https://github.com/robot-accomplice/bcc.git
cd bcc

# Build and install
cargo install --path .
```

## Quick Start

```bash
# Analyze an Ethereum address (auto-detects chain from address format)
bcc address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2

# Include transaction history and token balances
bcc address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --include-txs --include-tokens

# Analyze addresses on other chains (auto-detected or explicit)
bcc address DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy --chain solana
bcc address TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf --chain tron

# Look up a transaction
bcc tx 0xabc123def456789012345678901234567890123456789012345678901234abcd

# Risk assessment for compliance (NEW)
export ETHERSCAN_API_KEY="your_key_here"
bcc risk 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --detailed

# Pattern detection (NEW)
bcc analyze 0xabc... --patterns structuring,layering

# Token crawling with report generation
bcc crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --chain ethereum

# Live monitor with real-time dashboard
bcc monitor 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --chain ethereum

# Interactive mode
bcc interactive
```

## Compliance Features (New)

BCC now includes enterprise-grade compliance and risk analysis:

### Risk Assessment
```bash
# Basic risk score
bcc risk 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2

# Detailed breakdown with evidence
bcc risk 0xabc... --detailed --format markdown

# Export for compliance records
bcc risk 0xabc... --output risk-report.json
```

### Pattern Detection
```bash
# Detect structuring, layering, velocity anomalies
bcc analyze 0xabc... --patterns structuring,layering,velocity

# Time-range analysis
bcc analyze 0xabc... --range 30d
```

### Transaction Tracing
```bash
# Trace fund flow through multiple hops
bcc trace 0xtxhash... --depth 5 --flag-suspicious
```

**Note**: Set `ETHERSCAN_API_KEY` environment variable for full compliance analysis. Without it, basic scoring is used.

## Interactive Mode

Launch a REPL where context persists between commands:

```bash
$ bcc interactive

bcc:ethereum> chain solana
Chain set to: solana

bcc:solana> address 7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU
# Uses solana chain automatically

bcc:solana> format json
Format set to: json

bcc:solana> exit
```

## Configuration

Create `~/.config/bcc/config.yaml`:

```yaml
chains:
  ethereum_rpc: "https://mainnet.infura.io/v3/YOUR_KEY"
  solana_rpc: "https://api.mainnet-beta.solana.com"
  
  api_keys:
    etherscan: "YOUR_ETHERSCAN_KEY"
    solscan: "YOUR_SOLSCAN_KEY"

output:
  format: table
  color: true
```

### Environment Variables

- `ETHERSCAN_API_KEY` - Required for compliance features
- `BCC_CONFIG` - Path to custom config file
- `RUST_LOG` - Log level override

## Development

```bash
# Run tests
cargo test

# Format code
cargo fmt

# Run lints
cargo clippy -- -D warnings

# Build release
cargo build --release
```

Or use just:
```bash
just test      # Run all tests
just ci-test   # Full CI workflow
just format    # Format code
just lint      # Run lints
```

## Supported Chains

| Chain | Type | Address Format | Explorer |
|-------|------|---------------|----------|
| Ethereum | EVM | 0x... | Etherscan |
| Polygon | EVM | 0x... | Polygonscan |
| Arbitrum | EVM | 0x... | Arbiscan |
| Optimism | EVM | 0x... | Optimistic Etherscan |
| Base | EVM | 0x... | Basescan |
| BSC | EVM | 0x... | BscScan |
| Aegis | EVM | 0x... | JSON-RPC |
| Solana | Non-EVM | Base58 | Solscan |
| Tron | Non-EVM | T... | Tronscan |

## CI/CD

GitHub Actions workflow runs:
- ✅ Check - Fast compilation
- ✅ Format - Code style
- ✅ Clippy - Linting
- ✅ Test - Unit and integration tests
- ✅ Docs - Documentation build
- ✅ Build - Release binaries (Linux, macOS)
- ✅ Security - Dependency audit

## License

MIT License - see [LICENSE](LICENSE) for details.

---

*Built with Rust. Designed for analysts, compliance officers, and blockchain researchers.*
