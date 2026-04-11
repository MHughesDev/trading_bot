# Multi-asset portfolio behavior (FB-SPEC-07)

## As implemented

- **`market_data_symbols`** drives multiple independent **decision ticks** (one stream per symbol in `live_service` / replay loops).
- **Aggregate exposure (FB-SPEC-07-01):** `RiskEngine.evaluate` enforces **`risk_max_total_exposure_usd`** on *new* notional against **`current_total_exposure_usd`**, and **`risk_max_per_symbol_usd`** per symbol (`default.yaml` `risk:` or **`NM_RISK_MAX_*`**). No extra “tier” caps beyond these two knobs.
- **Cross-symbol policy (FB-SPEC-07-02):** **independent per-symbol** decision ticks; no correlation matrix, risk-parity allocator, or cross-asset hedge logic in the hot path.
- **`RiskEngine`** also applies drawdown, spread, stale-data, mode, and reduce-only rules shared across symbols.
- **`Position reconcile`** (if enabled) is per adapter account; multi-symbol positions remain under the same limits.

## Operator warnings

- Adding symbols multiplies concurrent risk checks; it does **not** add automatic cross-asset netting.
- **`replay_multi_asset_decisions`** can simulate one portfolio timeline; live remains per-symbol loops plus shared **`RiskEngine`** unless you add custom orchestration.

## Limits vs backlog

- **Correlation-aware sizing**, **risk parity**, or **capital allocation across symbols** are **not** implemented as a dedicated module. Operators should size `market_data_symbols` and risk limits to match what independent per-symbol ticks can support.
- For coordinated multi-asset strategies, track future work under **FB-N3** (out of scope) or a new backlog item when scope is defined.

## Related

- [`RISK_PRECEDENCE.MD`](../RISK_PRECEDENCE.MD) — risk ordering
- [`RISK_ENGINE.MD`](RISK_ENGINE.MD) — exposure caps
