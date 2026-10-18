# SLv2 — the strategy language, one section at a time

Served by `get_slv2_guide(section)`. Every section is at most 4 KB so a small model
fetches the one a diagnostic's `see` field names (`guide#exits.units` is the section
`exits.units`). SLv2 is the text of v1 strategy definitions; SLv3 has its own guide and
its own tools (`*_strategy_v3`).

## The language in one page {#grammar}

A strategy is a few lines, one clause each, keyword first. Order does not matter after
the header. A `#` starts a comment; a comment above a clause is attached to it when the
text is parsed, but it is not stored in the saved definition.

```
strategy trend_demo on $bound_at_init timeframe 1h
param fast: int[5..50 step 1] = 7
param slow: int[20..200 step 5] = 21
constraint param('fast') < param('slow')
input fast_ma = ema(close, fast)
input slow_ma = ema(close, slow)
entry long when fast_ma > slow_ma and rsi(close, 14) < 60
size fixed 0.01
exit stop 2 % from entry
exit target 5 % from entry
exit time 48 bars
risk max_position 0.5
```

The clauses: `strategy` (header), `version`, `asset_class`, `trust`, `param`, `constraint`,
`input`, `entry`, `size`, `exit`, `order`, `risk`. An unknown first word lists them.

Every rule that makes a model's life easier is enforced by the compiler: units are
explicit on every distance, a call used twice must be an `input`, an inline expression
over 120 characters must be an `input`, and a refused construct is refused by name with
the mechanism that is missing. `compile_strategy` returns every diagnostic at once.

## Header, version, asset class, trust {#grammar.header #header}

`strategy <name> on <instrument> timeframe <tf>` — first line, once.

- `<name>` becomes the strategy id (the slug). **Keep the same name while you iterate:**
  saving appends a revision whose parent is the previous one, and a new name starts a
  separate history.
- `<instrument>` is `$bound_at_init` (reusable on any instrument, the usual choice) or a
  quoted instrument such as `'BTC-USD'`.
- `<tf>` is a number and a unit, such as `1m`, `5m`, `1h`, `4h` or `1d`.

Optional, each on its own line: `version 1.4` (pins the definition version; leave it off
and the compiler picks the one it needs: 1.3, or 1.4 when a scaled unit is used), `asset_class
crypto_spot_cex` (the default), `trust centralized_exchange`.

## Expressions {#grammar.expressions #expressions}

Conditions and inputs are expressions over numbers, bar fields and features.

- Comparisons: `>` `<` `>=` `<=` `==` `!=`. Do not chain them: `a < b < c` is two
  comparisons joined by `and`.
- Logic: `and`, `or`, `not`, with the usual precedence (`not` binds tightest).
- Arithmetic: `+ - * /` and unary minus, on numbers and features.
- Bar fields: `open`, `high`, `low`, `close`, `volume` — the bar being decided on.
- Features: a call such as `ema(close, 20)` or `rsi(close, 14)`. The first argument is the
  source (`close`), the rest are whole numbers or declared `param` names. `list_features`
  gives every family and its example; see `guide#features`.
- Declared names: an `input`'s name, or a `param`'s name inside a feature call.
- `between(x, lo, hi)` is true when `x` is inside the range.

A condition is evaluated on every bar close. A comparison of a feature with a feature
(`fast_ma > slow_ma`) is a level test: it is true on every bar the relation holds, not
only on the bar it starts holding. See `guide#functions.history` for what that means for
crossings.

## Functions the compiler provides {#functions}

Provided: `between`, the comparison and logic operators above, and arithmetic.

Refused by name, with the reason and what to write instead:

- `abs`, `min`, `max`, `clip`, `log`, `exp`, `sqrt`, `sign` — `needs_operator`: the
  runtime has no such operation. Compare against a threshold, or find a feature family
  that computes the quantity (`list_features(query="…")`).
