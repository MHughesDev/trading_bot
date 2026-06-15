# AI Model Studio — Set H

**Completion: 0% (0 / 64 primary tasks)**

**Status:** Design-complete · Implementation pending
**Created:** 2026-06-15
**Scope class:** End-state architecture (NOT an MVP cut — every subsystem is
specified at full fidelity; phases are build-ordering, not feature-gating).

---

## 1. Overview

Set H specifies the **AI Model Studio** — a first-class control surface and
backend for every *model-like component* the trading system uses: creating,
registering, training, testing, evaluating, comparing, versioning, renaming,
promoting, deploying, and retiring models. It is reachable at the route
`/models` and labelled **"AI Model Studio"** in the primary navigation.

Today the platform has the *consumer* of models but not the models themselves:

- The visual strategy builder ships an **AI Forecast node**
  (`frontend/src/nodes/AIForecastNode.tsx`) hardcoded to a single
  `price_forecaster`, which compiles to a `model_forecast` condition in the
  canonical strategy JSON (`frontend/src/utils/compiler.ts`).
- `GET /assets/models/{symbol}` (`crates/api/src/routes/asset_lifecycle.rs`)
  is an **empty stub** returning `{ "symbol": symbol }`.
- The Rust strategy runtime has **no evaluator** for `model_forecast`.
- The legacy Python system this platform is replacing *did* have models —
  `forecaster_model/`, `orchestration/nightly_retrain.py`, and an MLflow
  registry stub (`models/registry/mlflow_registry.py`, see
  [`docs/specs/MODELS_AND_ORCHESTRATION.MD`](../../../specs/MODELS_AND_ORCHESTRATION.MD)).
  None of it has been ported to the Rust platform.

The Studio is the **canonical home** for that capability, rebuilt to match this
repo's conventions: a Rust **system-of-record + orchestrator**, with **Python
sidecars** doing the actual machine learning. It is to models what the strategy
system (FEAT-001) is to strategies and what backtesting (FEAT-002) is to
simulation runs — a frozen definition format, a Postgres-backed registry, an
orchestrated job lifecycle, a REST/WebSocket API, and a polished React surface.

**Why now.** The strategy system already references models it cannot resolve.
Every strategy that drops an AI Forecast node today is writing a promise the
backend cannot keep. The Studio closes that loop and unlocks the platform's
differentiated capability: strategies driven by trained, versioned, evaluated
models rather than hand-tuned indicator thresholds.

---

## 2. Scope

### In scope — the model taxonomy

The Studio governs **all model-like components**, not just price forecasters.
Each model declares a `model_kind` that determines its inputs, outputs, training
contract, Test Lab surface, and how strategies consume it:

| `model_kind` | Purpose | Output contract | Trains in-house? |
|--------------|---------|-----------------|------------------|
| `forecaster` | Predict future price / return / volatility over a horizon | `{ direction, magnitude, confidence, horizon }` | Yes (Python sidecar) |
| `signal_ranker` | Score/rank a universe of instruments | `{ instrument_id, score }[]` | Yes |
| `trade_decision` | Map features → discrete action (enter/exit/hold) | `{ action, confidence }` | Yes |
| `risk_sizing` | Position size / risk budget from state + forecast | `{ size_fraction, max_risk }` | Yes |
| `embedding` | Text/event → vector (wraps the `semantic` crate path) | `{ vector[] }` | No (external API) |
| `external_llm_adapter` | Proxied decision-support LLM (Ollama/OpenAI/etc.) | `{ text, tokens, latency, cost, trace_id }` | **No — register/proxy only** |

**Trading models are the v1 priority** (forecaster → signal_ranker →
trade_decision → risk_sizing). Embeddings and external LLM adapters are
first-class registry citizens but are **inference/proxy only**; the Studio does
**not** do in-house LLM training. Pure-Rust inference may be added later for
selected hot-path production models where packaging/latency justifies it.

### Out of scope (this set)

- In-house LLM fine-tuning / training. LLMs appear only as **adapters**.
- Replacing the existing `semantic`/`embedder` embedding pipeline — the Studio
  *registers and governs* the embedding model, it does not re-implement Milvus.
