# Production-Readiness — Stub, Placeholder & Tech-Debt Master Plan — Set E

**Completion: 0% (0 / 28 primary tasks; 3 deferred-by-design — see Progress Log)**

> Future-scope items (TigerGraph, Milvus) live in [future-scope.md](future-scope.md)
> and are **not** counted in the primary total.

## Overview

Set E consolidates every unfinished, stubbed, stale, or placeholder code path
found in a repository-wide sweep (2026-06-13) into one sequenced plan. It
supersedes and absorbs the tracking table in `/COMPLACENCY_LOG.md` (v2, 21 open
items) and adds findings that log missed:

- The frontend ships a **complete cookie-session auth UX** (`authApi`,
  `useAuthStore`, `LoginPage`, `SignUpPage`) calling `/auth/*` routes that **do
  not exist** in the Rust API — the login flow is dead today and the SPA falls
  back to a hardcoded `dev-local` bearer token.
- `BearerToken` identity is represented **three different ways** across routes
  (UUIDv5-of-token in backtests, raw token string in `streams.rs:48`,
  `Uuid::nil()` DEV_USER in automations) — unifying it is itself cross-cutting.
- Frontend strategy-builder gaps (`toDefinition.ts`): SMA/ATR, AND/OR
  conditions, exit rules, and percent-of-equity sizing are accepted in the UI
  but warned/dropped on save.
- `PercentOfBalance` / `RiskUnit` sizing are parse-only in the **shared**
  `strategy-runtime` (non-executable live *and* in replay).
- Hot-path **stage-3 strategy evaluation is a `None` placeholder** — no intents
  are ever produced by the ring pipeline.
- `crates/graph` (TigerGraph) and `crates/semantic` (Milvus) compile but have
  **zero functional consumers**, yet their heavyweight containers run by default.

Two findings the log under-rated are promoted here to **safety/correctness**:

- `reconciliation/freshness.rs` compares **UTC wall-clock** against venue-local
  session times, so the kill-switch staleness watchdog **silently fails to fire
  during live US equity/options hours** — the highest-consequence window.
- `venues/zerox.rs` `query_order` returns a **hardcoded `Filled`** with dummy
  values, telling the reconciler a DEX trade settled when it may not have; and
  its `10^18` token scaling mis-sizes any non-18-decimal token (USDC/USDT).

## Guiding constraints

- **ADR-0008 (live = replay).** Any sizing/indicator/strategy change must run
  through the one shared codepath so live and backtest stay byte-identical.
  Sizing cannot ship live-only.
- **ADR-0007 / ADR-0010 (one frozen canonical strategy format, three front
  doors).** AND/OR conditions and exit nodes require an **additive
  definition-version bump** (v1.5), following the precedent already set by the
  v1.5 universe nodes in `NodeKind`. SMA/ATR and `PercentOfBalance` do **not**.
- **Security gate.** The non-loopback bind guard (`apps/platform/src/main.rs`)
  is the single line protecting the placeholder auth. It is removed **last**,
  only after end-to-end session auth is verified and TLS/CSRF are in place.
- **Fail-honest over fail-silent.** Where a real implementation is deferred,
  return a clear error (`NotImplemented`, 503) — never fabricate success.

---

## Phase Summary

| Phase | File | Label | Tasks | Completion | Goal |
|-------|------|-------|-------|------------|------|
| 0 | [phase-0.md](phase-0.md) | Correctness & safety quick wins | 5 | 0% | fix the live-trading hazards that are cheap to fix now |
| 1 | [phase-1.md](phase-1.md) | Authentication & multi-user | 7 | 0% | real session auth; unlock network binding |
| 2 | [phase-2.md](phase-2.md) | Live P&L, manual orders & shared data | 3 | 0% | real $ rollup + paper manual orders via real risk context |
| 3 | [phase-3.md](phase-3.md) | Execution venue adapters | 4 | 0% | Tradier/Tradovate account sources; real DEX quotes |
| 4 | [phase-4.md](phase-4.md) | Strategy surface & sizing | 4 | 0% | SMA/ATR, %-of-balance sizing, scanner, v1.5 format |
| 5 | [phase-5.md](phase-5.md) | Data pipeline & observability | 5 | 0% | metrics, compaction, order book, hot-path strategies |
| FS | [future-scope.md](future-scope.md) | Future scope (not counted) | — | — | TigerGraph / Milvus — keep code, gate infra |

**Recommended sequencing:** 0 → 1 → 2 makes the platform safe-to-run and
network-deployable with real P&L. 3 → 4 → 5 broadens venues, the strategy
surface, and operability. Phase 0 can land immediately and independently.

---

## Item → Phase Map

Every finding from the sweep and its home. `CL` = row in `/COMPLACENCY_LOG.md`;
`NF` = new finding from the 2026-06-13 sweep.

