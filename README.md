# Trading Bot

An all-in-one, **data & asset scalable** trading platform built in Rust. Designed from the ground up to support any asset class — crypto spot, equities, options, futures, FX, perpetuals, DEX/AMM pools, prediction markets, and beyond — through a metadata-driven architecture where adding a new market type is additive and requires zero changes to core runtime, risk, storage, or replay code.

The Python → Rust refactor is **complete** (Phase 7 done). This is the canonical, production system.

---

## Architecture and documentation

All system design documentation lives in [`docs/`](docs/README.md):

- [`docs/architecture.md`](docs/architecture.md) — system map, components, data flow, repo structure
- [`docs/adr/`](docs/adr/README.md) — 11 Architecture Decision Records
- [`docs/specs/`](docs/specs/README.md) — component and feature specifications (all `Implemented`)
- [`docs/plans/`](docs/plans/README.md) — the completed 10-phase refactor plan (Phase A → Phase 7)
- [`docs/parity-matrix.md`](docs/parity-matrix.md) — behavior parity verification (Python vs Rust)

**Agent/operator instructions:** [`AGENT.md`](AGENT.md)

---

## Quickstart

### Prerequisites

- Rust (toolchain pinned in `rust-toolchain.toml`)
- Docker (for NATS, Postgres, ClickHouse, Redis)
- Alpaca paper trading account (free at alpaca.markets)

### 1. Start infrastructure

```bash
docker compose up -d
```

### 2. Set environment variables

```bash
cp .env.example .env
# Edit .env: DATABASE_URL, NATS_URL, CLICKHOUSE_URL, REDIS_URL
# Add: ALPACA_API_KEY_ID, ALPACA_API_SECRET_KEY
```

### 3. Run database migrations

```bash
sqlx migrate run
```

### 4. Start the platform

```bash
cargo run -p platform
```

Platform starts on:
- `http://localhost:8080` — REST API
- `ws://localhost:8081` — WebSocket live feed

### 5. Start collectors (optional, for live data)

```bash
# Crypto (Kraken)
cargo run -p collector-crypto

# Equity (Alpaca IEX — free tier)
cargo run -p collector-equity -- AAPL SPY
```

### 6. Phase 7 satellites (optional, for knowledge layer)

These services require additional env vars — see `.env.example` for all required values.

```bash
# Web page scraper — requires WEB_SCRAPER_URLS and optionally PLAYWRIGHT_BIN
cargo run -p collector-web

# Semantic embedder — requires OPENAI_API_KEY, MILVUS_HOST/MILVUS_HTTP_PORT
cargo run -p embedder
```

Start the knowledge-layer backends (TigerGraph + Milvus) with:
```bash
docker compose up -d tigergraph etcd minio milvus
```

### 7. MCP server (optional, for AI agent integration)

```bash
cargo run -p mcp-server   # JSON-RPC 2.0 over stdin/stdout
```

---

## System overview

```
Market Venues
  ├── Kraken WS (crypto)
  └── Alpaca WS (equity)
       │ normalize() → EventEnvelope
Satellite Collectors
       │ publish to NATS JetStream
Event Bus (NATS JetStream)
       ├── Storage Writers (Postgres, ClickHouse, Parquet)
       ├── Feature Engine (PURE — same code live and replay)
       └── Strategy Runtime
               │ order intents
               ▼
          Risk Gate ← single chokepoint, no bypass
               │ approved orders
               ▼
          Execution Engine
               ├── Coinbase (live crypto)
               └── Alpaca (paper / live equity)

React Frontend ↔ REST API + WebSocket
MCP Server ↔ REST API (strategy authoring only)
```

---

## Key properties

| Property | Mechanism |
|----------|-----------|
| No float money | `Price(Decimal)` / `Size(Decimal)` — no `From<f64>` |
| Single risk gate | `ApprovedOrder._sealed: ()` — private field, no external construction |
| No lookahead in replay | `world.now()` returns `event.available_time`, never wall clock |
| Asset class parity | Equities and crypto use identical core code; differences in instrument metadata only |
| Tighten-only overrides | Strategy `risk_overrides` validated at load time, not at order time |
| Idempotent deduplication | Every order has an `idempotency_key`; gate caches decisions |

---

## Running tests

```bash
cargo test --workspace        # 255 tests, all pass
```

---

## Docker build

```bash
docker build -t trading-bot:latest .
docker run -p 8080:8080 -p 8081:8081 trading-bot:latest
```

---

## Operational procedures

- [Start/stop, kill switch, recovery](docs/procedures/operate-the-stack.md)
- [Add a new venue or asset class](docs/procedures/add-a-venue.md)
