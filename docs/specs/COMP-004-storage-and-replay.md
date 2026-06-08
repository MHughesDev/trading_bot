# COMP-004: Storage and Replay

**Status:** Draft
**Version:** 0.1
**ADR(s):** ADR-0004, ADR-0008, ADR-0009
**Success Conditions:** SC-3, SC-4

## 1. Purpose

Defines the storage architecture (split by access pattern across Postgres, ClickHouse, Parquet/object store, and Redis) and the backtest replay path. The raw normalized event archive is the ground truth of the system: written before any derivation, append-only, immutable. The backtest engine (`market_simulator`, an external repository) reads from this archive via Arrow IPC export. This component owns data durability, partitioning, batching, compaction, and the replay-clock guarantee.

## 2. Scope & Non-Goals

**In scope:**
- Storage split rationale per access pattern.
- Postgres responsibilities: transactional/core data.
- ClickHouse responsibilities: time-series analytics, `ReplacingMergeTree` dedup.
- Parquet/object store: raw event archive as ground truth, partitioning scheme.
- Redis/Valkey: latest-state cache (never source of truth for orders/fills).
- Append-only guarantee and why it exists.
- Write batching policy (10k events or 100ms).
- Nightly compaction job.
- Snapshots for expensive state.
- `market_simulator` integration: Arrow IPC export, adapter crate, run request shape.
- Replay via `available_time` as the simulated clock.
- Backtest fidelity caveat (bar-level vs tick-level).

**Not in scope (deliberate):**
- The `market_simulator` fill simulation internals — external repository, not owned here.
- OpenSearch/Meilisearch for document/event search — deferred post-v1.
- pgvector/Qdrant for semantic search — deferred post-v1 (arrives with news/social/AI features).
- Schema migrations tooling — implementation detail.
- Backup and disaster recovery — infrastructure concern.

## 3. Design

### 3.1 Storage Split by Access Pattern

| Purpose | Store | Rationale |
|---------|-------|-----------|
| Users, accounts, configs, permissions | PostgreSQL | Transactional, relational, low volume |
| Orders, fills, positions, audit ledger | PostgreSQL (+ event log) | ACID, critical correctness, low-to-medium volume |
| High-volume bars / trades / features | ClickHouse | Column-oriented, fast range scans, `ReplacingMergeTree` dedup |
| Raw normalized event archive (ground truth) | Object storage + Parquet | Append-only, immutable, cheap, ground truth for replay |
| Latest-state cache | Redis / Valkey | Sub-millisecond reads; never source of truth for money |
| Internal event fabric | NATS JetStream | Durable message bus; not long-term storage |

Do not force everything into Postgres; do not stand up five databases before they are needed. This split is the v1 allocation — each store is justified by a specific access pattern.

**Redis is a cache and live-state tool. It is never the source of truth for orders, fills, or positions.** If Redis loses data (restart, eviction), the system rehydrates from Postgres without data loss.

### 3.2 The Raw Event Archive Is Ground Truth

Raw normalized events are written to the Parquet/object store archive **before any derivation** (bars are derived from trades; features are derived from bars; none of that happens before the write). The archive is:

- **Append-only and immutable.** No event is ever modified. Late data emits a revision event (a new row), not an in-place update.
- **The data the `market_simulator` reads** when running a backtest simulation.
- **The insurance policy** when a derived store (ClickHouse, Postgres) is found to be wrong — rebuild from raw.

History is never recomputed a different way than live. Replay feeds the same raw normalized events through the same builder code. That is why "same strategy, same result" is a structural guarantee.

### 3.3 Partitioning

**ClickHouse:**
```sql
ORDER BY (instrument_id, available_time)
PARTITION BY toYYYYMM(available_time)
```
Optimized for the dominant backtest query: one instrument over a long date range. Contiguous reads, no universe scan.

**Parquet (object store):**
```
s3://bucket/events/{lane}/venue={venue_id}/instrument={instrument_id}/date={date}/
```
Example:
```
s3://bucket/events/market_bars/venue=coinbase/instrument=BTC-USDT/date=2026-06-08/
s3://bucket/events/market_trades/venue=alpaca/instrument=AAPL/date=2026-06-08/
```
Enables predicate pushdown via DataFusion for research queries across instruments.

**Postgres:**
Orders and fills partitioned by user/account. Volume is bounded and this is sufficient.

**Redis:**
Latest-state keys: `latest:{lane}:{instrument_id}` — never a warehouse scan for live-state reads.

### 3.4 Write Batching

Storage writers batch events before flushing:
- **Batch trigger:** 10,000 events accumulated, OR 100ms elapsed — whichever comes first.
- **Never** one database insert per event — this is a throughput and latency killer.
- Batches are written atomically per partition.

### 3.5 Nightly Compaction

Small Parquet files accumulate during normal operation (each batch flush creates a file). A nightly compaction job merges small files into large ones per partition (`lane/venue/instrument/date`). This maintains efficient read performance for backtest queries without requiring writes to hold open large files.

### 3.6 Snapshots for Expensive State

Do not replay from genesis for order-book state, indicator windows, or strategy state. The pattern:
- Persist a snapshot at interval N.
- On recovery, load the most recent snapshot.
- Replay only the deltas (events) since the snapshot.

Applies to: order-book reconstructors, indicator rolling windows, strategy `WorldState`, position/balance aggregates.

### 3.7 Replay and the available_time Clock

The replay engine (used by `market_simulator` via the adapter) advances a single simulated clock. The clock sorts events strictly by `available_time`. A strategy can only be handed event E when the simulated clock reaches `E.available_time`.

