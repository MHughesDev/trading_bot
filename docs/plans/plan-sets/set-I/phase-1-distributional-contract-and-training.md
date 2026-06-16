# Phase 1 — Distributional contract & probabilistic training

**Completion: 0% (0 / 12 tasks)**

**Goal:** Turn the Studio's **point** forecaster into a **distributional** one.
Freeze the distributional output contract (sorted quantiles in σ-units → return
units; derived risk read-outs), evolve the Model Definition v1.0 → v1.1 additively,
add probabilistic targets/labeling (move-size, triple-barrier, quantile,
devolatized), ship quantile-regression and GARCH-t adapters, run **overfitting-aware
HPO inside the walk-forward folds**, and enforce the sorted-quantile contract for
every model at train and serve — over one bundle so train/serve parity holds.

**Depends on:** Phase 0 (walk-forward folds, pinned datasets, calibration role).
**Blocks:** Phase 2 (the eval suite scores distributions), Phase 4 (ensembles
combine distributions), Phase 6 (the publish contract serves them).

---

## Design notes

**The distributional output contract (frozen, ADR-0016).** Extend the canonical
runtime envelope rather than replace it, so existing strategy `model_forecast`
consumers keep working:

```rust
// crates/domain/src/model/forecast.rs  (additive — Forecast gains an optional dist)
pub struct ForecastDistribution {
    pub quantile_levels: Vec<f64>,   // e.g. [0.05,0.1,…,0.9,0.95]; sorted, in (0,1)
    pub quantiles_sigma: Vec<f64>,   // values in σ-units, sorted ascending (monotone)
    pub quantiles_return: Vec<f64>,  // σ-rescaled to return units (f64 — not money, D-4)
    pub median_return: f64,
    pub sigma: f64,                  // realized-vol scale used for the rescale
}
// Forecast keeps { direction, magnitude: Decimal, confidence: f64, horizon } as a
// DERIVED point view: direction = sign(median_return); magnitude = Decimal(median);
// confidence = f(interval width). Strategies that pinned the point view are unaffected.
```

Risk read-outs (VaR/ES/skew/spread) are **derived from the quantiles**, not stored
separately — one source of truth (computed in Phase 6 for the publish seam; the
shape lives here).

**Definition v1.0 → v1.1 (additive, D-5).** Bump `DEFINITION_VERSION` to `"1.1"`,
add a v1.0→v1.1 migrator (fills defaults), and add optional blocks:

```jsonc
{
  "schema_version": "1.1",
  "model_kind": "forecaster",
  "framework": "lightgbm",
  "target": { "field": "return", "horizon": "1h", "transform": "logret" },
  "output":      { "quantile_levels": [0.05,0.1,0.25,0.5,0.75,0.9,0.95] }, // NEW
  "cv":          { "mode": "expanding", "folds": 5, "train_bars": 4000,
                   "cal_bars": 500, "test_bars": 500, "purge_bars": 8,
                   "embargo_bars": 8 },                                     // NEW (Phase 0)
  "hpo":         { "enabled": true, "max_trials": 40, "metric": "crps" },   // NEW
  "calibration": { "method": "conformal", "fit_on": "cal" },               // NEW (Phase 4 uses)
  "label_spec":  { "type": "triple_barrier", "pt": 2.0, "sl": 1.0, "vert": "1h",
                   "devol": true }
}
```

v1.0 specs (no `output`) migrate to a default quantile grid and remain valid.

**Where the math lives (D-3).** All fitting/scoring is **Python sidecar** work
(`apps/model-trainer`); Rust dispatches frozen specs + pinned folds and stores
results. New Python deps land behind the sidecar's `pyproject.toml` (e.g.
`optuna` for HPO, `arch` for GARCH) — **no Rust ML crates** (Set H D-4 holds).

---

## Tasks

### ☐ I-1.1 Author ADR-0016 (Distributional Forecast Contract v1.1) — S
Write `docs/adr/0016-distributional-forecast-contract.md`: sorted quantiles in
σ-units → return units, median, σ; point fields become a derived view; risk
read-outs are derived; f64/Decimal boundary (D-4). Record the v1.0→v1.1 evolution
as additive per ADR-0015. Mark Accepted; link from `docs/adr/README.md` + MASTER §9.
**Acceptance:** ADR-0016 exists and is referenced by the `ForecastDistribution` doc comment.

### ☐ I-1.2 `ForecastDistribution` domain type + derived point view — M
Add `ForecastDistribution` (shape above) to `crates/domain/src/model/forecast.rs`;
add `Forecast.distribution: Option<ForecastDistribution>` (`#[serde(default)]`).
Implement `Forecast::from_distribution(...)` deriving `direction`/`magnitude`
(`Decimal`)/`confidence` from the distribution, and a monotonicity invariant
(`quantiles_sigma` sorted). f64 arrays only — no `Price`/`Size`.
**Acceptance:** `cargo test -p domain`: round-trip; derived point view matches a hand-computed example; non-monotone quantiles rejected by a constructor check.

### ☐ I-1.3 Model Definition v1.1 + migrator — M
Bump `DEFINITION_VERSION` to `"1.1"`; add `output: Option<OutputSpec>`,
`hpo: Option<HpoSpec>`, `calibration: Option<CalibrationSpec>` (and `cv` from
Phase 0) to `ModelDefinition`; update `validate.rs` (accept 1.0 *and* 1.1) and add
`migrate_v1_0_to_v1_1` filling default quantile grid + single-fold CV. Mirror the
strategy-format migration precedent.
**Acceptance:** a stored v1.0 definition loads, migrates to 1.1, and validates; a 1.1 definition round-trips; validator rejects an unsorted/empty `quantile_levels`.

