---
Type: Formal
Status: Complete
Derived From: C-017, C-060, C-065, C-077, C-091, C-099, C-101, C-102, C-103, C-106, C-112
---

# Phase 2 — Collector Infrastructure

> **Self-contained execution doc.** You need only: this file, [`../../architecture.md`](../../architecture.md),
> the specs (especially
> [`../../specs/COMP-001-data-quality-and-ingestion.md`](../../specs/COMP-001-data-quality-and-ingestion.md)
> §3/§7 and [`../../specs/DATA-002-instrument-metadata.md`](../../specs/DATA-002-instrument-metadata.md)),
> and the existing codebase. Existing collectors (`crates/collectors`: Kraken crypto, Alpaca equity
> data) and the NATS JetStream wrappers in `crates/event-bus` are your templates — read them first.
> Phase 1 must be complete: `DataType`, `SupportedVenue`, the 8-class `AssetClass` exist.

## Phase goal

After this phase the **live-data plane scales to the full venue set and is browser-consumable**: the
collector-sharing model is upgraded (server-wide ref-counted demand by lane key, 120-second warm
period, server-wide rate-limit admission control); the browser subscribes to OHLCV directly over
NATS.ws via a defined subject mapping; a freshness watchdog alarms only during expected tradable
periods using instrument trading-hours metadata; per-venue health-check endpoints exist; new venue
collectors (OANDA FX, Kalshi prediction+perps, Tradier options, 0x DEX quotes, Tradovate futures
demo) and the Reddit collector satellite stream normalized events onto the bus; and the TigerGraph +
Milvus infrastructure containers are stood up with client code ready (schema/population is Phase 7).

## Prerequisites

- Phase 1 done: `DataType`, `SupportedVenue`, `AssetClass` (8), registries seeded.
- Existing Kraken + Alpaca-data collectors compile and run as satellites.
- NATS JetStream available via `crates/event-bus`.

## Invariants this phase must respect

- **Minimum source data** (C-112/C-129): collectors ingest 1-minute OHLCV (plus firm quotes / funding
  / prediction prices where the market structure requires). **No order-book/DOM/tick-list ingestion.**
- **Collectors are server-wide shared** (C-091): ref-counted by lane key; free-tier rate-limit budget
  is a single server-wide resource; 120-second warm period before teardown on zero demand.
- **Append-only / revision semantics** carry over from existing collectors — late data emits a
  revision, never a mutation.
- **Canonical vs lossy split** — collectors feed the canonical bus; the browser NATS.ws view is the
  lossy consumer feed.

---

## Tasks

### P2-T01 — DemandRegistry ref-counting by lane key
- **Goal:** A server-wide registry that ref-counts active demand per subscription lane and drives
  collector start/stop.
- **Files:** `crates/demand-manager/src/registry.rs`, `crates/demand-manager/src/lib.rs`.
- **Context:** Per C-091. A **lane key** is `(SupportedVenue, AssetClass, DataType, instrument_id)`.
  `DemandRegistry::acquire(lane_key)` increments the count and starts the collector lane if it was 0;
  `release(lane_key)` decrements and, on reaching 0, schedules teardown after a **120-second warm
  period** (a later `acquire` within the window cancels teardown). Demand from *all* users/UI panels
  for the same lane shares one collector. State is in-memory with an `Arc<Mutex<…>>` or dashmap.
- **Acceptance:** `crates/demand-manager/tests/refcount.rs` proves: two acquires + one release keeps
  the lane live; the final release schedules teardown; an acquire within 120 s cancels teardown; an
  acquire after the window starts a fresh lane.
- **Depends on:** Phase 1 (`SupportedVenue`, `DataType`).

### P2-T02 — Server-wide rate-limit admission control
- **Goal:** A shared budget that admits/denies new collector subscriptions against per-venue free-tier
  rate limits.
- **Files:** `crates/demand-manager/src/rate_budget.rs`.
- **Context:** Per C-091. Each `SupportedVenue` declares a free-tier budget (requests/min, max
  concurrent subscriptions). `RateBudget::try_admit(venue, cost)` returns `Ok` or a structured
  `BudgetExceeded`. The `DemandRegistry` consults it before starting a new lane; if denied, the lane
  is queued or rejected with a typed error surfaced to the UI. The budget is a single server-wide
  resource shared across all users.
- **Acceptance:** `crates/demand-manager/tests/rate_budget.rs` proves admitting up to the cap succeeds
  and the next admission is denied until a release frees budget.
- **Depends on:** P2-T01.

### P2-T03 — NATS.ws subject mapping for browser subscription
- **Goal:** A deterministic subject scheme so the browser can subscribe to OHLCV directly via NATS.ws
  (no SSE bridge).
- **Files:** `crates/event-bus/src/subjects.rs` (new or extend), `crates/ui-gateway/src/nats_ws.rs`
  (new).
