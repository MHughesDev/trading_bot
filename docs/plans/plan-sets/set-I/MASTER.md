# AI Models — Probabilistic Forecasting Suite — Set I

**Completion: 0% (0 / 77 primary tasks)**

**Status:** Not started — plan authored
**Created:** 2026-06-16
**Scope class:** End-state architecture (NOT an MVP cut — every subsystem is
specified at full fidelity; phases are build-ordering, not feature-gating).

---

## 1. Overview

Set H shipped the **AI Model Studio** — a Rust system-of-record + orchestrator
(`crates/model-registry`, `crates/domain/model_def`, `/api/models/**`, the
`models.jobs` WS lane, the React `/models` Studio) driving **Python ML sidecars**
(`apps/model-trainer`, `apps/model-inference`). That spine is real and strong:
model identity, immutable versions, movable aliases (`production`/`candidate`/
`staging`/`fallback`), gated promotion, rollback, deployments, lineage, traces,
and a self-describing train/serve-parity artifact bundle (`tb-bundle-1`).

Set I builds the **probabilistic forecasting core** on top of that spine — the
capabilities the [AI Models Suite Capability Spec](../../../specs/AI_MODELS_SUITE_CAPABILITY_SPEC.md)
calls for but Set H did not deliver. The Studio today produces **point estimates**:
the canonical output is `domain::model::forecast::Forecast { direction, magnitude,
confidence, horizon }`. There is no distribution, no walk-forward CV, no proper
scoring, no ensembles, no factory, and no quality monitoring. Two Set H modules
are also stubbed: `model-registry/src/datasets.rs` (`materialize` returns
`row_count = 0`, never queries ClickHouse) and `model-registry/src/metrics.rs`
(per-kind metrics are `null` proxies with no realized-outcome join).

**What this set delivers** (mapped to the capability spec §1–§11 + the handoff seam):

- A **leakage-safe data foundation** — point-in-time resampling, walk-forward
  windows (train/calibration/test, expanding|rolling, **purge + embargo**), a
  data-quality preview, and automated leakage tests — built on the *real* PIT
  primitives already in the repo (`backtest::store::BarStore::load_bars_bucketed`,
  `backtest::aggregate::aggregate_bars`, `available_time` ordering). Closes the
  `datasets.rs` stub. **(§1)**
- A **distributional output contract** — sorted quantiles in σ-units, rescaled to
  return units, with derived risk read-outs (VaR/ES/skew/spread) — enforced for
  every model, plus probabilistic targets/labeling and in-fold HPO. **(§3, §4)**
- A **forecast-quality evaluation suite** — CRPS, pinball, log-score,
  PIT/coverage/reliability, VaR backtests (Kupiec, Christoffersen), baselines,
  Diebold–Mariano, overfitting diagnostics, leaderboard, shareable reports.
  Closes the `metrics.rs` stub. **(§5)**
- A **feature library** (versioned families, multi-resolution, devolatization,
  importance/correlation, pluggable custom features) and **reproducibility**
  (spec→hash, reproduce-from snapshot+seed). **(§2, §9)**
- **Ensembles** (roster + combiners + conformal calibration + quantile-crossing
  repair) as first-class versioned artifacts. **(§6)**
- The **pipeline factory** (declarative DAG, templated fan-out across
  asset×timeframe×window, bar-cadence scheduling, caching/retries, training vs
  inference pipelines) and **suite-scoped quality monitoring** with
  drift-triggered retrain. **(§7, §10)**
- The **publish contract** (the one seam that matters) and the **workbench UX**
  (distribution charts, calibration plots, leaderboard, ensemble & pipeline
  builders, run queue). **(§8, §11, handoff)**

**Why now.** Set H closed the loop on *consuming* a model from a strategy, but a
point `{direction, magnitude, confidence}` cannot be calibrated, cannot express
risk, and cannot be trusted without proper scoring. The capability spec is
explicit: *steps 1–3 (CV + leakage + the eval suite) are the trust foundation;
nothing above them is trustworthy until they're solid.* Set I makes the Studio's
forecasts **trustworthy, distributional, and publishable behind a stable seam** —
the prerequisite for any downstream surface to rely on them with real capital.

---

## 2. Scope

