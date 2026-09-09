# Strategy Authoring Guide (trading-bot MCP)

You are talking to a trading-strategy research platform. Through these tools you can
**design trading algorithms, backtest them against real historical market data, read
the results, and iterate**. You cannot place orders or trade — there is deliberately
no order tool on this server.

## The recommended loop

1. `list_instruments` — see which instruments have stored bar history (and how much,
   per timeframe). Only backtest windows inside an instrument's coverage, or set
   `auto_collect: true` to backfill. If a symbol you want isn't there at all,
   `init_asset` onboards it (background seeding — poll `get_asset_init_job`).
   `get_bars` shows real OHLCV history with summary stats — look at the market
   before you design for it.
2. Write a strategy definition (JSON, format below). Prefer writing the full JSON and
   checking it with `validate_strategy` — it returns structured, fixable errors.
   (The step-by-step `new_strategy_draft` builder tools exist as an alternative.)
3. `create_strategy` — persists the definition. **`strategy_id` is an upsert key**:
   re-using a slug overwrites that strategy. When iterating, either intentionally
   update the same slug or mint versioned slugs (`my_strat_v1`, `my_strat_v2`, …)
   to keep every variant comparable.
4. `create_backtest` — launch a run. Returns a `backtest_id` immediately; the run
   proceeds in the background and can take minutes to over an hour for large windows.
5. `wait_for_backtest` — blocks server-side until the run finishes or the call's
   `timeout_seconds` elapses. On timeout it returns current progress and you simply
   call it again — chain calls for hour-long runs. (`get_backtest` gives a one-shot
   snapshot if you just want to peek.)
6. Read the results (`get_backtest` with `detail: "full"` for everything), decide what
   to change, go to step 2 with a new slug.

Backtest capacity is limited (3 concurrent runs, shared with the human UI). A run
sitting in `queued` is normal — just wait. A `create_backtest` rejection with
`invalid_request` may also mean the queue or data checks failed; read the message.

## Strategy definition format (v1.0, frozen)

A strategy is one canonical JSON document:

```json
{
  "strategy_id": "ema_cross_v1",
  "definition_version": "1.0",
  "asset_class": "crypto_spot_cex",
  "inputs": [
    { "lane": "market.bars.1m", "instrument": "$bound_at_init" },
    { "lane": "features.technical", "instrument": "$bound_at_init",
      "features": ["ema_7", "ema_21"] }
  ],
  "nodes": [
    { "id": "c1", "type": "condition", "expr": "feature('ema_7') > feature('ema_21')" },
    { "id": "s1", "type": "signal", "when": "c1", "emit": "entry" }
  ],
  "actions": [
    { "on_signal": "entry", "type": "place_order",
      "order": { "side": "buy", "size_mode": "fixed", "size": "0.05" } }
  ]
}
```

