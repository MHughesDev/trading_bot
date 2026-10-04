# Stored strategies on the one engine — v1 → v3 migration

Generated 2026-10-04 09:31 UTC by `cargo run -p api --bin migrate_strategies_v3` (Set U U-8.5; FEAT-007 §12). **Dry run: nothing was written to the database.**

Translated strategies are not expected to backtest as they did: FEAT-007 §12. Every difference below names the listed behaviour change it is attributed to. Attribution is by mechanism — the change is present in the run and moves that metric — not by re-running with each change removed. Models answer `unavailable` in these re-runs (no inference service), so a model-driven rule holds.

| strategy | timeframe | outcome | orders old → new | P&L old → new |
|---|---|---|---|---|
| `ai_model_smoke` | 1h (default: no backtest on record) | does not compile — [unknown_model] at m1__model: no model answers to 'btc_forecaster@production' — fix: use `name@version` or `name@alias` for a model in the registry; list them with the models catalogue | — | — |
| `btc_model_signal_v1` | 1h (default: no backtest on record) | refused — `inputs`: it reads `BTC-USD`, another instrument; declare it as a v3 `input` by hand | — | — |
| `ema_crossover` | 1m (its latest backtest) | translated, re-run | 333 → 567 | 0.00 → -3770.11 |
| `ema_crossover_scanner` | 1h (default: no backtest on record) | translated, no backtest to re-run | — | — |
| `ema_cross_p_smoke_1788927738` | 1h (default: no backtest on record) | translated, no backtest to re-run | — | — |
| `ema_cross_p_smoke_1788928410` | 1h (default: no backtest on record) | translated, no backtest to re-run | — | — |
| `ema_cross_v1` | 1m (its latest backtest) | translated, re-run failed | — | — |
| `ema_rsi_trend` | 1h (default: no backtest on record) | translated, no backtest to re-run | — | — |
| `ema_trend_sell` | 1h (default: no backtest on record) | translated, no backtest to re-run | — | — |
| `model_signal_v1` | 1h (default: no backtest on record) | does not compile — [unknown_feature] at n1~a: there is no indicator 'model_signal' — fix: indicator names are family_params, e.g. rsi_14, ema_20, atr_14 (e.g. {"op":"feature","args":{"name":{"str":"rsi_14"}}}) | — | — |
| `rsi_mean_reversion` | 1h (default: no backtest on record) | translated, no backtest to re-run | — | — |
| `rsi_overbought_scanner` | 1h (default: no backtest on record) | translated, no backtest to re-run | — | — |
| `rsi_oversold_bounce` | 1h (default: no backtest on record) | translated, no backtest to re-run | — | — |
| `triple_ema` | 1h (default: no backtest on record) | translated, no backtest to re-run | — | — |

## `ai_model_smoke`

v1 revision `10e54102be7d2f2cf2c519531f6018e09170978db44da24985e0eb60694765f4`.

**Translated but does not compile today**: [unknown_model] at m1__model: no model answers to 'btc_forecaster@production' — fix: use `name@version` or `name@alias` for a model in the registry; list them with the models catalogue

```slv3
strategy ai_model_smoke kind trading asset_class crypto_spot_cex timeframe 1h
description "translated from v1.1"

let m1__model = model("btc_forecaster@production")
let m1 = is(m1__model.direction, "bullish") and m1__model.confidence >= 0.6

when m1 and not prev(m1, 1.0) -> order(side: buy, size: units(x: 0.01), style: market, tif: gtc)
otherwise hold
```

## `btc_model_signal_v1`

v1 revision `f9c8616839ae527f3761e11896a5aa60336b7b0241476d1749c98e35f579fe61`.

**Refused**, not approximated: `inputs` at `inputs[lane=market.bars.1h]` — it reads `BTC-USD`, another instrument; declare it as a v3 `input` by hand. It keeps running on v1 until the translator covers it.

## `ema_crossover`

v1 revision `99011bd86e372817ab7be00ee1536b520e37dcfe8e998fbb42c48dd318c044fb`.

v3 revision `8615811218343b2703eea684eabcd7bcf9b77f83b77f8ef7444c340fa0defcf6`.

```slv3
strategy ema_crossover kind trading asset_class crypto_spot_cex timeframe 1m
description "translated from v1.0"

let c_bull = feature("ema_7") > feature("ema_21")
let c_bear = feature("ema_7") < feature("ema_21")

when c_bull and not prev(c_bull, 1.0) -> order(side: buy, size: units(x: 0.1), style: market, tif: gtc)
when c_bear and not prev(c_bear, 1.0) -> order(side: sell, size: units(x: 0.1), style: market, tif: gtc)
otherwise hold
```

