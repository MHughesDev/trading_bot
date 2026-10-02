# ADR-0047: Every Decision Is Journaled, and Paper Is Replayed Through the Backtest Nightly

**Status:** Accepted
**Date:** 2026-10-01
**Deciders:** platform architect
**Traces:** [FEAT-007](../specs/FEAT-007-unified-strategy-engine.md) §11; QA-1, QA-2; the nightly
feature-consistency job (INV-14) is the precedent

## Context

"The backtest and the paper trader run strategies the same way" has been true by assertion before,
and was false. Sharing code (ADR-0039) makes it true today; nothing keeps it true. The one place the
platform already proves a parity claim continuously is the nightly feature-consistency diff.

## Decision

1. **Every engine step, in every environment, writes one journal record**: the decision instant, a
   digest of every input value, every model request and response with version and fold, the intents,
   the planned orders, the risk-gate verdicts, and every `missing` with its reason. Backtests keep it
   in the run artifact; paper and live in `strategy_decision_journal`.
2. **A nightly parity job** replays the previous day's paper journal — its recorded inputs, including
   the model responses it actually received — through the engine as the backtest environment builds
   it, and requires byte-identical decisions. A difference is reported with the first differing step
   and treated as an engine-boundary defect.

## Consequences

- Good: "exactly the same" is a measured property with a named failure, not a claim.
- Good: every decision is explainable after the fact — which input, which model answer, which rule.
- Bad: journal volume. Bounded: one record per decision tick per deployment (bar-paced, ADR-0042),
  digests rather than raw windows, retention per the data budget.
