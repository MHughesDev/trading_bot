# ADR-0026: Two-layer strategy representation — Python research strategies and Strategy Language v2

**Status:** Proposed
**Date:** 2026-09-11
**Deciders:** Mason Hughes (with Claude)
**Supersedes (in part):** ADR-0007 (the frozen v1.0 grammar stays readable; v2 becomes the authoring format)

## Context

ADR-0007 froze a strategy grammar with:
- one comparison;
- no functions, lookback or state;
- a single `place_order` action.

The JSON node graph was designed for a drag-and-drop builder. Backtests support
only EMA and RSI (BS-007 G-01), exits are dropped (G-04), and sizing is fixed
(G-05). The agent writes code fluently, and research needs unbounded
expressiveness. Deployment needs safety, speed and live/backtest parity.

## Decision

1. **Layer 1: research strategies** are pure Python functions in the agent's
   workspace, mapping PIT data to target positions. The backtest service executes
   them as `position_series` runs under an explicit execution policy and cost
   model. A **truncation test** re-runs the function on truncated data at Gate 0;
   any difference in pre-truncation positions is look-ahead and fails the gate.
   Layer 1 strategies are never deployable.
2. **Layer 2: Strategy Language v2 (SLv2)** is a text DSL, parsed to a canonical
   **AST (DATA-004 v2)**, validated, and compiled to the existing bytecode VM
   (`crates/strategy-runtime/src/bytecode.rs`). It covers:
   - expressions over the feature engine;
   - state;
   - entries, exits, sizing, orders and risk;
   - typed model calls.
   SLv2 is the only deployable form. The AST is the storage format and the UI's
   edit target.
3. **Reconciliation** is required before a Layer 1 idea is promoted. Its SLv2
   translation must reproduce the Layer 1 position series above a threshold (Set J
   `reconcile`).
4. Every v1.x definition translates mechanically to v2. Versions become immutable
   content hashes.

## Rationale

- Code gives the agent the whole long tail of research: models, text features,
  cross-asset inputs. The truncation test makes look-ahead mechanically
  detectable, because the strategy is a function.
- A constrained, AST-evaluated deployable layer gives stability and parity (Hubble,
  arXiv 2604.09601). It also reuses the tested `StrategyInstance` runtime.
- Adding node kinds one at a time would not fix an expression model that is too
  small.

## Consequences

- New work:
  - SLv2 parser, type checker and compiler;
  - the v1 → v2 translator;
  - a research-runner sandbox;
  - the `position_series` run kind;
  - the reconciliation gate.
- The frontend builder becomes an editor over the AST.
- Both layers count trials identically (INV-1).

## Alternatives Considered

- **SLv2 only.** Safe, but it slows exploration and limits what can be expressed.
- **Python only, including live.** Untrusted code on the live hot path, and a
  harder Rust runtime.
- **Extend the v1 JSON graph with more node kinds.** It doesn't scale, and it's
  agent-hostile.

## References

- [FEAT-004](../specs/FEAT-004-strategy-representation-v2.md)
- BS-007 [08_STRATEGIES](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/08_STRATEGIES.MD)
- ADR-0007, ADR-0010
