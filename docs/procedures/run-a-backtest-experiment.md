# Procedure — Run a Backtest Suite Experiment (Set J)

This walks one strategy idea through the honest-evaluation funnel end to end:
create an Experiment, declare its null, run research Studies (watch the trial
counter climb), pass Gates 0→3, spend the vault, go live, and reconcile. It maps
to the crates in `crates/backtest/src/{run,study,experiment,nulls,gates,reconcile,stats}/`
and is exercised by `crates/backtest/tests/funnel_e2e.rs`.

> **The three invariants** ride along the whole way: costs/counter/holdout on by
> default (INV-1), distributions are sealed (INV-2), and significance is never
> shown without its null and trial count (INV-3).

## 0. The atom — a Run

A `RunConfig` is content-addressed by `run_id = sha256(all fields)`. Build it
through `RunConfigBuilder` (never set `run_id` yourself); identical configs
collide and serve from cache, so the trial counter can't be fooled.

```rust
let cfg = RunConfigBuilder::new("ema_cross", "v-hash", data_slice, "cost:floor", "sizing:fixed", "snap:2024")
    .seed(7)
    .build();               // run_id computed; unsafe = false (INV-1)
let result = backtest.run(&cfg);   // cache-aware: executes once, then serves cached
```

## 1. Create the Experiment + declare the null

The Experiment owns the **trial counter** and the **holdout vault**. The null is
*recommended, never defaulted* — surface the recommendation, let the user choose,
and log any override reason.

```rust
let recommended = recommend_null("intraday_mean_reversion");   // → BlockPermutation (a prompt)
let null = Null::new(recommended, NullParams { block_length: Some(10), ..Default::default() })?;
null_store.put(null.clone());
null_store.record_choice(NullChoice::record("exp-1", &null, recommended, None)?);

let mut exp = Experiment::new("exp-1", "ema-family", holdout_tail_slice, null.null_id.to_string());
```

The holdout tail is now locked: any research Study whose data slice intersects it
is refused.

## 2. Research — Studies auto-increment the counter

Run Studies **through the Experiment** so the counter increments before you ever
see a result. Distributions are sealed — there is no `best_run`.

```rust
let sweep = StudyConfig { kind: ParameterSweep, vary: Params { grid }, question: "how does perf vary?".into(), .. };
let dist = exp.run_study(&sweep, &backtest)?;     // trial_counter += members
// dist.distribution → median / IQR / worst_5pct / spread   (no peak accessor)
// To carry one config forward, declare a SelectionRule (e.g. MedianStableCentroid) — never argmax.
```

## 3. The funnel — Gates 0→4

A `GateRunner` enforces order: each gate's entry requires the prior gate's
**passing** verdict.

```rust
let mut runner = GateRunner::new(&mut exp);

// GATE 0 — integrity (every config; hard stop). Catches the close-stamp leak:
// a higher-TF signal acted on before its constituent bar closed leaks the bar.
runner.gate0(&IntegrityInputs { signals, gross_return, cost_floor, .. })?;

// GATE 1 — one honest walk-forward under pessimistic costs; pass iff median > 0.
runner.gate1(&walk_forward_result)?;

// GATE 2 — robustness as a SHAPE: CPCV median > 0 AND worst-5% survivable AND a
// neighborhood plateau (not an isolated spike).
runner.gate2(&cpcv, &synthetic, &neighborhood, worst5_threshold)?;

// GATE 3 — the ONE primary permutation test, selection-bias-corrected by the
// live trial counter, with DSR + PBO as corroborators (disagreement → investigate).
let (outcome, passed) = runner.gate3(observed, &null_distribution, null.null_id, &corroborators, 0.05)?;
// outcome.significance.render() → "p=… vs null:… @ N trials"   (INV-3, inseparable)

// GATE 4 — the vault: one logged, self-sealing holdout evaluation.
let (vault_result, _verdict) = runner.gate4(&candidate_cfg, &backtest, "alice")?;
// exp.state == Validated on success; a second call is refused (VaultSpent).
```

## 4. Live + reconciliation

Promote to live, then the only permitted Studies compare realized performance to
the backtested distribution. Drift below the planned worst-5% auto-flips the
Experiment to `decaying`.

```rust
exp.transition(ExperimentState::Live)?;
let verdict = reconcile_experiment(&mut exp, &realized_returns, &backtest_distribution, 0.10)?;
// verdict.drifting == true → exp.state becomes Decaying

// Across all validated experiments, is the suite itself calibrated?
let calibration = suite_calibration(&all_reconciliation_points);
// calibration.optimistic == true → the suite's thresholds are too loose
```

## What enforces what (spec §2.3)

| Guarantee | Mechanism |
|-----------|-----------|
| Funnel can't be skipped | `GateRunner::enter` requires the prior gate's passing verdict |
| Counter can't be gamed | `Experiment::run_study` auto-increments; Gate 3 reads `trial_counter()` |
| Vault can't be peeked | Gate-3-gated, one logged access, `spent` self-seals |
| Null can't be hidden | Gate 3 emits a `SignificanceResult` (no naked p) — INV-3 |
| Best member can't be cherry-picked | `StudyResult` is sealed; only a pre-declared `SelectionRule` yields one config — INV-2 |

## References

- Spec: [`BACKTEST_SUITE_CORE_SPEC.md`](../specs/BACKTEST_SUITE_CORE_SPEC.md)
- Plan: [`plan-sets/set-J/MASTER.md`](../plans/plan-sets/set-J/MASTER.md)
- ADRs: [0019](../adr/0019-run-study-experiment-object-model.md) ·
  [0020](../adr/0020-null-library-and-selection-discipline.md) ·
  [0021](../adr/0021-staged-gate-funnel-and-honesty-mechanics.md)
- Tests: `crates/backtest/tests/funnel_e2e.rs`, `crates/backtest/tests/sealed_distributions.rs`
