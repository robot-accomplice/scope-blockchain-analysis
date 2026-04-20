# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.5] - 2026-04-20

### Fixed
- **Release pipeline**: Upgraded `softprops/action-gh-release` from v1 to v2 and added `make_latest: true` so tagged releases publish assets directly instead of failing with "Cannot upload assets to an immutable release." This unblocks the automated GitHub Release + asset flow that was silently broken for v0.5.1 through v0.5.4.
- **Linux arm64 cross-compile**: Switched `reqwest` from native-tls (OpenSSL) to `rustls-tls`, eliminating the `openssl-sys` cross-compilation failure that was blocking the `aarch64-unknown-linux-gnu` release binary. All four release targets (linux-x64, linux-arm64, macos-x64, macos-arm64) now build cleanly in a single workflow run. Also retains `gzip` and `brotli` response decompression that were in reqwest's default feature set.
- **Clippy 1.95 compliance**: `chains::analyze_gas_usage` now uses `checked_div().unwrap_or(0)` for divide-by-zero guards and `sort_by_key(|g| Reverse(g.total_gas))` for descending sorts, satisfying the new `manual_checked_ops` and `unnecessary_sort_by` lints.
- **Security advisories**: Bumped `rustls-webpki` to 0.103.12 via `cargo update` to address RUSTSEC-2026-0098 and RUSTSEC-2026-0099.

### Changed
- **CI actions modernized**: Bumped `actions/checkout` v4→v6, `actions/upload-artifact` v4→v7, `actions/download-artifact` v4→v8, and `codecov/codecov-action` v4→v6 to run on Node.js 24 ahead of the June 2026 platform-wide deprecation of Node.js 20 actions.

## [0.5.4] - 2026-02-20

### Fixed
- **Market pair parsing robustness**: `scope market` commands now normalize `key=value` style pair inputs (for example `pair_symbol=USDT_PUSD`) and correctly parse explicit base/quote pairs across `_`, `/`, and `-` delimiters before applying venue-specific symbol formatting.
- **Binance invalid symbol diagnostics**: Non-2xx API errors now include response body previews, and Binance `-1121` invalid symbol responses include a targeted hint about pair ordering, venue format expectations, and market availability.

## [0.5.3] - 2026-02-20

### Added
- **`ghola.buffer_size` config option**: Configurable read buffer size (default 4096 bytes) for the Ghola sidecar's `fasthttp.Client.ReadBufferSize`. Increase for APIs returning large response headers.
- **Buffer size in status display**: `scope setup --status` now shows the configured buffer size when Ghola transport is enabled.

## [0.5.2] - 2026-02-20