- `flat`, `long`, `short`, `bars_in_trade`, `unrealized_return`, `last_exit_reason` —
  `needs_position_state`: a condition cannot read the position it would open or close.
  Put the logic in an `exit` clause or a cooldown outside the strategy.
- `crossed_above`, `crossed_below`, `rising`, `falling` — `needs_history`, see
  `guide#functions.history`.

A feature cannot take another feature as its source: `pct_rank(rv(close, 20), 200)` does
not exist. Use the family that already has the shape you want (`pct_rank`, `zscore`,
`variance_ratio`, `efficiency_ratio`), or compare two features with each other.

## Crossings and history {#functions.history}

`crossed_above(a, b)` is refused (`needs_history`) and the refusal is correct. The runtime
has the operations for it, but the stored expression grammar has no such function, so a
strategy using one could not be saved, and the backtest evaluates conditions without the
previous bar.

Write the level test that says the same thing:

- `macd(close, 7, 21) > 0` for "the fast average is above the slow one";
- `fast_ma > slow_ma` with both bound as `input`s.

A level test fires on every bar the relation holds, not just the first. The exits and the
risk gate bound what that does: `exit time 48 bars` limits the holding period, and one
position is open at a time. If you need the first bar only, the closest honest form is a
tight band: `fast_ma > slow_ma and fast_ma < slow_ma * 1.002`.

Do not "approximate" a crossing with a comparison of the current bar against a lagged
one (`lag_1`): a feature of a feature does not exist, and the two are not the same rule.

## Entries {#entries}

`entry long when <condition>` or `entry short when <condition>`. Write several entries
for several alternatives; each is its own entry.

`<condition>` is a boolean expression (`guide#grammar.expressions`). Write it as a
conjunction of short clauses — `a > b and c < 30 and d > 1.5` — rather than one nested
expression: the blocking profile (`blocking_profile`) reads the clauses one by one and
tells you which one stopped the entry.

Refused by name: `cooldown N bars` and `hysteresis` (`clause_not_representable` — the
stored form has nowhere to keep them). A re-entry delay can be written as an `exit time`
long enough that the position outlives the signal.

## Inputs {#inputs}

`input <name> = <feature call>` names one feature. A name is mandatory for a call used
in two places and for any expression over 120 characters; the compiler says which and
what to write.

```
input z = zscore(close, window)
entry long when z < stretch
```

An input is a single feature call, nothing more (`input_not_a_feature`): write arithmetic
in the clause that uses it. An input nothing references is refused (`unused_input`).
Parameters go in the call's number positions: `ema(close, fast)` — the stored form
becomes `ema_{{fast}}`, which the sweep fills in.

## Parameters {#params #parameters}

`param <name>: int[lo..hi step s] = default`, `param <name>: float[lo..hi] = default`
(add `log` after the range to sweep on a log scale), or `param <name>: enum[a, b, c] = a`.

```
param window: int[10..100 step 5] = 20
param stretch: float[-4..-0.5] = -2
param mode: enum[fast, slow] = fast
```

Every number worth sweeping is a `param` with a range; the default must be inside it
(`bad_param_spec`). The sweep draws values; you do not pick them. A parameter used as a
feature argument makes the strategy sweepable. A literal that looks tuned and is not a
`param` is a number nobody can vary.

## Parameter constraints {#params.constraints}

`constraint <comparison of params>` rejects a sampled combination before a run exists.

```
constraint param('fast') < param('slow')
```

The text after the keyword is kept as written and read by the platform's own constraint
parser, so it is checked where it is written: `constraint blah` is refused here, not at
sweep time. Reference parameters as `param('name')`. Use it whenever the strategy only
makes sense for some combinations — a fast average slower than the slow one inverts the
rule rather than weakening it.

## Size {#size}

One `size` clause, required when there is an entry:

- `size fixed 0.01` — a fixed quantity per order;
- `size equity_fraction 0.1` — a fraction of equity;
- `size risk_per_trade 0.01` — the fraction of equity at risk, given the stop distance.

