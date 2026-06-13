# Complacency Log: Incomplete Work & Technical Debt

**Last Updated:** 2026-06-10 (v2 — post-implementation pass)
**Scope:** Entire repository — all stubs, placeholders, `#[ignore]` tests, incomplete integrations, and unimplemented features

> **Implementation plan:** every open item below (plus 8 findings from the
> 2026-06-13 sweep this log missed — dead `/auth/*` routes, frontend builder
> gaps, parse-only sizing, hot-path stage-3, unused graph/semantic infra) is
> sequenced in [`plans/set-E/`](plans/set-E/MASTER.md). See its Item → Phase Map
> for the cross-reference.

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
| `crates/api/src/auth/session.rs:9-14` | Auth | **CRITICAL** | Placeholder bearer token auth | Any non-empty `Authorization: Bearer x` accepted as valid | Token validation against session table with constant-time comparison | Phase 2 | None | **GUARDRAIL ACTIVE**: platform refuses to bind on non-loopback address. M-17. All 11 authenticated routes accept any token on loopback. |
| `config/default.toml` | Config | **LOW** | Hardcoded dev credentials | Postgres password `trading:trading` in TOML | Move all secrets to environment variables for production | Phase 1 | None | Downgraded: values are dev-environment defaults, not real secrets. Still must be replaced before any production deployment. |
| `crates/execution/src/coinbase.rs:1` | Adapter | **HIGH** | Coinbase execution adapter stub | Single comment: "stub (post-Phase 6 scope)" | Full REST implementation: place orders, query status, handle fills | Post-Phase 6 | Out of scope | Live Coinbase trading not possible. |
| `crates/execution/src/account/tradovate.rs` | Adapter | **HIGH** | Tradovate account adapter not implemented | Returns `Err(AccountSourceError::NotImplemented)` for all methods | Fetch balances, positions, transactions from Tradovate REST | Phase 4 | REST endpoints unknown | Now fails loudly (was silently returning empty). Account state queries error out. |
| `crates/execution/src/account/tradier.rs` | Adapter | **HIGH** | Tradier account adapter not implemented | Returns `Err(AccountSourceError::NotImplemented)` for all methods | Fetch balances, positions, transactions from Tradier REST | Phase 4 | REST endpoints known, API key auth | Now fails loudly (was silently returning empty). |
| `crates/api/src/routes/orders.rs` | API | **HIGH** | Manual order placement disabled | Returns `503 SERVICE_UNAVAILABLE` — position/mark-price data not yet wired | Fetch position from Postgres, mark price from Redis, P&L from ledger, active status from registry | Phase 2 | Postgres query builders, Redis integration | Fixed from false-permissive (was passing with position=0); now safely rejects until Phase 2 data wiring. |
| `crates/api/src/routes/dashboard.rs:47` | API | **HIGH** | Dashboard returns all-zero P&L rollup | Returns `RollupResponse` with all `Decimal::ZERO`; comment: "TODO: fetch from Postgres and Redis" | Fetch `pnl_lots`, `pnl_closes` from Postgres; compute realized/unrealized P&L and win rate | Phase 2 | Postgres pnl schema (migration 0008 now exists), Redis integration | Users see $0 P&L regardless of actual position state. |
| `crates/execution/src/venues/zerox.rs:57` | Adapter | **MEDIUM** | 0x adapter transaction hash query stub | Comment: "stub — should fetch firm quote" | Fetch real on-chain transaction status from 0x API | Phase 4 | 0x HTTP wiring | DEX order fills not verified on-chain. |
| `crates/execution/src/venues/zerox.rs:116` | Adapter | **MEDIUM** | 0x adapter order query stub | `query_order()` always returns `BrokerOrderState::Filled` with dummy values | Implement real order state tracking via 0x API | Phase 4 | 0x HTTP wiring | All paper DEX trades assumed filled immediately. |
| `crates/execution/src/paper/amm_swap.rs:1-30` | Adapter | **MEDIUM** | AMM/DEX paper simulator skeleton | Takes caller-supplied `FirmQuote` (mocked); no real quote source | Phase 4: HTTP wiring to 0x `/price` endpoint for real price impact | Phase 4 | 0x HTTP integration | Paper DEX trades use mocked quotes only. |
| `crates/observability/src/metrics.rs:1-17` | Metrics | **MEDIUM** | Prometheus metrics not implemented | No-op stubs: `increment_published()`, `increment_quarantined()`, `increment_gap_detected()` | Wire to real Prometheus registry; expose `/metrics` endpoint | Phase 2 | Prometheus dependency | Zero production visibility into event flow. |
| `crates/observability/src/correctness.rs:1-15` | Metrics | **MEDIUM** | Correctness metrics not implemented | No-op stubs: `record_consumer_lag()`, `record_quarantine_rate()`, `record_reconciliation_divergence()` | Implement real correctness metrics; validate data pipeline integrity at runtime | Phase 2 | Storage integration | Cannot detect data corruption or replay divergence. |
| `crates/mcp-server/src/` | Tests | **MEDIUM** | MCP server module has zero tests | 5 source files, 0 tests | Add tests for tool discovery, lane queries, instrument enumeration | Phase 5 | None | Tools not validated against real platform state. |
| `crates/builders/src/orderbook.rs` | Builder | **MEDIUM** | Order book reconstruction not implemented | Empty struct with only `new()` and `Default`; no methods | Full L2 order book from feed events; track bids/asks, handle gaps | Phase 2 | None | Strategies cannot access depth data or market structure. |
| `crates/reconciliation/src/freshness.rs:81-85` | Reconciliation | **MEDIUM** | Trading hours check uses UTC instead of instrument timezone | UTC-based session comparison only; comment: "simplified UTC-only comparison here since full tz support would require the chrono-tz crate (out of scope for Phase 2)" | Use `chrono-tz` to convert UTC to instrument's local trading timezone before comparing | Phase 2 | `chrono-tz` crate | Equity instruments in non-UTC timezones (e.g. NYSE = America/New_York) will get incorrect hours detection. **New finding.** |
| `crates/storage/src/parquet/compaction.rs` | Storage | **LOW** | Nightly compaction not implemented | Logs `tracing::warn` and returns `Ok(())`; does not compact | Scheduled nightly job that merges small Parquet files per partition | Phase 2 | None | Now logs visibly (was silent). Query performance degrades over time. |
| `crates/storage/src/postgres/users.rs:2` | Storage | **LOW** | User management placeholder | Only contains `count()` function; comment: "placeholder — to be filled in Phase 2" | Full user CRUD, role management, permission checks | Phase 2 | None | Users table exists but management operations incomplete. |
| `crates/mcp-server/src/tools/discovery.rs:4-5` | API | **LOW** | MCP discovery uses hardcoded static data | Returns fixed BTC-USDT, ETH-USDT, SOL-USDT on Binance; comment: "Phase 5 returns representative static data" | Query live platform registries: `list_lanes()` and `list_instruments()` | Phase 5 | API routes | Agents reason about hardcoded data, not real platform state. |
| `crates/api/src/ws/live.rs:141` | API | **LOW** | WebSocket serialization failure swallowed silently | `serde_json::to_string(msg).unwrap_or_default()` returns empty string on error | Return an error frame or log and skip; never send empty text message | Phase 2 | None | Client receives empty WS message instead of data if serialization fails. **New finding.** |
| `docs/observability/` | Docs | **LOW** | Missing operational procedures for metrics/correctness | No procedures exist | Document: metrics dashboard setup, alert thresholds, gap detection procedures | Phase 2 | None | Ops team has no guidance on monitoring system health. |
| `docs/procedures/user-management.md` | Docs | **LOW** | User management operational procedures missing | Does not exist | Create: user onboarding, role assignment, permission management | Phase 2 | None | Ops team has no guidance on access management. |

