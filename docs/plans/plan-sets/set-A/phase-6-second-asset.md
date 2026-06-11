---
Type: Formal
Status: Pending
Derived From: COMP-001, COMP-002, ADR-0006, SC-5
Note: Canonical executable plans live in docs/plans/. This copy is the traceable documentation record. On any conflict, [deleted - see Phase 7]/ wins.
---

# Phase 6 — Second asset class proves the abstraction (equities)

> **Self-contained execution doc.** You need only: this file, [`../architecture.md`](../architecture.md),
> and the specs — especially
> [`../specs/DATA-002-instrument-metadata.md`](../specs/DATA-002-instrument-metadata.md)
> (instrument metadata),
> [`../specs/COMP-002-execution-and-risk-gate.md`](../specs/COMP-002-execution-and-risk-gate.md)
> (stocks vs crypto), and
> [`../specs/COMP-001-data-quality-and-ingestion.md`](../specs/COMP-001-data-quality-and-ingestion.md)
> §8 (trust tiers).

## Phase goal

After this phase, **equities flow through the exact same schema, lanes, storage, risk gate, runtime,
and backtest as crypto** — with all asset-class differences (trading hours, halts, auctions,
settlement) living in **instrument metadata and the broker adapter**, not in core code. This is the
proof that the abstraction is real: if the event schema + metadata survive both asset classes
unchanged, the "new asset = a collector + a payload type + metadata rows" promise holds.

## Prerequisites

- Phases 1–4 complete (spine, money safety, UI optional, strategies/backtest). The crypto collector
  and paper execution already exist and define the patterns to mirror — **and to deliberately
  diverge from** so the abstraction is tested, not papered over (per
  [`../../research/OPEN_QUESTIONS.md`](../../research/OPEN_QUESTIONS.md) Q2).
- **Decision gate Q2 (equity half — resolved):** Use **Alpaca** for both equity market data and
  equity paper execution (all assets/domains). Alpaca provides a WS data feed for equities
  (`crates/collectors/src/equity/alpaca_data.rs`) and a paper account for order execution
  (`crates/execution/src/alpaca.rs` — already built in Phase 2, now exercised with equity
  instruments). The Coinbase live equity adapter (`crates/execution/src/coinbase.rs`) is **post-
  Phase-6 scope** — the `Broker` interface is ready for it, but Phase 6 proves the abstraction with
  paper first. Add the Alpaca equity data route to `crates/venue-router` routing table.
- `legacy_python/execution/alpaca_util.py`, `legacy_python/orchestration/alpaca_universe_*` contain
  prior equity integration — read for parity.

## Invariants this phase must respect

- **Core code does not branch on asset class.** No `if asset_class == Equity` in the runtime, risk
  gate, storage, or builders. Differences live in instrument metadata + the broker adapter. If you
  find yourself adding an asset-class branch to core, the abstraction is leaking — fix the metadata
  model instead.
- **Freshness watchdog reads trading hours.** An equity's normal session close must not false-alarm;
  a true feed outage during the session must alarm. (Already built in Phase 2 reconciliation — this
  phase exercises it with real equity hours.)
- **Halts respected by the gate.** The risk gate rejects orders against a halted instrument using the
  instrument's `halt_behavior`.

---

## Tasks

### P6-T01 — Equity instrument metadata
- **Goal:** Seed equity instruments with correct sessions/auctions/halt policy/precision/trust tier.
- **Files:** equity rows via `crates/storage/src/postgres/instruments.rs`; if needed, extend
  `TradingSchedule`/`HaltPolicy` in `crates/domain/src/instrument.rs` **without** breaking crypto.
- **Context:** Per [`../specs/DATA-002-instrument-metadata.md`](../specs/DATA-002-instrument-metadata.md)
  §instrument metadata and
  [`../specs/COMP-002-execution-and-risk-gate.md`](../specs/COMP-002-execution-and-risk-gate.md)
  §stocks vs crypto: equities have session+auction `trading_hours`, can halt (`halt_behavior`), quote
  precision 2dp, lot size rules, `trust_tier = Regulated`. Crypto stays 24/7. Any `TradingSchedule`
  extension must remain expressive enough for both (24/7 and session) without a core code branch.
- **Acceptance:** an equity and a crypto instrument coexist in the metadata table; the schedule model
  expresses both; no core crate gained an asset-class branch.
- **Depends on:** Phase 0/1 (instrument model + storage).

