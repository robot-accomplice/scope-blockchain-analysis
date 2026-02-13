# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] - 2026-02-13

### Added
- **Shell completion** (`scope completions bash|zsh|fish`): Generate tab-completion scripts for bash, zsh, and fish shells via `clap_complete`
- **Progress indicators**: Spinners and step-progress bars for all long-running operations (address, tx, crawl, compliance risk, insights, discover, export, token-health, report batch) via `indicatif`
- **Error remediation hints**: Actionable suggestions for common errors (invalid address/hash, missing config, network failures, API auth issues)
- **Help with examples**: `after_help` example invocations in top-level help and subcommands (address, tx, crawl)
- **Command map**: Decision tree in README and QUICKSTART showing which command to use for each task
- **Output Formats section**: Documentation of `--format json` vs `--ai` usage in README and QUICKSTART

### Changed
- **Command grouping**: Commands in `--help` now ordered by task category (entity lookup → token analysis → compliance → data/export → config) instead of alphabetically
- **Documentation link**: Top-level `--help` now shows GitHub repository URL and quickstart guide path
- **Typo suggestions**: Verified clap built-in fuzzy matching ("Did you mean: ...") for misspelled subcommands
- **Setup hints expanded**: Post-setup wizard now suggests `insights`, `monitor`, and `completions` commands
- **Web module feature-gated**: `scope web` / `scope serve` now requires `--features web` to compile (resolves pre-existing build failures)

### Fixed
- **Docstring consistency**: Replaced all `bca` references with `scope` in module docs (crawl, tx, address, setup, portfolio, export) and runtime strings (config file comments, empty portfolio messages)

## [0.3.1] - 2026-02-13

### Added
- **Discover command** (`scope discover` / `scope disc`): Browse trending and boosted tokens from DexScreener
  - `--source profiles|boosts|top-boosts`: Featured profiles, recently boosted, or top boosted tokens
  - `--chain`: Filter by chain (e.g., ethereum, solana)
  - `--limit`: Max tokens to show (default 15)
  - Output: table, json, csv
  - No API key required

### Changed
- Documentation and architecture diagrams updated for all commands
- C4 context and container diagrams include discover, token-health
- Release checklist added (`docs/RELEASE.md`)

## [0.3.0] - 2026-02-09

### Added
- **Market command** (`scope market summary`): Peg and order book health for stablecoin markets
  - Venues: Binance, Biconomy (CEX); Ethereum DEX, Solana DEX
  - Repeat mode: `--every` and `--duration` for periodic snapshots
  - `--report` and `--csv` for time-series export
- **Token Health Suite** (`scope token-health` / `scope health`): DEX analytics + optional order book
  - `--with-market` and `--market-venue` (binance, biconomy, eth, solana)
- **Agent output** (`scope --ai`): Markdown to stdout for agent/LLM parsing
  - Affects: address, tx, crawl, discover, portfolio, export, token-health
- **Reporting**: address --report/--dossier, market --report/--csv, portfolio summary --report, compliance risk --output, compliance compliance-report, report batch (--addresses, --from-file, --with-risk)
- Report versioning footer on all reports

## [0.2.2] - 2026-02-08

## [0.2.1] - 2026-02-08

## [0.2.0] - 2026-02-08

## [0.2.0] - 2026-02-08

### Added
- Initial release of Scope Blockchain Analysis
- Multi-chain support: Ethereum, Polygon, Arbitrum, Optimism, Base, BSC, Solana, Tron
- Address analysis with USD valuation via DexScreener
- Transaction decoding and tracing
- Token crawling with risk reports
- Live monitoring TUI with real-time charts
- Portfolio management across chains
- Interactive REPL mode with context persistence
- Data export (JSON, CSV)
- **Compliance module** with risk assessment, pattern detection, and transaction tracing
- GitHub Actions CI/CD workflow
- Comprehensive test suite (260+ tests)

### Compliance Features
- Risk scoring engine with weighted factors
- Behavioral pattern analysis (velocity, structuring, round numbers)
- Transaction association analysis
- Source of funds tracking
- Etherscan API integration for real data
- Pattern detection: structuring, layering, integration, velocity anomalies
- Transaction taint tracing (multi-hop)
- Markdown and JSON report generation

## [0.1.0] - 2026-02-08

### Added
- Initial public release
- Core blockchain analysis functionality
- CLI with subcommands: address, tx, crawl, monitor, portfolio, export, interactive
- Library API for programmatic usage
- Configuration system with YAML support
- Support for ERC-20, SPL, and TRC-20 tokens

[Unreleased]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.3.1...v1.0.0
[0.3.1]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.2.2...v0.3.0
[0.1.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/releases/tag/v0.1.0
[0.2.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.1.0...v0.2.0
[0.2.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.2.0...v0.2.0
[0.2.1]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.2.0...v0.2.1
[0.2.2]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.2.1...v0.2.2
[0.3.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.2.2...v0.3.0
