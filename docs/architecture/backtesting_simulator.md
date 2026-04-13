# Backtest simulator semantics

This describes how **`backtesting/simulator.py`** and **`replay_decisions(..., track_portfolio=True)`** apply fills in alignment with live `run_decision_tick`.

## Slippage

- **`slippage_bps`:** Half-spread style adjustment: for a **buy**, execution price = mid + `(slippage_bps / 10_000) * mid`; for a **sell**, mid minus that amount (`apply_slippage`).
- **`slippage_noise_bps`:** If `> 0`, a uniform random offset in `[-noise, +noise]` bps is added to the base bps on each fill (`effective_slippage_bps`). Use **`rng_seed`** / `NM_BACKTESTING_RNG_SEED` for reproducibility (`make_replay_rng`).

## Fees

- **`fee_bps`:** Charged on **absolute notional** of the fill: `fee = |qty * fill_price| * (fee_bps / 10_000)` (`fee_on_notional`).
- **Buy:** cash changes by `-(notional + fee)`.
- **Sell:** cash changes by `+(notional - fee)` (`cash_delta_for_trade`).

## Solvency (replay with portfolio)

When **`track_portfolio`** is enabled and **`backtesting.enforce_solvency`** is true (default), a simulated **buy** is **not** applied if `portfolio.cash + cash_delta < 0` after fees. The row sets **`solvency_blocked: true`** and **`trade: null`** for that bar; position and cash are unchanged. **`RiskEngine`** is unchanged—solvency is a **replay accounting** layer only (Issue 33).

## Multi-symbol replay (`replay_multi_asset_decisions`)

**`backtesting.replay.replay_multi_asset_decisions`** walks a **merged timeline** of bars for multiple symbols. It uses **one** shared `RiskState`, and with `track_portfolio=True` **one** `PortfolioTracker` and RNG (same fee/slippage settings as single-symbol replay). Symbols are processed in **sorted name order** at each timestamp for deterministic cash usage. Output rows look like `{ "timestamp", "symbols": { "BTC-USD": { ... }, ... }, "portfolio_cash", ... }`.

## Config

See **`app/config/default.yaml`** → `backtesting:` and env **`NM_BACKTESTING_*`**.