What changes:

- `fills_next_bar_open` — orders fill at the next bar's open, not at the close of the bar that decided
- `risk_gate_applied` — every order passes the production risk gate, in backtests too
- `missing_holds` — a value that cannot be computed (an indicator warming up, a model that does not answer) holds the rule, with the reason journaled, rather than comparing false

Re-run of its latest backtest: `BTC-USD`, 2026-06-15 → 2026-06-22, starting balance 10000.00.

| | old (`market_simulator`) | new (engine) |
|---|---:|---:|
| bars | 6022 | 10079 |
| orders | 333 | 567 |
| positions / round trips | 167 | 101 |
| P&L | 0.00 | -3770.11 |
| decisions | — | 10079 |
| gate rejections | — | 0 |
| holds | — | 0 |

- **orders** 333.00 → 567.00: `bars_differ`
- **pnl** 0.00 → -3770.11: `v1_venue_reported_no_fills`, `bars_differ`, `fills_next_bar_open`

## `ema_crossover_scanner`

v1 revision `aa8c1354274a4b24bb3dfa6784f6df65a47c8f6148ec73d19b9ad3d759b72ccb`.

v3 revision `3a62fffd9379180cb9201f346001a32fd9f79770ba61fac6c7c7732d6478cb8a`.

```slv3
strategy ema_crossover_scanner kind trading asset_class crypto_spot_cex timeframe 1h
description "translated from v1.0"

let c1 = feature("ema_7") > feature("ema_21")

otherwise hold
```

What changes:

- `fills_next_bar_open` — orders fill at the next bar's open, not at the close of the bar that decided
- `risk_gate_applied` — every order passes the production risk gate, in backtests too
- `missing_holds` — a value that cannot be computed (an indicator warming up, a model that does not answer) holds the rule, with the reason journaled, rather than comparing false
- `no_orders` — the v1 definition places no orders; the translation has no entry rule

No completed backtest on record to re-run.

## `ema_cross_p_smoke_1788927738`

v1 revision `86aa09df5bf5adc0610aa0feb6ee3b3f6f743973f5ab92e42ccd05993d6a68ea`.

v3 revision `8dec137a463625b2345e941c84642d8b93704c123e8dbcbec6ee6854561e0be1`.

```slv3
strategy ema_cross_p_smoke_1788927738 kind trading asset_class crypto_spot_cex timeframe 1h
description "translated from v1.2"
param fast: int(default: 7, max: 30, min: 3, step: 1)
param slow: int(default: 21, max: 90, min: 10, step: 1)
constraint "param('fast') < param('slow')"

let n1 = ema(close, $fast) > ema(close, $slow)
let n3 = ema(close, $fast) < ema(close, $slow)

when n1 and not prev(n1, 1.0) -> order(side: buy, size: units(x: 0.01), style: market, tif: gtc)
when n3 and not prev(n3, 1.0) -> order(side: sell, size: units(x: 0.01), style: market, tif: gtc)
otherwise hold
```

What changes:

- `fills_next_bar_open` — orders fill at the next bar's open, not at the close of the bar that decided
- `risk_gate_applied` — every order passes the production risk gate, in backtests too
- `missing_holds` — a value that cannot be computed (an indicator warming up, a model that does not answer) holds the rule, with the reason journaled, rather than comparing false

No completed backtest on record to re-run.

## `ema_cross_p_smoke_1788928410`

v1 revision `7f5e43d91b59f6e92002a456d56ad36f4e4dc4bda178ccedb9cf67813b62b06c`.

v3 revision `6c803de402d02ba8c2be0df622be9d695e73f286f8a476735284831cf9e83e0c`.

```slv3
strategy ema_cross_p_smoke_1788928410 kind trading asset_class crypto_spot_cex timeframe 1h
description "translated from v1.2"
param fast: int(default: 7, max: 30, min: 3, step: 1)
param slow: int(default: 21, max: 90, min: 10, step: 1)
constraint "param('fast') < param('slow')"

let n1 = ema(close, $fast) > ema(close, $slow)
let n3 = ema(close, $fast) < ema(close, $slow)

when n1 and not prev(n1, 1.0) -> order(side: buy, size: units(x: 0.01), style: market, tif: gtc)
when n3 and not prev(n3, 1.0) -> order(side: sell, size: units(x: 0.01), style: market, tif: gtc)
otherwise hold
```

What changes:

- `fills_next_bar_open` — orders fill at the next bar's open, not at the close of the bar that decided
- `risk_gate_applied` — every order passes the production risk gate, in backtests too
- `missing_holds` — a value that cannot be computed (an indicator warming up, a model that does not answer) holds the rule, with the reason journaled, rather than comparing false

