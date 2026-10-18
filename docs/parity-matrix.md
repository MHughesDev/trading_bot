# Behavior Parity Matrix

> **Phase 7 / P7-T01 + P7-T02.** Verified 2026-06-08.  
> All 255 workspace tests pass with zero failures (`cargo test --workspace`).

This matrix records what the Rust platform does relative to the Python system it replaces. For each behavioral domain the evidence column names the Rust test(s) that prove the behavior. Where the Python behavior was incorrect or undefined, the intentional correction is documented.

---

## Adversarial Test Sweep (P7-T02)

All "decided mechanism" tests from the spec are green:

| Test | Crate | Status |
|------|-------|--------|
| `kill_switch_trips::manual_trip_blocks_immediately` | risk | PASS |
| `kill_switch_trips::max_daily_loss_breach_trips_switch_and_blocks` | risk | PASS |
| `kill_switch_trips::reset_re_enables_order_flow` | risk | PASS |
| `kill_switch_trips::gate_blocks_when_started_active` | risk | PASS |
| `kill_switch_trips::trip_does_not_force_close_existing_positions` | risk | PASS |
| `tighten_only::tightening_max_position_is_applied` | risk | PASS |
| `tighten_only::tightening_rate_per_minute_is_applied` | risk | PASS |
| `tighten_only::loosening_max_position_is_rejected` | risk | PASS |
| `tighten_only::loosening_rate_per_second_is_rejected` | risk | PASS |
| `tighten_only::low_trust_intent_refused_by_gate` | risk | PASS |
| `idempotent_gate::redelivered_approved_intent_returns_approved_without_rerunning` | risk | PASS |
| `idempotent_gate::redelivered_rejected_intent_returns_same_rejection` | risk | PASS |
| `idempotent_gate::different_keys_are_independent_decisions` | risk | PASS |
| `gate::tests::equity_outside_session_rejected` | risk | PASS |
| `gate::tests::equity_halted_rejected` | risk | PASS |
| `gate::tests::equity_in_session_not_halted_approved` | risk | PASS |
| `gate::tests::crypto_non_haltable_ignores_halt_flag` | risk | PASS |
| `equity_gate::equity_order_outside_session_is_rejected` | risk | PASS |
| `equity_gate::equity_order_during_halt_is_rejected` | risk | PASS |
| `equity_gate::equity_order_in_session_not_halted_is_approved` | risk | PASS |
| `cross_asset_parity::kill_switch_blocks_both_asset_classes` | risk | PASS |
| `cross_asset_parity::single_gate_approves_both_asset_classes` | risk | PASS |
| `builders::bar::tests::late_data_revision_supersedes_original` | builders | PASS |
| `builders::bar::tests::watermark_respected_available_time` | builders | PASS |
| `replay_determinism` (integration) | tests/ | PASS |
| `strategy_end_to_end` (integration) | tests/ | PASS |
| `quarantine_replay` (integration) | tests/ | PASS |
| `reconciliation_halt` (integration) | tests/ | PASS |

**Total: 255 tests pass, 0 fail.**

---

## Domain-by-Domain Parity

### 1. Ingestion and Normalization

| Behavior | Python status | Rust behavior | Evidence |
|----------|--------------|---------------|----------|
| `TradePayload` normalized from Kraken WS JSON | Implemented (used `float`) | `Price`/`Size` backed by `Decimal`; `From<f64>` does not compile | `domain` crate compile-time; `collectors::kraken::tests::normalize_valid_trade` |
| `TradePayload` normalized from Alpaca IEX WS JSON | Not implemented | Implemented Phase 6 | `collectors::equity::alpaca_data::tests::normalize_valid_trade` |
| Missing required field → quarantine lane | Partially: exceptions swallowed silently | `NormalizeError::MissingField` routes raw bytes to quarantine | `collectors::equity::alpaca_data::tests::normalize_missing_price_returns_error` |
| Reconnect on WS disconnect | Yes (bare retry) | Exponential back-off via `ReconnectPolicy` | `collectors::reconnect::tests` |
| Sequence gap detection | No | `GapDetector` warns on sequence gaps | `collectors::gap::tests` |

**Intentional improvement:** The Python system used raw floats for prices and sizes. The Rust system enforces decimal arithmetic at compile time via newtypes — `f64` literal prices do not compile.

---

### 2. Bar Building

