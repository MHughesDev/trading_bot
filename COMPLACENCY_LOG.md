# Complacency Log: Incomplete Work & Technical Debt

**Last Updated:** 2026-06-10  
**Scope:** Entire repository — all stubs, placeholders, `#[ignore]` tests, incomplete integrations, and unimplemented features

---

## Column Definitions

| Column | Meaning |
|--------|---------|
| **File Path** | Location of the incomplete work |
| **Category** | Type: Auth, Metrics, Collector, Adapter, API, DB, Tests, Tooling, Docs, Config |
| **Severity** | CRITICAL (blocks deployment), HIGH (breaks feature), MEDIUM (degrades feature), LOW (nice-to-have) |
| **Issue** | What's incomplete or lazy |
| **Current State** | What code/implementation exists now |
| **Required State** | What needs to exist for production readiness |
| **Phase** | Scheduled phase or "Post-Phase 6" |
| **Blocker** | Dependencies or things preventing completion |
| **Notes** | Additional context, related issues, cross-references |

---

## Master Table: All Incomplete Work

| File Path | Category | Severity | Issue | Current State | Required State | Phase | Blocker | Notes |
|-----------|----------|----------|-------|----------------|----------------|-------|---------|-------|
| `crates/api/src/auth/session.rs:9-14` | Auth | **CRITICAL** | Placeholder bearer token auth | Any non-empty `Authorization: Bearer x` accepted as valid | Token validation against session table with constant-time comparison | Phase 2 | None | **MUST NOT DEPLOY ON NETWORK-ACCESSIBLE ENDPOINT**. Tracked as M-17. All 11 authenticated routes are accessible to anyone. |
| `migrations/0006_asset_class_registry.sql` | DB | **CRITICAL** | Missing migration for asset class registry | Does not exist | Create migration: asset_class_registry, data_type_registry tables + seed data | Phase 1 | None | Tests reference this migration; Postgres schema incomplete without it. |
| `migrations/0007_unknown.sql` | DB | **CRITICAL** | Gap in migration sequence | Does not exist | Determine schema and create | TBD | None | Phase alignment unclear; 0005 → missing → 0008 → 0009. |
| `migrations/0008_pnl_schema.sql` | DB | **CRITICAL** | Missing P&L persistence tables | Does not exist | Create: `pnl_lots`, `pnl_closes` tables per C-073/C-105 spec | Phase 1 | None | Documented in `crates/storage/src/pnl.rs:9` but migration missing. FifoEngine can't persist. |
| `config/default.toml` | Config | **CRITICAL** | Hardcoded plaintext credentials | Postgres password `trading:trading` in TOML | Move all secrets to environment variables; use vault/secrets manager | Phase 1 | None | Credentials in version control is a security risk. API keys for Alpaca, Coinbase, Kraken, 0x, etc. must be injected at runtime. |
| `crates/collectors/src/crypto/mod.rs:1` | Collector | **HIGH** | Crypto collector not wired up | Module stub; comment: "TODO(Phase 1): crypto collector wiring" | Full wiring of Kraken (and other crypto venues) to event bus; normalize → publish to NATS | Phase 1 | NATS lanes defined | Kraken websocket code exists in submodule but not connected to main event pipeline. |
| `crates/collectors/src/equity/mod.rs:1` | Collector | **HIGH** | Equity collector not wired up | Module stub; comment: "TODO(Phase 6): equity collector wiring (hours/halt aware)" | Full wiring of Alpaca IEX to event bus with trading hours + halt state | Phase 6 | Alpaca API integration | Alpaca source exists but not connected. Hours/halt awareness required before live trading. |
| `crates/execution/src/coinbase.rs:1` | Adapter | **HIGH** | Coinbase execution adapter stub | Single comment: "stub (post-Phase 6 scope)" | Full REST implementation: place orders, query status, handle fills | Post-Phase 6 | Out of scope | Live Coinbase trading not possible. |
| `crates/execution/src/account/tradovate.rs:1-57` | Adapter | **HIGH** | Tradovate account adapter incomplete | Returns empty vectors; parses credentials but no HTTP calls | Fetch balances, positions, transactions from Tradovate REST | Phase 4 | REST endpoints unknown | Account state queries fail silently. |
| `crates/execution/src/account/tradier.rs:1-50` | Adapter | **HIGH** | Tradier account adapter incomplete | Returns empty vectors after credential parsing | Fetch balances, positions, transactions from Tradier REST | Phase 4 | REST endpoints known, API key auth | Account state queries fail silently. |
| `crates/api/src/routes/orders.rs:110-115` | API | **HIGH** | Manual order placement uses hardcoded conservative defaults | GateContext populates position=0, mark_price=0, realized_pnl=0, instrument_active=true (stub values) | Fetch position from Postgres, mark price from Redis, realized P&L from ledger, active status from instrument registry | Phase 2 | Postgres query builders, Redis integration | Risk gate operates with zero position data instead of real account state. Dangerous for position limit enforcement. |
| `crates/api/src/routes/dashboard.rs:47` | API | **HIGH** | Dashboard returns all-zero P&L rollup | Comment: "TODO: fetch from Postgres and Redis. For now return zeros." Returns RollupResponse with all Decimals::ZERO | Fetch `pnl_lots`, `pnl_closes` from Postgres; compute realized/unrealized P&L and win rate | Phase 2 | Postgres pnl schema (migration 0008), Redis integration | Users see $0 PnL regardless of actual position state. No feedback on trading performance. |
| `crates/execution/src/venues/zerox.rs:57` | Adapter | **MEDIUM** | 0x adapter transaction hash query incomplete | Comment: "stub — should fetch firm quote" | Fetch real on-chain transaction status from 0x API | Phase 4 | 0x HTTP wiring | DEX order fills not verified on-chain; assumes filled without checking. |
| `crates/execution/src/venues/zerox.rs:116` | Adapter | **MEDIUM** | 0x adapter order query stub | `query_order()` returns all orders as atomically Filled | Implement real order state tracking via 0x API | Phase 4 | 0x HTTP wiring | All paper DEX trades assumed filled immediately. |
| `crates/execution/src/paper/amm_swap.rs:1-30` | Adapter | **MEDIUM** | AMM/DEX paper simulator skeleton | Phase 1 skeleton; takes caller-supplied FirmQuote (mocked); no real quote source | Phase 4: HTTP wiring to 0x `/price` endpoint for real price impact | Phase 4 | 0x HTTP integration | Paper DEX trades use mocked quotes only; real slippage/price impact not simulated. |
| `crates/observability/src/metrics.rs:1-17` | Metrics | **MEDIUM** | Prometheus metrics not implemented | No-op stubs: `increment_published()`, `increment_quarantined()`, `increment_gap_detected()` | Wire to real Prometheus registry; expose `/metrics` endpoint | Phase 2 | Prometheus dependency | Cannot monitor production event flow; no visibility into message rates, quarantine events, or gaps. |
| `crates/observability/src/correctness.rs:1-15` | Metrics | **MEDIUM** | Correctness metrics not implemented | No-op stubs: `record_consumer_lag()`, `record_quarantine_rate()`, `record_reconciliation_divergence()` | Implement real correctness metrics; validate data pipeline integrity at runtime | Phase 2 | Storage integration | Cannot detect data corruption or replay divergence in production. |
| `crates/builders/src/` | Tests | **MEDIUM** | Order book builder module has zero tests | 4 source files, 0 unit or integration tests | Add comprehensive test coverage for L2 reconstruction, feed events, gap handling | Phase 1 | None | Builder logic untested; order book state unknown at runtime. |
| `crates/mcp-server/src/` | Tests | **MEDIUM** | MCP server module has zero tests | 5 source files (discovery, tools), 0 tests | Add tests for tool discovery, lane queries, instrument enumeration | Phase 5 | None | Tools not validated against real platform state. |
| `crates/builders/src/orderbook.rs` | Builder | **MEDIUM** | Order book reconstruction not implemented | Module exists; implementation incomplete | Full L2 order book from feed events; track bids/asks, handle gaps, handle symbol changes | Phase 2 | None | Strategies cannot access depth data or market structure. |
| `crates/graph/src/populate.rs:226-272` | Graph | **MEDIUM** | INSTRUMENT_AT_VENUE edges marked as never populated (M-6 note stale) | Comment says "never populated" but code DOES populate edges (lines 227-239) | Verify edges are actually populated; update comment if resolved | Phase 7 | None | M-6 appears outdated; edge population code exists. May be documentation lag from fix. |
| `crates/storage/src/parquet/compaction.rs:1-2` | Storage | **LOW** | Nightly compaction stub | Comment: "Phase 2: merge all *.parquet files in partition into one" | Scheduled job (nightly cron) that merges small Parquet files per partition | Phase 2 | None | Query performance degrades over time; many small files instead of optimized structure. |
| `crates/storage/src/postgres/users.rs:2` | Storage | **LOW** | User management documentation placeholder | Comment: "placeholder — to be filled in Phase 2" | Implement user CRUD, role management, permission checks | Phase 2 | None | Users table exists but management incomplete. Doc-level placeholder. |
| `xtask/src/main.rs:10` | Tooling | **LOW** | Money safety check stub | Outputs: "check-money-f64: stub — TODO(Phase B) implement f64 scanner" | Scan workspace for f64 usage on price/size fields; fail CI if found | Phase B | AST analysis tool | CI check referenced but not enforcing money safety constraint. |
| `crates/mcp-server/src/tools/discovery.rs:4-5` | API | **LOW** | MCP discovery uses static data instead of querying platform | Comment: "In production these would query the platform; Phase 5 returns representative static data" | Query live platform registries: `list_lanes()` and `list_instruments()` | Phase 5 | API routes | MCP tools work but agents reason about hardcoded data, not real platform state. |
| `docs/observability/` | Docs | **LOW** | Missing operational procedures for metrics/correctness | No procedures exist | Document: metrics dashboard setup, alert thresholds, gap detection procedures | Phase 2 | None | Ops team won't know how to monitor system health. |
| `docs/procedures/user-management.md` | Docs | **LOW** | User management operational procedures missing | Does not exist | Create: user onboarding, role assignment, permission management | Phase 2 | None | Ops team won't know how to manage access. |

