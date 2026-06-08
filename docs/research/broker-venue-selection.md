# Broker and Venue Selection for v1

**Question:** Which brokers and venues should the platform use for live execution, paper execution, backtest execution, and market data in v1?
**Status:** Complete
**Outcome:** Coinbase Advanced Trade for live execution, Alpaca paper account for paper execution, market_simulator for backtest execution, Kraken WS for crypto market data, and Alpaca data feed for equity market data — all adopted with on-demand pipeline startup via venue-router.
**ADR(s):** ADR-0006 (broker and venue selection), ADR-0011 (on-demand pipeline startup)

---

## Method

Evaluation based on the two gating open questions from `spec/10-open-questions.md`:

- **Q-1:** Real money or paper first?
- **Q-2:** Which broker(s)/venue(s) for v1?

Each candidate was evaluated against the system's execution modes (live, paper, backtest), asset classes (crypto, equity), and data-pipeline requirements (market data for strategy runtime and storage). The backtest engine question (Q-9) was also resolved here because it directly determines one of the three execution adapters.

Alternatives evaluated:
- Using a single broker for all modes and all asset classes
- Building a custom backtest/fill-simulation engine inside this repository
- Using Coinbase as both execution broker and crypto market-data source
- Using Alpaca for both crypto and equity

## Findings

### Q-1: Real money or paper first? → Paper first

Running against real broker APIs before the risk gate, kill switch, and reconciliation are proven is not acceptable for a money-handling system. Paper execution allows the full execution adapter, order state machine, fill handling, and position reconciliation to be built and validated against a live broker API without capital at risk. Switching to live execution is an adapter swap — the same `Broker` trait is implemented by both the paper adapter and the live adapter. No rewrite is required when the risk layer is proven.

**Resolution:** Paper/simulated execution via the Alpaca paper account first. Flip to live broker APIs (Coinbase for crypto, Alpaca live for equity) only after the risk gate, kill switch, and reconciliation pass adversarial testing.

### Q-2: Broker and venue selection

The system requires three distinct execution modes and two distinct market-data sources. A single broker cannot satisfy all three modes or both asset classes cleanly. The selections below minimize operational overlap while proving the venue abstraction:

#### Live execution: Coinbase Advanced Trade (REST + WS)

Coinbase Advanced Trade provides REST order submission and WebSocket order updates for crypto assets. It is the target live broker for crypto. All assets and all crypto domains route through Coinbase when the system is in live mode. The Coinbase adapter is built in `crates/execution/src/coinbase.rs` and implements the shared `Broker` trait.

Alpaca is the natural live broker for equities; it uses the same adapter interface. Coinbase is crypto-only for live execution.

#### Paper execution: Alpaca paper account (all assets, all domains)

Alpaca provides a paper trading account that mirrors its production API surface. The paper account accepts orders for both equities and, via Alpaca's crypto trading interface, crypto assets. This allows the full execution path — order submission, acknowledgement, fill notification, position update — to be validated without real capital for both asset classes from a single paper adapter.

**Alternatives considered:**
- *Using a Coinbase sandbox:* Coinbase Advanced Trade does not provide a publicly available paper/sandbox environment equivalent to Alpaca's. Alpaca paper is the more complete paper execution environment.
- *Separate paper adapters per broker:* Adds implementation cost before the risk layer is proven. One paper adapter that covers both asset classes is sufficient for v1.

#### Backtest execution: market_simulator (github.com/MHughesDev/market_simulator)

This repository does not own a fill simulator or replay engine. Building a correct fill model (slippage, queue position, partial fills) is a well-scoped problem that belongs in a dedicated library, not in the trading platform itself. `github.com/MHughesDev/market_simulator` is the external library used for all backtest fill simulation.

The adapter (`crates/market-simulator-adapter`) exports raw archived events as Arrow IPC, submits a `RunRequest` to market_simulator, and translates results back to domain types. No replay loop, no fill model, and no `available_time` ordering logic live in this repository.