| Behavior | Python status | Rust behavior | Evidence |
|----------|--------------|---------------|----------|
| Bar closes on watermark expiry | Yes | `BarBuilder` closes when `event.available_time > bar_end + watermark` | `builders::bar::tests::watermark_respected_available_time` |
| Late data emits revision bar | No — late data was silently discarded | Emits `BarPayload` with `revision > 0`; original immutable | `builders::bar::tests::late_data_revision_supersedes_original` |
| `available_time` prevents lookahead | Not enforced | `available_time` is set to `max(observed_time, event_time + watermark)`; strategies receive only what was available at that time | `replay_determinism` integration test |

**Intentional improvement:** Late data handling — Python silently dropped late trades. Rust emits revision events so backtests and strategies can see the correction.

---

### 3. Risk Gate

| Behavior | Python status | Rust behavior | Evidence |
|----------|--------------|---------------|----------|
| Kill switch blocks all orders | Yes | `KillSwitch::is_active()` checked before any other check | `kill_switch_trips::manual_trip_blocks_immediately` |
| Kill switch trips on daily loss breach | Partially | `check_daily_loss` → auto-trip | `kill_switch_trips::max_daily_loss_breach_trips_switch_and_blocks` |
| Tighten-only risk overrides | Not enforced — overrides were not validated at strategy load time | Checked at validation time in `strategy-validator` + at gate evaluation | `tighten_only::loosening_max_position_is_rejected` |
| Idempotent order deduplication | Not reliable under redelivery | Idempotency key cache in gate; redelivered intents return cached decision | `idempotent_gate::redelivered_approved_intent_returns_approved_without_rerunning` |
| Equity session enforcement | Not present | `is_in_session` check; rejects outside NYSE session | `equity_gate::equity_order_outside_session_is_rejected` |
| Equity halt enforcement | Not present | `is_halted` + `HaltPolicy::Haltable` check | `equity_gate::equity_order_during_halt_is_rejected` |
| Trust tier gate | Not present | `check_trust` rejects if `event_trust_tier < strategy_min_trust_tier` | `tighten_only::low_trust_intent_refused_by_gate` |

**Intentional improvement:** Risk overrides — Python allowed strategies to set limits *looser* than the global gate (i.e., a strategy could override `max_position` to a larger value). Rust validates tighten-only at strategy creation time.

---

### 4. Strategy System

| Behavior | Python status | Rust behavior | Evidence |
|----------|--------------|---------------|----------|
| Strategy definition format frozen | No — fields were ad hoc | Frozen v1.0 JSON format; version `"1.0"` required | `strategy-validator::tests::*` |
| Expression language validated at load time | Partial — runtime errors at execution | Full parse + type-check at validation; sealed `ValidatedDefinition` | `strategy-validator::tests::*` |
| `world.now()` returns event time, not wall clock | Not enforced | `StrategyClock` trait; `WallClock` and `ReplayClock` impls; wall clock never read in runtime | `strategy-runtime::tests::no_wallclock` |
| Deterministic replay | Not guaranteed (wall-clock reads leaked) | Same event sequence → identical intents | `strategy-runtime::tests::replay_determinism` |

---

### 5. Paper Fill / Execution

| Behavior | Python status | Rust behavior | Evidence |
|----------|--------------|---------------|----------|
| All orders flow through risk gate | Partial — some manual paths bypassed | `ApprovedOrder` sealed type (`_sealed: ()`) — cannot be constructed outside `risk` crate | Compile-time enforcement; `gate::tests::valid_order_approved` |
| Idempotent fill application | Not reliable | `processed_fills` set; double-fill on redelivery is a no-op | `execution::fills::tests::*` |
| Reconciliation halts on divergence | Not present | `ReconciliationEngine` halts instrument on divergence | `reconciliation_halt` integration test |

---

### 6. Backtest

| Behavior | Python status | Rust behavior | Evidence |
|----------|--------------|---------------|----------|
| No-lookahead guarantee | Not enforced | `available_time` ordering enforced in replay; same `StrategyClock` as live | `replay_determinism` integration test |
| Arrow IPC export for market_simulator | Not present | `crates/market-simulator-adapter`: `bars_to_ipc_bytes` | `market-simulator-adapter::tests::backtest_adapter::*` |

---

## Behaviours with no Python counterpart (Set S phase 8, 2026-09-17)

This matrix compares the Rust platform to the Python system it replaced. The rows
below have no Python column because the Python system had none of them — they are
research-honesty mechanisms the rewrite added rather than ported. Recorded here so the
matrix stays a complete map of what the platform does, with the same rule: **the
evidence column names the test that proves the behaviour.**

