# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Market command** (`scope market summary`): Peg and order book health for stablecoin markets
  - Fetches level-2 depth from exchange APIs (Biconomy)
  - Configurable health checks: peg safety, bid/ask ratio, min levels, min depth
  - All thresholds configurable via CLI (--peg-range, --min-bid-ask-ratio, --max-bid-ask-ratio, etc.)
  - Optional repeated runs (--every, --duration)
  - Text and JSON output; chain/venue display
- **Reporting and analytics**:
  - `scope address --report report.md`: Markdown report for address analysis
  - `scope market summary --report path.md`: Save peg/orderbook report to file
  - `scope market summary --csv path.csv`: Time-series CSV of peg/spread/depth (repeat mode)
  - `scope portfolio summary --report report.md`: Portfolio markdown report (allocations, addresses)
  - `scope compliance risk --output file`: Export respects format (.json, .yaml, .md)
  - `scope compliance compliance-report`: Unified compliance report (risk + pattern analysis) for addresses
  - `scope report batch`: Batch report for multiple addresses (--addresses, --from-file)
  - Report versioning: All reports include Scope version and timestamp footer

## [0.2.2] - 2026-02-08

## [0.2.1] - 2026-02-08

## [0.2.0] - 2026-02-08

## [0.2.0] - 2026-02-08

### Added
- Initial release of Scope Blockchain Analysis
- Multi-chain support: Ethereum, Polygon, Arbitrum, Optimism, Base, BSC, Aegis, Solana, Tron
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

[Unreleased]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/releases/tag/v0.1.0
[0.2.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.1.0...v0.2.0
[0.2.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.2.0...v0.2.0
[0.2.1]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.2.0...v0.2.1
[0.2.2]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.2.1...v0.2.2
