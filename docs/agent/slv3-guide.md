# SLv3 — writing a `tbot.strategy/3` strategy

The strategy is a graph (FEAT-007 §5, ADR-0040). SLv3 is its text; the visual editor and builder
tool-calls are its other two views, and all three are the same strategy (ADR-0045). Every finding
the validator returns names one of the sections below. The operator and feature tables at the end
are generated from the registries the compiler checks against, and a test keeps them equal — so
nothing appears here that the platform does not have, and nothing it has is missing.

## Syntax

Statements start with a keyword; layout, line breaks and `# comments` are not part of the strategy,
so reformatting never changes its revision.

```
strategy ema_cross kind trading asset_class crypto_spot_cex timeframe 1h
param fast: int(default: 12, min: 5, max: 50)
let f = ema(close, $fast)
let s = ema(close, 48)
when crossed_above(f, s) and state.flat -> target(size: equity_fraction(x: 0.5))
when crossed_below(f, s) -> close
```

## Structure

The header names the strategy, its `kind` (`trading` or `selection`), its `asset_class` and its
decision `timeframe` (`1s 1m 5m 15m 1h 4h 1d`). Then, in any order: `description`, `param`,
`constraint`, `input`, `universe` (selection), `let`, rules (trading), `select` (selection),
`triggers {…}` and `risk {…}`. A trading strategy ends in rules; a selection strategy in a
`universe` and a `select`. A v1 definition is another format: translate it, do not edit it.

## Values

A bare name is a **reference**: a built-in source (`close`, `high`, …), a node (`f`), a field of a
node's record (`vol.sigma`), a parameter (`$fast`) or the account (`state.flat`, `state.long`,
`state.position`, `state.equity`, …). A quoted string is a **literal** (`"up"`). Numbers, `true`,
`false`, lists `[a, b]`, operators `a < b`, `a and b`, `not a`, `a + b`, `-x`, and calls `op(…)`.

## Nodes

`let name = op(…)` binds a node. Positional arguments bind to the operator's ports in the order
the table below shows; any port can be named instead (`ema(x: close, n: 20)`). Node names are
letters, digits and `_`, and unique. A `let` binds an operator — a reference needs no `let`.

## Params

`param fast: int(default: 12, min: 5, max: 50)`, `float(…)`, or `enum(default: long, choices: [long,
short])`; read as `$fast`. The default lies inside the range. `constraint "param('fast') <
param('slow')"` relates two parameters; a sweep is checked against both.

## Intents

What a rule does, written `kind(field: value, …)`: `hold`, `close` (optional `fraction`, `via`),
`target(size: …)`, `add`, `reduce`, `order(side: buy, size: …, style: …)`, `bracket(entry: …, stop:
…, take_profit: …, trailing: …)`, `protect(stop: …)`, `cancel`. Sizes are `units(x: …)`,
`notional(x: …)`, `equity_fraction(x: …)`, `risk(fraction: …, stop: …)`, `vol_target(target: …,
vol: …)`. Styles are `market`, `limit(price: …)`, `stop(trigger: …)`, `stop_limit(…)`,
`trailing_stop(trail: …)`. A distance is `{amount: 2, unit: atr(n: 14)}` — `pct`, `abs`,
`atr(n: …)` or `sigma(of: …)` — always with a unit and a positive amount; `from: mark` or
`high_water` changes its anchor.

## Rules

`when <condition> -> <intent>`; `rule name group g when …` names a rule or puts it in a group.
Within a group the first rule that matches wins, so two rules with the same condition in one group
leave the second unreachable. A strategy with no rules never acts. `otherwise hold` is implicit.
A condition that is missing (a model that timed out, an indicator warming up) holds, and the
decision journal says why.

## Selection

`universe list(instruments: [A, B])`, `asset_class(asset_class: …)` or `named(name: …)`. Then
`select <score> where <filter> top 10 order desc attribute weight = <value>`. The `cs_*` operators
compare members; every member is selected or excluded with its reason.

## Warmup

A strategy needs enough history for its longest lookback before its first decision. A window
shorter than that has nothing to decide on — start it earlier or shorten the lookback.

## Features

`feature("rsi_14")` reads a DATA-006 indicator computed from closed bars. The name includes its
parameters; the table below gives one that resolves for each family.

## Operators

The table below: each operator's ports, what it gives, and when to reach for it.

## Models

`model("name@version", port: value, …)` calls a registered model version; its inputs and outputs
are its contract (`get_strategy_catalogue` lists every callable version). Read an output as a
field: `m.sigma`, `m.probs`, `m.value`.

## Builder

