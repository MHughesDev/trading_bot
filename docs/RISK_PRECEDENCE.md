# Risk engine — evaluation precedence

When multiple constraints could block a trade, `RiskEngine.evaluate` applies checks in this **fixed order**. The first failing check returns `(None, risk_state)` with `risk_state` updated for the checks that ran.

| Order | Check | Setting / input |
|------:|--------|-----------------|
| 1 | **Feed staleness** | `feed_last_message_at` age vs `NM_RISK_STALE_DATA_SECONDS` (only if `feed_last_message_at` is passed) |
| 2 | **Data timestamp age** | `data_timestamp` vs `NM_RISK_STALE_DATA_SECONDS` |
| 3 | **Spread** | `spread_bps` vs `NM_RISK_MAX_SPREAD_BPS` |
| 4 | **Drawdown** | `current_drawdown_pct` vs `NM_RISK_MAX_DRAWDOWN_PCT` |
| 5 | **No proposal** | `proposal is None` |
| 6 | **System mode** | `MAINTENANCE`, `FLATTEN_ALL`, `PAUSE_NEW_ENTRIES` block new trades |
| 7 | **Per-symbol / total exposure** | `NM_RISK_MAX_PER_SYMBOL_USD`, `NM_RISK_MAX_TOTAL_EXPOSURE_USD` |
| 8 | **Quantity** | Rounded qty ≤ 0 |
| 9 | **REDUCE_ONLY** | New risk-increasing trades blocked (position-aware closes still TODO) |

Live runtime should pass **`feed_last_message_at`** from `CoinbaseWebSocketClient.last_message_at` so a silent feed blocks before bar timestamps.
