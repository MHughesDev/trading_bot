---
Type: Formal
Status: Complete
Completed: 2026-06-10
Derived From: C-061, C-072, C-082, C-084, C-085, C-100, C-113, C-117
---

# Phase 3 — Strategy System

> **Self-contained execution doc.** You need only: this file, [`../../architecture.md`](../../architecture.md),
> the specs (especially
> [`../../specs/DATA-004-strategy-definition-format.md`](../../specs/DATA-004-strategy-definition-format.md)
> and [`../../specs/FEAT-001-strategy-system.md`](../../specs/FEAT-001-strategy-system.md)), and the
> existing codebase. `crates/strategy-runtime` (WorldState/interpreter/instance lifecycle, partial
> node graph), `crates/strategy-validator`, the frozen `StrategyDefinition` v1.0 in `crates/domain`,
> and the `DataType` enum (Phase 1) are your foundation — read them first. Phase 1 must be complete.

## Phase goal

After this phase the **strategy system reaches its end-state semantics**: every strategy compiles into
a **capability manifest** at save time; strategy **kind is inferred** from the presence of an
execution block (no declared `strategy_type`); the **pipeline automation runtime** maintains stateful
per-instrument stage membership and executes on rising edges with an idempotency key; the v1.5
builder nodes (data-source selector, rank/sort/filter/take-top-N, alert/surface action) exist; the
**apply list filters** strategies to those compatible with the selected asset/venue/universe; and a
**default 7/21 EMA discovery strategy** seeds at account creation.

## Prerequisites

- Phase 1 done: `DataType` enum, 8-class `AssetClass`, `SupportedVenue`, registries.
- `crates/strategy-runtime` and `crates/strategy-validator` compile; `StrategyDefinition` v1.0 frozen.
- May run in parallel with Phase 2.

## Invariants this phase must respect

- **`StrategyDefinition` v1.0 is frozen** — do not change its schema version; kind inference and the
  manifest are *derived*, not new declared fields on the frozen format (C-061).
- **No risk UI** (C-114): strategies have no user-facing risk overrides.
- **Minimum source data** (C-112): the default strategy and manifest compilation assume 1-min OHLCV is
  the baseline; cross-asset compatibility is computed against provided/required capabilities.
- **Idempotency where automation acts** (C-085): rising-edge execution dedups by
  `(automation_id, instrument_id, stage_id, signal_epoch)`.
- **Same builders/features live** — the runtime consumes pure builders/features; no lookahead.

---

## Tasks

### P3-T01 — Strategy kind inference (remove declared `strategy_type`)
- **Goal:** Strategy kind (`Discovery` vs `Execution`) is computed from the strategy graph, never
  declared.
- **Files:** `crates/strategy-runtime/src/kind.rs` (new), `crates/strategy-validator/src/lib.rs`,
  and wherever `strategy_type` is currently read (grep `strategy_type`).
- **Context:** Per C-061. Rule: if the strategy graph contains an **execution block** node →
  `StrategyKind::Execution` (runs in Automations); otherwise → `StrategyKind::Discovery` (populates
  scanner panels). Remove any declared `strategy_type` field usage; if it exists in stored JSON,
  ignore it and infer. Add `fn infer_kind(def: &StrategyDefinition) -> StrategyKind`.
- **Acceptance:** `crates/strategy-runtime/tests/kind_inference.rs` proves a graph with an execution
  block infers `Execution`, one without infers `Discovery`, and a stored `strategy_type` field does
  not override the inferred value.
- **Depends on:** Phase 1.

### P3-T02 — Capability manifest compiler
- **Goal:** Compile a strategy definition into a capability manifest at save time.
- **Files:** `crates/strategy-runtime/src/manifest.rs` (new),
  `migrations/0009_strategy_manifests.sql` (new), `crates/storage/src/strategy_manifest.rs` (new).
- **Context:** Per C-082. At save, the backend computes a `CapabilityManifest { required_lanes:
  Vec<DataType>, required_primitives, required_features, evaluation_trigger, strategy_kind }` by
  walking the strategy graph (data-source nodes → `required_lanes`; feature nodes → `required_features`;
  trigger node → `evaluation_trigger` ∈ {`bar_close`,`tick`,`quote`,`event`,`scheduled`}; kind from
  P3-T01). Persist into `strategy_manifests(strategy_id UUID PK, required_lanes JSONB,
  required_primitives JSONB, required_features JSONB, evaluation_trigger TEXT, strategy_kind TEXT,
  compiled_at TIMESTAMPTZ)`.
