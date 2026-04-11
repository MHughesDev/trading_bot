# Microservice split plan (granular, monorepo-first)

This plan keeps the **same-ish repository shape** (monorepo) while introducing deployable services gradually.
It is intentionally implementation-oriented so work can be executed as small, testable changes.


## Implementation status

- ✅ Phase 1 scaffolding has been started in-repo with `services/*/main.py` + `wiring.py` modules and baseline `/healthz`, `/readyz`, `/status` endpoints.
- ✅ Shared messaging primitives were added under `shared/messaging/` (`envelope.py`, `topics.py`).
- ✅ Phase 2 bus foundations are in place (`bus.py`, `in_memory.py`, `redis_streams.py`, trace helpers), including local retry + dead-letter behavior for in-memory testing and baseline Redis publish/poll wiring.
- ✅ Local Phase 3 handoff wiring is in place for `features -> decision -> risk -> execution` using the in-memory bus.
- ✅ Milestone `decision_risk_service` is added with `/simulate` for local handoff smoke tests.
- ✅ Runtime bridge helper is in place (`services/runtime_bridge.py`) to push feature rows through microservice handoff for in-memory and explicit-poll Redis baselines.
- ✅ Runtime bridge can be enabled in `app/runtime/live_service.py` via `NM_MICROSERVICES_RUNTIME_BRIDGE_ENABLED=true` for shadow handoff publishing.
- ✅ Phase 3 gateway controls now include signed-intent verification, rejection events, fill/position events, and intent-id idempotency in the local execution gateway handler.
- ✅ Execution gateway service now exposes `/ingest/risk-accepted` and `/events/recent` for externalized handoff smoke testing.
- ✅ Risk service now exposes `/ingest/decision-proposal` and `/events/recent` for externalized proposal->risk smoke testing.
- ✅ Decision service now exposes `/ingest/features-row` and `/events/recent` for externalized feature->proposal smoke testing.
- ✅ Runtime bridge can run with **external execution gateway**: set `NM_MICROSERVICES_EXECUTION_GATEWAY_MODE=external`, `NM_MICROSERVICES_RUNTIME_BRIDGE_ENABLED=true`, `NM_MESSAGING_BACKEND=redis_streams`, and `NM_REDIS_URL` (matches `NM_REDIS_URL` / compose Redis). Live loop skips in-process `submit_order` when external mode is on; run `uvicorn services.execution_gateway_service.main:app` (polls `risk.intent.accepted.v1`) for the venue path.
- ⏭️ Next step: wire **real** `ExecutionService.submit_order` inside `execution_gateway_service` handlers (replace scaffold ack/fill) and run paper integration with Redis + live loop + gateway process.

---

## 0) Executive answer: do we need to reorganize folders now?

**No big-bang reorg is required.**

- Keep current top-level domains (`app/`, `data_plane/`, `decision_engine/`, `risk_engine/`, `execution/`, ...).
- Add a `services/` folder with thin entrypoints that compose existing modules.
- Extract process boundaries first; move code only when a boundary stabilizes.

This avoids high-risk churn while still achieving microservice isolation.

---

## 1) Current coupling seams that enable extraction

The current live runtime loop already coordinates independently separable concerns in one process:

- Market-data ingest and normalization.
- Rolling bars and feature generation.
- Decision inference (`DecisionPipeline`).
- Risk gating/signing (`RiskEngine`).
- Venue submission (`ExecutionService`).
- Observability persistence (QuestDB traces).

Because those are already module boundaries, we can convert them into service boundaries with smaller diffs.

---

## 2) Target architecture (logical services)

## 2.1 Market Data Service

**Owns**
- Kraken WS/REST clients, normalizers, product metadata refresh.

**Consumes**
- None (external Kraken only).

**Publishes**
- `market.tick.normalized`
- `market.book.normalized`
- `market.heartbeat`

**SLO focus**
- Freshness, reconnect stability, symbol coverage.

---

## 2.2 Feature Service

**Owns**
- Rolling bar aggregation and feature row computation.

**Consumes**
- `market.tick.normalized`, `market.book.normalized`

**Publishes**
- `features.row.v1`

**SLO focus**
- Deterministic feature schema and low-latency updates.

---

## 2.3 Decision Service

**Owns**
- Forecast + policy inference (`DecisionPipeline` only).

**Consumes**
- `features.row.v1`
- portfolio/risk context snapshot (read-only input)