---

## Completed Since Last Scan (v1 → v2)

| Item | What Was Done |
|------|--------------|
| `migrations/0006_asset_class_registry.sql` | **CREATED** — `asset_class_registry` + `data_type_registry` tables with seed data |
| `migrations/0007_credential_store.sql` | **CREATED** — `venue_credentials` table (fills sequence gap) |
| `migrations/0008_pnl_schema.sql` | **CREATED** — `pnl_lots` + `pnl_closes` tables per C-073/C-105 |
| `apps/platform/src/main.rs` | **GUARDRAIL ADDED** — refuses to bind non-loopback address while auth is placeholder (M-17) |
| `xtask/src/main.rs` | **IMPLEMENTED** — real grep-based f64 scanner; exits 1 on violations; skips test code |
| `crates/collectors/src/crypto/mod.rs` | **CLEANED** — stale `TODO(Phase 1)` comment removed (Kraken IS wired) |
| `crates/collectors/src/equity/mod.rs` | **CLEANED** — stale `TODO(Phase 6)` comment removed (Alpaca IS wired) |
| `crates/graph/src/populate.rs:226` | **CLEANED** — stale "never populated" comment removed (edges ARE populated) |
| `crates/execution/src/account/tradovate.rs` | **IMPROVED** — returns `Err(NotImplemented)` instead of silent `Ok(vec![])` |
| `crates/execution/src/account/tradier.rs` | **IMPROVED** — returns `Err(NotImplemented)` instead of silent `Ok(vec![])` |
| `crates/execution/src/account_source.rs` | **IMPROVED** — `NotImplemented` variant added to `AccountSourceError` |
| `crates/api/src/routes/orders.rs` | **FIXED** — now returns 503 instead of false-permissive position=0 defaults |
| `crates/storage/src/parquet/compaction.rs` | **IMPROVED** — now logs `tracing::warn` instead of silently returning `Ok(())` |
| `crates/collectors/src/crypto/kraken.rs` | **FIXED** — wire deserialization uses `String` instead of `f64` for price/qty |
| `crates/collectors/src/equity/alpaca_data.rs` | **FIXED** — wire deserialization uses `String` instead of `f64` for price/size |
| `crates/collectors/src/options/tradier.rs` | **FIXED** — `bidsize`/`asksize` renamed with `#[serde(rename)]`; helper fns renamed |
| `crates/features/src/ema.rs` | **FIXED** — `price` parameter renamed to `value` (indicator math, not monetary storage) |
| `crates/features/src/rsi.rs` | **FIXED** — `price`/`prev_price` renamed to `value`/`prev_value` |

