# Backtest Suite — Core Spec

**Scope:** The Backtest module only. Data ingestion (1-min bars + constructed higher timeframes) and strategy definition live in other modules and are treated here as defined, validated inputs. This document specifies two load-bearing subsystems:

1. The object model — **Run / Study / Experiment** — and what each logs.
2. The **Null Library** and the **staged-gate** evaluation logic.

Everything is language-neutral. Field types use a simple notation (`str`, `int`, `float`, `datetime`, `enum`, `hash`, `ref<X>` = reference to an object of type X, `dist<float>` = an empirical distribution of floats).

> **Implementation plan:** [`docs/plans/plan-sets/set-J/`](../plans/plan-sets/set-J/MASTER.md) — Set J builds this spec on top of the existing `crates/backtest` Run engine.

---

## Part 0 — Three governing invariants

These bind every component below. They are stated once here and assumed everywhere.

- **INV-1 (Skeptical by default):** Costs, the global trial counter, and the holdout lock are *on* by default. They must be explicitly disabled, and any run with them disabled is permanently flagged `unsafe=true`.
- **INV-2 (Distributions are sealed):** Any Study that produces a distribution reports the distribution's properties. It exposes no API to select, return, or promote the best-performing member. The best member is not addressable.
- **INV-3 (Significance is never naked):** A significance result is invalid unless it carries (a) the null it was tested against and (b) the trial count at the moment it was computed. The report renders all three inseparably or not at all.

---

# PART 1 — The Object Model

## 1.1 Run — the atom

A **Run** is the smallest reproducible unit: one strategy, one parameter set, one data slice, one cost model, one seed → one equity curve and its metrics. A Run is **immutable** once executed. You never edit a Run; you create a new one.

### RunConfig (the full reproducible input)

```
RunConfig {
  run_id:            str            # deterministic hash of all fields below
  strategy_ref:      ref<Strategy>  # from Strategy module; pinned by version hash
  strategy_version:  hash           # exact strategy code/definition version
  params:            map<str,any>   # the specific parameter set for THIS run
  data_slice: {
    universe_ref:    ref<Universe>  # symbol set, pinned by membership-calendar version
    start:           datetime
    end:             datetime
    base_resolution: enum {1m}      # always 1m foundation
    eval_resolution: enum {1m,5m,10m,15m,30m,1h,1d}
    construction:    enum {close_stamped}   # see INV on bar construction below
  }
  cost_model_ref:    ref<CostModel> # commission/slippage/spread/borrow/latency profile
  fill_model:        enum {next_bar_open, current_close, limit_prob, pessimistic_intrabar}
  sizing_ref:        ref<SizingModel>
  seed:              int            # controls all stochastic elements in THIS run
  data_snapshot:     hash           # point-in-time data version; guarantees reproducibility
  unsafe:            bool = false   # true if any default protection was disabled
}
```

The `run_id` is the hash of the entire config. **Two configs that differ in any field produce different run_ids; two identical configs produce the same run_id and may be served from cache.** This is what makes the trial counter trustworthy — you cannot accidentally re-count a cached identical run, and you cannot silently mutate a run.

### RunResult (the full output)

```
RunResult {
  run_id:            str            # matches the config that produced it
  status:            enum {ok, failed, rejected_integrity}
  equity_curve:      series<datetime,float>
  positions:         series<datetime, map<symbol,float>>
  trades:            list<Trade>    # entry/exit, MAE, MFE, holding period, costs paid
  metrics:           MetricSet      # see below
  integrity_flags:   list<Flag>     # leakage/lookahead/cost-sanity findings
  compute_cost:      { wall_ms:int, cpu_ms:int }   # for funnel budgeting
  produced_at:       datetime
  produced_by:       hash           # engine version
}
```

### MetricSet (standardized; every Run produces the same shape)