---

## By Severity

### 🔴 CRITICAL (Blocks Deployment) — 3 Items

1. **Auth Placeholder (M-17)** — `crates/api/src/auth/session.rs`
   - Status: Phase 2 upgrade required
   - Action: Implement token validation before exposing on network
   - Impact: **Do not ship to reachable network**

2. **Missing Migrations 0006–0008** — `migrations/`
   - Status: Blocking Postgres schema
   - Action: Create migration files (priority: 0008 for P&L persistence)
   - Impact: Database schema incomplete; P&L not persistable

3. **Hardcoded Credentials in Config** — `config/default.toml`
   - Status: Security violation
   - Action: Move all secrets to environment variables
   - Impact: Credentials in version control

---

### 🟠 HIGH (Breaks Feature) — 7 Items

| Feature | Status | Blocker | Impact |
|---------|--------|---------|--------|
| Crypto market data (Kraken) | Phase 1 wiring | NATS lanes ready | No crypto ingestion |
| Equity market data (Alpaca) | Phase 6 wiring | Alpaca hours/halt API | No equity ingestion |
| Coinbase live execution | Post-Phase 6 | Out of scope | No live crypto execution |
| Tradovate/Tradier account state | Phase 4 adapters | REST endpoints | Account balance/position unknown |
| Manual order placement with real gate | Phase 2 integration | Postgres/Redis | Orders use fake data |
| Dashboard P&L rollup | Phase 2 integration | Postgres pnl schema | No P&L visibility |
| 0x DEX execution | Phase 4 + integration | 0x HTTP wiring | DEX trades unverified |

