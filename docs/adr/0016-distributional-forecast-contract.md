# ADR-0016: Distributional Forecast Contract v1.1

**Status:** Accepted
**Date:** 2026-06-16
**Deciders:** Platform team

## Context

The AI Model Studio (Set-H) produces **point estimates**: the canonical runtime
output is `domain::model::forecast::Forecast { direction, magnitude, confidence, horizon }`.
A point estimate cannot be calibrated, cannot express risk, and cannot be
evaluated with proper scoring rules. The capability spec (§3, §4) requires a
distributional output for every model that claims to forecast.

Three design constraints drive this decision:

1. **Backward compatibility.** Strategies already reference `model_forecast` nodes
   that read `direction`, `magnitude`, `confidence`. These must keep working after
   distributional models are introduced.

2. **ADR-0002 (no f64 for money).** The distribution arrays are *not* monetary
   quantities — they are dimensionless scale factors in σ-units or return units.
   f64 is correct and intentional for distribution arrays (D-4).

3. **Train/serve parity.** Whatever coordinate system the model uses at training
   time must be preserved exactly at serve time. The σ scaler used to standardize
   training targets must travel with the artifact and be applied identically at
   inference.

## Decision

The `Forecast` type gains an **optional** `distribution: Option<ForecastDistribution>`
field, making the distributional output an additive extension of the v1.1
`ModelDefinition` (ADR-0015). The existing point fields become a **derived view**:
they are computed from the distribution when `distribution` is `Some`, and set
directly when `distribution` is `None` (point/classification models).

### `ForecastDistribution` shape

```rust
pub struct ForecastDistribution {
    pub quantile_levels: Vec<f64>,   // e.g. [0.05,0.1,…,0.9,0.95]; sorted in (0,1)
    pub quantiles_sigma: Vec<f64>,   // values in σ-units, sorted ascending (monotone)
    pub quantiles_return: Vec<f64>,  // σ-rescaled to return units: q_sigma * sigma
    pub median_return: f64,
    pub sigma: f64,                  // realized-vol scale for the σ↔return rescale
}
```

**Coordinate system:** models predict in σ-units (devolatized targets). `sigma`
(the realized vol of the training targets, fit on train only) restores return
units at serve. `quantiles_return[i] = quantiles_sigma[i] * sigma`.

### Derived point view

```
direction  = sign(median_return)              // Up / Down / Flat near zero
magnitude  = Decimal(median_return)           // ADR-0002: Decimal, not f64
confidence = 1 - |q90_return - q10_return| / (2 * sigma)   // clamped [0,1]
```

### Risk read-outs

VaR, ES, skew, and spread are **derived from the quantiles** at the publish seam
(Phase 6) — they are not stored in `ForecastDistribution`. One source of truth.

### Monotonicity invariant

`quantiles_sigma` must be sorted ascending. The Python sidecar enforces this via
`repair_quantiles` (sort-based) after prediction and before bundle write. The Rust
`validate()` method enforces it again at serve when reconstructing the distribution
from the sidecar response. A model that cannot produce a valid distribution fails
its training run rather than publishing a malformed artifact.

### Model Definition v1.1 blocks

Three optional blocks are added to `ModelDefinition` (additive; v1.0 specs
without them still validate and train):

```jsonc
"output":      { "quantile_levels": [0.05,0.1,0.25,0.5,0.75,0.9,0.95] }
"hpo":         { "enabled": true, "max_trials": 40, "metric": "crps" }
"calibration": { "method": "conformal", "fit_on": "cal" }
```

`DEFINITION_VERSION` bumps to `"1.1"`. The validator accepts `"1.0"` and `"1.1"`.
`migrate_v1_0_to_v1_1` fills the default quantile grid and HPO-disabled defaults.

### Bundle extension (tb-bundle-1)

The `wrap_bundle` header gains:

