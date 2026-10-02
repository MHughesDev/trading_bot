# ADR-0040: Strategies Are Typed Dataflow Graphs, of Two Kinds

**Status:** Accepted
**Date:** 2026-10-01
**Deciders:** platform architect, on the owner's direction (two kinds; models, indicators and
thresholds chained, branched and parallel in any blend)
**Supersedes:** DATA-004 v1.x node model (`condition` → `signal` → `action`) as the authoring and
execution model; ADR-0007's frozen v1 format is kept readable through translation only
**Keeps:** the 2026-06-08 "strategy = per-asset instance" decision for trading strategies
**Traces:** [FEAT-007](../specs/FEAT-007-unified-strategy-engine.md) §4, §5; QA-3, QA-4, QA-5

## Context

Today a strategy is a list of conditions that emit named signals that trigger market orders. A
value cannot flow anywhere except into a comparison: a model's quantile cannot become a limit
price, a model's output cannot feed another model, a branch cannot choose between two sub-chains.
Model outputs are pushed into a flat `feature('…')` namespace typed only as `f64`, and absence is
`NaN`, which compares false and is otherwise indistinguishable from a real value.

The owner asked for two different questions to be answerable — *what should we buy* and *what do
we do with this asset* — and for models, indicators and thresholds to compose freely.

## Decision

1. **A strategy is a typed dataflow graph** (`format: tbot.strategy/3`): nodes are operators from a
   registry; edges carry typed values (`num`, `bool`, `cat`, `probs`, `dist`, `vec`, `text`,
   `record`); the graph is evaluated in topological order per decision tick. Chains are wires,
   branches are `if`/`switch`, parallel paths are independent sub-graphs that meet later.
2. **Absence is a value**: `missing(reason)` replaces `NaN`. Arithmetic propagates it, boolean logic
   is Kleene three-valued, explicit handlers (`coalesce`, `is_missing`, `hold_last`) make absence a
   choice, and **an intent whose inputs are missing becomes `hold`**, journaled with the reason.
3. **Two kinds**, declared by `kind`:
   - **trading** — bound to one instrument per running instance; ends in trade-intent rules (ADR-0041);
   - **selection** — evaluated cross-sectionally over a universe; ends in a `select` sink returning
     ranked assets with scores and attributes.
4. **Deployments bind them**: a trading strategy deployed against `Instrument(id)` or against
   `Selection { … top_n, rebalance, on_removal }`.
5. **Anything the operator library cannot express is a model** (`transform` kind, ADR-0043).

## Considered options

- **Extend the v1 node list** with more node types. Rejected: the signal/action wiring is the
  limitation; every new capability becomes a new special node and a new evaluator branch.
- **Strategies as arbitrary code** (FEAT-004 Layer 1). Rejected as the *primary* form: code cannot
  be type-checked against model contracts before it runs, cannot be edited structurally by a small
  model, cannot be point-in-time verified statically, and cannot run identically in the live hot
  path without a sandbox. Kept as an escape hatch through `transform` models.
- **A single "multi-asset strategy"** that both selects and trades. Rejected: it reintroduces the
  multi-asset side effects the June decision removed, and makes every trading strategy pay for
  cross-sectional machinery it does not use.

## Consequences

- Good: all-model, no-model and mixed strategies are the same thing with different nodes.
- Good: the typechecker can refuse a wrongly-typed model input before anything runs.
- Good: selection logic finally has a caller and a consumer.
- Bad: a new canonical format; every stored v1 definition needs translation (FEAT-007 §12).
- Bad: graph editors are harder to build than form editors; mitigated by SLv3 text and builder
  tool-calls being first-class views of the same graph.
