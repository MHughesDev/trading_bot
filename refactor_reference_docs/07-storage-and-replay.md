# 07 — Storage & Replay

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

- The thing both live and replay read from (so they cannot diverge).
- The insurance policy when a derived store is found to be wrong (rebuild from raw).

## Partitioning (see also 03 §9)

- ClickHouse: `ORDER BY (instrument, available_time)`, partition by month.
- Parquet: `lane/instrument/date` for predicate pushdown.
- Postgres: orders/fills partitioned by user/account; bounded volume so this is fine.

```
s3://bucket/events/market_trades/venue=binance/instrument=BTC-USDT/date=2026-06-08/
s3://bucket/events/market_orderbook/venue=nasdaq/instrument=AAPL/date=2026-06-08/
```

## Batching (avoid small files / per-event inserts)

Storage writers batch: **10,000 events or 100ms, whichever comes first**, then insert to
ClickHouse and flush to Parquet per policy. A nightly **compaction** job rolls small Parquet files
into large ones. Never one DB insert per event.

## Snapshots for expensive state

Don't replay order books from genesis. Use **snapshot at interval N + deltas after snapshot**.
Same pattern for positions, balances, strategy state, indicator windows.

## Replay / backtest loop

```
1. Load selected historical raw events for the instruments/lanes/time range.
2. Merge by available_time and sequence.
3. Advance the simulated clock to the next event.
4. Feed it through the SAME builders/feature engines used live.
5. Update WorldState; call strategy.on_event().
6. Simulate order/fill behavior (paper execution adapter).
7. Record positions, PnL, risk, audit events.
8. Repeat.
```

Determinism guarantees (restated because they are the whole point):

- Strict `available_time` ordering → **no lookahead** (structurally impossible).
- Same builder code live and in replay → **no live/backtest divergence**.
- Immutable raw archive → reproducible forever.

## Honest caveat: backtest fidelity

Backtest is only as honest as its **simulated execution**. If paper fills at mid while reality
fills with slippage and queue position, the backtest flatters itself. Model slippage and partial
fills deliberately and treat backtest results as directional, not gospel — especially before any
real-money switch.
