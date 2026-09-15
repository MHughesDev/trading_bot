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

## Typed parameters (v1.2, additive) — make a strategy sweepable

Any number you might want to tune belongs in a `parameters` block, not as a
literal. Reference it from an expression as `param('name')`, or embed it in any
string — including feature names — as `{{name}}`. The runtime never sees these:
they are substituted with literals before validation and execution, so the
frozen v1.0 grammar is unchanged. A v1.0 document without a block is still valid.

```json
{
  "strategy_id": "ema_cross_p",
  "definition_version": "1.2",
  "asset_class": "crypto_spot_cex",
  "parameters": {
    "fast": { "type": "int",   "default": 12,   "min": 5,     "max": 50 },
    "slow": { "type": "int",   "default": 26,   "min": 20,    "max": 200 },
    "gate": { "type": "float", "default": 0.02, "min": 0.005, "max": 0.08, "scale": "log" },
    "exit": { "type": "enum",  "default": "trail", "choices": ["trail", "fixed"] }
  },
  "constraints": ["param('fast') < param('slow')"],
  "inputs": [
    { "lane": "market.bars.1m", "instrument": "$bound_at_init" },
    { "lane": "features.technical", "instrument": "$bound_at_init",
      "features": ["ema_{{fast}}", "ema_{{slow}}"] }
  ],
  "nodes": [
    { "id": "n1", "type": "condition",
      "expr": "feature('ema_{{fast}}') - feature('ema_{{slow}}') > param('gate')" },
    { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }
  ],
  "actions": [ { "on_signal": "long", "type": "place_order",
                 "order": { "side": "buy", "size_mode": "fixed", "size": "0.01" } } ]
}
```

Rules the validator enforces: every referenced name is declared; defaults sit
inside `[min, max]`; enum defaults are among `choices`; `constraints` are
`operand cmp operand` where an operand is `param('x')` or a number; a
definition with a block runs on its defaults when no overrides are supplied.
`scale: "log"` tells a sweep to traverse the range multiplicatively.

Declare honest ranges: the sweep can only *narrow* them, never widen. You never
pick a value yourself — `run_sweep` samples the space, and what comes back is
the neighbourhood Study's stable centroid, not the best sample.


## Tool notes

Everything below used to live in a tool's `description`. The whole catalogue is
sent on every model call, so a paragraph in a description is a paragraph paid for
on every turn of every run — 10.7 KB of it, measured (AGENT-005 §5.4). None of it
was wrong; it was in the layer that charges for prose by the turn instead of the
layer that charges for it once.

Fetch a single note with
`get_authoring_guide(section: "tool_notes", tool: "run_sweep")`.

### get_bars

Use it to inspect market behaviour before designing a strategy, or to sanity-check
a backtest window. Responses are capped: you get the full-window summary (return
%, high/low) plus the most recent `max_bars` bars, and earlier bars are summarised
rather than returned. Widen `interval_seconds` for long ranges — a year at 60 s is
half a million bars nobody reads.

### init_asset

Data collection only. It never trades. It starts a background job that backfills
historical bars *and* starts live 1-minute aggregation, so the symbol keeps
current after the backfill finishes. Poll `get_asset_init_job` until the status is
terminal; only then is the symbol backtestable.

### create_strategy

Upserts by `strategy_id` slug, so reusing a slug **overwrites** that strategy.
Mint versioned slugs (`my_strat_v2`) when iterating, or you will lose the variant
you were comparing against — and any Experiment pointing at the slug will silently
start running the new definition.

### get_backtest

It reads; it runs nothing. Use it only for a `backtest_id` you already have.
`detail: "summary"` (the default) truncates long arrays; `detail: "full"` returns
every trade and the whole equity curve.

If the task is to *run* a backtest, this is not the tool — use
`backtest_strategy`. Passing `get_backtest` the id of a training run is the single
most common wrong turn on the local tier, because it is the one tool with the word
in its name.

### wait_for_backtest

Blocks server-side until the run reaches a terminal state or `timeout_seconds`
elapses. On timeout it returns `{timed_out: true, status, progress}` — call it
again. Chaining calls waits out hour-long runs without spending turns on manual
polling, which is the difference between a run that finishes and a run that hits
its step budget watching.

### train_model

Creates the model if the slug is new, trains on real stored bars, and returns the
trained version with its metrics. Deterministic: the same slug, data window and
seed reproduce the same run. Training again with the same slug *retrains that
model* rather than creating a second one.

The REST surface underneath is granular — create, train, poll, list versions,
evaluate, promote — and right to be. This tool is one call because a model a
strategy needs is a precondition, not a research question.

