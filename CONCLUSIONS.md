# Conclusions: Verdict on Each Complacency Item

**Date:** 2026-06-10
**Method:** Each item from `COMPLACENCY_LOG.md` was investigated with targeted code inspection. This file records the **verdict** — is the claim accurate, how serious is it really, and what to do.

> **Headline:** Of the 23 logged items, **3 were inaccurate** (collectors actually work, builders have tests, graph edges are populated), several "stubs" are **correctly-deferred future scope**, and the investigation surfaced **one NEW critical finding the original audit missed** (the money-safety CI check is a no-op showing false-green).

---

## Severity Re-Rank After Investigation

| Rank | Item | Original | Revised | Why |
|------|------|----------|---------|-----|
| 1 | Money-safety CI is a no-op (false green) | LOW | **CRITICAL** | A core invariant ("no f64 on price/size") is advertised as enforced by CI but the xtask just prints and exits 0. Worst kind of debt: looks safe, isn't. |
| 2 | Manual order gate uses position=0 | HIGH | **CRITICAL** | Not a conservative default — it's *false-permissive*. Gate thinks you hold nothing, permits over-leverage. |
| 3 | Auth placeholder | CRITICAL | **CRITICAL** | Confirmed: any non-empty Bearer token accepted. |
| 4 | Missing migrations 0006–0008 | CRITICAL | **HIGH** | Confirmed missing; DB-backed P&L + registry tests can't run. But P&L engine is in-memory, so it doesn't block paper trading on its own. |
| 5 | Hardcoded credentials | CRITICAL | **LOW** | Investigated: they're local-dev docker defaults; real secrets are env-injected via `resolve_secrets()`. Pattern is correct. |

---

## Verdicts (item by item)

### 🔴 Item 1 — Auth placeholder (`crates/api/src/auth/session.rs`)
**Claim accurate:** YES.
**Finding:** The `BearerToken` extractor checks for a `Bearer ` prefix and non-empty token, then accepts unconditionally. No user/session store exists to validate against. The code self-documents this as Phase 1 placeholder.
**Verdict:** CRITICAL, but contained. The mitigation is operational, not code: **do not bind the API to any non-loopback interface until M-17 is implemented.** Real fix is Phase 2 — validate token against a session table with constant-time comparison.
**Action:** Keep API on `127.0.0.1` only. Add a startup assertion that refuses to bind `0.0.0.0` while auth is in placeholder mode (cheap guardrail).

---

### 🔴 Item 2 — Missing migrations 0006–0008 (`migrations/`)
**Claim accurate:** YES (partially). Files present: 0001–0005, 0009, 0010. **0006, 0007, 0008 are genuinely absent.**
**Finding:**
- `pnl_lots` / `pnl_closes` are referenced in `crates/storage/src/pnl.rs:10` ("see migration 0008") and exercised by `crates/storage/tests/pnl_schema.rs` (an `#[ignore]`d test with real INSERTs).
- `asset_class_registry` is expected by `crates/storage/tests/registry_seed.rs` ("migration 0006 seeds correct row counts").
- **BUT** the `FifoEngine` itself is pure in-memory — paper trading P&L does not require the DB tables to exist. The missing migrations block the *DB-backed persistence path* and the ignored integration tests, not the in-memory engine.
**Verdict:** HIGH, not CRITICAL. Downgraded because it doesn't block paper trading; it blocks durable P&L and the integration test suite.
**Action:** Write `0006_asset_class_registry.sql`, `0008_pnl_schema.sql` (pnl_lots, pnl_closes). Determine whether 0007 was intentionally skipped or deleted — if nothing references it, renumber or document the gap.

---

### 🔵 Item 3 — Hardcoded credentials (`config/default.toml`)
**Claim accurate:** TECHNICALLY, but misleading.
**Finding:** The only credentials in the TOML are `trading:trading@localhost` — standard docker-compose dev defaults. Real venue/API secrets (Alpaca, Coinbase, Kraken, OpenAI) are **blank** in `.env.example` and injected at runtime. `crates/config/src/secrets.rs::resolve_secrets()` explicitly overrides `DATABASE_URL`, `CLICKHOUSE_URL`, `NATS_URL`, `REDIS_URL` from env vars. The pattern is correct.
**Verdict:** LOW. **Downgraded from CRITICAL.** This is not a real secret leak; it's a local-dev convenience default.
**Action:** Optional hygiene — add a comment in `default.toml` clarifying these are dev-only, and ensure production deploys set the env overrides. No urgent change.

---

