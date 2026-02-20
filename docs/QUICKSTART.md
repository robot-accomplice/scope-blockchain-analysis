# Scope Quickstart Guide

Get the most out of Scope in minutes. This guide walks through common workflows and tips.

## First Run: One-Time Setup

On first run, Scope will prompt you to configure API keys and preferences. For basic use, you can skip most keys—only **Etherscan** is required for compliance features.

```bash
scope setup                    # Run the wizard
scope setup --status           # Check what's configured
```

**Minimum for most workflows:** No keys needed for address lookup, token crawl, discover, or monitor. Add `ETHERSCAN_API_KEY` for risk assessment and pattern detection.

### Shell Tab-Completion

Set up tab-completion for your shell so you can complete commands and flags with `<Tab>`:

```bash
# Bash
scope completions bash > ~/.local/share/bash-completion/completions/scope

# Zsh (add to fpath, then reload)
scope completions zsh > ~/.zfunc/_scope
# Add to .zshrc: fpath=(~/.zfunc $fpath); autoload -Uz compinit && compinit

# Fish
scope completions fish > ~/.config/fish/completions/scope.fish
```

---

## Workflow 1: Due Diligence on an Address

**Goal:** Quickly assess a wallet—balance, activity, and risk.

```bash
# Basic balance (auto-detects chain from address format)
scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2

# Full picture: transactions + token holdings
scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --include-txs --include-tokens

# Wallet dossier: address + risk assessment in one command
scope address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2 --dossier

# Save to file for records
scope address 0x742d... --dossier --report due-diligence.md
```

**Pro tip:** Address format determines chain. `0x...` → EVM, `DRpb...` → Solana, `T...` → Tron. Use `--chain` to override.

---

## Workflow 2: Token Research

**Goal:** Deep-dive into a token—liquidity, holders, risk score.

```bash
# Search by symbol (interactive if multiple matches)
scope crawl USDC

# By address
scope crawl 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --chain ethereum

# Quick health check: DEX + optional order book
scope token-health USDC
scope health USDC --with-market --venue binance

# Full report with risk scoring
scope crawl PEPE --report token-report.md --period 7d

# JSON for scripts or piping
scope crawl USDC --no-charts --format json --yes
```

**Pro tip:** `--yes` skips interactive prompts. `scope discover` surfaces trending tokens without knowing a symbol.

---

## Workflow 3: Compliance & Risk

**Goal:** Risk score an address, detect patterns, generate reports.

```bash
# Risk score (requires ETHERSCAN_API_KEY)
scope compliance risk 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2

# Detailed breakdown with evidence
scope compliance risk 0xabc... --detailed --format markdown

# Pattern detection: structuring, layering, velocity
scope compliance analyze 0xabc... --patterns structuring,layering --range 6m

# Trace fund flow
scope compliance trace 0xtxhash... --depth 5 --flag-suspicious

# Jurisdiction-specific report
scope compliance compliance-report 0xabc... --jurisdiction us --output report.md
scope compliance compliance-report 0xabc... --jurisdiction eu --report-type sar --output sar.md
```

**Pro tip:** Export formats auto-detect from extension: `--output risk.json`, `--output risk.yaml`, `--output risk.md`.

---

## Workflow 3b: Smart Contract Audit

**Goal:** Security-assess a smart contract—vulnerabilities, proxy patterns, access control, DeFi risks.

```bash
# Analyze a verified contract (Tether USDT)
scope contract 0xdAC17F958D2ee523a2206206994597C13D831ec7

# Short alias with chain override
scope ct 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --chain polygon

# JSON output for scripts or piping to jq
scope contract 0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D --json

# From interactive mode
scope interactive
scope> contract 0xdAC17F958D2ee523a2206206994597C13D831ec7
scope> ct 0xA0b86991... --chain=polygon --json
```

**What you get:** Security score (0-100), source code verification, proxy detection (EIP-1967/1822/1167/Diamond), access control mapping, vulnerability heuristics, DeFi protocol analysis (oracles, flash loans, DEX slippage), and external intelligence (audit reports, GitHub links, Sourcify).

**Pro tip:** The web UI (`scope web`) has a dedicated Contract panel with rich visual rendering of all findings.

---

## Workflow 4: Live Monitoring

**Goal:** Watch a token in real time with charts, alerts, and export.

```bash
# Launch monitor by symbol
scope monitor USDC
scope mon PEPE --chain ethereum --layout chart-focus

# From interactive mode (preserves context)
scope interactive
scope> chain ethereum
scope> monitor USDC
```

**Keybindings:** `Q` quit, `R` refresh, `P` pause, `E` toggle CSV export, `L` cycle layout, `C` chart mode (line/candlestick/volume).

---

## Workflow 5: Portfolio & Batch Reporting

**Goal:** Track multiple addresses and generate combined reports.

