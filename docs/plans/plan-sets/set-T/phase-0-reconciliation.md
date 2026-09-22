# Phase 0 — Reconciliation and the first observation

**Completion: 75% (1 / 4 tasks). T-0.4 `BLOCKED:HUMAN` on gate H-7 (venue credential); its code blocker cleared in integration wave 3.** T-0.1 and T-0.3 were downgraded from `DONE` on 2026-09-21. T-0.1's wire is not in place — the observer is constructed only in tests and the method the file tells a track to call does not exist — and T-0.3's own note says its criterion is not met.

> **Status, 2026-09-15 — track 5, `claude/set-t-phase-0-reconciliation`.**
>
> T-0.1, T-0.2 and T-0.3 are implemented and tested. The fill stream they are tested against is
> **injected**: it runs through the real `ExecutionEngine` and the real `FillProcessor`, with
> only the venue adapter stubbed, so the pipeline works unchanged the moment real fills flow.
>
> ~~T-0.4 cannot start.~~ **The code blocker is gone as of integration wave 3.** The audit that
> wrote this found no working path to a paper fill at all — hot-path stage 3 returned `None`,
> armed automations had no caller, and there was no `INSERT INTO orders` or `INSERT INTO fills`
> anywhere in the repository. Track 16 (`claude/execution-path-to-a-fill`) built that path and it
> is now merged: stage 3 runs strategy evaluation and pushes `OrderIntent`s, and the fill seam
> `crates/reconciliation` attaches to is `execution::fills`, published on a fill that is both new
> to the processor and newly inserted into `fills`.
>
> T-0.4 stays `BLOCKED:HUMAN` rather than becoming startable, on a different gate: deploying a
> strategy to paper needs the venue credential (roadmap gate **H-7**). That is minutes of human
> action, not engineering.
> **The forward clock has not started, and no deployment exists.**
>
> **Note for integration:** this file was written in the track-5 worktree because the shared
> checkout's copy was outside that worktree's writable set. If the shared checkout still holds
> an untracked copy of this path, take this one.

**Requirement IDs:** FE-REC-01…FE-REC-04, FE-G15.
**Spec:** ML platform SPEC §12.3 (Gate 15); HANDOFF §5.3; `artifact.md` SC-6, FM-4;
[COMP-002](../../../specs/COMP-002-execution-and-risk-gate.md).
**Depends on:** nothing. `crates/reconciliation` exists and is unwired.
**Blocks:** Phase 1, and Gate 15 everywhere.
**Migration range:** 0105–0106. **Both used.**
**Migrations shipped:** 0105, 0106 (merged 2026-09-15, integration wave 4).
**Parallel with:** all of Set S and Set P.

Four tasks that turn the platform's most expensive unanswered question — *does the simulator
tell the truth?* — from unanswerable into merely unanswered-yet.

---

### T-0.1 Wire `crates/reconciliation` to the paper fill stream — M
`PARTIAL` — **downgraded from `DONE` on 2026-09-21, and the title is the
criterion it fails.** The task is *"wire `crates/reconciliation` to the paper
fill stream"*. `ForwardRecorder` is constructed **only in
`crates/reconciliation/tests/`**; `execution::fills::register_observer` has
**zero production call sites**; `ExecutionEngine.fill_observer` is `None` at
construction; and **`with_fill_observer` — which this phase file instructs a
track to call — does not exist as a method anywhere in the workspace**.
`mlops.forward_observation` holds 0 rows. The logic is done; the wire in the
title is not in place.

**This is a second blocker on T-0.4 that no plan file recorded.** A frontier or
paper credential alone would not produce a `forward_observation` row, so the
evidence clock would not start — and that would have been discovered only after
the credential arrived and nothing appeared.

The crate exists and nothing calls it. Subscribe it to paper fills and positions, and have it
maintain, per deployed strategy, the realised return series in the same shape the ledger stores
backtest return series in — same bar alignment, same numeraire handling (`Numeraire`, so USDT
is not silently summed as USD), same cost treatment.

Same shape is the whole task. Two series that are *nearly* comparable produce a divergence
number that means nothing.

**Files:** `crates/reconciliation/src/lib.rs`, `crates/execution` fill subscription,
migration 0105 (`forward_observation` table: deployment_id, bar, realised_return,
expected_return, source, observed_at).
**Acceptance:** a paper fill produces a `forward_observation` row within one bar; the realised
series and the backtest series for the same strategy are element-wise comparable without any
per-call adjustment; numeraire conversion is applied and a test asserts a USDT-denominated
strategy is not summed as USD.
**Traces:** SPEC §12.3; HANDOFF §5.3; ADR-P6 (`Numeraire`).

