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
| [ADR-0021](0021-staged-gate-funnel-and-honesty-mechanics.md) | Staged-Gate Funnel, Trial Counter & Holdout Vault | Accepted | 2026-06-17 |
| [ADR-0022](0022-mcp-thin-client-and-internal-agent.md) | MCP Thin Client and Internal Agent | Accepted (internal-agent loop superseded in part by ADR-0024) | 2026-09-06 |
| [ADR-0023](0023-llm-proposes-optimiser-chooses-set-j-judges.md) | The LLM Proposes Structure, an Optimiser Chooses Parameters, Set J Judges | Accepted | 2026-09-08 |
| [ADR-0024](0024-agent-runtime-claude-agent-sdk-per-project-container.md) | Agent Runtime — Claude Agent SDK in a Per-Project Container, behind an LLM Proxy | Accepted | 2026-09-11 |
| [ADR-0025](0025-enforcement-at-platform-services-research-cutoff-and-scopes.md) | Enforce Research Invariants at Platform Services (Cutoff, Desk, Scopes, Report Validation) | Accepted | 2026-09-11 |
| [ADR-0026](0026-two-layer-strategy-representation.md) | Two-Layer Strategy Representation (Python Research + Strategy Language v2) | Accepted | 2026-09-11 |
| [ADR-0027](0027-free-only-data-sources.md) | Free-Only Data Sources for Agent Research | Accepted | 2026-09-11 |
| [ADR-0028](0028-skill-registry-and-platform-side-admission.md) | Skill Registry, Platform-Side Admission, Glossary as a Registry View | Accepted | 2026-09-11 |
| [ADR-0029](0029-single-feature-engine-with-python-bindings.md) | One Feature Engine (Rust) with Python Bindings, Fail-Closed | Accepted | 2026-09-11 |
| [ADR-0030](0030-durable-job-service-and-content-addressed-artifacts.md) | One Durable Job Service and a Content-Addressed Artifact Store | Accepted | 2026-09-11 |
| [ADR-0031](0031-harness-conformance-capability-profiles.md) | Harness Conformance — Capability Profiles, Tool Budgets and Containment | Accepted | 2026-09-11 |
| [ADR-0032](0032-local-tier-governor-loop.md) | The Local Tier — Target Hardware, the GOVERNOR Loop, and the Fence | Accepted | 2026-09-11 |
| [ADR-0033](0033-agent-as-a-conversation.md) | The Agent Is a Conversation, Not a Form | Accepted | 2026-09-12 |

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

- **ADR-0024** (agent runtime) supersedes the internal-agent loop of **ADR-0022**; the MCP thin-client principle stands and becomes one of several thin surfaces over `/api/*`.
- **ADR-0025** (platform-side enforcement) extends **ADR-0019/0020/0021** to a sandboxed agent: the holdout is enforced by the Data API cutoff, trials are counted at job submission (**ADR-0030**), authority by token scope.
- **ADR-0026** (two-layer strategies) supersedes **ADR-0007** as the authoring format; v1 stays readable through a translator. It depends on **ADR-0029** (one feature engine) for its expression language.
- **ADR-0033** (conversation) replaces the run form with a chat, removes the wall clock and iteration cap, and gives every conversation its own folder. It amends **ADR-0032**.
- **ADR-0032** (local tier) implements the Part 3 surface **ADR-0031** deliberately left unimplemented, and supersedes **D-01** as **D-18**.
- **ADR-0028** (skill registry) depends on **ADR-0030** (skill_verify jobs, artifacts) and **ADR-0025** (`skills.admit` never granted to agents).
- **ADR-0027** (free-only data) scopes the sources behind the Data API of **ADR-0025**.

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
