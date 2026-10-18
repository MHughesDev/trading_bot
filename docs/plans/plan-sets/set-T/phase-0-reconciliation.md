# Phase 0 — Reconciliation and the first observation

**Completion: 50% (2 / 4 tasks), plus 1 `PARTIAL` and 1 blocked (re-counted 2026-10-04, track VERIFY; it read 1 / 4). T-0.4 blocked on engineering only — gate H-7 (venue credential) was withdrawn 2026-10-02 when the owner decided paper trading runs in the app ([ADR-0048](../../../adr/0048-paper-trading-runs-in-the-app.md)).** T-0.1 and T-0.3 were downgraded from `DONE` on 2026-09-21. T-0.1's wire is not in place — the observer is constructed only in tests and the method the file tells a track to call does not exist — and T-0.3's own note says its criterion is not met.

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
`PARTIAL` 2026-10-04 (track T-P) — **re-checked, unchanged: the production call sites exist and the remainder is a running deployment.** `ForwardRecorders::install` runs at boot (`main.rs`); `deploy` and `resume_deploy` are called from `apps/platform/src/live_deployments.rs` and the runner calls `on_submit` / `close_bar` / `insert_observation` (`live_runner.rs`, Set U's files). The period-evidence reader that consumes the rows is now tested over planted rows (T-3.1, `a_period_is_judged_on_the_forward_rows_inside_it`). No deployment exists, so `mlops.forward_observation` holds no real row; that step is `BLOCKED:HUMAN` (`docs/operations/forward-clock-runbook.md`).
`PARTIAL` 2026-09-27 — **the wire in the title is in place now; what remains needs
a deployment.** `apps/platform/src/main.rs` calls
`reconciliation::recorder::ForwardRecorders::install()` before anything can produce
a fill, so `register_observer` has a production call site and this process is on
the fill stream. Two guards hold it there:
`reconciliation_is_installed_before_anything_can_produce_a_fill` asserts the call
precedes `PaperTradingEngine::realistic()`, `ExecutionEngine::new(` and
`axum::serve(` — proved load-bearing by moving the block after the engine and
watching it fail — and a second asserts the call is an unconditional binding whose
verdict is logged.

> **2026-10-03, track MONITOR — still `PARTIAL`; one production defect fixed, the
> remaining clause needs a deployment wire in Set U's territory.** Checked against
> the acceptance: (1) *a paper fill produces a `forward_observation` row within one
> bar* — not met in production, because nothing calls `ForwardRecorders::deploy`
> (gap 1 under T-0.4). (2) *element-wise comparable* and (3) *USDT not summed as USD*
> — met, by the tests listed below. **Fixed:** every function in
> `crates/reconciliation/src/store.rs` ran on the bare pool. The tables are
> `FORCE ROW LEVEL SECURITY` on `app.tenant_id`; tests connect as a superuser, who
> bypasses RLS, so they passed — and in the running platform, as `app_role` with no
> tenant set, `insert_observation` would have failed its `WITH CHECK` on the first
> real fill and `load_active_halts` would have read "no strategy is halted". All five
> now run in a transaction that sets `app.tenant_id` (`store::tenant_tx`).

**Ordering is part of the fix, not decoration.** The registry records
`execution::fills`' published count when it attaches and the fill stream has no
replay, so every fill before that watermark is one the forward record can never be
given and the verdict becomes `AttachedLate` for the life of the process.

**`with_fill_observer` was not created, and this file's instruction to call it is
stale.** `crates/execution/src/lib.rs:82` records that `ExecutionEngine` used to
carry an `Option<Arc<dyn FillObserver>>` field nothing used, and that it was
replaced by the global registry in `execution::fills`. The registry is the
mechanism; a per-engine field would be a second way to attach, and the registry's
`install()` is idempotent precisely because two subscriptions would deliver every
fill twice and double-count the number that is hardest to acquire.

**What keeps it `PARTIAL`:** no deployment exists, so no `ForwardRecorder` is
constructed and `mlops.forward_observation` still holds 0 rows. `ForwardRecorder` is
per-deployment — it needs a `DeploymentSpec` and an opening balance — and those
arrive when a strategy is actually deployed, which is gate H-1. The registry's
verdict is therefore `NoDeployment`, and **that is a different fact from
`NotInstalled`**: the first says nothing has been deployed, the second says this
process is not on the fill stream at all, and only the second makes an empty
`mlops.forward_observation` unreadable as evidence. The verdict is logged at startup
so an operator can tell which one they are in.

The "same shape as the ledger's backtest series" half below is unverified against
real fills for the same reason.

**The second blocker on T-0.4 that no plan file recorded is now discharged.** A
credential alone would still not have produced a `forward_observation` row, and
that would have surfaced only after H-1 was spent. It would now.

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
`DONE` 2026-10-03 (track UI2) — **the one unmet clause, "the halt is visible in the UI with its reason", is met.** `frontend/src/pages/ReconciliationPage.tsx` (route line requested from the orchestrator: `/reconciliation`) lists `GET /api/reconciliation/halts` — strategy, kind, raised-at, the reason, and the standing (`in force`, `in force, not persisted`, `persisted, not in force`) — and clears one through `POST /api/reconciliation/halts/{strategy_id}/clear` behind a confirm that cannot be dismissed with Esc or the scrim, is dead until the strategy id is typed **character for character** (no case folding or trimming) and a note is given, sends exactly `{confirm_strategy, note}`, prints the server's refusal and re-reads rather than dropping the row locally. There is no bulk clear and no control that continues on divergence. A halt list that did not load is "unknown, not empty"; a 404 is "not served by this build". The routes and `spawn_rehydration` are registered (`routes/mod.rs`, `main.rs`). Evidence: `pages/__tests__/the-halt-is-visible-and-clears-only-on-purpose.test.tsx` (8) and the reconciliation block of `lib/ops/__tests__/the-pages-state-what-they-do-not-know.test.ts`, plus the existing `divergence_halts_before_next_order` and `no_divergence_bypass` tests for the order-gate clauses. **Caveat that is not this task's:** no production code *raises* a divergence halt until T-0.4 deploys a recorder, so the page reads an empty list on a real platform today, and says that the empty list is the route's answer and not a measurement of health.

Earlier note — **downgraded from `DONE` on 2026-09-21 — by its own note, which
already said so.** The note reads verbatim: *"**NOT DONE — 'the halt is visible
in the UI with its reason'.** No API route and no React surface were built …
**This criterion is not met.**"* A task carrying an explicitly unmet acceptance
criterion was marked `DONE`. Confirmed against the tree: no halt route in
`routes/mod.rs`, no divergence surface in `frontend/src/pages/`.

> **2026-10-03, track MONITOR — still `PARTIAL`, and what is left is two lines and
> a page outside this track.** Built: `reconciliation::halts` — `active` (the gate's
> in-memory registry plus durable halts it does not yet hold, which it rehydrates),
> `clear` (a `HumanActor` and a note → `StrategyHaltRegistry::clear` →
> `mlops.divergence_halt_clearance`; a failed write leaves the durable halt, which
> the next rehydration re-imposes) and `spawn_rehydration` (every 30 s, every
> tenant, so a restart does not forget a halt). Served at
> `GET /api/reconciliation/halts` (reason, raised-at, `in_force`, `persisted`) and
> `POST /api/reconciliation/halts/{strategy_id}/clear` (typed strategy id + note,
> through `require_control` — the same scope agent and service tokens never carry),
> both in `crates/api/src/routes/monitor.rs`. The store's RLS defect (see T-0.1)
> would have made the durable half silently empty. **Missing, outside this track:**
> the two route lines in `routes/mod.rs`, the `spawn_rehydration` line in
> `apps/platform/src/main.rs` (both requested from the orchestrator), and the React
> surface that renders the list — so *"the halt is visible in the UI with its
> reason"* is still not met. Also: no production code *raises* a divergence halt
> yet, because no `ForwardRecorder` is deployed (T-0.4 gap 1).

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
> **2026-10-04, track F-DATA-T — the operations runbook is written: `docs/operations/forward-clock-runbook.md`.** The exact steps to deploy one hand-authored strategy and start the clock: preconditions to check (platform on 7080, migrations through 0190, monitor lease live, ceilings and `ramp_policy` present, bars flowing, a registered trial), choosing the strategy, the proposal and the human decision, `POST /api/deployments` with the `forward` block (what makes it measurable), the day-1 confirmation queries, the restart caveat (fills not persisted for resume yet), and the arming SQL. **The day-1 action is `BLOCKED:HUMAN`**: it needs the platform running with a deployment, the monitor lease live, and a person to decide the proposal. Nothing was deployed by this track.
> **2026-10-04 update.** Engineering items 1 and 2 are done and committed by the Set U session (76f37758, 5fe30259): `POST /api/deployments` takes an optional `forward` block, writes `mlops.paper_deployment`, and the live runner attaches a `ForwardRecorder`, steers a shadow run (`backtest::shadow::Shadow`, the same compiled plan on the SimulatedVenue) for the expected series, and stores each observation. **Item 3 (resume) is pending**: `ForwardRecorders::resume_deploy` and `reanchor` exist and are tested in `crates/reconciliation`, but nothing persists forward fills (`ResumeFill`), so every runner start currently opens a fresh recorder and cumulative divergence state resets. The gap itself reads as missing bars. The next step is a fill-persistence migration (0186). The operations half (deploy one strategy, record its date and thresholds, run it for a week) needs the platform running with a real deployment.

`BLOCKED:UPSTREAM` — **on engineering only, since 2026-10-02.** The human half of
this block (gate H-7, Alpaca paper credentials) is gone: the owner decided paper
trading runs on the platform's own paper venue
([ADR-0048](../../../adr/0048-paper-trading-runs-in-the-app.md)), and
`PaperVenue::Platform` / migration 0141 make `platform_paper` the only venue a
deployment may name. What remains is the three engineering gaps below. Found
2026-09-27, still true: deploying today would produce zero rows.

> **2026-10-03, track MONITOR — what remains for T-0.4, precisely (not attempted:
> it needs a live multi-day run).** Engineering, in order:
> 1. **Deploy a forward recorder with every v3 deployment.** `POST /api/deployments`
>    (Set U, `crates/api/src/routes/deployments.rs`) must, for a deployment that is
>    forward evidence, write the `mlops.paper_deployment` row (declared thresholds,
>    kill criteria, `trial_id`, `numeraire`, `bar_seconds`) and put its id in
>    `strategy_deployment.forward_deployment_id`, then call
>    `ForwardRecorders::installed().deploy(spec, starting_capital, gate.halt_registry())`.
>    (0141 already moved `paper_deployment.venue` to `'platform_paper'`.)
> 2. **Call the recorder from the runner.** `apps/platform/src/live_runner.rs` (Set U)
>    must call `recorder.on_submit(&intent, decision_price)` when it submits and
>    `recorder.close_bar(..)` on each bar close, and persist the result with
>    `reconciliation::store::insert_observation(pool, tenant, &obs)` — now
>    tenant-scoped, so it works as `app_role`.
> 3. **Rehydrate across a restart — built 2026-10-03, call it from the runner.**
>    `ForwardRecorders::resume_deploy(spec, starting_capital, open_bar, &observations,
>    &fills, last_mark, halts)` (and `ForwardRecorder::resume`,
>    `RealisedSeriesBuilder::resume`, `DivergenceMonitor::resume`). `observations` are
>    the deployment's stored `mlops.forward_observation` rows in bar order
>    (`reconciliation::fleet_job::load_window` or `store::load_observations`); `fills`
>    are `Vec<ResumeFill>` from `fills ⋈ orders` for the deployment with the decision
>    price each was submitted at; `last_mark` is the mark of the last closed bar (the
>    opening mark if none). Guarantees, each held by a test: the resumed series is
>    **bit-identical** to one built without the restart (every restart point 0..=5,
>    including `Vec<BarClose>` for later bars); an out-of-order, duplicate,
>    off-grid or non-finite stored bar is refused; and a balance that does not match
>    the stored chain (wrong mark, missing fill) is refused as `ResumeInconsistent`,
>    never silently re-based. The monitor's running totals are *folded*, not
>    re-judged, so a restart cannot re-raise a halt a human cleared. **Caller
>    obligations:** (a) call `recorder.on_submit(&intent, decision_price)` again for
>    every order still open — pending intents are not stored, and a fill for an
>    unregistered intent is not attributed; (b) a fill belongs to the bar whose close
>    is the first `>=` its `filled_at`, so journal fills with their fill time, not
>    the time they were read; (c) model rejects counted in the still-open bar restart
>    at zero.
>
>    **Restart after a re-anchor (2026-10-04).** After a restart that held a
>    position, recording pauses until live is flat; the runner then calls
>    `recorder.reanchor(bar, mark)` (no return for the skipped bars or the anchor
>    bar; `recorder.gaps()` lists the pauses; `RealisedSeries::align_over_gaps`
>    reports the missing bars). A *second* restart must still verify what it
>    resumes: `resume` is unchanged and infers the last anchor from the first
>    stored bar after a gap (`anchor = bar − bar_seconds`), then checks that the
>    stored returns **after that anchor** compound, from the anchor's equity, to
>    the equity the fills and `last_mark` rebuild. If live was **flat at the
>    anchor** (the designed case) the anchor's equity is just the cash the fills
>    up to the anchor leave, so no new input is needed. If the anchor held a
>    position, plain `resume` refuses with `ResumeAnchorUnverifiable` rather than
>    skip the check, and the caller uses the superset:
>    `RealisedSeriesBuilder::resume_anchored(spec, opening_balance, open_bar,
>    returns, fills, last_mark, anchor_mark: Option<Price>)` (`resume` is
>    `resume_anchored(.., None)`); `anchor_mark` is the mark passed to `reanchor`,
>    so **persist it with the anchor** if reanchoring at a non-flat bar is ever
>    allowed. Earlier segments are not re-verified (their equity changes are not
>    in the stored returns; each was checked when it was resumed). The
>    `ForwardRecorder`/`ForwardRecorders` wrappers still call plain `resume`.
>
>    **Shadow-run expectations (Set U's plan: expected return per live bar from a
>    shadow run of the same compiled plan on the SimulatedVenue over the identical
>    bars).** `ExpectedBar` fits — `expected_return` is the shadow equity's return
>    over the bar, `expected_turnover` its traded notional, `modelled_slippage_bps`
>    the notional-weighted slippage the simulator charged the shadow fills,
>    `signal_matched` whether the shadow decided the orders the live run submitted.
>    One API change was needed: `ExpectedBar` carries no bar, so an expectation for
>    bar *n-1* would pair silently with bar *n*'s realised return. Build it only with
>    `ExpectedBar::from_shadow(bar, &ShadowBar { bar, .. })`, which refuses a
>    different bar (`ShadowBarMismatch`) and a non-finite number (`ShadowNotFinite`).
>    `close_bar(bar, mark, expected, source)` is otherwise unchanged. Shadow and live
>    must use the same `bar_seconds` grid and the same close timestamps, or
>    `RealisedSeries::align` will refuse the comparison later.
> 4. **Spawn `reconciliation::halts::spawn_rehydration`** from `main.rs` (done by the
>    orchestrator).
> Operations, after those: pick one hand-authored strategy, record its thresholds and
> deployment date here, let it run a week, and record the first realised-vs-expected
> comparison. The fleet job (T-2.2) and Gate 15 start reading the moment rows exist.

T-0.1's fill-stream wire is in place, so the process is observed. But three things
between a deployment and a `forward_observation` row do not exist, and none was
recorded in any plan file:

1. **`deploy`, `on_submit`, `open`, `close_bar` and `store::insert_observation`
   have zero production callers.** `install()` makes the process *attached*; it does
   not make it *recording*. Those call sites are T-1.1/T-1.3 in `crates/api` and
   `crates/domain`.
2. **`mlops.paper_deployment` has no writer and no reader anywhere in Rust** —
   `crates/api/src/routes/monitor.rs:160` already says so in a comment. Nothing
   would record that a deployment exists.
3. **`paper_deployment` has no opening-balance column, and `RealisedSeriesBuilder`
   has no resume-from-stored-returns constructor.** So a restart mid-deployment
   cannot reconstruct the equity chain from its original base — and rehydrating
   from the *current* balance would silently make the realised series
   non-comparable with the backtest's, which is the one failure this whole phase
   exists to prevent. Two series that are nearly comparable produce a divergence
   number that means nothing. Needs a migration.

Recorded here rather than discovered after the gate is spent, which is what
happened to T-0.1's blocker.

**2026-10-02, Set U Phase 4:** a deployment record now exists —
`strategy_deployment` (migration 0142) is written by `POST /api/deployments` and carries
`starting_capital`, which answers gap 3's missing opening balance, and a
`forward_deployment_id` column for `paper_deployment` to attach to. v3 deployments run on
paper through the one strategy engine with a decision journal and checkpoints. Gaps 1 and 2
stand as written: nothing writes or reads `mlops.paper_deployment`, and `ForwardRecorder` has
no production caller.

The ops task, and the reason this phase is in Stage 0. Take a simple, hand-authored,
already-validated strategy, run it through the existing Experiment path, deploy it to the
platform's paper venue (ADR-0048), and let it run.

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

**Reversed by the owner 2026-10-02 ([ADR-0048](../../../adr/0048-paper-trading-runs-in-the-app.md)).**
The finding below was right that in-app forward evidence cannot test fill realism — after
ADR-0044 paper and backtest share one fill model by construction. ADR-0048 records what
it does measure (the strategy on unseen data, the pipeline in observed time, replay
parity) and moves the fill-realism question to a separate fill-model check on the first
live trades. Gate 15 verdicts on `platform_paper` are labelled as resting on modelled
fills. The original finding, kept for the record:

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
`FillResult::Applied` — never on a duplicate.

**Corrected 2026-09-27: do not attach with `ExecutionEngine::with_fill_observer`.**
That method does not exist and must not be recreated.
`crates/execution/src/lib.rs:79-91` records that the
`Option<Arc<dyn FillObserver>>` field was **removed rather than completed**,
because a per-engine observer makes "is this platform recording?" a question with
one answer per engine instance — and the hot path builds its own engine. ADR-I-07
reverses it. The mechanism is the process-wide registry:

```rust
// Startup, before the first order can be submitted. Idempotent; registers with
// execution::fills before it returns. Already wired in apps/platform/src/main.rs.
let forward_recorders = reconciliation::ForwardRecorders::install();

// Assert the stream is observed. `execution::fills::require_recording()`, NOT
// `ForwardRecorders::require_recording()` — the latter answers Err(NoDeployment)
// until a strategy is deployed and would refuse every boot.
execution::fills::require_recording().expect("the fill stream must be observed");

// Per deployment, at H-1. Builds AND attaches in one step; there is no way to do
// either separately, which is what makes a deployed recorder an attached one.
let recorder = forward_recorders.deploy(spec, opening_balance, halts)?;
```

A recorder built with `ForwardRecorder::new` is **not** on the fill stream and
cannot be put there — it does not implement `FillObserver`, so
`register_observer` will not compile for it. That is deliberate and asserted by
`a_recorder_built_outside_the_registry_never_sees_a_fill`.

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
