# Phase 4 — Inference & Strategy Integration

**Completion: 0% (0 / 8 tasks)**

**Goal:** Connect the Studio to the thing it exists for — **strategies**. Build
the inference gateway (alias resolution, caching, fallback, traces, rate limits),
implement the `model_forecast` evaluator the strategy runtime is currently
missing, wire the existing AI Forecast node to real registered models, and derive
"used-by." After this phase, a strategy with an AI Forecast node actually
consults a trained, promoted model at runtime.

**Depends on:** Phases 0–3 (serves the `production` alias). **Blocks:** nothing —
this closes the loop.

---

## Design — the runtime path

```
strategy-runtime evaluates a `model_forecast` condition
        │  { model: "egogemma-core"|slug, alias?: "production", direction, min_confidence }
        ▼
crates/model-registry  InferenceGateway::forecast(model_ref, instrument, features)
        │  1. resolve alias → version (Redis hot map, Postgres fallback)
        │  2. cache check (Redis, short TTL, keyed by version+feature_hash)
        │  3. miss → inference sidecar predict()  (or rust runtime if runtime=rust, D-5)
        │  4. write trace (ClickHouse), update cost/latency counters
        │  5. on sidecar failure → `fallback` alias, else neutral/abstain
        ▼
returns domain::Forecast → condition = (direction matches) && (confidence ≥ min)
```

**Safety invariant (ADR-0005).** A model's `Forecast` is *advice consumed by a
strategy condition*. It can gate or size an intent, but every resulting order
still passes the single risk-gate chokepoint unchanged. The Studio adds no path
that places or sizes an order outside the risk gate.

---

## Tasks

### ☐ H-4.1 `InferenceGateway` in `model-registry` — L
`InferenceGateway { registry, sidecar, redis, traces }` with
`forecast(model_ref, instrument, features) -> Forecast`,
`score_universe(...)`, `decide(...)`, `complete_llm(...)` per kind. Alias
resolution (Redis hot `model→alias→version` map, invalidated on promote/rollback),
short-TTL prediction cache, fallback-alias failover, and circuit-break to
abstain on repeated sidecar failure.
**Acceptance:** resolves `production` to the promoted version; cache hit avoids a
sidecar call; promoting a new version invalidates the hot map within one tick;
sidecar-down falls back or abstains (never blocks the trading loop).

### ☐ H-4.2 Strategy format: add `model_forecast` condition (v1.1) — M
The v1.0 strategy definition (ADR-0007) has **no** `model_forecast` node — the
frontend already emits one, but `crates/domain/src/strategy_def/nodes.rs` cannot
represent it. Add it as a **versioned, additive** extension: bump strategy
`schema_version` to `1.1`, add the `ModelForecast` condition variant
`{ model_ref, alias, direction, min_confidence }`, and provide a v1.0→v1.1
migration (additive, lossless). Record this in an ADR addendum (ADR-0007 evolution
note) per the format's own evolution rule.
**Acceptance:** strategy validator accepts a `model_forecast` condition; existing
v1.0 strategies still load unchanged; the frontend compiler output round-trips
through the domain type.

### ☐ H-4.3 `model_forecast` evaluator in strategy-runtime — L
Implement evaluation of the `ModelForecast` condition: pull the instrument's
current features (same builders, live + replay, ADR-0008), call
`InferenceGateway::forecast`, apply `direction` + `min_confidence`, return the
boolean. Works identically in live and backtest (the backtest harness, Phase 3.4,
uses the same evaluator against the same gateway pointed at the eval version).
**Acceptance:** a strategy with an AI Forecast node produces different entries when
the underlying model version changes; live and replay agree for the same inputs;
an abstaining gateway yields a deterministic, documented condition result (default
`false`, never an error that halts the strategy).

### ☐ H-4.4 Wire AI Forecast node to real models (contract side) — S
Back the existing `frontend/src/nodes/AIForecastNode.tsx` model dropdown with
`GET /api/models?kind=forecaster&asset_class=…` (the node currently hardcodes
`price_forecaster`). Define the response shape the node needs (id, slug,
display_name, production version, status). The actual React change lands in
Phase 5; this task freezes the contract + the `assetApi.models` payload.
**Acceptance:** the endpoint returns the fields the node binds to; a model with no
`production` alias is shown as unselectable/disabled (not silently broken).

