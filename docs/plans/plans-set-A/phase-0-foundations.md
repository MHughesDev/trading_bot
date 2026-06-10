---
Type: Formal
Status: Pending
Derived From: DATA-001, DATA-002, DATA-003, DATA-004, ADR-0002, ADR-0007, ADR-0008, ADR-0009, SC-1, SC-3, SC-4
Note: Canonical executable plans live in docs/plans/. This copy is the traceable documentation record. On any conflict, [deleted - see Phase 7]/ wins.
---

# Phase 0 — Foundations (the irreversible core)

> **Self-contained execution doc.** You need only: this file, [`../architecture.md`](../architecture.md),
> and the specs in [`../specs/`](../specs/) — especially
> [`../specs/DATA-001-event-envelope-and-payloads.md`](../specs/DATA-001-event-envelope-and-payloads.md),
> [`../specs/COMP-001-data-quality-and-ingestion.md`](../specs/COMP-001-data-quality-and-ingestion.md),
> and [`../specs/FEAT-001-strategy-system.md`](../specs/FEAT-001-strategy-system.md).
>
> **Why this phase is special:** The types built here are *tattoos*. Everything stored depends on them
> forever. Get them right; stay loose on transport. Nothing in later phases starts until this exists.

## Phase goal

After this phase, the `domain` crate fully defines the irreversible core: the universal event
envelope, the four-timestamp model, the `Price`/`Size` money types that make a float impossible, the
trust tiers, the v1 payloads, the instrument-metadata model, the deterministic identity/dedup keys,
the canonical lane names, and the **strategy-definition format frozen at `1.0`**. The Postgres and
ClickHouse schemas for these types exist as migrations, and the raw-event-archive layout is designed
and documented.

## Prerequisites

- Phase B complete (workspace skeleton compiles; infra runs).
- **Decision gate Q3** (strategy-format 1.0 freeze) is *produced* in this phase (P0-T08). The
  expression language, node types, `$each` fan-out semantics, and tighten-only override rule must be
  pinned here. If unanswered, make the decisions per the sketch in
  [`../specs/FEAT-001-strategy-system.md`](../specs/FEAT-001-strategy-system.md) and document them.

## Invariants this phase must respect

- **No `From<f64>` on `Price`/`Size`.** This is the single most important compile-time guarantee in
  the system. A float must not be able to construct a price.
- **Append-only semantics are encoded in the types**, not bolted on later: `BarPayload.revision`,
  immutable envelopes, deterministic dedup keys.
- **The strategy format, once written here as `1.0`, is frozen.** Later changes require a new version
  field value, never an edit to `1.0`'s meaning.

---

## Tasks

### P0-T01 — Money types (`Price`/`Size`)
- **Goal:** Decimal newtypes that the compiler refuses to let a float into.
- **Files:** `crates/domain/src/money.rs`, `crates/domain/tests/money_no_float.rs`.
- **Context:** `pub struct Price(pub Decimal); pub struct Size(pub Decimal);` over `rust_decimal`.
  Provide `from_str`, arithmetic via `Decimal`, serde as string/decimal. **Do not** implement
  `From<f64>` or `TryFrom<f64>`. Provide a `quantize(precision)` that quantizes to an instrument's
  precision (used at normalization) — never truncating real money away (see
  [`../specs/DATA-001-event-envelope-and-payloads.md`](../specs/DATA-001-event-envelope-and-payloads.md)
  §money types).
- **Acceptance:** `money_no_float.rs` proves (compile-fail doc-test or trait-absence test) that
  `Price::from(0.1f64)` does not compile; arithmetic + quantize behave; serde round-trips.
- **Depends on:** none.

### P0-T02 — Trust tiers
- **Goal:** `TrustTier` as a first-class ordered enum.
- **Files:** `crates/domain/src/trust.rs`.
- **Context:** Variants `Regulated`, `CentralizedExchange`, `OnchainConfirmed`, `OnchainTentative`,
  `SocialDerived` (per [`../specs/COMP-001-data-quality-and-ingestion.md`](../specs/COMP-001-data-quality-and-ingestion.md)
  §8). Provide ordering so a strategy's `min_trust_tier` can be compared against an event's tier.
  serde.
- **Acceptance:** ordering test (`Regulated >= CentralizedExchange`, etc. per chosen semantics);
  serde round-trip.
- **Depends on:** none.

### P0-T03 — Timestamp model
- **Goal:** The four timestamps and `available_time` semantics.
- **Files:** `crates/domain/src/timestamp.rs`.
- **Context:** `event_time` (Option), `observed_time`, `ingested_time`, `available_time` — all
  `DateTime<Utc>` (see [`../specs/DATA-003-timestamps-and-identity.md`](../specs/DATA-003-timestamps-and-identity.md)
  §timestamp model). Provide a helper to compute
  `available_time = max(window_close, observed) + watermark + processing_delay`.
  Document that `available_time` is the replay sort key.
- **Acceptance:** unit test of the `available_time` computation including a processing-delay case.
- **Depends on:** none.

