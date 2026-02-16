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
- **Live Monitoring**: Real-time TUI dashboard with five layout presets (Dashboard, Chart, Feed, Compact, Exchange), responsive terminal sizing, config-driven widget visibility, price/volume/candlestick charts, price alerts, whale detection, CSV export, and auto-pause on input
- **Exchange Venues**: Data-driven CEX integration via YAML descriptors — 11 built-in venues (Binance, Biconomy, Bitget, Bybit, Coinbase, Crypto.com, Gate.io, HTX, Kraken, MEXC, OKX) with order book, ticker, trades, and OHLC support; add custom venues by dropping a YAML file in `~/.config/scope/venues/`
- **Portfolio Management**: Track multiple addresses across chains with labels, tags, and aggregated balance views
- **Compliance & Risk Assessment**: Risk scoring, transaction pattern detection, taint analysis, and compliance reporting
- **Data Export**: Export address history and portfolio data to JSON or CSV with date range filtering
- **Token Discovery**: Browse trending and boosted tokens from DexScreener (`scope discover` / `scope disc`)
- **Market Command** (`scope market summary`, `ohlc`, `trades`): Peg and order book health; OHLC/candlestick data; recent trades; any CEX venue from the registry or DEX (Ethereum, Solana); repeat mode with `--every`/`--duration`; `--report` and `--csv` export
- **Token Health Suite** (`scope token-health` / `scope health`): DEX analytics with optional order book summary; any venue from the registry or DEX
- **Venue Management** (`scope venues` / `scope ven`): List available venues, view the YAML schema, initialise user venues directory, validate custom descriptors
- **Contract Analysis** (`scope contract` / `scope ct`): Comprehensive smart contract security analysis — source code verification, proxy detection (EIP-1967/1822/1167/Diamond), access control mapping, vulnerability heuristics (reentrancy, selfdestruct, unchecked calls, tx.origin, overflow), DeFi protocol checks (oracle, flash loan, DEX slippage), external intelligence (GitHub linking, audit reports, Sourcify), and security scoring (0-100)
- **Insights** (`scope insights` / `insight`): Infer chain and type (address, tx, token) from input; run relevant analyses and present unified observations
- **Agent Output** (`scope --ai`): Global flag for markdown output to stdout (address, tx, crawl, discover, portfolio, export, token-health)
- **Reporting**: Markdown reports for address, token, portfolio, and market commands; batch reports for multiple addresses (`scope report batch`); address dossier (address + risk combined)
- **Shell Completion**: Tab-completion for bash, zsh, and fish (`scope completions <shell>`)
- **Progress Indicators**: Spinners and progress bars for all long-running operations; auto-hidden in pipes
- **Colorized Errors**: Errors in red, remediation hints dimmed, plain text when piped
- **Browser Mode** (`scope web` / `scope serve`): Locally hosted web UI (feature-gated; build with `--features web`)
- **Interactive Mode**: REPL with preserved context between commands for faster workflow
- **Setup Wizard**: Guided first-run configuration with `scope setup` for API keys and preferences
- **USD Valuation**: Native token balances enriched with real-time USD prices via DexScreener
- **Multi-Chain Support**:
  - EVM chains: Ethereum, Polygon, Arbitrum, Optimism, Base, BSC
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

## Shell Completion

Generate tab-completion scripts for your shell:

```bash
# Bash
scope completions bash > ~/.local/share/bash-completion/completions/scope

# Zsh (add to your fpath)
scope completions zsh > ~/.zfunc/_scope
# Then add to .zshrc: fpath=(~/.zfunc $fpath); autoload -Uz compinit && compinit

# Fish
scope completions fish > ~/.config/fish/completions/scope.fish
```

## Output Formats

Scope supports multiple output formats via `--format` or the global `--ai` flag:

| Flag | Format | Use case |
|------|--------|----------|
| *(default)* | Table | Human-readable terminal output |
| `--format json` | JSON | Programmatic consumption, piping to `jq` |
| `--format csv` | CSV | Spreadsheet import |
| `--ai` | Markdown | LLM/agent parsing (global flag, all commands) |

Use `--format json` when piping output to other tools or scripts. Use `--ai` when feeding output to an LLM or agent for analysis.

## Quick Start

