# ADR-0003: NATS JetStream as Event Fabric

**Status:** Accepted
**Date:** 2026-06-08
**Deciders:** Platform team

## Context

The platform requires an internal event bus that connects satellite collectors to the main binary and its internal consumers (UI gateway, strategy runtime, storage writers, feature engine, backtest recorder). The bus must satisfy several requirements simultaneously:

- **Durability and replayability:** Storage writers and strategy runtimes must be able to replay from a past offset when they restart. A pure in-memory pub/sub (like bare NATS Core) loses messages when a consumer is down.
- **Typed lanes:** Different event categories (bars, trades, order-book deltas, fills, system events) must be isolatable so consumers subscribe only to what they need. A single firehose is operationally unmanageable.
- **Backpressure handling:** Consumers can fall behind. The bus must buffer rather than drop, and must surface consumer lag metrics.
- **Failure isolation:** The event bus is the spine — if it goes down, nothing trades. It must be simple enough to operate reliably and recover quickly. Complex distributed brokers (Kafka with ZooKeeper/KRaft, multiple broker processes) add operational surface area that can itself become a source of outages.
- **Operational simplicity:** A small team must be able to run, monitor, and recover the bus without a dedicated infrastructure engineer.

The quarantine lane design also requires a bus that can receive malformed/failed messages without contaminating healthy lanes, and that allows replaying the quarantine lane after a normalizer fix.

## Decision

Use **NATS JetStream** (via the `async-nats` crate) as the platform's internal event fabric for all normalized event lanes. JetStream streams are organized as typed lanes partitioned by instrument and venue. All lanes are durable and replayable. A dedicated `quarantine` stream receives events that fail schema validation. The bus is the single spine — if it is unavailable, no trading occurs.

## Rationale

NATS JetStream ships as a single lightweight binary (no ZooKeeper, no broker cluster required for v1). It provides durable, at-least-once delivery with consumer offsets, stream replay from any offset, and subject-based fan-out — exactly the capabilities required. Its operational weight is proportional to the team size.

The `async-nats` crate integrates natively with Tokio, the platform's async runtime, providing non-blocking publish and subscribe without bridging threads or executor incompatibilities.

Kafka/Redpanda would provide higher throughput and more powerful stream processing primitives, but the platform does not yet need Kafka-scale throughput. At the volume of a v1 trading platform (thousands of events per second per instrument, dozens of instruments), JetStream is more than sufficient. The bus is an explicitly reversible choice — nothing in the platform's domain types or crate API surfaces is coupled to NATS-specific primitives.

JetStream's subject hierarchy maps cleanly to the platform's typed-lane design:

```
market.bars.1m.{venue}.{instrument}
market.trades.{venue}.{instrument}
market.orderbook.l2.{venue}.{instrument}
orders.{event_type}
features.{type}.{instrument}
quarantine.{original_lane}
```

Each stream can have independent retention policies, replication factors, and consumer groups.

## Consequences

**Positive:**
- Single lightweight binary: one `nats-server` process covers the v1 deployment.
- At-least-once delivery with durable consumer offsets; consumers replay from their last acknowledged offset on restart.
- Typed lane isolation: a bad feed on one lane cannot pollute another lane's consumers.
- Native Tokio integration via `async-nats`; no thread-bridging overhead.
- Quarantine lane is a first-class JetStream stream; replay after normalizer fix is a standard consumer seek.
- Bus down = halt trading is a simple, correct safety property.

**Negative:**
- JetStream throughput limits are lower than Kafka at extreme scale. If the platform ever handles millions of high-frequency order-book events per second, the bus will need to be replaced.
- NATS JetStream's at-least-once guarantee means consumers must be idempotent (already required by the data engineering design). Exactly-once requires additional coordination at the application layer.
- Consumer group semantics differ from Kafka consumer groups; teams familiar with Kafka must learn JetStream's consumer model.