Builder tool-calls build the same graph one step at a time: `new`, `add_node`, `connect`, `set_arg`,
`remove_node`, `add_rule`, `set_param`, `set_universe`, `set_select`. A builder string is always a
literal; references go through `connect`.

## Reference

<!-- generated from the registries by strategy_lang::v3::catalogue::guide_block; do not edit -->

### Operators

Positional arguments bind to the ports in the order shown; `lit` ports take a literal or a `$param`.

| operator | ports | gives | use when |
|---|---|---|---|
| `feature` | name: text (lit) | num | Read any DATA-006 indicator by name — rsi_14, atr_14, roc_20, bb_width_20 — computed from closed bars. |
| `bar` | field: text (lit), input: text (lit) (optional) | num | Read a closed bar's open/high/low/close/volume, of this instrument or a declared input. |
| `hour` |  | num | The decision instant's UTC hour, 0–23, for time-of-day rules. |
| `weekday` |  | num | The decision instant's UTC weekday, 0 = Monday, for day-of-week rules. |
| `selection_attr` | selection: text (lit), attr: text (lit) | num | Read an attribute a selection strategy computed for this instrument — e.g. its momentum rank. |
| `features` | set: text (lit), lookback: num (lit), timeframe: text (lit) (optional) | window | Build a model's input window from a named feature set and a lookback. |
| `add` | a: num, b: num | num | a + b. |
| `sub` | a: num, b: num | num | a − b. |
| `mul` | a: num, b: num | num | a × b. |
| `div` | a: num, b: num | num | a ÷ b; missing (undefined) when b is zero. |
| `neg` | x: num | num | −x. |
| `abs` | x: num | num | \|x\|. |
| `sqrt` | x: num | num | √x; missing for negative x. |
| `log` | x: num | num | Natural log; missing for x ≤ 0. |
| `exp` | x: num | num | eˣ; missing on overflow. |
| `sign` | x: num | num | −1, 0 or 1. |
| `pow` | base: num, exp: num | num | base^exp. |
| `min` | a: num, b: num | num | The smaller of two values. |
| `max` | a: num, b: num | num | The larger of two values. |
| `clip` | x: num, lo: num, hi: num | num | Bound x to [lo, hi] — e.g. a model's size fraction. |
| `round` | x: num, digits: num (lit) | num | Round to a number of decimal places. |
| `gt` | a: num, b: num | bool | a > b. |
| `lt` | a: num, b: num | bool | a < b. |
| `ge` | a: num, b: num | bool | a ≥ b. |
| `le` | a: num, b: num | bool | a ≤ b. |
| `eq` | a: num, b: num | bool | a = b, exactly. |
| `ne` | a: num, b: num | bool | a ≠ b. |
| `between` | x: num, lo: num, hi: num | bool | lo ≤ x ≤ hi. |
| `is` | x: cat<L>, label: text (lit) | bool | Whether a category — e.g. a classifier's predicted class — is a given label. |
| `and` | a: bool, b: bool | bool | Both. False if either is false, even when the other is missing. |
| `or` | a: bool, b: bool | bool | Either. True if either is true, even when the other is missing. |
| `not` | x: bool | bool | Negation; missing stays missing. |
| `if` | cond: bool, then: T, else: T | T | Choose between two branches — each may be its own chain of models and indicators. |
| `switch` | on: cat<L>, default: T, …named | T | Choose a branch per label — e.g. a different sizing per detected regime. |
| `prev` | x: T, n: num (lit) | T | The value n decision ticks ago. |
| `change` | x: num, n: num (lit) | num | x minus its value n ticks ago. |
| `crossed_above` | a: num, b: num | bool | True on the tick a moves from at-or-below b to above it. |
| `crossed_below` | a: num, b: num | bool | True on the tick a moves from at-or-above b to below it. |
| `rising` | x: num, n: num (lit) | bool | Strictly increasing over the last n ticks. |
| `falling` | x: num, n: num (lit) | bool | Strictly decreasing over the last n ticks. |
| `rolling_mean` | x: num, n: num (lit) | num | Mean of any value over its last n ticks — including a model's output. |
| `rolling_std` | x: num, n: num (lit) | num | Sample standard deviation of any value over its last n ticks. |
| `rolling_min` | x: num, n: num (lit) | num | Lowest value over the last n ticks. |
| `rolling_max` | x: num, n: num (lit) | num | Highest value over the last n ticks. |
| `rolling_sum` | x: num, n: num (lit) | num | Sum over the last n ticks. |
| `ema` | x: num, n: num (lit) | num | Exponential moving average of any value; of a bar field it is the DATA-006 ema_n. |
| `bars_since` | cond: bool | num | Ticks since cond was last true; missing until it has been. |
| `count_true` | cond: bool, n: num (lit) | num | How many of the last n ticks cond was true. |
| `latch` | set: bool, reset: bool | bool | Becomes true when set fires and stays true until reset fires. |
| `hysteresis` | x: num, on: num, off: num | bool | True once x rises above on, false again only when it falls below off — no flip-flopping. |
| `hold_last` | x: T | T | The last present value of x, when the current one is missing — e.g. a slow model's last answer. |
| `cooldown` | cond: bool, n: num (lit) | bool | cond, but at most once every n ticks. |
| `model` | ref: text (lit), …named | per contract | Call any MLOps model by name@version; its inputs and outputs are typed by its contract. |
| `argmax` | p: probs<L> | cat<L> | The most probable label of a classifier's probabilities. |
| `prob` | p: probs<L>, label: text (lit) | num | The probability a classifier gives one label. |
| `quantile` | d: dist, q: num (lit) | num | A quantile of a distributional forecast — e.g. q=0.25 as a limit price offset. |
| `mean` | d: dist | num | The mean implied by a distributional forecast's quantiles. |
| `prob_above` | d: dist, x: num | num | The probability a distributional forecast puts above x. |
| `cosine` | a: vec, b: vec | num | Cosine similarity of two embeddings. |
| `parse` | text: text, schema: record (lit) | per schema | Turn an LLM's reply into typed fields; missing when it does not match the schema. |
| `weighted_mean` | xs: [num], weights: [num] (lit) | num | Blend several values — indicators, models, or both — with fixed weights. |
| `vote` | xs: [bool], k: num (lit) | bool | True when at least k of the conditions are true. |
| `zscore` | x: num, n: num (lit) | num | How unusual x is against its own last n ticks. |
| `pct_rank` | x: num, n: num (lit) | num | Percentile of x within its own last n ticks, 0–1. |
| `cs_rank` | x: num | num | Rank of x across the universe, 1 = highest. *(selection only)* |
| `cs_zscore` | x: num | num | x standardised across the universe at this instant. *(selection only)* |
| `cs_pct_rank` | x: num | num | Percentile of x across the universe, 0–1. *(selection only)* |
| `cs_mean` | x: num | num | Mean of x across the universe — the same for every member. *(selection only)* |
| `cs_median` | x: num | num | Median of x across the universe. *(selection only)* |
| `cs_count` | cond: bool | num | How many universe members satisfy cond — breadth. *(selection only)* |
| `coalesce` | x: T, default: T | T | x, or default when x is missing — making a fallback a decision rather than an accident. |
| `is_missing` | x: T | bool | Whether x is missing — never itself missing. |
| `missing_reason` | x: T | text | Why x is missing, as text; missing when x is present. |

