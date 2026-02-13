# Unit Test Coverage Work — Handoff to Cursor

## Current Status

**Coverage:** 89.01% (run `cargo tarpaulin --out Stdout` for latest). Target: 80% ✓

**Implementation plan:** [docs/architecture/test-coverage-improvement-plan.md](docs/architecture/test-coverage-improvement-plan.md) — phased plan with tasks, patterns, and success criteria.

## Critical Files to Test

### Priority 1: Compliance Module ✅

- [x] `src/compliance/risk.rs` — 67.6% coverage (GOOD)
- [x] `src/compliance/datasource.rs` — 100% coverage (HTTP 500, malformed JSON, invalid tx, null result tests)
- [x] `src/compliance/mod.rs` — 100% coverage (sanctions summary, empty lists, MatchType tests)

### Priority 2: CLI Commands (0-15% Coverage — URGENT)

- [x] `src/cli/discover.rs` — Token discovery; mock DexScreener profiles/boosts/top-boosts; run_with_client, truncate_address
- [x] `src/cli/market.rs` — parse_duration tested; run_summary has mockito integration tests (text + JSON)
- [x] `src/cli/report.rs` — resolve_targets, batch_report_to_markdown; addresses, from_file, chain filter
- [x] `src/cli/address_report.rs` — generate_address_report_section, generate_address_report, generate_dossier_report
- [x] `src/cli/compliance.rs` — path fix applied; resolve_targets, parse_address_line, export md/yaml, trace Err path
- [ ] `src/cli/address.rs` — 15.8% coverage
  - Mock chain clients
  - Test USD valuation integration
  - Test token balance fetching
- [ ] `src/cli/crawl.rs` — 5.2% coverage
  - Mock DEX API responses
  - Test report generation
  - Test error handling
- [ ] `src/cli/tx.rs` — 23.9% coverage
  - Test transaction decoding
  - Test trace functionality
- [ ] `src/cli/portfolio.rs` — test fixtures updated; improve coverage
  - Test CRUD operations
  - Test summary aggregation

### Priority 3: Chain Clients (13-25% Coverage)

- [ ] `src/chains/ethereum.rs` — 19.6% coverage
  - Mock Etherscan API
  - Test all RPC methods
  - Test error handling
- [ ] `src/chains/solana.rs` — 21.7% coverage
  - Mock Solana RPC
  - Test SPL token parsing
- [ ] `src/chains/tron.rs` — 28.2% coverage
  - Mock TronGrid API
- [x] `src/chains/dex.rs` — Discovery methods tested (get_token_profiles, get_token_boosts, get_token_boosts_top); mock DexScreener API

### Priority 4: Display & Utils (DONE or LOW PRIORITY)

- [x] `src/display/compliance.rs` — 100% coverage ✅
- [x] `src/display/charts.rs` — 91.3% coverage ✅
- [ ] `src/display/report.rs` — 71.4% coverage (OK)

## Testing Strategy

### Use Mockito for HTTP Mocking

```rust
use mockito::{mock, Server};

#[tokio::test]
async fn test_etherscan_fetch() {
    let mut server = Server::new();
    let _m = mock("GET", "/api")
        .with_status(200)
        .with_body(r#"{"status":"1","result":[]}"#)
        .create();
    
    // Test your code here
}
```

### Test Organization

- Unit tests in same file: `#[cfg(test)] mod tests { }`
- Integration tests in `tests/` directory (`tests/cli_integration.rs`)
- Use `tempfile` crate for file operations
- Use `assert_cmd` for CLI testing

### CLI Output Tests (`tests/cli_integration.rs`)

Covers terminal output for:

- Help/version (main, subcommands: address, tx, portfolio, export, discover, market, token-health, crawl, report, compliance, insights, completions)
- Setup `--status` output (Scope Configuration Status, Config file, API Keys)
- Error handling (missing args, invalid formats, unsupported chain)
- Global flags (`--ai`, `--no-color`, `-v`, `--config`)
- Insights command: `insights --help`, `insight` alias, requires target
- Shell completions: `completions --help`, `completions bash/zsh/fish`

### Progress Indicators (`src/cli/progress.rs`)

Unit tests for:

- `Spinner` creation, message update, and finish variants (success, warning, clear)
- `StepProgress` creation, increment, and finish
- Non-TTY fallback behavior (hidden progress bars in test/piped contexts)

### Error Remediation Hints (`src/main.rs`)

Unit tests for `error_suggestion()`:

- `InvalidAddress` → format hint (EVM, Solana, Tron)
- `InvalidHash` → hash format hint
- `Config(NotFound)` → `scope setup` suggestion
- `Network` → network/retry hint
- `Api` with 401/403 → API key hint
- `NotFound` → verify resource hint
- `Other` → returns `None` (no hint)

### Help Display & Typo Suggestion Tests (`tests/cli_integration.rs`)

- `test_help_shows_examples` — Top-level help contains "Examples:" and "Documentation:" sections
- `test_address_help_shows_examples` — Address help contains example invocations
- `test_typo_suggestion` — Misspelled command ("adress") shows "similar" suggestion
- `test_cli_completions_parsing` — Completions subcommand parses correctly

### Coverage Quick Wins

1. Add tests for all error paths (currently mostly uncovered)
2. Add tests for all public functions
3. Test boundary conditions (empty inputs, max values)
4. Test format conversions (table → JSON → CSV)

## Commands

```bash
# Run tests with coverage
cargo tarpaulin --out html

# Check coverage (80% min + no regression); used by pre-push hook
just coverage-check
# or: ./scripts/check-coverage.sh

# Install pre-push hook (runs coverage check before every push)
just install-hooks

# Run specific test
cargo test test_name -- --nocapture

# Check coverage for one module
cargo tarpaulin --out Stdout -- src/cli/compliance.rs
```

## Notes for Cursor

- Tests should be deterministic (no random data)
- Use `tokio::test` for async tests
- Mock external APIs, don't make real calls
- Test both success and failure cases
- Keep tests fast (< 100ms each)
- Add `#[ignore]` for slow integration tests

## Acceptance Criteria

- [ ] All public functions have tests
- [ ] All error paths tested
- [ ] Coverage report shows 80%+ for compliance module
- [ ] Coverage report shows 60%+ for CLI module
- [ ] All tests pass: `cargo test`
- [ ] No warnings: `cargo clippy`

## Files Ready for Testing

Already set up with test infrastructure:

- `src/compliance/risk.rs` — Good example of comprehensive tests
- `src/compliance/datasource.rs` — Has basic tests, needs more
- `src/display/compliance.rs` — Has 100% coverage (reference)

## GitHub Actions

Coverage is uploaded to Codecov automatically on every push to main.  
Target: Block release if coverage < 80%.

---

Last updated: 2026-02-13  
Project exceeds 80% coverage target; priority files above still have room to improve.

**Test counts:** 1303 lib + 12 binary + 49 integration + 28 doc = **1392 total tests** (0 failures).