**-> [Full Quickstart Guide](docs/QUICKSTART.md)** — workflow-oriented guide for due diligence, token research, compliance, monitoring, and more.

```bash
# Analyze an Ethereum address (auto-detects chain from address format)
scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2

# Include transaction history and token balances
scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --include-txs --include-tokens

# Save address report to markdown
scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --report report.md

# Wallet dossier: address + risk assessment in one view
scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --dossier
scope address 0x742d... --dossier --report dossier.md

# Analyze addresses on other chains (auto-detected or explicit)
scope address DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy --chain solana
scope address TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf --chain tron

# Look up a transaction
scope tx 0xabc123def456789012345678901234567890123456789012345678901234abcd

# Analyze a smart contract (security scoring, proxy detection, vulnerabilities)
scope contract 0xdAC17F958D2ee523a2206206994597C13D831ec7
scope ct 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --json

# Risk assessment for compliance
export ETHERSCAN_API_KEY="your_key_here"
scope compliance risk 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --detailed

# Pattern detection
scope compliance analyze 0xabc... --patterns structuring,layering

# Token crawling with report generation
scope crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --chain ethereum

# Live monitor -- launch directly from the command line
scope monitor USDC                                          # defaults to ethereum
scope mon PEPE --chain ethereum --layout chart-focus        # short alias with options
scope monitor USDC --layout exchange                        # exchange-style: order book + chart + trades
scope monitor USDC --venue binance                           # real exchange OHLC instead of synthetic candles

# Exchange venues
scope venues list                                           # show all available venues
scope ven list --format json                                # machine-readable
scope venues schema                                         # YAML schema for writing custom venues
scope venues init                                           # copy built-in descriptors to ~/.config/scope/venues/
scope venues validate my-venue.yaml                         # validate a custom descriptor

# Token health (DEX + optional market)
scope token-health USDC
scope health USDC --with-market --venue binance

# Market peg and order book health
scope market summary USDC
scope market summary PUSD --venue biconomy --format json
scope market summary USDC --venue kraken

# OHLC and recent trades
scope market ohlc USDC --venue binance --interval 1h --limit 100
scope market trades USDC --venue binance --limit 50

# Unified insights — infer type and chain from any target
scope insights 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2   # address -> balance, tokens
scope insights 0xabc123...                                   # tx hash -> decoded tx
scope insights USDC                                          # token symbol -> DEX analytics

# Discover trending and boosted tokens
scope discover
scope discover --source boosts --chain ethereum --limit 10

# Export data
scope export --address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --output data.json

# Batch report for multiple addresses
scope report batch --addresses 0x742d...,0xabc... --output batch-report.md

# Agent-friendly markdown output
scope --ai address 0x742d...

# Interactive mode (includes live monitor, portfolio, and all commands)
scope interactive
```

### Browser Mode

Launch a local web UI with the same features as the CLI:

```bash
# Start web server (default: http://127.0.0.1:8080)
scope web

# Custom port
scope web --port 3000

# Run as background daemon
scope web --daemon

# Stop running daemon
scope web --stop

# Alias: 'serve'
scope serve --port 9090
```

The web UI provides:
- All CLI commands accessible via browser forms
- JSON REST API at `/api/*` for programmatic access (exchange: snapshot, OHLC, trades)
- Live WebSocket monitor with real-time price charts
- Exchange venue selector and market snapshots
- Configuration status and setup page

### Command Map

Not sure which command to use? Here's a quick decision tree:

| I want to... | Command |
|--------------|---------|
| Look up an address (balance, txs, tokens) | `scope address <addr>` |
| Look up a transaction | `scope tx <hash>` |
| Auto-detect input and run everything | `scope insights <target>` |
| Get token DEX data (price, volume, holders) | `scope crawl <token>` |
| Token DEX + order book health (stablecoins) | `scope token-health <token> --with-market` |
| Live real-time dashboard for a token | `scope monitor <token>` |
| Exchange-style monitor (order book + trades) | `scope monitor <token> --layout exchange` |
| Browse trending/boosted tokens | `scope discover` |
| Check market peg/depth for a stablecoin | `scope market summary <symbol>` |
| Fetch OHLC/candlestick data from a venue | `scope market ohlc <symbol> --venue <venue> --interval 1h` |
| Fetch recent trades from a venue | `scope market trades <symbol> --venue <venue>` |
| List available exchange venues | `scope venues list` |
| Add a custom exchange venue | `scope venues schema` then drop YAML in `~/.config/scope/venues/` |
| Assess compliance risk for an address | `scope compliance risk <addr>` |
| Manage a portfolio of watched addresses | `scope portfolio add/remove/list/summary` |
| Export data to JSON/CSV | `scope export --address <addr> --output file.json` |
| Generate a report for multiple addresses | `scope report batch --addresses <...>` |

