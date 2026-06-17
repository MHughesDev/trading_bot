# Phase 3 — Feature library & reproducibility

**Completion: 100% (10 / 10 tasks)**

**Goal:** With the trust foundation in place (Phases 0–2), invest in **features** and
**reproducibility**. Turn the single built-in `fs_core_ohlcv_v3` into a **versioned
feature library** with built-in families, multi-resolution assembly,
devolatization, and importance/correlation inspection — plus **pluggable
user-defined features**. Make every run reproducible: a **whole-spec deterministic
hash** (definition + pinned snapshot + seed + sidecar env) and **reproduce-from-hash**
that returns identical numbers, with side-by-side run compare.

**Depends on:** Phase 0 (multi-resolution rests on PIT resampling + pinned
snapshots), Phase 1 (devolatization σ source), Phase 2 (compare uses eval metrics).
**Blocks:** Phase 4–5 (ensembles/pipelines reference library features and spec
hashes), Phase 6 (feature preview/importance UI).

---

## Design notes

**Keep `crates/features` PURE.** Its `Cargo.toml` is explicit: *"features is PURE —
no storage, event-bus, redis, sqlx, or clickhouse deps."* All new transforms are
deterministic, incremental, versioned (`FeatureValue { name, value, feature_version,
available_time }`) — the same code runs live and in replay (ADR-0008). The library
is a registry of named specs (`FeatureSetSpec { name, version, features, description }`,
extending `feature_sets.rs`); a feature set is referenced from a model spec by
`feature_set_ref` (already a field on `ModelDefinition`).

**Reproducibility (D-10).** The dataset already hashes (Phase 0). Extend to a
**spec hash** over the canonicalized `{ definition (v1.1), dataset content_hash,
seed, sidecar env fingerprint, feature_set versions }`. Same hash ⇒ cache hit and a
guarantee that reproduce returns identical numbers (the capability spec's "re-run,
get identical numbers").

```
spec_hash = sha256( canonical_json(definition)
                  ‖ dataset_version.content_hash
                  ‖ seed
                  ‖ feature_set_versions
                  ‖ sidecar_env_fingerprint )
```

**Pluggable custom features.** A user-defined feature is a named, versioned transform
registered against the library with a pure evaluation contract (inputs: a bar window;
output: a `FeatureValue`). It runs in the same materialization path, so train/serve
parity and leakage-safety are inherited, not re-proven per feature.

---

## Tasks

### ☑ I-3.1 Versioned feature library registry — M
Extend `crates/features/src/feature_sets.rs` into a registry of multiple named,
versioned `FeatureSetSpec`s with `resolve`, `validate_features`, and a
`list_feature_sets()` surface; keep `fs_core_ohlcv_v3` as one entry. Reject unknown
feature names at spec-validation time (already the pattern).
**Acceptance:** multiple feature sets resolve and validate; an unknown feature name is rejected; `cargo test -p features` covers it.

### ☑ I-3.2 Built-in feature families — L
Add deterministic, versioned transforms for the spec's families: returns/lags,
range-based volatility (Parkinson/Garman–Klass), momentum, mean-reversion (z-score
of price vs MA), volume (e.g. OBV/relative volume), calendar/session (one-hot
session, time-of-day), and cross-asset context (a reference instrument's return). Each
carries a `FEATURE_VERSION`.
**Acceptance:** each family computes correct values on a fixture vs a hand-calc; versions are stable; live and replay produce identical series.

### ☑ I-3.3 Multi-resolution feature assembly — M
Assemble features from multiple timeframes onto the base grid using the Phase 0 PIT
resampler (forming-bar-safe): a 1h feature attached to a 5m row uses only the last
*settled* 1h bar (no peeking into the forming bar).
**Acceptance:** a multi-resolution feature set materializes; a test asserts the higher-timeframe value at each base row is the last settled bucket ≤ that row's `available_time`.

### ☑ I-3.4 Devolatization feature op — S
Expose σ-standardization as a library transform (fit-on-train σ from Phase 1),
so features as well as targets can be devolatized. Reuse the Phase 1 σ source; do not
duplicate the estimator.
**Acceptance:** a devolatized feature has ~unit train variance; the σ is the same one persisted in the bundle (no second estimator).

### ☑ I-3.5 Feature preview — S
Add `POST /api/models/features/preview` returning a sample of computed feature values
(+ basic stats) for a (feature set, instrument, window), read-only, before training.
**Acceptance:** preview returns a tabular sample + per-feature mean/std/NaN-rate for a real window.

### ☑ I-3.6 Importance & correlation inspection — M
Surface model feature importance (the XGBoost adapter already emits gain; extend to
the others where available) and compute a **feature correlation matrix** + a
collinearity flag (e.g. |ρ| > 0.95 clusters) for a feature set, to "fight
collinearity" per the spec.
**Acceptance:** an importance ranking is returned post-train; a correlation matrix + high-collinearity pairs are reported for a feature set with two near-duplicate features.

### ☑ I-3.7 Pluggable user-defined features — L
Define a registration contract for custom features (named, versioned, pure
evaluation over a bar window) and a safe in-suite mechanism to add one without a core
rewrite (a declarative composition of existing primitives is the v1 surface;
arbitrary code execution is explicitly out of scope). Custom features flow through
the same materialization (parity + leakage inherited).
**Acceptance:** a user-defined feature (e.g. a custom ratio of two built-ins) registers, materializes, trains, and serves identically; the leakage harness still passes with it present.

### ☑ I-3.8 Whole-spec deterministic hash — M
Implement `spec_hash` (shape above) over the canonicalized definition + dataset hash
+ seed + feature-set versions + sidecar env fingerprint; persist it on the training
run and use it as the cache key.
**Acceptance:** two identical specs hash equal and the second reuses the first's result; changing any input (a hyperparameter, the seed, a feature version) changes the hash.

### ☑ I-3.9 Reproduce-from-hash — M
Add `POST /api/models/runs/reproduce` that, given a `spec_hash` (or run id),
reloads the pinned snapshot + seed + definition and re-executes to **identical**
metrics. Pin the sidecar env (record versions; mismatch ⇒ explicit warning, not
silent drift).
**Acceptance:** reproducing a completed run yields bit-identical (within documented float tolerance) metrics; an env mismatch is surfaced, not hidden.

### ☑ I-3.10 Run / experiment compare — S
Add a compare surface over runs/experiments: params, metrics (Phase 2 scores),
artifacts, and spec-hash diffs side by side; `GET /api/models/runs/compare?ids=...`.
**Acceptance:** comparing two runs shows differing hyperparameters and their CRPS/coverage deltas; identical runs show an empty diff.

---

## Phase 3 exit criteria

- A versioned feature library with the spec's built-in families exists; sets resolve,
  validate, preview, and report importance + correlation.
- Multi-resolution assembly is forming-bar-safe; devolatization reuses the Phase 1 σ.
- User-defined features register and flow through the parity/leakage-safe path.
- Every run carries a whole-spec hash; reproduce-from-hash returns identical numbers;
  runs compare side by side.
- `crates/features` stays PURE (no I/O deps). `cargo test -p features -p model-registry`
  + sidecar pytest green; `just lint`, `just check-money` green.
