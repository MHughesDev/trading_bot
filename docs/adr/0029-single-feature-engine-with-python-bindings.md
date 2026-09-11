# ADR-0029: One feature engine (Rust) with Python bindings, fail-closed on every path

**Status:** Proposed
**Date:** 2026-09-11
**Deciders:** Mason Hughes (with Claude)
**Extends:** ADR-0008 (same builders for live and replay)

## Context

Features are computed in four places, and the results disagree (BS-007 01 §5):
- the backtest (`ema_N`, `rsi_N` only);
- `crates/features` (about 35 names);
- the Python trainer, which silently skips unknown names;
- `crates/api/src/features_compute.rs`, which zero-fills them.

Models train and serve on different inputs. Most hypotheses can't be backtested.

## Decision

1. `crates/features` is the **single implementation**. It wraps the Nautilus
   indicators in the market_simulator fork, plus the platform's families (range and
   realised volatility, including realised quarticity; persistence; EDGE spread; OFI;
   calendar; regime; prediction series; text).
2. **Incremental and batch modes share one definition,** and a parity suite requires
   bit-identical results.
3. **Expression-defined features** use the SLv2 expression language. They are
   parsed, type-checked, content-hashed, and registered per project or globally.
4. **PyO3 bindings** (`tbot_features`) are used by the agent container, the
   research-runner and the trainer.
5. **Fail closed:** unknown features are errors everywhere. The Python feature
   computation and `features_compute.rs` are deleted. Backtest requirement
   derivation asks the engine.
6. Warm-up, lookback and finalisation are derived and enforced by the engine.

## Rationale

This gives parity by construction, the same principle as ADR-0008. It unlocks
every archetype for backtests, and lets the agent invent features without platform
code changes.

## Consequences

- A migration alias table maps legacy names. Model bundles trained with
  zero-filled features are flagged `requires_retrain`.
- The trainer image gains a Rust-built wheel.

## Alternatives Considered

- **Keep separate implementations with a shared name registry.** Parity would stay
  accidental.
- **Python as the source of truth.** Too slow for the live hot path, and it
  duplicates the Nautilus indicators.

## References

- [DATA-006](../specs/DATA-006-unified-feature-engine.md)
- BS-007 [07_FEATURES](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/07_FEATURES.MD)