| Behaviour | Rust behaviour | Evidence |
|-----------|----------------|----------|
| A battery is one call under one trial family | `Tier::Workflow` declarations in `mcp_server_lib::tools::workflows`; a family id minted before the first step is admitted and carried on every reservation (`0134`) | `tools::workflows::tests::{a_battery_spends_nothing_of_its_own,the_result_carries_every_steps_ticket,a_caller_cannot_unpin_a_pinned_step_parameter}` |
| A battery that cannot run is absent, never short | `Stage::Owed`; `specs()` filters on `is_runnable()` | `tools::workflows::tests::{a_battery_with_an_owed_step_is_not_offered,every_owed_stage_names_what_it_needs}` |
| A gate refuses without the ticket that licenses it | `backtest::gates::tickets::check` over rows only the engine writes (`0135`); checked before any Study runs | `backtest::gates::tickets::tests::*` and `invariants::gates_demand_tickets::*` |
| A report cannot be finalised on an unsupported claim | Post-hoc span alignment over the evidence bundle; `Unverified` blocks the dossier | `harness::report::claims::tests::*`, `harness::report::dossier::tests::an_unsupported_claim_blocks_compilation` |
| A mandatory section says "none found" rather than being omitted | `Section::when_empty`, per section; `Parts.critique: Option<Vec<_>>` so "no critic ran" and "found nothing" cannot collapse | `harness::report::dossier::tests::{an_empty_section_still_renders_and_says_so,a_critique_that_never_ran_is_not_an_empty_critique,each_empty_section_says_what_was_empty}` |
| The critic cannot read the producer's reasoning | `Projection::Critic` excludes it by construction; `Delegation::check` refuses another projection | `harness::report::critic::tests::the_model_critic_cannot_read_the_producers_reasoning` |
| A claim can be re-derived against the platform as it stands | `POST /api/dossiers/{ticket}/verify/{claim}`; three outcomes, with "the evidence no longer resolves" distinct from "the claim failed" | `api::routes::dossiers::tests::{a_recheck_fails_only_when_the_claim_got_weaker,the_verification_words_are_the_ones_the_desk_knows}` |
| Free-form code runs outside the platform process or not at all | `infra/agent-image` executor; `POST /api/code/run` refuses with no launcher configured; the `code` namespace is hidden below the `full` tier | CI job `agent-sandbox-image` (three refusals proved against the built image), `invariants::code_execution_is_tiered::*`, `mcp-server-lib::tools::code::tests::*` |
| A sandbox holding a credential does not start | `executor.py` checks ten variable names and exits non-zero | CI job `agent-sandbox-image`, step "It refuses to start holding a credential" |

---

## Behaviours with no Python counterpart (Set S close-out, 2026-10-04)

Same rule as the section above: the Python system had none of these, so there is no
Python column, and **the evidence column names tests that exist** (each name was grepped
against the tree on 2026-10-04). Where the proof is partial the row says so; the
acceptance criteria themselves are in `docs/specs/AGENT-005` to `AGENT-014`, each with a
`Verified by:` slot.