**Publishes**
- `decision.proposal.v1`
- `decision.diagnostics.v1`

**SLO focus**
- Inference latency and model availability.

---

## 2.4 Risk & Signing Service (authoritative gate)

**Owns**
- Proposal validation, risk precedence, signed `OrderIntent` generation.

**Consumes**
- `decision.proposal.v1`
- freshness/exposure/mode inputs

**Publishes**
- `risk.accepted.v1` (signed intent)
- `risk.blocked.v1` (reason codes)

**SLO focus**
- Correctness and policy enforcement over raw throughput.

---

## 2.5 Execution Gateway Service

**Owns**
- Adapter routing, broker credentials, submit/reconcile.

**Consumes**
- `risk.accepted.v1` only (signed intents)

**Publishes**
- `execution.ack.v1`
- `execution.fill.v1`
- `execution.position.v1`

**SLO focus**
- Exchange API reliability and idempotent order handling.

---

## 2.6 Control Plane Service

**Owns**
- Operator API and mode/param mutation commands.

**Consumes/Publishes**
- RPC + command topics for cross-service configuration/state.

---

## 2.7 Observability Writer Service

**Owns**
- Durable writes to QuestDB and trace sinks.

**Consumes**
- decision/risk/execution events.

**Publishes**
- sink health + lag metrics.

---

## 2.8 Offline Training Service (later)

**Owns**
- retraining / walk-forward / registry workflows.

**Note**
- Generally separate cadence from live trading path.

---

## 3) Monorepo layout proposal (minimal-disruption)

Add this structure without moving existing domain modules immediately:

```text
services/
  market_data_service/
    main.py
    wiring.py
  feature_service/
    main.py
    wiring.py
  decision_service/
    main.py
    wiring.py
  risk_service/
    main.py
    wiring.py
  execution_gateway_service/
    main.py
    wiring.py
  control_plane_service/
    main.py
    wiring.py
  observability_writer_service/
    main.py
    wiring.py
shared/
  messaging/
    topics.py
    envelope.py
  contracts/
    (re-export/bridge from app/contracts)
```

### Rule
- `services/*` should orchestrate only.
- Core business logic stays in existing domain packages until a later optional cleanup phase.

---

## 4) Granular migration phases (ticket-ready)

## Phase 1 — Service scaffolding only (no behavior change)

1. Add `services/` package + one `main.py` per service.
2. Each service entrypoint simply calls existing module code.
3. Keep existing `python -m app.runtime.live_service` runnable.

**Exit criteria**
- Current integration tests still pass.
- New service entrypoints can start and health-check.

---

## Phase 2 — Introduce messaging abstraction

1. Add `shared/messaging/envelope.py` with event envelope fields:
   - `event_id`, `event_type`, `event_version`, `symbol`, `ts_event`, `ts_ingest`, `producer`, `payload`.
2. Add `shared/messaging/topics.py` constants.
3. Implement local adapter (in-memory / Redis Streams backend behind one interface).

**Exit criteria**
- Live path can run with in-memory bus in local mode.
- Event contracts validated with pydantic models.

---

## Phase 3 — Extract execution gateway first

1. Move order submit + reconcile loop behind `execution_gateway_service` API/topic consumer.
2. Keep risk in-process for now but call gateway externally.
3. Enforce signed intent requirement at gateway ingress.

**Exit criteria**
- End-to-end paper flow works with externalized execution process.
- No direct adapter calls outside gateway.

---

## Phase 4 — Extract risk/signing

1. Decision output becomes proposal event (`decision.proposal.v1`).
2. Risk service consumes proposal + context, emits accepted/blocked events.
3. Execution consumes only `risk.accepted.v1`.

**Exit criteria**
- Risk precedence unchanged.
- Block reason metrics preserved.

---

## Phase 5 — Extract decision service

1. Decision inference runs in its own process with model loading lifecycle.
2. Inputs: `features.row.v1` + context snapshots.
3. Outputs: proposal + diagnostics.

**Exit criteria**
- Replay/live parity tests still pass against same decision version.

---

## Phase 6 — Extract market + feature services

1. Move Kraken ingest loops to market-data service.
2. Move rolling bar and feature transforms to feature service.
3. Partition by symbol for horizontal scaling.

**Exit criteria**
- Feature parity validated against baseline snapshots.
- Freshness SLO alerts available.

---

## Phase 7 — Split control plane and observability writers

