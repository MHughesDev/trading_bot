# Phase 1 — The Study layer & sealed distributions

**Completion: 0% (0 / 10 tasks)**

**Goal:** Build the layer that orchestrates many Runs along **one** varying
dimension and reports the *distribution* of outcomes — and enforce **INV-2**: a
Study exposes no API to select, return, or promote its best-performing member.
The best member is **not addressable**. Where a downstream consumer genuinely
needs one config carried forward (e.g. walk-forward picking params per window),
that selection happens *inside* the Study under a fixed, pre-declared rule —
never by a user reaching for the peak. Each Study emits a `trial_delta` (the
input Phase 2's counter consumes).

**Depends on:** Phase 0 (`Backtest::run`, `RunConfig`, `MetricSet`, `RunStore`).
**Blocks:** Phase 2 (Experiment consumes `trial_delta`), Phase 4 (every gate is
one or more Studies).

---

## Design notes

**A Study answers one question.** `parameter_sweep` answers *"how does
performance vary across this region?"* (you want a broad plateau), **not**
*"which parameter won?"*. `walk_forward` answers *"is the edge robust to
history?"*. The human-readable `question` is logged before the Study runs — the
single best defense against post-hoc reinterpretation.

**Contracts (frozen shape, `crates/backtest/src/study/mod.rs`):**

```rust
pub struct StudyConfig {
    pub study_id: StudyId,
    pub kind: StudyKind,                // catalog below
    pub base_config: RunConfig,         // the "center" being studied
    pub vary: VarySpec,                 // what dimension this study perturbs
    pub null_ref: Option<NullRef>,      // REQUIRED iff kind == PermutationNull (Phase 3)
    pub budget: StudyBudget,            // { max_runs, max_wall_ms }
    pub question: String,               // logged BEFORE running
    pub selection_rule: SelectionRule,  // pre-declared; how (if at all) one config is carried forward
}

pub enum StudyKind {
    ParameterSweep, WalkForward, Cpcv, NestedCv,
    PermutationNull, SyntheticPaths, CostSweep,
    TradeMonteCarlo, RegimeConditional, Neighborhood,
}

pub struct StudyResult {
    pub study_id: StudyId,
    pub member_run_ids: Vec<RunId>,     // provenance ONLY — not ranked, not promotable
    pub distribution: Distribution,     // metric, dist<f64>, median, iqr, worst_5pct, spread
    pub verdict: StudyVerdict,          // study-kind-specific summary (Phase 4 reads it)
    pub trial_delta: i64,               // runs this study added to the global counter
    pub sealed: bool,                   // INV-2: always true; best member not addressable
}
```

**`VarySpec` is what distinguishes the kinds:** `ParameterSweep` varies `params`
over a grid/sample; `WalkForward` varies `data_slice` over rolling IS/OOS windows
and re-fits per window; `Cpcv` varies which groups are train vs test across all
C(N,k) combinations; `PermutationNull` holds the config fixed and varies the null
generator's seed (each run is the strategy on once-shuffled data); `SyntheticPaths`
varies the synthetic-history seed; `CostSweep` varies `cost_model_ref` across an
optimistic→pessimistic ladder.

**Sealing is the design (spec ADR-002).** There is deliberately no `best_run`,
no `argmax`, no sorted list. `member_run_ids` is provenance for audit, returned
in **insertion order** (never sorted by metric). A warning is willpower; sealing
is structural — this removes the single largest p-hacking surface.

---

## Tasks

### ☐ J-1.1 `StudyConfig` + `VarySpec` + `StudyKind` — M
Add `crates/backtest/src/study/config.rs` with the design-notes shapes.
`VarySpec` is an enum mirroring `StudyKind` so an ill-typed pairing (e.g.
`WalkForward` with a `params` grid) does not construct. `question` is mandatory
(non-empty) and `null_ref` is required iff `kind == PermutationNull` — validated
at construction.
**Acceptance:** `cargo test -p backtest` covers round-trip; constructing a
`PermutationNull` study without `null_ref`, or any study with an empty
`question`, is rejected.

### ☐ J-1.2 Sealed `StudyResult` + `Distribution` — M
Add `Distribution { metric: MetricKind, dist: Vec<f64>, median, iqr: [f64;2],
worst_5pct, spread }` and `StudyResult` with `sealed: bool` (constructed `true`,
no setter to `false`). **No field, method, or trait on `StudyResult` returns a
single member keyed by performance** — assert this with a compile-fenced doc test
and a code-review checklist item. `member_run_ids` returns insertion order only.
**Acceptance:** a unit test asserts there is no `best`/`argmax`/`max_by` over
members reachable from `StudyResult`; `worst_5pct` and `median` compute correctly
on a fixture distribution.

### ☐ J-1.3 `StudyEngine` driver (fan-out over `Backtest::run`) — L
Add `crates/backtest/src/study/engine.rs`: `StudyEngine::run(StudyConfig) ->
StudyResult`. Expand `VarySpec` into N `RunConfig`s, execute each via the
cache-aware `Backtest::run` (Phase 0) — in parallel, respecting `budget` — collect
the chosen `MetricSet` field into the distribution, and set `trial_delta = N`
(**every** member, including cache hits and failures, counts). Failed runs
contribute to the count and are recorded in `member_run_ids`, but their metric is
excluded from `dist` (with a logged note).
**Acceptance:** a sweep of 50 configs produces a 50-member `member_run_ids`,
`trial_delta = 50`, and a distribution over the surviving (non-failed) members;
re-running the identical study re-uses cache but still reports `trial_delta = 50`.

### ☐ J-1.4 Pre-declared in-Study `SelectionRule` — M
Add `SelectionRule` (e.g. `None`, `MedianStableCentroid`, `WorstCaseRobust`) and
`StudyResult::carry_forward() -> Option<RunConfig>` that applies the
**pre-declared** rule — never an argmax. The rule is fixed in `StudyConfig`
before the Study runs and logged. For `WalkForward`, per-window param selection
uses the same mechanism (a rule, declared up front).
**Acceptance:** `carry_forward()` with `MedianStableCentroid` returns the
region-centroid config, not the peak; with `None` returns `None`; the rule is
immutable post-construction (test).

### ☐ J-1.5 `parameter_sweep` + `neighborhood` — M
Implement `ParameterSweep` (grid/Latin-hypercube sample over `params`) and
`Neighborhood` (perturb each param ±k steps around `base_config`). The
`Neighborhood` verdict reports **plateau vs spike** (ratio of neighbor median to
center). Both seal their distributions.
**Acceptance:** a synthetic objective with a broad plateau yields a plateau
verdict; an isolated spike yields a spike verdict; neither exposes the peak
config except via a declared `SelectionRule`.

### ☐ J-1.6 `walk_forward` + `cpcv` + `nested_cv` — L
Implement `WalkForward` (rolling IS/OOS windows over `data_slice`, re-fit per
window via the declared selection rule), `Cpcv` (combinatorial purged CV — all
C(N,k) train/test group combinations, with purge + embargo reused from
`features::walk_forward` if present, else a local generator), and `NestedCv`
(outer test / inner selection). Each reports an OOS-performance distribution.
**Acceptance:** CPCV over N groups produces C(N,k) members; no test index leaks
into its own training fold (property test); the OOS distribution's worst-5% is
reported and the best path is not addressable.

### ☐ J-1.7 `synthetic_paths` + `cost_sweep` + `regime_conditional` — M
Implement `SyntheticPaths` (vary synthetic-history seed; generator supplied by
the Null Library in Phase 3 — here, wire the seed-varying loop), `CostSweep`
(vary `cost_model_ref` along an optimistic→pessimistic ladder; report where the
edge dies), and `RegimeConditional` (condition the distribution on a regime
label per member).
**Acceptance:** `CostSweep` reports the cost level at which median return crosses
zero; `RegimeConditional` produces a per-regime sub-distribution; both sealed.

### ☐ J-1.8 `trade_monte_carlo` — M
Implement `TradeMonteCarlo`: resample/reorder the executed trade sequence (block
bootstrap over trades) to produce a distribution of path-dependent metrics (max
drawdown, terminal equity) holding the trade set fixed. Answers *"how lucky was
this particular ordering?"*.
**Acceptance:** the distribution's worst-5% drawdown is materially worse than the
single realized path on a fixture with autocorrelated trades; sealed.

### ☐ J-1.9 Study persistence + `question` log — M
Persist `StudyConfig` (including `question`, `null_ref`, `selection_rule`),
`StudyResult` (distribution + verdict + `trial_delta`), and `member_run_ids`
references via Postgres migration **0027** (`backtest_studies`,
`backtest_study_members`). The `question` text is stored immutably at creation,
**before** results exist.
**Acceptance:** a stored Study round-trips; the `question` is recorded at
creation time (asserted by a timestamp earlier than the result); member ids
resolve back to `RunStore` rows.

### ☐ J-1.10 INV-2 adversarial test suite — S
Add `crates/backtest/tests/sealed_distributions.rs`: attempt, by every reachable
API path, to obtain the best-performing member of each study kind — and assert
each path is absent or returns only the pre-declared `SelectionRule` output. Add
a clippy/code-review note forbidding `members.iter().max_by(metric)` outside a
`SelectionRule`.
**Acceptance:** the test compiles only because no best-member API exists; a
deliberate "expose argmax" patch makes the test fail (documented in the test).

---

## Exit criteria

- A Study runs many Runs along one `VarySpec` and reports a **sealed**
  distribution (median / IQR / worst-5% / spread) with no addressable best member
  (INV-2).
- All 10 study kinds execute and seal; `PermutationNull` requires a `null_ref`.
- `trial_delta` counts every member (cache hits + failures included).
- Carry-forward is possible **only** via a pre-declared `SelectionRule`.
- The `question` is logged before the Study runs. `cargo test -p backtest` green,
  including the INV-2 adversarial suite.
