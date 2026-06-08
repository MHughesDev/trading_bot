---
Type: Formal
Status: Pending
Derived From: COMP-002, ADR-0005, ADR-0006, SC-2, SC-6, SC-7
Note: Canonical executable plans live in refactor_reference_docs/plans/. This copy is the traceable documentation record. On any conflict, refactor_reference_docs/ wins.
---

# Phase 2 — Money safety (before any automation touches orders)

> **Self-contained execution doc.** You need only: this file, [`../architecture.md`](../architecture.md),
> and the specs — especially
> [`../specs/COMP-002-execution-and-risk-gate.md`](../specs/COMP-002-execution-and-risk-gate.md) and
> [`../specs/COMP-001-data-quality-and-ingestion.md`](../specs/COMP-001-data-quality-and-ingestion.md)
> §3, §7.
>
> **This is the phase that separates a fun project from "the night a strategy drained the account."**
> It is cheap to build and decisive. No automation (Phase 4) is allowed to emit orders until this
> exists and its adversarial tests are green.

## Phase goal

After this phase, **every order — manual or automated — passes through one idempotent risk gate**
before reaching a **paper execution adapter** that implements the same interface a live broker will.
A global **kill switch** can stop all new orders synchronously and trips automatically on defined
danger conditions. A **reconciliation loop** continuously checks internal positions against the paper
broker and halts the affected instrument on divergence. A user can submit a **manual order** via REST
that flows UI → risk gate → paper execution → position update.

## Prerequisites

- Phase 1 complete: bus, storage, API skeleton, instrument metadata available.
- **Decision gate Q1 (resolved):** Paper execution uses the **Alpaca paper account** for all assets
  and all domains — `crates/execution/src/alpaca.rs` implements the `Broker` trait against Alpaca's
  paper trading REST API. This is the paper adapter for Phase 2; the Coinbase live adapter
  (`coinbase.rs`) is post-Phase-6 scope. All three systems share the same `Broker` interface.
- `legacy_python/risk_engine/` and `legacy_python/execution/` contain the existing gate/execution
  behavior — read for parity (limits, sizing, reconciliation), do not import.

## Invariants this phase must respect

- **One chokepoint, no bypass.** The only way to submit an order is through `crates/risk`'s gate.
  No other crate may call a broker's `submit`. (Enforced in review: only `execution` holds a
  `Broker`, and it only acts on an `ApprovedOrder` produced by `risk`.)
- **Idempotency everywhere money moves.** The gate dedups intents by idempotency key; fills are
  keyed by fill id and replay as no-ops; a missing ack triggers a **query, never a blind retry**.
- **`risk_overrides` may only tighten.** A definition/limit that loosens the global gate is rejected.
- **Sacred events are never dropped.** `orders.*`, `positions.*`, `balances.*` use never-drop bus
  policies.

---

## Tasks

### P2-T01 — Risk limits
- **Goal:** The individual limit checks.
- **Files:** `crates/risk/src/limits.rs`.
- **Context:** Implement, reading instrument metadata where needed (per
  [`../specs/COMP-002-execution-and-risk-gate.md`](../specs/COMP-002-execution-and-risk-gate.md)
  §checks): max position size per instrument, max order rate (per second/minute — catches a submit
  loop), price sanity bounds (using `tick_size` + a band around current market), lot/tick validity,
  max daily loss per user/account. Each returns a structured pass/`RiskRejection`.
- **Acceptance:** unit tests per limit, including boundary cases (exactly-at-limit, fat-finger price,
  sub-tick size).
- **Depends on:** Phase 0 (instrument metadata, order types), Phase 1 (latest market price via Redis).

### P2-T02 — Tighten-only overrides + trust gate
- **Goal:** Apply per-strategy `risk_overrides` as tighten-only, and refuse orders derived from data
  below the strategy's `min_trust_tier`.
- **Files:** `crates/risk/src/overrides.rs`, `crates/risk/src/trust_gate.rs`,
  `crates/risk/tests/tighten_only.rs`.
