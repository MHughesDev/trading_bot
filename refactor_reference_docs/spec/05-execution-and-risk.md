# 05 — Execution & Risk

This is the component the original architecture underweighted and the one that most separates a
fun project from "the night a strategy drained the account." It is cheap to build and decisive.

## The risk gate: one chokepoint, no bypass

**Every order — manual or automated — passes through one risk gate before execution.** The
strategy runtime cannot bypass it. The manual UI cannot bypass it. No order leaves the building
without clearing it.

```
Manual order (UI) ─┐
Strategy intent ───┴─▶  RISK GATE  ─▶  Execution Engine  ─▶  Broker / Exchange
```

Because v1 is a small trusted group (not multi-tenant), the gate can be simple: a few config
limits per user, enforced synchronously.

### Checks (v1)

- **Max position size** per instrument.
- **Max order rate** (per second / per minute) — catches a strategy stuck in a submit loop.
- **Price sanity bounds** — reject fat-finger prices using `tick_size` and a band around the
  current market (reads instrument metadata).
- **Lot/tick validity** — size and price conform to the instrument.
- **Max daily loss** per user/account — breach trips the kill switch.
- **Trust gate** — refuse orders derived from data below the strategy's `min_trust_tier`.

`risk_overrides` in a strategy definition may **tighten** these but never **loosen** the global
limits.

### Idempotency

The gate is idempotent: every order carries an **idempotency key**; a redelivered intent does not
double-submit. (See [03-data-engineering.md](./03-data-engineering.md) §3.)

## The kill switch

One action stops **all** new orders immediately, and it must work even if half the system is
broken:

- A global `trading_enabled` flag checked synchronously by the risk gate.
- Trips automatically on: max-daily-loss breach, position/broker reconciliation divergence,
  market-data staleness on an active instrument, broker disconnect, bus down.
- Trips manually via a UI button and a REST endpoint.
- Open positions are **not** force-closed by the kill switch (that's a separate, deliberate
  action); it only blocks *new* orders.

## Execution engine responsibilities

- Order submission / cancellation against broker/exchange APIs.
- Fill and partial-fill handling.
- Order state machine.
- Position and balance updates.
- Execution audit trail.

Publishes (these events are **sacred — never dropped**, never treated like UI messages):

```
orders.accepted        orders.rejected
orders.submitted       orders.cancel_requested
orders.cancelled       orders.partially_filled
orders.filled          positions.updated        balances.updated
```

## Reconciliation (where money is actually lost or saved)

The dangerous failures are desync failures. The system continuously answers *does my internal
view match the broker's view?*

- **On startup / after any disconnect:** before resuming trading, query the broker for open
  orders and positions and reconcile against internal state. Do **not** trade on stale state.
- **Order ack desync:** if an order is sent and no ack arrives, the system must **query**, never
  blindly retry (blind retry double-fills). The idempotency key makes a confirmed-safe retry
  possible.
- **Continuous:** position reconciliation on every fill + 30-second sweep; divergence → halt that
  instrument + alarm.

## Stocks vs crypto execution differences (v1)

These live in instrument metadata and broker adapters, not in the core:

- **Trading hours / sessions / auctions** — equities reject orders outside session; crypto is
  24/7.
- **Halts** — equities can halt; the gate must respect halt state.
- **Settlement / shorting rules** — equities have constraints crypto does not; encode per-venue.

The core order flow and risk gate are identical across asset classes; only the broker adapter and
metadata differ.

## Real money vs paper (gating decision)

This is an open question (see [10-open-questions.md](./10-open-questions.md)) and it changes the
risk posture and the broker adapter. **Recommendation: build and run against a paper/simulated
execution adapter first.** The same execution interface backs both paper and live, so flipping to
a live broker adapter is a swap, not a rewrite — but only after reconciliation and the kill switch
are proven.
