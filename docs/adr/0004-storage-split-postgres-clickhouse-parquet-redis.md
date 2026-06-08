# ADR-0004: Storage Split — Postgres, ClickHouse, Parquet, Redis

**Status:** Accepted
**Date:** 2026-06-08
**Deciders:** Platform team

## Context

The platform stores fundamentally different kinds of data with fundamentally different access patterns:

- **Transactional records** (users, accounts, permissions, orders, fills, positions, strategy definitions) are relational, require ACID guarantees, are accessed by row/ID, and are updated or inserted at low to moderate volume. Analytical queries over this data are rare and not latency-sensitive.
- **Time-series analytics data** (OHLCV bars, normalized trades, computed features) arrives at high volume (hundreds of thousands of rows per day per instrument), is almost always queried by time range over a single instrument, and is never updated in place. Column-oriented storage with time-ordered compression is orders of magnitude more efficient than a row store for these access patterns.
- **Raw normalized events** (the ground truth archive) must be written append-only, immutable, and cheaply retained for long periods. They are read in bulk for backtest data export, not queried by single-row lookup. Object storage (S3-compatible) with Parquet columnar format and date-partitioned file layouts is the correct physical representation.
- **Latest-state reads** (current price of an instrument, latest position snapshot, subscription state, rate-limit counters) require sub-millisecond reads and are needed constantly by the UI and risk gate. A key-value cache is the right tool; a database scan is wrong.

Using a single database engine for all four patterns forces compromises: a row store used as a time-series store either cannot hold the data volume or becomes extremely slow; an object store used as a transactional store lacks ACID; a cache used as source of truth for orders is dangerous.

## Decision

Split storage across four systems, assigned strictly by access pattern:

| Access Pattern | System | What Lives Here |
|----------------|--------|-----------------|
| Transactional / relational (ACID, row access) | **PostgreSQL** (`sqlx`) | Users, accounts, permissions, orders, fills, positions, strategy definitions, audit ledger |
| High-volume time-series analytics (column scan, time range) | **ClickHouse** | OHLCV bars, normalized trades, computed features |
| Immutable raw event archive (bulk export, long retention) | **Parquet + object storage** (arrow/parquet/datafusion) | Raw normalized events partitioned by `lane/instrument/date` |
| Latest-state cache (key-value, sub-millisecond) | **Redis/Valkey** (`redis`) | Latest prices, position snapshots, subscription state, rate-limit counters |

Redis is explicitly a cache — **never** the source of truth for orders or fills.

## Rationale

Each store is the best tool for its access pattern:

**PostgreSQL** is the correct home for relational, transactional data. SQLx provides compile-time-checked queries with no ORM magic, giving full SQL control. Postgres's ACID guarantees are required for order/fill records, where partial writes are unacceptable.

**ClickHouse** is purpose-built for high-volume, time-ordered, append-heavy columnar analytics. Its `ReplacingMergeTree` engine provides eventually-consistent deduplication (required for idempotent storage writes on JetStream redelivery). Its `ORDER BY (instrument, available_time)` with monthly partitioning yields contiguous on-disk reads for the dominant backtest query pattern (one instrument, long time range).

**Parquet on object storage** is the ground-truth archive. Writing raw normalized events here — before any derivation — means every derived store (ClickHouse bars, Redis snapshots, Postgres aggregates) can be rebuilt from scratch if found to be wrong. Object storage is cheap, durable, and scales to arbitrary retention without operational tuning. DataFusion provides pushdown predicate scanning for research queries across the archive.

**Redis/Valkey** answers "what is the latest state of X" in microseconds, which no database can match at query time. The key schema `latest:{lane}:{instrument}` means a single O(1) lookup returns the latest price or snapshot pointer. Redis eviction is acceptable because these are derived/cached values — the authoritative record is always in Postgres or the Parquet archive.

## Consequences

**Positive:**
- Each store operates well within its design envelope; no store is abused for a use case it handles poorly.
- ClickHouse scan performance for backtest data export is far superior to Postgres for time-series ranges.
- Parquet archive provides ground truth for replay and disaster recovery without a database license.
- Redis sub-millisecond reads keep the risk gate and UI responsive without warehouse queries on hot paths.
- Storage failures degrade gracefully: a storage writer falling behind does not block execution (spec §failure posture: "Degrade — keep trading; backfill later").

**Negative:**
- Four storage systems means four operator concerns: backup policies, version upgrades, connection pool tuning, and monitoring for each.
- Cross-store queries (e.g., join orders from Postgres with features from ClickHouse) require application-side assembly or a query federation layer. No single SQL query spans stores.
- Storage writers must handle each store's write semantics: batching for ClickHouse and Parquet, idempotency for all four.

**Neutral:**
- Post-v1 document/event search (OpenSearch/Meilisearch) and semantic search (pgvector/Qdrant) are explicitly deferred. They arrive with news/social/AI features.
- Storage writers batch at 10,000 events or 100ms (whichever comes first) to avoid per-event insert overhead. A nightly compaction job consolidates small Parquet files into large ones.
- Redis is always populated from a durable source on startup; there is no "warm up" period where the cache is the only available state.

## Alternatives Considered

### Option A: Postgres for Everything
Store time-series data in Postgres alongside relational data. Simple: one database to operate.

Not chosen because: Postgres row storage is poorly suited to high-volume time-series inserts and range scans. At the volume of a live trading platform, Postgres time-series tables become slow and large. Table partitioning helps but does not close the gap with a column store designed for this access pattern.

### Option B: ClickHouse for Everything (Including Orders/Fills)
Use ClickHouse as the primary store and eliminate Postgres.

Not chosen because: ClickHouse is an OLAP store, not an OLTP store. It has no row-level locking, no true ACID transactions, and no foreign key constraints. Storing orders and fills in ClickHouse risks data integrity problems under concurrent writes and partial failures. Order records require exactly the guarantees Postgres provides.

### Option C: Single Time-Series Database (TimescaleDB or QuestDB)
Use TimescaleDB (Postgres extension) or QuestDB as a unified store for relational + time-series data.

Not chosen because: TimescaleDB adds complexity on top of Postgres without matching ClickHouse's read performance for the dominant backtest scan pattern. QuestDB has excellent time-series performance but a less mature ecosystem and less operational familiarity. ClickHouse is a clearer, more proven choice for the time-series tier at this scale.

## References

- [spec/07-storage-and-replay.md](../../refactor_reference_docs/spec/07-storage-and-replay.md) — storage split table, raw event archive as ground truth, partitioning, batching
- [spec/03-data-engineering.md](../../refactor_reference_docs/spec/03-data-engineering.md) §9 — partitioning for reads, small-files problem, ClickHouse `ReplacingMergeTree`
- [spec/09-tech-stack.md](../../refactor_reference_docs/spec/09-tech-stack.md) — sqlx, clickhouse, redis, arrow/parquet/datafusion crate selections
- [spec/01-architecture.md](../../refactor_reference_docs/spec/01-architecture.md) — storage layer in end-to-end shape, failure-mode posture for storage writers