- **Context:** `overrides.rs` takes global limits + a definition's overrides and produces the
  effective limits, rejecting any override that would loosen a global value (per
  [`../specs/COMP-002-execution-and-risk-gate.md`](../specs/COMP-002-execution-and-risk-gate.md)
  and [`../specs/DATA-004-strategy-definition-format.md`](../specs/DATA-004-strategy-definition-format.md)).
  `trust_gate.rs` compares the event trust tier behind an intent against `min_trust_tier`.
- **Acceptance:** `tighten_only.rs` proves a loosening override is rejected and a tightening one is
  applied; a too-low-trust intent is refused.
- **Depends on:** P2-T01, Phase 0 (trust tier, strategy def).

### P2-T03 — Kill switch
- **Goal:** A global `trading_enabled` flag, checked synchronously, that blocks all new orders and
  trips on danger conditions — and works even if half the system is broken.
- **Files:** `crates/risk/src/kill_switch.rs`, `crates/risk/tests/kill_switch_trips.rs`.
- **Context:** Per [`../specs/COMP-002-execution-and-risk-gate.md`](../specs/COMP-002-execution-and-risk-gate.md)
  §kill switch: trips automatically on max-daily-loss breach, position/broker reconciliation
  divergence, market-data staleness on an active instrument, broker disconnect, bus down. Manual trip
  via a method (REST endpoint added in P2-T07). The flag is checked synchronously by the gate. It
  does **not** force-close positions — it only blocks new orders. Back the flag with Postgres (`0005`
  migration) + an in-memory fast path.
- **Acceptance:** `kill_switch_trips.rs` proves each auto-trip condition blocks new orders and that a
  manual trip blocks immediately; open positions are untouched.
- **Depends on:** P2-T01.

### P2-T04 — The risk gate (idempotent chokepoint)
- **Goal:** Compose limits + overrides + trust + kill switch into one synchronous, idempotent gate.
- **Files:** `crates/risk/src/gate.rs`, `crates/risk/src/lib.rs`,
  `crates/risk/tests/idempotent_gate.rs`.
- **Context:** `RiskGate::check(intent) -> Result<ApprovedOrder, RiskRejection>` runs the kill-switch
  check, all limits, the trust gate, and applies effective (tightened) limits. **Idempotent by
  idempotency key**: a redelivered intent with a seen key returns the prior decision, never a second
  approval (per [`../specs/COMP-001-data-quality-and-ingestion.md`](../specs/COMP-001-data-quality-and-ingestion.md)
  §3). `ApprovedOrder` is the only type `execution` will accept — it cannot be constructed outside
  `risk`.
- **Acceptance:** `idempotent_gate.rs` proves a redelivered intent does not double-approve; rejections
  carry structured reasons.
- **Depends on:** P2-T02, P2-T03.

### P2-T05 — Execution engine + Alpaca paper broker adapter
- **Goal:** The order state machine + **Alpaca paper account** adapter behind the `Broker` interface.
- **Files:** `crates/execution/src/{lib,broker,alpaca,order_state,fills,positions,audit,events}.rs`,
  `crates/execution/tests/{idempotent_fills,ack_timeout_query,partial_fill}.rs`.
- **Context:** Per [`../specs/COMP-002-execution-and-risk-gate.md`](../specs/COMP-002-execution-and-risk-gate.md)
  §execution. `Broker` trait: `submit`, `cancel`, `query_open_orders`, `query_positions`. Implement
  `alpaca.rs` against the **Alpaca paper trading REST API** (all assets/domains). The Coinbase live
  adapter (`coinbase.rs`) is added post-Phase-6; the market_simulator backtest adapter
  (`market_simulator.rs`) is added in Phase 4. All three implement the same `Broker` trait — the
  runtime and risk gate never know which is active. `order_state.rs` is the state machine; `fills.rs`
  handles fills/partials **idempotently by fill id** (replay = no-op); `positions.rs` updates
  positions/balances from fills; `audit.rs` is the append-only execution trail; `events.rs`
  publishes the **sacred** `orders.*`/`positions.*`/`balances.*` lanes (never dropped). On a missing
  ack from Alpaca, **query** rather than blind-retry (the idempotency key makes a safe retry).
  Read `legacy_python/execution/alpaca_util.py` for Alpaca API parity (do not import).