- **Context:** Per C-060. Define `fn ohlcv_subject(venue, asset_class, instrument_id) -> String`
  producing e.g. `md.ohlcv.kraken.crypto_spot_cex.BTC-USD`. The collector publishes the lossy UI view
  to this subject; the browser (Phase 5 hook) subscribes over the NATS.ws port. `nats_ws.rs` documents
  the websocket port/credentials config and any subject allow-list/authz so a browser token can only
  subscribe to permitted lanes. This is the lowest-latency path (no SSE bridge).
- **Acceptance:** `crates/event-bus/tests/subjects.rs` proves `ohlcv_subject` is stable and round-trips
  to its components via a parser; an integration check publishes one bar to the subject and a NATS.ws
  test client receives it.
- **Depends on:** P2-T01.

### P2-T04 — Freshness watchdog driven by trading-hours metadata
- **Goal:** A per-lane watchdog that alarms only when a lane is silent during the instrument's expected
  tradable hours.
- **Files:** `crates/reconciliation/src/freshness.rs` (extend if present), wired from the collector
  supervisor.
- **Context:** Per C-099. For each active data lane, read the instrument's `TradingSchedule` /
  trading-hours metadata (already in `domain`/`storage`). The watchdog tracks last-event time per lane
  and raises an alarm only if no event arrived within the staleness threshold **while the instrument
  is in an expected-tradable window** — a normal 4 pm equity close or a weekend FX gap must not
  alarm. 24/7 classes (crypto) are always expected-tradable.
- **Acceptance:** `crates/reconciliation/tests/freshness_hours.rs` proves a normal session close does
  not alarm while a true mid-session outage does; a 24/7 crypto lane alarms on any gap beyond
  threshold.
- **Depends on:** P2-T01.

### P2-T05 — Per-venue health-check endpoints
- **Goal:** A health check per venue used for credential verify-before-save and for ops monitoring.
- **Files:** `crates/api/src/routes/venue_health.rs` (new), register in routes `mod.rs`; per-venue
  check fns alongside each collector/adapter.
- **Context:** Per C-077. `GET /api/venues/{venue}/health` (and a callable `async fn
  health_check(venue, creds) -> HealthStatus`) performs a cheap authenticated round-trip (e.g. fetch
  account/server time) against the venue. This is the verifier passed to the Phase 1
  `verify_then_store` credential flow. Returns `{ ok, latency_ms, message }`; never returns credential
  material.
- **Acceptance:** `crates/api/tests/venue_health.rs` proves the endpoint returns `ok=false` with a
  structured message for bad/missing credentials and `ok=true` against a mocked healthy venue.
- **Depends on:** Phase 1 (`SupportedVenue`, credential service), P2-T03.

### P2-T06 — OANDA FX collector (demo)
- **Goal:** A satellite collector streaming OANDA FX 1-min OHLCV + quotes onto the bus.
- **Files:** `crates/collectors/src/fx/oanda.rs` (new), `apps/collector-fx/` wiring (new app crate
  mirroring `apps/collector-crypto`).
- **Context:** Per the vision (OANDA = FX data + execution, demo for MVP). Use the OANDA v20 demo REST/
  streaming API. Normalize candles into the existing OHLCV `EventEnvelope` payload with correct
  4-timestamp semantics and `DataType::MarketOhlcv`. Register the collector's provided capabilities
  (lane keys) with the DemandRegistry so it starts on demand only. Model on the Kraken collector.
- **Acceptance:** `crates/collectors/tests/oanda_normalize.rs` proves a sample OANDA candle payload
  normalizes to a correct `EventEnvelope` (right instrument, timestamps, `Price`/`Size`, trust tier).
- **Depends on:** P2-T01, P2-T03.

### P2-T07 — Kalshi prediction + perpetuals collector
- **Goal:** A satellite collector for Kalshi prediction-market YES/NO prices and perpetuals data.
- **Files:** `crates/collectors/src/prediction/kalshi.rs` (new), `apps/collector-kalshi/` wiring (new).
- **Context:** Per the vision (Kalshi = prediction markets + perpetuals). Stream market prices as
  `DataType::PredictionMarketPrice` (YES/NO binary in [0,1]) and perpetuals OHLCV/funding as
  `MarketOhlcv`/`MarketFundingRate`. Two asset classes from one venue (`PredictionMarket`,
  `PerpetualSwap`). Normalize to `EventEnvelope`.
- **Acceptance:** `crates/collectors/tests/kalshi_normalize.rs` proves a YES/NO market normalizes to a
  prediction-price event in [0,1] and a perpetual sample normalizes to OHLCV + funding events.
- **Depends on:** P2-T01, P2-T03.

### P2-T08 — Tradier options collector
- **Goal:** A satellite collector for Tradier options 1-min OHLCV + quotes.
- **Files:** `crates/collectors/src/options/tradier.rs` (new), `apps/collector-options/` wiring (new).
- **Context:** Per the vision (Tradier = options data + execution). Normalize option chains/contracts
  to OHLCV + quote events with `AssetClass::Option`. Respect the minimum-data baseline — bars + quotes
  only, no full option-book depth.
