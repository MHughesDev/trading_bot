# Risk engine — evaluation precedence

When multiple constraints could block a trade, `RiskEngine.evaluate` applies checks in this **fixed order**. The first failing check returns `(None, risk_state)` with `risk_state` updated for the checks that ran.

Layered **canonical notional** sizing (`risk_engine/canonical_sizing.compute_canonical_notional`) runs **after** the hard gates and system modes below, using `ActionProposal.size_fraction`, `RiskState` canonical inputs, and `AppSettings` exposure caps.

| Order | Check | Setting / input |
|------:|--------|-----------------|
| 1 | **Feed staleness** | `feed_last_message_at` age vs `NM_RISK_STALE_DATA_SECONDS` (only if `feed_last_message_at` is passed) |
| 2 | **Data timestamp age** | `data_timestamp` vs `NM_RISK_STALE_DATA_SECONDS` |
| 3 | **Spread** | `spread_bps` vs `NM_RISK_MAX_SPREAD_BPS` |
| 4 | **Drawdown** | `current_drawdown_pct` vs `NM_RISK_MAX_DRAWDOWN_PCT` |
| 5 | **Product tradable** | `product_tradable` |
| 6 | **MAINTENANCE** | Blocks all |
| 7 | **FLATTEN_ALL** | If `position_signed_qty` ≠ 0, emits closing market action (ignores proposal) |
| 8 | **No proposal** | `proposal is None` (skipped for `FLATTEN_ALL` when position ≠ 0) |
| 9 | **PAUSE_NEW_ENTRIES** | Blocks when proposal exists |
| 10 | **REDUCE_ONLY** | Blocks buys if long, blocks sells if short |
| 11 | **Canonical notional** | `compute_canonical_notional` → `last_risk_sizing`; zero notional blocks |
| 12 | **Quantity** | Rounded qty from notional / `mid_price`; zero blocks |
| 13 | **Available cash** | BUY blocked if `notional > available_cash_usd` when set |
| 14 | **REDUCE_ONLY qty cap** | Clips quantity to remaining position |

Live runtime should pass **`feed_last_message_at`** from **`KrakenWebSocketClient.last_message_at`** (`data_plane/ingest/kraken_ws.py`) so a silent feed blocks before bar timestamps.

**Signing:** `RiskEngine.to_order_intent` HMAC-signs intents when a signing secret is configured (`execution/intent_gate` enforces).
