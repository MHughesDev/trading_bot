# Multi-asset portfolio behavior (FB-SPEC-07)

## As implemented

- **`market_data_symbols`** drives multiple independent **decision ticks** (one stream per symbol in `live_service` / replay loops).
- **Aggregate exposure (FB-SPEC-07-01):** `RiskEngine.evaluate` applies **`risk_max_total_exposure_usd`** as a **portfolio-wide** ceiling on *new* notional (`current_total_exposure_usd + proposed_notional`), and **`risk_max_per_symbol_usd`** per symbol. Configure both in `default.yaml` under `risk:` or via **`NM_RISK_MAX_TOTAL_EXPOSURE_USD`** / **`NM_RISK_MAX_PER_SYMBOL_USD`**. There is no separate “per-tier” cap beyond these two knobs.
- **Cross-symbol policy (FB-SPEC-07-02):** the hot path uses an **independent per-symbol** contract — no correlation matrix, shared factor risk, or risk-parity allocator. Decision ticks do not coordinate entries across symbols beyond the shared global/per-symbol USD caps above.
- **`RiskEngine`** also applies drawdown, spread, stale-data, mode, and reduce-only rules shared across symbols.
- **`Position reconcile`** (if enabled) is per adapter account; multi-symbol positions remain under the same limits.

## Operator warnings

- Increasing **`len(market_data_symbols)`** multiplies concurrent risk checks but **does not** add cross-asset hedging or netting logic in the decision layer.
- **`replay_multi_asset_decisions`** (backtesting) can simulate one portfolio timeline; live remains per-symbol loops + shared **`RiskEngine`** limits unless you add custom orchestration.

## Limits vs backlog

- **Correlation-aware sizing**, **risk parity**, or **capital allocation across symbols** are **not** implemented as a dedicated module. Operators should size `market_data_symbols` and risk limits to match what independent per-symbol ticks can support.
- For coordinated multi-asset strategies, track future work under **FB-N3** (out of scope) or a new backlog item when scope is defined.

## Related

- [`RISK_PRECEDENCE.MD`](../RISK_PRECEDENCE.MD) — risk ordering
- [`RISK_ENGINE.MD`](RISK_ENGINE.MD) — exposure caps
