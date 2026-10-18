# Phase 2 — Evidence-standard extensions

**Completion: 38% (3 / 8 tasks), plus 2 `PARTIAL`, 2 `BLOCKED:UPSTREAM`, 1 `NOT STARTED`.** Re-counted 2026-10-05 (track VERIFY2) from the per-task markers; it read 0 / 8.

**Requirement IDs:** EV-02, EV-03, EV-06, EV-07, T9, T12, T15, T18
**Spec:** [BS-007 11_EVALUATION_STANDARD](../../../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/11_EVALUATION_STANDARD.MD) §6, §7, §9
**Depends on:** Phase 1.
**Migration range:** none.

BS-007 11 requires more than tools: the G3 thresholds, a dossier, and the QC rules by topic T1 to T20. Set S took the honesty arithmetic and the leakage battery. What is left is
the G3 rules that *consume* the new tools, and the topic rules that need data the platform does not yet hold.

---

### Q-2.1 G3 applies the theme posterior — M
`DONE` 2026-10-05 (track R-MQ) - `gates/theme_posterior.rs` applied inside Gate 8 (ADR-Q-11); the golden test pins it to the posterior_alpha template. 104 backtest gate tests passed before the stack tests and later modules were added; those are unrun. Missing: the funnel's `GateRunner::gate3` has no posterior clause.

EV-07: a candidate passes G3 only if the theme-posterior alpha is positive with a credible interval excluding zero (consumes `posterior_alpha`, Q-1.2), beside DSR >= 0.95 and PBO <= 0.5.

**Files:** `crates/backtest/src/gates/` (outside this track)
**Acceptance:** a candidate whose posterior interval includes zero fails G3 with that reason; DSR and PBO clauses are unchanged (a test of the old behaviour).
**Traces:** EV-07; BS-007 11 §6

### Q-2.2 G3 applies the family test to campaigns — M
`DONE` 2026-10-05 (track R-MQ) - closed by Gate 14 (ADR-Q-07); four tests in `gates/stack.rs` are written but unrun. Verified 2026-10-05: compiled and run green (backtest lib 563, mcp-server-lib 388 incl. the 16-per-namespace cap and 1,200-byte budget, templates_run 9, pytest 51).

A candidate that is part of a campaign must survive the family-wise test over the campaign's candidates (consumes `family_wise_test`).

**Files:** `crates/backtest/src/gates/`, `crates/api/src/gate_worker.rs` (outside this track)
**Acceptance:** a campaign candidate the family test does not reject passes that clause; one it rejects fails G3 naming the adjusted p-value; a solo experiment is unaffected.
**Traces:** EV-07

### Q-2.3 Specification count, exploration ledger and `post_hoc` reported (EV-03) — S
`PARTIAL` 2026-10-04 (track R-MQ) - `gates/dossier_clauses.rs::validate` refuses a dossier missing the specification count, ledger ref or post_hoc. The report validator in crates/api must call it. Verified 2026-10-05: compiled and run green (backtest lib 563, mcp-server-lib 388 incl. the 16-per-namespace cap and 1,200-byte budget, templates_run 9, pytest 51).

Verify what Set S Phases 3 and 8 took; report the three quantities in every dossier. Closed by a test that fails when a dossier omits one.

**Files:** `crates/api` dossier assembly (outside this track)
**Acceptance:** a dossier without the specification count is refused by the report validator.
**Traces:** EV-03; BS-007 11 §5

### Q-2.4 Covariance default is the nonlinear shrinkage (T9) — S
`DONE` 2026-10-04 (track R-MQ) - `portfolio/allocate_book.py` defaults to the Ledoit-Wolf shrunk covariance, names it, sample only by name (linear shrinkage, not nonlinear: ADR-Q-10). pytest 28 pass, mutation-checked.

`shrunk_covariance` exists as a template; make it the default for multi-asset sizing and state it in the report.

**Files:** `crates/api`, `apps/skill-runner/templates/portfolio/shrunk_covariance.py`
**Acceptance:** the multi-asset path names the estimator it used; the sample covariance is not reachable by default.
**Traces:** T9

### Q-2.5 Dossiers carry an expected live haircut (T18) — S
`PARTIAL` 2026-10-04 (track R-MQ) - `dossier_clauses::expected_haircut` (58 vs 26, cited, agrees with expected_decay.py). Dossier assembly in crates/api must call it. Verified 2026-10-05: compiled and run green (backtest lib 563, mcp-server-lib 388 incl. the 16-per-namespace cap and 1,200-byte budget, templates_run 9, pytest 51).

Consumes `expected_decay` (Q-1.5).

**Files:** `crates/api` dossier assembly (outside this track)
**Acceptance:** a dossier states a haircut and its source; a literature-sourced idea shows the larger one.
**Traces:** T18; EV-06

### Q-2.6 Order-flow features before price-only features (T12) — L
`BLOCKED:UPSTREAM`

Integrated multi-level OFI needs L2 history. **Blocked on Set P**: the platform holds no order-book history.

**Files:** `crates/features`
**Acceptance:** an intraday liquid-strategy experiment reports an OFI baseline.
**Traces:** T12

### Q-2.7 `basis_report` and never-gross-of-cost carry (T15) — M
`BLOCKED:UPSTREAM`

Needs a funding lane (Set S S-3.14 produces the read route; the lane's history is Set P).

**Files:** `apps/skill-runner/templates`
**Acceptance:** a carry strategy's report is net of funding and fees; a missing funding history refuses.
**Traces:** T15

### Q-2.8 Hypothesis registration is required by `create_experiment` (EV-02) — S
`NOT STARTED`

Migration 0127 (preregistration) exists; verify `create_experiment` refuses an experiment with no sealed hypothesis and add the missing test if it does not.

**Files:** `crates/mcp-server`, `crates/api` (outside this track)
**Acceptance:** an experiment created without a registered hypothesis is refused naming the missing registration.
**Traces:** EV-02