- **Acceptance:** `crates/strategy-runtime/tests/manifest_compile.rs` proves the default 7/21 EMA
  strategy compiles to a manifest with `required_lanes = [market.ohlcv]`,
  `evaluation_trigger = bar_close`, `strategy_kind = Discovery`; a strategy with an execution block
  compiles to `Execution`.
- **Depends on:** P3-T01.

### P3-T03 — Apply-list compatibility filtering
- **Goal:** Given an asset/venue/universe selection, return only strategies whose manifest is
  satisfied by the instrument's provided capabilities (incompatible ones hidden, not flagged).
- **Files:** `crates/strategy-runtime/src/compatibility.rs` (new),
  `crates/api/src/routes/strategies.rs` (extend with an apply-list endpoint).
- **Context:** Per C-113/C-117. Instrument metadata declares `provided_lanes`, `provided_primitives`,
  etc. (from the venue/collector capability registry). Compatibility is computed **dynamically at
  assignment**: a strategy is compatible iff every `required_lane`/`required_primitive`/
  `required_feature` in its manifest is in the instrument's provided set. `GET
  /api/strategies/apply-list?instrument=…&venue=…` returns only compatible strategies. Incompatible
  strategies are **omitted** from the response (not returned-with-flag).
- **Acceptance:** `crates/strategy-runtime/tests/apply_list_filter.rs` proves a strategy requiring
  `market.funding_rate` is omitted for a spot-equity instrument that provides only `market.ohlcv`,
  while the 7/21 EMA strategy (requires only `market.ohlcv`) appears for every asset class.
- **Depends on:** P3-T02.

### P3-T04 — Default 7/21 EMA discovery strategy seed
- **Goal:** Every new account is seeded with the default cross-asset discovery strategy.
- **Files:** `crates/api/src/accounts/seed.rs` (new or extend account-creation handler),
  a strategy-definition JSON fixture under `crates/strategy-runtime/fixtures/default_ema.json` (new).
- **Context:** Per C-072. The default strategy: 7-period EMA crossing over 21-period EMA, **1-min
  OHLCV only**, no execution block (→ `Discovery`), cross-asset compatible (manifest requires only
  `market.ohlcv`). On account creation, insert this strategy for the user and compile its manifest
  (P3-T02).
- **Acceptance:** `crates/api/tests/account_seed.rs` proves a freshly created account owns exactly one
  strategy whose compiled manifest requires only `market.ohlcv` and whose kind is `Discovery`.
- **Depends on:** P3-T02.

### P3-T05 — `AutomationPlan` model + schema
- **Goal:** The data model and Postgres schema for single-instrument and pipeline automations.
- **Files:** `crates/strategy-runtime/src/automation/plan.rs` (new),
  `migrations/0010_automations.sql` (new), `crates/storage/src/automation.rs` (new).
- **Context:** Per C-084. `AutomationPlan` has two shapes: **SingleInstrument** (asset_class,
  instrument_id, execution_strategy_id, time_window) and **Pipeline** (asset_class, universe, ordered
  `stages: Vec<FilterStage>`, one final `execution_action`). A `FilterStage` references a discovery
  strategy/filter applied to the running universe. `time_window` is asset-class-aware (24/7 vs
  sessioned). Schema: `automations(id UUID PK, user_id, kind TEXT, account_mode TEXT, spec JSONB,
  armed BOOLEAN, created_at)` and `automation_stage_membership(automation_id, stage_id, instrument_id,
  entered_at, PRIMARY KEY(automation_id, stage_id, instrument_id))`.
- **Acceptance:** migration applies; `crates/storage/tests/automation_crud.rs` proves a pipeline plan
  with 3 stages round-trips through the DB and the membership table accepts/rejects on its composite
  key.
- **Depends on:** P3-T02.

### P3-T06 — Pipeline runtime: stateful stage membership
- **Goal:** A stateful runtime that recomputes per-instrument stage membership on each evaluation
  trigger and tracks enter/exit deltas.
- **Files:** `crates/strategy-runtime/src/automation/pipeline.rs` (new).
- **Context:** Per C-084/C-085. On each `evaluation_trigger` (`bar_close`/`tick`/`quote`/`event`/
  `scheduled`), the runtime re-evaluates each pipeline stage over its input universe, updates the
  `automation_stage_membership` set (additions = enter delta, removals = exit delta), and flows
  instruments that clear the final stage into the execution action. Membership is **stateful** —
  deltas are computed against the prior set. Counts and deltas are exposed for the UI stage board
  (Phase 6).
- **Acceptance:** `crates/strategy-runtime/tests/pipeline_membership.rs` proves: an instrument that
  newly passes stage 1 produces an enter delta; one that stops passing produces an exit delta; an
  instrument clearing the final stage is handed to the execution action exactly once per crossing.
- **Depends on:** P3-T05.

### P3-T07 — Rising-edge execution + idempotency key
- **Goal:** The pipeline/single-instrument automations emit orders on rising edges only, deduped by a
  stable idempotency key.
- **Files:** `crates/strategy-runtime/src/automation/edge.rs` (new), wired into the execution router
  hand-off.
- **Context:** Per C-085. The idempotency key is `(automation_id, instrument_id, stage_id,
  signal_epoch)`. A signal fires only on the **rising edge** (false→true transition of the final
  condition), never while it stays true. A redelivered/recomputed signal with a seen key is a no-op.
  All automated orders flow through the strategy runtime and execution router with **no user
  confirmation** once armed.
- **Acceptance:** `crates/strategy-runtime/tests/rising_edge.rs` proves: a condition staying true
  across N evaluations emits exactly one order; the condition going false then true again emits a
  second order with a new `signal_epoch`; a replayed signal with a seen key emits nothing.
- **Depends on:** P3-T06.

### P3-T08 — v1.5 builder nodes (backend semantics)
- **Goal:** Backend node definitions + interpreter support for the v1.5 nodes: data-source selector,
  rank/sort, filter, take-top-N, and an alert/surface action.
- **Files:** `crates/strategy-runtime/src/nodes/` (extend: `data_source.rs`, `rank.rs`, `filter.rs`,
  `take_top_n.rs`, `surface_action.rs`), update the interpreter dispatch.
- **Context:** Per C-100. These nodes enable discovery/pipeline strategies: **data-source selector**
  chooses which `DataType` lane feeds the graph (and thus a manifest `required_lane`); **rank/sort**
  orders the universe by a feature; **filter** keeps instruments meeting a predicate; **take-top-N**
  keeps the top N after ranking; **alert/surface action** is the discovery output that populates a
  scanner panel (a non-execution terminal action). The presence of `surface_action` (and absence of an
  execution block) keeps the strategy `Discovery`.
- **Acceptance:** `crates/strategy-runtime/tests/v15_nodes.rs` proves a graph
  `data_source → rank → take_top_n(5) → surface` evaluates over a 20-instrument universe and surfaces
  exactly the top 5 by the ranked feature, and that its compiled manifest reflects the selected data
  source lane.
- **Depends on:** P3-T02.

---

## Phase exit criteria
- [x] Strategy kind is inferred from the execution-block presence; declared `strategy_type` is ignored.
- [x] Capability manifest compiles at save into `strategy_manifests`; default EMA → `[market.ohlcv]`,
      `bar_close`, `Discovery`.
- [x] Apply-list endpoint returns only compatible strategies (incompatible omitted, not flagged).
- [x] A new account is seeded with exactly the default 7/21 EMA cross-asset discovery strategy.
- [x] `AutomationPlan` (single + pipeline) model and schema round-trip through Postgres.
- [x] Pipeline runtime maintains stateful stage membership with correct enter/exit deltas.
- [x] Rising-edge execution emits one order per crossing, deduped by
      `(automation_id, instrument_id, stage_id, signal_epoch)`.
- [x] v1.5 builder nodes (data-source/rank/filter/take-top-N/surface) work and feed the manifest.
- [x] `cargo check --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
      --all-features` all green.