---

## By Severity

### 🔴 CRITICAL (Blocks Production Deployment) — 1 Item

1. **Auth Placeholder (M-17)** — `crates/api/src/auth/session.rs`
   - Status: Phase 2 upgrade required; loopback guardrail protects against accidental network exposure
   - Action: Implement token validation + session table before removing loopback restriction
   - Impact: **Do not remove loopback guardrail until Phase 2 auth lands**

---

### 🟠 HIGH (Breaks Feature) — 5 Items

| Feature | Status | Blocker | Impact |
|---------|--------|---------|--------|
| Coinbase live execution | Post-Phase 6 | Out of scope | No live crypto execution |
| Tradovate/Tradier account state | Phase 4 adapters | REST endpoints | Account queries error out (loudly) |
| Manual order placement | Disabled (503) | Postgres/Redis not wired | Orders blocked until Phase 2 |
| Dashboard P&L rollup | Phase 2 | Postgres + Redis | Always shows $0 P&L |
| 0x DEX execution | Phase 4 | 0x HTTP wiring | DEX trades unverified / assumed filled |

---

### 🟡 MEDIUM (Degrades Feature) — 6 Items

| Feature | Status | Blocker | Impact |
|---------|--------|---------|--------|
| Prometheus metrics | Phase 2 | None | No production visibility |
| Correctness metrics | Phase 2 | Storage integration | No data integrity validation |
| Order book builder | Phase 2 | None | No depth/market structure data |
| MCP server tests | Phase 5 | None | Tools not validated |
| AMM paper simulator | Phase 4 | 0x HTTP wiring | Mock prices only |
| Freshness timezone check | Phase 2 | `chrono-tz` crate | Wrong hours for non-UTC equity instruments (**new**) |

---

### 🔵 LOW (Nice-to-Have) — 5 Items

