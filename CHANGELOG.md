# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed (breaking)
- `crawl --yes` no longer selects the first match for an ambiguous symbol. It stops with the candidates; pass `--token <contract>`. `--yes` now skips only the save question.
- `token-health`, `insights` and `market` on a DEX venue no longer take the first search match. An ambiguous symbol asks in a terminal, or stops with the candidates. The web API also returns this error for an ambiguous symbol.
- A token search matches exact symbols only (case-insensitive) or an exact contract; a search by name (for example `crawl uniswap`) no longer finds tokens.
- `market` on a DEX venue and `monitor` look for the symbol on their own chain only. Before, the default chain searched every chain and could report a pool from another chain.
- A single match is used without the save question; pass `--save` to save it.
- All scope files are now in one folder, `~/.scope-bca` (`SCOPE_BCA_HOME` moves it): `config.yaml`, `venues/`, `data/` and `state/`. On the first start, scope copies the files from `~/.config/scope` and the platform data folder, and writes `state/migration.jsonl`. The old files are not changed.
- **Library (`scope-bca-core`): the HTTP transport is passed to each client; there is no process-wide transport.** `scope::http::init`, `scope::http::transport` and `scope::http::shared::{init, init_with, transport, transport_with, describe}` are removed: build a transport with `scope::http::build(&config)` and pass its `client`. `ChainClientFactory` has a new required method `http()`, the transport for the clients it does not create itself. New constructors take the transport: `ExchangeClient::from_descriptor_with_http`, `VenueRegistry::create_exchange_client_with_http`, `MarketVenue::{resolve_with_http, from_name_with_http}`, `fetch_token_summary_by_name_with_http`, `BlockchainDataClient::{new_with_http, from_env_opt_with_http}` and `RiskEngine::from_env_with_http`. The constructors without a transport (`DexClient::new`, `EthereumClient::{new, for_chain}`, `SolanaClient::new`, `TronClient::new`, `ConfigurableExchangeClient::new` and the old names of those above) are deprecated and will be removed in the next minor release; each builds its own transport from the environment and the default config. (#59)
