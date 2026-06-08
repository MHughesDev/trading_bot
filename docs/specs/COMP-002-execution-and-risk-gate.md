# COMP-002: Execution and Risk Gate

**Status:** Implemented
**Version:** 1.0
**ADR(s):** ADR-0005, ADR-0006
**Success Conditions:** SC-2, SC-6, SC-7

## 1. Purpose

Defines the risk gate — the single chokepoint through which every order passes before touching any broker — and the execution engine that handles order submission, fill processing, and reconciliation. The risk gate is the component that most separates a functional trading platform from one that drains an account. Every order, manual or automated, clears this gate. There is no bypass path.

## 2. Scope & Non-Goals

**In scope:**
- Risk gate as the sole order chokepoint: checks, kill switch, `risk_overrides` enforcement.
- Kill switch: automatic trips, manual trips, what it does and does not do.
- Order state machine: states and transitions.
- Fill handling: idempotent fill application.
- Reconciliation: on-fill, 30-second sweep, on-reconnect, halt-on-divergence.
- Broker adapter interface: how `Coinbase` (live), `Alpaca` (paper/live), and `market_simulator` (backtest) plug in.
- Idempotency key on orders.
- Asset-class differences (trading hours, halts) expressed through instrument metadata, not core branching.

**Not in scope (deliberate):**
- Strategy definition format — specified in DATA-004.
- Strategy runtime — specified in FEAT-001.
- Broker credential storage — infrastructure concern (encrypted at rest, per-user, per-venue).
- Paper trading live path fill simulation — owned by the broker adapter, not the risk gate.
- Backtest fill simulation — owned by `market_simulator` (external repository), not this component.
- Balance and position display in the UI — specified in COMP-003.

## 3. Design

### 3.1 The Risk Gate — One Chokepoint, No Bypass

```
Manual order (UI REST) ─┐
Strategy order intent ──┴──▶  RISK GATE  ──▶  Execution Engine  ──▶  Broker Adapter
```

Both paths converge on the same `RiskGate::check(order_request, context)` call. The strategy runtime cannot call a broker adapter directly. The UI cannot submit to an execution path that skips the gate. The gate is synchronous — an order is either approved and handed to the execution engine, or rejected with a structured error.

### 3.2 Risk Gate Checks (v1)

All checks are evaluated in order; the first failure rejects the order:

1. **Kill switch** — if `trading_enabled == false`, reject all orders immediately.
2. **Max position size** — order would not cause position to exceed configured limit for the instrument. Reads `Instrument.tick_size` / `lot_size` from instrument metadata (DATA-002).
3. **Lot validity** — order `size` is a positive multiple of `Instrument.lot_size`.
4. **Tick validity** — order `price` (for limit orders) is a multiple of `Instrument.tick_size`.
5. **Price sanity bounds** — for limit orders, price must be within a configurable band around the current market mid. Prevents fat-finger prices. Reads current market from `WorldState`.
6. **Max order rate** — orders per second and per minute from this user/strategy do not exceed limits.
7. **Max daily loss** — cumulative realized + unrealized loss for today does not exceed the configured threshold. Breach trips the kill switch automatically.
8. **Trust gate** — if the strategy has a `min_trust_tier`, the data that triggered the order intent must meet that tier. Checks `EventEnvelope.trust_tier` of the triggering event.
9. **Halt state** — for haltable instruments (`HaltPolicy::Haltable`), the instrument must not be in a halted state.
10. **Idempotency key** — if the `idempotency_key` on this order has already been processed (committed fill), reject as a duplicate (no double-submit on redelivery).

`risk_overrides` in the strategy definition may tighten checks 2, 6, 7 further. The gate applies the stricter of (global limit, strategy override).

### 3.3 Kill Switch

The kill switch is a global `trading_enabled` flag checked synchronously at the start of every risk gate evaluation. It can be set by any of these:

**Automatic trips:**
- Max daily loss breach for a user/account.
- Position/broker reconciliation divergence on any instrument.
- Market data staleness on an instrument that has an active strategy.
- Broker disconnection.
- Internal event bus (NATS JetStream) down.

**Manual trips:**
- UI "Kill Switch" button.
- `POST /api/trading/kill` REST endpoint.
- `POST /api/trading/resume` to re-enable (requires explicit human action).

**What the kill switch does:** blocks all new order submission.
**What it does not do:** force-close open positions. Closing open positions is a separate, deliberate action — automated position squaring is out of scope for v1 and must be explicitly implemented and tested before use.

### 3.4 Order State Machine

```
Created
  │
  ▼
Pending (sent to risk gate)
  │           │
  ▼           ▼
Approved    Rejected (terminal)
  │
  ▼
Submitted (sent to broker)
  │           │
  ▼           ▼
Accepted    Failed (terminal, retry eligible with new idempotency key)
  │
  ├──▶ PartiallyFilled (repeating until all filled or cancelled)
  │
  ├──▶ Filled (terminal)
  │
  ├──▶ CancelRequested
  │       │
  │       ▼
  └──▶ Cancelled (terminal)
```

All state transitions are published as events (see §4). State events are sacred — never dropped, never treated like UI messages.

### 3.5 Fill Handling — Idempotent

