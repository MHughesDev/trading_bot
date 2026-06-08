# ADR-0006: Three-System Broker Architecture — Coinbase, Alpaca, market_simulator

**Status:** Accepted
**Date:** 2026-06-08
**Deciders:** Platform team

## Context

Two open questions from the platform design were gating all execution and data collection code:

**Q-1 (Real money or paper first?):** Building directly against live broker APIs without a proven risk gate, kill switch, and reconciliation loop is dangerous. A bug in the strategy runtime or execution engine that has never been exercised against real fills could drain an account in minutes.

**Q-2 (Which broker(s)/venue(s) for v1?):** The platform must handle both crypto and equities. These asset classes have different trading hours, settlement rules, halt behavior, and market structure. The choice of broker determines the API surface, rate limits, market-data granularity, and execution semantics for the entire v1 platform.

Additionally, the platform needs a backtest execution capability that is both realistic and clearly separated from live execution to prevent accidental cross-contamination.

These three decisions (paper-first execution, broker selection, and backtest execution ownership) are tightly coupled: the paper broker choice affects the data collection design, the backtest adapter design depends on the data archive format, and all three share the same order-routing interface.

## Decision

The platform operates three parallel execution systems and two parallel data systems, all sharing a common interface via `crates/venue-router`:

**Execution systems:**
1. **Coinbase Advanced Trade** — live execution for crypto assets. Used only after the risk gate, kill switch, and reconciliation are proven against paper.
2. **Alpaca paper account** — paper execution for all assets (crypto and equities) during development and validation.
3. **market_simulator** (`github.com/MHughesDev/market_simulator`) — backtest execution. External library, not owned by this repo. Accepts strategy definitions and Arrow IPC event data; returns per-trade records and performance metrics.

**Market data systems:**
1. **Kraken WebSocket** — crypto market data feed (OHLCV bars, trades, order book). Chosen as the primary crypto data source because its API is well-documented and its collector design intentionally differs from Coinbase's, proving the venue abstraction.
2. **Alpaca data feed** — equity market data (OHLCV bars, trades).

**Routing:** `crates/venue-router` resolves `(AssetClass, DataType, ExecutionMode)` to the appropriate venue adapter at runtime. The strategy runtime and risk gate are unaware of which physical venue handles a given order or data subscription.

This decision resolves Q-1 (paper first, flip to live only after proving the safety systems) and Q-2 (Coinbase for crypto execution, Alpaca for equity execution, Kraken for crypto data).

## Rationale

**Paper first (Q-1):** The same execution interface (`crates/execution`) backs both paper and live adapters. Flipping from paper to live is an adapter swap in the venue-router configuration, not a rewrite. Running against Alpaca paper for an extended period proves the reconciliation loop, kill switch mechanics, idempotency of fills, and order state machine against real API responses without real monetary risk.

**Coinbase + Alpaca (Q-2):** Coinbase Advanced Trade is the leading institutional crypto exchange with a stable WebSocket API and a well-documented REST execution interface. Alpaca provides both equity execution and equity market data through a single brokerage relationship, simplifying account management. Using Alpaca's paper account for the validation phase aligns the paper environment as closely as possible with the live equity environment.

**Kraken for crypto data (separate from execution):** Coinbase's WebSocket market data API has limitations for reliable sub-minute streaming. Kraken's API is better suited as the primary crypto market data source for the v1 MVP. Using Kraken for data and Coinbase for execution also proves the venue abstraction: the system demonstrates that data source and execution venue can be different providers for the same asset class, which is necessary for the platform's long-term multi-venue scalability.

**market_simulator for backtesting:** Building a fill simulator in this repo would duplicate engineering effort and would likely be less accurate than a purpose-built simulation engine. `market_simulator` is a production-grade, event-driven fill simulation engine. This repo's responsibility is to archive raw events correctly (in the format market_simulator expects: Arrow IPC), not to own fill simulation mechanics.

## Consequences

**Positive:**
- Live money exposure is gated behind a proven paper execution period. The risk gate and kill switch must pass paper validation before Coinbase live execution is enabled.
- All three execution modes share one interface; strategy and risk code never branch on execution mode.
- Venue-router makes future venue additions (Binance, Interactive Brokers) a new adapter crate, not a change to existing logic.
- Backtest fill simulation is owned by a dedicated external library, freeing this repo from maintaining a fill model.
- Two crypto data/execution venue pairs (Kraken data, Coinbase execution) prove the abstraction before the second asset class is added.

**Negative:**
- Three broker relationships to manage: Coinbase, Alpaca, and Kraken API credentials, rate limits, and API change notifications.
- The market_simulator interface is an external dependency. If market_simulator's Arrow IPC schema changes, the adapter crate must be updated. The platform does not control the release cadence of market_simulator.
- Kraken WebSocket collector and Coinbase execution adapter have different API surface areas. Two sets of integration tests and two reconnection/backoff implementations.
- Alpaca paper fills do not perfectly simulate live equity fills (slippage, queue position, partial fills are modeled simply). Backtest results at 1-minute bar granularity are directional, not precise.

**Neutral:**
- The v1 MVP operates on 1-minute OHLCV bars for both crypto and equity data. Sub-minute and order-book lanes are architected and have valid schema types but are not populated by v1 collectors.
- Coinbase live execution is the eventual target for crypto; it is treated as a configuration-controlled swap, not a future architectural decision.
- Kraken is the designated "next crypto venue after MVP" to add a second collector that proves the abstraction without rearchitecting.

## Alternatives Considered

### Option A: Single Broker (Alpaca for Everything)
Use Alpaca for both crypto and equity execution and data, eliminating the Coinbase and Kraken integrations.

Not chosen because: Alpaca's crypto execution offering is less established than Coinbase's for institutional-grade crypto trading. The platform's long-term value depends on supporting the dominant crypto execution venue. Using only Alpaca also fails to prove the multi-venue abstraction.

### Option B: Build a Fill Simulator In This Repo
Implement a backtest replay engine and fill simulator inside this codebase rather than delegating to market_simulator.

Not chosen because: fill simulation is a specialized domain (order book mechanics, queue position modeling, slippage estimation) that benefits from dedicated focus. Duplicating this in the trading platform repo creates maintenance burden and likely results in a less accurate simulator than market_simulator's dedicated implementation.

### Option C: Live-First Development Against Real Broker APIs
Build directly against Coinbase live execution from the start, using small test positions.

Not chosen because: the risk gate, kill switch, and reconciliation loop must be proven before live money is at risk. The sequence matters: paper proves the safety systems, then live execution is enabled. This is not merely a preference — it is the explicit outcome of the Q-1 decision process.

## References

- [spec/10-open-questions.md](../../refactor_reference_docs/spec/10-open-questions.md) — Q1 (paper first, decided), Q2 (Coinbase + Alpaca, decided), Q9 (market_simulator, decided)
- [spec/07-storage-and-replay.md](../../refactor_reference_docs/spec/07-storage-and-replay.md) — market_simulator integration, Arrow IPC export, backtest fidelity caveat
- [spec/05-execution-and-risk.md](../../refactor_reference_docs/spec/05-execution-and-risk.md) — paper vs live execution posture, reconciliation requirement before resuming
- [spec/03-data-engineering.md](../../refactor_reference_docs/spec/03-data-engineering.md) §11 — data granularity model, 1-minute bar MVP constraint