No completed backtest on record to re-run.

## `ema_cross_v1`

v1 revision `b329d970884dddce4b092e6d4d44bb1944fb0c429cd9d92419688500996e7766`.

v3 revision `fa957ede23d05e9bb99c5cb53dcb67a8cee4e07c0406bcec6d78c38770b26cd9`.

```slv3
strategy ema_cross_v1 kind trading asset_class crypto_spot_cex timeframe 1m
description "translated from v1.0"

let c1 = feature("ema_7") > feature("ema_21")

when c1 and not prev(c1, 1.0) -> order(side: buy, size: units(x: 0.01), style: market, tif: gtc)
otherwise hold
```

What changes:

- `fills_next_bar_open` — orders fill at the next bar's open, not at the close of the bar that decided
- `risk_gate_applied` — every order passes the production risk gate, in backtests too
- `missing_holds` — a value that cannot be computed (an indicator warming up, a model that does not answer) holds the rule, with the reason journaled, rather than comparing false

Re-run of its latest backtest: `BTC-USDT`, 2026-06-15 → 2026-09-13, starting balance 100000.00.

The v3 re-run did not run: no instrument metadata for 'BTC-USDT': the risk gate refuses an                          instrument it cannot size, here as live (ADR-0046).

## `ema_rsi_trend`

v1 revision `bc683cd69be02415aea3683b70db96019063e317ad15e3fb398c0b135cffd24a`.

v3 revision `52ca01f0153ffc419f22a7127340cae16a5bdf984e05e46c0bfb4102a4e86ee4`.

```slv3
strategy ema_rsi_trend kind trading asset_class crypto_spot_cex timeframe 1h
description "translated from v1.0"

let c_bull = if(feature("ema_7") > feature("ema_21"), 1.0, 0.0) * if(feature("rsi_14") < 70.0, 1.0, 0.0) > 0.0
let c_bear = feature("ema_7") < feature("ema_21")

when c_bull and not prev(c_bull, 1.0) -> order(side: buy, size: units(x: 0.1), style: market, tif: gtc)
when c_bear and not prev(c_bear, 1.0) -> order(side: sell, size: units(x: 0.1), style: market, tif: gtc)
otherwise hold
```

What changes:

- `fills_next_bar_open` — orders fill at the next bar's open, not at the close of the bar that decided
- `risk_gate_applied` — every order passes the production risk gate, in backtests too
- `missing_holds` — a value that cannot be computed (an indicator warming up, a model that does not answer) holds the rule, with the reason journaled, rather than comparing false

No completed backtest on record to re-run.

## `ema_trend_sell`

v1 revision `51379c54922d657609828d546b0d2738be423d988aa9e1e5f3bfc45cf321bb33`.

v3 revision `a516ea433211fa954b8a1c2c86cdaa5befbc17a0c0da17785b5992f7a7743a72`.

```slv3
strategy ema_trend_sell kind trading asset_class crypto_spot_cex timeframe 1h
description "translated from v1.0"

let c1 = feature("ema_7") < feature("ema_21")

when c1 and not prev(c1, 1.0) -> order(side: sell, size: equity_fraction(x: 0.02), style: market, tif: gtc)
otherwise hold
```

What changes:

- `fills_next_bar_open` — orders fill at the next bar's open, not at the close of the bar that decided
- `risk_gate_applied` — every order passes the production risk gate, in backtests too
- `missing_holds` — a value that cannot be computed (an indicator warming up, a model that does not answer) holds the rule, with the reason journaled, rather than comparing false

No completed backtest on record to re-run.

## `model_signal_v1`

v1 revision `8670c871e567010b362ad9365ed2b8db92dbbbed9e208a8fb1a514c3bd575c5e`.

**Translated but does not compile today**: [unknown_feature] at n1~a: there is no indicator 'model_signal' — fix: indicator names are family_params, e.g. rsi_14, ema_20, atr_14 (e.g. {"op":"feature","args":{"name":{"str":"rsi_14"}}})

```slv3
strategy model_signal_v1 kind trading asset_class crypto_spot_cex timeframe 1h
description "translated from v1.2"
param exit: enum(choices: [trail, fixed], default: trail)

let n1 = feature("model_signal") > 0.5

when n1 and not prev(n1, 1.0) -> order(side: buy, size: units(x: 0.01), style: market, tif: gtc)
otherwise hold
```

## `rsi_mean_reversion`

v1 revision `9911208568cfa47cdd416acc57097410220fc5650c1c7e0bd99050376a5d9f5d`.

v3 revision `6709358d8971ad85d38ad455803052018eb98725035cdb0f2e8a5985af128c63`.

