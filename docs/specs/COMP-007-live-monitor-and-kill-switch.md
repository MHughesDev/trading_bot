# COMP-007: Live Strategy Monitor and Kill Switch — an independent service that watches the book and can stop it

> **Status:** **Partially implemented** — Set S phase 7. Re-checked 2026-09-21.
>
> The kill switch, the control limits and the monitor's read surface are built and the scope that arms them is refused to every agent token. Running it in production is Set T phase 2.
>
> **The live count is in [`docs/plans/plan-sets/set-S/phase-7-monitor.md`](../plans/plan-sets/set-S/phase-7-monitor.md), and the programme total is in [`docs/plans/PLAN.md`](../plans/PLAN.md).** It is deliberately not restated here: a number written into a spec is a number nobody recomputes, which is how this line came to say "ready to build" about code that was already serving traffic. `crates/invariants/tests/the_plan_counts_what_the_plans_say.rs` holds those two files honest; nothing holds a hand-copied number here.
**Date:** 2026-09-14
**Author:** Claude (Fable 5.1)
**Extends:** COMP-002 (execution and risk gate) §3; `crates/risk/src/kill_switch.rs`
**Related:** AGENT-006 §5.7 (`portfolio.check_control_limits`, read-only), AGENT-010 (the agent may *query* this service and never *skip* it), AGENT-012 §11 (its UI), ADR-0034 §2 principle "the checking mechanism is independent of the thing it checks"

---

## 0. Why this spec exists

`crates/risk/src/kill_switch.rs` is an `AtomicBool` with `trip()` and `reset()`, persisted by "the caller". Nothing trips it automatically, nothing scopes it, nothing records why, and it lives in the same process as everything it would stop. The research brief's strongest operational finding (Theme 8, cited from NYIF and 3forge practice) is that a kill switch has to run in a different process from the strategy, or the failure that requires killing also disables the kill. The second finding is that P&L is a *lagging* signal: fill quality, signal frequency and holding-period drift flag a broken strategy earlier than its equity curve.

The agent specs deliberately keep this out of the agent's hands: `check_control_limits` is a read-only tool, and no envelope lets an agent reset a halt. This spec is the service those specs assume.

## 1. Principles

- **P1 — Separate process, separate failure domain.** The monitor is its own binary (`crates/monitor`, service `tbot-monitor`), with its own DB connection and its own heartbeat. If the API or the strategy runtime is wedged, the monitor still runs; if the monitor dies, the runtime fails *closed* (§4).
- **P2 — Limits are set before the drawdown.** Control limits are declared per strategy at promotion time from its backtest and paper statistics, stored, and versioned. Changing a limit during a breach requires the two-step pattern and is logged with the breach it overrode.
- **P3 — Inputs before P&L.** The monitor watches the upstream signals first: fill rate, slippage vs. model, signal frequency, holding period, order rate, cancel ratio, venue concentration, data staleness; P&L control charts are one family among them.
- **P4 — Reversible actions are automatic; irreversible ones are human.** Scale-down fires on its own (it is reversible); halt of a strategy fires on its own (positions are flattened only by policy, §5); a *global* halt fires on its own for platform-level faults; **reset** is always a typed human confirmation.
- **P5 — Every action is a record with a reason.** Trip, scale, halt, reset: who or what, which metric, which limit, the values, the strategy, the time. The record is the UI's timeline and the audit's evidence.

## 2. Scope