### ☑ H-4.5 Used-by derivation — M
**Status: DONE (2026-10-04, track F-UI).** `ModelManager::get_used_by` now selects from `strategy_definitions` by JSONB containment of `{model_ref: <id or slug>}` on `definition_json->'nodes'`, served by a GIN `jsonb_path_ops` index (migration `0200_strategy_nodes_gin.sql`), returns `running_instances` from `strategy_instances`, and a failed read is an error (422), no longer an empty list. Evidence: `crates/model-registry/tests/pg_training_curve.rs::a_strategy_that_names_the_model_by_id_or_slug_is_used_by_it` (run against a scratch DB through 0201: matches by id and by slug, does not leak another model's strategy, drops out when the reference is removed). Mutation-checked: probing `model_refx` instead of `model_ref` fails the test with `by id: []`. Not done: the route does not scope strategies to the caller (strategies and models are owned separately and the route never did); ADR row H-ADR-4.5a below.

**History -- PARTIAL (2026-10-03) -- the route is broken, not merely unindexed.** `GET /api/models/{id}/used-by` exists, but `ModelManager::get_used_by` (`crates/model-registry/src/manager.rs`) selects `strategy_id, display_name, definition_json FROM strategies`; no table named `strategies` exists (the tables are `strategy_definitions`, `strategy_instances`, ...), and the error is swallowed by `.unwrap_or_default()`, so every call answers an empty list -- "not used by anything" for a model that is. It is also a `definition_json::text LIKE '%model_id%'` scan with LIMIT 100, not an indexed path or join table, and ignores the live-instance count. The frontend now reads the served `{strategy_id, display_name}` shape (`normaliseUsedBy`), so it lights up when the query is fixed. Fix needed: query `strategy_definitions` (+ `strategy_instances` for the count), match a `model_forecast` condition on slug/id through a maintained index, and stop swallowing the error.

`GET /api/models/{id}/used-by`: scan `strategy_definitions.definition_json`
(JSONB) for `model_forecast` conditions referencing the model's slug/id; return
the strategies (+ live instance count if running). Index/materialize for the
command-center "used by" chips.
**Acceptance:** creating a strategy that references the model makes it appear in
used-by; removing the reference removes it; query is bounded (indexed JSONB path
or a maintained join table), not a full scan per request.

### ◐ H-4.6 Inference traces + cost/latency rollups — M
**Status: PARTIAL (2026-10-03).** The gateway writes an `inference_trace` row into `model_events` (version, instrument, latency_ms, result in hit/miss/abstain/error) and `GET .../traces` returns them; there is no ClickHouse `model_traces` row, no cost field, and no maintained per-version rollup. The cockpit Health panel now rolls p95 latency (over real sidecar calls only) and error rate (over attempts only, abstentions excluded) from the traces served, labelled with the window ("last N traces"), and says "not measured" with no traces; cost per 1k is not shown because no trace carries cost. Evidence: `frontend/src/components/mlops/__tests__/studio.helpers.test.ts` (traceStats, 2), `frontend/src/components/mlops/__tests__/the-cockpit-overview-reads-what-the-registry-serves.test.tsx` (4).

Every gateway call writes a `model_traces` row (Phase 0 ClickHouse:
latency_ms, cost_usd, input/output hash, status) and updates per-version rollups
(`error_rate`, `p95_latency`, `cost_per_1k`) surfaced on the model card + overview
health panel. `GET …/traces` returns recent traces for the Test Lab/overview.
**Acceptance:** a burst of inferences shows in `/traces` and moves the rollups;
rollups match a manual aggregation; decimal cost stored as string (ADR-0002).

### ◐ H-4.7 Rate limiting + permissions on inference — S
**Status: PARTIAL (2026-10-03).** Only the Test Lab route has a limit: a process-local in-memory window (20 calls/min per user+model) in `routes/models.rs` that answers 429 with `retry_after_secs` -- not a Redis token bucket, not shared across instances, and not applied to gateway/strategy inference. Ownership is enforced on the registry reads (`ensure_model_owned`), but "a deployed production model is resolvable by the strategy runtime regardless of caller" and provider 429 pass-through are not evidenced. Frontend: the Test Lab surfaces the failure text; no 429-specific retry hint UI.

Per-user/per-model inference rate limits (Redis token bucket) and the user-scope
check (only the owner, or a deployed `production` model, is callable). Adapters
honour the provider's own limits and surface 429s cleanly.
**Acceptance:** exceeding the limit returns 429 with a retry hint, not a crash;
a non-owner cannot Test-Lab a private draft; a deployed production model is
resolvable by the strategy runtime regardless of caller.

### ◐ H-4.8 Nightly retrain orchestration (scheduled job) — M
**Status: PARTIAL (2026-10-04, track F-UI) -- now reached; train -> eval -> gate is still not scheduled.** `RetrainScheduler` is spawned from `apps/platform/src/main.rs` (separate block after the interrupted-run recovery) with period `TBOT_RETRAIN_PERIOD_HOURS` (default 24, `0` disables, first tick one full period after boot so a restart does not retrain). It only starts training for models flagged `auto_retrain`, skips a model with a run already queued or running, and logs a failed read instead of swallowing it; nothing it starts is promoted, so a scheduled run cannot reach `production` without the promotion gate. The candidate's `version_created` event is on the new events route (`GET /api/models/{id}/events`), which is the activity-feed source. **Still missing:** the evaluation step after training and the gated promotion; no test drives `run_cycle` (needs a live sidecar). ADR row H-ADR-4.8a.

**History -- PARTIAL (2026-10-03) -- built but unreached.** `crates/model-registry/src/scheduler.rs` defines `RetrainScheduler` (polls `auto_retrain` models on an interval and starts training), but nothing in `apps/` or `crates/` constructs or spawns it (grep for `RetrainScheduler` finds only its own file), so no scheduled run ever happens. It also has no eval and no gated promotion step, and the activity feed has no source for scheduled runs. Needs: spawn from the platform, then train -> eval -> gate, with events readable.

Port the legacy `nightly_retrain.py` *cadence* into a Rust scheduler that, on a
configurable schedule, kicks training → eval → (gated) promotion for models
flagged `auto_retrain`, emitting the same events/traces as manual runs. No
auto-promotion to **live** without a passing gate (ADR-0005 spirit; legacy CI
guard preserved).
**Acceptance:** a model flagged `auto_retrain` produces a fresh evaluated
candidate on schedule; it never reaches live `Active` without passing the gate;
the run is visible in the model's activity feed.

---

## Phase 4 exit criteria

- Strategies consult real, promoted models at runtime through the inference
  gateway, identically in live and backtest.
- The pre-existing AI Forecast node + `GET /assets/models/{symbol}` are fully
  backed (no stubs remain in the model path).
- Used-by, traces, cost/latency rollups, rate limits, and scheduled retrain are
  live; the risk gate remains the sole order chokepoint.

## ADR rows (track F-UI, 2026-10-04)

| ID | Decision | Rejected alternative |
|----|----------|----------------------|
| H-ADR-4.5a | Used-by matches `model_ref` on a node (id or slug) by JSONB containment on `definition_json->'nodes'` with a GIN index, and does not scope by caller. | A materialised join table maintained on every strategy write: it adds a write path to the strategy store (Set U territory) for a read that the index already bounds. |
| H-ADR-4.8a | The scheduler is spawned unconditionally with a 24 h default and an env off switch; it starts training only and never promotes. | Opt-in by env: a scheduler nobody switched on is the built-but-unreached defect again, and the `auto_retrain` flag on the model is already the per-model opt-in. |
| H-ADR-5.5a | Per-epoch losses are persisted to `mlops.training_metric` (migration 0201) from the progress frames, and served by a polled route. | Folding history into the in-memory run snapshot: lost on restart, and the snapshot is a hot read. A WebSocket consumer for `models.jobs` was not added: polling at 2 s while live is adequate and the lane has no auth story yet. |