### P0-T04 — Payload types
- **Goal:** The v1 versioned payloads.
- **Files:** `crates/domain/src/payloads/{mod,trade,quote,orderbook,bar}.rs`.
- **Context:** Exactly the structs in [`../specs/DATA-001-event-envelope-and-payloads.md`](../specs/DATA-001-event-envelope-and-payloads.md)
  §v1 payloads: `TradePayload`, `QuotePayload`, `OrderBookPayload` (`BookUpdateKind`, levels,
  `sequence`, `is_tentative`), `BarPayload` (`Timeframe`, OHLCV as `Price`/`Size`, `trade_count`,
  `revision`). Each payload carries/knows its `schema_version`. `mod.rs` defines a `Payload` trait
  with `event_type()`/`schema_version()` and a registry of versioned types (the compiled structs
  **are** the schema registry).
- **Acceptance:** serde round-trip per payload; `bar.revision` defaults to 0; OHLCV fields are
  `Price`/`Size`, not floats.
- **Depends on:** P0-T01.

### P0-T05 — Event envelope
- **Goal:** The universal `EventEnvelope<T>`.
- **Files:** `crates/domain/src/envelope.rs`, `crates/domain/tests/envelope_roundtrip.rs`.
- **Context:** Exactly the struct in [`../specs/DATA-001-event-envelope-and-payloads.md`](../specs/DATA-001-event-envelope-and-payloads.md)
  §envelope: `event_id`, `event_type`, `schema_version`, `lane`, `instrument_id`, `venue_id`,
  `source`, `trust_tier`, the four timestamps, `sequence`, `correlation_id`, `causation_id`,
  `payload: T`. Generic over payload `T: Payload`.
- **Acceptance:** `envelope_roundtrip.rs` round-trips the envelope wrapping each payload type.
- **Depends on:** P0-T02, P0-T03, P0-T04.

### P0-T06 — Identity & dedup keys + lanes
- **Goal:** Deterministic identity derived from the source (never a random UUID at ingest), plus the
  canonical lane names.
- **Files:** `crates/domain/src/ids.rs`, `crates/domain/src/lanes.rs`.
- **Context:** Dedup-key forms from [`../specs/DATA-003-timestamps-and-identity.md`](../specs/DATA-003-timestamps-and-identity.md)
  §identity: `lane+instrument+venue+sequence+source`, `venue+exchange_trade_id`,
  `chain+tx_hash+log_index` (future). `lanes.rs` enumerates lane name constants + a typed lane enum
  from the lane list in [`../specs/SYS-001-system-overview.md`](../specs/SYS-001-system-overview.md)
  (`market.trades`, `market.quotes`, `market.orderbook.l2`, `market.bars.1s`, `market.bars.1m`,
  `features.technical`, `strategy.signals`, `orders.commands`, `orders.events`, `positions.events`,
  `quarantine`).
- **Acceptance:** identical inputs produce identical dedup keys; differing inputs differ; lane enum
  ↔ string round-trips.
- **Depends on:** P0-T05.

### P0-T07 — Instrument metadata model
- **Goal:** The `Instrument` model that makes asset classes a data concern, not a code concern.
- **Files:** `crates/domain/src/instrument.rs`.
- **Context:** Exactly the struct in [`../specs/DATA-002-instrument-metadata.md`](../specs/DATA-002-instrument-metadata.md)
  §instrument metadata: `instrument_id`, `asset_class` (`Crypto`/`Equity`, extensible), `venue_id`,
  `base_precision`, `quote_precision`, `tick_size`, `lot_size`, `trading_hours` (`TradingSchedule`:
  24/7 for crypto, session+auctions for equities), `halt_behavior` (`HaltPolicy`), `trust_tier`,
  `active`. Include a per-source `watermark` field hook (Q7) — default 2s.
- **Acceptance:** serde round-trip; a crypto (24/7) and an equity (session) instance construct;
  `tick_size`/`lot_size` are `Decimal`.
- **Depends on:** P0-T01, P0-T02.

### P0-T08 — FREEZE the strategy-definition format at 1.0
- **Goal:** The canonical strategy-definition format, pinned to `1.0`. This is the contract all three
  front doors (Phase 5) target; freezing it now is the whole point of doing it in Phase 0.
- **Files:** `crates/domain/src/strategy_def/{mod,inputs,nodes,actions,risk_overrides}.rs`,
  `crates/domain/tests/strategy_def_schema.rs`.
- **Context:** Build out the sketch in [`../specs/DATA-004-strategy-definition-format.md`](../specs/DATA-004-strategy-definition-format.md)
  into concrete types: `StrategyDefinition { strategy_id, definition_version: "1.0", asset_universe,
  min_trust_tier, inputs, nodes, actions, risk_overrides }`. Decide and **document in module
  docs** (resolving Q3):
  - the **expression language** for `condition.expr` (recommend a small, explicitly-defined
    grammar — comparison/arithmetic over `feature(...)`, `bar(...)`, literals — parsed by
    `strategy-validator` in Phase 5; here just define the AST/string type and document the grammar);
  - the **node types** (at least `condition`, `signal`) and the action types (`place_order`,
    `size_mode: fixed|...`);
  - how **`$each`** fans `inputs` across `asset_universe`;
  - that **`risk_overrides` may only tighten** the global gate (documented invariant; enforced by
    `strategy-validator` in Phase 5 and `risk` in Phase 2).