Every fill carries an `idempotency_key` (derived from the broker's fill identifier — not a random UUID). Applying fill `F` to a position:

1. Check if `F.idempotency_key` is in the `processed_fills` set for this account.
2. If yes: no-op — return success (idempotent replay).
3. If no: apply to position, update balance, record `F.idempotency_key`, emit `orders.filled` / `orders.partially_filled`.

This ensures broker reconnects and JetStream redeliveries are safe.

### 3.6 Reconciliation

Reconciliation is the mechanism that catches dangerous desync between internal state and broker state. It runs continuously:

| Trigger | Action |
|---------|--------|
| Every fill received | Reconcile position for that instrument against broker. |
| Every 30-second sweep | Reconcile all open positions and open orders against broker state. |
| On startup / after any disconnect | Query broker for open orders and positions before resuming trading. Do **not** trade on stale state. |
| On divergence detected | Halt new orders on the affected instrument; raise alarm. Require explicit human confirmation to resume. |

**Ack desync rule:** if an order was submitted and no ack arrived, the system must **query** the broker to determine the order's actual state — never blindly retry (blind retry double-fills). The idempotency key makes a confirmed-safe resubmit possible once the query confirms the original was not accepted.

### 3.7 Broker Adapter Interface

The execution engine talks to brokers through a trait, not concrete implementations:

```rust
pub trait BrokerAdapter {
    async fn submit_order(&self, order: &Order) -> Result<BrokerAck, BrokerError>;
    async fn cancel_order(&self, order_id: &str) -> Result<(), BrokerError>;
    async fn query_order(&self, order_id: &str) -> Result<OrderState, BrokerError>;
    async fn query_positions(&self) -> Result<Vec<Position>, BrokerError>;
    async fn query_open_orders(&self) -> Result<Vec<Order>, BrokerError>;
}
```

v1 adapters:

| Adapter | Mode | Notes |
|---------|------|-------|
| `CoinbaseAdapter` | Live | Crypto spot CEX. 24/7. No halts under normal operation. |
| `AlpacaAdapter` | Paper (v1) / Live (later) | Equities. Session hours, pre/post market, halt handling. |
| `MarketSimulatorAdapter` | Backtest | Wraps the external `market_simulator` library. Accepts Arrow IPC event data; returns fill records. No live path. |

Asset-class differences (trading hours, session restrictions, halt states) are expressed through `Instrument` metadata — the broker adapter reads them but the core order flow does not branch on them.

### 3.8 Execution Audit Trail

Every order, every state transition, every fill, every risk gate rejection, and every reconciliation event is written to the Postgres audit ledger as an immutable append. This is the authoritative record for:
- Debugging unexpected positions.
- Regulatory / compliance purposes.
- Post-incident reconstruction.

## 4. Interfaces

**Order events emitted (sacred — never dropped):**
```
orders.accepted          orders.rejected
orders.submitted         orders.cancel_requested
orders.cancelled         orders.partially_filled
orders.filled            positions.updated
balances.updated
```

**Risk gate entry point:**
```rust
pub fn check(
    order: &OrderRequest,
    context: &RiskContext,     // current position, balance, market mid, trust tier of trigger
) -> Result<ApprovedOrder, RiskRejection>;
```

**REST endpoints:**
```
POST   /api/orders                  — manual order → risk gate
DELETE /api/orders/{id}             — cancel order
POST   /api/trading/kill            — manual kill switch trip
POST   /api/trading/resume          — re-enable trading (explicit human action)
```

**Kill switch flag:** global `trading_enabled: AtomicBool` checked synchronously in `check()`.

## 5. Dependencies

- DATA-001 — `EventEnvelope.trust_tier` checked by the trust gate.
- DATA-002 — `Instrument` metadata: `tick_size`, `lot_size`, `halt_behavior`, `trading_hours`.
- DATA-003 — `available_time` of triggering event (for trust gate context).
- DATA-004 — `risk_overrides` from strategy definition (tighten-only).
- FEAT-001 — strategy runtime that emits order intents to the risk gate.
- COMP-004 — Postgres audit ledger for all order events.
- `market_simulator` (external) — `MarketSimulatorAdapter` wraps this for backtest fills.

## 6. Acceptance Criteria

- [x] AC-1: An order intent emitted by the strategy runtime cannot reach a broker adapter without passing through `RiskGate::check()` — no direct path exists in the codebase — Verified by: Compile-time: `ApprovedOrder._sealed: ()` is private — no path to broker without gate
- [x] AC-2: A manual order submitted via `POST /api/orders` passes through the same `RiskGate::check()` code path as a strategy order intent — Verified by: `risk::gate::tests::valid_order_approved` (manual + strategy paths converge on same `RiskGate::check`)
- [x] AC-3: When `trading_enabled` is `false`, all calls to `RiskGate::check()` return `RiskRejection::KillSwitch` without evaluating further checks — Verified by: `risk::tests::kill_switch_trips::manual_trip_blocks_immediately`
- [x] AC-4: Replaying the same fill event twice (simulating JetStream redelivery) results in the position being updated exactly once — Verified by: `execution::fills::tests` (idempotent fill tests)
- [x] AC-5: A position/broker reconciliation divergence halts new orders on the affected instrument and raises an alarm before any further order submission is attempted — Verified by: `reconciliation_halt` integration test in `tests/`
- [x] AC-6: On startup after a simulated disconnect, the system queries the broker for open orders and positions before the risk gate allows any new order submission — Verified by: `reconciliation_halt` integration test (startup query before resuming)
- [x] AC-7: A strategy `risk_overrides.max_position` value higher than the global limit is rejected at strategy validation time, not at order submission time — Verified by: `risk::tests::tighten_only::loosening_max_position_is_rejected`

## 7. Open Questions

Q-N (from `open-questions.md`): Real money vs paper mode gating decision — the system is currently scoped to paper/simulated execution first. The Alpaca adapter starts as paper-only. Switching to live requires the kill switch, reconciliation, and idempotency paths to be proven in production-grade paper trading before enabling.