`vol_target` and `kelly` are refused (`size_mode_not_representable`): none of the three
modes scales a quantity by a measured volatility. `risk_per_trade` is the nearest rule
that exists and it is a different rule; the skeleton `vol_target` says so in its notes.

## Order types {#orders #actions}

Orders are market orders. `order market` is accepted and means the same as no `order`
clause. `order limit`, `order stop` and `order stop_limit` — with `offset 0.1 %` and
`tif gtc|ioc|fok|day` — parse and are then refused (`clause_not_representable`), because
the stored order specification has no order type. An `offset` carries `%`: a bare number
is a tenth of a percent to one reader and a tenth of the price to the next.

Every order passes the platform risk gate; there is no way to write a strategy that
bypasses it. Entries place an order when their condition becomes true while flat; exits
close the position (`guide#exits`).

## Exits {#exits}

At least one `exit` clause (`exit none` says the strategy delegates to the risk layer):

- `exit stop 2 % from entry` — stop loss;
- `exit target 5 % from entry` — take profit;
- `exit trailing 3 % from high_water` — trailing stop;
- `exit time 48 bars` — close after a holding period;
- `exit when <condition>` — close when a condition holds (an exit signal).

The first exit to trigger wins. A strategy without a time exit or a stop can hold for
ever: `preview_signals` reports the declared exit policy, so you can see whether you
wrote one. Distances need a unit and an anchor: see `guide#exits.units`.

## Units on a distance {#exits.units}

A distance is `<number> <unit> from <anchor>`. The unit is never optional — a bare
`exit stop 1.5` is `distance_without_unit` — and it is one of:

- `%` — a percentage of the anchor price: `2 % from entry`;
- `atr(N)` — N-bar average true range multiples: `1.5 atr(14) from entry`;
- `sigma(N)` — N-bar standard deviations of the close: `2 sigma(20) from entry`.

`atr` and `sigma` take their lookback in parentheses and it is not optional (`atr(14)`,
not `atr`): ATR over 14 bars and over 50 are different measurements, and the compiler
will not choose for you. Put a space between the number and the unit (`1.5 atr(14)`, not
`1.5atr`). The anchor is `entry` or `high_water` (for a trailing stop).

A scaled unit needs definition version 1.4; leave `version` unwritten and the compiler
selects it. Do not hand-convert an ATR distance into a fraction: it is wrong on the next
bar and looks identical in storage.

## Risk {#risk}

`risk max_position 0.5`, `risk max_orders_per_min 6`, `risk max_orders_per_sec 2` —
limits for this strategy. They can only tighten the platform's risk gate, never widen it
(`loosening` is refused by the validator, not warned). `risk max_daily_loss` is refused
(`risk_item_not_representable`): the daily loss limit lives in the platform risk
configuration.

A strategy with no `risk` clause runs under the platform limits. Write one only to be
stricter than the platform.

## Warm-up {#warmup}

A feature has no value until it has seen enough bars: `ema_N` needs 5N, `rsi_N` about
5N, a rolling statistic N. The longest warm-up among a strategy's features is its
warm-up; no signal can fire before it. `explain_strategy` reports it and `preview_signals`
reports the one a backtest will use.

If the window is shorter than the warm-up (`warmup_exceeds_window`), start the window
earlier or shorten the longest lookback: `ema_200` to `ema_100` halves it. A preview that
shows no entries often shows only this: check `warmup_ok` before editing the rule.

## Previewing before spending {#preview_signals}

`preview_signals` runs a draft over stored bars with no trial and no ledger row: entries,
the bars they fired on, the feature values that caused them, the warm-up, and — when it
never fires — which of the possible reasons it was. Reads stop at the research cutoff; a
window reaching into the sealed holdout is refused by name, not shortened.

