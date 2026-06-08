# Phase 1 — Spine + one of each

> **Self-contained execution doc.** You need only: this file, [`../file-structure.md`](../file-structure.md),
> and the specs — especially [`01-architecture.md`](../spec/01-architecture.md),
> [`03-data-engineering.md`](../spec/03-data-engineering.md), and
> [`07-storage-and-replay.md`](../spec/07-storage-and-replay.md).

## Phase goal

After this phase there is a working **data spine**: the main binary serves an Axum REST API with
auth, the NATS JetStream event fabric carries typed lanes (including `quarantine`), **one crypto
collector** connects to a venue, normalizes messages into `domain` events (quarantining schema
failures, detecting sequence gaps), the storage writers persist to ClickHouse + Postgres + the
Parquet raw archive with batching and dedup, and a **pure bar builder** turns trades into 1s/1m bars
using the same code that replay will later use. End to end: a live trade from the venue lands in
ClickHouse and the Parquet archive, and a 1m bar is published on the bus.

## Prerequisites

- Phase 0 complete: `domain` provides envelope, payloads, money, instrument metadata, ids/lanes; the
  Postgres/ClickHouse DDL exists; the storage-and-replay spec (`docs/specs/COMP-004-*`) defines the
  raw-archive contract (partition layout, immutability, batching, compaction).
- **Decision gate Q2 (resolved):** Use **Kraken** for crypto market data (`crates/collectors/src/crypto/kraken.rs`).
  Kraken WS provides trades, quotes, L2 orderbook, and tickers. (`legacy_python/data_plane/ingest/`
  and `legacy_python/execution/` contain working venue integration to port behavior from.)
- **Demand-driven data engines (critical invariant):** Data pipelines **never start on system init**.
  The venue-router + demand-manager are the gate. A pipeline starts only when a strategy or UI panel
  declares demand. This is enforced in the collector wiring tasks below.

## Invariants this phase must respect

- **Data engines start on demand only.** No collector process starts on system init. The
  `venue-router` + `demand-manager` are the gate. Until a strategy or UI panel declares demand for
  a `(lane, instrument)` pair, that pipeline stays idle. Wire the collector launcher through the
  venue-router, not directly from the binary startup sequence.
- **Builders are pure.** `crates/builders` must not depend on `storage`/`event-bus`. The bar builder
  is a pure function over a trade stream; the live path *feeds* it.
- **Append-only + dedup.** The storage writer is idempotent via the deterministic dedup key from
  `domain::ids`; ClickHouse `ReplacingMergeTree` + a Redis short-window seen-set.
- **Quarantine, never coerce.** `normalize()` returning `Err` sends raw bytes + error to the
  `quarantine` lane; nothing malformed reaches storage.
- **Raw archive is written before derivation.** The raw normalized event is archived to Parquet
  before bars/features are derived from it.

---

## Tasks

### P1-T01 — Config + observability wiring
- **Goal:** Typed config loading and the tracing/metrics stack used by every binary.
- **Files:** `crates/config/src/{lib,model,secrets}.rs`,
  `crates/observability/src/{lib,tracing_setup,metrics,correctness}.rs`.
- **Context:** `config::load()` reads `config/*.toml` + env into a typed `Config` (db urls, nats url,
  ports, batch sizes, watermark defaults, risk defaults) and fails fast. `observability::init()`
  sets up JSON tracing + a metrics registry, including the **correctness** metrics scaffold (consumer
  lag, queue depth, quarantine rate) per [`../spec/03-data-engineering.md`](../spec/03-data-engineering.md) §7.
- **Acceptance:** `platform` boots, loads config from compose env, emits a structured startup log and
  exposes a metrics endpoint/exporter.
- **Depends on:** Phase 0.

### P1-T02 — Event-bus crate (NATS JetStream)
- **Goal:** Typed producer/consumer wrappers over JetStream + lane→subject mapping + quarantine.
- **Files:** `crates/event-bus/src/{lib,nats,lanes,publish,subscribe,quarantine,backpressure}.rs`.
- **Context:** `Producer`/`Consumer` traits in `lib.rs`; `nats.rs` implements them with `async-nats`
  JetStream (durable streams, partitioned by instrument/venue per
  [`../spec/01-architecture.md`](../spec/01-architecture.md)). `publish<T>(EventEnvelope<T>)` serde-
  encodes; `subscribe` gives durable consumers with ack/nack and handles redelivery. `quarantine.rs`
  publishes raw bytes + error to the `quarantine` lane. `backpressure.rs` wraps bounded queues + lag
  metrics (never unbounded growth).
- **Acceptance:** integration test: publish N envelopes to a lane, durable-subscribe, receive all N;
  a forced bad message lands in `quarantine`; redelivery is observed and ack works.
- **Depends on:** P1-T01.

