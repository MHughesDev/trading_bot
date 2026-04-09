# Risk engine — evaluation precedence

When multiple constraints could block a trade, `RiskEngine.evaluate` applies checks in this **fixed order**. The first failing check returns `(None, risk_state)` with `risk_state` updated for the checks that ran.

| Order | Check | Setting / input |
|------:|--------|-----------------|
| 1 | **Feed staleness** | `feed_last_message_at` age vs `NM_RISK_STALE_DATA_SECONDS` (only if `feed_last_message_at` is passed) |
| 2 | **Data timestamp age** | `data_timestamp` vs `NM_RISK_STALE_DATA_SECONDS` |
| 3 | **Spread** | `spread_bps` vs `NM_RISK_MAX_SPREAD_BPS` |
| 4 | **Drawdown** | `current_drawdown_pct` vs `NM_RISK_MAX_DRAWDOWN_PCT` |
| 5 | **No proposal** | `proposal is None` (skipped for `FLATTEN_ALL` when position ≠ 0) |
| 6 | **MAINTENANCE** | Blocks all |
| 7 | **FLATTEN_ALL** | If `position_signed_qty` ≠ 0, emits closing market action (ignores proposal) |
| 8 | **PAUSE_NEW_ENTRIES** | Blocks when proposal exists |
| 9 | **REDUCE_ONLY** | Blocks buys if long, blocks sells if short; caps qty to reduce position |
| 10 | **Per-symbol / total exposure** | After mode checks |
| 11 | **Quantity** | Rounded qty ≤ 0 |

Live runtime should pass **`feed_last_message_at`** from `CoinbaseWebSocketClient.last_message_at` so a silent feed blocks before bar timestamps.