## Output Examples

Representative output for key commands:

### Venues list

```bash
> scope venues list
```

```text
┌─ Available Venues ──────────────────────────────
│  biconomy          order_book, ticker, trades, ohlc
│  binance           order_book, ticker, trades, ohlc
│  bitget            order_book, ticker, trades, ohlc
│  bybit             order_book, ticker, trades, ohlc
│  coinbase          order_book, ticker, trades, ohlc
│  crypto_com        order_book, ticker, trades, ohlc
│  gateio            order_book, ticker, trades, ohlc
│  htx               order_book, ticker, trades, ohlc
│  kraken            order_book, ticker, trades, ohlc
│  mexc              order_book, ticker, trades, ohlc
│  okx               order_book, ticker, trades, ohlc
├──────────────────────────────────────────────────
│  Loaded            11 venues (0 built-in, 11 user)
│  User dir          ~/.config/scope/venues
└──────────────────────────────────────────────────
```

### Market summary (text)

```bash
> scope market summary USDC
```

```text
┌─ USDC/USDT (binance) ───────────────────────────
│
├── Metrics
│  Venue             binance
│  Peg Target        1.0000
│  Best Bid          1.0003  (+0.030%)
│  Best Ask          1.0004  (+0.040%)
│  Mid Price         1.0004  (+0.035%)
│  Spread            0.0001  (0.010%)
│  Volume (24h)      767883091 USDT
│  Exec 10K buy      ~0.50 bps slippage
│  Exec 10K sell     ~0.50 bps slippage
│
├── Ask Side — 42 levels, 95630691 USDT (+58 outliers excl.)
│    1.0004  39143043.00 USDC  39158700.22 USDT
│    1.0005  19270846.00 USDC  19280481.42 USDT
│    1.0006  8915655.00 USDC  8921004.39 USDT
│    ... +39 more levels
│
├── Bid Side — 49 levels, 174252594 USDT (+51 outliers excl.)
│    1.0003  77790847.00 USDC  77814184.25 USDT
│    1.0002  31092292.00 USDC  31098510.46 USDT
│    1.0001  31875025.00 USDC  31878212.50 USDT
│    ... +46 more levels
│
├── Health Checks
│  ✓ No sells below peg
│  ✓ Bid/Ask ratio: 1.82x
│
│  HEALTHY
└──────────────────────────────────────────────────
```

### Market summary (JSON)

```bash
> scope market summary USDC --format json
```

```json
{
  "pair": "USDC/USDT",
  "venue": "binance",
  "peg_target": 1.0,
  "best_bid": 1.0003,
  "best_ask": 1.0004,
  "mid_price": 1.00035,
  "spread": 0.0001,
  "volume_24h": 767883090.5,
  "healthy": true,
  "checks": [
    {"status": "pass", "message": "No sells below peg"},
    {"status": "pass", "message": "Bid/Ask ratio: 1.82x"}
  ],
  "execution_10k_buy": {"fillable": true, "slippage_bps": 0.5},
  "execution_10k_sell": {"fillable": true, "slippage_bps": 0.5}
}
```

### Market OHLC

```bash
> scope market ohlc USDC --venue binance --interval 1h --limit 5
```

```text
OHLC — USDCUSDT (binance) interval=1h limit=5
──────────────────────────────────────────────────────────
           Open Time          Open         High          Low        Close         Volume
──────────────────────────────────────────────────────────
  2026-02-15 12:00    1.000400    1.000500    1.000300    1.000450    1250000.00
  2026-02-15 11:00    1.000350    1.000420    1.000280    1.000400    980000.00
  ...

  5 candles returned
```

### Market trades

```bash
> scope market trades USDC --venue binance --limit 5
```

