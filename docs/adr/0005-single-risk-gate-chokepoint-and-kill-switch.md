# ADR-0005: Single Risk Gate Chokepoint and Kill Switch

**Status:** Accepted
**Date:** 2026-06-08
**Deciders:** Platform team

## Context

A trading platform that supports both manual (UI-originated) and automated (strategy-originated) order submission has multiple entry points for orders. Without an enforced architectural chokepoint, it is possible — through incremental feature additions, convenience shortcuts, or bugs — for an order to reach a broker without passing through risk validation. This scenario has caused real monetary losses in production systems.

The specific failure modes that motivated this decision:

- A strategy stuck in a submit loop sending hundreds of orders per second.
- A fat-finger order at a price several standard deviations from the market.
- A strategy exceeding its declared position size because position state drifted from the broker.
- A manual UI order submitted during a market data staleness event, trading blind.
- A redelivered order intent from JetStream being submitted twice, doubling a position.
- A max-daily-loss breach continuing to send orders because the halt was only advisory.

The system also serves a trusted group of users (not anonymous multi-tenant), which means the risk gate can be simple and synchronous without complex tenant isolation — but it must be universally enforced.

## Decision

Every order — whether originated by a strategy instance or submitted manually through the UI — passes through a single `crates/risk` risk gate before reaching the execution engine. There is no bypass path. The strategy runtime and UI have no direct path to a broker adapter; they can only produce order intents that flow through the gate.

The gate performs synchronous checks (v1):
- Maximum position size per instrument
- Maximum order rate (per second / per minute)
- Price sanity bounds (band around current market, using instrument `tick_size`)
- Lot/tick validity (size and price conform to instrument metadata)
- Maximum daily loss per user/account
- Trust gate (refuse orders derived from data below the strategy's declared `min_trust_tier`)

A global `trading_enabled` kill switch is checked synchronously by the gate on every order. It trips automatically on: max-daily-loss breach, position/broker reconciliation divergence, market data staleness on an active instrument, broker disconnect, and bus unavailability. It can also be tripped manually via a UI button or a REST endpoint.

Strategy `risk_overrides` may tighten individual limits but may never loosen them beyond the global configuration.

## Rationale

The risk gate as a single chokepoint is not a performance optimization — it is a safety property. For it to be reliable, it must be structurally impossible to route around. Placing `crates/risk` as the only interface between order intents and `crates/execution` enforces this at the module boundary level, not just by convention.

Synchronous gate execution on the same thread (or async task) as the order intent means there is no window between "risk check passed" and "order submitted" where system state can change. An async RPC to a separate risk service introduces exactly that window, as well as a failure mode where the service is slow or unavailable.

The kill switch must remain functional "even if half the system is broken." A simple boolean flag checked synchronously in the gate, rather than a distributed coordination mechanism, satisfies this requirement. Open positions are not force-closed (that is a separate deliberate action), preserving the human's ability to decide what to do with existing exposure.

Idempotency keys on every order intent prevent JetStream redelivery from double-submitting. The gate records processed idempotency keys so a redelivered intent is a no-op.

## Consequences

**Positive:**
- Structural impossibility of bypass: no code path from strategy runtime or UI reaches the execution engine without clearing the risk gate.
- Kill switch halts all new orders immediately regardless of which component triggers it; even a partially degraded system cannot submit new orders.
- Fat-finger, rate-loop, and daily-loss scenarios are caught before any network call to a broker.
- Risk gate idempotency prevents double-submission on redelivery.
- Strategy `risk_overrides` that attempt to loosen limits are rejected by the gate validator at strategy initialization time, not at order submission time.

**Negative:**
- The risk gate is a synchronous chokepoint: a slow or stuck gate blocks all order submission. The gate implementation must be fast, well-tested, and never block on I/O.
- Adding a new check to the gate requires careful review; a bug in the gate that rejects valid orders halts all trading, not just the buggy orders.
- The kill switch halts new orders but does not close open positions. During an event that requires immediate position exit, operators must take a separate deliberate action. This is a feature, not a bug, but requires operators to be trained on the distinction.

**Neutral:**
- Because v1 is a small trusted group, the gate's limits can be simple per-user configuration rather than complex multi-tenant policy hierarchies. This simplicity is intentional and should be defended against premature generalization.
- The gate's idempotency key store (processed fill IDs) must be persisted or sized appropriately to survive a process restart. A restart that clears the seen-set could allow a redelivered intent to double-submit.

## Alternatives Considered

### Option A: Risk Checks in Each Order Entry Point
Implement risk checks inside the strategy runtime intent handler and separately inside the UI order handler. Each entry point validates independently.

Not chosen because: duplicated validation logic diverges. When a new check is added (e.g., trust tier gate), both implementations must be updated in sync. One missed update creates a bypass. Coordinated duplication is not a safety guarantee.

### Option B: Risk as a Separate Microservice
Run the risk gate as a separate network service. Every order intent makes an HTTP or NATS request to the risk service before routing to execution.

Not chosen because: a network call introduces a failure mode (risk service slow or unavailable) that would block all order submission. It also introduces a time window between the risk check and the order submission during which state could change (position updates, kill switch transitions). The synchronous in-process gate eliminates both failure modes.

### Option C: Advisory Risk Warnings (Non-Blocking)
Emit risk warnings as events but allow orders to proceed. Operators can monitor and intervene.

Not chosen because: advisory systems fail exactly when they are needed most. A strategy in a runaway loop does not pause for warnings. The kill switch must be mechanically enforced, not advisory.

## References

