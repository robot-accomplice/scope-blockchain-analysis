# Scope Quickstart Guide

Get the most out of Scope in minutes. This guide walks through common workflows and tips.

## First Run: One-Time Setup

On first run, Scope will prompt you to configure API keys and preferences. For basic use, you can skip most keys—only **Etherscan** is required for compliance features.

```bash
scope setup                    # Run the wizard
scope setup --status           # Check what's configured
```

**Minimum for most workflows:** No keys needed for address lookup, token crawl, discover, or monitor. Add `ETHERSCAN_API_KEY` for risk assessment and pattern detection.

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
scope health USDC --with-market --market-venue binance

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
scope market summary PUSD --market-venue biconomy

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
| `crawl` | `token` |
| `portfolio` | `port` |
| `monitor` | `mon` |
| `token-health` | `health` |
| `discover` | `disc` |
| `interactive` | `shell` |

---

## Output Formats

Most commands support `--format table|json|csv`. Use `--ai` for markdown to stdout (agent-friendly):

```bash
scope --ai address 0x742d...
scope --ai portfolio list
```

---

## Next Steps

- **Configuration:** `scope setup --status` and `~/.config/scope/config.yaml`
- **Architecture:** [docs/architecture/](architecture/README.md) for dataflow diagrams
- **Full reference:** `scope <command> --help` for subcommand options