```bash
# Add addresses to portfolio
scope portfolio add 0x742d... --label "Treasury" --tags main,eth
scope portfolio add DRpb... --chain solana --label "Solana Wallet"

# Summary across chains
scope portfolio summary
scope portfolio summary --chain ethereum --tag main

# Batch report for multiple addresses
scope report batch --addresses 0x742d...,0xabc... --output batch-report.md

# Include risk assessment per address
scope report batch --addresses 0x742d...,0xabc... --output batch-report.md --with-risk

# From file
scope report batch --from-file addresses.txt --output batch-report.md
```

---

## Workflow 6: Market & Peg Health

**Goal:** Check stablecoin peg and order book health.

```bash
# One-shot summary (default: Binance)
scope market summary USDC
scope market summary DAI --venue binance

# Repeat mode for monitoring
scope market summary USDC --every 30s --duration 10m

# JSON for automation
scope market summary USDC --format json

# Report to file
scope market summary USDC --report peg-report.md
```

**Venues:** `binance`, `biconomy`, `eth` (Ethereum DEX), `solana` (Solana DEX).

---

## Workflow 7: Interactive Mode (Power Users)

**Goal:** Chain commands with preserved context—no retyping addresses or chain.

```bash
scope interactive
```

```text
scope:ethereum> address 0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2
# ... output ...

scope:ethereum> format json
scope:ethereum> address
# Reuses last address

scope:ethereum> chain solana
scope:solana> crawl USDC
# Uses solana automatically

scope:solana> monitor USDC
# Launches TUI; exit with Q to return to prompt

scope:solana> portfolio summary --report port.md
scope:solana> exit
```

---

## Quick Reference: Command Aliases

| Full | Alias |
|------|-------|
| `address` | `addr` |
| `transaction` | `tx` |
| `contract` | `ct` |
| `insights` | `insight` |
| `crawl` | `token` |
| `portfolio` | `port` |
| `monitor` | `mon` |
| `token-health` | `health` |
| `discover` | `disc` |
| `interactive` | `shell` |
| `setup` | `config` |

---

## Command Map

Not sure which command to use? Here's a quick decision tree:

| I want to... | Command |
|--------------|---------|
| Look up an address (balance, txs, tokens) | `scope address <addr>` |
| Look up a transaction | `scope tx <hash>` |
| Audit a smart contract (security, proxy, vulns) | `scope contract <addr>` |
| Auto-detect input and run everything | `scope insights <target>` |
| Get token DEX data (price, volume, holders) | `scope crawl <token>` |
| Token DEX + order book health (stablecoins) | `scope token-health <token> --with-market` |
| Live real-time dashboard for a token | `scope monitor <token>` |
| Browse trending/boosted tokens | `scope discover` |
| Check market peg/depth for a stablecoin | `scope market summary <symbol>` |
| Assess compliance risk for an address | `scope compliance risk <addr>` |
| Manage a portfolio of watched addresses | `scope portfolio add/remove/list/summary` |
| Export data to JSON/CSV | `scope export --address <addr> --output file.json` |
| Generate a report for multiple addresses | `scope report batch --addresses <...>` |

---

## Output Formats

Scope supports multiple output formats:

| Flag | Format | Use case |
|------|--------|----------|
| *(default)* | Table | Human-readable terminal output |
| `--format json` | JSON | Programmatic consumption, piping to `jq` |
| `--format csv` | CSV | Spreadsheet import |
| `--ai` | Markdown | LLM/agent parsing (global flag, all commands) |

Use `--format json` when piping output to other tools or scripts:

```bash
scope address 0x742d... --format json | jq '.balance'
```

Use `--ai` when feeding output to an LLM or agent:

```bash
scope --ai address 0x742d...
scope --ai portfolio list
```

---

## Error Hints

When something goes wrong, Scope provides remediation hints for common errors:

```text
Error: Invalid address format: 0x123

Hint: Ensure the address format matches the target chain.
      EVM: 0x followed by 40 hex characters
      Solana: base58 encoded public key
      Tron: T followed by base58 characters
```

For config issues: `scope setup` will repair missing configuration.  
For network issues: use `-v` for more details on failing requests.

---

## Progress Indicators

Long-running operations show spinners or progress bars. These automatically hide when output is piped:

```bash
# Terminal: shows animated spinner
scope address 0x742d... --include-txs

# Pipe: no spinner, clean output
scope address 0x742d... --format json | jq '.'
```

---

## Next Steps

- **Configuration:** `scope setup --status` and `~/.config/scope/config.yaml`
- **Shell completion:** `scope completions zsh > ~/.zfunc/_scope`
- **Architecture:** [docs/architecture/](architecture/README.md) for dataflow diagrams
- **Full reference:** `scope <command> --help` for subcommand options with examples
- **Usability guidelines:** [docs/architecture/cli-usability-guidelines.md](architecture/cli-usability-guidelines.md)
