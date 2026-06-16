# ADR-0017: Walk-Forward Cross-Validation and Leakage Discipline

**Status:** Accepted
**Date:** 2026-06-16
**Deciders:** Platform team

## Context

Set I (the probabilistic forecasting suite, `docs/plans/plan-sets/set-I/`) makes the
AI Model Studio's forecasts trustworthy. The capability spec is explicit that the
**trust foundation** — cross-validation, leakage-safety, and proper-scoring
evaluation — must land before any new feature family, ensemble, or pipeline:
*nothing above the foundation is trustworthy until it is solid.*

The Set H Studio splits a dataset with a single ordinal train/validation/test cut
(`engine.split_indices`). For time-series forecasting that is insufficient on two
counts:

1. **No calibration role.** Conformal calibration and ensemble weighting (Set I
   Phases 4) need a split that is neither the training data the estimator saw nor
   the test data used to score it. A two-way train/test split has nowhere to fit a
   calibrator without contaminating one of them.

2. **Label leakage across the split boundary.** A forward-looking label (e.g. the
   return over the next `H` bars) computed at a row near a split boundary overlaps
   data on the other side of the boundary. A model trained on such rows has
   effectively seen the future of its own test set. ADR-0008 makes *event*
   lookahead impossible by `available_time` ordering, but it does not by itself
   stop *label-window* overlap at a CV boundary.

The platform already owns the primitives that make a correct split possible:
point-in-time bar access and forming-bar-safe resampling
(`backtest::store::BarStore::load_bars_bucketed`, `backtest::aggregate::aggregate_bars`),
`available_time` as the universal sort key (ADR-0008), and the append-only
ground-truth archive (ADR-0009). What is missing is a *frozen specification* of how
those rows are sliced into folds, and an enforced rule that every training run runs
a leakage test.

## Decision

Set I adopts **walk-forward cross-validation with three roles, purge, and embargo**
as the only sanctioned way to split a dataset for training and evaluation. The plan
is declared by an optional, additive `cv` block on the model definition
(`crates/domain/src/model_def/cv.rs`, `WalkForwardSpec`), and the fold geometry is
computed by a PURE generator (`features::walk_forward::walk_forward_folds`) that
emits index ranges — never by the Python sidecar.

The specification pins:

- **Three roles per fold: train · calibration · test.** `train` fits the estimator.
  `calibration` (the role Set H lacked) is reserved for conformal / calibration
  fitting and ensemble weighting and is **never** seen by hyperparameter scoring.
  `test` is strictly out-of-sample.

- **Window mode: expanding or rolling.** Expanding grows the train window from the
  start of the index each fold; rolling slides a fixed-length train window forward.
  Both share the same forward-marching calibration/test anchors, so test windows
  never overlap across folds.

- **Purge.** At every role boundary, rows whose forward label window of length `H`
  (the label horizon, in base-timeframe bars) overlaps the next role are dropped.
  The generator enforces `effective_purge = max(spec.purge_bars, horizon_bars)`, so
  the leakage property — *no train or calibration row's `[i, i+H]` label window
  reaches into a later role* — holds by construction regardless of how the spec
  author set `purge_bars`.

- **Embargo.** A gap of at least the label horizon is inserted after the test window
  before the next fold's training data resumes (`embargo_bars ≥ horizon_bars`,
  enforced by `WalkForwardSpec::validate`). This is the López de Prado embargo: it
  prevents a test row's label from bleeding into a later fold's training window.

- **Rust owns the geometry; the sidecar receives index ranges.** Fold boundaries are
  computed in Rust over the pinned dataset's `available_time`-sorted index and handed
  to the sidecar in the train dispatch. The sidecar never chooses its own split and
  never sees rows outside the role it is fitting. It is handed pre-windowed,
  point-in-time-correct data and holds no ClickHouse client.

- **Back-compatible default.** A v1.0 model definition without a `cv` block keeps
  today's behaviour: a single expanding fold. No stored definition breaks.

