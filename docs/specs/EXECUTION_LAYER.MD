# Execution layer

> **Architecture status:** This file describes the **repository as implemented today**. For **target** system architecture during the canonical APEX migration, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`../Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/). Where this document conflicts with canonical APEX specs on **future** design, treat this file as **as-built** only, not the roadmap.

> **Tombstone (FB-CAN-023):** As-built execution map. APEX execution logic **requirements** → canonical specs + **[`CANONICAL_PRECEDENCE.MD`](CANONICAL_PRECEDENCE.MD)**.

## Routing and service

- **`router.py`** — **`create_execution_adapter`**: selects adapter from `AppSettings` (`paper` → Alpaca paper, `live` → Coinbase live); duplicate alias removed (**FB-CAN-021**)
- **`service.py`** — **`ExecutionService`**: central submit path; runs **`execution_logic.prepare_order_intent_for_execution`** (guidance + optional size scale) then intent gate + adapter
- **`execution_logic.py`** — **FB-CAN-008:** execution confidence, style (`passive` / `aggressive` / `staggered` / `twap` / `suppress`), stress mode, worst-case edge; **`ExecutionFeedback`** / **`reconcile_partial_fill`** helpers; live loop attaches **`metadata.execution_context`**
- **`partial_fill_reconcile.py`** — **FB-CAN-048:** typed **`PartialFillReconciliation`** from **`reconcile_partial_fill_record`** (uses **`apex_canonical.domains.execution`** partial-fill thresholds)
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