### 🟢 Item 4 — Crypto collector "not wired" (`crates/collectors/src/crypto/mod.rs`)
**Claim accurate:** **NO — claim is WRONG.**
**Finding:** The `mod.rs` carries a stale `TODO(Phase 1)` comment, but `crates/collectors/src/crypto/kraken.rs` is a **complete 266-line production collector**: WS connection to `wss://ws.kraken.com/v2`, trade parsing, normalization to `EventEnvelope<TradePayload>`, reconnect/backoff, gap detection, quarantine. `apps/collector-crypto/src/main.rs` instantiates and runs it.
**Verdict:** NOT A GAP. The complacency item was a false alarm caused by a misleading comment on the `mod.rs`.
**Action:** Delete the stale `TODO(Phase 1): crypto collector wiring` comment so it stops scaring auditors.

---

### 🟢 Item 5 — Equity collector "not wired" (`crates/collectors/src/equity/mod.rs`)
**Claim accurate:** **NO — claim is WRONG.**
**Finding:** Same story. `crates/collectors/src/equity/alpaca_data.rs` is a complete 361-line collector: WS to Alpaca IEX, auth, trade parsing, `TrustTier::Regulated`, gap detection, reconnect, unit tests. `apps/collector-equity/src/main.rs` spawns multi-symbol collectors.
**Verdict:** NOT A GAP. (Note: trading-hours/halt awareness is a separate refinement, but ingestion works.)
**Action:** Delete the stale `TODO(Phase 6)` comment. Track hours/halt-awareness separately if still wanted.

---

### 🟡 Item 6 — Coinbase live adapter stub (`crates/execution/src/coinbase.rs`)
**Claim accurate:** YES — it's a one-line stub.
**Finding:** Entire file is `//! Coinbase live broker adapter — stub (post-Phase 6 scope).`
**Verdict:** ACCEPTABLE / EXPECTED. Not needed for current scope — Alpaca covers equity execution, paper sim covers crypto. Coinbase *live* crypto execution is explicitly post-Phase 6.
**Action:** None now. Leave as documented future scope.

---

### 🟡 Item 7 — Tradovate account adapter stub (`crates/execution/src/account/tradovate.rs`)
**Claim accurate:** YES — `fetch_balances`/`fetch_positions` return `Ok(vec![])`.
**Verdict:** ACCEPTABLE / EXPECTED. Futures venue, Phase 4 scope. Returning empty is safe (no false data); a consumer sees "no account state" rather than wrong state.
**Action:** None now. Phase 4. Consider returning an explicit `Err(NotImplemented)` instead of empty vecs so callers can't mistake "no positions" for "empty account."

---

### 🟡 Item 8 — Tradier account adapter stub (`crates/execution/src/account/tradier.rs`)
**Claim accurate:** YES — parses `token:account_id` then returns `Ok(vec![])`.
**Verdict:** ACCEPTABLE / EXPECTED. Phase 4. Same note as Tradovate: empty-vec-as-stub is mildly risky if a caller treats it as truth.
**Action:** Phase 4. Same `Err(NotImplemented)` suggestion.

---

### 🟡 Item 9 — 0x DEX adapter `query_order` stub (`crates/execution/src/venues/zerox.rs`)
**Claim accurate:** YES for `query_order` (always returns `Filled`); **NO for the whole adapter** — `submit()` makes real HTTP calls to the 0x quote API.
**Finding:** Hybrid. Quote-fetching is real; on-chain status polling is stubbed as atomically-filled.
**Verdict:** ACCEPTABLE for current scope (0x live is Phase 4). The always-`Filled` assumption is fine for atomic swaps but must be replaced with real on-chain tx-status checks before live DEX trading.
**Action:** Phase 4 — implement real tx-receipt polling in `query_order`.

---

### 🟡 Item 10 — AMM paper simulator skeleton (`crates/execution/src/paper/amm_swap.rs`)
**Claim accurate:** YES — Phase 1 skeleton; takes a caller-supplied `FirmQuote` and applies a `price_impact_bps` model.
**Finding:** It's a *working* simulator with a parameterized price-impact model; it just doesn't fetch live 0x quotes (those are mocked/caller-supplied by design).
**Verdict:** ACCEPTABLE / EXPECTED. Correct phase-appropriate design.
**Action:** Phase 4 — wire to 0x `/price` for real slippage.

---