```json
"output_kind":   "distribution" | "point"
"quantile_levels": [...]
"sigma_scaler":  <float>
"calibration":   null   // Phase 4 fills this
```

Legacy point bundles (no new fields) continue to load unchanged.

## Rationale

**Why optional, not required?** Classification models and simple regressors cannot
produce a distribution without significant refactoring. Making `distribution`
optional preserves the Set-H promotion pipeline while letting distributional
models participate incrementally.

**Why σ-units?** Markets exhibit heteroskedasticity: a 50 bps move is very
different in a low-vol vs. high-vol regime. Devolatizing targets removes that
regime dependency from the model's task, so the model learns the normalized shape.
`sigma` restores the actual scale at serve with a single multiply — no ambiguity.

**Why sort-based repair, not isotonic regression?** Sort is O(n log n), always
terminates, and produces the unique minimum-perturbation solution for unconstrained
quantile crossing. Isotonic regression is equivalent for this case but more
complex. Repair count is reported in metrics so degenerate models surface quickly.

**Why CRPS as the HPO metric?** CRPS (≡ mean pinball loss for quantile regression)
is a strictly proper scoring rule: minimizing it over the test set is equivalent to
maximizing sharpness subject to calibration. No other single metric jointly
incentivises both. The calibration role is never used for HPO scoring to prevent
leakage into the conformal calibration step (Phase 4).

## Consequences

**Positive:**
- Strategies using `model_forecast` keep working without modification.
- The distribution flows train→bundle→inference→Rust `Forecast` in one contract.
- Risk read-outs (Phase 6) are derived, not duplicated.
- ADR-0002 is satisfied: all monetary outputs are `Decimal`; distribution arrays are f64.

**Negative:**
- Bundles now carry a σ scaler that must be fit on train-only data. Any future
  re-use of the scaler across datasets must verify temporal alignment.
- The σ-unit coordinate introduces an implicit assumption: `sigma` is stable
  enough across the inference window. Regime shifts can violate this. Phase 4
  calibration partially mitigates it.

**Neutral:**
- `ForecastDistribution::validate` is the single enforcement point; it is called
  at both train completion (Python) and serve (Rust) to close the contract.
- v1.0 definitions migrate automatically; the migration fills harmless defaults
  and is idempotent.

## Alternatives Considered

### Option A: Add distribution as a separate endpoint
Return raw distribution from a new `/predict/distribution` endpoint; keep existing
`/predict` returning the point view.

Not chosen because: the distribution and point view are correlated — returning them
separately introduces inconsistency risk (e.g., direction from one call contradicts
sign of median from the other). A single payload with an optional distribution
block is the correct encoding.

### Option B: Store VaR/ES/skew in ForecastDistribution
Pre-compute and store risk read-outs in the domain type alongside the quantiles.

Not chosen because: risk read-outs are functions of the quantiles — storing them
creates a redundant source that can diverge. Derivation at the publish seam (Phase 6)
is cleaner and allows changing the read-out formulas without re-training models.

### Option C: Normal distribution parameterisation (μ, σ)
Instead of sorted quantile vectors, store (μ, σ) and derive quantiles analytically.

Not chosen because: quantile regression makes no distributional assumption. A
Normal parameterisation would require returns to be Normal — empirically they are
fat-tailed and skewed. The quantile vector is assumption-free and compatible with
GARCH-t, ensemble stacking (Phase 4), and conformal calibration.

## References

- ADR-0002: Decimal Money Newtypes — No f64 (distribution arrays are f64 by D-4)
- ADR-0015: Freeze Model Definition Format v1.0 (the additive-migrator pattern)
- ADR-0017: Walk-Forward CV and Leakage Discipline (σ scaler fit on train only)
- `crates/domain/src/model/forecast.rs` — `ForecastDistribution`, `Forecast::from_distribution`
- `apps/model-trainer/app/engine.py` — `fit_sigma_scaler`, `repair_quantiles`, `validate_distribution`
