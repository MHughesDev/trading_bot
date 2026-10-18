# Phase 4 — Research-to-deploy reconciliation and the reference venue

**Completion: 0% (0 / 4 tasks), plus 2 `PARTIAL`, 1 `BLOCKED:HUMAN`, 1 `NOT STARTED`.**

**Requirement IDs:** ST-10
**Spec:** [FEAT-004](../../../specs/FEAT-004-strategy-representation-v2.md) §4, §7; [FEAT-005](../../../specs/FEAT-005-backtest-engine-integration.md) §9
**Depends on:** Phase 1 (M-1.2) and Set U (U-7).
**Migration range:** 0205.

A Layer 1 idea earns a Layer 2 version only by reconciling: both are run on the experiment's in-sample window with one policy and
costs, and the positions must agree to `recon.agreement_min` with P&L tracking error under `recon.te_max`, every disagreement inside a
declared approximation class (FEAT-004 §7). The lineage record is `strategy_revision` (migration 0058), not the proposed
`strategy_versions` table (FEAT-004 §4 amendment, Q-G4-01).

---

### M-4.1 The strategy_reconciliation record — M
`PARTIAL` 2026-10-04 — `migrations/0205_strategy_reconciliation.sql`, keyed to `strategy_revision` (ADR-M-06). Validated on a scratch Postgres 17 (5433): applies twice; the CHECK refuses a `passed` row that contradicts its own recorded thresholds, an unexplained disagreement on a pass, incomplete thresholds (a `COALESCE` was needed: a missing key made the CHECK evaluate NULL, which passes), the same layer on both sides and an unknown revision; UPDATE and DELETE refused; `app_role` can insert and read and cannot update. Test file `crates/backtest/tests/pg_layer_reconciliation.rs` (`#[ignore]`, scratch DB). **Missing:** the platform's `strategy_revision` (0058) was not applied in the scratch DB (a one-column stand-in was), and nothing writes a row.

Migration 0205: `strategy_reconciliation(id, layer1_revision, layer2_revision, experiment_id, agreement, pnl_tracking_error, explained_diffs jsonb, passed, created_at)`,
keyed to `strategy_revision` hashes, append-only, with `GRANT SELECT, INSERT ... TO app_role` in the same migration.

**Files:** `migrations/0205_strategy_reconciliation.sql` (new)
**Acceptance:** the table refuses UPDATE and DELETE; an `app_role` insert and select succeed (`SET LOCAL ROLE app_role` test); the same pair and experiment cannot be recorded twice with different verdicts.
**Traces:** ST-10; FEAT-004 §4, §7

### M-4.2 The reconciliation comparator — M
`PARTIAL` 2026-10-04 — `layer_reconciliation::{reconcile, ReconcileThresholds, ApproximationClass}`: agreement, tracking error and classified disagreements; `ReconcileThresholds::new` validates and there is no `Default`. Tests include `a_faithful_translation_passes`, `a_translation_off_by_one_bar_is_blocked_and_lists_where` (FEAT-004 S7), `an_unexplained_disagreement_fails_however_high_the_agreement`, `tracking_error_alone_can_fail_a_series_that_agrees_everywhere`. **Missing:** no caller; it compares position series and equity curves it is handed and does not run the two layers itself.

Pure: two position series on one clock, agreement fraction over decision times, P&L tracking error between two evaluations, and a list of
disagreements each classified into a declared approximation class or left unexplained.

**Files:** `crates/backtest/src/layer_reconciliation.rs` (new)
**Acceptance:** identical series pass; a series differing at 2% of decision times fails the 0.99 default and lists the timestamps; an unexplained disagreement fails even above the agreement threshold; thresholds are inputs, not constants.
**Traces:** ST-10; S7

### M-4.3 The market_simulator reference differential — L
`BLOCKED:HUMAN`

Same bars and same order stream through `SimulatedVenue` and the pinned `market_simulator`, with a stated tolerance on fills and equity.
**Human step:** the second repo must be checked out beside this one and its rev bumped deliberately (the workspace `Cargo.toml` pins
`f915b050`); a build that fetches git dependencies is a decision the owner makes.

**Files:** `crates/backtest/tests/reference_venue.rs` (new, `#[ignore]` until the repo is present)
**Acceptance:** the differential runs on a market, a limit and a bracket fixture and reports the maximum fill-price and equity difference; the tolerance is written beside the result.
**Traces:** ADR-0044; FEAT-005 §9

### M-4.4 A Layer 2 version of a Layer 1 idea cannot pass Gate 3 unreconciled — M
`PARTIAL` 2026-10-04 (track R-MQ) - `gates/reconciliation_gate.rs`, `GateRunner::with_reconciliation` (refuses Significance and the Vault), `apply_to_stack` (refuses gates 3 and up), and `api::layer1_reconciliation` (reader and writer of 0205). Missing: nothing in the funnel or `gate_worker` supplies the requirement yet. Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

The gate stack refuses Gate 3 and later for a Layer 2 revision whose recorded parent is a Layer 1 idea unless a passed reconciliation
for that pair exists.

**Files:** `crates/backtest/src/gates/`, `crates/api/src/gate_worker.rs` (outside this track)
**Acceptance:** a Layer 2 version with no reconciliation is refused at Gate 3 with the reason; a passed one proceeds; a failed one stays refused.
**Traces:** ST-10; FEAT-004 §7
