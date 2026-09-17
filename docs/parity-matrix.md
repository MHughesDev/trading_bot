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

## Sign-off

Parity verified by: automated test suite (`cargo test --workspace`), 2026-06-08.

All intentional differences from Python behavior are improvements (decimal money types, revision bars, tighten-only overrides, no wall-clock in runtime, sealed `ApprovedOrder`). None constitute regressions.
