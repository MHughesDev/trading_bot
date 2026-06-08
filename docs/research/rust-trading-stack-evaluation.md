# Rust Trading Stack Evaluation

**Question:** Why Rust for this trading platform rewrite, and which libraries should form the core stack?
**Status:** Complete
**Outcome:** Rust adopted as the sole backend language with tokio/axum/async-nats/sqlx/ClickHouse/Redis/parquet+arrow/rust_decimal as the primary library stack.
**ADR(s):** ADR-0001 (modular monolith architecture), ADR-0003 (NATS JetStream event fabric), ADR-0004 (storage split: Postgres/ClickHouse/Parquet/Redis)

---

## Method

Evaluation based on a review of the system's primary technical requirements:
- A money-handling system where price/size type safety must be compile-time enforced.
- A data-heavy pipeline (high-frequency bars, trades, order-book deltas) requiring real async I/O without per-task overhead.
- A small team where legibility and correctness are more valuable than raw throughput.
- A local-first, private-network deployment where operational complexity is a cost, not an acceptable trade-off.

Each library choice was evaluated against the system's six planes (Control, UI Live, Data, Storage, Strategy, Replay), the crate purity rule (builders/features must be pure functions with no I/O), and the invariant that no `f64` ever touches a price or size.

## Findings

### Language: Rust

Rust was selected over Python (the current language) and Go as the rewrite target.

**Against Python:** The existing Python system is organically grown, lacks compile-time enforcement of architectural boundaries, and cannot express the `Price`/`Size` newtype contract that prevents `f64` from ever touching money. Python's GIL and async model add complexity to a high-throughput event pipeline without the type guarantees that justify the design.

**Against Go:** Go's interface system cannot enforce the `no From<f64>` invariant on money types at compile time. Go's goroutine model is simpler but provides no zero-cost abstraction for the crate boundary enforcement that replaces process-boundary discipline in the monolith. Rust's ownership model eliminates an entire class of bugs (use-after-free, data races) without runtime overhead.

**Rust advantages for this system:**
- The `rust_decimal` newtype pattern (`Price(Decimal)`, `Size(Decimal)`, no `From<f64>`) is enforced by the compiler on every code path.
- Cargo workspaces enforce crate-boundary discipline (the architectural rule that `builders`/`features` cannot import `storage` or `event-bus`) via the dependency graph — CI enforces it without process boundaries.
- Zero-cost async with tokio eliminates per-task overhead on the high-frequency data path.
- The same `builders`/`features` crate code runs identically in live consumption and in replay (no branching on execution mode), guaranteed by the purity rule.

### Async Runtime: tokio

De-facto production async runtime for network-heavy Rust. The entire stack — NATS subscribers, broker WebSocket connections, Axum HTTP server, sqlx connection pools, ClickHouse batch writers — runs on a single tokio executor. Alternatives (async-std, smol) provide no material advantage for this workload and would fragment the ecosystem library choices.

### REST + WebSocket server: axum + tower/tower-http

Axum provides ergonomic routing, native WebSocket upgrade support, and a tower-compatible middleware stack. tower-http supplies CORS, structured tracing, compression, request-id injection, and timeout middleware as composable layers. The alternative (actix-web) is more performant at the micro-benchmark level but trades ergonomics and tower-compatibility for a marginal throughput gain that this workload will never exercise.

### Event Fabric: NATS JetStream (async-nats)

NATS JetStream provides durable, replayable pub/sub via a single lightweight binary with no ZooKeeper or broker cluster to operate. At the scope of this platform (one team, local-first, private network), Kafka's operational weight is not justified. The bus is the spine — everything else is replaceable; the bus choice is reversible if the system ever outgrows JetStream.

Key properties needed and provided:
- Durable consumer groups for storage writers and strategy runtime (never drop fills or order events).
- Replayable subjects for the raw event archive path.
- Quarantine subject for schema normalization failures.
- Bounded queues via consumer flow-control to enforce backpressure.

### Transactional Database: PostgreSQL via sqlx

Postgres via sqlx provides direct SQL with compile-time-checked queries, no ORM magic, and full support for UUID, chrono timestamps, and rust_decimal. Postgres stores the transactional core: users, orders, fills, positions, strategy definitions, risk config. An ORM (Diesel, SeaORM) was considered and rejected — the required control level is direct SQL, and the compile-time query checking that sqlx provides eliminates the main safety argument for an ORM.

### Time-Series Database: ClickHouse

