# ADR-0011: Demand-Driven Data Engines — No Auto-Start

**Status:** Accepted
**Date:** 2026-06-08
**Deciders:** Platform team

## Context

A multi-user, multi-instrument trading platform faces a fundamental resource management question: when should data pipelines (market data collectors, bar builders, feature engines) be running, and for which instruments?

The naive answer — start all pipelines for all instruments on system startup — fails immediately:

- The platform supports an open-ended instrument universe (crypto and equities across multiple venues). Starting pipelines for every possible instrument on boot consumes bandwidth, CPU, and memory proportional to the full instrument universe, regardless of whether any strategy or user panel actually needs any of those instruments.
- Many instruments will never be used. Running unused pipelines wastes resources and adds noise to operational metrics.
- Multiple consumers may need the same instrument (two users both running strategies on BTC-USDT). If each consumer starts its own pipeline independently, duplicate streams emerge: two connections to the Kraken feed, two bar builders, two feature engine instances — all for the same data.

The alternative naive answer — let each strategy and UI panel start its own dedicated pipeline when it needs data — solves the "don't start until needed" problem but reintroduces the duplicate pipeline problem. It also fails silently on teardown: when a strategy shuts down, does its pipeline shut down too? What if another consumer was sharing it?

The platform requires a coordinated mechanism that starts pipelines only when there is genuine demand, deduplicates pipelines across consumers sharing the same instrument, and stops pipelines when the last consumer exits.

## Decision

Data pipelines start **only when a strategy instance or UI panel declares demand** via the **Demand Manager** in the main binary. No pipeline starts on system initialization. No pipeline starts because a configuration file lists an instrument. No pipeline starts because an administrator pressed a "start" button.

Demand is declared as a structured request:

```json
{
  "consumer_id": "strategy_instance_user42_btc_usdt",
  "consumer_type": "strategy_runtime",
  "needs": [
    { "lane": "market.bars.1m", "instrument": "BTC-USDT" },
    { "lane": "features.technical", "instrument": "BTC-USDT" }
  ]
}
```

The Demand Manager maintains a reference count per `(lane, instrument)` pair. When demand is declared:
- If the reference count was zero, the Demand Manager starts the pipeline (instructs the venue-router to activate the collector subscription and bar builder for that instrument).
- If the reference count was already above zero (another consumer already declared the same demand), no new pipeline is started — the existing pipeline serves all consumers.

When a consumer drops its demand (strategy stops, UI panel closes, user disconnects):
- The reference count decrements.
- If the count drops to zero (last consumer), the pipeline is stopped, paused, or downshifted to archival/low-frequency mode.

The Demand Manager is part of the main binary. Satellite collectors receive start/stop instructions from it via the event fabric or a control channel, not from static configuration.

## Rationale

Demand-driven pipeline management is the correct model for a multi-consumer system with a large potential instrument universe. The reference-counting design ensures that resource usage is proportional to actual demand, not to the size of the potential universe.

Deduplication at the Demand Manager layer prevents the thundering-herd problem: if ten users simultaneously initialize strategies on BTC-USDT, the Demand Manager starts exactly one BTC-USDT pipeline and routes all ten consumers to it. Without the Demand Manager, ten parallel implementations would each independently try to start a pipeline, producing either ten connections to Kraken or ten races to acquire a single connection.

The graceful shutdown path (reference count drops to zero → pipeline stops) ensures that idle pipelines do not consume resources indefinitely. In a live system, instruments cycle in and out of interest as users activate and deactivate strategies. Pipelines that run during off-hours with no consumers are pure waste.

The architecture of pure-function builders (ADR-0008) makes this possible: because bar builders and feature engines are stateless pure functions over event streams, starting and stopping them is a matter of starting and stopping the event stream subscription. There is no accumulated state that would be lost on stop and need to be rebuilt on restart (beyond the configurable snapshot-at-interval pattern described in spec/07).

## Consequences

**Positive:**
- Resource usage is proportional to actual demand; idle instruments consume zero pipeline resources.
- Duplicate pipelines for shared instruments are structurally prevented by the reference-counting Demand Manager.
- Graceful scale-down: when the last user interested in an instrument stops, its pipeline stops automatically, without manual intervention.
- The pipeline lifecycle is explicit and auditable: the Demand Manager's reference count table at any moment is a complete picture of which instruments are being actively watched and by whom.

**Negative:**
- Cold-start latency: when the first consumer declares demand for an instrument, there is a pipeline startup delay before data flows. For market data that was streaming before (e.g., a user re-opens a panel), a small delay is expected and acceptable. For a strategy that immediately needs the latest bar to make a decision, this startup delay must be accounted for in the strategy initialization flow.
- The Demand Manager is a centralized coordinator inside the main binary. A bug in demand reference counting (e.g., a consumer that declares demand but never releases it on shutdown) can cause pipelines to run longer than needed or — worse — a consumer that fails to declare demand correctly will not receive data.
- Snapshot-at-interval state recovery for features and order books must be handled when a pipeline restarts after a period of being stopped, so that consumers do not receive a cold-start period with no feature values.

**Neutral:**
- The Demand Manager's reference count table is operational state, not durable state. On a full system restart, the Demand Manager starts with zero demand; pipelines are restarted as strategy instances and UI panels reconnect and re-declare their demand. This is correct behavior: the system should not assume previous demand persists across restarts without consumers confirming it.
- "Downshifting to archival mode" (rather than a full stop when demand drops to zero) is an optimization for instruments where the cost of a full restart is high (e.g., re-requesting a level-2 order book snapshot). This optimization is a future enhancement; the v1 behavior is stop on zero demand.

## Alternatives Considered

### Option A: Start All Pipelines for All Instruments on Boot
Configure a static instrument universe; start all pipelines on system initialization regardless of demand.

Not chosen because: the instrument universe is open-ended (all Coinbase-listed crypto, all Alpaca-listed equities). Starting pipelines for every possible instrument on boot is not feasible. Even for a fixed set of 50 instruments, running pipelines for 40 that no active strategy cares about wastes 80% of pipeline resources.

### Option B: Consumer-Managed Pipeline Lifecycle (Each Consumer Starts Its Own)
Each strategy instance and UI panel is responsible for starting and stopping its own data pipelines.

Not chosen because: when two consumers need the same instrument, each independently starts a pipeline, producing duplicate connections to the venue's feed, duplicate bar builders, and duplicate feature engine instances. The platform would need out-of-band coordination between consumers to avoid this — which is exactly what the Demand Manager provides, but without central accountability.

### Option C: Admin-Controlled Pipeline Start/Stop
An administrator explicitly starts and stops pipelines for each instrument via an admin UI or configuration file.

Not chosen because: administrative manual management does not scale to a system with dynamic user activity and an open instrument universe. It also creates a support burden where a strategy fails because an administrator forgot to start its instrument's pipeline. The Demand Manager automates what would otherwise be manual, error-prone operations.

## References

- ADR-0008 — Pure function builders (prerequisite enabling demand-driven start/stop without state loss)
