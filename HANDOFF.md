# Handoff / PR — MLOps consolidation + AI-in-strategy backend

> **Status: HANDOFF — NOT auto-mergeable.** This branch is behind `main` and
> conflicts with work that landed in parallel. Do **not** merge as-is; follow the
> reconciliation plan below. This file doubles as the PR description.

- **Branch:** `claude/set-i-complete-phase6`
- **Work commit:** `178b079` — *feat(mlops): unify AI Models/Pipelines into /mlops + forecaster strategy nodes*
- **Base when built:** an older `origin/main` (since advanced 10+ commits)
- **Excluded from the commit (machine-specific):** `config/local.toml` and
  `frontend/vite.config.ts` carry a local `8080 → 7080` port workaround (Flutter's
  dartvm squats 8080 on the dev box). Keep `main` on 8080.

---

## TL;DR

Two bodies of work were built on this branch:

1. **Phase 1 — MLOps consolidation** — collapses the three AI/ML nav destinations
   (AI Models, Ensembles, Pipelines) into one **`/mlops`** suite. **Net-new vs main.**
2. **Phase 2 Stage 1 — forecaster models in strategies** — makes an AI model node
   save/validate/backtest end-to-end. **The frontend half is superseded by main's
   newer "AI Inference" block; the backend half is still the missing piece on main.**

Everything here compiles and is tested **against the older base**, not current `main`.

---

## What's in commit `178b079`

### Phase 1 — MLOps consolidation (KEEP — net-new)
- `frontend/src/components/layout/GlassPillNav.tsx` — nav collapsed to a single **MLOps** pill.
- `frontend/src/App.tsx` — `/mlops` route tree (`/mlops`, `/mlops/data`, `/mlops/automation`,
  `/mlops/leaderboard`, `/mlops/lineage`, `/mlops/models/:id`) + redirects from old URLs.
- `frontend/src/components/mlops/MLOpsSubNav.tsx` — new shared sub-nav.
- `frontend/src/pages/DataWorkbenchPage.tsx` — new: data quality + walk-forward folds + feature library.
- `frontend/src/pages/ModelStudioPage.tsx` — re-headed "MLOps" + sub-nav; internal links → `/mlops/...`.
- `frontend/src/pages/ModelDetailPage.tsx` — "Forecast Quality" tab → **Monitoring**.
- `frontend/src/pages/PipelinesPage.tsx` — rewritten against the real `/api/pipelines` API (the
  old page was typed against a non-existent shape) + a working create form (templates/fan-out/schedule).
