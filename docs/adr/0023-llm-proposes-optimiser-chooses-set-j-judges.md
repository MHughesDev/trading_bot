# ADR-0023: The LLM proposes structure, an optimiser chooses parameters, Set J judges

Date: 2026-09-08
Status: Accepted (Phase 1 implemented; see FEAT-003 for phases 2–4)

## Context

ADR-0022 gave the platform an internal agent that designs strategies and runs
backtests through the same tool layer as the MCP front door. Left as built it
was a guess-and-check loop: the model wrote a definition, created a raw
backtest, read a summary, and guessed again — up to fifteen times — with no
objective, no parameter search, and no path through the Backtest Suite (Set J)
that exists precisely to say whether a result is real.

Two facts shaped the design. First, the search space factors into *structure*
(discrete, needs ideas), *parameters* (numeric) and *context* (when a structure
works), and each wants different machinery. Second, the Suite's invariants are
non-negotiable: costs, counter and holdout on by default (INV-1), distributions
sealed with no addressable best member (INV-2), significance never reported
without its null and trial count (INV-3).

## Decision

1. **The LLM never sets a numeric parameter.** Definitions declare a typed
   `parameters` block (v1.2, additive); expressions reference `param('x')` or
   embed `{{x}}`; [`materialize`](../../crates/domain/src/strategy_def/params.rs)
   substitutes literals before validation and execution so the frozen v1.0
   grammar, runtime and simulator are untouched. The agent may only *narrow*
   declared ranges. A sampler chooses values.

2. **Sampling is exploration; carry-forward is Set J's.** The sweep engine
   (`crates/research`) runs sampler-proposed batches as sealed
   `ParameterSweep` Studies on an Experiment — every sample a counted trial —
   then one neighbourhood Study whose pre-declared
   `SelectionRule::MedianStableCentroid` is the **only** thing carried
   forward. The sampler sees per-sample scores through an in-process,
   never-over-HTTP path (`SuiteManager::run_param_batch`); the `SweepReport`
   exposes a sealed distribution, a surface *description* and the rule's
   output. A property test asserts no ranked sample list can serialise out.

3. **Runs are real.** `SimRunExecutor` resolves the stored definition, applies
   `RunConfig.params`, loads bars from ClickHouse and drives the
   `market_simulator` engine — the Set K live leg the Suite had deferred. Runs
   never collect data (ADR-001: a Run is a pure function of stored data).

4. **The agent cannot create raw backtests.** Its tool profile drops
   `create_backtest`/`rerun_backtest`; Experiments and Studies are the only way
   it can cause a Run, so everything it reads is sealed and counted.

5. **Diagnostics drive structural edits.** A `DiagnosticBundle` (trade shape,
   monthly slices, worst trades, longest drawdown, exposure, cost drag) is what
   the model reads before proposing one structural change as a new versioned
   strategy and a new Experiment.

6. **Racing waits for calibration.** Multi-fidelity successive halving is
   implemented as a measured gate (`FidelityCalibration`, Spearman ρ ≥ 0.8
   between tiers) and stays *off* until a calibration exists for the
   instrument. Full fidelity is slower but never wrong by design.

## Consequences

- Objectives are data (`Objective { primary, constraints, aggregate }`),
  immutable per Experiment; win rate is never a primary; `MinTrades` defaults
  to 50. `MetricKind::Expectancy` was added.
- `SuiteManager` is generic over a boxed executor and no longer holds its lock
  while a Study executes; the study/funnel/vault handlers run on blocking
  threads. The funnel still executes under the lock (pre-existing; a known
  limitation to lift in Phase 2).
- Experiments carry `strategy_ref` (the concrete stored slug) and an optional
  `objective`; `RunStudySpec` accepts `base_params` so a neighbourhood holds
  other dimensions where research left them.
- New HTTP surface under `/api/research/*` and fourteen agent tools; the
  agent's system prompt teaches the research protocol and the termination
  contract now cites the Experiment.
- Suite state (experiments, studies, runs) and sweep jobs remain in-memory,
  as Set J shipped; persistence is a separate item.

## Alternatives rejected

- *Let the LLM tune numbers.* Expensive, poorly calibrated, non-reproducible,
  and it puts the model's worst failure mode inside the loop.
- *A supervised "predict the best strategy" model.* There is no ground-truth
  label; the only signal is backtest performance, which is what the Suite
  already measures honestly.
- *Argmax carry-forward with a robustness check afterwards.* Violates INV-2
  and rewards spikes; the stable-centroid rule already existed and is what a
  plateau-seeking researcher wants.