ClickHouse handles high-volume bar/trade/feature data with fast historical scans for backtest data loading and analytics. Its columnar storage and ReplacingMergeTree engine (with dedup key on instrument + available_time) handle bar revisions correctly. TimescaleDB was considered but ClickHouse's columnar scan performance for the research/backtest workload is materially better. InfluxDB was rejected for its limited SQL surface and weaker deduplication model.

### Cache: Redis/Valkey

Redis is the latest-state cache only — latest price snapshots, subscription state, rate-limit counters. It is explicitly never the source of truth for orders, fills, or positions. The `redis` crate with async/tokio support (`aio`, `tokio-comp` features) provides the required interface. Valkey (the Redis fork) is a drop-in replacement and is acceptable.

### WebSocket clients: tokio-tungstenite

For connecting to venue market-data and user streams (Kraken, Coinbase, Alpaca). Integrates natively with the tokio executor.

### HTTP Client: reqwest (rustls)

For broker REST APIs (Coinbase, Alpaca). rustls-tls avoids the OpenSSL system dependency. The `json`, `gzip`, and `brotli` features cover the REST API surface of the target brokers.

### Serialization: serde / serde_json

JSON for the control plane (REST API) and UI (WebSocket frames). Binary serialization (MessagePack, Bincode) was deferred — it can be added to the NATS lanes without changing the type system if throughput demands it.

### Columnar / Research: arrow + parquet + datafusion

The raw event archive is written as Parquet (ground truth, immutable, partitioned by lane/instrument/date). Arrow IPC is the export format to `github.com/MHughesDev/market_simulator` for backtesting. DataFusion provides ad-hoc analytical queries over the archive. This is the foundation of the Replay plane.

### Money types: rust_decimal

`Price(Decimal)` and `Size(Decimal)` newtypes in `crates/domain::money`. The `From<f64>` impl is deliberately absent — the type system enforces the no-float-money invariant across the entire codebase. Floating-point arithmetic is not acceptable for any price, size, or PnL calculation.

### Time: chrono

All timestamps are UTC `DateTime<Utc>`. The four timestamps (source, exchange, received, available) are chrono types. No wall-clock reads inside strategy or builder logic — world.now() provides the clock for deterministic replay.

### IDs: uuid

Event IDs, correlation/causation IDs, idempotency keys for fills and order submissions. `v4` for generation, `serde` feature for JSON serialization.

### Observability: tracing + tracing-subscriber

Structured logs (JSON output) and metrics via the tracing ecosystem. Correctness metrics (consumer lag, queue depth, quarantine rate, reconciliation divergences, freshness-watchdog state) are first-class alongside throughput metrics.

### Deliberately excluded in v1

| Excluded | Reason |
|----------|--------|
| Kafka / Redpanda / rdkafka | JetStream is sufficient; Kafka's ops weight is unjustified at this scope |
| Heavy ORM (Diesel, SeaORM) | Direct SQL via sqlx is the right control level |
| OpenSearch / vector DBs | Post-v1 AI/news features; isolate when they arrive |
| Headless browsers in workers | Post-v1 scraping; will be isolated behind a separate service |

## Recommendation

Adopt the full stack as specified in `spec/09-tech-stack.md`:

```
tokio (async runtime) + axum/tower-http (API) + async-nats (event bus) +
sqlx+Postgres (transactional) + ClickHouse (time-series) + Redis (cache) +
arrow+parquet (archive+backtest) + rust_decimal (money) + chrono + uuid + tracing
```

This stack is the minimum viable set for the system's six planes. No dependency is speculative — each maps directly to a crate in the architecture. The non-Rust parts of the stack (NATS JetStream, Postgres, ClickHouse, Redis) are all operated via their respective Rust client crates and require no non-Rust service SDK.

## References

- `/home/user/trading_bot/refactor_reference_docs/spec/09-tech-stack.md` — choices, rationale, starting Cargo.toml
- `/home/user/trading_bot/refactor_reference_docs/spec/01-architecture.md` — the six planes, monolith rationale
- `/home/user/trading_bot/refactor_reference_docs/file-structure.md` — crate dependency graph and purity rules
- tokio: https://tokio.rs
- axum: https://docs.rs/axum
- async-nats / NATS JetStream: https://docs.nats.io / https://docs.rs/async-nats
- sqlx: https://docs.rs/sqlx
- ClickHouse Rust client: https://docs.rs/clickhouse
- rust_decimal: https://docs.rs/rust_decimal
- arrow / parquet (Apache Arrow): https://docs.rs/arrow
- github.com/MHughesDev/market_simulator — backtest engine (external dependency)
