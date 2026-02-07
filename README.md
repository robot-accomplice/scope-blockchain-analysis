# BCC - Blockchain Crawler CLI

```
  ██████╗  ██████╗ ██████╗             ______ 
  ██╔══██╗██╔════╝██╔════╝    /^|^|^\_/_o__o_\_/^|^|^\
  ██████╔╝██║     ██║         \ \ \ \ \_v__v_/ / / / /
  ██╔══██╗██║     ██║
  ██████╔╝╚██████╗╚██████╗    Blockchain Crawler CLI
  ╚═════╝  ╚═════╝ ╚═════╝
```

A production-grade command-line tool for blockchain data analysis, portfolio tracking, and transaction investigation.

## Features

- **Address Analysis**: Query balances, transaction history, and token holdings for blockchain addresses
- **Transaction Analysis**: Decode and trace blockchain transactions, including internal calls
- **Portfolio Management**: Track multiple addresses across chains with labels and tags
- **Data Export**: Export analysis results in JSON, CSV, or formatted table output
- **Interactive Mode**: REPL with preserved context between commands for faster workflow
- **Multi-Chain Support**: 
  - EVM chains: Ethereum, Polygon, Arbitrum, Optimism, Base, BSC, Aegis
  - Non-EVM chains: Solana, Tron

## Installation

```bash
# Clone the repository
git clone https://github.com/yourusername/bcc.git
cd bcc

# Build and install
cargo install --path .
```

## Quick Start

```bash
# Analyze an Ethereum address
bcc address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2

# Analyze an address on other EVM chains
bcc address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --chain polygon
bcc address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --chain bsc

# Analyze a Solana address
bcc address DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy --chain solana

# Analyze a Tron address
bcc address TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf --chain tron

# Analyze a transaction
bcc tx 0xabc123def456789012345678901234567890123456789012345678901234abcd

# Add an address to your portfolio
bcc portfolio add 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --label "Main Wallet"

# List portfolio addresses
bcc portfolio list

# Export data
bcc export --address 0x742d35... --output history.json

# Launch interactive mode
bcc interactive
```

## Interactive Mode

Launch a REPL where context (chain, format, flags) persists between commands:

```bash
$ bcc interactive

  ██████╗  ██████╗ ██████╗              ______ 
  ██╔══██╗██╔════╝██╔════╝    /^|^|^\_/_o__o_\_/^|^|^\
  ██████╔╝██║     ██║        \ \ \ \ \_v__v_/ / / / /
  ██╔══██╗██║     ██║
  ██████╔╝╚██████╗╚██████╗    Blockchain Crawler CLI
  ╚═════╝  ╚═════╝ ╚═════╝

Welcome to BCC Interactive Mode!
Type 'help' for available commands, 'exit' to quit.

bcc:ethereum> chain solana
Chain set to: solana

bcc:solana> address 7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU
Address Analysis Report
=======================
Chain:        Solana
Address:      7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU
Balance:      1.5 SOL
...

bcc:solana> tx 5VERv8NMvzbJMEkV8xnrLkEaWRtSz9CosKDYjCJjBRnbJLgp8uirBgmQpjKhoR4tjF3ZpRzrFmBV6UjKdiSZkQUW
# Uses Solana context automatically

bcc:solana> format json
Format set to: json

bcc:solana> tokens
Include tokens: on

bcc:solana> address
# Re-runs last address with new settings

bcc:solana> exit
Goodbye!
```

### Interactive Commands


| Command        | Description                        |
| -------------- | ---------------------------------- |
| `chain [name]` | Set or show current chain          |
| `format [fmt]` | Set output format (table/json/csv) |
| `ctx`          | Show current session context       |
| `clear`        | Reset context to defaults          |
| `tokens`       | Toggle include_tokens flag         |
| `txs`          | Toggle include_txs flag            |
| `trace`        | Toggle trace flag                  |
| `decode`       | Toggle decode flag                 |
| `limit [n]`    | Set transaction limit              |
| `help`         | Show available commands            |
| `exit`         | Exit interactive mode              |


Session context is automatically saved and restored between sessions.

## Commands

### Address Analysis

Analyze a blockchain address to view balances and transaction history.

```bash
bcc address <ADDRESS> [OPTIONS]

Options:
  -c, --chain <CHAIN>     Target blockchain (default: ethereum)
  -f, --format <FORMAT>   Output format: table, json, csv
  --include-txs           Include transaction history
  --include-tokens        Include token balances
  --limit <N>             Max transactions to retrieve (default: 100)
```

### Transaction Analysis

Analyze a specific transaction.

```bash
bcc tx <HASH> [OPTIONS]

Options:
  -c, --chain <CHAIN>     Target blockchain (default: ethereum)
  -f, --format <FORMAT>   Output format: table, json, csv
  --trace                 Include internal transactions
  --decode                Decode input data
```

### Portfolio Management

Track multiple addresses across chains.

```bash
# Add an address
bcc portfolio add <ADDRESS> [OPTIONS]
  -l, --label <LABEL>     Human-readable label
  -c, --chain <CHAIN>     Blockchain network
  -t, --tags <TAGS>       Comma-separated tags

# List addresses
bcc portfolio list

# Remove an address
bcc portfolio remove <ADDRESS>

# View portfolio summary
bcc portfolio summary
```