### In scope

The eleven capability areas of the [AI Models Suite Capability Spec](../../../specs/AI_MODELS_SUITE_CAPABILITY_SPEC.md)
and its handoff contract, built on (and extending, never duplicating) the Set H
spine:

| Spec area | Set H today | Set I delivers |
|-----------|-------------|----------------|
| §1 Data & windowing | PIT primitives real; `datasets.rs` materialize stubbed | Walk-forward (train/cal/test, expanding\|rolling, purge+embargo), DQ preview, leakage tests, real materialization |
| §2 Features | one built-in set (`fs_core_ohlcv_v3`); train-time z-score | Versioned library + families, multi-resolution, devolatization, importance/correlation, pluggable custom |
| §3 Targets | `TargetSpec{field,horizon,transform}` | + move-size, triple-barrier, quantile, devolatized targets, label-overlap metadata |
| §4 Train one | families real; single ordinal split; point output | Walk-forward CV on by default, in-fold HPO with trial counting, **sorted-quantile σ-unit output enforced** |
| §5 Test & eval | point metrics; `metrics.rs` null proxies | CRPS/pinball/log-score, PIT/coverage/reliability, VaR backtests, baselines, DM, deflated metrics, leaderboard, reports |
| §6 Ensemble | absent | Roster + LOP/CRPS-weighted/stacking, conformal wrapper, quantile-crossing repair, versioned Ensemble artifact |
| §7 Pipelines | lineage + calendar retrain only | Declarative DAG, templated fan-out, bar-cadence schedule, cache/retries, training vs inference kinds, drift→retrain |
| §8 Registry | **strong, mostly done** | Extend for ensembles/pipelines/distributional artifacts + free-form tags/annotations |
| §9 Reproducibility | seed + dataset hash | Whole-spec hash, reproduce-from (spec+snapshot+seed), run compare |
| §10 Monitoring | trace rollups + calendar retrain | Rolling CRPS/coverage drift, calibration/data-drift alerts, staleness→retrain trigger |
| §11 Collab/UX | Studio shell + save/clone/share | Templates/presets, run queue/compute panel, distribution/calibration/leaderboard/ensemble/pipeline UI |

### Out of scope (this set — and the suite, permanently)

The capability spec draws a hard scope boundary; Set I honors it exactly:

- **Strategy logic / combining signals into trades.** Lives on the Strategies surface.
- **Strategy backtesting with execution, costs, slippage, sizing, P&L.** A
  downstream surface *consumes* a published model; it is not part of this suite.
  The Set H `backtest_bridge.rs` P&L stub is **intentionally left as-is** — Set I's
  measuring instrument is forecast quality (CRPS, coverage, VaR backtests), not
  P&L. (Consistent with `open-questions.md` Q-6/Q-9.)
- **Order management, execution, exchange routing, fills/reconciliation.**
- **Risk engine** (exposure limits, drawdown circuit breakers, kill switches). The
  single risk gate (ADR-0005) is unchanged and authoritative.
- **Capital allocation / portfolio construction** across strategies.
- **Live trade P&L and performance monitoring.** Set I's monitoring is
  *model-quality* drift only, never trade performance.
- **Multi-tenant org/RBAC** beyond the existing per-user `BearerToken` scoping (§8).
- **A parallel Rust ML/stats stack** — probabilistic compute stays in the Python
  sidecars (D-3).

---

## 3. Locked Decisions (2026-06-16)

These are the fixed points every phase builds on.

