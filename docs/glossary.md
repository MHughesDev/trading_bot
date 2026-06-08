# Glossary

Terms used throughout the system design documentation. Sourced from `refactor_reference_docs/spec/12-glossary.md`.

---

**Available time** — The timestamp marking when a strategy or backtest is *allowed* to use an
event, including processing delay. The clock the replay engine sorts by; the field that prevents
lookahead bias. The single most important timestamp.

**Backpressure** — Mechanisms (bounded queues, lag metrics, drop/throttle policies) that keep the
system stable when downstream consumers fall behind.

**Canonical stream** — The exact, ordered, lossless event stream consumed by storage and the
strategy runtime. Contrast with the lossy UI view.

**Collector** — A satellite process that connects to one venue/source, normalizes raw messages
into typed events, and publishes them. Crashes/reconnects independently of the core.

**Control plane** — REST surface for commands, config, and history. Not the heavy data path.

**Data plane** — Internal normalized events flowing on the bus between services.

**Dedup key** — Deterministic identity derived from the source (not a random UUID at ingest) used
to collapse duplicate deliveries.

**Demand Manager** — Tracks which lanes/instruments are needed by active UI panels and strategy
instances and starts/stops/downshifts the underlying pipelines accordingly.

**Event envelope** — The universal outer wrapper around every event (ids, type, timestamps,
trust tier, sequence) with a typed, versioned payload inside.

**Event fabric / bus** — The streaming backbone (NATS JetStream in v1) carrying typed lanes.

**Front door** — One of the three ways to author a strategy (visual builder, JSON API, MCP
server); all produce the same canonical strategy definition.

**Idempotency** — Property that applying the same event/order twice has the same effect as
applying it once. Required for all money-mutating consumers and the risk gate.

**Instrument metadata** — The table describing each tradable instrument (asset class, venue,
precision, tick/lot, trading hours, halt behavior, trust tier). Where asset-class differences
live so code stays asset-agnostic.

**Kill switch** — A global flag that immediately blocks all new orders; trips automatically on
defined danger conditions or manually. Does not force-close positions.

**Lane** — A typed stream/topic on the bus (e.g. `market.trades`, `orders.events`). Consumers
subscribe to lanes, not asset types.

**Lookahead bias** — Letting a strategy see, at a past moment, information that was not actually
available until later. The main reason backtests lie; prevented by `available_time` ordering.

**Lossy view** — The UI stream, which may drop intermediate frames (e.g. 20 fps order book) for
human visualization. Never used for execution or storage.

**Quarantine lane** — Where schema-validation failures go (with raw bytes + error) instead of
being dropped or coerced; replayable after the normalizer is fixed.

**Reconciliation** — Continuously checking internal state (positions, balances) against the
broker's view and halting on divergence. Where money is actually lost or saved.

**market_simulator** — The external backtest engine (`github.com/MHughesDev/market_simulator`).
This repo is a consumer of it, not its owner. The adapter crate exports data to it and parses
results back. Owns the replay engine, fill simulation, and look-ahead enforcement — not this repo.

**market_simulator adapter** — The crate in this repo (`crates/market-simulator-adapter`) that
translates between this repo's domain types and the market_simulator's Arrow IPC contracts.
Submits Run Requests; returns BacktestReports. Contains no fill logic.

**Replay engine** — Owned by `market_simulator`, not this repo. Feeds recorded raw events through
the same builders used live in `available_time` order, enforcing look-ahead safety.

**Revision event** — An append-only event that supersedes an earlier one (e.g. a bar corrected by
late data). History is never mutated in place.

**Risk gate** — The single synchronous chokepoint every order (manual or automated) passes
through before execution. Enforces limits and the kill switch.

**Strategy definition** — The canonical, versioned JSON document describing a strategy (inputs
declared with `$bound_at_init`, nodes, signals, actions, risk overrides, `asset_class` scope).
The contract all three front doors target. Not pre-bound to an instrument.

**Strategy instance** — A running binding of a strategy definition to a specific instrument,
created when a user clicks "Initialize" on an asset. One instance per instrument per user (MVP
UX constraint). Each instance maintains its own `WorldState`.

**Trust tier** — A first-class classification of how trustworthy a data source is
(`regulated`, `centralized_exchange`, `onchain_confirmed`, `onchain_tentative`,
`social_derived`); quality gates scale with it.

**Watermark** — How long a window (e.g. a bar) waits for late data before being published.
Data after the watermark produces a revision rather than mutating the published value.

**WorldState / WorldContext** — The runtime-maintained local view a strategy reads (latest bar,
order book, features, position, open orders) so it doesn't manually join across tables.
