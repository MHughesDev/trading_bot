# ADR-0020: The Null Library & Null-Selection Discipline

**Status:** Accepted
**Date:** 2026-06-17
**Deciders:** Platform team

## Context

The Backtest Suite ([core spec](../specs/BACKTEST_SUITE_CORE_SPEC.md), [plan-set
J](../plans/plan-sets/set-J/MASTER.md)) establishes significance via permutation
tests (Gate 3, Phase 4). A permutation test's entire validity rests on the null
being appropriate to the question: the wrong null gives a confident, precise,
*meaningless* p-value. The right null differs by strategy type, time horizon, and
what structure must be preserved. A null that destroys the wrong thing is the most
common silent error in this whole domain.

The suite must therefore treat the null not as a hidden default but as a
first-class object the researcher selects, parameterizes, and logs — and which
travels attached to every significance result (INV-3).

## Decision

**1. The null is a first-class, content-addressed object.** `Null { null_id,
kind, params, preserves, destroys }`. `null_id` is the SHA-256 of `kind + params`
(identical specs collide; a new params set is a new null). Seven kinds ship:
`signal_return_decouple`, `block_permutation`, `stationary_bootstrap`,
`bar_permutation`, `synthetic_garch`, `regime_block`, `random_entry_matched`.

**2. `preserves`/`destroys` are mandatory and rendered.** They are not
documentation — they are the explicit statement of the hypothesis (what structure
the null keeps vs breaks), non-empty by construction, and shown in every report.

**3. The null is recommended, never defaulted.** `recommend_null(strategy_type)`
seeds a *prompt*; the researcher must choose explicitly. The chosen null and any
override reason are logged per Experiment (`NullChoice`). An override (choosing a
kind other than the recommendation) requires a logged reason. A `permutation_null`
Study cannot run without an explicitly selected `null_ref` (enforced at
`StudyConfig` validation).

**4. Significance is never naked (INV-3).** `SignificanceResult { p_value,
null_ref, trial_count_at_eval }` has no constructor that omits the null or the
trial count. Gate 3 produces it; the workbench renders all three inseparably or
renders nothing.

**5. The math stays in Rust (D-9).** Null generation (permutation, bootstrap,
GARCH simulation) is deterministic numeric compute in `crates/backtest/src/nulls`;
no Python sidecar is introduced. Each generator is deterministic given a seed, so
the `permutation_null`/`synthetic_paths` Studies reproduce exactly.

## Rationale

An invisible default is an invisible assumption. Surfacing the recommendation as
a logged, overridable choice forces the researcher to state the hypothesis the
null encodes. Pinning `preserves`/`destroys` to the type and rendering them
everywhere makes "this null destroys the wrong thing" visible rather than silent.
Making `SignificanceResult` unconstructable without its null and trial count turns
INV-3 from a guideline into a type-level guarantee.

## Consequences

- **Good:** every p-value carries its hypothesis and its multiplicity context;
  the most common silent error (wrong null) is surfaced; results are reproducible.
- **Cost:** the researcher must choose a null (more friction than a default) — by
  design. Adding a new null kind means adding a generator + its preserves/destroys
  catalog entry + a property test on what it keeps and breaks.
- Generators operate on a neutral `NullData` (bars / signals / forward-returns /
  regime labels / trades), decoupling the library from the simulator.

## Alternatives Considered

- **A single hidden default null.** Rejected: it hides the load-bearing
  assumption of the whole significance claim.
- **Let Gate 3 emit a bare p-value and attach the null in the UI.** Rejected: a
  p-value that can exist without its null is a p-value that will be quoted without
  it. INV-3 must be structural.

## References

- [Backtest Suite Core Spec](../specs/BACKTEST_SUITE_CORE_SPEC.md) §0 (INV-3),
  §2.1 (Null contract + catalog)
- [Plan-set J](../plans/plan-sets/set-J/MASTER.md) Phase 3
- ADR-0019 (Run/Study/Experiment object model), ADR-0021 (staged-gate funnel —
  consumes the null at Gate 3; forthcoming)