Pass materialised parameters: the preview takes feature names as written, so a name that
still contains `{{fast}}` is not a feature. `blocking_profile` does this for you.

The loop is write → `compile_strategy` → `preview_signals` → adjust → `create_strategy`.
A strategy that never fires, or fires every bar, is caught before a counted trial.

## Which clause blocked the entry {#preview.blocking}

`blocking_profile` evaluates each clause of each entry over a window and reports, per
clause, the share of bars it was true on and the share of the failing bars where it was
the only false clause (`sole_blocker_share`).

> `rsi(close, 14) < 30` was true on 0.4% of bars and was the sole blocker on 91% of the
> rest.

That sentence says which threshold is the problem. The verdict is `blocked` (one clause
carries at least half the failures), `no_single_blocker` (several are false together, so
moving one threshold will not help), `fires`, or `nothing_evaluated` (the window was all
warm-up). `suggest_moves` reads this profile and turns it into candidate edits, each
citing the numbers it was read from.

## The stored form (JSON) {#json #definition}

SLv2 compiles to the canonical JSON definition; `compile_strategy` shows it and
`explain_strategy` reads a stored one back as SLv2. The shape, for an MCP client that
writes it directly:

```
{"strategy_id": "…", "definition_version": "1.4", "asset_class": "crypto_spot_cex",
 "inputs": [{"lane": "market.bars.1h", "instrument": "$bound_at_init"},
            {"lane": "features.technical", "instrument": "$bound_at_init",
             "features": ["ema_7", "ema_21"]}],
 "nodes": [{"id": "n1", "type": "condition", "expr": "feature('ema_7') > feature('ema_21')"},
           {"id": "n2", "type": "signal", "when": "n1", "emit": "long"}],
 "actions": [{"on_signal": "long", "type": "place_order",
              "order": {"side": "buy", "size_mode": "fixed", "size": "0.01"}}],
 "exits": {…}}
```

The expression grammar there is the frozen one: comparisons, arithmetic, `feature('name')`
and `bar('field')`, and a parenthesised comparison counts as 1 or 0, which is how `and`,
`or` and `not` are stored. Write SLv2 and let the compiler produce this.

## Nodes {#nodes}

A stored definition is a graph of nodes. A `condition` node holds one expression; a
`signal` node says `when` (the **id** of a condition node, never an expression) and what
it emits (`long`, `short`, or a name an exit refers to). An `action` says which signal
places which order. IDs are unique.

`when_names_expression` means a signal's `when` held an expression: add a `condition` node
with that expression and put its id in `when`. In SLv2 none of this is visible — each
`entry` is a condition and a signal, written once.

## Moves, diffs and lineage {#moves}

Change one thing at a time, and let the platform check that you did.

- `apply_move(definition, edit)` makes one typed edit — `threshold`, `add_filter`,
  `remove_filter`, `replace_filter`, `timeframe`, `sizing`, `add_exit`, `replace_exit`,
  `remove_exit` — and returns the new version with the move it turned out to be. An edit
  that changes nothing, or changes two things, is refused.
- `diff_strategies(before, after)` says what changed in words and whether it is a single
  change. A version that is two moves cannot be attributed to either.
- `suggest_moves` proposes the next move from a measurement.
- Save with the **same strategy id**: `create_strategy` appends a revision whose parent is
  the previous head, and that chain is the lineage. A new id has no parent.

Clause addresses are the ones `explain_strategy` prints: `entry.long[0].clause[1]`.

## The clause grammar for small models {#form}

A model that cannot write free text fills one clause at a time through a JSON Schema: a
header, then params, inputs, entries, size, exits, risk. The schema has one branch per
feature family, each with its own arity and each parameter's range, so an unknown feature
or a wrong number of arguments cannot be written at all. The fills are assembled into SLv2
text and sent through the same parser and compiler as free text — there is no second
compiler — so a strategy that validates in the form validates in the language.