**Neutral:**
- The event fabric is explicitly called out as a reversible architectural choice. Migrating from JetStream to Kafka or Redpanda requires changing the `event-bus` crate's producer/consumer implementations and the `async-nats` dependency, but does not require changing domain types or strategy logic.
- NATS subject naming conventions must be agreed upon and codified in the `crates/event-bus/src/lanes.rs` file to prevent ad-hoc lane proliferation.

## Alternatives Considered

### Option A: Apache Kafka / Redpanda
Production-grade, battle-tested at massive scale. Consumer offsets, durable streams, rich ecosystem.

Not chosen because: Kafka's operational weight (broker cluster, ZooKeeper or KRaft quorum, topic partition tuning, JVM tuning for Kafka / separate binary for Redpanda) is not justified at v1 scale. Kafka is not a single binary. The ops burden falls on a small team for whom the bus itself must not be a source of alerts.

### Option B: Redis Streams
Redis already exists in the stack (for latest-state cache). Redis Streams provide durable, consumer-group-based pub/sub.

Not chosen because: Redis is designated as a cache layer, never source of truth. Mixing durable event-bus semantics and cache semantics in the same Redis instance creates operational confusion about what can be evicted. Redis Streams also lack JetStream's subject-based fan-out and per-stream retention policy granularity.

### Option C: In-Process Tokio Channels (tokio::sync::broadcast)
No external bus; collectors communicate with the main binary through in-process channels.

Not chosen because: collectors are satellite processes — separate OS processes — so in-process channels cannot span the collector-to-core boundary. Additionally, in-process channels provide no durability or replay capability: if a storage writer task falls behind and is restarted, it loses everything buffered in the channel.

## Amendment: JetStream is the tail, not the spine

**Date:** 2026-06-11
**Status:** Amends the decision above; the original rationale is unchanged.

### What changed

The original ADR placed JetStream on the **critical path** of every market-data event: the `Publisher::publish` call was awaited inline by collectors before the strategy ever saw the tick.  Profiling revealed that awaiting the JetStream server ACK adds 0.5–5 ms per tick — three orders of magnitude above the target strategy latency (< 50 µs p99 tick-to-intent).

### New architecture: tee pattern

The hot path for the Kraken BTC/USD instrument is now an in-process SPSC ring pipeline:

```
Kraken WS  →  ring_raw (4 096)  →  bar-builder  →  ring_world (1 024)
           →  strategy-eval  →  ring_intent (256)  →  risk/exec  →  broker
```

All four stages are `tokio::spawn` tasks that communicate through bounded `rtrb` lock-free rings.  **No stage awaits any network call.**

JetStream receives every event via an asynchronous **tee task** (`apps/platform/src/tee.rs`):

```
Kraken WS  ──(mpsc clone)──▶  tee_task  ──▶  JetStream publish_fire_and_forget
```

The tee task drains an unbounded `tokio::sync::mpsc` channel and calls `Publisher::publish_fire_and_forget`, which spawns a background tokio task per event.  If the tee task falls behind it drops events — JetStream writes are **best-effort** for replay; they are never on the trade-decision critical path.

### Why

JetStream ACK latency is incompatible with sub-millisecond strategy response times.  The ACK round-trip requires a disk write on the NATS server before the call returns, making it unsuitable as an inline pipeline stage for live trading.

### Replay guarantee

The tee task retries on publish failure at the tokio-task level.  Events that are not persisted to JetStream (e.g. during a NATS outage) will not appear in replay.  This is an acceptable trade-off: live trading correctness takes precedence over replay completeness during outages.  A future improvement (set-C issue #35) will bound the tee channel and add local buffering with retry semantics.

### What stays the same

- Satellite collectors (web scraper, reddit, embedder) remain separate processes communicating via NATS — they are low-frequency and not on the hot path.
- JetStream subject naming and lane conventions are unchanged (`crates/event-bus/src/lanes.rs`).
- The `Publisher` type is unchanged; only `publish_fire_and_forget` is new.
- The quarantine lane is unchanged.

## References