- Multi-tenant org/RBAC beyond the per-user scoping the platform already uses
  (auth remains the Phase-1 `BearerToken` placeholder; see §8).
- Distributed/multi-GPU training clusters. The sidecar contract is designed to
  *allow* a remote worker pool later, but v1 targets a single trainer service.

---

## 3. Locked Decisions (2026-06-15)

Captured from the requirements interview. These are the fixed points every
phase builds on.

| # | Decision | Locked Choice |
|---|----------|---------------|
| D-1 | Section identity | **"AI Model Studio"**, route `/models`. A model command-center, not a settings page. |
| D-2 | Scope | **Trading AI Studio** — all model-like components (forecaster, signal_ranker, trade_decision, risk_sizing, embedding, external_llm_adapter). Trading models prioritized. |
| D-3 | LLMs | **Adapters only** — external/proxied decision-support models. No in-house LLM training. |
| D-4 | Runtime | **Hybrid** — Rust = system-of-record + orchestrator; **Python sidecars** run ML (PyTorch, scikit-learn, XGBoost/LightGBM, time-series) over HTTP/NATS. |
| D-5 | Pure-Rust inference | **Deferred** — added later only for selected production models when latency/packaging demands it. Contract leaves room (`runtime: rust\|python`). |
| D-6 | Definition format | **Frozen Model Definition v1.0** with `schema_version`, mirroring ADR-0007 for strategies. New ADR-0015 records the freeze. |
| D-7 | Job model | **Mirror `BacktestManager`** — async tokio jobs, atomic progress, Postgres-persisted snapshots, **polling** for progress (WebSocket lane added for live consoles). |
| D-8 | Lifecycle | `Draft → Training → Evaluating → Candidate → Active → Archived` (+ `Failed`), with movable **aliases** (`production`, `candidate`, `staging`, `fallback`). |
| D-9 | Promotion | **Gated** — a version reaches `Active` only through an explicit promotion gate (passed eval, no regression, artifact hash verified, rollback available). |
| D-10 | Deliverable for Set H | **Full end-state spec**, authored as this plan set. No code in this set. |

---

## 4. Architecture

```
┌──────────────────────────── FRONTEND (React) ────────────────────────────┐
│  /models  AI Model Studio                                                 │
│  Command Center · Detail Cockpit · Create Wizard · Training Console ·      │
│  Test Lab · Evaluations Arena · Version Timeline · Lineage Graph          │
│       │  REST (TanStack Query)        │  WS (ui-gateway lanes)            │
└───────┼──────────────────────────────┼───────────────────────────────────┘
        ▼                              ▼
┌──────────────────────── RUST  (system-of-record) ────────────────────────┐
│  crates/api/routes/models.rs       REST surface (/api/models/**)          │
│  crates/api  ws lane "models.jobs" live training/eval progress            │
│  crates/model-registry  (NEW)      ModelManager, job orchestration        │
│  crates/domain/model_def  (NEW)    frozen Model Definition v1.0           │
│  crates/storage/postgres/models.rs registry persistence (sqlx)            │
│  crates/strategy-runtime           model_forecast evaluator (inference)   │
│  crates/features  +  ClickHouse    feature/dataset materialization        │
│       │  HTTP / NATS  (orchestration contract §Phase 2)                    │
└───────┼───────────────────────────────────────────────────────────────────┘
        ▼
┌──────────────────────── PYTHON SIDECARS (ML compute) ─────────────────────┐
│  apps/model-trainer    (NEW)  train jobs: torch / sklearn / xgboost / ts  │
│  apps/model-inference  (NEW)  scoring service: load artifact, predict     │
│  (ports legacy forecaster_model/ + orchestration/nightly_retrain.py)      │
└───────────────────────────────────────────────────────────────────────────┘

Stores:  Postgres (registry, runs, aliases, events)   ·  ClickHouse (features,
predictions, traces)   ·  Object store FS→S3/MinIO (artifacts)   ·  NATS (job
events)   ·  Redis (inference cache, hot alias→version map)
```