### 🔴 Item 11 — Manual order gate uses hardcoded zeros (`crates/api/src/routes/orders.rs:109-118`)
**Claim accurate:** YES, and **more serious than logged.**
**Finding:**
```rust
let ctx = GateContext::for_manual_order(
    Decimal::ZERO,      // current position — TODO(P2+): fetch from DB
    None,               // mark price   — TODO(P2+): fetch from Redis
    Decimal::new(1, 2), // 0.01 default tick
    Decimal::new(1, 3), // 0.001 default lot
    Decimal::ZERO,      // realized P&L — TODO(P2+)
    true,               // instrument active — TODO(P2+)
    0, 0,
);
```
`position=0` is **false-permissive**, not conservative: the position-limit check believes you hold nothing and will therefore approve larger orders than your real exposure allows. `mark_price=None` may bypass price-sanity. `instrument_active=true` lets halted instruments through.
**Verdict:** **CRITICAL — upgraded.** This silently undermines the position-limit and price-sanity guarantees for the *manual order* path specifically (strategy-runtime orders build their own real context, so the automated path is unaffected — verify this).
**Action:** Before enabling manual order placement against a funded account: wire real position (Postgres), mark price (Redis), realized P&L (ledger), and active-status (instrument registry). Until then, **disable the manual order route** or gate it behind paper-only mode.

---

### 🟡 Item 12 — Dashboard returns zero P&L (`crates/api/src/routes/dashboard.rs:47`)
**Claim accurate:** YES — returns an all-zero `RollupResponse` with a TODO.
**Finding:** The `FifoEngine` compute logic exists and is unit-tested; it's simply not wired to this route (which would need to read `pnl_lots`/`pnl_closes` from Postgres + marks from Redis).
**Verdict:** MEDIUM. Cosmetic/UX, not a safety issue — users see $0 regardless of real performance. Depends on Item 2 (migrations) for the DB tables.
**Action:** Phase 2 — wire the rollup once migration 0008 lands.

---

### 🟡 Item 13 — Prometheus metrics are no-ops (`crates/observability/src/metrics.rs`)
**Claim accurate:** YES — `increment_published/quarantined/gap_detected` are empty bodies. No registry, no exporter.
**Verdict:** MEDIUM. No production observability into message rates, quarantine, or gaps. Doesn't affect correctness, only visibility.
**Action:** Phase 2 — wire to a real Prometheus registry + `/metrics` endpoint before any production (non-paper) run.

---

### 🟡 Item 14 — Correctness metrics are no-ops (`crates/observability/src/correctness.rs`)
**Claim accurate:** YES — `record_consumer_lag/quarantine_rate/reconciliation_divergence` are empty.
**Verdict:** MEDIUM. Same as Item 13 — no runtime visibility into pipeline integrity/replay divergence.
**Action:** Phase 2 — implement alongside Prometheus wiring.

---

### 🟢 Item 15 — Builders has zero tests (`crates/builders/`)
**Claim accurate:** **NO — claim is WRONG.**
**Finding:** `crates/builders/src/bars.rs` has 3 passing tests (windowing, window-close, late-trade revision). The *bar builder* — the part that matters for live/replay parity — is tested.
**Verdict:** NOT A GAP for bars. (The orderbook builder is a separate, genuinely-unimplemented item — see Item 18.)
**Action:** None for bars. Remove this from the debt list.

---

### 🟡 Item 16 — MCP server has zero tests (`crates/mcp-server/`)
**Claim accurate:** YES — no `#[test]`/`#[cfg(test)]` in the crate, though tools (discovery, authoring, lifecycle) have real logic.
**Verdict:** MEDIUM. MCP is an agent-authoring surface, not in the money path. Untested but low blast radius.
**Action:** Add tests when MCP features are next touched (Phase 5). Not a trading blocker.

---

### 🔵 Item 17 — MCP discovery uses static data (`crates/mcp-server/src/tools/discovery.rs`)
**Claim accurate:** YES, and **intentional/documented** (Phase 5 returns representative static data).
**Verdict:** LOW. By design. Agents reason over representative data until live platform queries are wired.
**Action:** Phase 5 — query live registries.

---

### 🟡 Item 18 — Order book builder not implemented (`crates/builders/src/orderbook.rs`)
**Claim accurate:** YES — 18-line skeleton: `struct OrderBookBuilder; fn new()`. Comment: "wired up in Phase 2."
**Verdict:** MEDIUM, and correctly Phase 2. Strategies needing L2 depth can't run yet; trade/bar-based strategies are unaffected.
**Action:** Phase 2. Only blocks depth-based strategies.

---