```
MetricSet {
  # return
  cagr, total_return, ann_vol: float
  sharpe, sortino, calmar, information_ratio: float
  alpha, beta: float                # vs the universe benchmark
  detrended_sharpe: float           # net of market drift + net of avg position bias (Aronson)
  # risk
  max_drawdown, avg_drawdown: float
  time_in_drawdown_pct: float
  cvar_95, ulcer_index: float
  # activity
  turnover, exposure_gross, exposure_net: float
  hit_rate, profit_factor: float
  n_trades: int
  # honesty hooks (populated by Studies, null at Run level)
  trial_count_at_eval: int = null
  is_oos_gap: float = null          # in-sample minus out-of-sample, when applicable
}
```

> **Design note — why the Run is dumb.** The Run does not know about cross-validation, nulls, or trial counting. It is a pure function: `RunConfig → RunResult`. All intelligence about *how runs combine* lives one level up, in the Study. This keeps the atom cacheable and trivially parallelizable, which matters enormously because the funnel will spawn millions of them.

### What the Run logs

Every executed Run is written immutably to the run store: full `RunConfig`, full `RunResult`, engine version, and timestamps. Nothing is ever deleted. Failed and rejected runs are logged too — **the failures are part of the trial count.**

---

## 1.2 Study — a deliberate set of Runs answering one question

A **Study** orchestrates many Runs along one varying dimension and reports the *distribution* of outcomes. It is where INV-2 is enforced.

```
StudyConfig {
  study_id:       str
  type:           enum {                 # see catalog below
                    parameter_sweep, walk_forward, cpcv, nested_cv,
                    permutation_null, synthetic_paths, cost_sweep,
                    trade_monte_carlo, regime_conditional, neighborhood
                  }
  base_config:    RunConfig              # the "center" config being studied
  vary:           VarySpec               # what dimension this study perturbs
  null_ref:       ref<Null> = null       # REQUIRED iff type == permutation_null
  budget:         { max_runs:int, max_wall_ms:int }
  question:       str                     # human-readable; logged. "Is the edge robust to history?"
}
```

The `VarySpec` is what distinguishes Study types. Examples:

- `parameter_sweep` → varies `params` over a grid/sample.
- `walk_forward` → varies `data_slice` over rolling IS/OOS windows; re-fits models per window.
- `cpcv` → varies which groups are train vs test across all C(N,k) combinations.
- `permutation_null` → holds config fixed, varies the *seed of the null generator* (each run is the strategy on once-shuffled data).
- `synthetic_paths` → varies the synthetic-history seed.
- `cost_sweep` → varies `cost_model_ref` across an optimistic→pessimistic ladder.

### StudyResult — distribution-first, best-member-sealed

```
StudyResult {
  study_id:        str
  member_run_ids:  list<str>        # provenance only; NOT ranked, NOT promotable
  distribution: {                   # the actual product of the study
    metric:        enum             # which metric the distribution is over (e.g. detrended_sharpe)
    dist:          dist<float>      # empirical distribution across members
    median:        float
    iqr:           [float,float]
    worst_5pct:    float            # tail — what you should actually plan around
    spread:        float
  }
  verdict:         StudyVerdict     # study-type-specific summary (see funnel)
  trial_delta:     int              # how many runs this study added to the global counter
  sealed:          bool = true      # INV-2: best member not addressable through any field
}
```

> There is deliberately **no `best_run` field, no `argmax`, no sorted list.** A `parameter_sweep` Study answers "how does performance vary across this parameter region" (you want a broad plateau), not "which parameter won." If a downstream consumer needs a single config to carry forward — e.g. walk-forward must pick params per window — that selection happens *inside* the Study under a fixed, pre-declared rule (e.g. "median-stable region centroid"), never by the user reaching in and grabbing the peak.

### What the Study logs

The `StudyConfig` (including the human-readable `question` and the null reference if any), the `StudyResult` distribution, the `trial_delta`, and references to every member run_id. The question text matters: it forces you to state, before running, what you were testing — which is the single best defense against post-hoc reinterpretation of results.

---

## 1.3 Experiment — the whole investigation of one strategy idea

