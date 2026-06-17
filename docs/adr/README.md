# Architecture Decision Records

This directory contains Architecture Decision Records (ADRs) for the Rust trading platform refactor.
Each ADR documents a significant architectural decision: its context, the decision made, the rationale,
consequences, and alternatives that were considered.

ADRs are immutable once accepted. A superseded ADR is updated only to note its status and the ID of the
ADR that replaces it.

## Index

| ID | Title | Status | Date |
|----|-------|--------|------|
| [ADR-0001](0001-rust-modular-monolith-with-satellite-collectors.md) | Rust Modular Monolith with Satellite Collectors | Accepted | 2026-06-08 |
| [ADR-0002](0002-decimal-money-newtypes-no-f64.md) | Decimal Money Newtypes — No f64 | Accepted | 2026-06-08 |
| [ADR-0003](0003-nats-jetstream-event-fabric.md) | NATS JetStream as Event Fabric | Accepted | 2026-06-08 |
| [ADR-0004](0004-storage-split-postgres-clickhouse-parquet-redis.md) | Storage Split — Postgres, ClickHouse, Parquet, Redis | Accepted | 2026-06-08 |
| [ADR-0005](0005-single-risk-gate-chokepoint-and-kill-switch.md) | Single Risk Gate Chokepoint and Kill Switch | Accepted | 2026-06-08 |
| [ADR-0006](0006-three-system-broker-architecture-coinbase-alpaca-market-simulator.md) | Three-System Broker Architecture — Coinbase, Alpaca, market_simulator | Accepted | 2026-06-08 |
| [ADR-0007](0007-freeze-strategy-definition-format-v1.md) | Freeze Strategy Definition Format v1.0 | Accepted | 2026-06-08 |
| [ADR-0008](0008-available-time-ordering-and-same-builders-live-and-replay.md) | available_time Ordering and Same Builders for Live and Replay | Accepted | 2026-06-08 |
| [ADR-0009](0009-append-only-raw-event-archive-as-ground-truth.md) | Append-Only Raw Event Archive as Ground Truth | Accepted | 2026-06-08 |
| [ADR-0010](0010-three-front-doors-one-canonical-strategy-json.md) | Three Front Doors, One Canonical Strategy JSON | Accepted | 2026-06-08 |
| [ADR-0011](0011-demand-driven-data-engines-no-auto-start.md) | Demand-Driven Data Engines — No Auto-Start | Accepted | 2026-06-08 |
| [ADR-0015](0015-freeze-model-definition-format-v1.md) | Freeze Model Definition Format v1.0 | Accepted | 2026-06-15 |
| [ADR-0016](0016-distributional-forecast-contract.md) | Distributional Forecast Contract v1.1 | Accepted | 2026-06-16 |
| [ADR-0017](0017-walk-forward-cv-and-leakage-discipline.md) | Walk-Forward Cross-Validation and Leakage Discipline | Accepted | 2026-06-16 |
| [ADR-0018](0018-ensemble-combination-and-conformal-calibration.md) | Ensemble Combination and Conformal Calibration | Accepted | 2026-06-16 |
| [ADR-0019](0019-run-study-experiment-object-model.md) | Run / Study / Experiment Object Model + Sealed Distributions | Accepted | 2026-06-17 |
| [ADR-0020](0020-null-library-and-selection-discipline.md) | The Null Library & Null-Selection Discipline | Accepted | 2026-06-17 |

## Decision Relationships

The following ADRs have explicit dependencies or cross-references:

- **ADR-0001** (modular monolith) is the deployment container for the risk gate (**ADR-0005**), the Demand Manager (**ADR-0011**), and all three front doors (**ADR-0010**).
- **ADR-0002** (no f64) is enforced throughout; referenced by **ADR-0004** (storage column types) and **ADR-0003** (event payloads on the bus).
- **ADR-0003** (NATS JetStream) is the transport layer that **ADR-0009** (quarantine lane), **ADR-0011** (pipeline start/stop signaling), and **ADR-0005** (order intent routing) all depend on.
- **ADR-0007** (format freeze) is a prerequisite for **ADR-0010** (front doors); all three front doors target the frozen format.
- **ADR-0008** (available_time + same builders) is a prerequisite for **ADR-0009** (append-only archive); the archive is the input to the replay that ADR-0008 governs.
- **ADR-0006** (broker architecture) resolves open questions Q-1 and Q-2 from spec/10-open-questions.md.
- **ADR-0007** resolves open question Q-3 from spec/10-open-questions.md.
- **ADR-0011** (demand-driven pipelines) depends on **ADR-0008** (pure function builders) being true; stateful builders would make pipeline stop/restart expensive.
- **ADR-0015** (model format freeze) mirrors **ADR-0007** and is a prerequisite for all Set-H phases.
- **ADR-0016** (distributional forecast contract) extends **ADR-0015** (model format) additively to v1.1; distribution arrays are f64 per **ADR-0002** D-4; σ scaler must be fit on train-only data per **ADR-0017** (no lookahead).
- **ADR-0017** (walk-forward CV & leakage discipline) extends **ADR-0008** (lookahead impossible by construction) from event ordering to cross-validation boundaries, and uses the additive-migrator mechanism of **ADR-0015**; it is the trust foundation for Set I.
- **ADR-0018** (ensemble combination & conformal calibration) builds on **ADR-0016** (σ-unit distributional output) and **ADR-0017** (calibration role); the stacking combiner is trained only on the calibration role to prevent leakage.

## Format

Each ADR follows this structure:

```
# ADR-NNNN: [Title]

**Status:** Accepted | Superseded by ADR-XXXX
**Date:** YYYY-MM-DD
**Deciders:** Platform team

## Context
## Decision
## Rationale
## Consequences
## Alternatives Considered
## References
```