### ☐ I-1.4 Probabilistic targets & labeling — M
Extend `TargetField` with `MoveSize`; add labelers in the sidecar for
**triple-barrier** (pt/sl/vertical), **quantile** targets, and **volatility**; add
**devolatized target construction** (divide label by a realized-σ estimate fit on
train). Emit **label-overlap metadata** (effective label horizon in bars) so Phase 0
purge is correct. Keep `return`/`direction` working unchanged.
**Acceptance:** sidecar unit tests produce correct triple-barrier labels on a fixture; devolatized targets have ~unit variance on train; label-overlap is reported to the windowing layer.

### ☐ I-1.5 Quantile-regression adapters — L
Add distributional adapters in `apps/model-trainer/app/adapters/`: LightGBM-Q
(`objective=quantile`, one model per level or a multi-quantile head), XGBoost
pinball, and sklearn `GradientBoostingRegressor(loss="quantile")`. Each emits the
full sorted quantile vector in σ-units. Register them in `_route` by
`(framework, output=quantile)`.
**Acceptance:** training a LightGBM-Q forecaster yields a bundle whose predictions are a sorted quantile vector at the configured levels; pinball loss decreases vs a flat baseline on a fixture.

### ☐ I-1.6 GARCH-t volatility adapter — L
Add a GARCH-t adapter (Python `arch`) producing a **distributional volatility**
forecast (σ + Student-t shape → quantiles). This is both a first-class
`field=volatility` forecaster and the σ source for devolatization/rescale. Gate the
new dep behind the sidecar extra; document it.
**Acceptance:** a GARCH-t run produces a horizon-H predictive distribution; its 1-step σ tracks realized vol on a fixture within tolerance; absence of the `arch` extra degrades to a clear "unavailable" error, not a crash.

### ☐ I-1.7 Devolatization / σ-rescale in train & serve — M
Fit the σ scaler on **train only** (no leakage), standardize targets/features as
configured, and persist σ in the bundle so serve-time rescales σ-units → return
units identically. This is the "spine as coordinate-setter": models predict
standardized shapes; σ restores scale.
**Acceptance:** train and serve produce identical return-unit quantiles for the same input (parity); σ is read from the bundle, never recomputed at serve.

### ☐ I-1.8 Quantile-crossing repair — S
After prediction (and before scoring/serving), enforce monotone quantiles via
sort/isotonic projection in the sidecar; record how many repairs occurred as a
quality signal.
**Acceptance:** a deliberately non-monotone raw output is repaired to monotone; the contract check (I-1.12) then passes; repair count is reported in run metrics.

### ☐ I-1.9 In-fold overfitting-aware HPO — L
Add HPO (Optuna) that runs **inside the walk-forward folds** (Phase 0), optimizing a
proper score (CRPS/pinball) on the test role only, never the calibration role.
**Count and persist the trial count** (feeds Phase 2 deflated metrics). Respect
`hpo.max_trials`; seed for reproducibility.
**Acceptance:** an HPO run reports `trials = N`, selects params by CRPS across folds, and persists the trial count on the training run; with `hpo.enabled=false` the path is a no-op.

### ☐ I-1.10 Extend the parity bundle for distributions — M
Extend the `tb-bundle-1` header (`engine.wrap_bundle`) to carry `quantile_levels`,
the σ scaler, the calibration placeholder (Phase 4), and `output_kind=distribution`,
**without breaking** existing point bundles (version the header; old bundles still
load). Inference reads the new header to reconstruct the exact distribution path.
**Acceptance:** a distributional bundle round-trips train→serve to identical quantiles; a legacy point bundle still loads and serves its derived point view.

### ☐ I-1.11 Inference emits a calibrated distribution — M
`apps/model-inference` returns the full sorted distribution + median; the Rust
`InferenceGateway::forecast` returns a `Forecast` with `distribution: Some(..)`. The
strategy `model_forecast` evaluator reads the **derived point view** unchanged
(no strategy migration needed). Caching keys include the quantile grid.
**Acceptance:** `/predict` returns sorted quantiles + median; the gateway surfaces them; an existing strategy using `model_forecast` still evaluates against the derived direction/magnitude.

### ☐ I-1.12 Enforce the output contract everywhere — S
Add a single `validate_distribution(...)` check (levels sorted & in (0,1), quantiles
finite & monotone, σ > 0) invoked at **train completion** and at **serve**; a model
that cannot produce a valid distribution fails its run rather than publishing a
malformed one. This is the spec's "enforced for every model."
**Acceptance:** a malformed distribution fails the train run with a clear error and is never written to the registry; a valid one passes both gates.

---

## Phase 1 exit criteria

- Every trainable model emits **sorted quantiles in σ-units**, rescaled to return
  units, with a median — validated at train and serve (no malformed distributions
  reach the registry).
- Model Definition is at v1.1; all stored v1.0 specs migrate and still validate.
- Quantile-regression (LightGBM-Q/XGBoost/sklearn) and GARCH-t adapters exist;
  devolatization and quantile-crossing repair are in the train/serve path.
- HPO runs inside walk-forward folds, optimizes a proper score, and records its
  trial count.
- Train/serve parity holds over one extended bundle; existing strategy consumers
  are unaffected (derived point view).
- `cargo test -p domain -p model-registry` + sidecar pytest green; `just lint`,
  `just check-money` green (distribution arrays are f64; no `Price`/`Size` touched).