- **Acceptance:** `idempotent_fills` (replayed fill = no-op), `ack_timeout_query` (missing ack →
  query, not retry), `partial_fill` (partials aggregate correctly into the position) all pass.
- **Depends on:** P2-T04, Phase 1 (bus for publishing events).

### P2-T06 — Reconciliation loop
- **Goal:** Continuously reconcile internal vs broker state; halt + alarm on divergence.
- **Files:** `crates/reconciliation/src/{lib,positions,freshness,sequence,divergence}.rs`,
  `crates/reconciliation/tests/{position_divergence_halts,freshness_respects_hours}.rs`.
- **Context:** Per [`../specs/COMP-002-execution-and-risk-gate.md`](../specs/COMP-002-execution-and-risk-gate.md)
  §reconciliation and
  [`../specs/COMP-001-data-quality-and-ingestion.md`](../specs/COMP-001-data-quality-and-ingestion.md)
  §7: position reconciliation on every fill + a 30-second sweep + on every reconnect; on divergence →
  `divergence.rs` trips the kill switch for that instrument + alarms. `freshness.rs` is the per-lane
  watchdog that **reads instrument `trading_hours`/`halt_behavior`** so a normal 4pm equity close
  does not false-alarm. `sequence.rs` consumes `gap.detected` and marks windows suspect.
- **Acceptance:** `position_divergence_halts` proves a forced divergence halts new orders on that
  instrument; `freshness_respects_hours` proves a normal close does not alarm while a true feed
  outage does.
- **Depends on:** P2-T03 (kill switch), P2-T05 (broker query).

### P2-T07 — Manual order REST + kill REST
- **Goal:** Expose manual order submission and the kill switch over REST, both routed through the
  risk gate.
- **Files:** `crates/api/src/routes/orders.rs`, `crates/api/src/routes/trading.rs`, route registration
  in `crates/api/src/routes/mod.rs`.
- **Context:** `POST /api/orders` (manual) builds an `OrderIntent` (with idempotency key) and runs it
  through **the same risk gate** as strategies will (per
  [`../specs/FEAT-001-strategy-system.md`](../specs/FEAT-001-strategy-system.md) §manual+automated
  coexist); `DELETE /api/orders/{id}` cancels; `POST /api/trading/kill` trips the kill switch (per
  [`../specs/COMP-003-ui-streaming-gateway.md`](../specs/COMP-003-ui-streaming-gateway.md)). Private
  order/position data is scoped to the authenticated user.
- **Acceptance:** a manual order via REST flows through the gate to paper execution and updates the
  position; a rejected order returns the structured reason; `POST /api/trading/kill` blocks the next
  order.
- **Depends on:** P2-T04, P2-T05, Phase 1 (API + auth).

### P2-T08 — Manual order flow integration test
- **Goal:** Prove the manual money path end to end.
- **Files:** `tests/manual_order_flow.rs`, `tests/reconciliation_halt.rs`.
- **Context:** `manual_order_flow`: REST order → risk gate → paper execution → fill → position +
  `positions.updated` event. `reconciliation_halt`: force an internal/broker divergence → instrument
  halted + alarm; subsequent orders on it rejected.
- **Acceptance:** both pass against compose infra.
- **Depends on:** P2-T06, P2-T07.

---

## Phase exit criteria

- [ ] `crates/{risk,execution,reconciliation}` implemented; the only path to a broker is through the
      gate (verified in review: `execution` acts only on `ApprovedOrder`); `alpaca.rs` implements
      `Broker` against Alpaca paper account; `coinbase.rs` and `market_simulator.rs` are stubs only.
- [ ] The gate is idempotent; `risk_overrides` are tighten-only; the trust gate refuses low-trust
      intents.
- [ ] The kill switch blocks new orders synchronously and trips on each defined condition without
      force-closing positions.
- [ ] Paper execution handles fills/partials idempotently and queries (never blind-retries) on a
      missing ack.
- [ ] Reconciliation halts an instrument on divergence and the freshness watchdog respects trading
      hours.
- [ ] Manual orders flow UI→REST→gate→paper execution; `tests/manual_order_flow.rs` and
      `tests/reconciliation_halt.rs` pass.