```slv3
strategy rsi_mean_reversion kind trading asset_class crypto_spot_cex timeframe 1h
description "translated from v1.0"

let c_oversold = feature("rsi_14") < 30.0
let c_overbought = feature("rsi_14") > 70.0

when c_oversold and not prev(c_oversold, 1.0) -> order(side: buy, size: units(x: 0.1), style: market, tif: gtc)
when c_overbought and not prev(c_overbought, 1.0) -> order(side: sell, size: units(x: 0.1), style: market, tif: gtc)
otherwise hold
```

What changes:

- `fills_next_bar_open` — orders fill at the next bar's open, not at the close of the bar that decided
- `risk_gate_applied` — every order passes the production risk gate, in backtests too
- `missing_holds` — a value that cannot be computed (an indicator warming up, a model that does not answer) holds the rule, with the reason journaled, rather than comparing false

No completed backtest on record to re-run.

## `rsi_overbought_scanner`

v1 revision `8e34188eb048865566954ddc4bde96888b4c9cdf25416177deabeaa7b625b852`.

v3 revision `7d9b21c21e75c2eeedf8fa87ea1fea26957772803e0d4ad03adfa9dbed163f43`.

```slv3
strategy rsi_overbought_scanner kind trading asset_class crypto_spot_cex timeframe 1h
description "translated from v1.0"

let c1 = feature("rsi_14") > 70.0

otherwise hold
```

What changes:

- `fills_next_bar_open` — orders fill at the next bar's open, not at the close of the bar that decided
- `risk_gate_applied` — every order passes the production risk gate, in backtests too
- `missing_holds` — a value that cannot be computed (an indicator warming up, a model that does not answer) holds the rule, with the reason journaled, rather than comparing false
- `no_orders` — the v1 definition places no orders; the translation has no entry rule

No completed backtest on record to re-run.

## `rsi_oversold_bounce`

v1 revision `dc82e8c20ff785e82c5d214b0d7567f7a4f09cec9852d018db3b18586566d9af`.

v3 revision `67cf7ae3c90dc72e986b4ef30e9c7e54baa92435dcad77af531d94827f2e6965`.

```slv3
strategy rsi_oversold_bounce kind trading asset_class crypto_spot_cex timeframe 1h
description "translated from v1.0"

let c1 = feature("rsi_14") < 30.0

when c1 and not prev(c1, 1.0) -> order(side: buy, size: equity_fraction(x: 0.01), style: market, tif: gtc)
otherwise hold
```

What changes:

- `fills_next_bar_open` — orders fill at the next bar's open, not at the close of the bar that decided
- `risk_gate_applied` — every order passes the production risk gate, in backtests too
- `missing_holds` — a value that cannot be computed (an indicator warming up, a model that does not answer) holds the rule, with the reason journaled, rather than comparing false

No completed backtest on record to re-run.

## `triple_ema`

v1 revision `780213d44569a1f296c92a217369611e5759b10aab9711f92f580417a9dda5e5`.

v3 revision `d662b90e43d6aafaf32d088e8e759e37449cd3c0dbfef51c33a6d1151f966d1d`.

```slv3
strategy triple_ema kind trading asset_class crypto_spot_cex timeframe 1h
description "translated from v1.0"

let c_bull = if(feature("ema_7") > feature("ema_21"), 1.0, 0.0) * if(feature("ema_21") > feature("ema_50"), 1.0, 0.0) > 0.0
let c_bear = if(feature("ema_7") < feature("ema_21"), 1.0, 0.0) * if(feature("ema_21") < feature("ema_50"), 1.0, 0.0) > 0.0

when c_bull and not prev(c_bull, 1.0) -> order(side: buy, size: units(x: 0.1), style: market, tif: gtc)
when c_bear and not prev(c_bear, 1.0) -> order(side: sell, size: units(x: 0.1), style: market, tif: gtc)
otherwise hold
```

What changes:

- `fills_next_bar_open` — orders fill at the next bar's open, not at the close of the bar that decided
- `risk_gate_applied` — every order passes the production risk gate, in backtests too
- `missing_holds` — a value that cannot be computed (an indicator warming up, a model that does not answer) holds the rule, with the reason journaled, rather than comparing false

No completed backtest on record to re-run.

`v1_venue_reported_no_fills`: the v1 backtest venue facade recorded orders and reported no fills back (FEAT-007 §1 defect 9), so a v1 backtest's P&L was zero however the strategy traded.

`bars_differ`: the old run processed a different number of bars than the bar store now holds for the same window (bars backfilled since, or gaps the old run skipped), so the two runs did not see the same market.
