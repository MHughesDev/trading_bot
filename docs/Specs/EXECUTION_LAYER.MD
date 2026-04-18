# Execution layer

> **Architecture status:** This file describes the **repository as implemented today**. For **target** system architecture during the canonical APEX migration, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`../Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/). Where this document conflicts with canonical APEX specs on **future** design, treat this file as **as-built** only, not the roadmap.


## Routing and service

- **`router.py`** — **`create_execution_adapter` / `get_execution_adapter`**: selects adapter from `AppSettings` (`paper` → Alpaca paper, `live` → Coinbase live)
- **`service.py`** — **`ExecutionService`**: central submit path; runs **`execution_logic.prepare_order_intent_for_execution`** (guidance + optional size scale) then intent gate + adapter
- **`execution_logic.py`** — **FB-CAN-008:** execution confidence, style (`passive` / `aggressive` / `staggered` / `twap` / `suppress`), stress mode, worst-case edge; **`ExecutionFeedback`** / partial-fill reconciliation helpers; live loop attaches **`metadata.execution_context`**
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