An **Experiment** is the container for everything you do to one strategy idea, across its whole life. It owns the two things that make the suite honest: the **global trial counter** and the **holdout vault**.

```
Experiment {
  experiment_id:   str
  strategy_family: ref<Strategy>    # the idea being investigated (version-agnostic root)
  state:           enum {           # lifecycle — gates the allowed operations
                     candidate, validated, live, decaying, retired
                   }
  studies:         list<ref<Study>>
  trial_counter:   int              # AUTOMATIC, MONOTONIC, IRREVERSIBLE
  holdout: {
    slice:         data_slice       # the locked tail of data
    access_log:    list<{when:datetime, run_id:str, by:user}>  # every touch, forever
    spent:         bool             # true after the single permitted vault run
  }
  primary_test:    ref<Null>        # the ONE designated significance test (declared up front)
  verdict:         ExperimentVerdict
  created, updated: datetime
}
```

### The trial counter — the spine of honest significance

- It increments by `study.trial_delta` on **every** Study, including parameter sweeps, failed runs, and abandoned directions.
- It is **monotonic and irreversible** — there is no decrement, no reset, no "fresh start." Starting over means a *new* Experiment with a *new* idea; you cannot launder trial count by renaming.
- It is the input to selection-bias correction and to the Deflated Sharpe Ratio. A Sharpe of 1.5 after 3 trials and after 3,000 trials are radically different claims, and the counter is what forces that into the math automatically (INV-3).

> **Design note — the counter is the point.** Most backtesters fail not because they lack tests but because the user runs 500 variations, remembers the 3 that worked, and reports those as if they were the only attempts. The counter removes the human from the counting loop. You physically cannot under-report trials because the suite counts them for you, at the Study level, before you ever see a result.

### The holdout vault

- A slice of data (typically the most recent tail) that **no Study can address** while the Experiment is in `candidate` or `validated` state.
- Reaching it requires the Experiment to pass the full gate sequence (Part 2) and to be explicitly promoted to a final-validation run.
- The vault grants **exactly one** evaluation. `spent` flips to true and never flips back. Every access is permanently logged with who and when. A second attempt is refused at the API level.

### Lifecycle states and what each permits

| State | Meaning | Allowed operations |
|---|---|---|
| `candidate` | under active research | all Studies except vault; trial counter runs hot |
| `validated` | passed all gates, vault spent | no further research Studies (would invalidate the vault result); may promote to live |
| `live` | deployed | only live-vs-backtest reconciliation Studies |
| `decaying` | live perf drifting below backtest distribution | reconciliation + diagnostic Studies; flagged for review |
| `retired` | pulled | read-only |

The state machine is one-directional through validation: once you spend the vault and reach `validated`, you cannot drop back to `candidate` and keep researching against the same holdout. Doing more research requires a fresh holdout (new data) and a fresh Experiment.

## 1.4 How the three compose

```
Experiment (one strategy idea, owns trial counter + vault)
 └── Study (one question, owns a distribution, sealed)
      └── Run (one config, immutable, cacheable, dumb)
```

A single research session might be: Experiment E → Study(parameter_sweep, 200 runs) → Study(cpcv, 120 runs) → Study(permutation_null, 1000 runs). Trial counter after that session = 1,320, and that 1,320 is what the significance math sees. No member of any Study is independently promotable; only the Experiment as a whole advances through the lifecycle.

---

# PART 2 — The Null Library and the Staged Gates

## 2.1 Why the null is a first-class object

A permutation test's entire validity rests on the null being appropriate to the question. The wrong null gives a confident, precise, *meaningless* p-value. The right null differs by strategy type, time horizon, and what structure must be preserved. So the null is not a hidden default — it is a selectable, parameterized, logged object that the user must choose, and that travels attached to every significance result (INV-3).

### Null contract

