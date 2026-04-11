# Multi-asset portfolio behavior (FB-SPEC-07)

## As implemented

- **`market_data_symbols`** drives multiple independent **decision ticks** (one stream per symbol in `live_service` / replay loops).
- **`RiskEngine`** enforces **global** caps (`risk_max_total_exposure_usd`, `risk_max_per_symbol_usd`, drawdown, stale data) shared across symbols — there is **no** explicit cross-symbol correlation or portfolio optimizer in the hot path.
- **`Position reconcile`** (if enabled) is per adapter account; multi-symbol positions are still subject to the same risk limits.

## Limits vs backlog

- **Correlation-aware sizing**, **risk parity**, or **capital allocation across symbols** are **not** implemented as a dedicated module. Operators should size `market_data_symbols` and risk limits to match what independent per-symbol ticks can support.
- For coordinated multi-asset strategies, track future work under **FB-N3** (out of scope) or a new backlog item when scope is defined.

## Related

- [`RISK_PRECEDENCE.MD`](../RISK_PRECEDENCE.MD) — risk ordering
- [`RISK_ENGINE.MD`](RISK_ENGINE.MD) — exposure caps