The tool is `compile_strategy_form`: its arguments are the form and its input schema is the
grammar, so a tier that decodes a tool's arguments under the tool's schema is decoding under
it. It returns the assembled SLv2 and the compiled definition, and persists nothing.

A whole strategy is usually four fills: the header, an entry, the size and an exit. The
form is narrower than the language on purpose; where free SLv2 says something the form
cannot hold, the refusal names the limitation and whether writing it as text would help.

<!-- generated by strategy_lang::guide; do not edit below this line -->

## Feature families {#features}

62 families, read from the engine's registry. A family with parameters is written with them, as in the example column — `rsi` is not a feature, `rsi_14` is. In SLv2 the same feature is `rsi(close, 14)`. `list_features(query="…")` searches them; the reference sections after this one say when to use each.

| family | example | warm-up bars |
|---|---|---|
| `open` | `open` | 1 |
| `high` | `high` | 1 |
| `low` | `low` | 1 |
| `close` | `close` | 1 |
| `volume` | `volume` | 1 |
| `returns` | `returns_20` | 21 |
| `log_returns` | `log_returns_20` | 21 |
| `lag` | `lag_20` | 21 |
| `diff` | `diff_20` | 21 |
| `rolling_mean` | `rolling_mean_20` | 20 |
| `rolling_std` | `rolling_std_20` | 20 |
| `rolling_var` | `rolling_var_20` | 20 |
| `rolling_min` | `rolling_min_20` | 20 |
| `rolling_max` | `rolling_max_20` | 20 |
| `rolling_sum` | `rolling_sum_20` | 20 |
| `rolling_median` | `rolling_median_20` | 20 |
| `rolling_quantile` | `rolling_quantile_20_50` | 20 |
| `rolling_skew` | `rolling_skew_20` | 20 |
| `rolling_kurt` | `rolling_kurt_20` | 20 |
| `zscore` | `zscore_20` | 20 |
| `pct_rank` | `pct_rank_20` | 20 |
| `ewma` | `ewma_10` | 80 |
| `ema` | `ema_20` | 100 |
| `rsi` | `rsi_20` | 101 |
| `atr` | `atr_20` | 101 |
| `roc` | `roc_20` | 21 |
| `macd` | `macd_12_26` | 130 |
| `macd_signal` | `macd_signal_12_26_9` | 175 |
| `macd_hist` | `macd_hist_12_26_9` | 175 |
| `bb_upper` | `bb_upper_20_2` | 20 |
| `bb_lower` | `bb_lower_20_2` | 20 |
| `bb_width` | `bb_width_20_2` | 20 |
| `bb_pctb` | `bb_pctb_20_2` | 20 |
| `donchian_upper` | `donchian_upper_20` | 20 |
| `donchian_lower` | `donchian_lower_20` | 20 |
| `donchian_mid` | `donchian_mid_20` | 20 |
| `donchian_width` | `donchian_width_20` | 20 |
| `keltner_upper` | `keltner_upper_20_2` | 101 |
| `keltner_lower` | `keltner_lower_20_2` | 101 |
| `obv` | `obv` | 256 |
| `rel_volume` | `rel_volume_20` | 20 |
| `hour_sin` | `hour_sin` | 1 |
| `hour_cos` | `hour_cos` | 1 |
| `dow_sin` | `dow_sin` | 1 |
| `dow_cos` | `dow_cos` | 1 |
| `utc_session_asia` | `utc_session_asia` | 1 |
| `utc_session_london` | `utc_session_london` | 1 |
| `utc_session_us` | `utc_session_us` | 1 |
| `parkinson_vol` | `parkinson_vol_20` | 20 |
| `garman_klass_vol` | `garman_klass_vol_20` | 20 |
| `rogers_satchell_vol` | `rogers_satchell_vol_20` | 20 |
| `rv` | `rv_20` | 21 |
| `bv` | `bv_20` | 21 |
| `jump_share` | `jump_share_20` | 21 |
| `autocorr` | `autocorr_20_1` | 22 |
| `variance_ratio` | `variance_ratio_20_4` | 29 |
| `efficiency_ratio` | `efficiency_ratio_20` | 21 |
| `yang_zhang_vol` | `yang_zhang_vol_20` | 20 |
| `hurst` | `hurst_257` | 257 |
| `fracdiff` | `fracdiff_20_40` | 20 |
| `lookback_ensemble_zscore` | `lookback_ensemble_zscore_10_20_40` | 40 |
| `lookback_ensemble_returns` | `lookback_ensemble_returns_10_20_40` | 41 |

