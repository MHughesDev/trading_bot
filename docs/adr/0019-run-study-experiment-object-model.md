# ADR-0019: Run / Study / Experiment Object Model + Sealed Distributions

**Status:** Accepted
**Date:** 2026-06-17
**Deciders:** Platform team

## Context

The Backtest Suite ([core spec](../specs/BACKTEST_SUITE_CORE_SPEC.md), built in
[plan-set J](../plans/plan-sets/set-J/MASTER.md)) layers an honest-evaluation
architecture on top of the existing `crates/backtest` Run engine. Most
backtesters mislead not for lack of tests but because the researcher runs many
variations, remembers the few that worked, and reports those as if they were the
only attempts. The architecture must remove the human from the counting and
selecting loops.

Two foundational decisions, taken verbatim from the core spec's appendix (its
ADR-001 and ADR-002), govern everything built above them and are recorded here as
the platform's binding decisions.

## Decision

**1. The Run is a pure, dumb, content-addressed function.** A Run is
`RunConfig → RunResult` with no knowledge of cross-validation, nulls, or trial
counting. Its identity `run_id` is the SHA-256 of a canonical encoding of every
config field except the id itself (object-key order is normalized; array order is
significant). Identical configs collide on `run_id` and may be served from cache;
any field change is a new id. Runs are immutable once executed and are stored —
`ok`, `failed`, and `rejected_integrity` alike — never deleted. Failures count.

**2. Distributions are sealed.** A Study reports a distribution's properties
(median, IQR, worst-5%, spread) and exposes **no** API to select, return, or
promote the best-performing member. `member_run_ids` is provenance in insertion
order only — never ranked. A single config is carried forward solely through a
**pre-declared selection rule** (e.g. median-stable centroid), never a
user-reachable argmax.

The three layers compose strictly: `Experiment ⊃ Study ⊃ Run`. The Experiment
owns the monotonic trial counter and the one-shot holdout vault (ADR-0021); the
Study owns the sealed distribution; the Run owns one immutable result.

## Rationale

A dumb Run is cacheable by `run_id` and trivially parallelizable, which matters
because the gate funnel spawns millions of Runs. Putting all combination
intelligence one level up (the Study) keeps the atom reproducible and the counter
trustworthy: you cannot accidentally re-count a cached run or silently mutate one.

Sealing removes the single largest p-hacking surface. A warning ("don't grab the
peak") is willpower; a type with no best-member accessor is structural. Every
distribution-producing tool is an overfitting vector if its peak is addressable,
so the peak is made unaddressable.

## Consequences

- **Good:** caching and massive parallelism for free; the trial counter is
  trustworthy by construction; the largest p-hacking surface is closed.
- **Bad:** Studies carry all orchestration complexity, and a Study bug cannot be
  caught at the Run level — so every Study type and gate carries an adversarial
  test. Legitimate "carry one config forward" needs a pre-declared in-Study
  selection rule, which is more design work than a simple argmax.
- The `run_id` content hash binds reproducibility (ADR-0008 `available_time`
  ordering keeps the underlying data point-in-time) to the cache key.

## Alternatives Considered

- **Smart Runs that know about CV/nulls.** Rejected: it makes the atom
  non-cacheable, couples orchestration to execution, and bloats the unit the
  funnel spawns by the million.
- **Expose ranked members with a warning.** Rejected: a warning is willpower;
  sealing is structural. The peak must not be addressable at all.

## References

- [Backtest Suite Core Spec](../specs/BACKTEST_SUITE_CORE_SPEC.md) §0 (INV-1/2/3),
  §1.1–§1.4, Appendix ADR-001/ADR-002
- [Plan-set J](../plans/plan-sets/set-J/MASTER.md) Phases 0–1
- ADR-0008 (`available_time` ordering), ADR-0002 (Decimal money boundary),
  ADR-0014 (backtesting via market_simulator SDK)
- ADR-0021 (staged-gate funnel, trial counter & holdout vault — forthcoming)
