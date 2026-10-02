# ADR-0043: Typed Model Contracts, Resolved Per Environment and Point-in-Time

**Status:** Accepted
**Date:** 2026-10-01
**Deciders:** platform architect, on the owner's direction that a strategy can use "any AI model that
can be trained and housed in the MLOps … not just an LLM" and must input and receive "the proper data
type and information in each model"
**Supersedes:** the `OutputBinding` fixed forecast-field list (DATA-004 v1.1); the sidecar's
`Forecast`-for-every-model response; zero-filling of missing model inputs
**Builds on:** ADR-0016 (distributional contract — becomes one contract among many), ADR-0017
(walk-forward CV), FEAT-006 §6 (prediction series)
**Traces:** [FEAT-007](../specs/FEAT-007-unified-strategy-engine.md) §8; QA-2, QA-3, QA-5

## Context

Every model, whatever it is, answers as a forecast: `direction` from a hard-coded 0.55/0.45
threshold, `magnitude`, `confidence`. A classifier's class probabilities, a regressor's value, a
ranker's score and an embedding are squashed or dropped. A missing input feature is silently
filled with `0.0`. Strategies reference models by alias, so the model behind a backtest changes
when the alias moves. No strategy can call a model in a backtest at all, and calling a model trained
on the whole history inside a backtest of that history would be lookahead.

## Decision

1. **Every model version carries an IoContract**: typed named inputs (scalar, feature row, feature
   window with lookback and timeframe, cross-section, text template) and typed named outputs (`num`,
   `bool`, `cat`, `probs`, `dist`, `vec`, `text`, `record`), plus horizon, PIT metadata
   (walk-forward, embargo), cost class and timeout. Existing models get a contract derived from
   their kind and bundle header.
2. **The typechecker validates every model wiring against the contract**; a mismatch is a
   validation error, not a runtime surprise.
3. **The serving protocol returns named typed outputs**, and a missing required input is an
   abstain with a reason. Zero-fill is removed.
4. **Versions are pinned**: aliases are resolved once, at backtest start or deployment, and recorded.
5. **Resolution is per environment**: in a backtest, the answer at *t* comes from the walk-forward
   fold with `train_end + embargo < t` (batched, cached as a prediction series); with no such fold
   the value is `missing(model_unavailable)` and the strategy holds. In paper and live, the pinned
   version answers through the inference gateway with a per-contract timeout. A final-model-
   everywhere backtest is allowed only as a declared `optimistic_models` policy, flagged in every
   report.
6. **Calls fit the synchronous engine by request/resume**: the engine returns the model requests a
   tick needs; the environment resolves them; the engine resumes. Chained models resolve layer by
   layer.

## Considered options

- **Keep the forecast shape and add fields.** Rejected: the shape is the problem; every new model
  type would bend it further.
- **Untyped JSON in and out.** Rejected: nothing could be checked before running, and absence would
  again be indistinguishable from a value.
- **Call the live gateway from backtests.** Rejected: slow, and lookahead unless fold-aware.

## Consequences

- Good: any model type plugs in by declaring a contract; strategies consume its outputs natively.
- Good: model-driven backtests become possible *and* honest; a leaky variant fails G0 by name.
- Bad: walk-forward folds must be trained for a model to answer early in a backtest; until then the
  strategy holds there, and the report says so.
- Bad: the Python sidecar's protocol changes (versioned; v1 kept for derived forecast contracts).