- `strategy_id` — slug, `[a-z0-9_]`, the upsert key.
- `asset_class` — use `"crypto_spot_cex"` (this platform's v1 focus is crypto).
- `inputs` — lane subscriptions. `"$bound_at_init"` means "the instrument this
  strategy is applied to" — always use it so the strategy is instrument-portable.
  Declare a `market.bars.<tf>` lane to pin the evaluation timeframe (e.g.
  `market.bars.5m`); if you omit it, the backtest's `timeframe` parameter decides.
  Declare every feature you reference on a `features.technical` input.
- `nodes` — the decision graph:
  - `{ "id", "type": "condition", "expr" }` — boolean predicate (grammar below).
  - `{ "id", "type": "signal", "when": "<condition node id>", "emit": "<name>" }` —
    emits a named signal on the **rising edge** of the condition (fires when the
    condition flips from false to true, not on every true bar).
- `actions` — `{ "on_signal", "type": "place_order", "order": { "side": "buy"|"sell",
  "size_mode": "fixed"|"percent_of_balance"|"risk_unit", "size": "<decimal string>" } }`.
  Sizes are decimal **strings**, never floats.
  **Backtests support only `size_mode: "fixed"`** (a base-asset quantity, e.g. `"0.05"`
  BTC) — the simulator rejects `percent_of_balance`/`risk_unit` strategies at the
  simulating phase. Use fixed sizing for anything you intend to backtest; the other
  modes exist for live/paper execution.
- `risk_overrides` (optional) — `{ "max_position": "<decimal>",
  "max_order_rate_per_minute": N, "max_order_rate_per_second": N }`. **Tighten-only**:
  overrides may only be stricter than the platform defaults; loosening fails validation.

## Expression grammar (frozen — nothing else parses)

```
expr        = comparison
comparison  = term ( ( ">" | "<" | ">=" | "<=" | "==" | "!=" ) term )?
term        = factor ( ( "+" | "-" ) factor )*
factor      = unary ( ( "*" | "/" ) unary )*
unary       = "-" unary | primary
primary     = number | feature('name') | bar('field') | "(" expr ")"
field       = open | high | low | close | volume
number      = decimal literal (never scientific notation)
```

Examples:
- `feature('ema_7') > feature('ema_21')`
- `feature('rsi_14') < 30`
- `bar('close') > bar('open') * 1.002`
- `(feature('ema_7') - feature('ema_21')) / feature('ema_21') > 0.001`

There are **no** functions beyond `feature()` and `bar()` — no abs/min/max, no lookback
indexing, no cross() helper. Express crossovers as a plain comparison; the signal
node's rising-edge semantics turn it into a cross event.

## Features available in backtests

Backtests compute indicators during replay. Supported feature names:

- `ema_N` — exponential moving average, any period N ≥ 1 (e.g. `ema_7`, `ema_50`, `ema_200`)
- `rsi_N` — relative strength index, period N ≥ 2 (e.g. `rsi_14`)

Anything else (`macd_*`, `sma_*`, custom names) is **rejected** by the backtest with
`unsupported feature`. Warm-up is handled automatically (EMA gets 5×period bars of
lead-in, RSI period+1) — you never need to pad your window for warm-up.

## Choosing backtest windows (crypto)

- Crypto trades 24/7; instrument ids look like `BTC-USD`, `ETH-USD` (see
  `list_instruments` for what actually has data).
- Pick the timeframe to match the idea: `1m`/`5m` for intraday signals, `1h`/`4h`
  for swing logic. Bar count drives runtime: 90 days of 1m bars ≈ 130k bars (slow,
  possibly an hour with data collection); 90 days of 1h ≈ 2.2k bars (fast).
  Start iterations on `1h`, confirm the survivor on finer timeframes.
- `start`/`end` are RFC3339 UTC (e.g. `"2026-06-01T00:00:00Z"`).
- `auto_collect: true` (default) backfills missing history before simulating —
  this can dominate runtime on first touch of an instrument/timeframe.

## Reading results

`wait_for_backtest` / `get_backtest` return the run snapshot: `status`
(`queued → checking_data → [collecting_data] → loading_data → simulating →
completed | failed | cancelled`), `progress` (0–100), `error` + `failed_phase` on
failure, and on completion a `result` document with orders, positions, PnL and
return statistics plus an equity curve. `detail: "summary"` (default) truncates
long arrays to keep responses small; use `detail: "full"` when you need every trade.

**Include an exit.** A strategy with only a buy action never closes a position, so
`stats_returns` (Sharpe, profit factor, win rate) come back null — there are no
round-trips to measure. Pair the entry with an opposing condition/signal/action
(e.g. sell when `feature('ema_7') < feature('ema_21')`) so trades close and the
return statistics become meaningful.

Judge a strategy on: net PnL vs buy-and-hold, max drawdown, number of trades (a
2-trade wonder is noise, not edge), and stability across instruments/windows. Beware
overfitting: a parameter set tuned on one window should be re-checked on a different
window before you call it good.

`compare_backtests` puts 2–5 runs side by side (headline metrics only) — use it to
pick between variants on evidence rather than memory. `rerun_backtest` re-executes a
spec unchanged; `delete_backtest` cleans up failed experiments.

## Supporting tools

- **Data**: `get_bars` (OHLCV + summary stats, size-capped), `get_instrument`,
  `list_asset_classes`, `list_initialized_assets`, `init_asset` / `get_asset_init_job`
  (onboard a new symbol — data collection only).
- **Strategy context**: `list_compatible_strategies` (what already exists that runs on
  this instrument), `list_models` / `get_model` (registered AI models; v1.1 definitions
  can bind their outputs into `feature('…')` slots via Inference nodes).
- **Observability (read-only)**: `get_dashboard_rollup`, `get_paper_activity`,
  `get_trading_status`, `get_order`. There is no order-placement tool; nothing you do
  here trades.
