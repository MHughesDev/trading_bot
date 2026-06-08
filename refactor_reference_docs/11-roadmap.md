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
18. **Strategy runtime**: run ONE strategy on ONE asset, consuming canonical events, emitting
    intents through the risk gate.
19. **Backtest replay** from stored events through the same builders + runtime + paper execution.
20. **Multi-asset, multi-strategy** runtime.

## Phase 5 — Authoring front doors

21. **JSON strategy API** (create/validate/apply against the frozen format).
22. **Visual n8n-style builder** (serializes to the same JSON).
23. **MCP server** (thin; authors/applies the same JSON; no order tool).

## Phase 6 — Second asset class proves the abstraction

24. **One equity collector + equity broker adapter**, built deliberately differently from crypto,
    with hours/halt/auction handling in metadata. If the schema + metadata survive both asset
    classes unchanged, the abstraction is real.

## Later (direction, not v1)

Order-book reconstruction depth features; news/web scraper activity streams; AI event extraction;
social sentiment; on-chain/DEX data (with reorg handling + `onchain_tentative` trust tier);
options; ETFs; bond yields; Parquet/DataFusion research layer; MessagePack/Protobuf on the wire;
multi-user scaling. Each new asset class is *a collector + a payload type + metadata rows* —
never a redesign.

## The discipline that makes the roadmap real

- Don't let breadth paralyze Phase 0. Build for crypto + stocks; let the abstraction carry the
  rest *later*.
- Don't let optimism skip tests: every decided mechanism gets an adversarial test (see Q in
  [10-open-questions.md](./10-open-questions.md)).
- Don't let deliberation replace decision: pick the broker/venue (Q2), freeze the format (Q3),
  answer paper-vs-real (Q1), and start the `domain` crate.