A warm-up is the bars before the feature has a value. `ema_N` needs 5N, so `ema_200` needs 1000: check it against the window (`warmup_exceeds_window`).

## Feature reference 1 of 2 {#features.reference.1}

- `open` — the bar's open; use as the reference for an intrabar move
- `high` — the bar's high
- `low` — the bar's low
- `close` — the bar's close; the default source of every windowed family
- `volume` — the bar's volume; compare with rel_volume_N rather than an absolute level
- `returns_20` — simple return over n bars; the usual momentum primitive
- `log_returns_20` — log return over n bars; additive across bars, so use it when summing returns
- `lag_20` — the close n bars ago; compare a level with its own past
- `diff_20` — close minus close n bars ago, in price units, not a ratio
- `rolling_mean_20` — mean close over n bars; the trend baseline a price is compared against
- `rolling_std_20` — sample standard deviation of close over n bars; the sigma unit of an exit distance
- `rolling_var_20` — sample variance of close over n bars
- `rolling_min_20` — lowest close over n bars
- `rolling_max_20` — highest close over n bars
- `rolling_sum_20` — sum of close over n bars
- `rolling_median_20` — median close over n bars; an outlier-robust baseline
- `rolling_quantile_20_50` — the q-th percentile close over n bars, q in whole percent
- `rolling_skew_20` — sample skewness of close over n bars
- `rolling_kurt_20` — excess kurtosis of close over n bars; high values mean fat tails
- `zscore_20` — how many sigma the close is from its n-bar mean; the mean-reversion primitive
- `pct_rank_20` — fraction of the last n closes below this one, 0..1; use to gate on a regime percentile
- `ewma_10` — exponentially weighted mean with the given halflife in bars
- `ema_20` — exponential moving average of close over n bars
- `rsi_20` — Wilder RSI of close, 0..100; below 30 is the conventional oversold gate
- `atr_20` — Wilder average true range in price units; the unit an exit distance should use
- `roc_20` — rate of change over n bars, in percent
- `macd_12_26` — ema_fast minus ema_slow; positive means the fast trend leads
- `macd_signal_12_26_9` — the signal-period ema of macd_fast_slow
- `macd_hist_12_26_9` — macd minus its signal line; the crossover primitive
- `bb_upper_20_2` — rolling mean plus k sample standard deviations of close
- `bb_lower_20_2` — rolling mean minus k sample standard deviations of close
- `bb_width_20_2` — band width as a fraction of the mean; a squeeze is a low value
- `bb_pctb_20_2` — where close sits in the band, 0 at the lower and 1 at the upper
- `donchian_upper_20` — highest high over n bars; the breakout level
- `donchian_lower_20` — lowest low over n bars
- `donchian_mid_20` — midpoint of the n-bar high-low channel
- `donchian_width_20` — channel height as a fraction of the midpoint
- `keltner_upper_20_2` — ema_n plus k atr_n; a volatility band that does not assume normality
- `keltner_lower_20_2` — ema_n minus k atr_n
- `obv` — on-balance volume over a fixed 256-bar window
- `rel_volume_20` — this bar's volume over the n-bar mean; 1.0 is an ordinary bar
- `hour_sin` — sine of the UTC hour; pair with hour_cos so midnight is next to 23:00
- `hour_cos` — cosine of the UTC hour
- `dow_sin` — sine of the UTC day of week
- `dow_cos` — cosine of the UTC day of week
- `utc_session_asia` — 1 inside 00:00-08:00 UTC, else 0; a UTC window, not an exchange calendar