- `frontend/src/pages/{LeaderboardPage,ModelLineagePage,ModelCreatePage}.tsx` — `/mlops` links + sub-nav.
- `frontend/src/pages/EnsemblesPage.tsx` — **deleted**.
- `crates/api/src/routes/mod.rs` — `/api/ensembles` routes unwired.
- `migrations/0026_pipelines.sql` — **new**: the `pipelines/pipeline_runs/pipeline_node_runs`
  tables were never created by any migration (so `/api/pipelines` 500'd on a real DB).

### Phase 2 Stage 1 — forecaster strategy node
- **Frontend (DROP — superseded by main's AI Inference block):**
  `frontend/src/nodes/AIForecastNode.tsx`, `components/strategy/Palette.tsx`,
  `types/spec.ts`, `utils/compiler.ts`, `utils/toDefinition.ts`, `utils/fromDefinition.ts`.
- **Backend (KEEP + ADAPT — still missing on main):**
  - `crates/strategy-validator/src/schema.rs` — accept `definition_version` 1.0 **or 1.1**;
    let `signal.when` reference `ModelForecast` nodes; validate direction/confidence/model_ref.
  - `crates/strategy-runtime/src/interpreter.rs` (+ `lib.rs`) — `evaluate_signals_with_models`.
  - `crates/backtest/src/{forecast.rs,sim.rs,manager.rs,lib.rs}` — `ForecastProvider` trait +
    `SimulationInputs.model_results` threaded into the per-bar handler.
  - `crates/model-registry/src/backtest_forecast.rs` (+ `lib.rs`) — `GatewayForecastProvider`
    implementing the trait over the inference gateway (dep-inversion: backtest can't depend on
    model-registry, which already depends on backtest).
  - `apps/platform/src/main.rs` — wires the provider into `BacktestManager`.
- **Incidental:** `StrategyBuilderPage.tsx` + `ModelTestTab.tsx` fix two **pre-existing** `tsc`
  errors that were silently blocking `npm run build`.

---

## Divergence from current `main` (READ BEFORE MERGING)

`main` advanced 10+ commits (Set J backtest suite, a new `/workbench` nav item, and an
"AI Inference" strategy block from a parallel "ai-blocks-v2" effort).

| Area | main today | This branch | Action |
|---|---|---|---|
| MLOps consolidation | absent (separate AI nav + new `/workbench`) | `/mlops` suite | **KEEP**, reconcile with `/workbench` |
| Strategy AI node (frontend) | richer **AI Inference** (model/ensemble/pipeline targets, feature sets, timeframe/lookback) | simpler **AI Model** (forecaster only) | **DROP mine**, use main's |
| Validator v1.1 | still rejects ≠ "1.0" | accepts 1.0/1.1 + validates model nodes | **KEEP**, adapt to main's `model_forecast` shape (`target_kind`, `input{feature_set,timeframe,lookback}`) |
| Backtest model execution | none | `ForecastProvider` + per-bar results | **KEEP**, adapt to main's node shape |

Net: main has a richer AI **frontend** that is **non-functional end-to-end** (its v1.1
definitions are rejected by main's validator, and there's no backtest execution). This branch's
**backend** is exactly that missing half — but it targets the simpler node shape and must be
adapted to main's `NodeKind::ModelForecast` fields.

---

## Recommended reconciliation (do NOT just rebase-and-merge)

1. New branch off **current** `origin/main`.
2. Re-apply **Phase 1** (MLOps consolidation), reconciling the nav with main's `/workbench`.
3. Re-apply **Phase 2 backend** (validator v1.1 + runtime + backtest execution), adapting to
   main's richer `model_forecast` node shape so main's **AI Inference** block works end-to-end.
4. **Drop** this branch's Phase 2 frontend (`AIForecastNode` etc.) — main's AI Inference wins.
5. Re-run verification against main (`cargo build`/`test`, `npm run build`), then PR.

---

## Verification performed (against the OLD base)
- `cargo build` clean: strategy-runtime, strategy-validator, backtest, model-registry, platform.
- `cargo test` green: validator (incl. new v1.1 model-node tests), runtime, backtest (41 tests).
- `npm run build` clean (also fixed 2 pre-existing tsc errors).
- Live API: `POST /api/strategies` v1.1 model strategy → **201**; bad fields → **422** with
  per-field errors. MLOps UI walked end-to-end (nav, sub-pages, redirects, pipeline create+run).

## Known gaps / follow-ups
- **No trained models exist** in the registry, so "a forecaster actually drives backtest trades"
  is unproven (needs a trained + production-aliased forecaster and a reachable inference sidecar).
- Backtest provider feeds the strategy's **indicator** features, not the forecaster's own training
  feature set — a fidelity follow-up.
- Live execution for model nodes is **not** wired: `apps/platform/src/hot_path.rs` `stage_strategy_eval`
  is a placeholder that never loads a strategy.
- Pre-existing executor quirk: a pipeline parent run can show `failed` with an empty error even when
  all node runs succeed.

## Pointers
- Plan: `~/.claude/plans/sharded-inventing-canyon.md` (Phase 1 + Phase 2 staged plan).
- Session memory: `mlops-consolidation.md`, `set-i-complete.md` (corrections re: never-migrated tables).