```
Null {
  null_id:     str
  type:        enum {                  # catalog below
                 signal_return_decouple, block_permutation,
                 stationary_bootstrap, bar_permutation,
                 synthetic_garch, regime_block, random_entry_matched
               }
  params:      map<str,any>            # e.g. block_length, n_resamples
  preserves:   list<str>               # what structure this null KEEPS intact
  destroys:    list<str>               # what it BREAKS (the thing being tested)
  generate:    fn(data, seed) -> data' # produces one null-world dataset
}
```

The `preserves`/`destroys` fields are not documentation — they are the explicit statement of the hypothesis, rendered in every report. A null that destroys the wrong thing is the most common silent error in this whole domain.

### Null catalog (which null for which question)

| Null type | Preserves | Destroys | Use for |
|---|---|---|---|
| `signal_return_decouple` | marginal return dist, signal dist | the *pairing* of signal→forward-return | "does the signal predict, or is it coincidence?" — general purpose |
| `block_permutation` | short-horizon autocorrelation (within block) | signal timing across blocks | intraday / mean-reversion where serial correlation matters |
| `stationary_bootstrap` | autocorrelation structure (random block lengths) | specific historical ordering | return-distribution robustness; daily trend |
| `bar_permutation` | bar-level OHLC integrity | inter-bar sequence | testing whether sequence (not just bar shape) carries the edge |
| `synthetic_garch` | volatility clustering, fat tails | the specific realized path | "would this work in markets that *could* have happened?" |
| `regime_block` | within-regime structure | cross-regime arrangement | strategies suspected of being one-regime wonders |
| `random_entry_matched` | trade frequency, holding period, exposure | entry *timing* skill | "is the edge in timing, or just in being in the market?" |

> **Design note — the null is chosen, not defaulted.** The suite ships with a *recommended* null per declared strategy type (e.g. block_permutation for an intraday mean-reversion strategy), but the recommendation is surfaced as a prompt, logged as a user decision, and overridable with a logged reason. There is no invisible default, because an invisible default is an invisible assumption.

---

## 2.2 The staged gates — a funnel, not a menu

Tests are ordered by cost-per-unit-of-discriminating-power. Cheap, high-power filters run first on everything; expensive tests run only on survivors. The ordering simultaneously solves the compute explosion and enforces the discipline (you cannot reach the vault without surviving the cheap gates).

```
   [ candidate strategy ]
            │
   GATE 0 — INTEGRITY            cheap, always, fail = hard stop
            │ pass
   GATE 1 — SINGLE-PATH SANITY   cheap, one honest walk-forward
            │ pass
   GATE 2 — ROBUSTNESS           moderate, distribution over histories
            │ pass
   GATE 3 — SIGNIFICANCE         expensive, the primary null test
            │ pass
   GATE 4 — THE VAULT            one shot, irreversible, logged
            │ pass
   [ validated → eligible for live ]
```

### GATE 0 — Integrity (runs always, on every config)

Automated checks before any performance claim is allowed to exist:

- **Lookahead / leakage scan:** does any feature read data timestamped at or after the decision bar? Specifically for your stack: **is every higher-timeframe signal stamped at the constituent bar's _close_, never its open?** A 1-min-constructed daily bar is only complete at the daily close; a signal that acts on it earlier has leaked the entire bar. This is your single most likely real-world leak and it is checked here, mechanically.
- **Cost sanity:** is the *gross* edge even larger than minimum realistic cost? If the edge dies under the floor cost model, it dies here.
- **Look-ahead in labels** (model strategies): does the label horizon overlap the feature window without purging?

Failure here sets `status = rejected_integrity` and the strategy proceeds no further. Cost: trivial. Run on 100% of configs.

### GATE 1 — Single-path sanity (cheap)

One honest walk-forward run with the **pessimistic** cost model. If it is not profitable on a single forward path with realistic costs, stop — no amount of fancy resampling rescues a strategy that fails the simplest honest test. Cost: one walk-forward Study. Run on Gate-0 survivors.

### GATE 2 — Robustness to history (moderate)

Three Studies, each producing a sealed distribution:

