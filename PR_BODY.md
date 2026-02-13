# Release v0.3.1 — Discover & Documentation

## Summary

Adds token discovery command and updates documentation. Version 0.3.0 is already on crates.io; this release publishes discover and doc updates.

**Note:** v0.3.0 (market, token-health, reporting, --ai) was published 2026-02-09. This PR adds discover and doc refresh for v0.3.1.

## New Features (v0.3.1)

### Token Discovery (`scope discover` / `scope disc`)
- Browse trending and boosted tokens from DexScreener without knowing a symbol or address
- Sources: `profiles` (featured), `boosts` (recent), `top-boosts` (most active)
- `--chain` filter, `--limit`, output: table, json, csv
- No API key required

### Other Features (from v0.3.0, already on crates.io)

### Market Command (`scope market summary`)
- Peg and order book health for stablecoin markets
- Fetches level-2 depth from Binance, Biconomy, or DEX liquidity
- Configurable health checks, repeated runs (`--every`, `--duration`)
- `--report` and `--csv` for time-series export

### Token Health Suite (`scope token-health` / `scope health`)
- Composite DEX + market command
- DEX analytics with optional order book summary
- Venues: binance, biconomy, eth, solana

### Agent Output (`scope --ai`)
- Markdown to stdout for agent/LLM consumption
- Affects: address, tx, crawl, discover, portfolio, export, token-health

### Reporting & Analytics
- `scope address --report`, `--dossier`
- `scope market summary --report`, `--csv`
- `scope portfolio summary --report`
- `scope compliance risk --output`, `scope compliance compliance-report`
- `scope report batch` (--addresses, --from-file, --with-risk)
- All reports include version and timestamp footer

## Documentation

- Dataflow diagrams for discover, token-health, and all commands
- C4 context/containers updated
- Feature plan success criteria updated
- Release checklist added (`docs/RELEASE.md`)

## Version

- **0.3.1** (bump from 0.3.0)
- CHANGELOG updated for release

## Checklist

- [x] `cargo test` passes
- [x] `cargo clippy -- -D warnings` passes
- [x] CHANGELOG.md updated
- [x] Version bumped in Cargo.toml
- [ ] CI passes on merge
- [ ] Tag `v0.3.1` after merge
- [ ] `cargo publish --dry-run` then `cargo publish` for crates.io