### 🔵 Item 19 — Parquet compaction stub (`crates/storage/src/parquet/compaction.rs`)
**Claim accurate:** YES — `compact_partition` returns `Ok(())` doing nothing.
**Finding:** **Subtle risk:** it's scheduled to be invoked in Phase 2 but silently no-ops, so it will *appear* to run while doing nothing — small-file accumulation continues unnoticed.
**Verdict:** LOW now, but flag the silent-success behavior.
**Action:** Phase 2 — implement, or make it log "compaction not implemented" so a scheduled run isn't mistaken for a successful merge.

---

### 🔵 Item 20 — User management placeholder (`crates/storage/src/postgres/users.rs`)
**Claim accurate:** **PARTIALLY — claim overstates it.** The doc-comment says "placeholder," but `count()` is implemented (`SELECT COUNT(*) FROM users`).
**Verdict:** LOW. Minimal but functional; not empty.
**Action:** Flesh out CRUD when auth (Item 1) is implemented — they're naturally paired (Phase 2).

---

### 🔵 Item 21 — Money-safety xtask stub (`xtask/src/main.rs`) → ⚠️ **ESCALATED TO CRITICAL**
**Claim accurate:** YES on the stub, **but the real finding is bigger than logged.**
**Finding:** `xtask check-money-f64` is `println!("...stub...")` then exits 0. CI (`.github/workflows/ci.yml`) runs a job named **"Money safety (no f64 on price/size)"** that invokes this xtask. **Because the stub exits 0, the job shows GREEN while enforcing nothing.** The "no float money" invariant — one of the system's headline guarantees — is **not actually checked by CI.**
**Verdict:** **CRITICAL.** This is the most dangerous item found: a safety invariant advertised as enforced, displaying false-green, with zero actual coverage. An f64 could be introduced on a `Price`/`Size` field and CI would not catch it.
**Action:** Implement the scanner (even a grep-based `rg 'f64' ` over price/size-bearing modules is better than nothing), OR rename the CI job to "Money safety (NOT YET ENFORCED)" so the green check stops lying. Prefer implementing it. **Verify current code has no f64-on-money regressions in the meantime.**

---

### 🟢 Item 22 — Graph edges never populated, M-6 (`crates/graph/src/populate.rs`)
**Claim accurate:** **NO — comment is STALE.**
**Finding:** Lines 226–272 actively populate all three edge types: `INSTRUMENT_AT_VENUE` (from instruments), `VENUE_SUPPORTS_ASSET_CLASS` (from venues), `STRATEGY_FOR_ASSET_CLASS` (from strategies).
**Verdict:** NOT A GAP. The "never populated" comment is leftover from before the M-6 fix.
**Action:** Delete/Update the stale comment.

---

### 🔵 Item 23 — `#[ignore]`d integration tests (repo-wide)
**Claim accurate:** YES — 8 ignored tests across 6 files, all with legitimate external deps:
- Semantic (1): "requires Milvus + OPENAI_API_KEY"
- Storage (5): "requires a running migrated Postgres DB"
- Graph (2): "requires a running TigerGraph instance"
**Verdict:** LOW / by-design. These are correctly gated on real infrastructure. **However**, they are the *only* tests that would validate the migrations (Item 2) and the TigerGraph/Milvus port fixes from the prior audit — so they're not optional forever.
**Action:** Stand up docker-compose and run the ignored suite **once** before declaring the storage/graph/semantic layers trustworthy. This single step closes the largest verification gap in the system.

---

## Bottom Line

### What actually blocks paper trading
1. **Item 21 (money-safety CI false-green)** — fix or relabel; verify no current f64-on-money.
2. **Item 11 (manual-order gate false-permissive)** — disable manual orders or wire real context.
3. **Item 1 (auth)** — keep API on loopback only.
4. **Item 2 (migrations)** — needed for durable P&L + to run the ignored integration tests.

### What does NOT block paper trading (corrected myths)
- Collectors work (Items 4, 5) — Kraken + Alpaca are fully functional.
- Bar builder is tested (Item 15).
- Graph edges are populated (Item 22).
- Credentials are dev-defaults with proper env override (Item 3).

### Correctly-deferred future scope (no action now)
- Coinbase live, Tradovate, Tradier, 0x live, AMM live wiring (Items 6–10) — Phase 4 / post-Phase 6.
- Dashboard rollup, metrics, order-book builder, user CRUD (Items 12–14, 18, 20) — Phase 2.
- MCP tests + dynamic discovery (Items 16, 17) — Phase 5.

### The one thing to do first
**Stand up `docker compose up` and run the 8 `#[ignore]`d tests.** That single action validates the migrations, the TigerGraph/Milvus port fixes, and the DB-backed P&L path all at once — it's the highest-leverage verification step available and would convert most of the "unverified" risk into either green checks or concrete bugs to fix.
