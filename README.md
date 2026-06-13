# Trading Bot

An all-in-one, **data & asset scalable** trading platform built in Rust. Designed to support any asset class — crypto spot, equities, options, futures, FX, perpetuals, DEX/AMM pools, prediction markets — through a metadata-driven architecture where adding a new market type is additive with zero changes to core runtime or storage code.

---

## Documentation

**Start here:** [`docs/README.md`](docs/README.md) — central navigation hub for all design content.

| Path | Purpose |
|------|---------|
| [`docs/NEWCOMERS.md`](docs/NEWCOMERS.md) | Onboarding guide — mental models, module walkthroughs, glossary |
| [`docs/procedures/AGENT.md`](docs/procedures/AGENT.md) | Agent/operator instructions |
| [`docs/architecture.md`](docs/architecture.md) | System map, components, data flow, repo structure |
| [`docs/adr/`](docs/adr/README.md) | Architecture Decision Records (ADR-0001 – ADR-0013) |
| [`docs/specs/`](docs/specs/README.md) | Feature and component specifications |
| [`docs/plans/plan-sets/`](docs/plans/plan-sets/) | Implementation plan sets (A through G) |

---

## Quickstart

```bash
# 1. Start infrastructure
docker compose up -d

# 2. Configure environment
cp .env.example .env
# Edit .env: DATABASE_URL, NATS_URL, CLICKHOUSE_URL, REDIS_URL, ALPACA_API_KEY_ID, etc.

# 3. Run migrations
sqlx migrate run

# 4. Start platform
cargo run -p platform           # REST API: localhost:8080  WebSocket: localhost:8081

# Optional: live data collectors
cargo run -p collector-crypto   # Kraken WS
cargo run -p collector-equity -- AAPL SPY  # Alpaca IEX

# Optional: AI agent integration
cargo run -p mcp-server         # JSON-RPC 2.0 over HTTP at localhost:3002
```

---

## Key properties

| Property | Mechanism |
|----------|-----------|
| No float money | `Price(Decimal)` / `Size(Decimal)` — no `From<f64>` |
| Single risk gate | `ApprovedOrder._sealed: ()` — private field, no external construction |
| No lookahead in replay | `world.now()` returns `event.available_time`, never wall clock |
| Asset class parity | Equities and crypto use identical core code; differences in instrument metadata only |
| Idempotent deduplication | Every order has an `idempotency_key`; gate caches decisions |

---

## Tests

```bash
cargo test --workspace
```

---

## Operations

- [Start/stop, kill switch, recovery](docs/procedures/operate-the-stack.md)
- [Add a new venue or asset class](docs/procedures/add-a-venue.md)
