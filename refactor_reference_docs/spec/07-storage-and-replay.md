# 07 — Storage & Backtesting

## Storage split (minimal for v1)

Do not force everything into Postgres; do not stand up five databases on day one either. Split by
**access pattern**.

| Purpose | Store (v1) |
|---------|-----------|
| Users, accounts, configs, permissions | PostgreSQL |
| Orders, fills, positions, audit ledger | PostgreSQL (+ event log) |
| High-volume bars / trades / features | ClickHouse |
| Raw normalized event archive (ground truth) | Object storage + Parquet |
| Latest-state cache | Redis / Valkey |
| Internal event fabric | NATS JetStream |

Defer until actually needed: OpenSearch/Meilisearch (document/event search), pgvector/Qdrant
(semantic search). Those arrive with news/social/AI features, which are post-v1.

Redis is a **cache / live-state** tool — **never** the source of truth for orders/fills.

## The raw event archive is ground truth

Write **raw normalized events to durable storage before any derivation**, append-only,
immutable. This archive is:

- The data that the market_simulator backtest engine reads when running a simulation.
- The insurance policy when a derived store is found to be wrong (rebuild from raw).

## Partitioning (see also 03 §9)

- ClickHouse: `ORDER BY (instrument, available_time)`, partition by month.
- Parquet: `lane/instrument/date` for predicate pushdown.
- Postgres: orders/fills partitioned by user/account; bounded volume so this is fine.

```
s3://bucket/events/market_trades/venue=coinbase/instrument=BTC-USDT/date=2026-06-08/
s3://bucket/events/market_bars/venue=alpaca/instrument=AAPL/date=2026-06-08/
```

## Batching (avoid small files / per-event inserts)

Storage writers batch: **10,000 events or 100ms, whichever comes first**, then insert to
ClickHouse and flush to Parquet per policy. A nightly **compaction** job rolls small Parquet files
into large ones. Never one DB insert per event.

## Snapshots for expensive state

Don't replay order books from genesis. Use **snapshot at interval N + deltas after snapshot**.
Same pattern for positions, balances, strategy state, indicator windows.

## Backtesting — this repo does NOT own the backtest engine

**The backtest engine lives in a separate repository: `github.com/MHughesDev/market_simulator`.**

This repo is a **consumer** of that library, not its owner. The market_simulator is a
production-grade, event-driven fill simulation engine that accepts strategy definitions and
historical market data and returns per-trade records and performance metrics.

This repo's responsibilities in the backtest flow:
1. **Store raw normalized events** in the Parquet/ClickHouse archive (this repo does this already
   as part of the storage layer).
2. **Export data in the format market_simulator expects** — typed Apache Arrow IPC files matching
   the market_simulator data contract.
3. **Provide an adapter crate** (`crates/market-simulator-adapter`) that translates between this
   repo's domain types and the market_simulator's contracts, submits run requests, and returns
   results in this repo's format.
4. **Surface backtest results** through the REST API and React UI.

```
This repo                              market_simulator (external)
─────────────────────────────────────  ──────────────────────────
Parquet raw archive  ──Arrow IPC──▶   Run Request
Strategy definition  ──────────────▶  │
                                       ▼
                     ◀──TradeRecords── Fill engine + metrics
REST /api/backtests ◀──Metrics──────  (per-trade, PnL, drawdown)
```

**Do not build a fill simulator in this repo.** Any simulated execution in this repo is only
for the paper trading live path; backtest fill simulation is owned entirely by market_simulator.

### market_simulator scope (v1)

The market_simulator currently supports order-book-style assets (Engine A: equities, crypto spot
CEX). The adapter in this repo is written against that contract. As market_simulator expands to
support additional price-formation mechanics (AMM, derivatives, etc.), the adapter is extended —
the platform does not own those simulation mechanics.

The market_simulator is stateless: no session, no database, no persistence between invocations.
Each Run Request is self-contained. This repo owns all persistent state; the simulator is called
per-run and discarded.

### Data format for backtesting (MVP)

For the MVP, historical data exported to market_simulator is **1-minute OHLCV bars** (the primary
live data granularity for Coinbase and Alpaca — see [03-data-engineering.md](./03-data-engineering.md)).
The architecture supports exporting finer data (second bars, order-book snapshots) when those
lanes are populated in the archive; the adapter should be written to handle all supported
granularities, with 1m being the default for v1.

## Honest caveat: backtest fidelity

Backtest fidelity is bounded by the historical data granularity supplied. 1-minute bar fills are
less accurate than tick-level fills — slippage modeling differs significantly. Treat bar-level
backtest results as directional, not gospel, until finer data is available. The market_simulator
reports which fidelity level a run used and flags lower-accuracy runs explicitly.
