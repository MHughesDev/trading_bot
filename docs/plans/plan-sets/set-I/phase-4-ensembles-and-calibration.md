# Phase 4 — Ensembles & conformal calibration

**Completion: 0% (0 / 11 tasks)**

**Goal:** Make an **Ensemble** a first-class, versioned, publishable artifact in the
*same* registry and lifecycle as a model. Compose a roster, choose a combiner
(linear opinion pool, CRPS-weighted adaptive, or stacking), configure weight floors
and temperature, combine **σ-standardized shapes** and rescale by σ
(spine-as-coordinate-setter), wrap the output in an **adaptive conformal
calibration** fit on the calibration role, repair quantile crossing, and evaluate
with the **exact same** suite as a model (Phase 2).

**Depends on:** Phase 1 (distributions, σ), Phase 2 (the eval suite + CRPS weights),
Phase 0 (the calibration role conformal fits on).
**Blocks:** Phase 5 (pipelines can produce/serve ensembles), Phase 6 (Ensemble
Builder UI + publishing ensembles).

---

## Design notes

**Reuse the spine (D-8).** An ensemble is a model-like object: it has versions,
aliases (`production`/`candidate`/…), gated promotion, rollback, deployments, and
the same WS/job model. `EnsembleManager` mirrors `ModelManager`; no parallel
lifecycle.

**Ensemble definition (frozen shape):**

```jsonc
{
  "schema_version": "1.1",
  "roster": [
    { "model_ref": "mdl_abc", "alias": "production" },
    { "model_ref": "mdl_def", "alias": "production" },
    { "model_ref": "mdl_ghi", "alias": "candidate" }
  ],
  "combiner": "crps_weighted",        // linear_opinion_pool | crps_weighted | stacking
  "weight_floor": 0.05,               // no member below this weight
  "temperature": 1.0,                 // sharpen/soften the weight distribution
  "calibration": { "method": "conformal", "adaptive": true, "fit_on": "cal" }
}
```

**Spine-as-coordinate-setter.** Members may be trained on different σ scales. Combine
in **σ-units** (the shared coordinate system), then rescale the *combined*
distribution by a single σ. The roster's first member (or an explicit spine) sets the
coordinate frame; others are projected onto it before pooling.

**Combiners:**
- **Linear opinion pool** — mixture: average member quantile *functions* (in σ-space)
  with weights; simple, robust.
- **CRPS-weighted (adaptive)** — weights ∝ recent inverse-CRPS (Phase 2), updated
  over a rolling window; better members get more say, adaptively.
- **Stacking** — a meta-learner (trained on the **calibration** role only, never
  test) maps member outputs → a combined distribution.

**Conformal wrapper.** After combination, an adaptive conformal layer fit on the
calibration role adjusts intervals so empirical coverage matches nominal — the
calibration guarantee the eval suite then verifies (PIT/coverage in Phase 2).

---

## Tasks

### ☐ I-4.1 Author ADR-0018 (ensemble combination & conformal calibration) — S
Write `docs/adr/0018-ensemble-combination-and-conformal-calibration.md`: the three
combiners, σ-coordinate combination, the conformal-on-cal-role rule, and quantile
repair. Cite ADR-0016 (distribution contract) + ADR-0017 (CV roles). Mark Accepted.
**Acceptance:** ADR-0018 exists, linked from `docs/adr/README.md` + MASTER §9.

### ☐ I-4.2 `EnsembleDefinition` domain type + validation — M
Add the ensemble definition (shape above) to `crates/domain` (e.g. `model_def`),
with validation: roster non-empty, all `model_ref` resolvable, weight_floor·|roster|
≤ 1, temperature > 0, valid combiner/calibration. Carries `schema_version`.
**Acceptance:** `cargo test -p domain`: round-trip + each validation reject (empty roster, infeasible weight floor, bad combiner).

### ☐ I-4.3 `EnsembleManager` — first-class artifact in the registry — M
Add `EnsembleManager` mirroring `ModelManager`: persist ensembles + versions +
aliases; reuse gated promotion, rollback, deployments, and the `models.jobs` WS lane.
Add migrations (Postgres 0026+: `ensembles`, `ensemble_versions`, `ensemble_members`).
**Acceptance:** an ensemble is created, versioned, promoted, and rolled back through the same lifecycle/API shape as a model; rows are user-scoped.

### ☐ I-4.4 Combiner: linear opinion pool — M
Implement the LOP combiner in the sidecar: weighted average of member quantile
functions in σ-space, then rescale.
**Acceptance:** a 2-member LOP on a fixture yields a distribution between the members; equal weights give the symmetric pool; output is monotone after repair (I-4.10).

### ☐ I-4.5 Combiner: CRPS-weighted (adaptive) — M
Implement rolling inverse-CRPS weighting (using Phase 2 per-member CRPS over a window),
with `temperature` sharpening and `weight_floor` flooring; weights recomputed as new
outcomes arrive.
**Acceptance:** the better member (lower CRPS) receives higher weight; flooring keeps every member ≥ weight_floor; a member that degrades loses weight over the window.

### ☐ I-4.6 Combiner: stacking — M
Implement a stacking meta-learner trained on the **calibration role only** mapping
member outputs → combined distribution; never fit on test (leakage).
**Acceptance:** stacking trains on cal, scores on test, and the leakage harness passes; on a fixture where one member is noise, stacking down-weights it.

### ☐ I-4.7 Weight floors & temperature — S
Wire `weight_floor` and `temperature` through all weighted combiners; validate
feasibility; expose in the definition + API.
**Acceptance:** changing temperature visibly sharpens/softens weights; an infeasible floor is rejected at validation, not at run time.

### ☐ I-4.8 Spine-as-coordinate-setter (σ-combine + rescale) — M
Combine members in σ-units against a shared coordinate frame (spine), then rescale
the combined distribution by a single σ to return units. Document which member/σ sets
the frame.
**Acceptance:** members trained at different σ scales combine correctly; a unit test shows the combined return-unit distribution is invariant to a uniform σ re-scaling of inputs.

### ☐ I-4.9 Adaptive conformal calibration wrapper — L
Add an adaptive conformal layer fit on the calibration role that adjusts the combined
intervals to hit nominal coverage; persist its state in the bundle (parity).
**Acceptance:** post-conformal coverage on test is within tolerance of nominal where pre-conformal was miscalibrated; the wrapper state serves identically (train/serve parity).

### ☐ I-4.10 Quantile-crossing repair on ensemble output — S
Apply the Phase 1 monotone repair to the combined+calibrated output; record repair
counts.
**Acceptance:** a crossing introduced by combination is repaired to monotone before scoring/serving.

### ☐ I-4.11 Evaluate ensembles with the same suite — S
Run an ensemble through the **identical** Phase 2 eval loop (CRPS/pinball/log-score,
PIT/coverage, VaR, baselines, DM) and onto the leaderboard alongside models.
**Acceptance:** an ensemble produces a full evaluation report and a leaderboard row; DM compares the ensemble vs its best member.

---

## Phase 4 exit criteria

- An ensemble is a first-class versioned artifact with the model lifecycle (aliases,
  gated promotion, rollback, deployments).
- All three combiners (LOP, CRPS-weighted, stacking) work; weight floors + temperature
  are honored; combination happens in σ-coordinates and rescales correctly.
- Adaptive conformal calibration fits on the cal role and improves coverage; output
  quantiles are monotone.
- Ensembles evaluate with the same suite and appear on the leaderboard.
- Train/serve parity holds for ensembles (one bundle). `cargo test` + sidecar pytest
  green; `just lint`, `just check-money` green.
