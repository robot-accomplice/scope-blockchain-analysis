# Chain Normalization Strategy

**Goal:** Normalize functionality across all chains and normalize the presentation of retrieved data.

## Current State

### Supported Chains

| Chain | Type | Balance | Tx | Token Balances | Token Info | Holders | Holder Count |
|-------|------|---------|-----|----------------|------------|---------|--------------|
| Ethereum | EVM | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Polygon, Arbitrum, Optimism, Base, BSC | EVM | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Solana | Non-EVM | ✓ | ✓ | ✓ | ✓ (RPC decimals) | ✗ | 0 |
| Tron | Non-EVM | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |

### Functionality Gaps

1. **Token metadata in balance lists**
   - **EVM**: Full symbol/name from explorer APIs
   - **Tron**: `get_token_balances` returns generic `"TRC20"` / `"TRC-20 Token"` for all tokens; does not call `get_token_info` per contract
   - **Solana**: Symbol/name from Metaplex or RPC where available; otherwise truncated mint

2. **Portfolio native symbol**
   - `get_native_symbol()` only handles `solana`, `ethereum`, `tron`; EVM L2s return `"???"`

3. **Compliance / risk assessment**
   - `BlockchainDataClient.get_transactions()` supports `ethereum` | `mainnet` only
   - Risk scoring unavailable for Polygon, Arbitrum, Solana, Tron

4. **Solana holder data**
   - `get_token_holders` returns empty (would require Solscan Pro API)

5. **Transaction fee presentation**
   - Solana: special-case parsing of `gas_price` as lamports → fee
   - EVM: `fee_wei = gas_price * gas_used`
   - Tron: uses EVM-style path (may not match Tron semantics)

### Presentation Gaps

1. **Token balance formatting**
   - Chains use different decimals (18, 9, 6); `formatted_balance` formats vary
   - No shared helper for "human-readable with consistent decimals/suffixes"

2. **Field naming**
   - Address report: `report.tokens` uses `TokenBalance` with `contract_address`
   - Portfolio: `TokenSummary` uses `mint` for the same concept

3. **Explorer links**
   - Chain-specific URLs (expected) but generated in multiple places; could centralize

4. **Error messages**
   - Chain-specific validation messages; some inconsistent phrasing

5. **CSV / JSON output**
   - Column ordering and field names can vary by command

---

## Normalization Plan

### Phase 1: Functionality Parity

#### 1.1 Enrich Tron token balances with metadata
- In `TronClient::get_token_balances`, optionally call `get_token_info(contract_address)` for each unique TRC20 to populate symbol/name/decimals
- Or: batch lookup via Tronscan if available; fallback to `"TRC20"` on failure

#### 1.2 Complete `get_native_symbol` for all chains
- Add: `polygon` → `MATIC`, `arbitrum` → `ETH`, `optimism` → `ETH`, `base` → `ETH`, `bsc` → `BNB`
- Source from `ChainClient::native_token_symbol()` where possible

#### 1.3 Compliance datasource for non-Ethereum chains (future)
- Extend `BlockchainDataClient` or add chain-specific clients (Polygonscan, Arbiscan, etc.)
- Or: use existing chain clients' `get_transactions` and normalize to `EtherscanTransaction`-like format for risk engine

### Phase 2: Unified Data Structures

#### 2.1 Single source of truth for chain metadata
- Introduce `ChainMetadata` (or extend config) with: `name`, `native_symbol`, `decimals`, `explorer_base`, `explorer_token_path`
- All display and formatting logic reads from this

#### 2.2 Normalize `TokenBalance` / `TokenSummary`
- Align field names: use `contract_address` (or `address`) consistently; avoid `mint` in some paths and `contract_address` in others
- Ensure all chains populate: `symbol`, `name`, `decimals`, `formatted_balance` in a consistent way

#### 2.3 Shared formatting helpers
- `format_token_balance(raw, decimals)` → human-readable string
- `format_usd(value)` (exists in report) → reuse across CLI
- `format_large_number(n)` (exists in crawl) → reuse

### Phase 3: Presentation Consistency

#### 3.1 Unified output templates
- Define canonical field order for table output: Address, Chain, Balance, Symbol, Token Count, etc.
- Same section order for markdown reports regardless of chain

#### 3.2 CSV schema per command
- Document and standardize CSV headers (e.g. `address,chain,balance,balance_usd,tx_count,token_count`)

#### 3.3 Error message style guide
- Consistent format: `"<context>: <requirement> (actual: <value>)"`
- Same phrasing for "not supported", "not found", "invalid"

---

## Implementation Order

| Priority | Task | Effort | Impact |
|----------|------|--------|--------|
| 1 | Fix `get_native_symbol` for all EVM chains | Low | High (portfolio, reports) |
| 2 | Tron: enrich token balances with `get_token_info` | Medium | High (presentation) |
| 3 | Centralize chain metadata (symbol, decimals, explorer) | Medium | High (foundation) |
| 4 | Align `TokenSummary` / `TokenBalance` field names | Low | Medium |
| 5 | Shared `format_token_balance` / number helpers | Low | Medium |
| 6 | Compliance for non-Ethereum (Polygon, etc.) | High | Medium (compliance users) |

---

## Open Questions

- **Solana holders**: Integrate Solscan Pro (paid) or leave empty?
- **Compliance**: Normalize tx format from all chains for risk engine, or keep Ethereum-only for now?
- **Tron balance enrichment**: Call `get_token_info` per token (N+1) or is there a batch Tronscan endpoint?
