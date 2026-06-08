# 04 — Strategy System

## Three front doors, one room

The visual builder, the JSON API, and the MCP server are **front doors that all produce the same
artifact**: a versioned **strategy definition document**.

```
Visual Builder (n8n-style) ─┐
JSON Strategy API ──────────┼──▶  Strategy Definition (JSON)  ──▶  Validator  ──▶  Runtime
MCP Server ─────────────────┘
```

- The **visual editor** serializes its node graph to the strategy definition JSON.
- The **JSON API** accepts that JSON directly.
- The **MCP server** lets an agent author and apply the same JSON via tool calls
  (see [08-mcp-server.md](./08-mcp-server.md)).

Three doors, one room. Therefore the **strategy definition format is the most important contract
in the system after the event schema** — it is what all three interfaces target and what the
runtime executes. It is also effectively irreversible: once users have built strategies in it,
changing it breaks their work. **Version it from event zero.**

## The asset model: one strategy instance per asset

A **strategy definition** is asset-class-scoped (it declares what kind of data it needs) but is
**not** pre-bound to a specific instrument. A **strategy instance** is the runtime binding of a
definition to exactly **one instrument**.

The UI flow reflects this:

```
User clicks an asset (e.g. BTC-USDT on Coinbase)
  → selects a strategy definition from their library
  → clicks "Initialize"
  → a strategy instance is created and started in the runtime for that instrument
```

There is no "run this strategy on a list of assets at once" action from the UI. If a user wants
EMA-cross running on both BTC-USDT and AAPL, they click each asset separately and initialize the
strategy on each. This produces two independent instances, each with its own `WorldState`,
each routing intents through the risk gate independently.

This keeps the UX mental model simple (one asset = one active strategy decision point) while the
underlying runtime is still multi-instance capable.

## Strategy definition format (sketch)

A strategy is a **versioned graph of nodes**: data sources → indicators/conditions → signals →
order actions. Explicit declared inputs let the runtime build the right subscriptions.

```json
{
  "strategy_id": "ema_cross_v1",
  "definition_version": "1.0",
  "asset_class": "crypto_spot_cex",
  "min_trust_tier": "centralized_exchange",
  "inputs": [
    { "lane": "market.bars.1m", "instrument": "$bound_at_init" },
    { "lane": "features.technical", "instrument": "$bound_at_init",
      "features": ["ema_7", "ema_21"] }
  ],
  "nodes": [
    { "id": "n1", "type": "condition",
      "expr": "feature('ema_7') > feature('ema_21')" },
    { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }
  ],
  "actions": [
    { "on_signal": "long", "type": "place_order",
      "order": { "side": "buy", "size_mode": "fixed", "size": "0.01" } }
  ],
  "risk_overrides": { "max_position": "0.5" }
}
```

Notes:
- `$bound_at_init` is resolved to the specific instrument when the user initializes the strategy
  on an asset. The definition is reusable; the instance is instrument-specific.
- `asset_class` scopes which instruments this definition is valid for (e.g. a bond strategy
  cannot be initialized on a crypto spot asset).
- `min_trust_tier` lets the strategy refuse to act on data dirtier than it tolerates.
- `risk_overrides` may **tighten** but never **loosen** the global risk gate (see
  [05-execution-and-risk.md](./05-execution-and-risk.md)).

## Asset class coverage

The strategy system is designed to be **asset-class-agnostic at the runtime level**. The
instrument metadata table and the asset-specific data payloads carry all the per-class
differences. This means the runtime, risk gate, and strategy definition format do not need to
change as new asset classes are added — only collectors, payload types, and metadata rows change.

**v1 MVP scope:** Coinbase (crypto spot CEX) and Alpaca (equities) only.

**Target asset classes** the system must be architecturally capable of supporting (not all in
v1, but no redesign should be required to add them):

| Asset Class | Notes |
|-------------|-------|
| Equities | Common stocks, REITs, ADRs — Alpaca for MVP |
| ETFs & Funds | OHLCV + NAV data; leveraged/inverse mechanics |
| Crypto Spot (CEX) | Coinbase for MVP; Kraken and others later |
| DEX / AMM | Uniswap v2/v3, Curve — AMM price-impact mechanics |
| Futures (expiring) | Roll schedules, continuous series |
| Perpetual Swaps | Funding payments, mark-price liquidation |
| Options | IV surface, greeks, early exercise |
| Bonds & Fixed Income | Scheduled cash flows, accrued interest, YTM |
| FX | Currency pairs, overnight swap rates |
| NFTs | Non-fungible positions, listing-based fills |
| Prediction Markets | Binary contracts, oracle resolution |

Strategy definitions declare their `asset_class`; the validator rejects initialization on an
incompatible instrument. Lanes and payload types are extended per asset class; the core runtime
loop does not branch on asset class.

## The runtime

A strategy runtime instance:

1. Loads a definition + the bound instrument (set at initialization time).
2. Declares demand for the lanes/instruments in `inputs`, resolved to the bound instrument.
3. Subscribes to the **canonical** bus events (never the UI feed).
4. Maintains a local `WorldState` so the strategy does not manually join timestamps across tables.
5. On each event, calls the strategy; emits order **intents** which flow through the risk gate.

```rust
pub trait Strategy {
    fn on_event(&mut self, event: &WorldEvent, world: &mut WorldContext) -> StrategyResult;
}
```

`WorldContext` exposes:

```rust
world.now();                                 // real live (no wall-clock reads in strategies)
world.latest_bar(instrument, timeframe);
world.latest_orderbook(instrument);
world.feature(instrument, "ema_7");
world.recent_events(instrument, Duration::hours(1));
world.position(instrument);
world.open_orders(instrument);
world.place_order(order_request);            // -> risk gate -> execution
```

## Manual + automated coexist on the same asset

Manual orders (from the UI) and strategy order intents flow through the **same** execution path
and the **same** risk gate. That's what lets a user trade `BTC-USDT` by hand while a strategy also
runs on `BTC-USDT` — both are just order intents hitting one chokepoint. The runtime never has a
private path to the broker.

## Determinism caveats to test

- Wall-clock reads inside strategies are forbidden — use `world.now()`.
- Random seeds, if any, must be part of the definition and recorded.
- Floating point in indicator math is acceptable for *features* but feature values are versioned
  and recorded, so replay sees the exact values live saw.