## Feature reference 2 of 2 {#features.reference.2}

- `utc_session_london` — 1 inside 07:00-16:00 UTC, else 0; a UTC window, not an exchange calendar
- `utc_session_us` — 1 inside 13:00-22:00 UTC, else 0; a UTC window, not an exchange calendar
- `parkinson_vol_20` — Parkinson range volatility over n bars; not annualised
- `garman_klass_vol_20` — Garman-Klass volatility over n bars; uses the open as well as the range
- `rogers_satchell_vol_20` — Rogers-Satchell volatility over n bars; drift-independent
- `rv_20` — realised volatility: root sum of squared bar log returns over n bars
- `bv_20` — bipower variation over n bars; jump-robust, so rv minus bv isolates jumps
- `jump_share_20` — share of realised variance attributable to jumps, 0..1
- `autocorr_20_1` — autocorrelation of bar returns at the given lag over n bars
- `variance_ratio_20_4` — Lo-MacKinlay variance ratio at horizon q; above 1 is trending, below 1 mean-reverting
- `efficiency_ratio_20` — net move over total path length, 0..1; the persistence filter a trend rule wants
- `yang_zhang_vol_20` — Yang-Zhang volatility over n bars; handles gaps between bars as well as the range
- `hurst_257` — rescaled-range Hurst exponent of bar returns; above 0.55 trending, below 0.45 mean-reverting
- `fracdiff_20_40` — fractional difference of log close, order d/100 over n bars; stationary but keeps memory
- `lookback_ensemble_zscore_10_20_40` — equal-weight mean of zscore over three lookbacks; a mean-reversion rule that does not hinge on one window
- `lookback_ensemble_returns_10_20_40` — equal-weight mean of simple returns over three lookbacks; a momentum rule that does not hinge on one window

## Skeletons {#skeletons}

A correct first version per hypothesis family, so the first draft is never a syntax lesson. `strategy_skeleton(hypothesis_family)` returns one; it compiles as written, and every parameter is declared so a sweep can vary it. Preview it before changing it.

- `trend` — `{#skeletons.trend}`
- `mean_reversion` — `{#skeletons.mean_reversion}`
- `breakout` — `{#skeletons.breakout}`
- `carry` — not writable yet: a `carry` strategy needs the funding rate or the futures basis it is paid out of, and no feature family computes it: funding_rate, basis, carry are not in the engine registry
- `vol_target` — `{#skeletons.vol_target}`
- `model_signal` — not writable yet: a `model_signal` strategy needs a model's prediction read through the MLOps registry (DATA-006 F4, unbuilt), and no feature family computes it: prediction, regime, text are not in the engine registry

## Skeleton: trend {#skeletons.trend}

```
# Trend: hold while the fast baseline leads the slow one, and do not buy a move
# that has already run.
strategy trend_skeleton on $bound_at_init timeframe 1h
# The two baselines are what the hypothesis is about, so they are the two things
# a sweep varies. `constraint` keeps the sweep from proposing a fast line slower
# than the slow one, which would invert the rule rather than weaken it.
param fast: int[5..50 step 1] = 7
param slow: int[20..200 step 5] = 21
constraint param('fast') < param('slow')
input fast_ma = ema(close, fast)
input slow_ma = ema(close, slow)
# 60 rather than 70: this is a trend rule, so the filter is there to refuse an
# entry into an extended move, not to call a top.
entry long when fast_ma > slow_ma and rsi(close, 14) < 60
size fixed 0.01
exit stop 2 % from entry
exit target 5 % from entry
# A trend that has not worked in two days was not the trend this rule is about.
exit time 48 bars
risk max_position 0.5
```