**Responsibility split**

- **Rust owns** the registry (models, versions, aliases), datasets/feature sets,
  job scheduling + progress, evaluation records, strategy integration, the
  inference gateway, deployment aliases, traces, and every byte the UI reads.
- **Python owns** only the *compute*: given a frozen training spec + a
  materialized dataset, produce an artifact + metrics; given an artifact +
  features, produce predictions. Python holds **no source of truth** — it is a
  stateless worker addressed over HTTP/NATS.

This keeps the Rust modular monolith (ADR-0001) authoritative while using the
mature Python ML ecosystem where it is genuinely better, and it gives the legacy
`forecaster_model` / `nightly_retrain` code a migration target instead of a
rewrite.

---

## 5. Model lifecycle

```
   create            train succeeds        eval passes        promote (gated)
  ┌───────┐  train   ┌──────────┐  eval   ┌────────────┐  ▶  ┌───────────┐  ▶  ┌────────┐
  │ Draft │ ───────▶ │ Training │ ──────▶ │ Evaluating │     │ Candidate │     │ Active │
  └───────┘          └────┬─────┘         └─────┬──────┘     └─────┬─────┘     └───┬────┘
                          │ fail                │ regress          │ reject        │ retire
                          ▼                     ▼                  ▼               ▼
                       ┌────────┐            ┌────────┐                        ┌──────────┐
                       │ Failed │            │ Failed │                        │ Archived │
                       └────────┘            └────────┘                        └──────────┘
```

- **Status lives on the *version*, not the model.** A model is a stable identity
  (`model_id`, human-renamable `display_name`, immutable `slug`); each
  **version** is an immutable record with its own status.
- **Aliases** (`production`, `candidate`, `staging`, `fallback`) are movable
  pointers from a name to a version — the unit strategies and the inference
  gateway resolve against. Promotion = moving the `production` alias under a
  gate; rollback = moving it back. This is the MLflow alias model, grounded.

See `phase-0` for the exact enums and `phase-3` for the promotion gate.

---

## 6. Phase Summary

Each phase is independently reviewable and leaves the system in a consistent
state. Build order is top-to-bottom; Phase 5 (frontend) can begin against
Phase 1's API contract as soon as it is frozen, in parallel with Phases 2–4.

| Phase | File | Label | Tasks | Goal |
|-------|------|-------|-------|------|
| 0 | [phase-0-foundations.md](phase-0-foundations.md) | Foundations | 12 | Frozen Model Definition v1.0, domain types, Postgres schema (0017–0024), artifact store, ADR-0015 |
| 1 | [phase-1-registry-and-api.md](phase-1-registry-and-api.md) | Registry & API | 13 | `crate model-registry`, storage adapters, full REST surface, WS progress lane, AppState wiring |
| 2 | [phase-2-runtime-and-training.md](phase-2-runtime-and-training.md) | Hybrid Runtime & Training | 12 | Python trainer/inference sidecars, orchestration contract, datasets & feature sets, training runs |
| 3 | [phase-3-evaluation-and-promotion.md](phase-3-evaluation-and-promotion.md) | Evaluation & Promotion | 10 | Eval runs, forecast-vs-actual, scorecards, regression detection, promotion gate, deployments & aliases |
| 4 | [phase-4-inference-and-integration.md](phase-4-inference-and-integration.md) | Inference & Strategy Integration | 8 | Inference gateway, alias resolution, `model_forecast` evaluator, AIForecastNode wiring, used-by, traces |
| 5 | [phase-5-frontend-studio.md](phase-5-frontend-studio.md) | Frontend Studio | 9 | Command center, detail cockpit, create wizard, training console, Test Lab, evals arena, versions, lineage graph |

---

## 7. Cross-cutting principles

1. **Reuse the proven spine.** The registry mirrors `strategy_definitions`
   (migration `0004`); the job engine mirrors `BacktestManager`
   (`crates/backtest/src/manager.rs`); the WS lane mirrors the ui-gateway
   transport (`crates/ui-gateway/src/transport.rs`); the UI reuses the existing
   `components/ui` kit, `@xyflow/react` canvas, `lightweight-charts`, and the
   `--tb-*` design tokens. **Net-new surface area is minimized.**