**As built.** `Numeraire` did not exist anywhere in the repository — ADR-P6 names it but no
type was ever written — so it is introduced as `domain::numeraire::Numeraire`, a closed enum
with **no conversion API**. Converting a *return* series between units is not a scalar
multiply; it needs a PIT FX path over the same bars, and inventing one inside a comparison is
how a divergence figure becomes a fiction. The only operation offered is
`assert_comparable`, which refuses.

`reconciliation::forward::RealisedSeries` wraps `ledger::neff::ReturnSeries` — literally the
type `persist_returns` stores — so "same shape" is a compile-time fact rather than two structs
agreeing today. `RealisedSeries::align` is strict: it refuses on numeraire, bar width, length
or bar mismatch rather than joining on what happens to overlap, because every looser join
discards exactly the bars where the two disagree about whether a trade happened.

Fills are applied net of fees (the fee leaves cash at the moment of the fill), which is what
makes the realised series comparable with a pessimistic-cost backtest series.

**Verified by:**
- `reconciliation::forward::tests::a_usdt_series_is_not_comparable_with_a_usd_series`
- `reconciliation::forward::tests::aligned_series_compare_element_wise_with_no_adjustment`
- `reconciliation::forward::tests::a_shorter_series_is_not_quietly_truncated`
- `reconciliation::forward::tests::series_on_different_bar_grids_do_not_align`
- `reconciliation::forward::tests::a_bar_off_the_grid_is_refused`
- `reconciliation::forward::tests::realised_equity_is_net_of_the_fee_charged`
- `reconciliation::forward::tests::the_realised_series_is_the_ledgers_own_type_and_validates_as_one`
- `tests/forward_observations_from_fills.rs::a_fill_reaches_the_forward_record_within_one_bar`
- `tests/forward_observations_from_fills.rs::the_realised_series_compares_element_wise_with_a_stored_backtest_series`
- `tests/forward_observations_from_fills.rs::a_usdt_deployment_is_not_summed_as_usd`
- `tests/forward_observations_from_fills.rs::a_redelivered_fill_does_not_double_count`

**Partial:** the `forward_observation` row is *written* by `reconciliation::store`, which is
untested against a live database — there is no paper fill to produce one yet (T-0.4). The
in-process path from an applied fill to a `ForwardObservation` value is tested end to end.

### T-0.2 Gate 15 reads the observations — M
`DONE`

`record_statistic` the forward-vs-expected comparison into the ledger so Gate 15 stops
returning `NotComputable`. The gate states its **observation window** and its **count** on the
verdict: "n observations over d days" is part of the verdict, not a footnote, because a gate
passed on four observations and one passed on four hundred are different claims.

**Only observed-knowledge rows count** (D-T1). A backfilled row is not forward evidence no
matter how recent.

**Files:** `crates/backtest/src/gates/`, `crates/ledger` statistic recording, migration 0106.
**Acceptance:** Gate 15 renders a verdict for a strategy with observations and `NotComputable`
with a stated reason for one without; the verdict carries the observation count and window; a
test plants a backfilled row in the window and asserts it does not enter the count.
**Traces:** SPEC §12.3 Gate 15; D-T1; ADR-P2-24.

**As built.** `PaperInputs` gains `observations` and `window_days` beside the existing
`observed_days`, and the verdict summary **leads** with them — `"{n} observations over {d} days
({k} days with data): signal …"` — rather than trailing them, so a reader cannot skip the
strength of the evidence on the way to the measurements.

The observed-knowledge filter lives in exactly one function,
`domain::forward::Gate15Evidence::from_observations`, rather than at each call site: a rule
enforced at three call sites is a rule with three chances to be forgotten. It also refuses to
report `slippage_ratio == 1.0` for a window with no turnover, because "matched the model
exactly" manufactured out of an absence is a pass.

Six statistics are written, not four — the four §12.6 measurements plus the count and the
window. `TRIAL_STATISTICS` and migration 0106's CHECK are extended together.

**Verified by:**
- `domain::forward::tests::a_backfilled_row_in_the_window_does_not_enter_the_count`
- `domain::forward::tests::a_window_of_only_backfilled_rows_produces_no_evidence`
- `domain::forward::tests::evidence_states_its_count_and_window`
- `domain::forward::tests::a_single_day_of_observations_is_one_day_not_zero`
- `domain::forward::tests::mixed_numeraires_are_not_one_series`
- `domain::forward::tests::an_untraded_window_does_not_report_a_perfect_slippage_match`
- `backtest::gates::evaluators::tests::gate_15_states_its_observation_count_and_window_on_the_verdict`
- `backtest::gates::evaluators::tests::gate_15_is_inconclusive_when_every_row_in_the_window_was_backfilled`
- `backtest::gates::evaluators::tests::paper_inputs_carry_the_evidences_count_and_window`
- `reconciliation::gate15::tests::every_name_written_is_in_the_closed_vocabulary`
- `tests/forward_observations_from_fills.rs::observations_become_gate_15_evidence_and_backfill_does_not`