Watches: every strategy instance in `paper` or `live` environment, every venue connection, the data pipeline's freshness, and the platform's own health (API heartbeat, job service, DB). Acts on: strategy size, strategy run state, venue order flow, and the global order gate. Does not: place orders itself beyond flattening under policy, change limits, or reason about alpha (that is the agent's job through `attribute_divergence`).

## 3. Metrics and control limits

For each strategy instance, the monitor computes on a fixed cadence (default 60 s; 5 s for order-flow metrics):

| Family | Metric | Default limit source |
|---|---|---|
| Execution quality | fill rate; realised slippage vs. fill-model prediction (bps, p50/p95); latency p95; partial-fill share | shadow/paper baseline ± 3σ (EWMA) |
| Behaviour | signals per hour; holding period p50; orders per minute; cancel ratio; position count | backtest baseline ± 3σ; venue rate-limit ceiling (AGENT-006 `get_venue_constraints`) |
| Exposure | notional; leverage; concentration by venue/instrument; max position vs. risk overrides | risk gate limits (`crates/risk/src/limits.rs`) — the monitor re-checks them independently |
| P&L | rolling 30/60/90-day Sharpe vs. backtest; CUSUM on daily returns; drawdown vs. the backtest's 95th-percentile Monte Carlo drawdown (AGENT-006 `trade_reshuffle_bootstrap`); time under water | thresholds declared at promotion, defaults: Sharpe < 50% of backtest over 60 d → warning; CUSUM breach or drawdown > MC p95 → breach |
| Data | last bar age per instrument; gap count; impossible bars (from `probe_data_quality`) | staleness > 3 × timeframe → strategy warning; > 10 × → strategy halt |
| Venue | connection state; error rate; rate-limit headroom; reconciliation drift (`crates/reconciliation`) | any reconciliation mismatch → venue halt |
| Platform | API heartbeat; job service heartbeat; DB latency; monitor's own lag | missed 3 heartbeats → global halt |

Limits are stored in `monitor_limits(strategy_id, metric, version, lower, upper, method, declared_from, declared_at, declared_by)`. `declared_from` names the backtest/paper run the baseline came from. A strategy with no declared limits cannot be promoted to `live` (COMP-002 gate amendment).

## 4. States and actions

Per strategy: `nominal → warning → breach → scaled_down → halted`, plus `resumed` transitions. Per venue and global: `armed → halted`.

| Event | Automatic action | Human action available |
|---|---|---|
| warning | record; notify (AGENT-012 §13) | acknowledge; open diagnosis conversation (`attribute_divergence`) |
| breach (behaviour/execution/data) | **scale down** to 25% size for that strategy; record; notify | resume at full; halt |
| breach (P&L CUSUM or MC-drawdown) | **halt strategy** (no new orders); existing positions handled by the strategy's halt policy (`monitor::state::HaltPolicy`: hold / flatten / reduce — see the correction below) | resume (typed confirmation), flatten now |
| venue reconciliation mismatch or connection loss | **halt venue** order flow; positions held | resume after reconciliation passes |
| platform heartbeat loss, monitor lag > 2 cadences, DB unreachable | **global halt**: the runtime's order gate refuses all new orders | reset (typed confirmation, reason required) |

> **Correction, 2026-09-15 (Set S Phase 7 implementation).** §4 above and S-7.8
> cited `crates/domain/src/instrument.rs:198` for a `HaltPolicy` carrying `hold /
> flatten / reduce`. That enum exists but is a **different concept**: it is
> `Haltable | NonHaltable` and answers "can the *exchange* halt this instrument",
> which the risk gate already consults in `limits::check_halt`. Nothing anywhere
> has ever carried hold/flatten/reduce.
>
> Reusing the name for two questions would have been worse than the missing type:
> `check_halt` and the monitor would have matched on the same enum meaning
> opposite things. So the policy is implemented as `monitor::state::HaltPolicy`
> (`hold | flatten | reduce`), persisted per strategy on
> `monitor.strategy_state.halt_policy`, with `hold` the default. §10's open
> question — that the default for a new *live* crypto promotion should probably be
> `reduce` — is left open and is a promotion-time choice, not a schema default.
>
> Everything else in §4 stands unchanged.

### 4.1 Dwell time, and the monitor's own precision

A 3σ EWMA band on a noisy intraday metric will breach regularly by construction. A monitor
that cries wolf trains the operator to click through it, which is the same failure mode the
HITL research warns about for approvals — and it is worse here, because the thing being
ignored is the last line before money. Three mechanisms, none optional:

- **Dwell.** A metric must hold outside its limit for `min_dwell_cadences` (default 3)
  **consecutive** evaluations before the state changes. A single-cadence excursion records a
  `blip` and nothing else. Exempt: reconciliation mismatch, lease loss, and data staleness
  past the hard multiple — those are facts, not estimates, and act immediately.
- **Hysteresis on the way back.** Recovery needs `2 × min_dwell_cadences` inside the limit
  before an automatic scale-down is released, so a metric oscillating on the boundary cannot
  flap the book.
- **The monitor is scored like anything else here.** Every breach is later labelled — by the
  human who resumed, or automatically when a scaled-down strategy is restored to full with
  no intervening change — and the monitor's **precision per metric family** is published
  monthly beside the strategy scorecards. A family whose precision falls below 0.5 has its
  limits re-derived from data rather than left to erode trust. The platform measures its
  strategies; it does not get to exempt its own instruments from measurement.

**Fail closed.** The runtime's order path checks a lease the monitor renews every cadence (`monitor_lease` row with an expiry). No valid lease → orders are refused with `monitor_unavailable`. This is how "if the monitor dies the platform stops" is enforced without the monitor being in the process.

### 4.2 Drills: an untested kill switch is a decoration

Nobody will trip a global halt on a live book to see whether it works, so without a rehearsal
mechanism this system's most important path is the one path never exercised — and it will be
exercised for the first time on the day it matters, by someone under pressure.

**Drill mode** (`POST /api/monitor/drill`, human scope only) runs the full trip path against a
`shadow` scope: the monitor evaluates, the state machine transitions, events fire,
notifications send, the desk banner appears marked **DRILL**, and the order gate is asked —
and answers — without refusing anything real. It measures the three numbers that matter:
detection latency (breach to state change), propagation latency (state change to the runtime
refusing), and notification latency. A drill is recorded like any other trip, with
`scope: drill`.

- Drills run **automatically after every deploy** touching `crates/monitor`, `crates/risk` or
  `crates/execution`, and **monthly** regardless.
- A drill whose propagation latency exceeds one cadence **fails the deploy**. That is the
  entire assertion of §4's fail-closed design, and until a drill proves it, the design is a
  claim.
- Drill results are published beside the monitor's precision (§4.1).

### 4.3 Maintenance windows

A rolling deploy looks exactly like a platform heartbeat loss, and §4 says a heartbeat loss is
a global halt. Without a window every deploy halts trading, and every operator learns to
pre-emptively reset the switch — which is how a safety mechanism becomes a nuisance and then a
habit of ignoring it.

A **maintenance window** (opened by a human, or by the deploy pipeline with a service scope,
bounded and auto-expiring, default 15 minutes) suppresses *platform-health* trips only. It
does **not** suppress reconciliation mismatches, venue errors, data staleness or any
strategy-level breach — those are facts about the market and the book, and a deploy is no
reason to stop believing them. An open window is visible in the desk chrome exactly as a trip
is, because a window nobody can see is a hole nobody can audit, and it records who opened it
and why.

## 5. The kill switch, redone

[DO] Replace the in-memory flag with a scoped, persisted switch:

```
kill_switch(scope: global | venue:<id> | strategy:<id>, state: armed | halted,
            tripped_by: monitor | human:<user> | agent:<run>?  -- agents can never trip or reset; the column exists to prove it stays null,
            reason, metric?, value?, limit?, tripped_at, reset_by?, reset_reason?, reset_at?)
```

The runtime keeps an in-memory mirror refreshed on every lease renewal and on a NOTIFY; `is_active(scope)` stays an atomic read on the hot path. `reset` requires: the human's typed confirmation of the scope name, a reason, and — for global — that every venue reconciles clean first. Resets are rate-limited (one per five minutes per scope) so a flapping fault cannot be reset into.

## 6. Interfaces

- **Service** `tbot-monitor`: config (cadences, default limit methods), Postgres, NATS (consumes fills, orders, bars, heartbeats; publishes `monitor.events`), no dependency on the API process.
- **API** (served by the API process from the monitor's tables, read-mostly): `GET /api/monitor/strategies`, `GET /api/monitor/strategies/{id}/metrics?window=`, `GET /api/monitor/events`, `POST /api/monitor/strategies/{id}/{scale|halt|resume}`, `GET /api/monitor/kill-switch`, `POST /api/monitor/kill-switch/{scope}/{trip|reset}`, `PUT /api/monitor/limits/{strategy_id}` (versioned; two-step during breach). Human actions carry the user; the agent's service token has none of the POST scopes.
- **Agent tool** `portfolio.check_control_limits` (AGENT-006 §5.7): read-only over the same tables.
- **UI**: AGENT-012 §11 and the top-bar indicator and banner.
- **Events**: every state change is an `agent_events`-style row in `monitor_events` and a NATS message; AGENT-012 §13 notifications subscribe.

## 7. Build order

| Phase | Items |
|---|---|
| 1 | `crates/monitor` service skeleton with heartbeat, lease, and the scoped kill switch (§5); runtime order gate checks the lease and the switch; UI indicator and banner |
| 2 | data and venue families (§3) with venue/strategy halts; reconciliation hook |
| 3 | execution-quality and behaviour families; automatic scale-down; limits declared at promotion; COMP-002 gate amendment |
| 4 | P&L family (CUSUM, MC-drawdown, rolling Sharpe); monitor page (AGENT-012 §11); notifications |
| 5 | limit versioning with two-step change during breach; monthly review report of warnings vs. breaches vs. resets |

## 8. Acceptance

1. Killing the API process while a paper strategy runs: within one cadence the runtime refuses new orders with `monitor_unavailable`; restarting the API does not resume orders until the monitor's lease is valid again.
   **Verified by:** `risk::lease_fails_closed::a_restarted_monitor_does_not_resume_orders_until_its_lease_is_valid_again`, `monitor::fail_closed_drill::a_database_the_runtime_cannot_read_closes_the_gate`.
2. Killing the monitor: same outcome from the runtime's side, visible in the UI banner within one cadence.
   **Verified by:** `monitor::fail_closed_drill::killing_the_monitor_stops_order_flow_within_one_cadence` (a real lease in a real database, allowed to lapse), `risk::lease_fails_closed::a_gate_with_no_monitor_seen_refuses_every_order`.
3. An injected reconciliation mismatch halts the venue and only that venue; `resume` is refused until reconciliation passes.
   **Verified by:** `monitor::fail_closed_drill::an_injected_mismatch_halts_one_venue_and_blocks_resume_until_clean`, `risk::lease_fails_closed::a_venue_halt_stops_that_venue_and_leaves_the_others_trading`.
4. A strategy whose fill rate drops 3σ below its paper baseline is scaled to 25% automatically and the event names the metric, values and limit.
   **Verified by:** `monitor::evaluate::tests::a_sustained_execution_breach_scales_down_and_names_the_numbers`, `monitor::events::tests::a_breach_event_names_the_metric_the_value_and_the_limit`, `monitor::baseline::tests::a_seeded_baseline_measures_distance_in_sigmas`.
5. A CUSUM breach halts the strategy and applies its halt policy; the trip record is present before the first refused order.
   **Verified by:** `monitor::state::tests::a_pnl_breach_halts_and_applies_the_policy`, `monitor::state::tests::the_record_always_comes_before_the_halt`, `monitor::fail_closed_drill::the_trip_record_is_present_before_the_first_refused_order`, `monitor::baseline::tests::a_persistent_downward_shift_is_caught`.
6. No agent token can call any monitor POST; `tripped_by` is never `agent:*` in any test or migration.
   **Verified by:** `api::monitor_authority::nothing_in_the_agents_vocabulary_reaches_the_monitor`, `api::monitor_authority::the_database_refuses_to_grant_monitor_control_to_any_service_session`, `api::monitor_authority::every_mutating_monitor_handler_asks_the_one_authority_question`, `monitor::fail_closed_drill::the_database_refuses_an_agent_tripped_or_agent_reset_row`, `monitor::fail_closed_drill::no_agent_tripped_row_exists_anywhere`.
7. A global reset without a typed scope confirmation and reason is refused; a second reset within five minutes is refused.
   **Verified by:** `monitor::control::tests::a_global_reset_without_the_typed_scope_is_refused`, `monitor::control::tests::a_reset_without_a_reason_is_refused`, `monitor::control::tests::a_second_reset_within_five_minutes_is_refused`, `monitor::control::tests::nothing_that_is_not_a_named_human_can_reset`, `monitor::fail_closed_drill::a_second_reset_within_five_minutes_is_refused_by_the_store`.
8. A strategy with no `monitor_limits` cannot be promoted to `live`.
   **Verified by:** `monitor::limits::tests::promotion_is_refused_with_no_limits_at_all`, `monitor::limits::tests::one_token_limit_does_not_satisfy_the_gate`, `monitor::fail_closed_drill::a_strategy_with_no_declared_limits_cannot_be_promoted_to_live`.
9. A single-cadence excursion produces a `blip` and no state change; a three-cadence breach
   acts. A metric oscillating on its limit does not flap the book, asserted with a synthetic
   oscillating series.
   **Verified by:** `monitor::dwell::tests::a_single_cadence_excursion_is_a_blip_and_nothing_else`, `monitor::dwell::tests::three_consecutive_cadences_act`, `monitor::dwell::tests::a_synthetic_oscillating_series_never_flaps_the_book`, `monitor::dwell::tests::recovery_needs_twice_the_dwell`.
10. Per-metric-family precision is computed and published; a family below 0.5 is flagged for
   re-derivation.
   **Verified by:** `monitor::precision::tests::a_family_below_half_is_flagged_for_rederivation`, `monitor::precision::tests::an_unlabelled_family_is_unmeasured_rather_than_perfect_or_terrible`, `monitor::precision::tests::the_report_covers_every_family_including_the_silent_ones`.

## 9. Non-goals

- Deciding *why* a strategy degraded; the monitor names the metric and hands off to the agent's diagnosis workflow.
- Portfolio-level rebalancing (AGENT-006 `allocate_book`).
- Regulatory reporting formats.

## 10. Open questions

- Whether scale-down should be proportional to the breach magnitude rather than a fixed 25%; start fixed, revisit with monitor event data.
- Flatten-on-halt as default for live crypto (24/7 markets, no session close to hide behind) versus hold; the instrument's `HaltPolicy` decides, but the default for a new live promotion should probably be `reduce`.
