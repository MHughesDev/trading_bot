# ADR-0018: Ensemble Combination and Conformal Calibration

**Status:** Accepted
**Date:** 2026-06-16
**Deciders:** Platform team

## Context

Phase 4 of Set I introduces **Ensembles** as first-class versioned artifacts in the
AI Model Studio registry.  Each ensemble composes a **roster** of individually-trained
distributional models and combines their predictive distributions into a single
calibrated distribution.  Three design questions must be locked before implementation:

1. **Combination space** — in what coordinate system should member distributions be pooled?
2. **Combiner variants** — which combination strategies are supported in v1?
3. **Calibration guarantee** — how do we achieve empirical coverage ≈ nominal after combination?

This ADR locks those choices and defines the constraints every implementation must honor.

## Decision

### 1. Spine-as-coordinate-setter (σ-unit combination)

Member models may be trained on different σ scales (assets, timeframes, target transforms).
The combination happens in **σ-units**, the shared coordinate space established by
**ADR-0016**.  The first roster member (or an explicitly-designated `spine_ref`) sets
the σ coordinate frame; all other members are projected onto it by:

```
q_projected[i, l] = q_member[i, l] × (σ_spine / σ_member)
```

After pooling, the combined σ-unit distribution is rescaled back to return units by σ_spine.
This ensures members trained at different volatility scales are comparable before averaging
and that the output is in the units the downstream consumer expects.

### 2. Three supported combiners

| Combiner | ID | Description |
|----------|----|-------------|
| Linear opinion pool | `linear_opinion_pool` | Weighted average of quantile *functions* in σ-space.  Simple, robust, calibration-compatible. |
| CRPS-weighted adaptive | `crps_weighted` | Weights ∝ `1 / rolling_CRPS_member` over a configurable window.  Better members receive more weight; weights are renormalized and floor-clipped. |
| Stacking | `stacking` | A meta-learner (ridge regression or isotonic; configurable) trained on the **calibration role only** (ADR-0017) maps member quantile outputs → a combined distribution.  Never fit on test rows — training on test data is a leakage violation. |

All three combiners operate on the σ-projected member quantiles before applying weight
floors and temperature softening.

Weight parameters:
- **`weight_floor`** (default 0.05): no member receives less than this fraction of total weight.  Feasibility constraint: `weight_floor × |roster| ≤ 1.0`.
- **`temperature`** (default 1.0): sharpening/softening exponent applied to raw weights before flooring; `w_i ← w_i^(1/T)`.  Temperature < 1 sharpens (winner-takes-more); > 1 softens (toward uniform).

### 3. Adaptive conformal calibration

After combination, an **adaptive conformal layer** is fit on the **calibration role**
(ADR-0017) — never on train or test rows.  The layer stores residuals
`r_{t} = |y_t − q̂_{0.5,t}|` and adjusts each nominal interval by a conformal
quantile of the residuals, guaranteeing empirical coverage ≥ nominal on exchangeable
data.  The adaptive variant (ACI) uses an online update rule to track distributional
shift.

The conformal state (residuals, quantile estimates) is persisted in the ensemble bundle
under the `calibration_state` key, ensuring train/serve parity (ADR-0016 D-9).

### 4. Quantile-crossing repair (inherited from Phase 1)

The post-calibration output quantiles must be monotone.  The isotonic-regression
repair from I-1.10 is applied unconditionally.  Crossing counts are recorded in the
ensemble metrics.

### 5. Same evaluation suite

An ensemble is evaluated through the **identical** Phase 2 scoring pipeline
(CRPS, pinball, log-score, PIT, coverage, VaR backtests, baselines, DM).  The Diebold–
Mariano test compares the ensemble vs its best individual member.  Ensembles appear on
the same leaderboard alongside individual models.

## Rationale

**Why σ-units combination?**  Member models may have very different raw magnitudes
depending on asset volatility.  Combining raw quantiles directly would let a
high-volatility member dominate.  σ-units normalization makes member contributions
comparable regardless of scale.

**Why three combiners?**  LOP is the calibration-safe baseline (convex combination of
calibrated distributions is calibrated).  CRPS-weighted handles non-stationary member
quality without retraining.  Stacking is the strongest combiner for stationary regimes
where calibration data is sufficient; the cal-only fit rule prevents leakage.

**Why adaptive conformal rather than a Platt scaler?**  Platt scaling requires a fixed
mapping from raw scores to probabilities; conformal calibration gives a finite-sample
coverage guarantee with no distributional assumption.  The adaptive variant tracks
concept drift, consistent with the monitoring objective of Set I.

**Why the same eval suite?**  Consistency — users compare models and ensembles on
identical metrics, and the leaderboard is a single ranked surface.

## Consequences

- `EnsembleDefinition` carries: `roster`, `combiner`, `weight_floor`, `temperature`,
  `calibration { method, adaptive, fit_on }`.
- `EnsembleManager` mirrors `ModelManager` — same lifecycle (versions, aliases, gated
  promotion, rollback, deployments).  No parallel infrastructure.
- The Python sidecar (`apps/model-trainer`) gains `ensemble.py`:
  `combine()`, `conformal_fit()`, `conformal_update()`, `repair_crossings()`.
- Stacking trains only on the calibration role; a leakage-harness test asserts this.
- `weight_floor × |roster| > 1.0` is rejected at definition-validation time.
- The ensemble bundle header carries `schema_version`, `roster_member_ids`, `combiner`,
  `sigma_spine`, `conformal_state`, `quantile_levels` (same shape as member bundles).

## References

- ADR-0016 — Distributional Forecast Contract v1.1 (σ-unit output, quantile_levels)
- ADR-0017 — Walk-Forward CV & Leakage Discipline (calibration role, leakage invariant)
- Gneiting & Raftery (2007) — Strictly proper scoring rules, prediction, and estimation
- Venn, Taylor & Shafer (2008) — Adaptive Conformal Inference
- Jacobs et al. (2021) — CRPS-weighted ensemble learning