- **CPCV** → distribution of OOS performance across many train/test combinations. You read the *median and the worst-5%*, never the best path.
- **Synthetic paths** (`synthetic_garch` or `stationary_bootstrap`) → would the edge survive in plausible alternate histories?
- **Neighborhood** → performance across the parameter region around the chosen config. Broad plateau = robust; isolated spike = overfit.

Gate-2 verdict is a *distribution shape*, not a number. A strategy passes if its OOS distribution is positive at the median **and** survivable at the worst-5%, and its parameter neighborhood is a plateau. Cost: moderate, parallelized. Run on Gate-1 survivors only — typically a handful.

### GATE 3 — Significance (expensive)

The **single primary test**, declared on the Experiment up front: a permutation test against the chosen `Null`, **selection-bias-corrected using the live trial counter**. This is the *the* p-value. It is computed once, against one null, with the trial count baked in (INV-3).

Corroborating diagnostics computed alongside — **Deflated Sharpe Ratio** and **Probability of Backtest Overfitting** — are *not* additional p-values to shop between. They should agree with the primary. **Disagreement is a flag to investigate, not a result to pick from.** This is what kills the "15 co-equal tests" multiplicity problem from the earlier design: there is one verdict and two corroborators, not seventeen votes.

Cost: high (1,000+ null worlds × walk-forward each), heavily parallelized. Run only on Gate-2 survivors — by now, one or two candidates.

### GATE 4 — The vault (one shot)

The Experiment runs **once** against the locked holdout. The holdout has never influenced any parameter, any selection, any null, anything. This is the only truly out-of-sample number in the entire process. `holdout.spent` flips to true; the access is logged forever. Pass → `validated`. There is no retry; a failed vault run means the idea is dead *for this holdout*, and continuing requires genuinely new data.

### After the gates — the reconciliation loop (live state)

Once `live`, the only Studies permitted compare realized performance against the backtested distribution for the same period. When live performance drifts below the backtest's predicted distribution (e.g. falls under the worst-5% you planned around), the Experiment auto-transitions to `decaying` and flags for review. This is the loop that catches the overfit that survived every gate — and, over time, tells you whether your *whole suite* is calibrated.

---

## 2.3 How the pieces enforce each other

- The **funnel** can't be skipped because each gate's entry requires the prior gate's pass verdict, which only exists if its Studies ran.
- The **trial counter** can't be gamed because Studies increment it automatically, and Gate 3's significance math reads it directly.
- The **vault** can't be peeked because it's addressable only from a `validated`-eligible Experiment, grants one logged access, and self-seals.
- The **null** can't be hidden because Gate 3 refuses to emit a verdict without one attached (INV-3).
- The **best member** can't be cherry-picked because Studies are sealed (INV-2).

The honesty is structural, not a matter of user willpower. That is the whole design goal: a researcher acting normally — iterating, trying things, chasing the promising direction — is automatically prevented from fooling themselves, because the architecture counts, seals, gates, and locks on their behalf.

---

## Appendix — Two decisions worth recording formally

### ADR-001 — The Run is a pure, dumb function

**Status:** Accepted.
**Context:** The funnel spawns millions of runs; intelligence about combination lives in Studies.
**Options:** (A) smart Runs that know about CV/nulls; (B) dumb Runs, smart Studies.
**Decision:** (B). Keeps the atom cacheable, parallelizable, and trivially reproducible.
**Consequence:** Good — caching by `run_id` and massive parallelism for free. Bad — Studies carry all orchestration complexity; a Study bug can't be caught at the Run level.

### ADR-002 — Distributions are sealed (no best-member access)

**Status:** Accepted.
**Context:** Every distribution-producing tool is an overfitting vector if the user can grab its peak.
**Options:** (A) expose ranked members with a warning; (B) seal — best member not addressable.
**Decision:** (B). A warning is willpower; sealing is structural.
**Consequence:** Good — removes the single largest p-hacking surface. Bad — legitimate "carry one config forward" needs a pre-declared in-Study selection rule, which is more work to design than a simple argmax.