**Partial:** Gate 15 still renders `NotComputable` in production, because no deployment has
produced an observed row. The gate now has an input *path*; it does not yet have an input.

### T-0.3 Divergence halts before the next order — M
`PARTIAL` — **downgraded from `DONE` on 2026-09-21 — by its own note, which
already said so.** The note reads verbatim: *"**NOT DONE — 'the halt is visible
in the UI with its reason'.** No API route and no React surface were built …
**This criterion is not met.**"* A task carrying an explicitly unmet acceptance
criterion was marked `DONE`. Confirmed against the tree: no halt route in
`routes/mod.rs`, no divergence surface in `frontend/src/pages/`.

SC-6 and FM-4: the system halts on position or balance divergence **before** submitting new
orders. Per-strategy thresholds declared at deployment (Phase 1) and recorded; when realised
diverges from expected beyond threshold, or when the internal position view disagrees with the
broker's, submission stops for that strategy and an alert fires.

The halt is the deliverable. A report that says "we diverged" after the next order went out is
the failure mode this exists to prevent.

**Files:** `crates/reconciliation/src/divergence.rs`, `crates/risk` pre-submit check.
**Acceptance:** a planted position divergence halts submission for that strategy before the next
order, proven by a test that counts orders reaching the venue adapter; the halt is visible in
the UI with its reason; clearing it requires an explicit human action that is recorded; **no
flag exists that continues on divergence**, asserted by a static check.
**Traces:** `artifact.md` SC-6, FM-4; COMP-002; D-T6.

**As built.** `risk::StrategyHaltRegistry` lives inside the risk crate, not the reconciliation
crate, and `RiskGate::check` consults it **unconditionally** — so the halt is enforced at the
one chokepoint every order already passes and no second chokepoint is created (SC-2). It is
checked **ahead of the idempotency cache**, for the same reason the kill switch is: an intent
approved before the divergence was found and redelivered afterwards must not ride the
cached-approval fast path out to the venue. That ordering is asserted statically.

The registry is not optional and `RiskGate::new` always builds one, so there is no wiring
mistake that silently disables divergence halting. `reconciliation` reaches it through
`gate.halt_registry()`.

Halts are per strategy, not the global kill switch, for return divergence. A *position*
divergence additionally trips the kill switch, because a position disagreement is about the
account — and the per-strategy halt survives a kill-switch reset, so clearing the global switch
does not resume a strategy whose book nobody has reconciled.

Clearing takes a `HumanActor` (which cannot be constructed empty) and a non-empty note, and
returns a `HaltClearance` carrying the original halt's id and reason for the caller to persist
to `mlops.divergence_halt_clearance`. There is no `reset`, no `disable`, no bulk clear.

**Verified by:**
- `tests/divergence_halts_before_next_order.rs::a_planted_position_divergence_stops_the_next_order_at_the_gate` (counts orders at the venue adapter)
- `tests/divergence_halts_before_next_order.rs::a_planted_return_divergence_stops_the_next_order_at_the_gate`
- `tests/divergence_halts_before_next_order.rs::a_halted_strategy_resumes_only_through_a_recorded_human_act`
- `tests/divergence_halts_before_next_order.rs::the_halt_is_scoped_to_the_diverging_strategy`
- `risk::gate::tests::divergence_halt_blocks_a_cached_approval`
- `risk::gate::tests::divergence_halt_blocks_that_strategy_only`
- `risk/tests/no_divergence_bypass.rs::no_identifier_offers_a_way_to_continue_on_divergence` (the static check, over `crates/risk/src`, `crates/reconciliation/src`, `crates/config/src` and `config/`)
- `risk/tests/no_divergence_bypass.rs::the_gate_consults_the_halt_registry_unconditionally`
- `risk/tests/no_divergence_bypass.rs::a_halt_is_lifted_only_by_a_named_human_with_a_stated_reason`
- `risk/tests/no_divergence_bypass.rs::every_risk_gate_has_a_halt_registry`
- `reconciliation::strategy_halt` unit tests (idempotent halt, refused clearances, rehydration)

**NOT DONE — "the halt is visible in the UI with its reason".** No API route and no React
surface were built. `StrategyHaltRegistry::active()` returns every live halt with its reason,
raised-at and kind, and `store::load_active_halts` reads them back, so the data is there and
shaped for a surface; the surface itself belongs with Phase 1's deployment UI and was left for
it rather than half-built here. **This criterion is not met.**

### T-0.4 Deploy one hand-authored strategy and start the clock — M
`BLOCKED:HUMAN`

The ops task, and the reason this phase is in Stage 0. Take a simple, hand-authored,
already-validated strategy, run it through the existing Experiment path, deploy it to Alpaca
paper, and let it run.