2. **Definitions are frozen user data.** Per ADR-0007's reasoning, a model
   definition outlives sessions and is reloaded on restart — it is frozen at
   v1.0 with `schema_version` and migrated explicitly (ADR-0015).
3. **Determinism & lineage.** Every prediction is traceable to
   `version → training_run → dataset_version → feature_set → code_ref`. Datasets
   are versioned and hashed; no training runs from an anonymous file
   (mirrors ADR-0008/0009 available-time + ground-truth discipline).
4. **Money-adjacent safety.** Models feed strategies that place orders. Nothing
   reaches `Active` without passing the promotion gate (D-9), and the existing
   single risk-gate chokepoint (ADR-0005) is unchanged and still authoritative —
   a model can advise size, it can never bypass the risk gate.
5. **Observability first.** Every train/eval/inference call emits structured
   traces (latency, cost, inputs/outputs hash) following
   `OBSERVABILITY_AND_INFRA.MD`.

---

## 8. Security & permissions

Auth remains the platform's current **Phase-1 placeholder**: the `BearerToken`
extractor (`crates/api/src/auth/session.rs`) derives a deterministic
`user_id` and does not yet validate sessions against the DB (tracked as M-17).
All registry rows are **user-scoped** by `created_by`, matching the backtest
manager. The Studio adds **no new privileged path** — it cannot place orders;
it can only produce advice a strategy may consume through the unchanged risk
gate. Real session validation is a platform-wide concern outside this set;
when it lands, the Studio inherits it for free via the shared extractor.

---

## 9. Derived From / Traceability

| Source | Relationship |
|--------|--------------|
| `docs/specs/MODELS_AND_ORCHESTRATION.MD` | As-built legacy models/orchestration this set replaces |
| `docs/specs/FEAT-001-strategy-system.md` | Pattern parent — frozen definition + registry + front doors |
| `docs/specs/FEAT-002-backtesting.md` | Pattern parent — async job lifecycle + progress + results |
| `docs/specs/DATA-004-strategy-definition-format.md` | Format-freeze precedent |
| `docs/specs/COMP-003-ui-streaming-gateway.md` | WS lane/subscription pattern for live consoles |
| `ADR-0001` | Rust modular monolith + satellite processes (sidecars fit this) |
| `ADR-0005` | Single risk-gate chokepoint (unchanged; models cannot bypass) |
| `ADR-0007` | Freeze-the-format precedent (mirrored by ADR-0015) |
| `ADR-0008` / `ADR-0009` | Available-time ordering + ground-truth archive (dataset discipline) |
| `ADR-0014` | Backtesting via market simulator (eval harness reuse) |
| **ADR-0015 (new, this set)** | Freeze Model Definition format v1.0 — authored in Phase 0 |

---

## 10. Glossary

| Term | Meaning |
|------|---------|
| **Model** | Stable identity + frozen definition; a renamable container of versions. |
| **Version** | Immutable trained (or registered) snapshot with its own status, metrics, artifact. |
| **Alias** | Movable pointer (`production`, `candidate`, …) from a name to a version. |
| **Training run** | One async attempt to produce a version from a dataset + spec. |
| **Evaluation run** | Scoring of a version against an eval dataset, producing a scorecard. |
| **Dataset version** | Hashed, immutable materialization of features + labels over a window. |
| **Feature set** | Named, versioned list of features (built on the `features` crate). |
| **Sidecar** | Stateless Python worker doing train/inference compute over HTTP/NATS. |
| **Promotion gate** | The set of checks a version must pass to move the `production` alias. |
| **Scorecard** | Normalized 0–100 sub-scores (Quality/Speed/Cost/Safety/Reliability) + overall. |
| **Used-by** | Strategies whose definition contains a `model_forecast` referencing the model. |

---

## 11. Progress Log

| Date | Phase | Task | Note |
|------|-------|------|------|
| 2026-06-15 | — | plan | Set H created. 10 decisions locked. 64 tasks across 6 phases. End-state design only. |
