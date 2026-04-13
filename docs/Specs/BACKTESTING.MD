# Backtesting

## Modules

- **`replay.py`** — **`replay_decisions`**, **`replay_multi_asset_decisions`**: walk historical bars through **`run_decision_tick`** (same path as live when configured consistently)
- **`simulator.py`** — Slippage, fees, fill price helpers
- **`portfolio.py`** — Cash and position tracking when `track_portfolio=True`
- **`execution_params.py`** — Backtest execution parameters from settings

## Semantics

- Optional **portfolio accounting** (slippage, fees, solvency) — see **`docs/architecture/backtesting_simulator.md`** for full detail
- **`backtesting_replay_available_cash`** (settings): when enabled with portfolio tracking, passes simulated cash into risk for buy cap consistency