`ema_N` warms up over 5N bars, so at `slow = 21` the first tradeable bar is 105 bars into the window — check `warmup_ok` before reading a preview that shows no entries.

`strategy_skeleton` returns the full notes.

## Skeleton: mean_reversion {#skeletons.mean_reversion}

```
# Mean reversion: buy a stretch away from the mean, but only while the series is
# the kind that comes back.
strategy mean_reversion_skeleton on $bound_at_init timeframe 1h
param window: int[10..100 step 5] = 20
# Negative, because it is a distance below the mean. Sweep it: this threshold
# decides whether the rule ever fires, and it is the first thing a blocking
# profile will point at.
param stretch: float[-4..-0.5] = -2
input z = zscore(close, window)
# Lo-MacKinlay variance ratio below 1 is the mean-reverting regime. Above 1 the
# series is trending and this rule is standing in front of it.
input persistence = variance_ratio(close, 64, 4)
entry long when z < stretch and persistence < 1
size equity_fraction 0.1
# The target is nearer than the stop on purpose: a mean-reversion rule wins
# often and small, and the stop is what stops it losing rarely and large.
exit target 1.5 % from entry
exit stop 3 % from entry
exit time 24 bars
risk max_orders_per_min 6
```

`zscore_N` is scale-free, so the `stretch` threshold means the same thing on any instrument — which is why the rule compares against a z-score rather than against a price distance.

`strategy_skeleton` returns the full notes.

## Skeleton: breakout {#skeletons.breakout}

```
# Breakout: take the move when price leaves the top or bottom of its recent
# range, in whichever direction it leaves.
strategy breakout_skeleton on $bound_at_init timeframe 1h
param channel: int[10..200 step 10] = 20
# Where this close sits among the last `channel` closes, 0 at the bottom and 1 at
# the top: 1 is a close above every other close in the window, which is a new high.
input rank = pct_rank(close, channel)
# Volume is the confirmation: a break on an ordinary bar is usually noise, and
# `rel_volume` is 1.0 on an ordinary bar by construction.
input volume_surge = rel_volume(close, 20)
entry long when rank > 0.95 and volume_surge > 1.5
entry short when rank < 0.05 and volume_surge > 1.5
size fixed 0.01
# Trailing rather than a fixed target: the whole hypothesis is that the move
# continues, so the exit should not cap it.
exit trailing 3 % from high_water
exit time 96 bars
```

The textbook breakout is a close above the highest high of the *previous* N bars, and the language cannot say it. `donchian_upper_N` is the highest high of the last N bars **including this one**, so `close > donchian_upper(close, N)` can never be true — a bar's close is never above its own high, and the channel's high is at least that. An earlier version of this skeleton wrote exactly that and could not fire on any market. Lagging the channel by one bar would fix it and is a feature of a feature, which is not in the language.

`strategy_skeleton` returns the full notes.

## Skeleton: vol_target {#skeletons.vol_target}

```
# Volatility target: keep what is at risk per trade steady, and stand aside when
# the market is not the one the rule was measured on.
strategy vol_target_skeleton on $bound_at_init timeframe 1h
param vol_window: int[10..200 step 10] = 20
# Band width is a fraction of the mean, so this ceiling is scale-free and means
# the same thing on any instrument.
param vol_ceiling: float[0.01..0.2 log] = 0.08
input band_width = bb_width(close, vol_window, 2)
# Efficiency ratio is net move over path length: 0.4 asks for a market that is
# going somewhere rather than one that is merely moving.
input trend_quality = efficiency_ratio(close, 50)
entry long when band_width < vol_ceiling and trend_quality > 0.4
size risk_per_trade 0.01
exit stop 2 % from entry
exit time 72 bars
risk max_position 0.25
```

**This skeleton targets volatility with the mechanisms the runtime has, and they are not the textbook ones. Read this before copying it.**

`strategy_skeleton` returns the full notes.

