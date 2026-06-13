# Phase 0 — Correctness & Safety Quick Wins

**Completion: 100% (5 / 5 tasks complete)**

**Goal:** Fix the cheap-to-fix hazards that affect live-trading correctness or
safety *now*, independent of the larger features. Every task here is small,
self-contained, and reduces real risk. **Addresses:** #10, #17, #19, #20, #27.

---

## Tasks

### ☑ 0.1 Freshness check honours instrument timezone — S — **safety**
**Addresses #17 (CL freshness).** The kill-switch staleness watchdog compares
**UTC** wall-clock to venue-local session strings, so during live US
equity/options hours (~14:00–21:00 UTC for a `09:30–16:00` NY session) it
believes the venue is *closed* and returns `StaleOutsideHours` — it will **not**
trip the kill switch on a genuine feed outage in the live session. The IANA
timezone is already on the struct (`domain::instrument::TradingSchedule.timezone`,
`instrument.rs:147-149`); the function ignores it.
- Add `chrono-tz`; parse `schedule.timezone` to a `Tz`; convert `now` with
  `with_timezone` before comparing to sessions. Handle midnight-crossing
  sessions and DST. Fall back to UTC (with a `warn`) if the tz string fails.
- **Files:** `crates/reconciliation/src/freshness.rs:76-97`,
  `crates/reconciliation/Cargo.toml`.
- **Verify:** replace the misleading UTC-labelled equity test with a real
  `America/New_York` case asserting a 14:00–21:00 UTC outage **trips** the
  switch, and an overnight gap does not.

### ☑ 0.2 0x adapter: fail-honest order status + correct decimals — S — **correctness**
**Addresses #10 (CL zerox:57,116).** `query_order` returns a hardcoded
`Filled` with dummy `qty=1`, empty instrument, `Side::Buy` — telling the
reconciler (`reconciliation/positions.rs` queries via `&dyn Broker`) a trade
settled when it may not have. Separately, token scaling hardcodes `10^18`
(`zerox.rs:76`), mis-sizing any non-18-decimal token (USDC/USDT = 6).
- `query_order`: honour `broker_order_id`; return `New`/`Unknown` (fail-honest)
  rather than a fabricated `Filled`; carry real `instrument_id`/`side`/`qty`.
- Replace `10^18` with per-asset token decimals from the instrument registry.
- Leave full on-chain submit/poll to **3.4 (deferred)** — this is the safe
  bug-fix pass only.
- **Files:** `crates/execution/src/venues/zerox.rs:76,104,114-127`.
- **Verify:** unit test that `query_order` does not report `Filled` without a
  real status, and that a 6-decimal token quantizes correctly.

### ☑ 0.3 MCP discovery queries real registries — S — **trust**
**Addresses #19 (CL discovery).** `list_lanes()` / `list_instruments()`
(`discovery.rs:26-71`) return hardcoded BTC/ETH/SOL on Binance + 2 lanes; this
path is **live** (dispatched at `mcp-server/src/lib.rs:54-60`), so an agent
reasons over fiction.
- Point `list_instruments` at the Postgres `instruments` table via `state.pg`
  (mirror `api/src/routes/assets.rs:35-42`), with an optional `asset_class`
  filter. Source asset classes from the `assets.rs` constant and lanes from the
  `streams.rs` constant. **Extract those two arrays into a shared
  `domain`/`config` constant** so API and MCP can't diverge.
- Optional follow-on: a `data_coverage` tool over `backtest::gaps`.
- **Files:** `crates/mcp-server/src/tools/discovery.rs`, shared constant module,
  `crates/mcp-server/src/lib.rs`.
- **Verify:** discovery returns the same instrument set the `/api/assets` route
  does (drives part of #21 / 5.5 test coverage).

### ☑ 0.4 WebSocket serialize failure is not swallowed — S
**Addresses #20 (CL ws/live.rs:141).** `serde_json::to_string(msg).unwrap_or_default()`
sends an **empty text frame** to the client on a serialization error.
- Log and skip the frame (or send a typed error frame); never emit empty text.
- **Files:** `crates/api/src/ws/live.rs:141`.
- **Verify:** unit test that a serialize error yields no empty frame.

### ☑ 0.5 Infra hygiene: gate unused services + fix Milvus port — S
**Addresses #27 (NF).** TigerGraph, Milvus, etcd, and minio all start by default
(`docker-compose.yml:66-126`) for components with no functional consumer
(see future-scope). Also a latent bug: `semantic` defaults to port 19530
(`lib.rs:21`) while compose/embedder expose 9091 — a connection would fail.
- Move `tigergraph`, `milvus`, `etcd`, `minio` behind a compose profile
  (e.g. `profiles: ["vector","graph"]`) so the default stack is lean.
- Reconcile the 19530/9091 discrepancy (pick the REST v2 port, fix both sides).
- Normalize the inconsistent phase taxonomy in code comments
  (Phase 2/4/5/7 / "P2-T12") to reference this plan's phase numbers or drop the
  stale labels.
- **Files:** `docker-compose.yml`, `crates/semantic/src/lib.rs:21`,
  `apps/embedder/src/main.rs:7`, assorted comment headers.
- **Verify:** `docker compose up` brings up only the active stack; documented
  `--profile` flag enables the vector/graph services.

---

## Definition of Done
The staleness watchdog fires correctly for non-UTC venues; the 0x adapter never
reports a fill it can't prove and sizes tokens by real decimals; MCP discovery
reflects real platform state; no empty WS frames; and the default container
footprint excludes the unused vector/graph services.
