# 11 — Roadmap

Build order is chosen so the **irreversible foundations and money-safety** come first, and the
fun streaming work stands on solid ground. The expensive-to-get-wrong things are decided before
anything depends on them.

## Phase 0 — Foundations (the irreversible core)

> Nothing else starts until this exists. Both data engineers and the architect point here.

1. **`domain` crate**: event envelope, v1 payloads (trade, quote, order-book, bar), the four
   timestamps + semantics, `Price`/`Size` newtypes (no `From<f64>`), `TrustTier`.
2. **Instrument metadata model + table** (the unsung hero of scalability).
3. **Strategy definition format frozen at `1.0`** (gates all three front doors — see Q3).
4. **Raw event archive design** (ground truth, append-only).

## Phase 1 — Spine + one of each

5. One main binary: **Axum REST API + auth**.
6. **NATS JetStream** event fabric with lane naming + the `quarantine` lane.
7. **One crypto collector** → normalize → publish; gap detection; quarantine on schema failure.
8. **Storage writers** → ClickHouse (bars/trades) + Postgres (core) + Parquet raw archive;
   batching + dedup.
9. **Bar builder** (1s + 1m) as a pure function (same code live + replay).

## Phase 2 — Money safety (before any automation touches orders)

10. **Risk gate + kill switch** (single chokepoint, idempotent).
11. **Paper execution adapter** behind the execution interface.
12. **Manual order submission** (UI → REST → risk gate → paper execution).
13. **Reconciliation loop** (startup + on-fill + 30s sweep) against the paper broker.

## Phase 3 — See it

14. **UI streaming gateway** (throttled, frontend-shaped, lossy views; snapshot-on-connect).
15. **React UI**: live data panels (chart, order book) + manual trade panel.
16. **Demand Manager** wired to UI subscriptions.

## Phase 4 — Strategies

17. **Feature engine** for a small indicator set (EMA/RSI to start), versioned, pure function.
18. **Strategy runtime**: user clicks an asset → initializes a strategy → runtime instance starts
    bound to that instrument. Emits intents through the risk gate. One instance per
    instrument per user (MVP UX constraint).
19. **market_simulator adapter** (`crates/market-simulator-adapter`): export raw archived events
    as Arrow IPC, submit Run Requests to market_simulator, return results. Wire to the REST
    backtest endpoints. **No fill simulator is built in this repo.**
20. **Multi-instance runtime**: several strategy instances active concurrently (one per initialized
    asset), each with its own `WorldState`, all funneling intents through the single risk gate.

## Phase 5 — Authoring front doors

21. **JSON strategy API** (create/validate/apply/stop against the frozen format, bound to a
    specific instrument at apply time).
22. **Visual n8n-style builder** (serializes to the same JSON; opens from the instrument detail
    view's strategy panel).
23. **MCP server** (thin; authors/applies the same JSON; no order tool; `run_backtest` delegates
    to the market_simulator adapter).

## Phase 6 — Second asset class proves the abstraction

24. **One equity collector + equity broker adapter** (Alpaca, built deliberately differently from
    Coinbase), with hours/halt/auction handling in instrument metadata.
25. **Dashboard P&L breakdown** by asset class: the multi-venue account model works across
    Coinbase + Alpaca; win rate and P&L are shown per asset class.

## Later (direction, not v1)

Second-level bars and order-book level streaming (when venue APIs support it reliably);
DEX/AMM collector; perpetuals; options; bonds; FX; NFTs; prediction markets; news/social
exogenous-signal plane; AI/ML model inference port; multi-user scaling; on-chain data with reorg
handling. Each new asset class is *a collector + a payload type + metadata rows + an
asset-spec in docs* — never a redesign of the runtime or risk gate.

The market_simulator's engine coverage expands independently; the adapter in this repo extends
to match as new price-formation mechanics are supported.

## The discipline that makes the roadmap real

- Don't let breadth paralyze Phase 0. Build for crypto + stocks; let the abstraction carry the
  rest *later*.
- Don't let optimism skip tests: every decided mechanism gets an adversarial test (see Q in
  [10-open-questions.md](./10-open-questions.md)).
- Don't let deliberation replace decision: pick the broker/venue (Q2), freeze the format (Q3),
  answer paper-vs-real (Q1), and start the `domain` crate.