```text
Recent Trades — USDCUSDT (binance)
──────────────────────────────────────
       Time  Side        Price           Qty
──────────────────────────────────────
  14:32:15   BUY    1.000350       5000.00
  14:32:14  SELL    1.000340       1200.00
  ...

  5 trades returned
```

### Token health

```bash
> scope token-health USDC
```

```text
Fetching token health data...

┌─ USDC   (USD Coin  ) ───────────────────────────
│
├── DEX Analytics
│  Price             $1.002300
│  24h Change        +0.00%
│  24h Volume        $846.04K
│  Liquidity         $810.64M
│  Market Cap        $76.31B
└──────────────────────────────────────────────────
```

### Discover (table)

```bash
> scope discover --limit 2
```

```text
Featured Token Profiles (2) — limit 2
--------------------------------------------------------------------------------
  1. solana | KeQnMAX5Ly...zJvMpump | -
     https://dexscreener.com/solana/keqnmax5lydfbrcwvqhojlu84bgfsg9ahthzjvmpump
  2. pulsechain | 0xE558edc9...0595aD11 | OLD GLORY RISE
     https://dexscreener.com/pulsechain/0xe558edc934fdbb65cdf4868617d5f0d80595ad11
```

### Discover (JSON)

```bash
> scope discover --format json --limit 1
```

```json
[
  {
    "chain": "solana",
    "address": "2CQfZ9wH3E4vCqKuGUkCzpZyXnhu7iwVbYc7ZAGfpump",
    "description": "Token description...",
    "url": "https://dexscreener.com/solana/2cqfz9wh3e4vcqkugukczpzyxnhu7iwvbyc7zagfpump"
  }
]
```

### Compliance risk

```bash
> scope compliance risk 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2
```

```text
Assessing risk for 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 on ethereum...

Risk Assessment Report
════════════════════════════════════════════════════════════
Address:             0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2
Chain:               ethereum
Risk Score:          1.9/10
Risk Level:          Low
Assessed At:         2026-02-13 13:40 UTC

Recommendations
────────────────────────────────────────────────────────────
1. Standard monitoring
```

## Exchange Venues

Scope uses **YAML descriptor files** to define CEX API integrations. This data-driven approach means adding a new exchange requires zero code changes — just a YAML file.

### Built-in Venues

| Venue | ID | Capabilities |
|-------|----|-------------|
| Binance Spot | `binance` | order book, ticker, trades |
| Biconomy | `biconomy` | order book, ticker, trades |
| Bitget | `bitget` | order book, ticker, trades |
| Bybit | `bybit` | order book, ticker, trades |
| Coinbase | `coinbase` | order book, ticker, trades |
| Crypto.com | `crypto_com` | order book, ticker, trades |
| Gate.io | `gateio` | order book, ticker, trades |
| HTX | `htx` | order book, ticker, trades |
| Kraken | `kraken` | order book, ticker, trades |
| MEXC | `mexc` | order book, ticker, trades |
| OKX | `okx` | order book, ticker, trades |

### Adding a Custom Venue

1. Run `scope venues schema` to see the YAML format
2. Create a YAML file following the schema
3. Place it in `~/.config/scope/venues/`
4. Validate with `scope venues validate my-venue.yaml`
5. Use it immediately: `scope market summary USDC --venue my-venue`

Or initialise the user directory with all built-in descriptors for reference:

```bash
scope venues init
# Copies all 11 built-in descriptors to ~/.config/scope/venues/
# Edit them to customise behaviour or use as templates
```

### Using Venues

Venues are specified with the `--venue` flag on `market summary`, `token-health`, and `monitor`:

```bash
scope market summary USDC --venue binance       # Binance (default)
scope market summary USDC --venue kraken         # Kraken
scope market summary USDC --venue okx            # OKX
scope health USDC --with-market --venue coinbase  # Coinbase order book
scope monitor USDC --layout exchange              # Exchange-style TUI
```

The DEX venues (`eth`, `solana`) are still available and use on-chain liquidity data rather than the venue registry.

## Compliance Features

Scope includes enterprise-grade compliance and risk analysis. All compliance commands live under `scope compliance`:

### Risk Assessment