### Added
- **Ghola sidecar integration**: New `src/http/` module introduces a pluggable HTTP transport abstraction (`HttpClient` trait) that transparently routes requests through the [Ghola](https://github.com/robot-accomplice/ghola) stealth proxy when available, with graceful fallback to native `reqwest` when absent.
- **`NativeHttpClient`**: Default `reqwest`-based transport implementation with 30-second timeout.
- **`GholaHttpClient`**: Sidecar bridge client with support for temporal drift, ghost signing, health checks, and automatic sidecar spawning.
- **`GholaConfig`**: New configuration section (`ghola.enabled`, `ghola.stealth`) with serde support and defaults.
- **Setup status**: `scope setup --status` now displays Ghola sidecar availability, transport mode, and stealth status with install instructions when the binary is absent.
- **Integration documentation**: `docs/GHOLA_INTEGRATION.md` with architecture diagram, key files, configuration reference, verification steps, and troubleshooting guide.

### Changed
- **Chain client architecture**: All chain clients (`EthereumClient`, `SolanaClient`, `TronClient`, `DexClient`) refactored from direct `reqwest::Client` usage to `Arc<dyn HttpClient>` via dependency injection. Each now has `_with_http` constructor variants.
- **`DefaultClientFactory`**: Now carries and propagates a shared `Arc<dyn HttpClient>` to all created chain clients, enabling consistent transport selection across the application.
- **Runtime transport selection**: `main.rs` and `web/mod.rs` auto-select Ghola or native transport based on config, with warning and install instructions on fallback.
- **Test coverage**: 2,770 tests (up from 2,731), 90.23% coverage. New tests cover HTTP module (53 cases), config serialization, setup status, factory transport sharing, and chain client injection.

## [0.5.1] - 2026-02-17

### Added
- **Console output standards**: Codified terminal display guidelines in a Cursor rule (`.cursor/rules/console-output-standards.mdc`). All CLI reports now use a consistent box-drawing visual language with `display::terminal` helpers.
- **New display helpers**: `score_bar`, `severity_label`, `warning_row`, `info_row`, `link_row`, `detail_row`, `bullet_row`, `table_header`, `table_row`, `numbered_row` — reusable, TTY-aware building blocks for structured terminal output.
- **Automatic word wrapping**: Long text in all content helpers (`kv_row`, `check_pass`, `check_fail`, `warning_row`, `info_row`, `detail_row`, `bullet_row`, `numbered_row`) now wraps at the detected terminal width via `crossterm`. URLs and other long tokens are preserved unbroken.
- **Web UI screenshots**: Six annotated screenshots added to README (Insights, Address, Contract, Address Book, Market, Setup).
- **`crossterm` dependency**: Used for terminal width detection in word-wrapping logic.

### Changed
- **Contract analysis output**: Completely rewritten to use box-drawing section headers, color-coded score bars, severity labels, and structured subsections instead of raw ASCII separators.
- **Ubiquitous helper adoption**: Seven CLI commands (`tx`, `setup`, `market`, `crawl`, `interactive`, `discover`) and one display module (`compliance`) migrated from manual `println!` formatting to `display::terminal` helpers. Zero remaining raw separators in terminal output.
- **README output examples**: Updated OHLC, trades, discover, compliance, and setup examples to match the new box-drawing format. Added contract analysis example.
- **Test coverage**: Increased to 90.41% (2,705 tests) with TTY wrapping continuation tests and DeFi branch coverage.

## [0.5.0] - 2026-02-13

### Added
- **Contract analysis module**: New `scope contract <address>` command with ABI decoding, proxy detection, external call mapping, and a composite security score (0-100). Supports Ethereum, BSC, Polygon, and Arbitrum via block explorer APIs.
- **Contract panel in web UI**: Full contract analysis interface in the browser mode with security score visualization, function tables, external call graphs, and proxy detection display.
- **Address book integration across all features**: All CLI commands and web API endpoints that accept address/token inputs now resolve `@label` shortcuts from the address book. The resolved chain is used as the default unless explicitly overridden.
- **Web UI address book autocomplete**: All address/token input fields in the web UI show `@label` suggestions via a browser-native datalist, populated from the address book and refreshed on add/remove.
- **`@label` hints in all CLI help output**: Every command that accepts an address or token now shows `@label` examples in its help text. The top-level `scope --help` includes a tip about address book shortcuts.
- **Interactive mode contract support**: The interactive REPL now includes a `contract` command for on-the-fly contract analysis.

### Changed
- **Test coverage increased to 90%+**: Comprehensive test suite expansion from ~80% to 90.08% coverage (2525+ tests) across all modules.
- **Codebase cleanup**: Replaced all legacy token references with generic stablecoin examples (DAI, USDC) across code, tests, help text, documentation, and changelog.
- **Clippy compliance**: Resolved all clippy warnings including `io_other_error`, `len_zero`, and `collapsible_if` lints.

## [0.4.4] - 2026-02-15

### Fixed
- **CI formatting**: Fixed `cargo fmt` violation in `try_cex_fallback` function signature that caused CI failure.

## [0.4.3] - 2026-02-15

### Fixed
- **Monitor token resolution**: Aligned with crawl command's chain filter logic — when chain is "ethereum" (default), DexScreener now searches all chains so exact symbol matches sort first regardless of chain. Previously, `scope monitor DAI` resolved to syrupUSDC (substring match on ethereum) instead of Dai (exact match on its native chain).
- **Monitor CEX fallback**: Added CEX ticker fallback to monitor's token resolution (matching crawl behavior) when DexScreener returns no results.

## [0.4.2] - 2026-02-15

### Added
- **Monitor `--pair` flag**: Bypass DexScreener token resolution entirely with `--pair DAI_USDT --venue binance`. Enables monitoring tokens not indexed by DexScreener.
- **Venue interval mapping**: `interval_map` field in venue OHLC descriptors translates canonical intervals (1m, 5m, 1h, 1d) to venue-specific formats. Unmapped intervals pass through unchanged.

### Fixed
- **Biconomy OHLC**: Resolved "Illegal parameter" error caused by Biconomy requiring non-standard interval names (`1min`, `5min`, `hour`, `day`) instead of the canonical format.
- **Monitor DAI resolution**: `scope monitor DAI --venue binance` no longer resolves to the wrong token (syrupUSDC) when used with `--pair`.

## [0.4.1] - 2026-02-15

### Added
- **OHLC/candlestick CLI** (`scope market ohlc`): Fetch real candlestick data from CEX venues. Supports `--venue`, `--interval` (1m, 5m, 15m, 1h, 4h, 1d), `--limit`, and `--format json`.
- **Recent trades CLI** (`scope market trades`): Fetch recent trades from CEX venues. Supports `--venue`, `--limit`, and `--format json`.
- **Monitor real OHLC**: `--venue <venue>` flag on `scope monitor` fetches real exchange OHLC data instead of synthetic candles. Time period selector maps to exchange intervals (1m, 5m, 15m, 1h, 4h, 1d).
- **OHLC web API endpoints**: `POST /api/exchange/ohlc` and `POST /api/exchange/trades` for programmatic access.
- **OHLC capability for all 11 venues**: All built-in venue descriptors now include `ohlc` endpoint configurations.

### Changed
- **CEX venue ticker fallback**: When DexScreener returns no results for a token (e.g., DAI), the system falls back to checking CEX venues (Binance) for ticker data.

### Fixed
- **Token search ranking**: Exact symbol matches are now preferred over substring matches, resolving the syrupUSDC-before-USDC ordering bug.

## [0.4.0] - 2026-02-14

### Added
- **Data-driven exchange venue system**: All CEX integrations are now described by YAML descriptor files instead of hardcoded Rust clients. Add new venues by dropping a YAML file in `~/.config/scope/venues/` — no code changes required.
- **11 built-in venue descriptors**: Binance, Biconomy, Bitget, Bybit, Coinbase, Crypto.com, Gate.io, HTX, Kraken, MEXC, OKX — all with order book, ticker, and trade history support.
- **Venue management CLI** (`scope venues` / `scope ven`): `list`, `schema`, `init`, `validate` subcommands for inspecting and authoring venue descriptors.
- **Exchange monitor layout**: New `--layout exchange` preset for the TUI monitor showing order book, price chart, and recent trades in an exchange-style view.
- **Ticker and trade history traits**: `TickerClient` and `TradeHistoryClient` traits alongside the existing `OrderBookClient`, composed by a unified `ExchangeClient` facade.
- **Configurable exchange client**: Generic `ConfigurableExchangeClient` interprets YAML descriptors at runtime — supports REST GET/POST, JSONPath navigation (`response_root`), symbol case normalization, and flexible field mapping.
- **Venue registry**: `VenueRegistry` loads built-in and user-defined descriptors with Levenshtein-distance typo suggestions for unknown venue names.
- **Web API endpoints**: `GET /api/venues` (list venues) and `POST /api/exchange/snapshot` (full market snapshot).
- **Spinner integration**: `Spinner::println` and `Spinner::suspend` for clean output alongside progress bars; spinner threaded through `resolve_token_input` and `fetch_analytics_for_input`.
- **Colorized error output**: Errors display in red and hints in dimmed text when stderr is a TTY; plain text when piped.
- **DEX aggregator client**: DEX data source abstraction for multi-chain liquidity queries.

### Changed
- **`--market-venue` renamed to `--venue`**: Shorter flag name in `market summary` and `token-health` commands (breaking).
- **Venue argument is now a free-form string**: Instead of an enum, venues are resolved from the registry at runtime. Any venue with a YAML descriptor is valid.
- **Config paths standardized**: All configuration and data stored under `~/.config/scope/` (XDG-compliant) on macOS and Linux.
- **Version reverted to 0.4.0**: The premature 1.0.0 release was yanked from crates.io; this release correctly follows SemVer for a pre-1.0 project with ongoing API changes.
- **Author email updated** to `robot@accomplice.ch`.

### Removed
- **Hardcoded `BiconomyClient` and `BinanceClient`**: Replaced by the generic `ConfigurableExchangeClient` driven by YAML descriptors.
- **`MarketVenue` enum**: Replaced by string-based venue lookup through `VenueRegistry`.

### Fixed
- Spinner output no longer garbles multi-line messages during token resolution.
- Error messages now include colored formatting for better readability in interactive terminals.

## [1.0.0] - 2026-02-13 [YANKED]

### Added
- **Shell completion** (`scope completions bash|zsh|fish`): Generate tab-completion scripts for bash, zsh, and fish shells via `clap_complete`
- **Progress indicators**: Spinners and step-progress bars for all long-running operations (address, tx, crawl, compliance risk, insights, discover, export, token-health, report batch) via `indicatif`
- **Error remediation hints**: Actionable suggestions for common errors (invalid address/hash, missing config, network failures, API auth issues)
- **Help with examples**: `after_help` example invocations in top-level help and subcommands (address, tx, crawl)
- **Command map**: Decision tree in README and QUICKSTART showing which command to use for each task
- **Output Formats section**: Documentation of `--format json` vs `--ai` usage in README and QUICKSTART

### Changed
- **Command grouping**: Commands in `--help` now ordered by task category (entity lookup -> token analysis -> compliance -> data/export -> config) instead of alphabetically
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

[Unreleased]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.3.1...v0.4.0
[1.0.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.3.1...v1.0.0
[0.3.1]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.2.2...v0.3.0
[0.1.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/releases/tag/v0.1.0
[0.2.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.1.0...v0.2.0
[0.2.1]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.2.0...v0.2.1
[0.2.2]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.2.1...v0.2.2
