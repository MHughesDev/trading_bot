# P&L ledger (source of truth)

**FB-DASH-05-01** — how realized P&L is recorded and how it may evolve.

## Current (v1): hybrid local file + venue positions

| Layer | Source | Role |
|-------|--------|------|
| **Realized P&L (USD)** | Append-only **`data/pnl_ledger.jsonl`** (gitignored) | Operator- or process-appended events; summed by time window for `GET /pnl/summary` |
| **Unrealized P&L** | **Execution adapter** open positions (same path as `GET /portfolio/positions`) | Venue/Kraken mark policy as in FB-DASH-04 |

The file is **not** auto-filled from live fills yet. Use it for manual reconciliation, imports, or future hooks that append on closed trades.

Each line is JSON:

```json
{"ts": "2026-04-11T12:00:00+00:00", "realized_pnl_usd": "123.45", "symbol": "BTC-USD", "source": "manual", "note": "optional"}
```

## Future options (not implemented)

1. **QuestDB** — durable `pnl_events` table aligned with bars/traces; control plane reads aggregates.
2. **Alpaca activity / fills API** — paper realized history when keys are present; map symbols to internal IDs.
3. **Hybrid** — venue backfill into JSONL or QuestDB; ledger remains the API-facing abstraction.

**API:** `GET /pnl/summary?range=hour|day|month|year|all` — realized from JSONL, unrealized from positions.
