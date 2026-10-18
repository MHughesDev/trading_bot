# Models v2 — Set N

**Completion: 56% left (18 / 41 tasks). 18 done, 20 partial, 3 not started.** (R-N2, 2026-10-05)

**Status:** PLANNED (2026-10-04, track F-RESERVED) — this file was the reserved letter in [ROADMAP §5](../../ROADMAP.md#5-set-succession-what-the-reserved-letters-still-mean) until today.
**Stage:** [4](../../ROADMAP.md#4-the-stages) — capability depth; absorbs Set H Phases 4–5
**Derived from:** [FEAT-006](../../../specs/FEAT-006-model-platform-v2.md) (with its 2026-09-15 amendment), BS-007 10_MODELS, AI_MODELS_SUITE_CAPABILITY_SPEC, ROADMAP §5, OPEN-TASKS Part B (Set H) and Part C3
**Requirement IDs:** MD-01…MD-13 (MD-08 is Set U's), Set H 4.5–5.9.
**Migration range:** 0207–0209 (inside 0205–0214, shared with sets M, R-a and R-b).
**Scope class:** End-state architecture.

---

## 1. Why this set exists

An agent that can train a model needs three things the platform does not yet give it: a way to *declare* the dataset so that purge and embargo are derived and not typed,
honest scoring that makes a model beat the dumb baselines before it is allowed to size a position, and calibration and serving that do not silently fall back to a wrong number.
Set I shipped the forecasting core (distributional contract, walk-forward CV, CRPS and PIT, ensembles); Set U shipped typed model contracts, point-in-time resolution, per-fold models and the prediction
series. What remains is the declaration, the family registry, the scoring additions with their usage rules, calibration and serving, and the studio.

## 2. Scope

| Area | Today | This set delivers |
|---|---|---|
| Dataset declaration | free-form training request | a typed, hashed declaration with derived purge and embargo that lowers to the existing planner |
| Families | a framework switch in `worker.py` | a registry keyed by `(family, variant)` with baselines always trained beside a candidate |
| Sequence models | seq-len 1 | real windows recorded in the bundle |
| Scoring | pinball, CRPS, PIT, DM, deflated CRPS | CT R-squared, Clark-West, QLIKE, MZ, ES backtests, FZ loss and the hard usage rules as a verdict type |
| Calibration and serving | split conformal; GARCH falls back to unconditional sigma | ACI with CUSUM resets; conditional sigma or abstain |
| Studio | three pages | the nine Set H Phase 5 surfaces on Meridian |

### Out of scope

- Walk-forward prediction series and per-fold models (Set U U-5.5, U-5.8).
- Typed model contracts and the model-slot point-in-time check (U-5.1 to U-5.4).
- Learned *research-process* models M3, M4, M5, M8, M9, M13 (Set R-b Phase 5, behind ledger-size triggers).
- ML pack Phase 6 (asset encoder, ANN, multi-node): deferred by decision.

### Excluded because something else already owns it

| What | Owner | Evidence |
|---|---|---|
| MD-08 `predict_series` | Set U | U-5.5 (series job against ClickHouse) and U-5.8 (per-fold models, PARTIAL) |
| Typed model contracts, model embeddings | Set U | U-5.1 to U-5.4, U-5.2 |
| Distributional contract, walk-forward CV, CRPS/PIT, ensembles | Set I | closed 77/77 |
| H-4.1 to H-4.4 (registry/inference foundations) | Set U | OPEN-TASKS Part B: replaced by U-5 |
| `InstrumentProfile` writer and the outcome tensor | track CAMPAIGN | `instrument_profile_worker`, 2026-10-03 |

## 3. Decisions made while planning (ADR rows)

Each row records the alternative that was rejected. They are this set's own ADR index; promote one to `docs/adr/` when a task relies on it.

| ID | Decision | Rejected alternative | Why |
|---|---|---|---|
| ADR-N-01 | `DatasetDeclaration` is a new type that lowers to `DatasetRequest`; the resolved `dataplane::dataset::DatasetSpec` and `dataset_id` are untouched. | Extend `dataplane::dataset::DatasetSpec` with the FEAT-006 §3 fields. | The resolved spec is the thing INV-12 hashes into a `dataset_id`; adding agent-facing options to it changes every existing hash. |
| ADR-N-02 | The `sequence` family is SHOULD; real windows (MD-07) are MUST. | Make sequence models MUST pending a GPU. | The seq-len-1 bug is a correctness defect on any hardware; the zoo is a capability bounded by the Pascal GPU and `max_gpu_hours`. |
| ADR-N-03 | Datasets at or below 5m take an embargo floor of 24 hours of bars. | No floor (spec leaves it open). | With a horizon of a few bars, 1% of the sample can be under an hour, which is inside the autocorrelation time of minute returns. |
| ADR-N-04 | Scoring additions are pure Rust in `crates/model-registry`, beside the Python `scoring.py`, not a port. | Add them to `scoring.py` only. | The usage rules run in the validator and the gate, which are Rust; the verdict must not need a sidecar call. The Python module keeps what the trainer needs. |
| ADR-N-05 | A missing measurement in a usage rule is `NotComputable`, and `NotComputable` blocks. | Treat missing as pass-with-warning. | The same rule as every gate: an unmeasured thing is not a passed thing. |
| ADR-N-06 | The prediction-series table and `predict_series` are Set U's and are not re-planned. | Re-plan FEAT-006 §6 here. | U-5.5 built the fold-by-fold job against ClickHouse; a second plan would be a second implementation. |

## 4. Phase summary

| Phase | Title | Tasks | Migrations | State |
|---|---|---|---|---|
| 0 | [Reconciliation: two DatasetSpecs, a stale Set H, a GPU question](phase-0-reconciliation.md) | 3 | none | 3 / 3 done |
| 1 | [The dataset declaration and `dataset_build`](phase-1-dataset-declaration.md) | 7 | 0207 | 5 / 7 done |
| 2 | [Model families and bring-your-own](phase-2-families.md) | 10 | 0208 | 6 / 10 done |
| 3 | [Scoring additions and the hard usage rules](phase-3-scoring.md) | 6 | none | 0 / 6 done |
| 4 | [Calibration, serving and operations](phase-4-calibration-serving-ops.md) | 6 | 0209 | 4 / 6 done |
| 5 | [The model studio frontend (Set H Phase 5, re-cut)](phase-5-studio-frontend.md) | 9 | none | 0 / 9 done |

### Territory note

Phases 2 and 5 are almost wholly in `apps/model-trainer` and `frontend/`. The F-RESERVED track built only what is pure and in `crates/model-registry`: the declaration (N-1.1, N-1.2), and the scoring
additions and usage rules (N-3.1, N-3.2, N-3.4). The Set U per-fold hunks in `worker.py`, `schemas.py`, `sidecar.rs` and `manager.rs` were left alone.
