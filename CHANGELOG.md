# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Monitor wizard and watchlists (#47).** `scope monitor <args>` opens the TUI with the monitor wizard, filled from the arguments: targets (pairs, tokens, addresses), checks (the market health preset, with the same results as `market summary`, and rules such as `holders >= 1000`), interval and duration (`--every`, `--duration`, default 60s and 1h), and outputs (`--csv`, one row per check per run; `--report`). Its run shows a grid of each target's pass/fail state, with a detail pane, charts, and keys that run the evidence commands. Ctrl-G switches between the grid and the shell. Watchlists are saved to `~/.scope-bca/watchlists/` (`--save`, Ctrl-S) and run again with `watch open <name>` in the TUI. The health threshold flags of `market summary` work on `monitor`, and `--yes` starts without the wizard.
- `scope::metrics` (scope-bca-core): one metric catalog, with token and address metrics. `scope::market::series` stays as a re-export for one release.
- In the TUI, `crawl <symbol>` asks which token in the pane, not on the plain terminal.
- Scope offers to install the Ghola sidecar when `ghola.enabled` is set and none is found. It asks on a terminal (once; a "no" is remembered), in the `setup` wizard, and suggests it in `setup --status`. It downloads the latest `robot-accomplice/ghola` release over HTTPS, checks its SHA-256 against `checksums.txt` before extracting, and installs `ghola` to `~/.scope-bca/bin` (PATH is not changed). With no archive for the system it offers `go install`. `--yes`, JSON/CSV/Markdown output, pipes, the web API and the TUI never install. Scope now starts an installed `ghola --serve` when no sidecar answers, and at exit asks whether to stop it (no answer within 15 s stops it).

### Fixed
- The Ghola install hint now names `go install github.com/robot-accomplice/ghola/cmd/ghola@latest`.

### Deprecated
- The old monitor dashboard: `scope monitor --dashboard` opens it for one more release. `--layout`, `--scale` and `--color-scheme` work only with it; `--export` is now `--csv`. The price bounds of `monitor.alerts` become watchlist rules; the whale and volume-spike alerts have no metric yet (#122).

### Changed (breaking)
- scope reads and writes YAML with serde-saphyr (safe Rust) in place of the deprecated serde_yaml, so `unsafe-libyaml` (unmaintained, RUSTSEC-2024-0449) is gone. Files that serde_yaml wrote read the same. In scope-bca-core, `ConfigError::Parse` now holds a `serde_saphyr::Error`, and YAML error messages read differently. (#118)
- `crawl --yes` no longer selects the first match for an ambiguous symbol. It stops with the candidates; pass `--token <contract>`. `--yes` now skips only the save question.
- `token-health`, `insights` and `market` on a DEX venue no longer take the first search match. An ambiguous symbol asks in a terminal, or stops with the candidates. The web API also returns this error for an ambiguous symbol.
- A token search matches exact symbols only (case-insensitive) or an exact contract; a search by name (for example `crawl uniswap`) no longer finds tokens.
- `market` on a DEX venue and `monitor` look for the symbol on their own chain only. Before, the default chain searched every chain and could report a pool from another chain.
- A single match is used without the save question; pass `--save` to save it.
- All scope files are now in one folder, `~/.scope-bca` (`SCOPE_BCA_HOME` moves it): `config.yaml`, `venues/`, `data/` and `state/`. On the first start, scope copies the files from `~/.config/scope` and the platform data folder, and writes `state/migration.jsonl`. The old files are not changed.
- **Library (`scope-bca-core`): the HTTP transport is passed to each client; there is no process-wide transport.** `scope::http::init`, `scope::http::transport` and `scope::http::shared::{init, init_with, transport, transport_with, describe}` are removed: build a transport with `scope::http::build(&config)` and pass its `client`. `ChainClientFactory` has a new required method `http()`, the transport for the clients it does not create itself. New constructors take the transport: `ExchangeClient::from_descriptor_with_http`, `VenueRegistry::create_exchange_client_with_http`, `MarketVenue::{resolve_with_http, from_name_with_http}`, `fetch_token_summary_by_name_with_http`, `BlockchainDataClient::{new_with_http, from_env_opt_with_http}` and `RiskEngine::from_env_with_http`. The constructors without a transport (`DexClient::new`, `EthereumClient::{new, for_chain}`, `SolanaClient::new`, `TronClient::new`, `ConfigurableExchangeClient::new` and the old names of those above) are deprecated and will be removed in the next minor release; each builds its own transport from the environment and the default config. (#59)