### P1-T03 — Storage backends + writer
- **Goal:** Persist core + time-series + raw archive, batched and deduped.
- **Files:** `crates/storage/src/postgres/{mod,instruments,orders,strategies,users}.rs`,
  `crates/storage/src/clickhouse/{mod,bars,trades,features}.rs`,
  `crates/storage/src/parquet/{mod,partition,compaction}.rs`, `crates/storage/src/redis.rs`,
  `crates/storage/src/writer.rs`.
- **Context:** Backends per [`../spec/07-storage-and-replay.md`](../spec/07-storage-and-replay.md).
  `writer.rs` is the bus consumer that **batches 10k events or 100ms, whichever first**, writes the
  **raw event to Parquet first** (ground truth), then inserts to ClickHouse; dedup via
  `domain::ids` + a Redis seen-set; `compaction.rs` is the nightly small→big file job. Postgres
  modules back instruments/orders/strategies/users (CRUD used in later phases; instruments CRUD used
  now to seed the collector's metadata). Redis is cache only — never source of truth.
- **Acceptance:** integration test: feed trade envelopes through `writer`, confirm rows in ClickHouse
  and files in the Parquet archive under the `lane/instrument/date` path; duplicate delivery yields
  one logical row; raw archive write precedes the ClickHouse insert.
- **Depends on:** P1-T02, Phase 0 DDL.

### P1-T04 — Bar builder (pure)
- **Goal:** Trades → 1s and 1m bars as a pure function, with watermark + revision logic. This is the
  same code Phase 4's replay uses.
- **Files:** `crates/builders/src/{lib,bars,watermark}.rs`,
  `crates/builders/tests/{bars_watermark,bars_determinism}.rs`.
- **Context:** Pure: input is an ordered trade stream + watermark config; output is `BarPayload`
  events. Per [`../spec/03-data-engineering.md`](../spec/03-data-engineering.md) §4: trades before the
  watermark go into the bar; a trade after the published watermark emits a **revision event**
  (`revision: 1`, new `available_time`) — the original bar is never mutated. `watermark.rs` reads the
  per-source watermark (default 2s, from instrument metadata). Stamp `available_time` to include
  build delay.
- **Acceptance:** `bars_watermark` proves a late trade produces a revision and leaves the original
  immutable; `bars_determinism` proves the same trade stream yields byte-identical bars across runs;
  no dependency on `storage`/`event-bus` (verified by `cargo tree`).
- **Depends on:** Phase 0 (payloads, timestamps, instrument metadata).

### P1-T05 — Collector framework + gap detection
- **Goal:** The shared collector machinery: connect/stream/normalize/reconnect + sequence-gap
  detection + quarantine on schema failure.
- **Files:** `crates/collectors/src/{lib,normalizer,reconnect,gap}.rs`.
- **Context:** `Collector` trait (connect, stream, `normalize() -> Result<Vec<EventEnvelope>,
  NormalizeError>`, reconnect policy). `reconnect.rs` backoff + replay-on-reconnect (which causes
  duplicate delivery — handled by dedup downstream). `gap.rs` detects `sequence` gaps, triggers a
  snapshot re-request, and emits `gap.detected` so downstream marks the window suspect
  (per [`../spec/03-data-engineering.md`](../spec/03-data-engineering.md) §7). `normalizer.rs`
  validates schema-on-write; failures route to quarantine.
- **Acceptance:** unit tests: a sequence gap emits `gap.detected` + re-request; a malformed message
  routes to quarantine (not dropped, not coerced).
- **Depends on:** P1-T02, Phase 0.

### P1-T06 — Kraken crypto collector
- **Goal:** The Kraken WS collector that publishes crypto trades, quotes, and L2 orderbook snapshots
  as `domain` events.
- **Files:** `crates/collectors/src/crypto/{mod,kraken}.rs`, `apps/collector-crypto/src/main.rs`.
- **Context:** Implement Kraken's WS v2 message shapes → `TradePayload`/`QuotePayload`/
  `OrderBookPayload`. Port connection/auth/heartbeat behavior from
  `legacy_python/data_plane/ingest/` (read for parity; do not import). The app binary is
  wiring-only: load config, construct the collector, publish to the bus, reconnect independently of
  the core (satellite process per [`../spec/01-architecture.md`](../spec/01-architecture.md)).
  **The collector process does NOT start automatically on system boot.** It is started on demand by
  the venue-router when demand for a Kraken lane is registered (P1-T10).
- **Acceptance:** when started by the venue-router on demand, live Kraken trades appear on
  `market.trades`; a deliberately corrupted frame lands in `quarantine`; the process reconnects after
  a dropped socket without taking the core down; no trades appear on the bus at cold start before
  any demand is declared.
- **Depends on:** P1-T05.

### P1-T07 — Wire the bar builder into the live path
- **Goal:** Consume `market.trades` from the bus, run the pure bar builder, publish `market.bars.1s`
  and `market.bars.1m`.
- **Files:** a live wiring module under `apps/platform/src/main.rs` (or a small `crates/builders`
  consumer adapter — keep the builder pure; the *adapter* does the bus I/O).
- **Context:** The adapter subscribes to `market.trades`, feeds the **pure** `builders::bars` function
  per instrument, and publishes resulting bar/revision events. This proves "same builder, live path."
- **Acceptance:** integration test (or live run): trades on `market.trades` produce `market.bars.1m`
  on the bus, and the bars also land in ClickHouse via the storage writer.
- **Depends on:** P1-T03, P1-T04, P1-T06.

### P1-T08 — Axum REST API + auth (read endpoints)
- **Goal:** The control-plane skeleton: the main binary serves REST with auth and the read endpoints
  that don't need execution yet.
- **Files:** `crates/api/src/{lib,state}.rs`, `crates/api/src/auth/{mod,session}.rs`,
  `crates/api/src/routes/{mod,assets,streams}.rs`, and wiring in `apps/platform/src/main.rs`.
- **Context:** Axum + tower-http middleware (CORS, trace, compression, timeout, request-id) per
  [`../spec/09-tech-stack.md`](../spec/09-tech-stack.md). Auth scopes private data per user
  (Q5 — keep minimal: session/token for the trusted group). Implement `GET /api/assets`,
  `GET /api/instruments/{id}`, `GET /api/streams/available` (per
  [`../spec/06-ui-and-streaming.md`](../spec/06-ui-and-streaming.md)). `state.rs` holds handles to
  storage/bus. Serve `frontend/dist` static files.
- **Acceptance:** `GET /api/assets` returns seeded instruments; `GET /api/instruments/{id}` returns
  metadata; unauthenticated access to a private route is rejected; the SPA static files are served.
- **Depends on:** P1-T01, P1-T03.

### P1-T10 — Venue router
- **Goal:** Implement `crates/venue-router` — the gate between the Demand Manager and the collectors
  that ensures data pipelines start only on demand, never on system init.
- **Files:** `crates/venue-router/src/{lib,registry,resolver,lifecycle}.rs`.
- **Context:** The `VenueRouter` holds a routing table (loaded from `config/lanes.toml`):
  `(AssetClass, DataType)` → `VenueId`. Routing for this phase:
  - `(Crypto, Trades | Quotes | OrderBook | Ticker, *)` → **Kraken**
  Equity routing is added in Phase 6. `lifecycle.rs` ref-counts demand per `(VenueId, lane,
  instrument)` and starts the corresponding collector (via a spawn handle or command channel) when
  the count goes from 0 → 1, and stops it when it returns to 0. The venue-router is called by the
  Demand Manager when demand changes; it never runs collectors speculatively.
  **Critical:** the platform binary must NOT start any collector during its startup sequence. The
  first demand signal (from a strategy or UI subscription) is what starts the pipeline.
- **Acceptance:** (a) cold-start the platform — no collector starts, no Kraken connection is made,
  no trades appear on `market.trades`; (b) declare demand for `(Crypto, market.trades, BTC-USD)` —
  the Kraken collector starts and trades appear; (c) drop demand to zero — the collector stops and
  the connection closes; (d) two consumers demanding the same lane share one collector (ref-count
  stays 1, not 2).
- **Depends on:** P1-T05, P1-T06.

### P1-T09 — End-to-end spine integration test
- **Goal:** Prove the whole spine in one test.
- **Files:** `tests/ingest_to_storage.rs`, `tests/quarantine_replay.rs`.
- **Context:** `ingest_to_storage`: a simulated venue feed → collector → bus → storage writer →
  ClickHouse row + Parquet file + a published 1m bar. `quarantine_replay`: malformed feed →
  quarantine → (simulate normalizer fix) → replay quarantine → row reaches storage (per
  [`../spec/03-data-engineering.md`](../spec/03-data-engineering.md) §1).
- **Acceptance:** both tests pass against the compose infra.
- **Depends on:** P1-T07, P1-T08.

---

## Phase exit criteria

- [ ] `crates/{config,observability,event-bus,storage,builders,collectors,venue-router,api}` are
      implemented for the spine scope and compile.
- [ ] **No collector starts on system init** — verified by cold-starting the platform with no demand
      and confirming no Kraken connection is made and no events appear on `market.*`.
- [ ] The Kraken collector starts when demand is declared and stops when demand drops to zero;
      two consumers sharing a lane share one collector instance (venue-router ref-count).
- [ ] Schema failures go to `quarantine`; gaps emit `gap.detected`.
- [ ] Storage writer persists raw→Parquet then ClickHouse, batched and deduped; Redis is cache only.
- [ ] The pure bar builder produces 1s/1m bars + revisions and is wired into the live path; it has no
      I/O-crate dependencies.
- [ ] REST API serves assets/instruments/streams with auth and serves the SPA.
- [ ] `tests/ingest_to_storage.rs` and `tests/quarantine_replay.rs` pass.