1. Control plane becomes command/status gateway for all services.
2. Observability writer consumes events asynchronously.

**Exit criteria**
- Operator endpoints report per-service health/degradation.

---

## 5) Event contract catalog (v1)

## 5.1 `features.row.v1`
- `symbol: str`
- `timestamp: datetime`
- `feature_row: dict[str, float]`
- `spread_bps: float`
- `mid_price: float`

## 5.2 `decision.proposal.v1`
- `symbol: str`
- `route_id: str`
- `direction: int`
- `size_fraction: float`
- `order_type: str`
- `diagnostics: dict[str, Any]`

## 5.3 `risk.accepted.v1`
- Signed `OrderIntent` payload + signature metadata.

## 5.4 `risk.blocked.v1`
- Proposal reference + `blocked_reason` + risk snapshot fields.

## 5.5 `execution.ack.v1`
- Venue ack id, submit timestamp, status.

---

## 6) Mapping current modules to services

- Market Data Service → `data_plane/ingest/*`, `data_plane/ingest/product_cache.py`
- Feature Service → `data_plane/bars/*`, `data_plane/features/*`, `decision_engine/feature_frame.py`
- Decision Service → `decision_engine/pipeline.py`, `forecaster_model/*`, `policy_model/*`
- Risk Service → `risk_engine/*`
- Execution Gateway → `execution/*`
- Control Plane Service → `control_plane/api.py`
- Observability Writer → `data_plane/storage/questdb.py`, `observability/*`
- Training Service → `orchestration/*`, `models/registry/*`

---

## 7) CI/CD and testing changes (documentation target)

Add dedicated checks as services appear:

1. **Contract tests**: schema compatibility per event version.
2. **Service smoke tests**: each `services/*/main.py` boots with test config.
3. **End-to-end paper test**: feature → decision → risk → execution (paper adapter).
4. **Replay parity test**: replay outputs remain aligned with live decision semantics.

Keep existing repository checks mandatory:
- `ruff check .`
- `python3 -m pytest tests/ -q`
- `bash scripts/ci_spec_compliance.sh`
- `bash scripts/ci_mlflow_promotion_policy.sh`

---

## 8) Non-negotiables during migration

- Kraken remains the only market data source.
- Risk remains the final gate before execution.
- Execution gateway rejects unsigned intents in production.
- Replay must keep using equivalent decision semantics/version as live.
- Adapter-specific broker logic remains inside execution adapters.

---

## 9) Risks and mitigations

1. **Risk:** Event schema drift.
   - **Mitigation:** versioned contracts + compatibility tests.
2. **Risk:** Latency regressions from network hops.
   - **Mitigation:** colocate services first, baseline p95 budgets before/after split.
3. **Risk:** State inconsistency across services.
   - **Mitigation:** authoritative ownership table + idempotent consumers.
4. **Risk:** Hidden bypass paths to brokers.
   - **Mitigation:** static checks and runtime rejection at gateway.

---

## 10) Minimal deliverable set (first production milestone)

Deliver these three services first (still monorepo):

1. `market_data_service`
2. `decision_risk_service` (combined initially)
3. `execution_gateway_service`

Then split `decision_risk_service` into separate decision + risk services only if throughput/compliance needs justify the extra complexity.

---

## 11) Definition of done (program-level)

This migration is considered complete when:

1. All live trading paths use inter-service contracts (not direct in-process calls across boundaries).
2. Risk signing authority is isolated and enforced.
3. Execution credentials are isolated to execution gateway.
4. Control plane surfaces per-service health, mode, and lag.
5. Replay parity checks pass on every release.


---

## 12) Communication layers (fully enumerated)

This section defines the complete communication stack so each phase can implement the same patterns.

## 12.1 Transport matrix

| Path | Pattern | Candidate tech | Delivery target |
|---|---|---|---|
| Market Data → Feature | Async event stream | Redis Streams (start), Kafka/NATS (optional later) | At-least-once |
| Feature → Decision | Async event stream | Redis Streams | At-least-once |
| Decision → Risk | Async event stream | Redis Streams | At-least-once |
| Risk → Execution | Async event stream | Redis Streams | At-least-once |
| Control Plane → Services | Sync RPC + command event | HTTP/gRPC + command topic | Request/response + eventual consistency |
| Services → Observability Writer | Async event stream | Redis Streams/Kafka | At-least-once |
| Service health probes | Sync HTTP | `/healthz` + `/readyz` | Best-effort |

