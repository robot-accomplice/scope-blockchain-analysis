# Scope Blockchain Analysis

[![CI](https://github.com/robot-accomplice/scope-blockchain-analysis/actions/workflows/ci.yml/badge.svg)](https://github.com/robot-accomplice/scope-blockchain-analysis/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/robot-accomplice/scope-blockchain-analysis/branch/main/graph/badge.svg)](https://codecov.io/gh/robot-accomplice/scope-blockchain-analysis)
[![Crates.io](https://img.shields.io/crates/v/scope-bca.svg)](https://crates.io/crates/scope-bca)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/)

A production-grade command-line tool for blockchain data analysis, portfolio tracking, transaction investigation, and **compliance-grade risk assessment**.

## Features

- **Address Analysis**: Query balances (with USD valuation), transaction history, and token holdings for blockchain addresses
- **Transaction Analysis**: Look up and decode blockchain transactions across all supported chains
- **Token Crawling**: Crawl DEX data for any token -- price, volume, liquidity, holder analysis, and risk scoring with markdown report generation
- **Live Monitoring**: Real-time TUI dashboard with four layout presets (Dashboard, Chart, Feed, Compact), responsive terminal sizing, config-driven widget visibility, price/volume/candlestick charts, price alerts, whale detection, CSV export, and auto-pause on input
- **Portfolio Management**: Track multiple addresses across chains with labels, tags, and aggregated balance views
- **Compliance & Risk Assessment**: Risk scoring, transaction pattern detection, taint analysis, and compliance reporting
- **Data Export**: Export address history and portfolio data to JSON or CSV with date range filtering
- **Interactive Mode**: REPL with preserved context between commands for faster workflow
- **Setup Wizard**: Guided first-run configuration with `scope setup` for API keys and preferences
- **USD Valuation**: Native token balances enriched with real-time USD prices via DexScreener
- **Multi-Chain Support**:
  - EVM chains: Ethereum, Polygon, Arbitrum, Optimism, Base, BSC, Aegis
  - Non-EVM chains: Solana, Tron

## Installation

```bash
# Install from crates.io
cargo install scope-bca
```

Or build from source:

```bash
git clone https://github.com/robot-accomplice/scope-blockchain-analysis.git
cd scope-blockchain-analysis
cargo install --path .
```

## Quick Start

```bash
# Analyze an Ethereum address (auto-detects chain from address format)
scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2

# Include transaction history and token balances
scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --include-txs --include-tokens

# Analyze addresses on other chains (auto-detected or explicit)
scope address DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy --chain solana
scope address TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf --chain tron

# Look up a transaction
scope tx 0xabc123def456789012345678901234567890123456789012345678901234abcd

# Risk assessment for compliance
export ETHERSCAN_API_KEY="your_key_here"
scope compliance risk 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --detailed

# Pattern detection
scope compliance analyze 0xabc... --patterns structuring,layering

# Token crawling with report generation
scope crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --chain ethereum

# Live monitor -- launch directly from the command line
scope monitor USDC                              # monitor by symbol (defaults to ethereum)
scope mon PEPE --chain ethereum --layout chart-focus  # short alias with options

# Export data
scope export --address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --output data.json

# Interactive mode (includes live monitor, portfolio, and all commands)
scope interactive
```

## Compliance Features

Scope includes enterprise-grade compliance and risk analysis. All compliance commands live under `scope compliance`:

### Risk Assessment

```bash
# Basic risk score
scope compliance risk 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2

# Detailed breakdown with evidence
scope compliance risk 0xabc... --detailed --format markdown

# Export for compliance records
scope compliance risk 0xabc... --output risk-report.json
```

### Pattern Detection

```bash
# Detect structuring, layering, and velocity anomalies
scope compliance analyze 0xabc... --patterns structuring,layering,velocity

# Time-range analysis (default: 30d)
scope compliance analyze 0xabc... --range 6m
```

Available pattern types: `structuring`, `layering`, `integration`, `velocity`, `round-numbers`.

### Transaction Tracing

```bash
# Trace fund flow through multiple hops (default depth: 3)
scope compliance trace 0xtxhash... --depth 5 --flag-suspicious
```

### Compliance Reporting

```bash
# Generate a compliance report for a specific jurisdiction
scope compliance compliance-report 0xabc... --jurisdiction us --output report.json

# Detailed SAR report
scope compliance compliance-report 0xabc... --jurisdiction eu --report-type sar --output sar.json
```

Available jurisdictions: `us`, `eu`, `uk`, `switzerland`, `singapore`.
Report types: `summary` (default), `detailed`, `sar`, `travel-rule`.

Compliance output formats: `table` (default), `json`, `yaml`, `markdown`.

**Note**: Set `ETHERSCAN_API_KEY` environment variable for full compliance analysis. Without it, basic scoring is used.

## Token Crawling

Crawl DEX data for any token by address or name. Supports searching by symbol, displaying ASCII charts, and generating markdown reports:

```bash
# Crawl by contract address
scope crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --chain ethereum

# Search by token name or symbol
scope crawl USDC
scope crawl "wrapped ether"

# Specify time period (1h, 24h, 7d, 30d)
scope crawl USDC --period 7d

# Generate a markdown report
scope crawl USDC --report report.md

# Disable ASCII charts, output as JSON
scope crawl USDC --no-charts --format json

# Skip interactive prompts (use first match)
scope crawl USDC --yes

# Save the selected token as an alias for future use
scope crawl USDC --save
```

## Data Export

Export address history or portfolio data to JSON or CSV:

```bash
# Export address transaction history
scope export --address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --output history.json

# Export as CSV with date range and limit
scope export --address 0x742d... --output history.csv --format csv --from 2025-01-01 --to 2025-12-31 --limit 500

# Export portfolio data
scope export --portfolio --output portfolio.json
```

## Interactive Mode

Launch a REPL where context persists between commands. Interactive mode provides access to all commands plus the live monitor:

```bash
$ scope interactive

scope:ethereum> chain solana
Chain set to: solana

scope:solana> address 7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU
# Uses solana chain automatically

scope:solana> format json
Format set to: json

scope:solana> monitor USDC
# Launches a real-time TUI dashboard with price/volume charts

scope:solana> exit
```

Available interactive commands:

- `address` / `addr` -- Analyze a blockchain address
- `tx` / `transaction` -- Analyze a transaction
- `crawl` / `token` -- Crawl token analytics
- `monitor` / `mon` -- Live TUI dashboard with four layout presets, widget toggles, price/volume/candlestick charts, alerts, CSV export, and auto-pause
- `portfolio` / `port` -- Portfolio management (add, remove, list, summary)
- `tokens` / `aliases` -- Token alias management (add, remove, list, recent)
- `setup` / `config` -- Configuration commands
- `chain` -- Set or show current chain
- `format` -- Set or show output format (table, json, csv)
- `limit` -- Set or show transaction limit
- `+tokens` / `+txs` -- Toggle token/transaction display flags
- `trace` / `decode` -- Toggle trace/decode flags
- `ctx` / `context` -- Show current session context
- `clear` / `reset` -- Reset context to defaults
- `help` / `?` -- Show help
- `exit` / `quit` -- Exit interactive mode

### Live Monitor

The monitor launches a real-time TUI dashboard. You can start it two ways:

```bash
# Direct from the command line (no interactive mode needed)
scope monitor USDC
scope mon PEPE --chain ethereum --layout chart-focus --refresh 3

# Or from interactive mode
scope interactive
scope> monitor USDC
```

**Direct-command flags:** `--chain` / `-c`, `--layout` / `-l`, `--refresh` / `-r`, `--scale` / `-s`, `--color-scheme`, `--export` / `-e`.

The monitor supports four layout presets that can be switched at runtime or configured in `config.yaml`:

| Preset | Description |
|---|---|
| **Dashboard** | Charts top, gauges middle, transaction feed bottom (default) |
| **Chart** | Full-width candles (~85%), minimal stats overlay |
| **Feed** | Transaction log takes priority, small metrics + buy/sell on top |
| **Compact** | Price sparkline and metrics only, for small terminals |

The monitor automatically selects the best layout for your terminal size (responsive breakpoints). Manual layout switching disables auto-selection until you press `A`.

**Monitor features:**

- **Price/volume charts**: Line, Candlestick, and Volume Profile modes
- **Six timeframes**: 1m, 5m, 15m, 1h, 4h, 1d
- **Log/linear scale**: Toggle Y-axis scaling for wide price ranges
- **Color schemes**: Green/Red (default), Blue/Orange, Monochrome
- **Holder count**: On-chain holder count (when API key is configured)
- **Liquidity depth**: Per-pair liquidity breakdown across DEXes
- **Alerts**: Configurable price min/max, whale detection, and volume spike thresholds
- **CSV export**: Record live data to `./scope-exports/` with a single keypress
- **Auto-pause**: Optionally pause data fetching while interacting

**Monitor keybindings:**

| Key | Action |
|---|---|
| `Q` / `Esc` | Quit monitor |
| `R` | Force refresh |
| `P` / `Space` | Pause/resume |
| `Shift+P` | Toggle auto-pause on input |
| `E` | Toggle CSV export (REC indicator when active) |
| `L` | Cycle layout forward (Dashboard -> Chart -> Feed -> Compact) |
| `H` | Cycle layout backward |
| `W` + `1-5` | Toggle widget visibility (1=price chart, 2=volume, 3=buy/sell, 4=metrics, 5=activity) |
| `A` | Re-enable auto layout |
| `C` | Cycle chart mode (Line / Candlestick / Volume Profile) |
| `S` | Toggle log/linear scale |
| `/` | Cycle color scheme (Green/Red, Blue/Orange, Monochrome) |
| `T` / `Tab` | Cycle time period |
| `1-6` | Select time period (1m, 5m, 15m, 1h, 4h, 1d) |
| `J` / `K` | Scroll activity log |
| `+` / `-` | Adjust refresh speed |

## Configuration

On first run, Scope detects that no configuration file exists and offers to launch the interactive setup wizard. The wizard walks you through configuring API keys, RPC endpoints, and output preferences, then saves the result to `~/.config/scope/config.yaml`.

You can revisit the setup wizard at any time without editing the config file directly:

```bash
# Re-run the full setup wizard
scope setup

# View current configuration status
scope setup --status

# Configure a single API key
scope setup --key etherscan

# Reset configuration to defaults
scope setup --reset
```

The generated config file follows this structure:

```yaml
chains:
  # EVM-compatible chains
  ethereum_rpc: "https://mainnet.infura.io/v3/YOUR_KEY"
  bsc_rpc: "https://bsc-dataseed.binance.org"
  aegis_rpc: "http://localhost:8545"

  # Non-EVM chains
  solana_rpc: "https://api.mainnet-beta.solana.com"
  tron_api: "https://api.trongrid.io"

  # Block explorer API keys
  api_keys:
    etherscan: "YOUR_API_KEY"
    bscscan: "YOUR_API_KEY"
    solscan: "YOUR_API_KEY"
    tronscan: "YOUR_API_KEY"

output:
  format: table  # table, json, or csv
  color: true

portfolio:
  data_dir: "~/.local/share/scope"

# Monitor TUI configuration (all optional, shown with defaults)
monitor:
  layout: dashboard          # dashboard | chart-focus | feed | compact
  refresh_seconds: 10
  scale: linear              # linear | log
  color_scheme: green-red    # green-red | blue-orange | monochrome
  auto_pause_on_input: false # pause data fetching while interacting
  widgets:
    price_chart: true
    volume_chart: true
    buy_sell_pressure: true
    metrics_panel: true
    activity_log: true
    holder_count: true
    liquidity_depth: true
  alerts:
    price_min: null           # alert when price drops below (e.g., 0.50)
    price_max: null           # alert when price exceeds (e.g., 2.00)
    whale_min_usd: null       # whale detection threshold (e.g., 10000.0)
    volume_spike_threshold_pct: null  # volume spike % (e.g., 100.0)
  export:
    path: null                # base directory (default: ./scope-exports/)
```

### Environment Variables

- `ETHERSCAN_API_KEY` - Required for compliance features
- `SCOPE_CONFIG` - Custom config file path (overrides default location)
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
| -------- | ------- | -------------- | -------------------- |
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

- **Check** - Fast compilation verification
- **Format** - Code style enforcement (`cargo fmt`)
- **Clippy** - Lint analysis (`cargo clippy`)
- **Test** - Unit and integration tests (`cargo test`)
- **Docs** - Documentation build verification
- **Build Release** - Release binaries (Linux, macOS)
- **Security Audit** - Dependency vulnerability scanning

## License

MIT License - see [LICENSE](LICENSE) for details.

---

*Built with Rust. Designed for analysts, compliance officers, and blockchain researchers.*
