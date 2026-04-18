# Backtesting

> **Architecture status:** This file describes the **repository as implemented today**. For **target** system architecture during the canonical APEX migration, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`../Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/). Where this document conflicts with canonical APEX specs on **future** design, treat this file as **as-built** only, not the roadmap.


## Modules

- **`replay.py`** — **`replay_decisions`**, **`replay_multi_asset_decisions`**: walk historical bars through **`run_decision_tick`** (same path as live when configured consistently). **FB-CAN-009:** optional **`ReplayRunContract`**, **`emit_canonical_events`**, **`fault_injection_profile`**; per-bar **`canonical_events`** (market, structural, safety, decision with trigger/auction/risk, fault, execution feedback).
- **`replay_core.py`** — shared step wiring deterministic replay (`replay_deterministic`).
- **`replay_events.py`** — build canonical **`ReplayEventEnvelope`** payloads.
- **`fault_injection.py`** — spread widen, stale timestamps, missing features, confidence corruption.
- **`simulator.py`** — Slippage, fees, fill price helpers
- **`portfolio.py`** — Cash and position tracking when `track_portfolio=True`
- **`execution_params.py`** — Backtest execution parameters from settings

## Semantics

- Optional **portfolio accounting** (slippage, fees, solvency) — see **`docs/BACKTESTING_SIMULATOR.MD`** for full detail
- **`backtesting_replay_available_cash`** (settings): when enabled with portfolio tracking, passes simulated cash into risk for buy cap consistency
