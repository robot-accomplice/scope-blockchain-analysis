# Test Coverage Improvement Plan

This document outlines a phased plan to address unit testing gaps. The goal is to bring priority modules to ≥80% coverage and ensure critical paths are exercised.

## Prerequisites

- All tests must pass (`cargo test`)
- No `cargo clippy` warnings
- Use existing patterns: `mockito` for HTTP, `#[tokio::test]` for async, in-file `#[cfg(test)] mod tests`

---

## Phase 1: Compliance Module (Target: 80%+)

**Files:** `datasource.rs`, `mod.rs`

### 1.1 `src/compliance/datasource.rs` (40.5% → 80%)

| Task | Approach |
|------|----------|
| Mock Etherscan API responses | Use `mockito::Server::new_async()` and `mock("GET", Matcher::Any)` with fixture JSON |
| Test error handling for API failures | Mock 4xx/5xx responses, network timeout, malformed JSON |
| Test transaction parsing edge cases | Empty result arrays, missing fields, invalid timestamps |
| Test `analyze_patterns` with varied inputs | Structuring, layering, velocity signals |

**Reference:** `src/compliance/risk.rs` tests (lines 591+) use injected `BlockchainDataClient` with mockito.

### 1.2 `src/compliance/mod.rs` (43.8% → 80%)

| Task | Approach |
|------|----------|
| Test sanctions check integration | Mock sanctions API (or bypass if none), assert flow |
| Test analyzer with different configurations | Vary `DataSources`, time ranges, pattern types |
| Test report generation entry points | Call `generate_report` with mock data |

---

## Phase 2: CLI Commands — Critical Gaps (0–25% → 60%+)

**Acceptance:** CLI module ≥60%.

### 2.1 `src/cli/compliance.rs` (0% → 60%)

| Task | Approach |
|------|----------|
| Test argument parsing | `ComplianceCommands::Risk(RiskArgs { … })` construction, all subcommands |
| Test error messages | Invalid address, missing API key, API failure paths |
| Test output formats | Table, JSON, Markdown, Yaml via `format_risk_report` |
| Test `handle_risk`, `handle_trace`, `handle_analyze`, `handle_compliance_report` | Inject mock `BlockchainDataClient`, assert output shape |
| Fix path handling | `args.output` is `String`; use `Path::new(&path).extension()` for extension detection |

**Reference:** `src/cli/market.rs` tests construct `SummaryArgs` and call `run_summary` with mockito server.

### 2.2 `src/cli/address.rs` (15.8% → 60%)

| Task | Approach |
|------|----------|
| Mock chain clients | Create mock `ChainClient` impl or use chain factory with mock URLs |
| Test USD valuation integration | Mock DexScreener in `chains::dex`, assert `AddressReport.usd` populated |
| Test token balance fetching | Mock RPC responses for ERC-20 / SPL / TRC-20 |
| Test `run_address` end-to-end | With mockito for chain + DEX APIs |

### 2.3 `src/cli/crawl.rs` (5.2% → 60%)

| Task | Approach |
|------|----------|
| Mock DEX API responses | DexScreener token/holder endpoints |
| Test report generation | `generate_report` with mock crawl data |
| Test error handling | Invalid address, API timeout, empty holder list |
| Test CSV/JSON/table output | Assert output contains expected fields |

### 2.4 `src/cli/tx.rs` (23.9% → 60%)

| Task | Approach |
|------|----------|
| Test transaction decoding | Mock chain RPC with known tx payload |
| Test trace functionality | Mock Etherscan-like trace API |
| Test invalid hash handling | Short, non-hex, wrong-length hashes |

### 2.5 `src/cli/portfolio.rs` (21.7% → 60%)

| Task | Approach |
|------|----------|
| Test CRUD operations | Add, remove, list with `tempfile` for persistence |
| Test summary aggregation | Multiple holdings, chain aggregation |
| Fix test structs | Ensure `SummaryArgs` and other test fixtures include all required fields (e.g. `report`) |

---

## Phase 3: Chain Clients (13–28% → 60%+)

### 3.1 `src/chains/ethereum.rs` (19.6% → 60%)