| # | Decision | Locked Choice |
|---|----------|---------------|
| D-1 | Position | **The probabilistic core on top of the Set H spine.** Reuse registry, versions, aliases, promotion, rollback, jobs, WS lane, and Studio; add the math and the factory. Net-new surface is minimized. |
| D-2 | Output contract | **Distributional.** Every model emits **sorted quantiles in σ-units**, rescaled to return units, plus median; derived risk read-outs (VaR/ES/skew/spread) computed from the distribution. The Set H point fields (`direction`/`magnitude`/`confidence`) become a **derived view** for v1.1 back-compat. Frozen in **ADR-0016**. |
| D-3 | Math placement | **Python sidecars own probabilistic compute** (quantile regression, GARCH-t, CRPS/pinball/log-score, PIT, Kupiec/Christoffersen, DM, conformal). **Rust owns** contracts, orchestration, storage, gates, and the leakage-safe data view. **No new Rust ML/stats stack** (consistent with Set H D-4; the workspace has zero ML crates today and keeps it that way). |
| D-4 | f64 vs Decimal | **Distribution arrays and scores are `f64`** in σ/return units — statistical, "not money," matching the existing `Forecast.confidence: f64` precedent. The suite **never constructs `Price`/`Size`**; the `Decimal` boundary (ADR-0002) is downstream. The derived point `magnitude` stays `Decimal` in the v1.1 envelope. `check-money-f64` (which only flags `Price`/`Size`) stays green. |
| D-5 | Format evolution | **Model Definition v1.0 → v1.1, additive.** Bump `DEFINITION_VERSION` and add a v1.0→v1.1 **migrator** (per ADR-0015's documented mechanism). New optional blocks: `cv`, `output` (quantile levels), `hpo`, `calibration`; new `TargetField` variants. Every v1.0 spec auto-migrates; no stored definition breaks. |
| D-6 | Trust foundation first | **Walk-forward CV + leakage tests + the eval suite (Phases 0–2) come before features, ensembles, and the factory.** Walk-forward = three roles (train/**calibration**/test), expanding\|rolling, **purge + embargo**, built on existing PIT primitives. A leakage test runs in **every** pipeline (invariant: `available_time` ordering, ADR-0008). |
| D-7 | Eval = forecast quality | **Proper scoring, not P&L.** CRPS/pinball/log-score, PIT/coverage/reliability, VaR backtests, DM, deflated metrics, single-use final holdout. Strategy P&L backtesting stays out of scope. |
| D-8 | Ensembles & Pipelines | **First-class versioned artifacts in the *same* registry & lifecycle as models** (reuse aliases/promotion/rollback). Combiners operate on **σ-standardized shapes** (spine-as-coordinate-setter), then rescale by σ; a conformal wrapper and quantile-crossing repair sit on the output. |
| D-9 | Train/serve parity | **One inference path.** Extend the existing self-describing bundle (`tb-bundle-1`/`TBNDL001`) header for distribution/calibration/ensemble metadata. The **published inference path is the exact code evaluation used** — no second implementation that can drift (capability-spec parity guarantee). |
| D-10 | Reproducibility | **Spec → deterministic hash** over the *whole* spec (definition + pinned dataset snapshot + seed + sidecar env), not just the dataset. Reproduce-any-run from the hash. Builds on the existing dataset `content_hash` + seed plumbing. |
| D-11 | Monitoring | **Suite-scoped model-quality drift only** — rolling CRPS/coverage, calibration drift, data/feature drift, staleness. Drift **triggers a retrain pipeline** (replacing the calendar-only `RetrainScheduler` trigger). Never trade P&L. |
| D-12 | Numbering | Reserve Postgres migrations **0026+**, ClickHouse DDL **05+**, ADRs **0016–0018**. Ensembles/Pipelines/leaderboard UI is **greenfield**, mirroring the `/models` Studio chrome (`@xyflow/react`, `--tb-*` tokens, `api/models.ts` patterns). *(Verify next-free numbers before locking — last used: migration 0025, ADR 0015, CH DDL 04.)* |

---

## 4. Architecture

```
┌──────────────────────────── FRONTEND (React /models) ─────────────────────────┐
│  Set H (reuse): Command Center · Cockpit · Create Wizard · Training Console    │
│  Set I (new): Distribution & fan charts · Calibration/PIT/reliability plots ·  │
│               Leaderboard · Ensemble Builder · Pipeline Builder (xyflow) ·     │
│               Run Queue / compute panel · Templates                            │
└───────┬───────────────────────────────────────────────────────────────────────┘
        │ REST /api/models/** , /api/ensembles/** , /api/pipelines/**  + WS lanes
        ▼
┌──────────────────────── RUST  (system-of-record + orchestrator) ──────────────┐
│  Set H spine (reuse): ModelManager · aliases · gated promotion · rollback ·    │
│                       deployments · InferenceGateway · traces · WS lane        │
│  Set I (new, in crates/model-registry + crates/features + crates/domain):      │
│    · WalkForward window engine (train/cal/test, expanding|rolling, purge+embargo)│
│    · DatasetManager.materialize REAL (PIT pull → features → Parquet → hash)     │
│    · DistributionalForecast contract (domain/model) + v1.0→v1.1 migrator        │
│    · EnsembleManager + PipelineManager (same lifecycle as ModelManager)         │
│    · Eval orchestration (dispatch scoring; store scorecards/leaderboard)        │
│    · Drift monitor → retrain-pipeline trigger (replaces calendar scheduler)     │
│    · Spec-hash + reproduce-from-snapshot                                         │
└───────┬───────────────────────────────────────────────────────────────────────┘
        │ HTTP (dispatch) / NATS (progress) ; bytes via ArtifactStore URIs
        ▼
┌──────────────────────── PYTHON SIDECARS (probabilistic compute) ──────────────┐
│  apps/model-trainer (extend): walk-forward CV runner · in-fold HPO (+ trial    │
│    counting) · quantile-regression & GARCH-t adapters · devolatization ·       │
│    quantile-crossing repair · conformal calibration · ensemble combiners       │
│  apps/model-inference (extend): emit calibrated distribution + risk read-outs; │
│    one bundle path = train/serve parity                                         │
│  apps/model-eval (NEW, optional split): CRPS/pinball/log-score, PIT/coverage,  │
│    Kupiec/Christoffersen, Diebold–Mariano, baselines, deflated metrics         │
└────────────────────────────────────────────────────────────────────────────────┘

Stores:  Postgres (registry, runs, ensembles, pipelines, drift, tags) ·
ClickHouse (predictions w/ quantiles, forecast-quality series, drift metrics) ·
Object store FS→S3 (Parquet snapshots, bundles) · NATS (job/drift events) ·
Redis (alias→version hot map, prediction cache)
```

**Responsibility split** (unchanged from Set H D-4, extended for the math):
**Rust** owns every contract, the leakage-safe data view, orchestration,
storage, the registry/lifecycle for models *and* ensembles *and* pipelines, the
promotion gates, and every byte the UI reads. **Python** owns only *compute*:
given a frozen spec + a materialized, pinned dataset, produce a **calibrated
distributional** artifact + proper-scoring metrics; given an artifact + features
+ timestamp, produce a **calibrated distribution**. Python holds no source of
truth and never reaches future bars (it is handed pre-windowed, PIT-correct data).

---

## 5. Where Set I sits in the suite build order

The capability spec prescribes a build order; Set I phases map to it directly,
trust-foundation first:

```
spec build order                         Set I phase
────────────────────────────────────────────────────────────────────
1  spec schema + single-model    ──────▶ (Set H, extended in Phase 1)
2  walk-forward CV + 3-role split ─────▶ Phase 0   ┐
3  test/eval suite (CRPS…)        ─────▶ Phase 2   ├─ trust foundation
   (+ distributional contract)    ─────▶ Phase 1   ┘
4  registry + experiment tracking ─────▶ Phase 3   (repro; registry from Set H)
5  ensemble + calibration         ─────▶ Phase 4
6  pipeline builder + fan-out      ────▶ Phase 5
7  publish contract + monitoring   ────▶ Phase 5 (monitor) + Phase 6 (publish/UX)
```

---

## 6. Phase Summary

Build order is top-to-bottom. Phase 6 (frontend + publish) can begin against each
contract as soon as it is frozen, in parallel with the phases above it.

| Phase | File | Label | Tasks | Goal |
|-------|------|-------|-------|------|
| 0 | [phase-0-data-foundation-and-cv.md](phase-0-data-foundation-and-cv.md) | Leakage-safe data & walk-forward CV | 11 | PIT windowing, walk-forward (train/cal/test, expanding\|rolling, purge+embargo), DQ preview, leakage tests, **real** `datasets.rs` materialization |
| 1 | [phase-1-distributional-contract-and-training.md](phase-1-distributional-contract-and-training.md) | Distributional contract & probabilistic training | 12 | ADR-0016 distributional `Forecast` + def v1.1 migrator, targets/labeling (move-size/triple-barrier/quantile/vol/devol), quantile & GARCH-t adapters, in-fold HPO, sorted-quantile contract enforced |
| 2 | [phase-2-forecast-quality-evaluation.md](phase-2-forecast-quality-evaluation.md) | Forecast-quality evaluation suite | 12 | CRPS/pinball/log-score, PIT/coverage/reliability, VaR backtests (Kupiec/Christoffersen), baselines, DM, deflated metrics + final holdout, leaderboard, reports; **close** `metrics.rs` |
| 3 | [phase-3-feature-library-and-reproducibility.md](phase-3-feature-library-and-reproducibility.md) | Feature library & reproducibility | 10 | Versioned feature library + families, multi-resolution, devolatization, importance/correlation, pluggable custom features, spec-hash + reproduce-from |
| 4 | [phase-4-ensembles-and-calibration.md](phase-4-ensembles-and-calibration.md) | Ensembles & conformal calibration | 11 | Roster + combiners (LOP/CRPS-weighted/stacking), weight floors/temperature, spine-as-coordinate-setter, conformal wrapper, quantile-crossing repair, versioned Ensemble artifact |
| 5 | [phase-5-pipeline-factory-and-monitoring.md](phase-5-pipeline-factory-and-monitoring.md) | Pipeline factory, fan-out & quality monitoring | 12 | Declarative DAG, templated fan-out (asset×tf×window), bar-cadence schedule, run cache/retries, training vs inference pipelines, drift detection → retrain trigger |
| 6 | [phase-6-publish-contract-and-workbench-ux.md](phase-6-publish-contract-and-workbench-ux.md) | Publish contract & workbench UX | 9 | Distributional publish contract + risk read-outs + parity guarantee, registry tags/templates, frontend (dist/calibration/leaderboard charts, ensemble & pipeline builders, run queue) |

---

## 7. Cross-cutting principles

1. **Trust foundation first.** CV + leakage tests + the eval suite (Phases 0–2)
   land before features, ensembles, and the factory. Per the capability spec,
   nothing above the foundation is trustworthy until it is solid.
2. **Leakage-safety is enforced, not assumed.** Every data view is point-in-time
   (`available_time` ordering, ADR-0008); every pipeline runs an automated leakage
   test (a deliberately-planted future bar must be unreachable, and a model that
   peeks must fail the test). Purge + embargo are mandatory in walk-forward.
3. **Train/serve parity.** The published inference path is the exact code the
   evaluation ran (D-9); one self-describing bundle, no parallel implementation.
4. **Reproducibility.** Pinned data snapshots, whole-spec hashing, seed +
   sidecar-env pinning; any run reproduces from its hash to identical numbers.
5. **Contract stability.** Published versions are immutable; the definition format
   evolves additively with an explicit migrator (ADR-0015 mechanism, ADR-0016 freeze).
6. **Modifiability.** Model families and features are pluggable behind interfaces;
   adding LightGBM-Q or a new feature family is additive, no core rewrite.
7. **The money boundary is downstream.** The suite is f64-statistical (returns, σ,
   quantiles, scores); it never constructs `Price`/`Size` (ADR-0002) and the single
   risk gate (ADR-0005) is untouched. A model advises; it never trades.
8. **Reuse the spine.** Registry, aliases, gated promotion, rollback, jobs, the WS
   lane, and the Studio chrome are reused for models, ensembles, and pipelines alike.
9. **Adversarial test per mechanism.** Every decided mechanism gets a test, and a
   task is not done until its test is green (master-plan invariant §2.8).

---

## 8. Security & permissions

Auth remains the platform's per-user `BearerToken` placeholder
(`crates/api/src/auth/session.rs`, tracked as M-17); all registry rows
(models, ensembles, pipelines, runs, reports) are user-scoped by `created_by`,
matching the backtest and Set H managers. Set I adds **no new privileged path**:
it cannot place orders, size positions, or touch capital. It produces advice a
strategy may consume through the unchanged single risk gate (ADR-0005). Sharing a
report/template is read-only export. Multi-user permissions/provenance (spec §11)
are stubbed at the share/clone level and inherit real session validation when the
platform-wide upgrade lands.

---

## 9. Derived From / Traceability

| Source | Relationship |
|--------|--------------|
| `docs/specs/AI_MODELS_SUITE_CAPABILITY_SPEC.md` | **Primary requirement** — every Set I task traces to a §1–§11 area or the handoff seam |
| `docs/plans/plan-sets/set-H/` | Predecessor — the spine Set I extends (registry, lifecycle, sidecars, Studio) |
| `docs/specs/set-H-api-contract.md` | Frozen REST/WS contract Set I extends additively (ensembles, pipelines, distributions) |
| `docs/specs/MODELS_AND_ORCHESTRATION.MD` | As-built model/orchestration context |
| `docs/specs/COMP-001-data-quality-and-ingestion.md` | Data-quality preview (§1) source |
| `docs/specs/COMP-004-storage-and-replay.md` | PIT/`available_time` storage discipline the data view builds on |
| `docs/specs/COMP-003-ui-streaming-gateway.md` | WS lane pattern for live consoles/leaderboard |
| `ADR-0001` | Rust modular monolith + satellite sidecars (eval sidecar fits this) |
| `ADR-0002` | Decimal money newtypes — defines the f64/Decimal boundary (D-4) |
| `ADR-0005` | Single risk gate — unchanged; models never bypass it |
| `ADR-0008` / `ADR-0009` | `available_time` ordering + ground-truth archive — the leakage discipline |
| `ADR-0012` | Canonical bar storage — base bars the resampler/windowing read |
| `ADR-0015` | Model Definition freeze + evolution mechanism (v1.0→v1.1 migrator) |
| **ADR-0016 (new, this set)** | Distributional Forecast Contract v1.1 — authored in Phase 1 |
| **ADR-0017 (new, this set)** | Walk-forward CV & leakage discipline — authored in Phase 0 |
| **ADR-0018 (new, this set)** | Ensemble combination & conformal calibration — authored in Phase 4 |

---

## 10. Glossary

| Term | Meaning |
|------|---------|
| **Distributional forecast** | Sorted quantiles + median in σ-units (rescaled to return units), the canonical Set I model output. |
| **σ-units / devolatization** | Targets/outputs standardized by a realized-volatility estimate; rescaled by σ at serve time ("spine as coordinate-setter"). |
| **Walk-forward CV** | Sequential train/calibration/test folds (expanding or rolling) with **purge** (drop label-overlapping rows) and **embargo** (gap after test). |
| **Calibration role** | The middle split reserved for conformal/calibration fitting — distinct from train and test (Set H had only train/val/test). |
| **CRPS** | Continuous Ranked Probability Score — the proper score for a full predictive distribution. |
| **Pinball / quantile loss** | Per-quantile proper score; averaged across levels approximates CRPS. |
| **PIT** | Probability Integral Transform — uniform under perfect calibration; drives reliability diagrams. |
| **VaR backtest** | Kupiec POF (unconditional coverage) + Christoffersen (independence) on the model's own tail. |
| **Diebold–Mariano** | Significance test for whether one forecaster's score beats another's. |
| **Deflated metric** | A score adjusted for the number of HPO trials, to expose overfitting. |
| **Conformal wrapper** | Adaptive post-hoc calibration on the calibration role, yielding coverage-valid intervals. |
| **Quantile-crossing repair** | Re-sorting/isotonic projection so output quantiles are monotone. |
| **Combiner** | How an ensemble fuses member distributions: linear opinion pool, CRPS-weighted, or stacking. |
| **Pipeline (model)** | A declarative DAG (data→features→target→train→calibrate→eval→register); *training* kind produces bundles, *inference* kind is published. |
| **Fan-out** | One templated pipeline run instantiated across many (asset, timeframe, window) cells. |
| **Spec hash** | Deterministic hash of the whole spec (definition + snapshot + seed + env) for caching and reproduce-from. |
| **Publish contract** | The immutable `predict(asset, timeframe, timestamp) → calibrated distribution` seam downstream surfaces call. |

---

## 11. Progress Log

| Date | Phase | Task | Note |
|------|-------|------|------|
| 2026-06-16 | — | plan | Set I created. 12 decisions locked. 77 tasks across 7 phases (0–6). Probabilistic core on top of the Set H spine; capability spec recorded at `docs/specs/AI_MODELS_SUITE_CAPABILITY_SPEC.md`. End-state design. |