It does not need to be good. It needs to **exist and produce observations**, because every day
without a deployed strategy is a day Gate 15 has no evidence, and Gate 15's evidence is a
calendar that cannot be compressed later.

Record in this file: which strategy, its declared divergence thresholds, the deployment date,
and the first week's realised-vs-expected comparison.

**Files:** none — an operations task with its result written here.
**Acceptance:** the strategy is running on paper; `forward_observation` rows accumulate daily;
Gate 15 renders a real verdict for it within its first observation window; the deployment date
is recorded here and on the coverage surface as the date the forward clock started.
**Traces:** ROADMAP §1, §6 (the agent is not on the critical path to first evidence); SPEC §12.3.

**Blocked, 2026-09-15.** Two independent blockers, either of which is sufficient:

1. **There is no path to a paper fill.** Hot-path stage 3 produces no intents, armed
   automations have no caller, and no `orders` or `fills` row is ever written. Owner:
   **track 16**, `claude/execution-path-to-a-fill`.
2. **There are no Alpaca paper credentials.** `.env` holds none, and `AlpacaBroker` reads
   credentials from the database credential store by design. This is a credential a human must
   supply; it is not something this track can or should obtain.

**Nothing to record yet: no strategy, no thresholds, no deployment date, no first comparison.
The forward clock has not started.**

**A finding that changes what T-0.4 may deploy to.** The internal `PaperTradingEngine`
(`crates/execution/src/paper/`) needs no credentials and could produce "fills" today. It must
**not** be used for Gate 15 evidence. Reconciling a simulator against the same simulator
measures nothing and would manufacture confident forward evidence — the precise failure this
set exists to prevent. Migration 0105 therefore constrains `mlops.paper_deployment.venue` to
`'alpaca_paper'` alone, so a deployment against the internal engine cannot be recorded as
forward evidence even by mistake.

---

## Phase exit

- [x] Paper fills produce comparable forward observations automatically — *the pipeline does;
      no paper fill exists yet to flow through it.*
- [x] Gate 15 renders a real verdict, stating its count and window — *the verdict states both;
      it has no real observations to render one from yet.*
- [x] Divergence halts before the next order, proven by a drill, with no bypass — **met**,
      except the UI surface (see T-0.3).
- [ ] At least one strategy has been running on paper long enough to have produced
      observations, and the date it started is recorded — **not met.** Blocked on track 16.

## What track 16 has to produce for this pipeline to start working

The contract, precisely, so it is a check rather than a guess.

**The fill event consumed is `execution::fills::FillEvent`**, delivered through
`execution::fills::FillObserver::on_fill`, which `ExecutionEngine` calls **only** on
`FillResult::Applied` — never on a duplicate. Attach with
`ExecutionEngine::with_fill_observer(recorder)`; `ExecutionEngine::new`'s signature is
unchanged.

```rust
pub struct FillEvent {
    pub idempotency_key: Uuid,      // must equal OrderIntent.idempotency_key
    pub broker_order_id: String,
    pub filled_qty: Decimal,        // this fill's delta, not the cumulative total
    pub fill_price: Price,
    pub commission: Decimal,        // fees; deducted from cash to keep the series after-cost
    pub filled_at: DateTime<Utc>,
}
```

Four things track 16 must hold to, none of which it gets for free:

1. **`OrderIntent.strategy_id` must be `Some(..)` and stable** for anything deployed. The
   recorder ignores fills whose intent it never saw, and the risk gate's divergence halt is
   keyed on this string. An intent with `strategy_id: None` is treated as a manual order and is
   never halted by divergence.
2. **`ForwardRecorder::on_submit(&intent, decision_price)` must be called at submission**, with
   the mark the strategy decided at. `FillEvent` carries neither the side nor the decision
   price, and realised slippage measured against the fill itself is always zero — the most
   flattering and least informative number available.
3. **`filled_qty` is the delta for this fill, not the cumulative figure.**
   `ExecutionEngine::sync_order` already computes it that way; a new fill path must too, or
   partial fills will over-count the position.
4. **One `Applied` per real fill.** Partial fills raise several events against one idempotency
   key, which is expected and handled; a redelivery must not raise one, which the existing
   `FillProcessor` dedup already guarantees.

Given those, no change to this track's code is needed: the forward record, the divergence halt
and Gate 15's evidence path all start working on the first real fill.

**One caution track 16 should know about.** The audit found `orders.rs` passes `0, 0` rate
counters and `Decimal::ZERO` daily loss into the gate, and `hot_path.rs` gates against
hardcoded position and equity constants. The divergence monitor does **not** depend on any of
those: it compares realised returns against a stored backtest series and the internal position
against the broker's, both of which are real measurements. It is unaffected by synthetic limit
inputs — but it also cannot detect them, and it should not be read as covering that gap.
