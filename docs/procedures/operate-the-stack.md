# Operate the Stack

> Verified against the as-built system. Phase 7 / P7-T05.

Covers: starting and stopping the platform, operating the kill switch, recovering from common faults, and acting on reconciliation alarms.

---

## 1. Starting the Stack

### Prerequisites

Set environment variables (or populate `config/local.toml`):

```
NATS_URL=nats://localhost:4222
DATABASE_URL=postgres://user:pass@localhost:5432/trading_bot
CLICKHOUSE_URL=http://localhost:8123
REDIS_URL=redis://localhost:6379
ALPACA_API_KEY_ID=<paper account key>
ALPACA_API_SECRET_KEY=<paper account secret>
```

### 1.1 Infrastructure (Docker)

```bash
docker compose up -d          # NATS, Postgres, ClickHouse, Redis
```

Verify:
```bash
docker compose ps             # all services healthy
```

### 1.2 Database Migrations

```bash
sqlx migrate run              # applies pending migrations in migrations/
```

### 1.3 Main Platform Binary

```bash
cargo run -p platform         # dev
./platform                    # or the release binary
```

The platform binary starts: Axum REST API (port 8080), WebSocket gateway (port 8081), strategy runtime, risk gate, execution engine, demand manager.

Healthy startup log:
```
INFO platform starting
INFO NATS connected and streams provisioned
INFO Postgres pool connected
INFO API listening on 0.0.0.0:8080
INFO WS gateway listening on 0.0.0.0:8081
```

### 1.4 Satellite Collectors

Each collector is a separate process:

```bash
# Crypto
./collector-crypto

# Equity (one process per symbol batch, or all symbols as args)
./collector-equity AAPL SPY MSFT
```

Healthy startup log:
```
INFO starting equity data collector symbols=["AAPL", "SPY"]
INFO NATS connected and streams provisioned
INFO subscribed to Alpaca trades symbol=AAPL
INFO subscribed to Alpaca trades symbol=SPY
```

### 1.5 MCP Server (optional)

```bash
./mcp-server                  # listens on stdin/stdout (JSON-RPC 2.0)
```

---

## 2. Stopping the Stack

### Graceful shutdown

Send `SIGTERM` to each process:
```bash
kill -TERM <pid>              # each binary handles SIGTERM and flushes
```

### Emergency stop (kill switch — do this first if trades are live)

```bash
curl -X POST http://localhost:8080/api/trading/kill \
  -H "Authorization: Bearer <token>"
```

This trips the global kill switch — all new order submissions are blocked immediately. Open positions remain open until manually closed.

Verify the switch is active:
```bash
curl http://localhost:8080/api/trading/status \
  -H "Authorization: Bearer <token>"
# returns: {"trading_enabled": false, "tripped_by": "manual"}
```

### Resume trading

```bash
curl -X POST http://localhost:8080/api/trading/resume \
  -H "Authorization: Bearer <token>"
```

Requires explicit human action — the API will not auto-resume.

---

## 3. Kill Switch

The kill switch is a global `trading_enabled` flag checked synchronously at the top of every `RiskGate::check()` call. When active:
- All new order submissions are rejected immediately with `RiskRejection::KillSwitchActive`.
- Open positions are **not** force-closed (closing is a deliberate separate action).
- Collectors and data pipelines continue operating normally.
- Strategy runtimes continue evaluating signals but the gate blocks their intents from reaching the broker.

### Automatic trips

| Trigger | Action |
|---------|--------|
| Daily loss limit exceeded | Kill switch trips; `KillSwitchActive` returned on next gate call |
| Position/broker reconciliation divergence | Kill switch trips for the affected instrument |
| Market data staleness on active strategy | Kill switch trips |
| Broker disconnection | Kill switch trips; reconciliation on reconnect |

### Manual trip

Via REST: `POST /api/trading/kill`  
Via UI: "Kill Switch" button on the dashboard.

### Recovery

After any automatic trip, investigate the cause before resuming:
1. Check tracing logs for the trip reason.
2. Reconcile positions manually with the broker if needed.
3. Confirm the issue is resolved.
4. Call `POST /api/trading/resume`.

---

## 4. Recovery Scenarios

### 4.1 Collector disconnects from venue

Collectors use `ReconnectPolicy` with exponential back-off. No human action needed — the collector reconnects automatically. Monitor logs for repeated `reconnecting to Alpaca WS` lines; if the venue is confirmed down, kill the collector and restart when the venue is back.

### 4.2 NATS JetStream down

The main platform binary will detect NATS disconnection and trip the kill switch. Collectors will buffer locally (in memory) up to the ReconnectPolicy limit. Restart NATS, then restart the platform binary; collectors reconnect automatically.

### 4.3 Postgres connection lost

The platform binary will fail health checks. No orders can be written to the audit ledger. The kill switch trips. Restore the Postgres connection, then restart the platform.

### 4.4 Sequence gap detected

The gap detector in each collector emits a `WARN` log:
```
WARN sequence gap detected instrument_id=AAPL lane=market.trades expected=101 got=105
```
This is informational — it means trades may have been missed. If the gap is persistent and you have an active strategy on the affected instrument, stop the strategy and restart the collector to re-establish a clean connection.

---

## 5. Reconciliation Alarms

The reconciliation engine runs continuously. Divergence produces a `WARN` or `ERROR` log:

```
ERROR reconciliation divergence instrument_id=AAPL 
      internal_position=100 broker_position=95 divergence=5
```

Response procedure:
1. Kill switch trips automatically on divergence.
2. Log the divergence report (instrument, amounts, timestamp).
3. Check broker account directly for the true position.
4. If positions match after manual reconciliation, resume via `POST /api/trading/resume`.
5. If they don't match, do not resume until the cause is understood.

**Never force-resume without understanding the cause.** Reconciliation alarms are safety signals.

---

## 6. Health Checks

```bash
curl http://localhost:8080/health            # platform binary
# {"status": "ok", "trading_enabled": true, "nats": "connected", "pg": "connected"}
```

The `/health` endpoint is suitable for container health checks and load balancer probes.