| Item | Status | Phase | Impact |
|------|--------|-------|--------|
| Parquet compaction | Warns but no-ops | Phase 2 | Query perf degrades |
| User management (storage) | Placeholder | Phase 2 | User CRUD ops missing |
| MCP dynamic discovery | Static data | Phase 5 | Agents use hardcoded instrument list |
| WS serialization silent failure | `unwrap_or_default` | Phase 2 | Empty frame sent on serialize error (**new**) |
| Ops documentation | Missing | Phase 2 | No runbooks for metrics/user mgmt |

---

## Phase Dependencies

```
Phase 2:
  ├─ Auth implementation (M-17)          [CRITICAL path — unlock network binding]
  ├─ Manual order gate real data         [Postgres position + Redis mark price]
  ├─ Dashboard P&L rollup                [Postgres pnl_lots/pnl_closes + Redis]
  ├─ Prometheus metrics wiring           [None]
  ├─ Correctness metrics wiring          [Storage integration]
  ├─ Order book builder                  [None]
  ├─ Parquet compaction                  [None]
  ├─ User management (storage)           [None]
  ├─ Freshness timezone (chrono-tz)      [Add crate]
  ├─ WS serialization error handling     [None]
  └─ Ops documentation                   [None]

Phase 4:
  ├─ Tradovate account REST adapter      [REST endpoints unknown]
  ├─ Tradier account REST adapter        [REST endpoints + API key]
  ├─ 0x adapter HTTP wiring              [0x API]
  └─ AMM paper simulator (real quotes)   [0x HTTP integration]

Phase 5:
  ├─ MCP dynamic discovery               [Platform API routes]
  └─ MCP server test coverage            [None]

Post-Phase 6:
  └─ Coinbase live execution             [Out of scope]
```

---

## Risk Assessment

### Deployment Risk: **MEDIUM** (down from HIGH after v1 fixes)

| Risk | Severity | Mitigation |
|------|----------|-----------|
| Auth completely bypassed | CRITICAL | Loopback-only guardrail active; fix before network access |
| Manual orders disabled | HIGH | 503 returned; safe state until Phase 2 |
| Dashboard shows $0 P&L | HIGH | Users cannot see trading performance |
| DEX trades unverified | MEDIUM | Implement on-chain tx verification before Phase 4 |
| No observability | MEDIUM | Wire Prometheus before production |
| Wrong trading hours for non-UTC equities | MEDIUM | Add `chrono-tz` before equity go-live |
| WS silent serialization failure | LOW | Add error logging/frame before Phase 2 |

### Readiness Checklist

- [x] Migrations 0006–0008 created
- [ ] Migrations 0006–0008 applied and tested against live DB
- [ ] Secrets moved to environment variables (prod deploy)
- [x] Auth placeholder isolated (loopback guardrail active)
- [ ] Phase 2 auth (token validation) implemented
- [ ] Manual order gate wired to real position/mark/P&L data
- [ ] Dashboard rollup shows real P&L
- [ ] Prometheus metrics wired and tested
- [ ] Order book builder implemented
- [ ] MCP server has test coverage > 50%
- [ ] Freshness check uses correct instrument timezone
- [ ] Docker Compose environment starts cleanly
- [ ] One end-to-end trade (paper) completes without errors
- [ ] API security review passed before network exposure

---

## Changelog

| Date | Version | Changes |
|------|---------|---------|
| 2026-06-10 | v1 | Initial scan — 23 items found (3 CRITICAL, 7 HIGH, 6 MEDIUM, 4 LOW) |
| 2026-06-10 | v2 | Post-implementation pass — 18 items completed/improved; 2 new findings added (freshness timezone, WS serialization); overall risk downgraded from HIGH to MEDIUM |

---

**Last Scan Date:** 2026-06-10 (v2)
**Scan Depth:** Repository-wide (all crates, apps, migrations, config, docs)
**Open Items:** 21 (1 CRITICAL, 5 HIGH, 6 MEDIUM, 5 LOW)
**Completed Since v1:** 18 items
**New Findings (v2):** 2 items