| Task | Approach |
|------|----------|
| Mock Etherscan API | `?module=account&action=balance`, `tokentx`, etc. |
| Test all RPC methods | Balance, transactions, token transfers |
| Test error handling | Rate limit, invalid key, malformed response |

### 3.2 `src/chains/solana.rs` (21.7% → 60%)

| Task | Approach |
|------|----------|
| Mock Solana RPC | JSON-RPC over HTTP, `getAccountInfo`, `getTokenAccountsByOwner` |
| Test SPL token parsing | Parse token account data, decimals, symbol |

### 3.3 `src/chains/tron.rs` (28.2% → 60%)

| Task | Approach |
|------|----------|
| Mock TronGrid API | `/v1/accounts/`, `/wallet/getnowblock`, transaction endpoints |
| Test TRC-20 parsing | Balance, decimals, name |

### 3.4 `src/chains/dex.rs` (13.7% → 60%)

| Task | Approach |
|------|----------|
| Mock DexScreener API | `/tokens/`, price endpoints |
| Test price caching | Verify cache hit/miss behavior if present |

---

## Phase 4: Display & Polish

### 4.1 `src/display/report.rs` (71.4% → 85%)

| Task | Approach |
|------|----------|
| Cover remaining branches | Empty inputs, missing market cap, edge formats |
| Test table generation | Column alignment, truncation |

---

## Implementation Order

| Phase | Est. effort | Dependencies |
|-------|-------------|--------------|
| 1 | 2–3 days | None |
| 2 | 3–4 days | Phase 1 (compliance used by CLI) |
| 3 | 2–3 days | Can overlap with Phase 2 |
| 4 | 0.5 day | None |

**Recommended sequence:** 1 → 2.1, 2.5 (fix compile) → 2.2, 2.3, 2.4 → 3 → 4.

---

## Quick Wins (Completed ✅)

1. ~~**Fix compile errors**~~ — Fixed in `portfolio.rs`, `compliance.rs`, `address.rs`.
2. ~~**`compliance.rs` path bug**~~ — Uses `Path::new(&path)` before `.extension()`.
3. ~~**`portfolio.rs` and `address.rs` tests**~~ — Added missing `report` field to `SummaryArgs` and `AddressArgs` in test fixtures.

---

## Test Patterns (Copy-Paste)

### Mockito HTTP mock (async)

```rust
#[tokio::test]
async fn test_fetch_with_mock() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", mockito::Matcher::Any)
        .with_status(200)
        .with_body(r#"{"status":"1","result":[]}"#)
        .create();
    // Use server.url() as base for client
}
```

### Tempfile for CLI persistence

```rust
let dir = tempfile::tempdir().unwrap();
let config_path = dir.path().join("config.yaml");
// Run command that reads/writes config_path
```

### assert_cmd for CLI

```rust
use assert_cmd::Command;
let mut cmd = Command::cargo_bin("scope").unwrap();
cmd.arg("compliance").arg("risk").arg("0x1234");
cmd.assert().success();
```

---

## Success Criteria

- [x] `cargo test` passes
- [x] `cargo tarpaulin --out Stdout` shows ≥80% project coverage (89.01%)
- [x] Compliance module ≥80% (datasource 100%, mod 100%)
- [x] CLI compliance expanded (resolve_targets, parse_address_line, export formats, trace error path)
- [x] No new `#[ignore]` tests without documented justification

## Execution Summary (2026-02-13)

**Phase 1 — completed:**
- `datasource.rs`: Added HTTP 500, malformed JSON, null result, invalid timestamp/value tests → 100% coverage
- `mod.rs`: Added sanctions empty lists, sanctioned summary, MatchType clone tests

**Phase 2.1 — completed:**
- `cli/compliance.rs`: Added export markdown/yaml, resolve_targets from file, parse_address_line, trace connection-refused (Err path), test_detect_chain_unknown #[test]

**Phase 4 — partial:**
- `display/report.rs`: Added test_report_footer

**Coverage:** 89.01% overall. Compliance datasource + mod at 100%. Remaining gaps: CLI market repeat mode, crawl error paths, chain clients.

---

*Last updated: 2026-02-13*