- **A leakage test runs in every pipeline.** Every training/evaluation run executes
  an automated leakage check: a synthetic future bar planted in the source must be
  unreachable through the point-in-time data view, and a deliberately leaky variant
  (label shifted the wrong way) must produce impossibly-good scores that the eval
  suite flags. A run that skips the leakage test is not a valid run.

## Rationale

A forecasting model that is even slightly contaminated by its own future is worse
than no model: it reports excellent backtest scores and then fails in production,
destroying trust in the whole suite. Purge and embargo are the standard discipline
(López de Prado, *Advances in Financial Machine Learning*) for time-series CV, and
the calibration role is a prerequisite for the conformal and ensemble work in later
phases. Pinning all three in an ADR — rather than leaving them to per-run
configuration — means the safe path is the default path and the unsafe path is not
expressible.

Computing the split in Rust, as pure index ranges over an `available_time`-sorted
index, is what makes leakage-safety *structural* rather than *aspirational*: the
sidecar cannot peek because it is never given the data to peek at. This extends
ADR-0008's "lookahead impossible by construction" from event ordering to CV
boundaries.

Enforcing `effective_purge = max(purge_bars, horizon_bars)` inside the generator
(rather than only validating it) means the leakage property is guaranteed even when a
user under-specifies the purge — defence in depth.

## Consequences

**Positive:**
- The calibration role exists and is isolated, unblocking conformal calibration and
  ensemble weighting (Phase 4) without contaminating train or test.
- Label-window leakage at fold boundaries is impossible by construction; the
  property is unit- and property-tested in `features::walk_forward`.
- The sidecar cannot leak because it never selects a split or queries bars.
- Every run carries a leakage test, so a regression that reintroduces leakage fails
  CI rather than shipping silently.
- v1.0 definitions are unaffected (single-expanding-fold default).

**Negative:**
- Walk-forward with purge + embargo consumes more history than a single split; short
  datasets may not fit the requested fold count (`FoldError::InsufficientHistory`,
  surfaced at materialization).
- Rust and the sidecar must agree on the index ordering exactly; the pinned,
  hashed dataset snapshot (Phase 0 materialization) is the contract that guarantees it.

**Neutral:**
- `WalkForwardSpec` is frozen as part of Model Definition v1.1 (ADR-0016 governs the
  envelope); changes require the additive-migrator mechanism of ADR-0015.
- The single-fold default keeps the isolated-train path identical to Set H.

## Alternatives Considered

### Option A: Keep the single ordinal train/val/test split
Reuse Set H's `engine.split_indices` unchanged.

Not chosen because: it has no calibration role for conformal/ensemble fitting and
does no purge/embargo, so boundary rows leak their label windows across the split.
It cannot support the trust foundation the capability spec demands.

### Option B: K-fold / shuffled cross-validation
Standard scikit-style K-fold with random assignment.

Not chosen because: shuffling time series destroys temporal order and guarantees
leakage (future rows train a model scored on past rows). It is categorically wrong
for forecasting.

### Option C: Let the Python sidecar compute the split
Hand the sidecar the full dataset and a CV config and let it slice.

Not chosen because: it puts the leakage-critical geometry in the component that
should never see future bars, making leakage-safety a matter of trust rather than
construction. Rust computing index ranges and handing role-scoped data to the
sidecar is the structural guarantee.

## References

- ADR-0008: available_time Ordering and Same Builders for Live and Replay
  (the lookahead-impossible invariant this ADR extends to CV boundaries)
- ADR-0009: Append-Only Raw Event Archive as Ground Truth (the realized outcomes
  the test role scores against)
- ADR-0015: Freeze Model Definition Format v1.0 (the additive-migrator mechanism the
  `cv` block uses)
- ADR-0016: Distributional Forecast Contract v1.1 (the v1.1 envelope `cv` ships in)
- `crates/domain/src/model_def/cv.rs` — `WalkForwardSpec`, `WindowMode`, validation
- `crates/features/src/walk_forward.rs` — the PURE fold generator
- `docs/plans/plan-sets/set-I/phase-0-data-foundation-and-cv.md` — the implementing plan
- López de Prado, *Advances in Financial Machine Learning* (purge + embargo)