## 12.2 Standard event envelope

All async messages should use one common envelope shape:

- `event_id` (UUID, required)
- `event_type` (string, required)
- `event_version` (string, required; e.g. `v1`)
- `trace_id` (string, required)
- `correlation_id` (string, optional but recommended)
- `producer_service` (string, required)
- `symbol` (string, optional)
- `partition_key` (string, optional)
- `ts_event` (UTC timestamp, required)
- `ts_ingest` (UTC timestamp, required)
- `payload` (typed object, required)
- `schema_hash` (string, optional safety checksum)

## 12.3 Topic naming convention

- `<domain>.<entity>.<event>.<version>`
- Examples:
  - `market.tick.normalized.v1`
  - `features.row.generated.v1`
  - `decision.proposal.created.v1`
  - `risk.intent.accepted.v1`
  - `execution.order.ack.v1`

## 12.4 Consumer-group strategy

- One consumer group per downstream service.
- Symbol-based partition key for ordering (`BTC/USD`, etc.).
- Horizontal scaling by adding consumers in same group.

## 12.5 Reliability requirements

- Retries: exponential backoff with jitter.
- Dead-letter queue per topic family.
- Poison message threshold and quarantine policy.
- Idempotency key (`event_id`) persisted in consumer cache/store.
- Outbox pattern for services that both write DB state and emit events.

## 12.6 Ordering and dedup rules

- Ordering guaranteed **within partition key only**.
- Consumers must dedupe by `event_id`.
- Replays/backfills must tag `source_mode` (`live|replay`).

## 12.7 Security/auth for service communication

- Internal auth: mTLS or signed service tokens.
- Network policy: only required service-to-service paths open.
- Secret storage: runtime env/secret manager, never in repo.
- Risk/Execution path requires signature verification before order submit.

## 12.8 Timeout and circuit breaker defaults

- RPC timeout budget per call class (`control`, `health`, `state`).
- Circuit breakers around external APIs and broker endpoints.
- Bulkheads: isolate market-data failures from execution path.

## 12.9 Contract governance

- Pydantic schema per event type in `shared/contracts` (or bridge from `app/contracts`).
- Backward-compatible additions only in minor revisions.
- Breaking changes require new event version (`v2`) and dual publish window.
- Contract test suite in CI for producer + consumer fixtures.

## 12.10 Telemetry for communication layers

- Metrics:
  - publish rate, consume lag, ack latency, retry count, DLQ depth.
- Tracing:
  - propagate `trace_id` across all events/RPC hops.
- Logging:
  - structured logs with `trace_id`, `event_id`, `service`, `symbol`.

---

## 13) Service API/event inventory (what each service exposes)

## 13.1 Market Data Service

**Produces topics**
- `market.tick.normalized.v1`
- `market.book.normalized.v1`
- `market.heartbeat.v1`

**HTTP endpoints**
- `GET /healthz`
- `GET /readyz`
- `GET /status` (feed lag, connected symbols)

## 13.2 Feature Service

**Consumes topics**
- `market.tick.normalized.v1`
- `market.book.normalized.v1`

**Produces topics**
- `features.row.generated.v1`

**HTTP endpoints**
- `GET /healthz`
- `GET /readyz`
- `GET /status` (feature freshness by symbol)

## 13.3 Decision Service

**Consumes topics**
- `features.row.generated.v1`
- `portfolio.snapshot.v1` (optional input stream)

**Produces topics**
- `decision.proposal.created.v1`
- `decision.diagnostics.generated.v1`

**HTTP endpoints**
- `GET /healthz`
- `GET /readyz`
- `GET /status` (model versions, inference p95)

## 13.4 Risk Service

**Consumes topics**
- `decision.proposal.created.v1`
- `control.mode.changed.v1`

**Produces topics**
- `risk.intent.accepted.v1`
- `risk.intent.blocked.v1`

**HTTP endpoints**
- `GET /healthz`
- `GET /readyz`
- `GET /status` (block counters, mode)

## 13.5 Execution Gateway Service

**Consumes topics**
- `risk.intent.accepted.v1`

**Produces topics**
- `execution.order.ack.v1`
- `execution.order.fill.v1`
- `execution.position.snapshot.v1`

**HTTP endpoints**
- `GET /healthz`
- `GET /readyz`
- `GET /status` (venue connectivity, pending orders)

## 13.6 Control Plane Service