### Data Export

Export analysis data to files.

```bash
bcc export [OPTIONS]

Options:
  -a, --address <ADDR>    Export data for an address
  -p, --portfolio         Export portfolio data
  -o, --output <PATH>     Output file path (required)
  -f, --format <FORMAT>   Output format (auto-detected from extension)
```

### Interactive Mode

Launch an interactive REPL session.

```bash
bcc interactive [OPTIONS]

Options:
  --no-banner             Skip the startup banner

Alias: bcc shell
```

## Configuration

BCC reads configuration from `~/.config/bcc/config.yaml`:

```yaml
chains:
  # EVM-compatible chains
  ethereum_rpc: "https://mainnet.infura.io/v3/YOUR_KEY"
  bsc_rpc: "https://bsc-dataseed.binance.org"
  aegis_rpc: "http://localhost:8545"  # Aegis/Wraith blockchain

  # Non-EVM chains
  solana_rpc: "https://api.mainnet-beta.solana.com"
  tron_api: "https://api.trongrid.io"

  # API keys for block explorers
  api_keys:
    etherscan: "YOUR_ETHERSCAN_KEY"
    polygonscan: "YOUR_POLYGONSCAN_KEY"
    arbiscan: "YOUR_ARBISCAN_KEY"
    bscscan: "YOUR_BSCSCAN_KEY"
    solscan: "YOUR_SOLSCAN_KEY"
    tronscan: "YOUR_TRONSCAN_KEY"

output:
  format: table  # table, json, csv
  color: true

portfolio:
  data_dir: "~/.local/share/bcc"
```

### Environment Variables

- `BCC_CONFIG`: Path to configuration file
- `RUST_LOG`: Override log level (e.g., `bcc=debug`)

## Library Usage

BCC can be used as a library in your Rust applications:

### Ethereum/EVM Chains

```rust
use bca::{Config, chains::EthereumClient};

#[tokio::main]
async fn main() -> bca::Result<()> {
    let config = Config::load(None)?;
    
    // Ethereum mainnet
    let client = EthereumClient::new(&config.chains)?;
    let balance = client.get_balance("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2").await?;
    println!("ETH Balance: {}", balance.formatted);
    
    // Other EVM chains
    let bsc_client = EthereumClient::for_chain("bsc", &config.chains)?;
    let aegis_client = EthereumClient::for_chain("aegis", &config.chains)?;
    
    Ok(())
}
```

### Solana

```rust
use bca::{Config, chains::SolanaClient};

#[tokio::main]
async fn main() -> bca::Result<()> {
    let config = Config::load(None)?;
    let client = SolanaClient::new(&config.chains)?;
    
    let balance = client.get_balance("DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy").await?;
    println!("SOL Balance: {}", balance.formatted);
    
    Ok(())
}
```

### Tron

```rust
use bca::{Config, chains::TronClient};

#[tokio::main]
async fn main() -> bca::Result<()> {
    let config = Config::load(None)?;
    let client = TronClient::new(&config.chains)?;
    
    let balance = client.get_balance("TDqSquXBgUCLYvYC4XZgrprLK589dkhSCf").await?;
    println!("TRX Balance: {}", balance.formatted);
    
    Ok(())
}
```

## Development

BCC uses [just](https://github.com/casey/just) as a task runner. Run `just --list` to see all available commands.

```bash
# Run all tests with nextest
just test

# Run full CI workflow locally
just ci-test

# Format code
just format

# Run lints
just lint

# Build release
just build-release

# Run with coverage
just coverage

# Run security audit
just audit
```

### Manual Commands

```bash
# Run tests
cargo test

# Run with coverage
cargo tarpaulin --out Html

# Build release
cargo build --release

# Run lints
cargo clippy -- -D warnings

# Format code
cargo fmt
```

## Supported Chains

### EVM-Compatible Chains


| Chain    | Explorer API         | Native Token | Address Format |
| -------- | -------------------- | ------------ | -------------- |
| Ethereum | Etherscan            | ETH          | 0x...          |
| Polygon  | Polygonscan          | MATIC        | 0x...          |
| Arbitrum | Arbiscan             | ETH          | 0x...          |
| Optimism | Optimistic Etherscan | ETH          | 0x...          |
| Base     | Basescan             | ETH          | 0x...          |
| BSC      | BscScan              | BNB          | 0x...          |
| Aegis    | JSON-RPC (direct)    | WRAITH       | 0x...          |


### Non-EVM Chains


| Chain  | API             | Native Token | Address Format       |
| ------ | --------------- | ------------ | -------------------- |
| Solana | Solana JSON-RPC | SOL          | Base58 (32-44 chars) |
| Tron   | TronGrid API    | TRX          | T... (34 chars)      |


## CI/CD

BCC includes a GitHub Actions workflow (`.github/workflows/ci.yml`) that runs:

1. **Check** - Fast compilation check
2. **Format** - Code formatting verification
3. **Lint** - Clippy with warnings as errors
4. **Test** - Unit and integration tests with nextest
5. **Docs** - Documentation build verification
6. **Coverage** - Code coverage reporting (main branch only)
7. **Build** - Release binary build
8. **Security** - Dependency vulnerability audit

## License

MIT License - see [LICENSE](LICENSE) for details.