- **Acceptance:** `crates/collectors/tests/tradier_normalize.rs` proves an option contract sample
  normalizes to correct OHLCV + quote events with the option instrument identity.
- **Depends on:** P2-T01, P2-T03.

### P2-T09 — 0x DEX quote-snapshot collector
- **Goal:** A satellite collector that snapshots 0x firm swap quotes as `DataType::DexQuote` events.
- **Files:** `crates/collectors/src/dex/zerox.rs` (new), `apps/collector-dex/` wiring (new).
- **Context:** Per the vision (0x = DEX swap aggregation). Poll the 0x swap quote API for the
  configured token pairs and emit firm-quote snapshots (sell/buy token, amounts, price, gas estimate)
  as `DexQuote` events with `AssetClass::CryptoSpotDex`. These firm quotes feed the AMM paper swap
  simulator (Phase 1/4) and DEX live execution (Phase 4).
- **Acceptance:** `crates/collectors/tests/zerox_normalize.rs` proves a 0x quote response normalizes to
  a `DexQuote` event carrying the firm out-amount and price with `Price`/`Size` types.
- **Depends on:** P2-T01, P2-T03.

### P2-T10 — Tradovate futures collector (demo)
- **Goal:** A satellite collector for Tradovate futures 1-min OHLCV (demo environment first).
- **Files:** `crates/collectors/src/futures/tradovate.rs` (new), `apps/collector-futures/` wiring (new).
- **Context:** Per the vision (Tradovate = futures, demo first). Connect to the Tradovate demo API,
  normalize futures bars to `MarketOhlcv` with `AssetClass::FuturesExpiring`, including contract
  expiry metadata on the instrument.
- **Acceptance:** `crates/collectors/tests/tradovate_normalize.rs` proves a futures bar sample
  normalizes to a correct OHLCV event with the expiring-contract instrument identity.
- **Depends on:** P2-T01, P2-T03.

### P2-T11 — Reddit collector satellite
- **Goal:** A satellite that ingests Reddit posts + top-level comments via the official OAuth Data API
  and links them to instruments.
- **Files:** `crates/collectors/src/social/reddit.rs` (new), `apps/collector-reddit/` wiring (new).
- **Context:** Per C-106. Use Reddit's official OAuth Data API with an **80 QPM** budget (admit via the
  P2-T02 rate budget). Ingest posts and top-level comments. **Two-stage instrument linking:** (1)
  cashtag/ticker extraction from text, (2) a confidence filter that only links when the match clears a
  threshold. Emit `DataType::SocialPost` events carrying text + linked instrument ids + confidence.
  Text bodies are later embedded into Milvus (Phase 7).
- **Acceptance:** `crates/collectors/tests/reddit_link.rs` proves a post mentioning `$BTC` links to the
  BTC instrument above threshold while an ambiguous mention falls below threshold and is left unlinked;
  a `SocialPost` event is emitted with the confidence score.
- **Depends on:** P2-T01, P2-T02 (rate budget).

### P2-T12 — TigerGraph + Milvus infra containers + client skeletons
- **Goal:** Stand up the graph and vector stores as containers with Rust client code ready (schema +
  population are Phase 7).
- **Files:** `docker-compose.yml` (extend), `crates/graph/src/lib.rs` (new TigerGraph client crate),
  `crates/semantic/src/lib.rs` (new Milvus client crate); register both in the workspace `Cargo.toml`.
- **Context:** Per C-102/C-103/C-065. Add `tigergraph` and `milvus` (+ its `etcd`/`minio` deps)
  services to `docker-compose.yml` with health checks and pinned image tags. `crates/graph` exposes a
  thin client with `connect()` + a `ping()`; `crates/semantic` exposes a Milvus client with
  `connect()` + `ping()` and a configured collection spec for `text-embedding-3-small` (1536 dims,
  metadata-filtered search). No schema/population yet — just connectivity.
- **Acceptance:** `docker compose up tigergraph milvus` reports healthy; `crates/graph/tests/ping.rs`
  and `crates/semantic/tests/ping.rs` connect and ping successfully against the compose services.
- **Depends on:** none (infra), but lands in this phase.

---

## Phase exit criteria
- [x] `DemandRegistry` ref-counts by lane key with a 120-second warm period; refcount test green.
- [x] Server-wide rate-limit admission control admits/denies against per-venue budgets; test green.
- [x] NATS.ws subject mapping defined; a browser-style NATS.ws client receives a published bar.
- [x] Freshness watchdog respects trading hours (no false alarm on normal close; alarms on true
      outage; 24/7 crypto always watched).
- [x] Per-venue health-check endpoint returns structured status without leaking credentials.
- [x] OANDA, Kalshi, Tradier, 0x, Tradovate collectors normalize sample payloads to correct
      `EventEnvelope`s; each registers on-demand lanes.
- [x] Reddit collector ingests posts/comments with two-stage instrument linking under an 80 QPM
      budget; linking test green.
- [x] TigerGraph + Milvus run via docker-compose; both client crates connect and ping.
- [x] `cargo check --workspace` green; `cargo test --workspace` 0 failures.