See [Output Examples](#output-examples) for sample output.

```bash
# Basic risk score
scope compliance risk 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2

# Detailed breakdown with evidence
scope compliance risk 0xabc... --detailed --format markdown

# Export for compliance records (format auto-detected from extension: .json, .yaml, .md)
scope compliance risk 0xabc... --output risk-report.json
scope compliance risk 0xabc... --output risk-report.md
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
# Generate a compliance report for a specific jurisdiction (output: markdown)
scope compliance compliance-report 0xabc... --jurisdiction us --output report.md

# Detailed SAR report
scope compliance compliance-report 0xabc... --jurisdiction eu --report-type sar --output sar.md
```

Available jurisdictions: `us`, `eu`, `uk`, `switzerland`, `singapore`.
Report types: `summary` (default), `detailed`, `sar`, `travel-rule`.

Compliance output formats: `table` (default), `json`, `yaml`, `markdown`.

**Note**: Set `ETHERSCAN_API_KEY` environment variable for full compliance analysis. Without it, basic scoring is used.

## Token Crawling

Crawl DEX data for any token by address or name. Supports searching by symbol, displaying ASCII charts, and generating markdown reports. Token health output format is similar — see [Output Examples](#output-examples).

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

## Token Discovery

Browse trending and boosted tokens from DexScreener without knowing a symbol or address:

```bash
# Featured token profiles (default)
scope discover

# Recently boosted tokens
scope discover --source boosts

# Top boosted tokens (most active)
scope discover --source top-boosts

# Filter by chain
scope discover --chain ethereum --limit 10
scope disc --source boosts -c solana

# JSON or CSV output
scope discover --format json
scope discover --format csv --limit 5
```

Sources: `profiles` (featured), `boosts` (recent), `top-boosts` (most active). No API key required.

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

scope:solana> venues list
# List all available exchange venues

scope:solana> exit
```

Available interactive commands:

- `address` / `addr` -- Analyze a blockchain address
- `tx` / `transaction` -- Analyze a transaction
- `crawl` / `token` -- Crawl token analytics
- `monitor` / `mon` -- Live TUI dashboard with five layout presets, widget toggles, price/volume/candlestick charts, alerts, CSV export, and auto-pause
- `portfolio` / `port` -- Portfolio management (add, remove, list, summary)
- `tokens` / `aliases` -- Token alias management (add, remove, list, recent)
- `venues` / `ven` -- Venue management (list, schema, init, validate)
- `setup` / `config` -- Configuration commands
- `chain` -- Set or show current chain
- `format` -- Set or show output format (table, json, csv)
- `limit` -- Set or show transaction limit
- `+tokens` / `+txs` -- Toggle token/transaction display flags for address command
- `trace` / `decode` -- Toggle trace/decode flags for tx command
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
scope monitor USDC --layout exchange    # Exchange-style: order book + chart + trades

# Or from interactive mode
scope interactive
scope> monitor USDC
```

**Direct-command flags:** `--chain` / `-c`, `--layout` / `-l`, `--refresh` / `-r`, `--scale` / `-s`, `--color-scheme`, `--export` / `-e`, `--venue` / `-v`.

Use `--venue binance` (or any CEX venue) to fetch real exchange OHLC data instead of synthetic candles. The time period selector (1m, 5m, 15m, 1h, 4h, 1d) maps directly to the venue's candlestick intervals.

The monitor supports five layout presets that can be switched at runtime or configured in `config.yaml`:

| Preset | Description |
| --- | --- |
| **Dashboard** | Balanced 2x2 grid with all widgets visible (default) |
| **Chart** | Price chart takes ~85% of the screen; minimal stats overlay |
| **Feed** | Transaction/activity feed prioritized; small price ticker |
| **Compact** | Minimal single-column sparkline view for small terminals |
| **Exchange** | Exchange-style view: order book, chart, recent trades, market info |

The monitor automatically selects the best layout for your terminal size (responsive breakpoints). Manual layout switching disables auto-selection until you press `A`.

**Monitor features:**

- **Price/volume charts**: Line, Candlestick, and Volume Profile modes
- **Six timeframes**: 1m, 5m, 15m, 1h, 4h, 1d
- **Log/linear scale**: Toggle Y-axis scaling for wide price ranges
- **Color schemes**: Green/Red (default), Blue/Orange, Monochrome
- **Holder count**: On-chain holder count (when API key is configured)
- **Liquidity depth**: Per-pair liquidity breakdown across DEXes
- **Exchange view**: Live order book, ticker, and recent trade history from any configured CEX venue
- **Alerts**: Configurable price min/max, whale detection, and volume spike thresholds
- **CSV export**: Record live data to `./scope-exports/` with a single keypress
- **Auto-pause**: Optionally pause data fetching while interacting

**Monitor keybindings:**

| Key | Action |
| --- | --- |
| `Q` / `Esc` | Quit monitor |
| `R` | Force refresh |
| `P` / `Space` | Pause/resume |
| `Shift+P` | Toggle auto-pause on input |
| `E` | Toggle CSV export (REC indicator when active) |
| `L` | Cycle layout forward (Dashboard -> Chart -> Feed -> Compact -> Exchange) |
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
  format: table  # table, json, csv, or markdown
  color: true

portfolio:
  data_dir: "~/.config/scope/data"

# Monitor TUI configuration (all optional, shown with defaults)
monitor:
  layout: dashboard          # dashboard | chart-focus | feed | compact | exchange
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

### Free Tiers: Solana & BSC

You can use Scope with **Solana** and **BSC** without paying for APIs.

**BSC (Binance Smart Chain)**

- **RPC:** Default `bsc-dataseed.binance.org` is free. Scope automatically falls back to BSC RPC for **balance** when the block explorer returns "Free API access is not supported" (Etherscan V2 free tier limits).
- **BscScan API:** Etherscan V2 free tier does not support BSC (chainid=56). Balance works via RPC fallback; transactions and token lists require a paid API plan.
- **Setup:** No API key needed for balance. For transactions and tokens, a paid Etherscan plan is required. Set `bsc_rpc` in config to override the RPC endpoint.

**Solana**

- **RPC:** Default `api.mainnet-beta.solana.com` is free (public Solana endpoint, rate limited).
- **Solscan:** Scope uses the public RPC for balance, transactions, and tokens. A Solscan key is optional and not required for normal use.
- **Setup:** No API key needed for basic Solana support. Use the default RPC in config, or set `solana_rpc` to a custom endpoint if needed.

**Summary:** For BSC, balance works without any API key (RPC fallback). For full BSC support (transactions, tokens), a paid Etherscan plan is required. For Solana, the default public RPC is sufficient.

### Environment Variables

- `ETHERSCAN_API_KEY` - Required for compliance features
- `SCOPE_CONFIG` - Custom config file path (overrides default location)
- `RUST_LOG` - Log level override

## Architecture

Dataflow and C4 architecture diagrams are in [`docs/architecture/`](docs/architecture/README.md):

- **C4 Context** — Scope and external systems (Etherscan, DexScreener, RPCs, CEX APIs)
- **C4 Containers** — CLI, Chains, Compliance, Market, Display, Config
- **Dataflow** — Per-command diagrams for address, crawl, discover, compliance, market, token-health, portfolio, export, report, monitor, interactive

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
just summary   # USDC market summary (peg, spread, volume, execution, health checks)
```

**Git hooks**: A pre-push hook runs the coverage check (80% minimum, no regression) before allowing a push. Install with `just install-hooks`. Requires `cargo install cargo-tarpaulin`.

### Peg & Order Book Health (`scope market`)

The `scope market` command has three subcommands:

**`scope market summary`** — Fetches level-2 order book data from any CEX venue in the registry or DEX (Ethereum, Solana) and reports peg, book health, volume, and execution checks:

- **Peg**: Target, best bid/ask deviation (%), mid price, spread
- **Volume**: 24h quote volume (from venue ticker; omitted for DEX venues)
- **Execution**: Simulated 10k USDT buy/sell slippage or "insufficient liquidity"
- **Order book**: Ask/bid levels with depth (base and quote amounts)
- **Health checks**: No sells below peg, bid/ask ratio, minimum levels and depth per side
- **Output**: Text (default) or JSON — see [Output Examples](#output-examples) for sample
- **Tunable thresholds**: All health-check thresholds are configurable. Defaults (min-levels=6, min-depth=3000, peg-range=0.001, bid/ask ratio 0.2-5.0x) originated from the PUSD Hummingbot config—override for other markets.

```bash
scope market summary                           # USDC on Binance (default, one shot)
scope market summary PUSD                      # PUSD on default venue
scope market summary USDC --venue kraken       # USDC on Kraken
scope market summary USDC --venue okx          # USDC on OKX
scope market summary --peg 1.0 --min-depth 5000
scope market summary --peg-range 0.002 --min-bid-ask-ratio 0.1   # Loosen thresholds
scope market summary --format json             # Machine-readable output

# Repeated runs (default: every 60s for 1h)
scope market summary --every 30s --duration 10m   # Every 30s for 10 min
scope market summary --every 5m --duration 1h     # Every 5 min for 1 hour
scope market summary --duration 24h               # Every 60s for 24h (default interval)
scope market summary --every 1m                   # Every 1 min for 1h (default duration)
```

**`scope market ohlc`** — Fetches real OHLC/candlestick data from CEX venues:

```bash
scope market ohlc USDC --venue binance --interval 1h --limit 100
scope market ohlc BTC --venue kraken --interval 15m --format json
```

**`scope market trades`** — Fetches recent trades from CEX venues:

```bash
scope market trades USDC --venue binance --limit 50
scope market trades PUSD --venue biconomy --format json
```

`just summary` invokes `scope market summary` under the hood.

### Token Health Suite (`scope token-health` / `scope health`)

Combines DEX analytics (crawl) with optional market/order book summary for stablecoins:

```bash
scope token-health USDC                        # DEX liquidity, volume, holders (table)
scope health USDC --with-market                # + order book/peg (default: Binance)
scope token-health USDC --with-market --venue kraken
scope token-health USDC --format json
scope token-health USDC --ai                   # Markdown to console for agent parsing
```

With `--with-market`, order book data is fetched from the selected venue (`--venue`): any CEX venue from the registry (default **binance**), or **eth** / **solana** for DEX on-chain liquidity. Run `scope venues list` to see all available venues.

### Agent-Oriented Output (`--ai`)

Use `--ai` to emit markdown-formatted output to stdout instead of tables or JSON. Useful for LLM/agent consumption and parsing:

```bash
scope --ai address 0x742d35Cc...
scope --ai crawl USDC
scope --ai discover --limit 5
scope --ai portfolio list
scope --ai token-health USDC
```

`--ai` affects all commands that support output formatting (address, tx, crawl, discover, portfolio, export, token-health).

Additional market options:

- `--report path.md`: Save markdown report to file (one-shot or final report in repeat mode)
- `--csv path.csv`: Append time-series of peg/spread/depth to CSV (repeat mode only)

## Reporting & Analytics

Scope supports markdown and structured report generation across commands:

| Command | Report Option | Description |
| --------- | --------------- | -------------- |
| `scope address` | `--report report.md` | Address analysis (balance, transactions, tokens) |
| `scope address` | `--dossier` / `--dossier --report dossier.md` | Address + risk assessment combined |
| `scope crawl` | `--report report.md` | Token analytics with risk scoring |
| `scope portfolio summary` | `--report report.md` | Portfolio allocations by chain and address |
| `scope market summary` | `--report report.md` | Peg and order book health |
| `scope market summary` | `--csv data.csv` | Time-series of peg/spread (repeat mode) |
| `scope compliance risk` | `--output file` | Risk assessment (format from extension: .json, .yaml, .md) |
| `scope compliance compliance-report` | `--output file` | Unified risk + pattern analysis (markdown) |
| `scope report batch` | `--addresses a,b,c` or `--from-file path` + `--output report.md` | Combined report for multiple addresses |
| `scope report batch` | `--with-risk` | Include risk assessment per address (uses ETHERSCAN_API_KEY) |
| `scope token-health` | `--with-market --venue <venue>` | DEX + optional market composite |
| `scope` | `--ai` (global) | Markdown console output for agent parsing |

All reports include version and timestamp metadata for audit trail.

## Supported Chains

| Chain | Type | Address Format | Explorer |
| -------- | ------- | -------------- | -------------------- |
| Ethereum | EVM | 0x... | Etherscan |
| Polygon | EVM | 0x... | Polygonscan |
| Arbitrum | EVM | 0x... | Arbiscan |
| Optimism | EVM | 0x... | Optimistic Etherscan |
| Base | EVM | 0x... | Basescan |
| BSC | EVM | 0x... | BscScan |
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