---

### 🟡 MEDIUM (Degrades Feature) — 6 Items

| Feature | Status | Blocker | Impact |
|---------|--------|---------|--------|
| Prometheus metrics | Phase 2 | None | No production visibility |
| Correctness metrics | Phase 2 | Storage integration | No data integrity validation |
| Order book builder | Phase 2 | None | No depth/market structure |
| Builders test coverage | Phase 1 | None | Untested logic |
| MCP server test coverage | Phase 5 | None | Tools not validated |
| AMM paper simulator | Phase 4 | 0x HTTP wiring | Mock prices only |

---

### 🔵 LOW (Nice-to-Have) — 4 Items

| Item | Status | Phase | Impact |
|------|--------|-------|--------|
| Parquet compaction | Stub | Phase 2 | Query perf degrades |
| User management docs | Placeholder | Phase 2 | Ops guidance missing |
| Money safety CI check | Stub | Phase B | Not enforced |
| MCP dynamic discovery | Static | Phase 5 | Agents use hardcoded data |

---

## Action Plan by Priority

### Before First Paper Trade

**Must Fix (CRITICAL):**
1. Create `migrations/0008_pnl_schema.sql` (P&L persistence)
2. Move secrets from `config/default.toml` to environment variables
3. **Isolate the API** (do not expose port 8080 to any network) until auth is implemented

