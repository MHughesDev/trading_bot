# 09 — Tech Stack

Rust's standard library is not enough for this platform; use the ecosystem. Versions below are a
**class of stack**, not pinned truth — verify latest at implementation time.

## Choices and rationale

| Concern | Choice (v1) | Why |
|---------|-------------|-----|
| Async runtime | **tokio** | De-facto production async runtime for network-heavy Rust |
| REST + WS server | **axum** + tower/tower-http | Ergonomic routing, WS upgrade, middleware (auth, CORS, trace, compression, timeouts, request-id) |
| Event fabric | **NATS JetStream** (`async-nats`) | Durable + replayable pub/sub, *one lightweight binary* — Kafka's ops weight isn't justified at this scope. Swap later if outgrown (bus is reversible). |
| Transactional DB | **PostgreSQL** via `sqlx` | Direct SQL, compile-time-checked queries, no ORM magic; for users/orders/fills/configs |
| Time-series DB | **ClickHouse** (`clickhouse`) | High-volume bars/trades/features, fast historical scans |
| Latest-state cache | **Redis/Valkey** (`redis`) | Latest price, snapshot pointers, subscriptions, rate-limit counters — never source of truth |
| Exchange WS clients | **tokio-tungstenite** | Connecting to venue market-data/user streams |
| HTTP client | **reqwest** (rustls) | Broker REST APIs, future scrapers |
| Serialization | **serde** / **serde_json** | JSON for control plane + UI; binary later if needed |
| Columnar / research | **arrow**, **parquet**, **datafusion** | Raw event archive + backtest research scans |
| Decimals | **rust_decimal** | Money types; never f64 for price/size |
| Time | **chrono** | Timestamps (all UTC) |
| IDs | **uuid** | Event ids, correlation/causation |
| Observability | **tracing** + tracing-subscriber | Structured logs + metrics (throughput AND correctness) |

## Starting `Cargo.toml` dependency set

```toml
[dependencies]
# async runtime
tokio = { version = "1", features = ["full"] }

# web API / middleware
axum = { version = "0.8", features = ["ws", "macros"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "trace", "compression-full", "timeout", "request-id"] }

# serialization / core types
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
rust_decimal = { version = "1", features = ["serde"] }

# databases / cache
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono", "json", "rust_decimal"] }
clickhouse = "0.13"
redis = { version = "0.32", features = ["aio", "tokio-comp"] }

# event fabric
async-nats = "0.42"

# external connectivity
tokio-tungstenite = "0.29"
reqwest = { version = "0.13", features = ["json", "rustls-tls", "gzip", "brotli"] }

# columnar analytics / backtest research
arrow = "58"
parquet = "58"
datafusion = "50"

# observability
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
```

## What we are deliberately NOT using in v1

- **Kafka/Redpanda/rdkafka** — JetStream is enough; revisit only at measured scale.
- **A heavy ORM** — SQLx's direct SQL is the right control level.
- **OpenSearch / vector DBs** — arrive with post-v1 news/social/AI features.
- **Headless browsers in every worker** — scraping is post-v1; when it comes, isolate browsers
  behind a separate scraping service.

## Monorepo crate layout

The `domain` crate is the shared core; it defines the irreversible types from
[02-data-model.md](./02-data-model.md) and is imported everywhere.

```
trading-platform/
  Cargo.toml                      # workspace
  crates/
    domain/                       # THE CORE — build this first
      src/
        envelope.rs               # EventEnvelope<T>
        payloads.rs               # Trade/Quote/OrderBook/Bar payloads (v1)
        instrument.rs             # Instrument metadata model + AssetClass
        timestamp.rs              # the four timestamps + semantics
        money.rs                  # Price/Size newtypes (no From<f64>)
        trust.rs                  # TrustTier
        strategy_def.rs           # canonical strategy definition format
        ids.rs                    # identity / dedup-key helpers
    api/                          # axum REST + ws upgrade + auth
      src/{routes/, ws/, auth/}
    event-bus/                    # producer/consumer wrappers, lane/topic naming
      src/{producer.rs, consumer.rs, lanes.rs}
    collectors/                   # satellite processes
      src/{crypto/, equity/, normalizer.rs}
    builders/                     # PURE functions: bar builder, orderbook reconstruction
      src/{orderbook.rs, bars.rs, watermark.rs}
    features/                     # PURE functions: indicators (EMA, RSI…)
      src/{lib.rs, ema.rs, rsi.rs, window.rs}
    strategy-runtime/
      src/{runtime.rs, world.rs, interpreter.rs, clock.rs, intents.rs}
    execution/                    # broker adapters + order state machine
      src/{broker.rs, paper.rs, order_state.rs, fills.rs}
    risk/                         # the risk gate + kill switch
      src/{gate.rs, limits.rs, kill_switch.rs}
    storage/
      src/{postgres.rs, clickhouse.rs, parquet.rs, redis.rs}
    market-simulator-adapter/     # adapter to github.com/MHughesDev/market_simulator
      src/{lib.rs, export.rs, run_request.rs, results.rs}
                                  # export.rs: raw archive → Arrow IPC for market_simulator
                                  # run_request.rs: build RunRequest from strategy + range
                                  # results.rs: parse TradeRecord/metrics → domain types
    mcp-server/                   # thin front door -> canonical strategy JSON
      src/{tools.rs}
    observability/
      src/{tracing.rs, metrics.rs}
```

**No `backtest/` crate.** Fill simulation is owned entirely by
`github.com/MHughesDev/market_simulator`. This repo provides the data (raw archive → Arrow IPC)
and the adapter; it does not implement a replay engine or a fill model.

Boundaries live in code even when deployed as one binary + collectors. Extract a crate into its
own process only when a measured pressure (independent failure or scaling) forces it.