| # | Source | Item | Phase · Task |
|---|--------|------|--------------|
| 1 | CL | Placeholder bearer auth accepts any token (M-17, CRITICAL) | 1.3 |
| 2 | CL | Platform loopback-bind guardrail | 1.7 |
| 3 | NF | `/auth/*` routes don't exist; frontend auth UX is dead | 1.2 |
| 4 | CL | `storage/postgres/users.rs` placeholder (count only) | 1.1 |
| 5 | CL+NF | Automations scoped to `DEV_USER` nil; `streams.rs` raw-token id | 1.4 |
| 6 | NF | Frontend `dev-local` hardcoded bearer token | 1.5 |
| 7 | CL | Coinbase live broker stub (HIGH, post-Phase 6) | 3 (deferred 3.x) |
| 8 | CL | Tradier account adapter `NotImplemented` (HIGH) | 3.1 |
| 9 | CL | Tradovate account adapter `NotImplemented` (HIGH) | 3.2 |
| 10 | CL | 0x `query_order` hardcoded `Filled` + tx-hash stub (MED) | 0.2 / 3 (deferred) |
| 11 | CL | AMM paper sim uses mocked `FirmQuote` (MED) | 3.3 |
| 12 | CL | Manual order placement disabled (503) (HIGH) | 2.3 |
| 13 | CL | Dashboard live P&L rollup all-zero (HIGH) | 2.2 |
| 14 | CL | Prometheus metrics no-op stubs (MED) | 5.1 |
| 15 | CL | Correctness metrics no-op stubs (MED) | 5.1 |
| 16 | CL | `OrderBookBuilder` empty struct (MED) | 5.3 |
| 17 | CL | Freshness UTC-only hours check (MED — promoted to safety) | 0.1 |
| 18 | CL | Parquet compaction no-op (LOW) | 5.2 |
| 19 | CL | MCP discovery static fabricated data (LOW) | 0.3 |
| 20 | CL | WS serialize silent failure `unwrap_or_default` (LOW) | 0.4 |
| 21 | CL | MCP server has zero tests (MED) | 5.5 |
| 22 | NF | `PercentOfBalance` / `RiskUnit` sizing parse-only | 4.2 / 4.5 |
| 23 | NF | Builder SMA/ATR indicators unsupported | 4.1 |
| 24 | NF | Builder AND/OR conditions + exit rules dropped (format bump) | 4.4 |
| 25 | NF | `ScannerPanel` strategy-change stub (no tile population) | 4.3 |
| 26 | NF | Hot-path stage-3 strategy instance is a `None` placeholder | 5.4 |
| 27 | NF | Milvus port discrepancy (19530 vs 9091) + always-on infra | 0.5 / FS |
| 28 | NF | TigerGraph unused, always-on infra | FS |
| 29 | CL | Config secrets (`config/default.toml`) + ops docs (LOW) | future-scope §3 |

---

## Open decisions (need a human)

These gate design and are surfaced from the cluster research. Resolve before the
relevant phase starts.

1. **Session vs JWT (Phase 1).** Evidence strongly favours **opaque
   server-side sessions in a cookie** (`withCredentials: true`, `/auth/touch`
   sliding expiry, `/auth/logout` revocation). Confirm — it decides whether the
   frontend change is "delete the dev-token interceptor" or "rewrite token
   storage," and whether CSRF protection is needed.
2. **Single-operator vs multi-tenant (Phase 1).** Schema is built multi-user
   (per-user `venue_credentials`, backtest/automation `user_id`) but comments say
   "single local operator." Decides registration policy (open / invite / seeded)
   and whether cross-user isolation is a hard security requirement.
3. **Legacy data migration (Phase 1).** Existing automations/backtests under
   `Uuid::nil()` and the old UUIDv5-of-token identity orphan once real `user_id`s
   land. **Armed live automations** under nil could detach from any owner — needs
   an explicit reassignment migration, not a silent "treat nil as legacy."
4. **Mark source of truth (Phase 2).** Which Redis lane is the canonical mark
   for risk/rollup (trade-last vs a dedicated mark lane), and the staleness
   policy when a mark is missing/expired (skip vs reject vs surface).
5. **Live manual orders (Phase 2).** Confirm **paper-only** is acceptable for
   the first cut (live needs a broker adapter + position feed that don't exist).
6. **Tradovate credential format (Phase 3).** Does the decrypted blob carry a
   pre-exchanged bearer token or `username:password:appId` requiring a
   `/auth/accesstoken` exchange inside the adapter? Drives S-vs-M effort.
7. **0x execution model (Phase 3).** Should `submit` broadcast on-chain in this
   crate at all, or remain a quote-only adapter handing tx-data to an external
   signer? Real on-chain submit+poll needs an off-repo signer/RPC component.
8. **Coinbase scope (Phase 3).** Confirm live Coinbase execution stays
   out-of-scope before investing in ES256 JWT signing (`jsonwebtoken` dep).
9. **Strategy-definition version bump (Phase 4).** Approve a **v1.5** bump for
   exit nodes + AND/OR (with a validator migration story) before touching the
   frozen grammar. SMA/ATR and `PercentOfBalance` need no bump.
10. **Equity definition parity (Phase 4).** Confirm live (`free_collateral` vs
    `cash` vs `equity`) and the backtest SDK use the **identical** equity figure
    for `PercentOfBalance` sizing.
11. **One universe feed (Phase 4).** Decide up front that ScannerPanel discovery
    and future Automations pipelines share one universe-feed path, not two.
12. **Hot-path strategy hand-off (Phase 5).** arc-swap snapshot vs SPSC command
    channel vs a real `PipelineFactory` replacing `NoopPipelineFactory` — and
    whether the API or the demand-manager owns instance lifecycle.

---

## Progress Log

Update this table and the per-phase completion headers as tasks land.

| Date | Phase | Task | Note |
|------|-------|------|------|
| 2026-06-13 | — | plan | Set E created from the repo-wide stub/stale sweep + 5-cluster deep research; consolidates COMPLACENCY_LOG v2 (21 items) + 8 new findings. **Deferred-by-design:** 3.4 (0x on-chain submit/poll — off-repo signer/RPC), 3.5 (Coinbase live — out of scope, ES256 JWT), 4.5 (RiskUnit — blocked on exit nodes), plus the future-scope appendix (TigerGraph, Milvus). |
