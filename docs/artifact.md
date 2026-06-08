# The Artifact

Fill this in before anything else. It does not need to be complete — it needs to be honest.
The goal is to get the most important things out of your head and into a form that can be designed against.

Each section feeds a downstream artifact: research briefs, plans, specs, or ADRs.

---

## What Are We Building?

An event-driven, local-first trading platform written in Rust. It ingests market data from multiple venues, runs user-defined strategies through a single risk gate, executes orders against broker APIs (live, paper, and backtest), and streams live views to a React frontend. It is a data platform first and a trading application second.

---

## Who Uses It?

A small, trusted group of users (friends and operators) running on a private network. Each user can initialize strategies on instruments, submit manual orders, view live positions and charts, and run backtests. The system is not multi-tenant in the public-cloud sense — it is a trusted shared workspace.

---

## What Problem Does It Actually Solve?

Python trading systems organically accumulate structure debt that makes correctness guarantees impossible to reason about — especially around money: duplicate order submissions, stale position state, and lookahead bias in backtests. This rewrite delivers a system where the compiler enforces money-type safety, every order is guaranteed to pass one risk gate, and live and backtest behavior are provably identical.

---

## What Does Good Look Like?

- SC-1: No f64 ever touches a price or size — the compiler enforces it (no `From<f64>` on `Price`/`Size`)
- SC-2: Every order (manual or automated) passes through `crates/risk` before reaching any broker — no bypass path exists
- SC-3: The same builder code runs live and in replay — identical strategy results are structurally guaranteed
- SC-4: `available_time` ordering makes lookahead bias structurally impossible in backtests
- SC-5: Adding a new asset class requires only a collector + payload type + instrument metadata rows — no core code changes
- SC-6: The system halts on position/balance divergence before submitting new orders (reconciliation safety)
- SC-7: All money-mutating paths are idempotent — broker reconnects and JetStream redeliveries are safe

---

## What Would Make This Fail?

- FM-1: A strategy sees future data (lookahead bias) — product — backtests are unreliable
- FM-2: An order bypasses the risk gate — technical — real money lost to unchecked positions
- FM-3: Duplicate order submission on reconnect — technical — double positions
- FM-4: Internal position view diverges from broker without halting — operational — trades on stale state
- FM-5: f64 arithmetic error on price/size — technical — silent money calculation errors
- FM-6: Data pipeline keeps running with no active consumers — operational — resource waste and operational confusion

---

## What Don't We Know Yet?

Several consequential questions remain open and gate downstream code. See [open-questions.md](./open-questions.md) for the full register. The most impactful unresolved items are:

- **Q-3** (strategy definition format v1.0 freeze) — gates all three front doors (visual builder, JSON API, MCP server)
- **Q-4** (capital/liability model) — gates any real-money switch
- **Q-5** (auth model for trusted group) — local + trusted, minimal per-user scoping not yet pinned
- **Q-6** (backtest execution fidelity) — slippage/partial-fill modeling assumptions in `market_simulator` not yet documented
- **Q-7** (watermark defaults per source) — 2s default for liquid CEX; per-instrument metadata tuning pending real feeds
- **Q-8** (retention policy) — how long raw events live in different storage tiers not yet decided

Q-1 (real money or paper first) and Q-2 (broker/venue selection for v1) are resolved. See the open-questions register for their resolutions.

---

## What Are We Deliberately Not Building?

- Twelve microservices (the scope does not require them)
- Multi-tenant isolation (this is a trusted shared workspace, not a public product)
- Kafka (over-built for this scope; NATS JetStream is the bus)
- Every asset class at once (v1 proves the abstraction with two; the rest are additive extensions)
- A custom fill simulator (delegated entirely to the external `market_simulator` dependency — this repo owns only the adapter)
- Custom auth infrastructure beyond minimal trusted-group auth

---

## What Are We Working Within?

- **Language and tooling:** Rust, Cargo workspace
- **Team:** One small team
- **Hosting:** Local, private network
- **Frontend:** Existing React frontend (`frontend/`) to be re-pointed at the new Rust API/WS contracts — not rewritten
- **Behavior reference:** Existing Python system in `legacy_python/` remains runnable until Phase 7 for A/B behavior checks
- **Backtest engine:** `market_simulator` (`github.com/MHughesDev/market_simulator`) is an external dependency — this repo owns the adapter (`crates/market-simulator-adapter`), not the replay engine or fill simulator
