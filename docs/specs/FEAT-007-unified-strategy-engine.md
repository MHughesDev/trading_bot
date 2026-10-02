# FEAT-007: Unified Strategy Engine — One Engine, Two Strategy Kinds, Typed Models, Every Environment

**Status:** Accepted (2026-10-01) — supersedes the evaluation halves of FEAT-004 §3.2–3.3 and FEAT-005 §4,
and the model-binding half of DATA-004 v1.1. Plan set: [Set U](../plans/plan-sets/set-U/MASTER.md).
**Version:** 1.0
**ADRs:** [0039](../adr/0039-one-deterministic-strategy-engine-environments-are-adapters.md) ·
[0040](../adr/0040-strategies-are-typed-dataflow-graphs-of-two-kinds.md) ·
[0041](../adr/0041-decisions-are-trade-intents-planned-into-orders.md) ·
[0042](../adr/0042-decision-cadence-bar-close-fills-and-schedules.md) ·
[0043](../adr/0043-typed-model-contracts-resolved-per-environment.md) ·
[0044](../adr/0044-native-simulated-venue-shares-the-paper-venue-core.md) ·
[0045](../adr/0045-one-language-front-end-graph-is-canonical.md) ·
[0046](../adr/0046-risk-gate-in-every-environment.md) ·
[0047](../adr/0047-decision-journal-and-replay-parity.md)
**Keeps:** ADR-0002 (decimal money), ADR-0005 (one risk gate), ADR-0019/0020/0021 (Run, nulls,
gate funnel — Set J consumes this engine's results unchanged), ADR-0029 (one feature engine),
ADR-0030 (job service), ADR-0038 (one live evaluation path — generalised here to *every* path),
INV-16 (every run is a counted trial).
**Owner direction (2026-10-01, verbatim intent):** strategies must evaluate *exactly* the same in
backtest, paper and live; backtest and paper should *work similarly but not exactly the same*;
two kinds of strategy — one that answers *what should we buy* and returns the assets that meet
the criteria, one that returns the trade decision; strategies say everything that goes into a
trade — what, when, how much, how — so anything the trading terminal can do a strategy can do;
any model trained and housed in MLOps can be called, not only LLMs, with the right data types in
and out; models, indicators and thresholds can be chained, branched and run in parallel in any
blend; humans and AI write, use and test strategies exactly the same way. Cadence and every other
engineering decision delegated.

---

## 1. Why this exists — what the audit found

An audit on 2026-10-01 traced every path a strategy takes. Ten defects, each verified in code:

| # | Defect | Where |
|---|---|---|
| 1 | **Three strategy evaluators.** Backtest runs its own bar loop over a stateless interpreter; the agent's preview runs a third copy of that loop; paper runs `StrategyInstance` (bytecode, history registers, model slots). Same strategy, different programs. | `backtest/src/sim.rs`, `backtest/src/preview.rs`, `apps/platform/src/hot_path.rs` |
| 2 | **Paper never computes indicators.** The hot path emits only bars; nothing in production emits feature values; slots start at `NaN`. Any strategy referencing an indicator never fires on paper. The one passing paper test injects indicator values by hand. | `hot_path.rs` stage 2, `strategy-runtime/src/world.rs:96` |
| 3 | **No strategy calls a model, anywhere.** Backtest has no inference. Live has model slots and `set_inference_outputs` has zero callers. | `strategy-runtime/src/runtime.rs:344` |
| 4 | **Every model is forced into a forecast shape.** The sidecar returns `{direction, magnitude, confidence}` for every model; a classifier's class probabilities, a regressor's value, a ranker's score, an embedding are squashed or lost. Missing inputs are **filled with 0.0**. Aliases move, so a backtest that used `production` cannot be reproduced. | `apps/model-inference/app/predictors/base.py` |
| 5 | **Strategies can only place market orders.** No target position, reduce, close, limit, stop, bracket, trailing or time-in-force — although `domain::order` and the paper engine already support limit and stop-limit. | `domain/src/strategy_def/actions.rs` |
| 6 | **Selection logic exists and nothing calls it.** `evaluate_universe_pipeline` and `PipelineRuntime` have zero production callers; nothing binds a selection to trading. | `strategy-runtime/src/nodes` |
| 7 | **Backtests skip the risk gate.** A backtest can show trades the live gate would refuse. | `backtest/src/sim.rs` (no `risk::`) |
| 8 | **The expression grammar is implemented three or four times** and they disagree. | domain EBNF, `strategy-validator`, `strategy-runtime` bytecode + interpreter, `strategy-lang` |
| 9 | **The backtest venue facade is market-only, one instrument, and reports no fills back**, so the host guesses its own position. | `market_simulator` `sdk.rs` (`SimOrderCommand::Market` only) |
| 10 | **Paper deployments are never recorded**, so forward evidence cannot accumulate. | `mlops.paper_deployment` has no writer |

Earlier designs saw several of these coming — FEAT-004 (target positions, prediction series in
backtests), FEAT-005 (drive the live instance in backtests "so parity holds by construction"),
FEAT-006 (walk-forward prediction series), COMP-002 (one broker interface for all three
modes). None of those halves were built, and the code that was built grew apart. This spec is
the consolidation, plus what the owner has now asked for that none of them covered: two
strategy kinds, any model type with typed I/O, graph composition, and full order control.

---

## 2. Architectural drivers

Quality-attribute scenarios, in priority order. Every decision below names the ones it serves.

| ID | Attribute | Scenario | Measure |
|---|---|---|---|
| **QA-1** | **Parity** | A paper session's recorded inputs are replayed through the backtest environment | the decision log is **byte-identical** — same intents, same orders, same order |
| **QA-2** | **Point-in-time integrity** | A backtest consumes a bar, an indicator, or a model output at time *t* | every input was knowable at *t*; a model output comes from a model trained before *t − embargo*; a violation fails the run naming the first offending timestamp |
| **QA-3** | **Extensibility** | Add a new model type, indicator family, order type, or operator | one registry entry or adapter; **no engine change**; existing strategies unaffected |
| **QA-4** | **Authorability** | A 7B local model, a frontier model and a human each author the same strategy | all three produce the same canonical graph through the same validator with the same teachable errors |
| **QA-5** | **Safety** | A strategy depends on a value that is unavailable (warm-up, model timeout, missing data) | it **holds**, never trades on a guess; every order in every environment passes the risk gate |
| QA-6 | Performance | One-instrument, one-year 1m backtest, no models | ≥ 1 M bar-steps/min single core; live decision latency < 50 ms excluding model calls |
| QA-7 | Determinism | The same backtest is run twice | identical results and identical `run_id` |

---

## 3. The shape of the system

```
                   ┌───────────────────────── authoring (identical for humans and AI) ─────────────────────────┐
                   │  visual graph editor   ·   SLv3 text   ·   builder tool-calls   →   ONE validator/compiler │
                   └──────────────────────────────────────────────┬────────────────────────────────────────────┘
                                                                  │ canonical StrategyGraph (content-hashed revision)
                                                                  ▼
 ┌────────────────────────────────── Environment (one per mode) ───────────────────────────────────┐
 │                                                                                                  │
 │  Clock ── MarketData ──┐                                                                         │
 │                        ▼                                                                         │
 │                ┌──────────────── STRATEGY ENGINE (pure, synchronous, deterministic) ──────────┐  │
 │  Portfolio ───▶│ features (DATA-006) → graph evaluation → intents → ORDER PLANNER → orders    │  │
 │  view          │            ▲                                                                 │  │
 │                │            └── model requests ⇄ ModelProvider (resume)                       │  │
 │                └──────────────────────────────────────────────┬───────────────────────────────┘  │
 │                                                               ▼                                  │
 │                                         RISK GATE  →  ExecutionEngine  →  Venue                  │
 │                                                               │                                  │
 │                                       fills / order updates ──┘  ──▶ back to engine + portfolio  │
 │                                                                                                  │
 │                                         Decision journal (every step, every environment)          │
 └──────────────────────────────────────────────────────────────────────────────────────────────────┘

 Environment     Clock        MarketData                  ModelProvider                      Venue
 ───────────     ─────        ──────────                  ─────────────                      ─────
 Backtest        simulated    BarStore replay (PIT)       walk-forward, fold-correct (PIT)   SimulatedVenue (bar fill model)
 Paper           wall         live bus + BarStore warmup  inference gateway, pinned version  PaperVenue (tick/mark fill model)
 Live            wall         live bus + BarStore warmup  inference gateway, pinned version  broker adapter (venue-router)
```

**The rule the whole design hangs on:** the engine never knows which environment it is in. It
receives events and a portfolio view, asks for model outputs by request, and returns decisions.
Everything that legitimately differs between backtest, paper and live — time, prices, fills,
how a model is reached — lives **outside** it, behind four ports. That is how "exactly the same
strategy, similar but not identical execution" becomes structural rather than a discipline.

---

## 4. Two kinds of strategy

| | **Trading strategy** | **Selection strategy** |
|---|---|---|
| Question | *What do I do with this asset, now?* | *Which assets should we be in?* |
| Bound to | one instrument per running instance (ADR 2026-06-08 "per-asset instance" kept) | a **universe** (named, as-of membership; an asset list; or all initialised assets of a class) |
| Evaluated | on its decision ticks (§7) | on its schedule or bar close, **cross-sectionally** over the universe |
| Returns | **trade intents** (§6) | a **Selection**: ranked assets, each with a score and named attributes |
| Can read | its own instrument, other declared instruments and timeframes, its own position and orders, models, a selection's attributes | per-instrument data for every member, models, cross-sectional statistics |

**A Selection is also a value a person can read** — a screener result, served and journaled.

**Deployments bind the two.** A deployment runs a trading strategy against either
`Instrument(id)` or `Selection { selection, top_n, rebalance, on_removal }`. When the selection's
membership changes, the deployment starts and stops trading instances; `on_removal` is `close`,
`let_strategy_exit`, or `hold`. Capital across members is equal-weight by default or follows a
`weight` attribute the selection emits. This keeps every trading strategy single-asset — the
mental model the platform chose in June — while answering *what to buy*.

A **backtest of a selection-bound deployment** is a portfolio backtest: one simulated clock,
every member's events merged in time order, one account.

---

## 5. Strategies are typed dataflow graphs

### 5.1 Values and types

Every wire carries a typed value. The static types:

| Type | Meaning | Examples |
|---|---|---|
| `num` | finite real | price, indicator, return, a model's regression output |
| `bool` | truth value | a comparison, a crossing |
| `cat<classes>` | one label from a declared set | a classifier's predicted class, a regime |
| `probs<classes>` | probability over a declared set | a classifier's class probabilities |
| `dist<levels>` | quantiles at declared levels | a distributional forecaster |
| `vec<n>` | fixed-length vector | an embedding |
| `text` | string | an LLM's free reply (must be parsed before it drives a decision) |
| `record{…}` | named fields of the above | a model's full output |

At run time any value may instead be **`missing(reason)`** — `warming_up`, `no_data`,
`model_abstained`, `model_timeout`, `model_unavailable`, `undefined` (division by zero, log of a
negative), `upstream(node)`. There is no `NaN` and no zero-fill anywhere in the engine:

- arithmetic and comparisons on a missing input are missing;
- `and`/`or`/`not` are Kleene three-valued — `false and missing = false`, `true or missing = true`;
- explicit handlers make absence a decision rather than an accident: `coalesce(x, d)`,
  `is_missing(x)`, `hold_last(x)`, `missing_reason(x)`;
- **a trade intent whose inputs are missing becomes `hold`**, and the journal records which value
  was missing and why (QA-5).

### 5.2 The graph

```jsonc
{
  "format": "tbot.strategy/3",
  "kind": "trading",                       // "trading" | "selection"
  "name": "trend_with_vol_gate",
  "asset_class": "crypto_spot_cex",
  "timeframe": "1h",                       // decision timeframe: 1s 1m 5m 15m 1h 4h 1d
  "params":  { "fast": {"type":"int","default":12,"min":5,"max":50}, "slow": {…} },
  "constraints": ["param('fast') < param('slow')"],
  "inputs":  [ {"id":"btc_1d", "instrument":{"kind":"fixed","instrument":"BTC-USD"}, "timeframe":"1d"} ],
  "nodes": [
    {"id":"f",    "op":"ema",           "args":{"x":"close", "n":"$fast"}},
    {"id":"s",    "op":"ema",           "args":{"x":"close", "n":"$slow"}},
    {"id":"vol",  "op":"model",         "args":{"ref":{"str":"vol_forecaster@12"},
                                               "window":{"op":"features","args":{"set":{"str":"core_v3"},"lookback":64}}}},
    {"id":"up",   "op":"crossed_above", "args":{"a":"f", "b":"s"}},
    {"id":"calm", "op":"lt",            "args":{"a":"vol.sigma", "b":0.03}},
    {"id":"go",   "op":"and",           "args":{"a":"up", "b":"calm"}},
    {"id":"entry","op":"and",           "args":{"a":"go", "b":"state.flat"}},
    {"id":"down", "op":"crossed_below", "args":{"a":"f", "b":"s"}},
    {"id":"leave","op":"and",           "args":{"a":"down", "b":"state.long"}}
  ],
  "decide": {                              // trading: ordered rules; first match wins per group
    "rules": [
      {"when":"entry", "then":{"kind":"bracket",
                               "entry":{"kind":"target","size":{"mode":"equity_fraction","x":0.5}},
                               "stop":{"amount":2,"unit":{"unit":"atr","n":14}},
                               "take_profit":{"amount":4,"unit":{"unit":"atr","n":14}}}},
      {"when":"leave", "then":{"kind":"close"}}
    ],
    "otherwise": "hold"
  },
  "triggers": {"bar_close": true, "on_fill": false},
  "risk": {"max_position_fraction": "0.5", "max_orders_per_min": 6}     // tighten-only
}
```

**Argument syntax.** A bare string is always a reference — `"close"` (a built-in source), `"f"` (a node),
`"vol.sigma"` (a field of a node's record), `"$fast"` (a parameter), `"state.flat"` (the portfolio
view). A string *literal* is written `{"str": "up"}` so that `"up"` can never be ambiguous between a
label and a node named `up`. A node may be written in place, without an id: `{"op": "features", "args": {…}}`.
Rules' `when` is a reference to a `bool` (or an inline node); expressions like `go and state.flat` are
SLv3 *text*, which lowers to nodes — the canonical form holds no expression strings.

A selection strategy replaces `decide` with `select`:

```jsonc
"universe": {"kind": "asset_class", "asset_class": "crypto_spot_cex"},
"select":   {"score": "mom_z", "where": "liq_ok", "top_n": 10, "order": "desc",
             "attributes": {"weight": "w", "regime": "reg"}}
```

### 5.3 Operator library

Operators are a **registry** (QA-3): each entry declares its input and output types, its warm-up,
whether it is stateful, and a one-line *use when*. The typechecker, the editor palette, the text
language and the agent's catalogue are all generated from it.

| Family | Operators |
|---|---|
| **Sources** | `open high low close volume` (any declared instrument/timeframe), `bar(field, instrument?, tf?)`, `feature(name)` — all 61 DATA-006 families, `param(name)`, `const`, `time.hour/dow/session`, `state.*` (§5.4), `selection(ref).attr` |
| **Math & logic** | `add sub mul div neg abs min max pow log exp sqrt sign clip round`, `gt lt ge le eq ne`, `and or not`, `between` |
| **Branching** | `if(cond, a, b)`, `switch(cat, {label: value, …}, default)` — any branch may be its own sub-chain of models and indicators |
| **Over time (stateful)** | `prev(x, n)`, `change(x, n)`, `crossed_above/below(a, b)`, `rising/falling(x, n)`, `rolling_{mean,std,min,max,sum}(x, n)` and `ema(x, n)` over **any node's output**, `bars_since(cond)`, `count_true(cond, n)`, `latch(set, reset)`, `hysteresis(x, on, off)`, `hold_last(x)`, `cooldown(cond, n)` |
| **Models** | `model(ref@version, inputs)` → `record`; field access `m.sigma`; adapters `argmax(p)`, `prob(p, label)`, `quantile(d, q)`, `mean(d)`, `prob_above(d, x)`, `cosine(v, w)`, `parse(text, schema)` |
| **Blending** | `weighted_mean([…], [w…])`, `vote([bools], k)`, `zscore(x, n)`, `pct_rank(x, n)` |
| **Cross-sectional** (selection only) | `cs_rank`, `cs_zscore`, `cs_pct_rank`, `cs_mean`, `cs_median`, `cs_count` |
| **Absence** | `coalesce`, `is_missing`, `missing_reason` |

Parallel paths are simply independent sub-graphs that meet at a later node; branching is
`if`/`switch`; chaining is a wire. A graph may be *all models*, *no models*, or any mixture —
there is no special case for any of them.

**Anything the library cannot express is a model.** A custom Python function registered in MLOps
as a `transform` (no training, a declared contract) is called exactly like a trained model. That
is the escape hatch, and it keeps the engine closed while the platform stays open (QA-3).

### 5.4 State a strategy can read

`state.flat/long/short`, `state.position` (signed units), `state.position_fraction` (of
equity), `state.entry_price`, `state.bars_in_position`, `state.unrealized_return`,
`state.realized_pnl_today`, `state.equity`, `state.cash`, `state.open_orders(tag?)`,
`state.last_fill_price`, `state.last_exit_reason`, `state.mark`. These come from the
environment's **portfolio view** at each decision, never from the engine's own guess (defect 9).

---

## 6. Decisions are trade intents

A trading strategy decides *what it wants*; one shared **order planner** works out the orders.
The intent vocabulary covers the trading terminal:

| Intent | Says | Planner does |
|---|---|---|
| `hold` | change nothing | nothing; working orders stay |
| `target {size}` | be at this position | diffs against position + working orders; rounds to lot; idempotent |
| `order {side, size, type, price?, trigger?, trail?, tif, reduce_only, post_only, tag}` | place exactly this | one order; `type` ∈ market · limit · stop · stop_limit · trailing_stop |
| `bracket {entry, stop, take_profit?, trailing?}` | enter and protect | entry, then OCO protective orders when it fills |
| `protect {stop?, take_profit?, trailing?}` | (re)set protection on the open position | replaces protective orders (e.g. move stop to breakeven) |
| `close {fraction}` | exit all or part | reduce-only market, or limit if a price is given |
| `add {size}` / `reduce {size}` | scale in / out | one order in the right direction |
| `cancel {tag?}` | withdraw working orders | cancels by tag or all |

`size` is `units(x)`, `notional(x)`, `equity_fraction(x)`, `risk(fraction, stop_distance)` or
`vol_target(target, vol)`; every number in every intent — size, limit price, stop distance,
trail — is a **graph value**, so a model's quantile can set a limit price and a model's fraction
can set size. Distances are `pct`, `abs`, `atr(k, n)` or `sigma(k)` from `entry`, `mark` or
`high_water`.

**Why intents rather than raw orders (ADR-0041).** A target is restart-safe: re-evaluating after
a crash yields the same target and no duplicate entry. It is environment-independent: the
planner is the same code everywhere, so only the venue's *answer* differs. And it makes the
strategy auditable: the journal shows *what was wanted* separately from *what was sent*.

The planner also owns: lot/tick rounding from instrument metadata; minimum order size; cancelling
a working entry that a new target contradicts; OCO bookkeeping (emulated when a venue lacks it);
and refusing an intent it cannot express on this venue, by name.

---

## 7. When a strategy thinks (ADR-0042)

| Trigger | Trading | Selection | Why |
|---|---|---|---|
| **Bar close** of the decision timeframe | default | default | the only cadence a bar-data backtest can reproduce exactly (QA-1) |
| **Own fill / order update** | opt-in (`on_fill`) | — | react to an entry filling: place protection, scale, cancel siblings |
| **Schedule** (cron, exchange-calendar aware) | opt-in | opt-in | rebalances, end-of-day exits |
| Every tick | **no** | **no** | cannot be reproduced from bars, so it would break parity by construction |

Tick-speed protection still exists: stops, targets and trailing stops are **resting orders at
the venue** (or venue-emulated), so they trigger on every tick in paper/live and on the bar's
high/low path in a backtest. The *decision* is bar-paced; the *protection* is not. Models are
called only on decision ticks.

---

## 8. Models (ADR-0043)

### 8.1 Contracts

Every model version carries an **IoContract**:

```jsonc
{
  "inputs": [
    {"name":"window", "shape":"window", "features":{"kind":"set","name":"core_v3"}, "lookback":64, "timeframe":"1h"},
    {"name":"regime", "shape":"scalar", "value_type":{"type":"cat","labels":["calm","trend","stress"]}, "optional":true}
  ],
  "outputs": [
    {"name":"sigma",     "value_type":{"type":"num"}},
    {"name":"quantiles", "value_type":{"type":"dist","levels":[0.05,0.25,0.5,0.75,0.95]}},
    {"name":"direction", "value_type":{"type":"probs","labels":["down","flat","up"]}}
  ],
  "horizon": "4h",
  "pit": {"walk_forward": true, "embargo": "4h"},
  "cost_class": "cheap",           // cheap | moderate | expensive (LLMs)
  "timeout_ms": 250
}
```

Input shapes: `scalar(type)`, `row(feature_set)`, `window(feature_set, lookback, timeframe)`,
`cross_section(...)` (selection), `text(template)`. The typechecker refuses a strategy that wires
the wrong type into a model, **at validation, not at run time** — "make sure the strategy is
capable of inputting and receiving the proper data type" is a compile error, not a hope.

Existing models get a **derived** contract from their kind and bundle header (`feature_order`) —
the contract their serving path actually honours today, not an aspirational one:
Forecaster → `{direction: cat<down,flat,up>, confidence: num, median_return: num, sigma: num,
quantiles: dist}` (a *label*, because the v1 server returns one; a forecaster that serves
probabilities declares `probs`); TradeDecision → `{action: cat, confidence: num}`; RiskSizing → `{size_fraction: num}`; SignalRanker →
`{score: num}`; Embedding → `{embedding: vec<n>}`; LLM adapters declare a JSON output schema and
are reached through `parse`. New generic kinds: `classifier`, `regressor`, `transform`.

### 8.2 Resolution per environment

| | Backtest | Paper / live |
|---|---|---|
| Version | pinned at run start (alias → version recorded in the manifest) | pinned at deployment; an alias moving does **not** change a running strategy |
| Which model answers at *t* | the walk-forward fold whose `train_end + embargo < t` | the pinned version |
| No such fold | `missing(model_unavailable)` → the strategy holds; counted and reported | n/a |
| Mechanism | batched calls per (model, fold), cached as a prediction series | inference gateway, batched per tick, timeout → `missing(model_timeout)` |

Using one final model over a whole backtest would let a model trained on the backtest's own
period trade it — lookahead. That is allowed only as a declared, flagged `optimistic_models`
policy for exploration, never as a default, the way `CurrentClose` fills are treated.

### 8.3 The serving protocol, v2

The sidecar returns **named typed outputs per contract**, not a `Forecast`. A missing required
input is an **abstain with a reason**, never a column of zeros. The v1 `Forecast` survives as
the derived contract of a forecaster, so nothing that works today breaks.

### 8.4 How a model call fits a synchronous engine

`engine.on_event()` evaluates up to the first layer of model nodes and returns
`NeedModels(requests)`; the environment resolves them (concurrently, batched, cached, timed out)
and calls `engine.resume(responses)`; chained models resolve layer by layer. The engine stays
pure and deterministic; only the provider differs.

---

## 9. Environments

### 9.1 Ports

| Port | Responsibility |
|---|---|
| `Clock` | the current instant; simulated in backtest, wall in paper/live |
| `MarketData` | bars (and declared extra series) for every instrument and timeframe the plan requires, point-in-time, plus the warm-up history |
| `ModelProvider` | §8.2 |
| `Venue` (via `RiskGate` → `ExecutionEngine`) | accepts orders; reports acks, fills, cancels; owns positions, cash, open orders |
| `Journal` | §11 |

### 9.2 Venues — similar, not identical (ADR-0044)

Backtest and paper share one **venue core** — order book of working orders, positions, cash,
fees from `FeeBook`, OCO, per-asset-class accounts — extracted from today's
`PaperTradingEngine`. They differ only in what drives it and how a fill is decided:

| | SimulatedVenue (backtest) | PaperVenue (paper) |
|---|---|---|
| Driven by | historical bars on a simulated clock | live marks and trades |
| Market order | fills at **next bar's open** ± modelled half-spread and slippage (default) | fills at the next mark ± the same cost model |
| Limit / stop | triggers on the next bars' high/low path | triggers on mark crossing |
| Both stop and target inside one bar | **stop first** (pessimistic intrabar) | whichever the tick stream crosses first |
| `CurrentClose` fills | declared optimistic policy only, flagged in every report | n/a |

This is the precise sense of *work similarly but not exactly the same*: the same machinery, the
same costs, the same accounting; a different clock and a different source of prices.

`market_simulator` (ADR-0006/0014) remains available as a **reference venue** behind the same
port for cross-checking fills on the instruments it supports. It is no longer the default,
because its facade is market-only, single-instrument and reports no fills back, and widening it
would put the platform's order capability in another repository.

### 9.3 Risk gate everywhere (ADR-0046)

Every order from every strategy in every environment passes `RiskGate::check` with that
environment's context (simulated equity and clock in backtests). A backtest reports gate
rejections as first-class events; a strategy that only made money through orders the gate
would have refused is visible as such.

### 9.4 Backtest runner

Resolves the revision, registers the trial (INV-16), materialises params, compiles, computes the
plan's warm-up, merges every required series into one time-ordered stream, steps the engine at
each decision tick, resolves model requests point-in-time, routes orders through the risk gate to
the SimulatedVenue, feeds fills back, and writes results in the shape Set J already consumes
(trade list, daily return series, manifest with execution policy and model pins). **Preview is a
short backtest run of the same engine** — there is no separate preview evaluator.

### 9.5 Live runner (paper and live)

Replaces hot-path stage 3. Per deployment: warm up from `BarStore`, subscribe to the live bus,
step the engine on decision ticks, resolve models asynchronously with a timeout, route through
the risk gate and execution engine, feed fills and order updates back, checkpoint engine state
so a restart resumes rather than re-enters. **Starting a deployment writes the deployment
record** (defect 10) — forward evidence accumulates from the first decision.

---

## 10. Authoring — identical for humans and AI (ADR-0045)

- **The canonical graph is the strategy.** Its content hash is the revision (ADR-0010's
  "canonical JSON" principle, carried forward).
- Three editors, one artefact: the **visual graph editor** (people), **SLv3 text** (the most
  token-efficient form for models; extends SLv2), and **builder tool-calls**
  (`add_node`, `connect`, `add_rule` — for small local models that need a constrained grammar).
  All three round-trip through the canonical graph.
- **One validator / typechecker / compiler** (`strategy-lang`), consuming the operator, feature
  and model-contract registries. It replaces `strategy-validator`'s grammar, the runtime
  interpreter, and the domain EBNF as an implementation. Errors stay teachable (code, fix,
  example — the `teach.rs` approach).
- One API, used by the UI and the agent alike (the MCP server is a thin client, ADR-0022):
  `validate`, `compile`, `explain`, `preview`, `backtest`, `run selection`, `propose deployment`.
- Testing is identical: validate → preview → backtest → Set J experiments and gates → a deployment
  proposal a human decides.

SLv3 text, illustrative:

```
strategy trend_with_vol_gate kind trading timeframe 1h
params fast: int[5..50]=12, slow: int[20..200]=48
let f   = ema(close, fast)
let s   = ema(close, slow)
let vol = model("vol_forecaster@12", window: features(core_v3, 64))
let go  = crossed_above(f, s) and vol.sigma < 0.03
when go and flat            -> bracket target 50% equity, stop 2 atr(14), take 4 atr(14)
when crossed_below(f, s) and long -> close
otherwise hold
```

---

## 11. The decision journal and the parity proof (ADR-0047)

Every engine step, in every environment, writes one record: the decision instant, a digest of
every input value, every model request and response (with version and fold), the intents, the
planned orders, the risk-gate verdicts, and every `missing` with its reason. Backtests keep it in
the run artifact; paper and live in `strategy_decision_journal`.

**The parity check:** a nightly job takes yesterday's paper journal, replays its recorded inputs
through the backtest environment's engine, and requires the decisions to be byte-identical
(QA-1). A difference is a defect in the engine boundary, named with the first differing step.
This is the mechanism that keeps "exactly the same" true after this spec is old.

---

## 12. Migration

- Stored v1.0–1.5 definitions translate mechanically: `condition`+`signal`+`place_order` → a rule
  whose intent is a market `order`; `exits` → `bracket`/`protect`; `Inference`, `ModelForecast`,
  `Decision`, `Sizing`, `LlmInference`, `Combine` → `model` nodes with derived contracts and
  adapters; universe nodes → a selection strategy.
- Translated strategies are **not** expected to backtest identically to their old results: the
  old results came from an evaluator that dropped model calls and filled at the bar close. The
  translator emits the list of behaviours that changed, per strategy.
- Old paths run until the replay parity check passes on translated strategies, then are deleted:
  `sim.rs`'s evaluation loop, `preview.rs`'s loop, `interpreter.rs`, `strategy-validator`'s
  grammar, hot-path stage 3's `StrategyInstance`.

## 13. Module view

| Crate | Owns after Set U |
|---|---|
| `domain::strategy_graph` (new module) | graph, value types, intents, selection, deployment, IoContract — data only |
| `strategy-lang` | SLv3 ↔ graph, typechecker, validator, compiler → `ExecutionPlan`, v1 translator, teachable errors |
| `strategy-engine` (**new**) | the pure engine: plan execution, features (via `features`), temporal state, value semantics, model request/resume, order planner, decision trace |
| `features` | unchanged — the one indicator engine |
| `model-registry` | contracts on versions, derived contracts, `ModelProvider` impls, prediction series |
| `execution` | venue core; `SimulatedVenue`, `PaperVenue`, broker adapters; `ExecutionEngine` |
| `risk` | unchanged — called from every environment |
| `backtest` | `BacktestRunner` (replaces the evaluation half of `sim.rs`), Set J layer unchanged |
| `apps/platform` | `LiveRunner` replaces hot-path stage 3; deployments |
| `strategy-runtime` | retired at the end of Set U |

Deployment: all of the above in the platform process and job workers; models in the Python
sidecar(s) as today.

## 14. Risks, sensitivity and trade-off points

- **Risk — model latency in live.** A slow model delays a bar-close decision. *Mitigation:* per-
  contract timeouts; `missing(model_timeout)` holds; the journal counts timeouts; decisions are
  bar-paced so a 250 ms budget is comfortable at 1m and above.
- **Risk — walk-forward folds are expensive to train.** Models without folds can only answer
  after their `train_end + embargo`, so early backtest history holds. *That is correct* and is
  reported, not hidden; the predict-series job amortises the cost.
- **Sensitivity point — the fill model.** Next-bar-open vs close moves every result. It is part of
  `RunConfig`, hashed into `run_id`, printed in every report, and the oracle test still gates it.
- **Trade-off — bar-paced decisions vs tick reaction.** Chosen for parity (QA-1) over reaction
  speed; protection rides on resting orders to recover most of what tick evaluation would give.
- **Trade-off — native venue vs `market_simulator`.** Chosen for full order types, multi-
  instrument portfolios and shared code with paper, at the cost of owning fill simulation. The
  reference venue and the forward reconciliation of Set T keep the native venue honest.
- **Non-risk — indicators.** DATA-006 already has one evaluation routine and a window that cannot
  read the future; the engine uses it as is.

## 15. Acceptance

1. One `strategy-engine` crate is the only code that evaluates a strategy in backtest, preview,
   paper and live — enforced by a source-scan guard naming the retired evaluators.
2. A paper journal replays through the backtest environment byte-identically (QA-1).
3. An indicator strategy fires on paper with no injected feature values.
4. A strategy chaining two models and an indicator through `if` backtests with fold-correct model
   outputs and paper-trades through the gateway; a G0 check fails a deliberately leaky variant.
5. A classifier's `probs`, a regressor's `num`, a forecaster's `dist`, an embedding's `vec` and an
   LLM's parsed record are each consumed by a strategy without being reshaped into a forecast.
6. A wrongly-typed model input fails validation with a teachable error.
7. Every intent kind in §6 executes in backtest and paper, including a bracket whose stop and
   target fall in one bar (stop wins in backtest).
8. A selection strategy run on a universe returns a ranked list; a deployment bound to it starts
   and stops trading instances as membership changes; the portfolio backtest has one account.
9. Every backtest order passes the risk gate; rejections appear in results.
10. A human in the editor and the agent through tools produce the same canonical graph for the
    same strategy, and the same validator errors for the same mistake.