### Features

Read with `feature("<name>")`. A family with parameters is named with them: the example column is a name that resolves.

| example | warm-up bars | use when |
|---|---|---|
| `open` | 1 | the bar's open; use as the reference for an intrabar move |
| `high` | 1 | the bar's high |
| `low` | 1 | the bar's low |
| `close` | 1 | the bar's close; the default source of every windowed family |
| `volume` | 1 | the bar's volume; compare with rel_volume_N rather than an absolute level |
| `returns_20` | 21 | simple return over n bars; the usual momentum primitive |
| `momentum_20` | 21 | same value as returns_n (alias of `returns`) |
| `ret_20` | 21 | same value as returns_n (DATA-006 spelling) (alias of `returns`) |
| `log_returns_20` | 21 | log return over n bars; additive across bars, so use it when summing returns |
| `logret_20` | 21 | same value as log_returns_n (alias of `log_returns`) |
| `lag_20` | 21 | the close n bars ago; compare a level with its own past |
| `diff_20` | 21 | close minus close n bars ago, in price units, not a ratio |
| `rolling_mean_20` | 20 | mean close over n bars; the trend baseline a price is compared against |
| `sma_20` | 20 | same value as rolling_mean_n (alias of `rolling_mean`) |
| `rolling_std_20` | 20 | sample standard deviation of close over n bars; the sigma unit of an exit distance |
| `rolling_var_20` | 20 | sample variance of close over n bars |
| `rolling_min_20` | 20 | lowest close over n bars |
| `rolling_max_20` | 20 | highest close over n bars |
| `rolling_sum_20` | 20 | sum of close over n bars |
| `rolling_median_20` | 20 | median close over n bars; an outlier-robust baseline |
| `rolling_quantile_20_50` | 20 | the q-th percentile close over n bars, q in whole percent |
| `rolling_skew_20` | 20 | sample skewness of close over n bars |
| `rolling_kurt_20` | 20 | excess kurtosis of close over n bars; high values mean fat tails |
| `zscore_20` | 20 | how many sigma the close is from its n-bar mean; the mean-reversion primitive |
| `pct_rank_20` | 20 | fraction of the last n closes below this one, 0..1; use to gate on a regime percentile |
| `ewma_10` | 80 | exponentially weighted mean with the given halflife in bars |
| `ema_20` | 100 | exponential moving average of close over n bars |
| `rsi_20` | 101 | Wilder RSI of close, 0..100; below 30 is the conventional oversold gate |
| `atr_20` | 101 | Wilder average true range in price units; the unit an exit distance should use |
| `roc_20` | 21 | rate of change over n bars, in percent |
| `macd_12_26` | 130 | ema_fast minus ema_slow; positive means the fast trend leads |
| `macd_signal_12_26_9` | 175 | the signal-period ema of macd_fast_slow |
| `macd_hist_12_26_9` | 175 | macd minus its signal line; the crossover primitive |
| `bb_upper_20_2` | 20 | rolling mean plus k sample standard deviations of close |
| `bb_lower_20_2` | 20 | rolling mean minus k sample standard deviations of close |
| `bb_width_20_2` | 20 | band width as a fraction of the mean; a squeeze is a low value |
| `bb_pctb_20_2` | 20 | where close sits in the band, 0 at the lower and 1 at the upper |
| `donchian_upper_20` | 20 | highest high over n bars; the breakout level |
| `donchian_lower_20` | 20 | lowest low over n bars |
| `donchian_mid_20` | 20 | midpoint of the n-bar high-low channel |
| `donchian_width_20` | 20 | channel height as a fraction of the midpoint |
| `keltner_upper_20_2` | 101 | ema_n plus k atr_n; a volatility band that does not assume normality |
| `keltner_lower_20_2` | 101 | ema_n minus k atr_n |
| `obv` | 256 | on-balance volume over a fixed 256-bar window |
| `rel_volume_20` | 20 | this bar's volume over the n-bar mean; 1.0 is an ordinary bar |
| `hour_sin` | 1 | sine of the UTC hour; pair with hour_cos so midnight is next to 23:00 |
| `hour_cos` | 1 | cosine of the UTC hour |
| `dow_sin` | 1 | sine of the UTC day of week |
| `dow_cos` | 1 | cosine of the UTC day of week |
| `utc_session_asia` | 1 | 1 inside 00:00-08:00 UTC, else 0; a UTC window, not an exchange calendar |
| `utc_session_london` | 1 | 1 inside 07:00-16:00 UTC, else 0; a UTC window, not an exchange calendar |
| `utc_session_us` | 1 | 1 inside 13:00-22:00 UTC, else 0; a UTC window, not an exchange calendar |
| `parkinson_vol_20` | 20 | Parkinson range volatility over n bars; not annualised |
| `garman_klass_vol_20` | 20 | Garman-Klass volatility over n bars; uses the open as well as the range |
| `rogers_satchell_vol_20` | 20 | Rogers-Satchell volatility over n bars; drift-independent |
| `rv_20` | 21 | realised volatility: root sum of squared bar log returns over n bars |
| `bv_20` | 21 | bipower variation over n bars; jump-robust, so rv minus bv isolates jumps |
| `jump_share_20` | 21 | share of realised variance attributable to jumps, 0..1 |
| `autocorr_20_1` | 22 | autocorrelation of bar returns at the given lag over n bars |
| `variance_ratio_20_4` | 29 | Lo-MacKinlay variance ratio at horizon q; above 1 is trending, below 1 mean-reverting |
| `efficiency_ratio_20` | 21 | net move over total path length, 0..1; the persistence filter a trend rule wants |
| `yang_zhang_vol_20` | 20 | Yang-Zhang volatility over n bars; handles gaps between bars as well as the range |
| `hurst_257` | 257 | rescaled-range Hurst exponent of bar returns; above 0.55 trending, below 0.45 mean-reverting |
| `fracdiff_20_40` | 20 | fractional difference of log close, order d/100 over n bars; stationary but keeps memory |
| `lookback_ensemble_zscore_10_20_40` | 40 | equal-weight mean of zscore over three lookbacks; a mean-reversion rule that does not hinge on one window |
| `lookback_ensemble_returns_10_20_40` | 41 | equal-weight mean of simple returns over three lookbacks; a momentum rule that does not hinge on one window |

<!-- end generated -->