**Consumes topics**
- `*.status.*` (optional)

**Produces topics**
- `control.mode.changed.v1`
- `control.params.updated.v1`

**HTTP endpoints**
- Existing control plane endpoints + service aggregation endpoints.

## 13.7 Observability Writer Service

**Consumes topics**
- `decision.*`
- `risk.*`
- `execution.*`

**Produces topics**
- `observability.write.error.v1` (optional)

**HTTP endpoints**
- `GET /healthz`
- `GET /readyz`
- `GET /status` (sink lag and write success rate)

---

## 14) Data ownership and state authority

| Data/State | Owner service | Storage of record | Replicas/caches |
|---|---|---|---|
| Market normalized events | Market Data | Stream log | Feature cache |
| Feature rows | Feature | Stream log / short-term cache | Decision cache |
| Proposals/diagnostics | Decision | Stream log | Observability sink |
| Risk mode + gating outputs | Risk | Redis + event log | Control-plane read model |
| Signed intents | Risk | Event log + audit table | Execution memory |
| Order/fill lifecycle | Execution | Venue + local order store | Risk exposure cache |
| Operator params | Control Plane | Redis/config store | Service local cache |
| Traces/metrics archive | Observability | QuestDB/Prometheus/Loki | Grafana queries |

**Ownership rule:** one writer of record per state domain; all others consume read models.

---

## 15) Phase-by-phase deliverables checklist (fully enumerated)

## Phase 1 checklist

- [ ] Create `services/*/main.py` + startup wiring.
- [ ] Add `/healthz` and `/readyz` to each service.
- [ ] Add process manager config (docker compose service entries).
- [ ] Add startup/shutdown logging with service name/version.
- [ ] Add smoke test per service process boot.

## Phase 2 checklist

- [ ] Implement shared envelope and topic constants.
- [ ] Add message bus abstraction + in-memory adapter.
- [ ] Add Redis Streams adapter.
- [ ] Implement producer/consumer middleware for trace IDs.
- [ ] Add DLQ topics and retry policies.
- [ ] Add contract tests for all v1 events.

## Phase 3 checklist (execution extraction)

- [ ] Route signed intents to execution gateway topic/API.
- [ ] Block unsigned intents at gateway ingress.
- [ ] Emit ack/fill/position events.
- [ ] Add idempotency protection on submit path.
- [ ] Run end-to-end paper test with external execution process.

## Phase 4 checklist (risk extraction)

- [ ] Decision emits proposal event only.
- [ ] Risk consumes proposals and emits accepted/blocked events.
- [ ] Risk mode commands received via control topics.
- [ ] Preserve risk precedence and block reason codes.
- [ ] Add replay parity checks with external risk service.

## Phase 5 checklist (decision extraction)

- [ ] Decision service loads configured models at startup.
- [ ] Emits proposal + diagnostics topics.
- [ ] Adds inference latency and model-version metrics.
- [ ] Supports hot-reload/restart strategy for model changes.

## Phase 6 checklist (market + feature extraction)

- [ ] Market-data service publishes normalized events with lag metrics.
- [ ] Feature service emits deterministic feature rows.
- [ ] Symbol partitioning strategy documented and tested.
- [ ] Feature parity snapshot tests pass.

## Phase 7 checklist (control + observability split)

- [ ] Control plane aggregates per-service status endpoints.
- [ ] Observability writer consumes domain topics and writes sinks.
- [ ] Dashboards updated for per-service latency/lag/error views.
- [ ] Incident runbook updated for microservice topology.

---

## 16) Deployment topology progression

## Stage A (local)
- Single docker-compose, all services on one network, Redis Streams bus.

## Stage B (staging)
- Separate service containers with resource limits and autoscaling hints.

## Stage C (production)
- Isolated service groups, network policies, secret manager integration, canary rollouts.

---

## 17) Rollback strategy

- Keep a feature flag to run legacy in-process orchestration path during migration.
- For each extraction phase, support rollback to previous phase without schema loss.
- Maintain dual-publish/dual-consume window during version upgrades.
- Freeze broker-impacting releases behind paper-trading soak tests.

---

## 18) Runbook additions required before each phase gate

- Startup order and dependency matrix.
- Failure mode matrix (service down, stream lag, broker timeout).
- Manual failover/flatten procedure with control-plane commands.
- Event replay procedure from last stable offset.
- SLA/SLO alert thresholds and pager ownership.