### P6-T02 — Alpaca equity data collector (built deliberately differently)
- **Goal:** A second collector using the **Alpaca WS data feed** for equities, intentionally
  structured differently from the Kraken crypto collector, publishing the **same** `domain` payloads
  on the **same** lanes.
- **Files:** `crates/collectors/src/equity/{mod,alpaca_data}.rs`, `apps/collector-equity/src/main.rs`.
- **Context:** Per [`../../research/OPEN_QUESTIONS.md`](../../research/OPEN_QUESTIONS.md) Q2: build the second collector
  deliberately differently (Alpaca uses REST+WS with different auth/snapshot model vs Kraken's WS v2)
  so the lane/metadata abstraction is *proven*. It normalizes Alpaca's equity feed shapes into the
  same `TradePayload`/`QuotePayload`/`OrderBookPayload`/`BarPayload` on the same `market.*` lanes,
  with the same quarantine + gap-detection machinery from `crates/collectors`. It must be hours/halt
  aware (publishing halt/session state into metadata-relevant signals). Add the Alpaca equity routing
  rule to `crates/venue-router` so this collector also starts **only on demand**. Read
  `legacy_python/execution/alpaca_util.py` for Alpaca API parity (do not import).
- **Acceptance:** when demand is declared, `collector-equity` publishes live equity trades/quotes on
  the same `market.*` lanes as Kraken crypto; the storage writer, bar builder, and feature engine
  consume them with **zero changes**; schema failures quarantine; gaps emit `gap.detected`; collector
  does not start on system init.
- **Depends on:** P6-T01, Phase 1 (collector framework + builders + venue-router).

### P6-T03 — Alpaca paper adapter exercised for equities + session/halt wiring
- **Goal:** Exercise the existing **Alpaca paper account adapter** (`crates/execution/src/alpaca.rs`,
  built in Phase 2) with equity instruments, and confirm sessions/halts are correctly enforced.
- **Files:** `crates/execution/src/alpaca.rs` (extend for equity session/auction semantics if
  needed); instrument metadata rows from P6-T01; keep `Broker` trait unchanged.
- **Context:** Per [`../specs/COMP-002-execution-and-risk-gate.md`](../specs/COMP-002-execution-and-risk-gate.md)
  §stocks vs crypto: session/auction order acceptance, halt rejection, and settlement/shorting
  constraints live in the **adapter and metadata** (not core). The Alpaca paper adapter handles both
  crypto and equity orders — this task proves it for equity. Coinbase live equity trading is
  **post-Phase-6 scope**; the `coinbase.rs` stub is ready for it but not implemented here. The risk
  gate already reads `halt_behavior`/`trading_hours` from Phase 2 — confirm those checks fire for
  equity instruments.
- **Acceptance:** an equity order outside session is rejected by the gate; an order against a halted
  equity is rejected; a valid in-session equity order fills via the Alpaca paper adapter and updates
  the position through the **same** execution/reconciliation path as crypto; no core crate changed.
- **Depends on:** P6-T01, Phase 2 (execution + risk + reconciliation).

### P6-T04 — Cross-asset abstraction proof test
- **Goal:** Prove crypto and equity share one path and that the schema/metadata survived unchanged.
- **Files:** extend `tests/` with a cross-asset test; reuse `tests/strategy_end_to_end.rs` patterns.
- **Context:** Run the same strategy definition (via `$each`) across a crypto and an equity
  instrument; confirm both ingest → build → feature → runtime → risk gate → paper fill through
  identical core code, with only metadata/adapters differing. Confirm the freshness watchdog respects
  the equity session close (no false alarm) and alarms on a simulated mid-session outage.
- **Acceptance:** one definition trades both asset classes through identical core; no core crate has
  an asset-class branch; freshness respects equity hours; the event schema and metadata model were
  not changed to accommodate equities (only rows/adapters added).
- **Depends on:** P6-T02, P6-T03, Phase 4 (runtime/backtest).

---

## Phase exit criteria

- [ ] The Alpaca equity data collector publishes on the same `market.*` lanes as Kraken crypto;
      downstream consumers (storage, builders, features) consume it unchanged; starts on demand only.
- [ ] The Alpaca paper adapter handles equity orders; sessions/halts are honored via metadata +
      adapter, not core branches; Coinbase live equity adapter is a stub (post-Phase-6 scope).
- [ ] One strategy definition runs across crypto + equity through identical core code.
- [ ] The freshness watchdog respects equity trading hours and alarms on a true outage.
- [ ] The abstraction is proven: the event schema + instrument-metadata model were **not** redesigned
      to add equities — only metadata rows + an adapter + a payload-compatible collector were added.