This is the structural mechanism for SC-4 (lookahead bias prevention). See DATA-003 §3.2–3.4 for the full invariant.

**Same builders, live and replay:** bar builders, feature engines, and order-book reconstructors are pure functions. Live path: they consume bus events. Replay path: they are fed recorded raw events from the Parquet archive in `available_time` order — through the same code. Identical results are structurally guaranteed, not hoped for.

### 3.8 market_simulator Integration

The backtest fill engine lives in `github.com/MHughesDev/market_simulator` — a separate repository. This component does not own fill simulation; it owns the data pipeline that feeds the simulator.

**This repo's responsibilities in the backtest flow:**

1. **Store raw normalized events** in the Parquet/ClickHouse archive (the storage layer above does this).
2. **Export data in Arrow IPC format** matching the `market_simulator` data contract.
3. **Provide `crates/market-simulator-adapter`** that translates between this repo's domain types and the simulator's contracts, submits run requests, and returns results in this repo's format.
4. **Surface backtest results** through the REST API and React UI.

```
This repo                                market_simulator (external)
────────────────────────────────────     ──────────────────────────
Parquet raw archive  ──Arrow IPC──▶      Run Request
Strategy definition  ──────────────▶     │
                                          ▼
                     ◀──TradeRecords──   Fill engine + metrics
REST /api/backtests  ◀──Metrics──────   (per-trade, PnL, drawdown)
```

**Do not build a fill simulator in this repo.** Any simulated execution in this repo is only for the paper trading live path; backtest fill simulation is entirely owned by `market_simulator`.

The `market_simulator` is stateless (no session, no persistence between invocations). Each run request is self-contained. This repo owns all persistent state.

### 3.9 Arrow IPC Export Schema

For MVP, historical data exported to `market_simulator` uses **1-minute OHLCV bars** as the primary granularity (matching the v1 collector data — see COMP-001 §3.11). The adapter is written to handle all supported granularities (1s bars, order-book snapshots when available), with 1m as the default.

The Arrow IPC file schema mirrors `BarPayload` fields plus `available_time` and `instrument_id`. The `market_simulator` adapter translates from `EventEnvelope<BarPayload>` to the simulator's typed Arrow schema.

### 3.10 Backtest Fidelity Caveat

Bar-level fills are less accurate than tick-level fills. Slippage modeling differs significantly. Bar-level backtest results are directional, not gospel. The `market_simulator` reports which fidelity level a run used and flags lower-accuracy runs explicitly. This is an honest system limitation at v1; tick-level fidelity becomes available when the corresponding collector lanes are populated.

## 4. Interfaces

**Parquet archive writer (internal):**
Receives `EventEnvelope<T>` from the bus. Batches. Flushes to object store on batch policy. No public API.

**ClickHouse writer (internal):**
Same consumption pattern. Writes to `ReplacingMergeTree` tables ordered by dedup key.

**Arrow IPC export:**
```rust
// In crates/market-simulator-adapter
pub fn export_bars_arrow(
    instrument_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<u8>, ExportError>;  // Arrow IPC bytes
```

**Backtest run request:**
```rust
pub async fn run_backtest(
    strategy_def: &StrategyDefinition,
    instrument_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<BacktestResult, SimulatorError>;
```

**REST endpoints for backtest:**
```
POST   /api/backtests         — submit run (strategy_id, instrument, date range)
GET    /api/backtests/{id}    — fetch result (status, metrics, per-trade records)
```

**Redis latest-state reads (internal):**
```
GET latest:{lane}:{instrument_id}
```

## 5. Dependencies

- DATA-001 — `EventEnvelope<T>` written to all storage paths.
- DATA-003 — `available_time` as the sort key for ClickHouse, Parquet, and the replay clock.
- COMP-001 — produces the raw normalized events that land in the archive.
- `market_simulator` (external) — backtest fill engine; this component provides the adapter.
- Apache Arrow / Parquet crates — `arrow`, `parquet`.
- `object_store` crate — S3-compatible object storage.
- ClickHouse Rust client.
- Redis/Valkey client.
- Postgres / `sqlx` — for orders, fills, positions, users.

## 6. Acceptance Criteria

- [ ] AC-1: A raw normalized event is written to the Parquet archive before any derived event (bar, feature) produced from it is written — Verified by: [—]
- [ ] AC-2: Replaying the Parquet archive through the same bar builder code produces identical `BarPayload` values to the live-path bar builder for the same input events — Verified by: [—]
- [ ] AC-3: Two `EventEnvelope` records with the same dedup key written to ClickHouse result in exactly one row after `ReplacingMergeTree` merge — Verified by: [—]
- [ ] AC-4: The storage writer never performs one database insert per event — it always batches — Verified by: [—]
- [ ] AC-5: The Arrow IPC export for a given `instrument_id` and date range includes all `BarPayload` events for that range stored in the Parquet archive, in `available_time` order — Verified by: [—]
- [ ] AC-6: A `market_simulator` run request submitted via the adapter returns `BacktestResult` metrics without any persistent state left in the simulator between runs — Verified by: [—]
- [ ] AC-7: A Redis cache miss (TTL expiry) for `latest:{lane}:{instrument_id}` causes the system to rehydrate from Postgres or ClickHouse without data loss or an error surfaced to the user — Verified by: [—]

## 7. Open Questions

Q-8: Retention policy for the raw Parquet archive — how long to keep raw events before tiering to cheaper cold storage? This affects storage cost and the window of full-fidelity backtest availability. Decision deferred; the system must be designed so the retention policy is configurable without architectural changes.
