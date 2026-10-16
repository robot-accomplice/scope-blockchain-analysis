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

### Added
- After `setup` in the TUI, the TUI reads the config again, so a new API key works without a restart. Transport settings (proxy, Ghola) still need a restart. (#99)
- In the TUI, a command asks which token a symbol means in a list (↑/↓, Enter; Esc cancels), instead of stopping.
- One token resolver for every command that takes a symbol: it lists the candidates and asks, saves the choice on request, takes `--token <contract>`, checks saved choices against a CEX venue's coin list, and records each lookup in `state/resolutions.jsonl`.
- `--duration forever` (or `inf`) runs a repeating command until `Esc` or `Ctrl-C`. (#96)

### Fixed
- On Windows, a debug build of `scope` no longer overflows its stack on commands such as `completions` and `venues list`: the CLI runs on a thread with an 8 MB stack on every platform.
- The tests and the TUI harness run on Windows, and CI checks Windows (clippy, tests and the offline TUI scenarios).
- On Windows, the monitor no longer acts on each key twice (Windows also reports key releases); `l` switched the layout twice.
- On Windows, Ctrl-C at a prompt of a command the TUI runs outside itself (`setup`, a token choice) cancels the command and returns to the TUI, as on Unix.
- On Windows, a TUI line keeps its backslashes: `save C:\Users\me\out.json` saves to that path. Quotes still group words; a backslash no longer escapes a character there.

## [0.7.0] - 2026-10-14

### Changed (breaking)
- **The REPL is replaced by a full-screen TUI.** `scope tui`, `scope interactive` and `scope shell` open it. Every CLI command runs inside it with the same flags and output; the REPL ran 7 of 17 commands. The REPL session commands (`chain`, `format`, `limit`, `+tokens`, `+txs`, `trace`, `decode`, `tokens`, `ctx`, `clear`) stay. Your history and saved context carry over. The `rustyline` dependency is removed. (#50)
- **Health checks can be `warn`.** JSON checks can have `"status": "warn"`: the value is outside its target but is not a failure, and it does not count against `healthy`. Consumers must handle `warn` (the web UI does).
- **New default rule: depth/volume.** Bid + ask depth within ±2% of mid should be 5–10% of 24h volume. Below 5% fails, so some books that were healthy are now unhealthy. Above 10% warns. Without 24h volume the rule is `n/a`. Overrides: `--depth-window-pct`, `--min-depth-volume-pct`, `--max-depth-volume-pct`, the same `market.health` keys, and web request fields.
- **Every health check shows its value and limit**, pass or fail (for example `Bid depth: 3294 USDT (min 3000)`). Before, a passing level count or depth rule was not shown.
- **Health checks can be `n/a`.** JSON checks are now `{"status": "pass" | "fail" | "n/a", "message": …}`. An `n/a` check does not apply to the book and does not count against `healthy`. Consumers that match only `pass` or `fail` must handle `n/a`. (#52)
- **DEX venues (`eth`, `solana`) use a synthetic book from the AMM curve.** Results for DEX venues change: level count and spread are `n/a`; top-3 and top-10 depth now mean depth within ±0.3% and ±1%. Without pool reserves, scope keeps the old single-level estimate and labels it. (#52)
- **`monitor --export FILE` writes to that file at launch.** Config `monitor.export.path` is the directory for the `E` key. Before, `--export` did nothing. (#51)
- **`monitor --venue` must name a known venue.** An unknown venue is now an error that lists the known ones; before, it was dropped without a word. (#54)
- **A failed write is an error.** For example `scope completions zsh | true` exits with status 1 and prints the I/O error; before, it panicked with status 101. (#48)
- **Log output (`-v`) goes to stderr.** Before, it went to stdout and mixed with piped JSON. (#50)
- **`scope tui` needs a terminal.** It exits with status 2 when stdin or stdout is not a terminal, or `TERM=dumb`. One rule decides color everywhere: `NO_COLOR`, `TERM=dumb` or no terminal turns it off. (#71)
- **Signals end the TUI cleanly.** SIGTERM, SIGHUP, SIGINT and SIGQUIT restore the terminal, save the history and exit with 128+N. Ctrl-Z stops the process and `fg` redraws it. (#71)
- **Status lines go to stderr.** Notes, warnings, prompts and `saved to` lines no longer mix into piped data, so `--format json --report` prints valid JSON. `compliance trace` and `analyze` failures exit with an error status. (#70–#77)
- **Prices use one precision rule everywhere** (CLI, TUI, monitor, web): 2 decimals from 1000, 6 near a USD peg, 4 from 1, then 6, 8, 10. A book takes its decimals from the peg, so its rows line up. Peg prices in the CLI change from `1.0001` to `1.000100`.
- **Amounts name their quote currency.** Depth, volume and size labels use the pair's quote (`USD` for a DEX book) instead of always `USDT`. JSON summaries gain `"quote"`; the web labels depth with it instead of `$`. A DEX book's volume is labeled `Volume (24h, best pool)`.
- **Token risk rules agree across the crawl report, compliance and insights.** 5 buys and 0 sells is now a honeypot sign in compliance too, $0 liquidity is very low there too, and insights calls $75K limited.
- **Chain aliases work everywhere** (`eth`, `matic`, `arb`, `op`, `bnb`, `sol`, `trx`), and `--chain` help and errors list every chain, `aegis` included.
- **`clear` empties the TUI pane; `reset` resets the session context** (was `clear`). (#67)
- **The TUI no longer runs `completions`**, as it already did not run `interactive` or `web`; it says to run it in a shell. (#66)
- **`market summary --csv` appends columns** after the existing ones (`peg_deviation_pct`, `spread_pct`, `bid_top3_depth`, `ask_top3_depth`, `bid_top10_depth`, `ask_top10_depth`). The existing columns and their format do not change. (#58)
- **CLI trades JSON** writes `side` as `buy`/`sell` (was `Buy`/`Sell`) and includes `id`, as the web does.
- **The theme is one system.** Colors come from `scope::display::theme` roles for the CLI, TUI and monitor. Monitor borders are dim, not dark gray; the Blue/Orange scheme keeps its up/down colors but no longer has its own accents; footer keys are bracketed words (`[q]uit`); scheme names are words (`green-red`). `owo-colors` is no longer a dependency.
- **Release process.** `just release` is replaced by `just release-prep X.Y.Z` (on `develop`) and `just release-tag` (on `main`). A crates.io publish error now fails the release. (#80, #81)
- **Library API (`scope-bca-cli`, `scope-bca-core`).**
  - Command handlers take an `&Output`, and the print macros (`outln!`, `errln!`) return `io::Result`. The command router moved from the binary to `scope_cli::cli::dispatch`. `display_error`, `Spinner::new` and `StepProgress::new` return `io::Result`. (#48)
  - `HealthCheck` has a third variant, `NotApplicable`. `OrderBook` and `MarketSummary` have a `source: BookSource` field. `DexPair` has `liquidity_base`. `HealthThresholds` has `amm_step_pct`. `order_book_from_analytics` takes `&HealthThresholds`. (#52)
  - `http::Request` has a new public field, `timeout: Option<Duration>`. Struct literals must set it. `Request::with_timeout` sets it for one request. `NativeHttpClient` uses it; `GholaHttpClient` ignores it. (#61)
  - `ScopeError::Io` is removed (use `ScopeError::io_context`); `ScopeError::InvalidInput` is new for request faults. `ExchangeClient::format_pair` is removed (use `venue_symbol(&Pair)`). `fetch_market_snapshot` takes a trades limit. `Ticker` gains `price_change_pct_24h` and `Serialize`; `Trade` and `Candle` gain `Serialize`. `VenueDescriptor` gains `error_envelope`. `orderbook_level` takes the quote currency.
  - New modules: `market::pair`, `market::venue`, `market::series`, `market::fetch`, `chains::registry`, `compliance::token_risk`, `display::theme`, `display::text`.

### Added
- `market summary` shows the pair's token contracts under the header when the venue descriptor has the new `assets` capability (the venue's coin list). JSON output gets a `tokens` array. (#101)
- **TUI learnability.** `Tab` completes commands, flags, enum values, `@labels`, token aliases, venues and chains. A row under the input shows the command's usage and the next argument. `Ctrl-K` opens a fuzzy command palette. `Esc` or `Ctrl-C` cancels a running command, including `--every` loops. (#50, #53)
- **TUI event log.** Each submitted line is recorded in `tui-events.jsonl` in the scope data directory: input, parsed arguments, route, outcome, error and duration. (#50)
- **`market.health.amm_step_pct`** (default 0.1): the price step of the synthetic AMM book, as a config key, the `--amm-step-pct` flag and a web request field. (#52)
- **The book source is shown** in text, JSON (`book_source`: `exchange`, `synthetic_amm`, `synthetic_estimate`) and markdown reports. (#52)
- **Errors show their cause chain** (`caused by: …`). When a TLS handshake gets a non-TLS reply, the hint names a router, firewall, proxy or VPN that may block the host. (#54)
- **`monitor --venue` falls back to the venue** when DexScreener fails (unreachable, blocked, or the token is unknown there), and says why. (#54)
- **`monitor --pair` accepts any common pair form** (`USDN_USDT`, `USDN/USDT`, `USDN-USDT`, `USDNUSDT`) and converts it to the venue's own format. (#56)
- **Library API: new module `http::shared`.** It selects the HTTP transport in one place. `build` makes a transport from config, `init` installs it for the process, and `transport` returns the installed one. (#61)
- **HTTP record and replay, for tests.** Set `SCOPE_HTTP_MODE=record` and `SCOPE_HTTP_FIXTURES=DIR` to save each response as a fixture file in `DIR`. Set `SCOPE_HTTP_MODE=replay` to answer from those files; a replay run never uses the network, and a request with no fixture is an error. Record does not write a fixture that contains a configured API key, token or RPC secret. Library API: new module `http::replay`. (#62)
- **TUI test harness.** The new `scope-tui-harness` crate runs YAML scenarios against the real `scope tui` in a pseudo-terminal. It types keys, waits for screen text and checks the screen, the event journal, files and the exit code. Offline scenarios use replayed HTTP fixtures. Run them with `just tui-test`. A failed scenario saves its screen and journal under `target/tui-harness/`.
- **Repeating view (#58).** In the TUI, `market summary … --every` shows one view that updates in place: the latest report beside its trends, a pass/fail strip with the last failure, and a chart per key value with its threshold line. Esc stops it and the view stays; the next key leaves it with one copy of the report and a summary (`42 runs · 1 failed (run 16)`). `save x.csv` writes the series.
- **The monitor shows the venue's real order book and trades** with `--venue` (#78), and its DEX book is the same AMM book as `market summary`.
- **Monitor keys (#68).** `?` lists every key; `r` confirms a refresh; the footer shows the refresh interval; a price beside an error is marked `(stale)`.
- **TUI discoverability (#66).** The first screen suggests commands and names Tab and Ctrl-K; help names aliases (`quit`, `q`); the palette marks its row and shows a key legend; hints name a flag's value and an unknown flag.
- **TUI editing (#67).** Ctrl-B, Alt-B, Alt-F and Alt-K; ↑ on an empty line brings back a line that Esc cleared; the pane says when it dropped old lines.
- **TUI release fixes (#70–#77).** Bracketed paste, wide characters, Ctrl-C cancels commands that run outside the TUI, `help <command>`, scrolling, search, a `Terminal too small` guard, footers (`✓ Done`, `✗ Failed`, `Cancelled after`), a status bar and run progress, copy (`Ctrl-Y`) and `save`, and a crash log.
- **`scope venues list` and `/api/venues` include the DEX venues** (`ethereum`, `solana`).
- **Venue errors sent with HTTP 200** show the venue's own message (Bybit, OKX, HTX, Kraken, Biconomy); venue descriptors can declare an `error_envelope`. (#69)
- **Local gates.** The pre-push hook checks the commit identity, then runs `fmt`, `clippy`, `cargo audit` and coverage; it no longer changes `.coverage-last` (use `just coverage-baseline`). (#40, #43)
- **CI.** Rust is pinned in `rust-toolchain.toml` (#41); a daily scheduled audit runs on `develop` and `main` (#42); one audit ignore list in `.cargo/audit.toml` (#40).
- **`just tui-record NAME`** records a harness scenario's API replies as fixtures.

### Fixed
- Compliance risk reports no longer score factors that have no data. Such factors are listed as not assessed, and the level is `Incomplete`. Compliance now uses the Etherscan key from the config and the Etherscan V2 API. (#100)
- Running the test suite overwrote the user's real config (API keys and RPC URLs), address book and token aliases. Tests now use a sandbox in the temp directory. `setup` now starts from the config file and keeps every section. (#98)
- Repeat runs drifted: each run waited a full interval after the last one ended, so a 1-hour run every 30 s made fewer runs than it showed. Runs now keep their schedule. The text report also shortens the 24h volume and the side totals. (#97)
- `insights` printed token liquidity and volume as `$$500.00K`. They now print `$500.00K`. (#95)
- rustls is now 0.23.46. This fixes RUSTSEC-2026-0346, 0347, 0348 and 0349. (#94)
- 24h volume used the ticker's base volume as if it were USDT when a venue reported no quote volume. It is now converted with the mid price.
- The monitor used a hard-coded DEX client for refreshes, which ignored the configured HTTP transport and Ghola. (#51)
- Some requests did not use the configured HTTP transport. Every request now uses it. Ghola now also covers the exchange venues, the DEX client, and the contract and compliance requests. (#61)
- `scope web` now logs a warning when Ghola is enabled but not available and it uses the native transport. Before, it gave no warning. (#61)
- CSV export errors in the monitor were dropped. A failed write now stops the export and logs the reason. (#51)
- `scope web` through the command router hit `unreachable!`. It now returns an error. (#48)
- Library warnings (for example the Etherscan Pro key notice) now go to stderr, or into the TUI pane while it runs. (#50)
- The README build-from-source command failed on the virtual workspace manifest. It is now `cargo install --path crates/scope-web`. (#46)
- Tests no longer call live APIs; coverage is the same with the network on or off (#39). Ten tests that called live venues and asserted nothing now assert against local mock servers.
- `SOLUSD` parsed as SO/LUSD; it is SOL/USD again.
- A symbol DexScreener does not know asked Binance for `USDCUSDTUSDT`; it asks for `USDCUSDT`.
- `/api/exchange/snapshot` ignored `trades_limit` and always fetched 50.
- With `ghola.enabled` and no sidecar, every request failed; scope now falls back to native HTTP and says why. (#64)
- Unknown venue `weex` suggested `okx`; suggestions now need a close match. (#69)
- Enter typed while the TUI started was lost. (#63)
- The monitor's key tests drove a test-only copy of the key handler; there is now one handler.

## [0.6.0] - 2026-10-09

### Changed (breaking)
- **Stricter default order book health rules.** A book is healthy only when each side meets all of these defaults:
  - 10 or more valid levels. A valid level has a price above 0 and a quantity above 0. The previous minimum was 6 levels.
  - A best bid/ask spread of 3% of mid or less.
  - 300 USDT or more in the top 3 valid levels.
  - 2,000 USDT or more in the top 10 valid levels.

  The existing checks stay: no sells below peg, bid/ask ratio 0.2–5.0x, and total in-band depth of 3,000 USDT. Markets with 6 to 9 levels that were healthy before are now unhealthy.
- **`MarketSummary::from_order_book` signature (`scope-bca-core`).** The `peg_target` argument is removed. The peg now comes only from `HealthThresholds::peg_target`. Callers change `from_order_book(&book, peg, &thresholds, vol)` to `from_order_book(&book, &thresholds, vol)`.
- **`HealthThresholds` has three new fields:** `max_spread_pct`, `min_top3_depth`, and `min_top10_depth`. Struct literals must set them or use `..HealthThresholds::default()`.

### Added
- **All 9 health thresholds are overridable on every surface.** The order of precedence is: CLI flag or web request field, then `market.health` in `config.yaml`, then the built-in default.
  - New config section `market.health`. Keys that you do not set keep the built-in default.
  - New CLI flags `--max-spread-pct`, `--min-top3-depth`, and `--min-top10-depth`.
  - `token-health --with-market` and `insights` now accept the same 9 threshold flags as `market summary`.
  - Web `/api/market` and `/api/token-health` accept all 9 threshold keys in the JSON body. Before, `/api/market` accepted 4 and `/api/token-health` accepted none.
  - `--help` shows the built-in default for each threshold flag.

### Fixed
- **`--peg` had no effect** on `token-health` (CLI and web) and `insights`. These paths passed a fixed peg of 1.0.
- **Clippy on Rust 1.99:** updated `async-trait` to 0.1.92 (`double_must_use`), and removed always-true test assertions (`bool_comparison`).
- **Security advisories:** updated `h2` to 0.4.20 (RUSTSEC-2026-0258) and `rustls` to 0.23.45 (RUSTSEC-2026-0285).
- **Coverage depended on the network.** Market summary, token-health market, and Etherscan source parsing now have offline tests on mock clients. One live-API test that could not fail is removed.

## [0.5.6] - 2026-04-20

### Fixed
- **crates.io publish pipeline (broken since v0.5.5)**: The v0.5.5 workspace split replaced the published `scope-bca` crate with three unpublished workspace members (`scope-core` / `scope-cli` / `scope-web`), silently breaking `cargo install scope-bca` for any user who hadn't cached v0.5.4. The release workflow's publish-crate job kept failing on `cargo publish --dry-run` (missing version requirements on internal path deps, and `include_str!` paths pointing outside the packaged tarball) with no alert path back to the release script.
- **Multi-crate publish under the `scope-bca` umbrella**: Each workspace crate now publishes under a crates.io name while keeping its workspace directory and `use` paths unchanged:
  - `crates/scope-core/` → `scope-bca-core` (library; `use scope::*`)
  - `crates/scope-cli/`  → `scope-bca-cli` (CLI handler library)
  - `crates/scope-web/`  → `scope-bca` (owns the `scope` binary, so `cargo install scope-bca` continues to install the CLI that users have bookmarked since v0.4.0)
  Internal path dependencies now carry explicit `version = "0.5.6"` pins (required for crates.io publish) and the release workflow publishes them in dependency order with sparse-index settle sleeps between steps.
- **Self-contained `scope-bca-core` tarball**: Moved the workspace-level `venues/` directory into `crates/scope-core/venues/` and updated the 11 `include_str!` paths plus one test `CARGO_MANIFEST_DIR`-relative path so the crate packages cleanly on its own. `cargo publish -p scope-bca-core --dry-run` now verifies end-to-end. User venue overrides at `~/.config/scope/venues/` are unaffected.
- **`just publish` and `just yank` recipes**: Updated to operate across all three crates in dependency order, with clear manual-fallback instructions.

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

[Unreleased]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.5.6...v0.6.0
[0.5.6]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.5.5...v0.5.6
[0.5.5]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.5.4...v0.5.5
[0.5.4]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.5.3...v0.5.4
[0.5.3]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.4.4...v0.5.0
[0.4.4]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.4.2...v0.4.4
[0.4.2]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.3.1...v0.4.0
[1.0.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.3.1...v1.0.0
[0.3.1]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.2.2...v0.3.0
[0.1.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/releases/tag/v0.1.0
[0.2.0]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.1.0...v0.2.0
[0.2.1]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.2.0...v0.2.1
[0.2.2]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v0.2.1...v0.2.2