### backtest_strategy

One call: creates the Experiment with a locked holdout, runs the sweep (these are
real backtests), returns the report. Use it whenever the task is to backtest,
evaluate, or measure the edge of a strategy you have created.

The windows and the objective are **derived** when you do not supply them —
210 days of research, a 30-day holdout tail that does not overlap it, and a
Sortino objective with a `min_trades: 20` constraint — and the reply states what
it used. The honest path (`create_experiment` → `run_sweep`) asks for ten fields,
two of them non-overlapping date ranges; measured on the degraded tier, that was
more than the model could assemble, and it reached for `get_backtest` instead on
every retry until the step budget went.

### create_experiment

The unit of honest research. It locks a holdout tail you cannot read until the
vault, declares an immutable objective, and starts a trial counter that every
Study increments irreversibly — which is what deflates significance later. The
research window must not overlap the holdout. Returns the experiment; use its `id`
everywhere else.

`objective` is
`{primary: sortino|calmar|profit_factor|expectancy|detrended_sharpe|sharpe,
constraints: [{kind: min_trades, value: 50}, {kind: max_drawdown_lte, value: 0.15}, ...],
aggregate: median|worst5_pct}`. **Win rate is never a primary.**

### run_sweep

THE inner loop. A sampler (TPE by default) proposes batches; each batch is a
sealed `ParameterSweep` Study with counted trials; then a neighbourhood Study
around the promising region yields the only carry-forward — its
median-stable-centroid member, **never the best sample**.

The report carries `carried_forward` (params to build on), `surface.text`
(per-parameter plateau / cliff / sensitivity — read this; plateaus are robust,
spikes are fragile), `distribution` (sealed), `violation_counts` (why samples were
rejected, e.g. too few trades) and `study_ids`.

It blocks up to `timeout_seconds`; on `timed_out`, call again with the returned
`sweep_id`. These are real backtests: 40 runs on 1h bars takes minutes.

`narrowing` tightens per parameter — `{fast: {min: 8, max: 30}, exit: {choices: ['trail']}}`
— and can only narrow what the strategy declared, never widen it.

### run_study

One honest-evaluation Study on the Experiment, with counted trials. Kinds:
`parameter_sweep`, `neighborhood`, `walk_forward`, `cpcv`, `nested_cv`,
`permutation_null`, `synthetic_paths`, `cost_sweep`, `trade_monte_carlo`,
`regime_conditional`. Prefer `run_sweep` for parameter tuning.

Returns the **sealed** distribution (median, IQR, worst-5%, spread) and the
verdict; member ids are provenance only. Use
`selection_rule: median_stable_centroid` to carry a config forward, then read it
with `get_carried_forward`.

`vary` is tagged by its `vary` field:
`{vary:'params', grid:[{fast:12},{fast:20}]}` ·
`{vary:'neighborhood', param:'fast', center:20, step:2, k:3}` ·
`{vary:'data_windows', windows:[[start,end],...]}` ·
`{vary:'cpcv_groups', n_groups:6, k_test:2}` ·
`{vary:'seeds', n:200}` ·
`{vary:'cost_ladder', cost_model_refs:[...]}` ·
`{vary:'trade_resamples', n:500, block:5}` ·
`{vary:'regimes', windows:[[start,end,label],...]}`.

### get_diagnostics

For a Run you reached through one of your Studies (member ids come from
`list_studies` or a sweep's study). Returns trade P&L shape, win/loss averages,
expectancy, longest losing streak, hold time, MAE/MFE, by-month slices, the ten
worst trades, the longest drawdown episode, exposure and cost drag, plus a compact
`text`.

This is what tells you *what* to change. Read it before proposing a structural
edit — a sweep tells you a parameter is wrong, this tells you the idea is.

### advance_gate

Runs the next gate, and blocks for minutes. Gate 2 runs CPCV plus synthetic paths
and the worst-5% must survive. Gate 3 runs the chosen permutation null with
selection-bias correction taken from the live trial counter, and requires deflated
Sharpe ≥ 0.95 and PBO ≤ 0.5. Choose a null (`choose_null`) before Gate 3.

### choose_null

Declares the primary significance null. Logged and immutable — there is no second
choice, which is the point: picking the null after seeing which one flatters the
result is the oldest way to fake significance.

Kinds: `signal_return_decouple`, `block_permutation`, `stationary_bootstrap`,
`bar_permutation`, `synthetic_garch`, `regime_block`, `random_entry_matched`.
`get_null_picker` says which is recommended for this strategy type and what each
one preserves and destroys; departing from the recommendation requires
`override_reason`.