- **Acceptance:** `strategy_def_schema.rs` deserializes the example JSON from
  [`../specs/DATA-004-strategy-definition-format.md`](../specs/DATA-004-strategy-definition-format.md)
  and re-serializes it stably; `definition_version` is `"1.0"`; module docs state the frozen grammar
  and the tighten-only rule.
- **Depends on:** P0-T02 (trust tier), P0-T01 (sizes in actions).

### P0-T09 — Order / position / error domain types
- **Goal:** The order-flow and error types other crates build on.
- **Files:** `crates/domain/src/order.rs`, `crates/domain/src/position.rs`,
  `crates/domain/src/error.rs`, and re-exports in `crates/domain/src/lib.rs`.
- **Context:** `OrderRequest`/`OrderIntent` (with an **idempotency key** field — see
  [`../specs/COMP-002-execution-and-risk-gate.md`](../specs/COMP-002-execution-and-risk-gate.md)
  §idempotency), `OrderState` machine states
  (`accepted/submitted/partially_filled/filled/cancelled/rejected`), `Side`, `OrderType`.
  `Position`, `Balance`. `error.rs` seeds `NormalizeError`, `ValidationError`, `RiskRejection`.
  `lib.rs` re-exports the full public API.
- **Acceptance:** types compile and serde round-trip; `lib.rs` exposes a coherent public surface;
  idempotency key is mandatory on `OrderIntent`.
- **Depends on:** P0-T01, P0-T07.

### P0-T10 — Postgres + ClickHouse schemas for the core
- **Goal:** Persistence DDL for the irreversible types (no writer code yet — that is Phase 1).
- **Files:** `migrations/0001_users_accounts.sql`, `migrations/0002_instruments.sql`,
  `migrations/0003_orders_fills_positions.sql`, `migrations/0004_strategies.sql`,
  `migrations/0005_risk_config.sql`; `clickhouse/01_trades.sql`, `clickhouse/02_bars.sql`,
  `clickhouse/03_features.sql`.
- **Context:** Money columns are Postgres `NUMERIC` / ClickHouse `Decimal128(scale)` — **never
  float** (see [`../specs/DATA-001-event-envelope-and-payloads.md`](../specs/DATA-001-event-envelope-and-payloads.md)
  §money, §storage). ClickHouse tables use `ReplacingMergeTree` ordered on the dedup key,
  `ORDER BY (instrument, available_time)`, partitioned by month (see
  [`../specs/COMP-001-data-quality-and-ingestion.md`](../specs/COMP-001-data-quality-and-ingestion.md)
  §9, [`../specs/COMP-004-storage-and-replay.md`](../specs/COMP-004-storage-and-replay.md)).
  Instruments table mirrors P0-T07. `0005` includes the global `trading_enabled` flag + per-user
  risk limits.
- **Acceptance:** `just migrate` applies Postgres migrations cleanly against the compose Postgres;
  ClickHouse DDL applies; columns for prices/sizes are decimal types, not float.
- **Depends on:** P0-T07, P0-T09.

### P0-T11 — Raw event archive design (in the storage spec)
- **Goal:** Pin down the ground-truth raw archive contract before any derivation is written, so
  Phase 1's writer has a contract — recorded in the documentation workspace, not a loose file.
- **Files:** extend the storage-and-replay spec from Phase A
  (`docs/specs/COMP-004-storage-and-replay.md`) with the raw-archive section (partition layout,
  immutability, batching, compaction); stub `crates/storage/src/parquet/partition.rs` with the path
  function signature + doc.
- **Context:** Per [`../specs/COMP-004-storage-and-replay.md`](../specs/COMP-004-storage-and-replay.md):
  raw normalized events written **before any derivation**, append-only, immutable, partitioned
  `lane/instrument/date`, e.g. `s3://bucket/events/market_trades/venue=binance/instrument=BTC-USDT/date=2026-06-08/`.
  Document batching policy (10k events or 100ms) and nightly compaction as the contract Phase 1
  implements.
- **Acceptance:** `COMP-004` specifies partition layout, immutability, batching, and compaction;
  `partition.rs` exposes a `partition_path(lane, instrument, date)` signature.
- **Depends on:** P0-T06.

---

## Phase exit criteria

- [ ] `crates/domain` fully implements envelope, timestamps, money (no `From<f64>`), trust, payloads,
      instrument metadata, ids/lanes, order/position/error types, and the **frozen 1.0** strategy
      format — all re-exported from `lib.rs`.
- [ ] `cargo test -p domain` passes, including `money_no_float`, `envelope_roundtrip`, and
      `strategy_def_schema`.
- [ ] `just migrate` applies all Postgres migrations; ClickHouse DDL applies; all money columns are
      decimal types.
- [ ] `COMP-004` (storage-and-replay spec) documents the ground-truth archive contract.
- [ ] Q3 is resolved: the strategy format's grammar, node types, `$each`, and tighten-only rule are
      documented in `strategy_def` module docs.
