# ADR: Canonical bar storage strategy (FB-AP-013)

**Status:** Accepted (2026-04-12)  
**Queue:** [`QUEUE.MD`](QUEUE.MD) **FB-AP-013** · follow-on **FB-AP-014**–**019**  
**Scope:** Where OHLCV history lives for charts, training, and backfill — **not** execution or risk.

---

## Context

The per-asset product epic requires **one authoritative time series per symbol** at **1-second** (or chosen) resolution, with REST backfill and live/WebSocket paths **converging** on the same schema (**FB-AP-014**–**016**). Before implementing tables and merge logic, we document **what exists today** and pick a **default direction** for new work.

---

## Current storage (as implemented)

| Location | Role | Bar / OHLC shape | Notes |
|----------|------|------------------|--------|
| **In-process `RollingBars`** (`data_plane/bars/rolling.py`) | Live loop: rolling OHLC from ticks | Per-symbol frame in memory | Ephemeral; source of features for `DecisionPipeline`. |
| **Redis** (`data_plane/storage/redis_state.py`) | Latest bar JSON per symbol | `BarEvent` JSON, TTL (`NM_REDIS_BAR_TTL_SECONDS`) | Fast “last bar”; **not** historical retention. |
| **QuestDB `canonical_bars`** (`QUESTDB_CANONICAL_BARS_DDL`, `QuestDBWriter.insert_bar`) | Durable canonical series (**FB-AP-014**) | `ts`, `symbol`, `interval_seconds`, OHLCV, `source`, `schema_version` | Partitioned by day; legacy `bars` table retained for existing deployments. |
| **Init pipeline Parquet** (`NM_ASSET_INIT_ARTIFACTS_DIR`, `orchestration/init_feature_artifacts.py`) | Bootstrap + feature training artifacts | Polars DataFrame → Parquet + manifest | **Per init job**; not the live canonical store. |
| **Backtesting / replay** | CSV or in-memory frames | Aligned with replay contracts | Separate from production ingest path. |

**Kraken** remains the **only** market-data source (REST + WS); Alpaca/Coinbase are execution venues, not bar sources (**AGENTS.md**).

---

## Requirements (from epic)

- **Long retention** for charts and walk-forward training without duplicating conflicting series.
- **Fast range reads** for UI (symbol + time window).
- **Sequential scans** for training pipelines.
- **Incremental append** with **idempotent merge** on `(symbol, ts)` (**FB-AP-015**).
- **Single logical schema** so REST backfill and WS-derived bars do not fork (**FB-AP-014**, **FB-AP-016**).

---

## Options considered

1. **QuestDB as primary canonical store**  
   - *Pros:* Already in stack; time-series SQL; good range queries; append-friendly.  
   - *Cons:* Must define **dedupe/upsert** policy (or `LATEST BY` / dedup at read); operational backup story (**RUNBOOKS.MD**).

2. **Parquet lake per symbol (object store or local `data/`)**  
   - *Pros:* Cheap bulk storage; training-friendly; easy to snapshot.  
   - *Cons:* Higher latency for interactive charts unless layered with cache; merge logic still required.

3. **Replace QuestDB for bars only** (e.g. dedicated TSDB or Postgres)  
   - *Pros:* Managed offerings in cloud.  
   - *Cons:* Extra moving part; migration cost; duplicates observability already on QuestDB for traces/events.

---

## Decision

**Extend the existing stack** rather than introduce a new primary TSDB for bars in the short term:

1. **Canonical logical schema** — Standardize on **one row per `(symbol, ts_bucket)`** with OHLCV + metadata (`source`, optional `interval_seconds`), documented in code and **FB-AP-014** (implementation slice). **Wall-clock UTC** for `ts` alignment across REST and WS.

2. **Primary write path for production hot path** — Use **QuestDB** as the **durable append target** for live and backfilled bars once merge rules exist, reusing the `bars` table **or** a successor table with an explicit **dedupe strategy** (see FB-AP-015). Avoid maintaining a second “truth” in Redis beyond **latest-bar cache**.

3. **Cold / training artifacts** — Keep **Parquet under `data/asset_init/`** (and similar) for **bootstrap and training exports**; **promotion** of trained artifacts is separate from the canonical bar series. Nightly or batch exports may **snapshot** from QuestDB or from merged Parquet — to be fixed when **FB-AP-036** wiring is defined.

4. **If QuestDB becomes a bottleneck** — Revisit **partitioning**, **materialized views** for coarser intervals (**FB-AP-017**), or a **read replica** / **managed TSDB**; add a new ADR row in **QUEUE.MD** only if we change the primary store.

**Non-goals for this ADR:** UI chart API shape (**FB-AP-024**), WebSocket aggregation details (**FB-AP-016**), exact DDL migration scripts — those belong to **FB-AP-014** and later rows.

---

## Consequences

- Implement **FB-AP-014** (schema + docs) and **FB-AP-015** (merge/dedupe) against **QuestDB + shared contracts**, not a greenfield database.

### FB-AP-015 (dedupe / idempotent merge)

- **Polars:** `data_plane.storage.merge_canonical_bars` — `dedupe_canonical_bars_last_wins`, `merge_canonical_bars_frames`; key ``(symbol, timestamp, interval_seconds)``, **last-write-wins**.
- **QuestDB:** `QuestDBWriter.insert_bar` **DELETE** then **INSERT** for the same key (idempotent replay).
- **Backup/restore** for QuestDB volumes remains the operator path for historical bars (**RUNBOOKS.MD**).
- Optional **QuestDB Cloud** or other managed TSDB is a **future** ADR (**FB-CONT-008**-style evaluation), not required to start **FB-AP-014**.

---

## References

- `data_plane/storage/schemas.py` — `bars` DDL  
- `data_plane/storage/questdb.py` — writer  
- `data_plane/storage/redis_state.py` — latest bar cache  
- [`PER_ASSET_OPERATOR.MD`](PER_ASSET_OPERATOR.MD) — init artifact layout  
- [`QUEUE.MD`](QUEUE.MD) **FB-AP-P0** data rows  
