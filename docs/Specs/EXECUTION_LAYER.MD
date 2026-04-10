# Execution layer

## Routing and service

- **`router.py`** — **`create_execution_adapter` / `get_execution_adapter`**: selects adapter from `AppSettings` (`paper` → Alpaca paper, `live` → Coinbase live)
- **`service.py`** — **`ExecutionService`**: central submit path; uses intent gate + adapter
- **`intent_gate.py`** — Validates execution allowed (signing, settings)

## Adapters (`execution/adapters/`)

- **`base_adapter.py`** — `ExecutionAdapter` protocol, `OrderAck`, `PositionSnapshot`
- **`alpaca_paper.py`** — Paper trading (requires `[alpaca]` extra)
- **`coinbase_live.py`** — Live Coinbase Advanced Trade via **`coinbase_advanced_http.py`** (CDP JWT REST for orders, cancel, accounts/positions)

## HTTP helper

- **`coinbase_advanced_http.py`** — REST signing and request helpers for Coinbase CDP

## Utilities

- **`alpaca_util.py`** — Alpaca-specific helpers confined to execution (not data plane)

## Invariants

- Market data does not flow through Alpaca in the main tree; only **execution** uses Alpaca for **paper** fills.