| Behaviour | Rust behaviour | Evidence |
|-----------|----------------|----------|
| Research decisions are scored against planted ground truth, per tier and workflow | `eval::judgement_suite` over `planted::PlantedCorpus` (40 cases) + 6 dead ideas; six criteria, never one number; scorecard cells `judgement/<workflow>` per tier; **the tapes are synthetic reference policies, not a measurement of a model** (live run: gate H-2) | `harness::eval::judgement_suite::tests::{a_harness_that_reports_a_planted_null_supported_fails_the_suite,a_run_that_re_tests_the_planted_dead_idea_fails_the_suite,the_scorecard_shows_judgement_per_tier_and_per_workflow}`, `evals/tests/test_judgement_runner.py` |
| The eval suite has a sealed 20% that never runs on a PR | `Holdout::draw` per suite; the PR build compiles in only the open tapes; sealed cases run in `nightly_*` tests | `harness::eval::judgement_suite::tests::{the_set_a_pr_runs_contains_no_sealed_case,a_sealed_tape_or_a_sealed_id_is_refused_on_a_pr_path}`, `harness::eval::cases::tests::the_set_a_pr_runs_contains_no_sealed_case`, `harness::eval::scorecard::tests::a_version_fitted_to_the_open_set_fails_promotion` |
| A regression in the 7B's routing blocks a PR even when the frontier improves | `scorecard::gate` over the committed replay baseline; a newly failing replay case blocks with no threshold; a live suite needs n >= 30 | `harness::eval::cases::tests::{a_regression_in_the_7b_routing_suite_blocks_even_if_the_frontier_improves,the_scorecard_a_pr_produces_does_not_regress_against_the_committed_baseline}` |
| A Chat turn cannot spend a trial, by exposure and again at the door | Ring filter before routing; `CallGate::for_chat_turn`; the reservation route refuses a body naming a Chat run | `harness::drive::tests::a_chat_turn_cannot_see_a_tool_that_writes`, `mcp-server-lib::gate::tests::a_chat_turn_cannot_spend_a_trial_even_if_it_reaches_the_door`, `api::routes::ledger::tests::a_reservation_naming_a_chat_run_is_refused_and_a_research_run_is_not` (partial: no live Chat turn) |
| A run cannot spend a trial without a sealed pre-registration naming its falsifier | `ledger::prereg` (no serde default on `delta_practical`; the falsifier is in the hash; the third amendment refused); `CallGate::under_preregistration` | `ledger::prereg::tests::{a_draft_without_delta_practical_does_not_deserialise,changing_the_falsifier_changes_the_hash,a_protocol_rewritten_three_times_is_not_a_protocol}`, `mcp-server-lib::gate::tests::a_run_with_no_sealed_registration_cannot_spend_a_trial` |
| The platform, never the model, writes a hypothesis verdict | `lifecycle::evaluate` over ledger tickets; `supported` needs the whole battery | `harness::lifecycle::evaluate::tests::{supported_needs_the_whole_battery,a_finding_that_grades_its_own_hypothesis_is_corrected}`, `api::routes::lifecycle::tests::a_hypothesis_cannot_arrive_with_its_own_verdict` |
| A design under the minimum track record is refused before any trial | `research::power` reads the *registered* delta and window, not an observed Sharpe | `research::power::tests::{a_window_that_cannot_settle_the_question_is_refused_with_a_number,a_zero_delta_is_refused_rather_than_answered}` |
| No agent tool writes the promoted set | Static scan of every MCP tool function; the two allowed writers are `Promotion` or exposure-reducing | `invariants/tests/no_agent_tool_writes_the_promoted_set.rs::no_tool_writes_the_promoted_set_except_the_two_that_are_gated_or_reduce_exposure` |
| A continuous session is a pure fold over an append-only log | `research-session::fold`; restart re-derives the same state; a hole in the log is an anomaly | `research-session::fold::tests::{restarting_between_start_and_record_re_derives_the_same_state,a_hole_in_the_log_is_an_anomaly,stopped_is_the_only_terminal_state}` |
| Exploration has a floor and every selection is logged with what it rejected | 10% floor in the type, in `0138`'s CHECK and in the decision row's whole candidate set | `invariants/tests/exploration_floor_is_one_number.rs::*`, `research-session::select::tests::the_exploration_share_is_achieved_over_forty_selections` |
| A session that stops producing knowledge stops itself | Stall detector: six barren runs, degenerating refinement, circling, cost drift; a stalled session claims nothing until a human acts | `research-session::stall::tests::{six_barren_runs_trip_the_streak_and_name_all_six,refinement_only_runs_that_establish_nothing_are_degenerating,the_stalled_event_carries_the_word_the_fold_reads_and_a_stalled_session_claims_nothing}` |
| A finding cannot cite an id no tool returned, and cannot be established on a number the cited result does not hold | `memory::verify` computes and stores the verdict; the loop refuses an uncited identifier | `harness::memory::verify::tests::{a_finding_citing_an_id_no_tool_returned_is_refused,a_number_absent_from_the_cited_result_is_stored_suggestive}` |
| A run that waits for a job spends no tokens | `Effect::Await` suspends and the driver exits; the manager resumes on `JobDone` | `harness/tests/waiting_costs_nothing.rs::a_run_waiting_on_a_job_emits_no_model_call` (partial: the DB resume path is untested end to end) |
| A killed run does not repeat a write | Effect log written before the effect; an outstanding write is answered from `agent_idempotency` or reported interrupted | `api::agent::local_driver::tests::a_resumed_run_does_not_repeat_an_interrupted_create_experiment`, `api::agent::resume::tests::an_interrupted_write_is_not_retried` (partial: simulated by a truncated log) |

---

## Sign-off

Parity verified by: automated test suite (`cargo test --workspace`), 2026-06-08.

All intentional differences from Python behavior are improvements (decimal money types, revision bars, tighten-only overrides, no wall-clock in runtime, sealed `ApprovedOrder`). None constitute regressions.
