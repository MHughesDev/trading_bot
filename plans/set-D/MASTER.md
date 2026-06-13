# Backtesting System — Decoupling & Hardening Master Plan — Set D

**Completion: 0% (0 / 33 primary tasks complete)**

> Phase 0-ALT (4 tasks) is an alternative to Phase 0 and is tracked separately,
> not counted in the primary total.

## Overview

The backtesting system (trading_bot ⇄ market_simulator) is built and merged on
branch `claude/backtesting-system-ui-i9rn2p`. Set D covers the **follow-up
work**: decoupling the two repositories so `market_simulator` is a standalone
GitHub dependency, then resolving every concern and note raised during the
build review.

The single change that forces the repos to live side by side today is the path
dependency in `crates/backtest/Cargo.toml`. **Phase 0 removes it.** Phases 1–5
work through correctness, security, robustness, fidelity, testing, and frontend
items, each cross-referenced to the original concern number (`#n`).

---

## Phase Summary

| Phase | File | Label | Tasks | Completion | Goal |
|-------|------|-------|-------|------------|------|
| 0 | [phase-0.md](phase-0.md) | Decouple the repos | 8 | 0% | market_simulator as a pinned git dependency; no sibling checkout |
| 0-ALT | [phase-0-alt.md](phase-0-alt.md) | Out-of-process boundary | 4 | 0% | *(alternative)* full runtime/toolchain separation via JSON service |
| 1 | [phase-1.md](phase-1.md) | Correctness & security | 5 | 0% | parameterized inserts, real precision, scoped auth, input validation |
| 2 | [phase-2.md](phase-2.md) | Robustness & operations | 6 | 0% | timeouts/retries, concurrency cap, auto-migrate, holiday calendar |
| 3 | [phase-3.md](phase-3.md) | Simulation fidelity | 5 | 0% | replay stored features, gap-merge fix, tick replay, warm-up |
| 4 | [phase-4.md](phase-4.md) | Testing & conventions | 5 | 0% | e2e test, manager/sim coverage, lints, security review, specs/ADR |
| 5 | [phase-5.md](phase-5.md) | Frontend | 4 | 0% | client consolidation, strategy picker, code-split, date picker |

**Recommended sequencing:** 0 → 1 → 2 yields a decoupled, correct, safe-to-run
system. 3 → 4 → 5 is fidelity, test depth, and polish. Run 0-ALT **instead of**
0 only if "completely separate" must also mean separate process/runtime.

---

## Concern → Phase Map

Every concern and actionable note from the review, and where it is addressed.

| # | Concern | Phase · Task |
|---|---------|--------------|
| 1 | Path dependency couples the repos | 0 (all) / 0-ALT |
| 2 | Toolchain pin not committed | 0.4, 0.5 |
| 3 | New crate opts out of workspace lints | 4.3 |
| 4 | Stored features recomputed, not replayed | 3.1 |
| 5 | Bars-only, no tick/quote replay | 3.3 |
| 6 | Signal semantics are an approximation | 3.5, 4.2 |
| 7 | Limited strategy surface | 3.5 |
| 8 | Precision inferred from data | 1.2 |
| 9 | Warm-up heuristic | 3.4 |
| 10 | Binance symbol substitution | 1.3 |
| 11 | Raw SQL string-built insert | 1.1 |
| 12 | Collectors: no timeout/retry/backoff | 2.1 |
| 13 | No market-holiday calendar | 2.4 |
| 14 | Coverage threshold + gap-merge over-reach | 3.2 |
| 15 | UI asset classes exceed collector support | 1.4 |
| 16 | Placeholder auth, no user scoping | 1.5 |
| 17 | Security review not run | 4.4 |
| 18 | No job concurrency limit | 2.2 |
| 19 | Jobs don't survive restart | 2.6 |
| 20 | Migration applied manually | 0.8 (note), 2.3 |
| 21 | nautilus `*::from` panic on bad input | 1.2 |
| 22 | List endpoint returns everything | 2.5 |
| 23 | End-to-end path never executed | 4.1 |
| 24 | manager.rs / sim.rs untested | 4.2 |
| 25 | Backtesting was removed from scope (2026-06-10) | 0.8 |
| 26 | Missing specs/ADRs | 0.8, 4.5 |
| 27 | Three axios clients | 5.1 |
| 28 | Strategy picker may be empty | 5.2 |
| 29 | Single ~1 MB JS chunk | 5.3 |

---

## Progress Log

Update this table and the per-phase completion headers as tasks land.

| Date | Phase | Task | Note |
|------|-------|------|------|
| — | — | — | not started |