**Alternative considered:**
- *Building a custom backtest engine in this repo:* Rejected. The backtest engine is a correctness-critical component (results must be trustworthy enough to inform trading decisions). Owning it means owning the fill model, the lookahead-prevention logic, and the slippage model. Delegating to a dedicated library with its own test suite reduces the attack surface.

#### Crypto market data: Kraken WS

Kraken provides a well-documented WebSocket API for crypto market data (trades, quotes, L2 order-book, tickers). The Kraken collector is built deliberately differently from the Coinbase execution adapter — using a separate collector crate (`crates/collectors/src/crypto/kraken.rs`) — to prove that the venue abstraction works across two independently implemented integrations.

**Alternative considered:**
- *Coinbase as market data source:* Coinbase could serve as both execution broker and crypto data source. However, having a single external dependency for both execution and data creates a single point of failure for the entire crypto pipeline. Kraken as market-data source with Coinbase as execution broker separates these concerns. The Kraken collector also provides a second integration proof for the collector abstraction.

#### Equity market data: Alpaca data feed

Alpaca provides a WebSocket data feed for US equities (trades, quotes, bars). The Alpaca equity collector (`crates/collectors/src/equity/alpaca_data.rs`) integrates with this feed. The equity collector must handle market-hours awareness and halt states (normal stock market close at 4pm must not trigger a stale-feed alarm).

### Venue routing and on-demand pipeline startup

The `crates/venue-router` crate resolves `(AssetClass, DataType)` to a `VenueId` at runtime using a config-driven routing table:

- `(Crypto, Any DataType, *)` → **Kraken** (market data)
- `(Equity, Any DataType, *)` → **Alpaca data feed** (market data)
- Execution routing is separate from data routing: live → Coinbase, paper → Alpaca paper, backtest → market_simulator

**Data pipelines start only on demand.** No collector spins up at system initialization. When a strategy instance or UI panel declares demand for a lane (via the Demand Manager), the Venue Router resolves the required venue and starts the appropriate collector. When demand drops to zero, the pipeline is stopped. This prevents idle collectors from consuming API rate-limit budget and simplifies failure isolation.

This is the resolution to Q-2 and directly informs ADR-0011.

## Recommendation

Adopt the following venue and broker assignments for v1:

| Mode | System | Crate |
|------|--------|-------|
| Live execution | Coinbase Advanced Trade (REST + WS) | `crates/execution/src/coinbase.rs` |
| Paper execution | Alpaca paper account (all assets) | `crates/execution/src/alpaca.rs` |
| Backtest execution | market_simulator (external) | `crates/market-simulator-adapter` |
| Crypto market data | Kraken WS | `crates/collectors/src/crypto/kraken.rs` |
| Equity market data | Alpaca data feed | `crates/collectors/src/equity/alpaca_data.rs` |
| Venue routing | `crates/venue-router` resolves at runtime | `crates/venue-router` |

Build order: paper execution first (Phase 2), live execution adapter after risk gate is proven, equity adapter in Phase 6 to prove the abstraction. All data pipelines start on demand only.

## References

- `/home/user/trading_bot/refactor_reference_docs/spec/10-open-questions.md` — Q-1 (real vs paper), Q-2 (broker/venue), Q-9 (backtest engine ownership)
- `/home/user/trading_bot/refactor_reference_docs/spec/01-architecture.md` — Demand Manager, on-demand pipeline lifecycle
- `/home/user/trading_bot/refactor_reference_docs/plans/00-master-plan.md` §3 — resolved decision gates for Q-1 and Q-2
- `/home/user/trading_bot/refactor_reference_docs/file-structure.md` — `crates/venue-router`, `crates/execution`, `crates/collectors` crate descriptions
- Coinbase Advanced Trade API: https://docs.cdp.coinbase.com/advanced-trade/docs/welcome
- Alpaca paper trading: https://docs.alpaca.markets/docs/paper-trading
- Alpaca market data: https://docs.alpaca.markets/docs/getting-market-data
- Kraken WebSocket API: https://docs.kraken.com/websockets-v2/
- github.com/MHughesDev/market_simulator — backtest fill simulation engine (external dependency)
