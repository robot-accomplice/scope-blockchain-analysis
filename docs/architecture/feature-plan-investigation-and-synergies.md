# Feature Plan: Investigation Gaps & Compound Opportunities

This plan combines:
1. **Missing investigative/query functionality** — gaps in current capabilities
2. **Compound opportunities** — combining existing functions for outputs greater than the sum of parts

---

## Part 1: Missing Investigation & Query Capabilities

### Transaction Investigation
| Gap | Current State | Priority |
|-----|---------------|----------|
| Internal transactions | `scope tx --trace` returns empty `vec![]` | High |
| Real ABI/input decoding | Only shows 4-byte selector, no param decode | High |
| Transaction logs/events | No access to Transfer, Approval, etc. | High |
| Block-level queries | No `scope block N` or block-by-hash | Medium |

### Address Investigation
| Gap | Current State | Priority |
|-----|---------------|----------|
| Token transfer history | Balances only; no "all transfers in/out" view | High |
| NFT support | No ERC-721, ERC-1155, SPL NFT | Medium |
| Token approvals | No approval listing | Medium |
| Point-in-time balance | No "balance at block X" | Low |

### Compliance & Forensics
| Gap | Current State | Priority |
|-----|---------------|----------|
| Multi-chain compliance | `get_transactions` only Ethereum/mainnet | High |
| Event-based tracing | No topic/signature log queries | High |
| Label/entity database | No address tagging (exchange, mixer, scam) | Medium |
| Cross-chain flow | No bridge/L2 flow tracing | Low |

### Discovery
| Gap | Current State | Priority |
|-----|---------------|----------|
| Token browse / trending | ~~No browse mode~~ `scope discover` (profiles, boosts, top-boosts) | Done |

### Advanced
| Gap | Priority |
|-----|----------|
| MEV/sandwich detection | Medium |
| Wallet clustering | Low |
| Gas estimation / oracle | Low |

---

## Part 2: Compound Opportunities (Synergies)

### Tier 1: High-Value Composites

**1. Wallet Dossier** (`scope address X --dossier` or `scope dossier X`)
- **Components:** address + compliance risk + pattern analysis
- **Output:** Single report with balance, tx history, token holdings, risk score, pattern flags, recommendations
- **Value:** One command instead of 3+; unified context for investigation
- **Effort:** Low — wire existing `AddressReport` + `RiskAssessment` + optional `analyze_patterns`

**2. Report Batch + Risk** (`scope report batch --with-risk`)
- **Components:** report batch + compliance risk per address
- **Output:** Batch report with risk score + tier per address, aggregate exposure by risk level
- **Value:** Compliance officers get risk context when reviewing multiple addresses
- **Effort:** Low — add risk call in batch loop, append section per address

**3. Transaction Deep-Dive** (`scope tx X --deep`)
- **Components:** tx lookup + internal txs (when implemented) + logs
- **Output:** Decoded input, internal call tree, event logs (Transfer, etc.), from/to brief context
- **Value:** Full forensic view of a tx in one place
- **Effort:** Medium — requires internal tx + logs APIs

**4. Token Health Suite** (`scope token-health SYMBOL` or `scope health USDC`)
- **Components:** crawl + market summary (for stablecoins)
- **Output:** DEX liquidity/volume + order book peg/depth + optional live tick
- **Value:** For stablecoins: peg + DEX + orderbook in one view
- **Effort:** Low — compose crawl + market summary, filter by token type

### Tier 2: Medium-Value Composites

**5. Flow Between Wallets** (`scope flow 0xA 0xB`)
- **Components:** address tx history × 2 + filter
- **Output:** Transactions connecting A↔B
- **Effort:** Medium

**6. Portfolio Risk Report** (`scope portfolio risk`)
- **Components:** portfolio summary + compliance risk per address
- **Output:** All watched addresses with risk scores, flagged holdings
- **Effort:** Low

**7. Token + Holder Risk** (crawl enhancement)
- **Components:** crawl + compliance risk on top N holders
- **Output:** Token analytics + "risky holder concentration" indicator
- **Effort:** Medium

**8. Activity Export Enriched** (`scope export --address X --include-token-transfers`)
- **Components:** export + tokentx
- **Output:** CSV/JSON with native txs + ERC-20 transfers
- **Effort:** Medium

### Tier 3: Future

- Block explorer view (`scope block N`)
- Transaction context (auto-show from/to summary in tx output)
- Cross-chain flow

---

## Part 3: Implementation Order

### Phase A: Quick Wins (1–2 days each)
1. **Wallet Dossier** — `scope address X --dossier`
2. **Report Batch --with-risk** — add risk to batch output
3. **Token Health Suite** — `scope token-health USDC` (crawl + market)

### Phase B: Foundation (1 week)
4. **Tx internal transactions** — implement real trace (Etherscan `txlistinternal` or `trace_transaction`)
5. **Token transfer history** — `scope address X --transfers` or in dossier
6. **Multi-chain compliance** — extend BlockchainDataClient to PolygonScan, Arbiscan, etc.

### Phase C: Deeper Composites (2+ weeks)
7. **Tx --deep** — internal txs + logs
8. **Flow between wallets**
9. **Portfolio risk report**
10. **Event-based queries** — logs by topic

---

## Success Criteria

- [x] `scope address X --dossier` produces combined report ✅
- [x] `scope report batch --with-risk` includes risk per address ✅
- [x] `scope token-health USDC` (or `scope health USDC`) combines DEX + market ✅
- [x] `scope --ai` outputs markdown to console for agent parsing ✅
- [x] `scope discover` browses trending/boosted tokens from DexScreener ✅
- [ ] Tx trace returns real internal transactions (Ethereum)
- [ ] Compliance supports Polygon (min) for tx fetching
- [x] Documentation and architecture diagrams updated ✅

---

*Created: 2026-02-13*
