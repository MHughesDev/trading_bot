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

## The North Star (recorded 2026-09-15)

The platform above is the *substrate*. The thing being built on it is:

> **A highly capable, persistent quantitative research agent that turns research questions
> and market observations into reproducible experiments, rigorously evaluated trading
> algorithms, and accumulated knowledge.**

It is judged on **the quality of its research decisions, the reliability of its evidence,
and its ability to identify robust strategies — or correctly reject weak ones.** Every
candidate is evaluated out-of-sample, after trading costs, under stated execution
assumptions, against a benchmark. **Research produces reusable knowledge even when a
strategy fails** — a failure with a named class and a closed question is a successful run.

This reframes what "finished" means. A trading platform is finished when it trades
correctly. A research platform is finished when its *verdicts can be trusted*, which is a
claim about evidence and about the accounting of looks taken, not about uptime. The
distinction is the reason the ledger, the pre-registration and the gate funnel exist at
all, and it is the root every plan set from L onward derives from.

**Two clocks govern this artifact, and only one of them is engineering.** The *build clock*
is parallelisable and agent-executable. The *evidence clock* — history depth, forward-test
observation, and the calendar a strategy must survive — is not. See
[`plans/ROADMAP.md`](./plans/ROADMAP.md), which sequences the whole programme around that
distinction.

---

## What Does Good Look Like?

- SC-1: No f64 ever touches a price or size — the compiler enforces it (no `From<f64>` on `Price`/`Size`)
- SC-2: Every order (manual or automated) passes through `crates/risk` before reaching any broker — no bypass path exists
- SC-3: The same builder code runs live and in replay — identical strategy results are structurally guaranteed
- SC-4: `available_time` ordering makes lookahead bias structurally impossible in backtests
- SC-5: Adding a new asset class requires only a collector + payload type + instrument metadata rows — no core code changes
- SC-6: The system halts on position/balance divergence before submitting new orders (reconciliation safety)
- SC-7: All money-mutating paths are idempotent — broker reconnects and JetStream redeliveries are safe

Added 2026-09-15, for the research agent (the North Star above):

- SC-8: No trial runs anywhere in the system without a ledger row, and no look goes uncounted in `N_eff` — there is no door, at any permission level, that spends compute against market data unregistered
- SC-9: Every trial-spending run is pre-registered with a falsifier *before* its first trial, and the verdict is computed by the platform from the stored evidence — never asserted by the model that ran the experiment
- SC-10: A published result reproduces byte-for-byte from its receipt (dataset hash, feature registry version, calendar package, engine version, strategy revision, cost model, seeds, harness version) on a machine that never saw the original run
- SC-11: A failed experiment leaves behind a named failure class, a closed question or a resolved contradiction that a later run retrieves and does not repeat — measured as yield per run and `cost_per_knowledge_unit`
- SC-12: Researching a strategy, changing a platform artifact, deploying to paper and touching live capital are four separate rings; the agent operates freely in the first, reaches the second and third only through a proposal a human decides, and has no tool at all for the fourth
- SC-13: The agent's research *judgement* is measured, not assumed — planted ground truth it must find, planted nulls it must reject, and a scorecard per model tier that gates what that tier is allowed to do unattended

---

## What Would Make This Fail?

- FM-1: A strategy sees future data (lookahead bias) — product — backtests are unreliable
- FM-2: An order bypasses the risk gate — technical — real money lost to unchecked positions
- FM-3: Duplicate order submission on reconnect — technical — double positions
- FM-4: Internal position view diverges from broker without halting — operational — trades on stale state
- FM-5: f64 arithmetic error on price/size — technical — silent money calculation errors
- FM-6: Data pipeline keeps running with no active consumers — operational — resource waste and operational confusion

Added 2026-09-15, for the research agent:

- FM-7: A backtest looks good because the evidence was selected after seeing it — product — the platform's entire claim is that it structurally cannot do this, so one instance voids the claim rather than costing one strategy
- FM-8: Looks accumulate uncounted, so deflation is too lenient and noise passes the gates — technical — the failure is silent and only shows up as live underperformance months later
- FM-9: A result cannot be reproduced six months later because a dependency moved underneath it — operational — the knowledge base becomes a set of claims nobody can check
- FM-10: The agent runs continuously and produces motion rather than knowledge — burns budget on restatements of things already known, or circles one subject — product — detectable only by measuring yield, which is why yield is a first-class object
- FM-11: Forward-collected series (funding, open interest, options chains, L2) were never switched on, so in two years the platform still has no history of them — operational — **this failure mode cannot be repaired retroactively at any cost**, which is why it outranks build work in the roadmap

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
- **Research substrate:** ClickHouse (bars, trials, outcomes), Postgres + pgvector (ledger, knowledge, jobs), NATS, Redis; the platform serves on **7080**
- **Data policy:** free sources only (ADR-0027) — which bounds history depth and makes several series forward-collect-only
- **Model tiers:** a frontier hosted tier and local tiers on one consumer GPU; every capability claim per tier is measured, never declared
- **Backtest engine:** `market_simulator` (`github.com/MHughesDev/market_simulator`) is an external dependency — this repo owns the adapter (`crates/market-simulator-adapter`), not the replay engine or fill simulator