**Should Fix (HIGH):**
4. Wire Alpaca equity collector (Phase 6 feature)
5. Implement manual order gate with real data (position, mark price, realized P&L)
6. Implement dashboard P&L rollup

### Before Production Deployment

**Mandatory (CRITICAL + HIGH):**
- Implement M-17 bearer token validation
- Create all missing migrations (0006, 0007, 0008)
- Wire crypto (Kraken) collector
- Implement Tradovate/Tradier account fetching

**Highly Recommended (MEDIUM):**
- Add Prometheus metrics
- Implement correctness metrics
- Add test coverage for builders and MCP server

---

## Phase Dependencies

```
Phase 1:
  ├─ Crypto collector wiring         [BLOCKED ON NATS]
  ├─ Auth placeholder (→ Phase 2)    [SECURITY DEBT]
  ├─ Missing migrations (0006-0008)  [CRITICAL]
  └─ Config secrets management       [CRITICAL]

Phase 2:
  ├─ Auth implementation (M-17)
  ├─ Manual order gate real data
  ├─ Dashboard P&L rollup
  ├─ Prometheus metrics
  ├─ Correctness metrics
  ├─ Order book builder
  ├─ Parquet compaction
  └─ User management

Phase 4:
  ├─ Tradovate/Tradier adapters
  ├─ 0x adapter HTTP wiring
  └─ AMM paper simulator integration

Phase 5:
  ├─ MCP dynamic discovery
  └─ MCP test coverage

Phase 6:
  ├─ Equity collector wiring
  └─ Equity hours/halt awareness

Post-Phase 6:
  └─ Coinbase live execution
```

---

## Risk Assessment

### Deployment Risk: **HIGH**

| Risk | Severity | Mitigation |
|------|----------|-----------|
| Auth completely bypassed | CRITICAL | Don't expose API; fix before network access |
| P&L not persisting | CRITICAL | Create migration 0008 before trading |
| Secrets in version control | CRITICAL | Rotate all credentials; move to env vars |
| Account state unknown (manual orders) | HIGH | Fetch real data before allowing orders |
| Market data missing (crypto/equity) | HIGH | Wire collectors; validate feeds |
| DEX trades unverified | MEDIUM | Implement on-chain tx verification before Phase 4 |
| No observability | MEDIUM | Wire Prometheus before production |
| Untested builder/MCP code | MEDIUM | Add tests before Phase 5+ features |

### Readiness Checklist

- [ ] Migrations 0006–0008 created and passing
- [ ] Secrets moved to environment variables
- [ ] Auth placeholder isolated (API not network-accessible)
- [ ] Alpaca equity collector wired and validated
- [ ] Manual order gate fetches real position/mark/realized P&L
- [ ] Dashboard rollup shows real P&L
- [ ] Prometheus metrics wired and tested
- [ ] Builders and MCP server have test coverage > 50%
- [ ] All `#[ignore]` tests pass when run
- [ ] Docker Compose environment starts cleanly
- [ ] One end-to-end trade (paper) completes without errors
- [ ] API security review passed before network exposure

---

## Notes

- **M-17 Auth**: Documented as Phase 1 placeholder; treat as open security item until implemented.
- **M-6 Graph Edges**: Note in populate.rs says edges are "never populated" but code appears to populate them. Verify and update comment.
- **Migrations**: Tests expect migration 0006; sequence broken at 0006–0008. Critical blocker.
- **Dashboard**: Returns all zeros; users cannot see trading performance.
- **Collectors**: Kraken code exists but not wired; Alpaca not wired until Phase 6.
- **Observability**: Metrics are no-ops; system has zero production visibility.
- **0x DEX**: Paper simulator uses mocked quotes; live trades unverified on-chain.

---

**Last Scan Date:** 2026-06-10  
**Scan Depth:** Repository-wide (all crates, migrations, frontend, config)  
**Items Found:** 23 incomplete/lazy  
**Items CRITICAL:** 3  
**Items HIGH:** 7  
**Items MEDIUM:** 6  
**Items LOW:** 4  

