# FEAT-004: Strategy Representation v2 — Research Strategies and Strategy Language v2

**Status:** Proposed (Phase 0 contract; not implemented)
**Version:** 0.1
**ADR(s):** ADR-0026 (two layers), ADR-0029 (shared expression language), ADR-0023 (P1:
optimiser chooses numbers); supersedes ADR-0007 as the authoring format
**Derived from:** BS-007 [08_STRATEGIES](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/08_STRATEGIES.MD)
**Updates:** DATA-004 (strategy definition format → v2 AST), FEAT-001
**Plan sets:** M (Layer 1), O (Layer 2)
**Crates and packages:**
- new `crates/strategy-lang` (SLv2 lexer, parser, AST, type checker, compiler, v1
  translator);
- `crates/strategy-runtime` (bytecode extensions, `StrategyInstance`);
- `crates/strategy-validator` (v2 rules);
- `crates/domain::strategy_def` (v2 AST types);
- `sdk/python/tbot/strategy_runtime.py` (Layer 1);
- the research-runner worker (COMP-005);
- `migrations/0041_strategy_v2.sql`.

---

## 1. Purpose

Give the agent unlimited expressiveness for research (Layer 1), and a safe, fast,
compiled, live-parity form for anything deployable (Layer 2). A mandatory
reconciliation check joins the two.

## 2. Layer 1: research strategies (position series)

### 2.1 SDK contract

```python
from tbot.strategy import research_strategy, Param, target_vol_scale

@research_strategy(
    instruments=["BTC-USD"], timeframe="1h",
    params={"lookback": Param.int(10, 100), "k": Param.float(0.5, 3.0)},
    position_units="fraction",          # fraction of equity in [-1, 1] (default) | units
    allow_short=False,                  # default False; explicit to enable
)
def positions(data, p) -> "polars.Series":   # index = decision times (bar close); values = target position
    rv  = data.feature("rolling_std(logret(close), 20)")
    rng = data.feature(f"rolling_max(high, {p.lookback}) - rolling_min(low, {p.lookback})")
    sig = (data.close > data.close.shift(1) * (1 + p.k * rv)) & (rng.pct_rank(250) < 0.2)
    return sig.cast(float) * target_vol_scale(rv, 0.4)
```

- **The `data` object** is supplied by the runner:
  - bars;
  - `feature(expr)` (DATA-006 batch engine);
  - `prediction(series_ref)`;
  - `text(series_ref)`;
  - other declared instruments;
  - `rng` (seeded).

  Everything is PIT, clipped at the project cutoff, and aligned to decision times.
- **Validity:**
  - output index ⊆ decision times;
  - values are finite, within bounds (`allow_short`), and NaN is treated as "no change";
  - the declared params are the only free parameters. The lint flags numeric literals
    that look tuned; a warning at M, configurable to an error.
- **Purity:** the runner executes in the research-runner container with no network, a
  read-only code snapshot, scratch-only writes, no clock (a fixed frozen time), and a
  seeded `rng`. Wall-time and memory limits apply.

### 2.2 Execution

`bt.run` with a Layer 1 file submits a `research_run` job, which produces a
`position_series` artifact. That triggers a `backtest` job with
`RunKind::PositionSeries { series_handle }` (FEAT-005 §4.2). Both are linked to the
Experiment; the backtest is the counted trial. The code snapshot hash is part of
`strategy_version` (COMP-005 §4).

### 2.3 Truncation test (Gate 0)

- For K truncation points (default K=8, stratified across the window, seeded), re-run
  the function on data ending at `t_k`.
- **Pass** iff `positions_trunc[t ≤ t_k] == positions_full[t ≤ t_k]` exactly, with
  NaN == NaN.
- **Failure** records the first differing timestamp and a feature diff (which inputs
  differed) in the gate verdict.
- The test is a `research_run` sub-job. Its cost counts toward compute, not trials.

### 2.4 Sweeps

The FEAT-003 sweep engine samples params (Random/TPE) and submits member
`research_run` + `backtest` pairs. INV-2 applies unchanged, and the agent never picks
values.

## 3. Layer 2: Strategy Language v2 (SLv2)

### 3.1 Grammar (EBNF, v2.0)

```ebnf
strategy   = "strategy" ident "on" instrument_ref "timeframe" tf NL
             [params] [inputs] { entry } [size] { exit } [risk] ;
params     = "params" param { "," param } NL ;
param      = ident ":" ptype "[" lit ".." lit "]" [ "=" lit ] ;       (* ptype: int | float | bool *)
inputs     = "inputs" binding { NL binding } ;
binding    = ident "=" expr ;                                          (* DATA-006 expression *)
entry      = "entry" ("long" | "short") "when" cond [ "cooldown" int "bars" ] [ "hysteresis" expr ] NL ;
size       = "size" size_expr [ "clamp" lit ".." lit ] NL ;
size_expr  = "fixed" lit | "equity_fraction" expr | "risk_per_trade" expr "stop" "=" stop_ref
           | "vol_target" "(" expr "," expr ")" | "kelly" "(" expr ["," "cap" "=" lit] ")" | "model" "(" model_ref ")" "." ident ;
exit       = "exit" ( "stop" dist | "target" dist | "trailing" dist | "time" int "bars" | "when" cond ) NL ;
dist       = expr [ "%" | "atr" | "sigma" ] "from" ("entry" | "high_water") ;
risk       = "risk" { risk_item } NL ;                                  (* tighten-only vs platform risk gate *)
risk_item  = "max_position" lit | "max_orders_per_min" int | "max_daily_loss" lit ;
cond       = expr cmp expr | cond ("and"|"or") cond | "not" cond | state_pred | "crossed_above" "(" expr "," expr ")" … ;
state_pred = "flat" | "long" | "short" ;
expr       = DATA-006 expression | state_field | ident ;
state_field= "position" | "entry_price" | "bars_in_trade" | "unrealized_return" | "equity" | "last_exit_reason" ;
model_ref  = string ;                                                   (* "alias@stage" or model_version_id *)
```

- **Order types:** optional `order` clauses on entry/exit (`order limit offset 0.1% tif gtc`,
  `order stop`, `order stop_limit …`); the default is market.
- **Reserved for the multi-instrument decision (OQ-040):** `universe`, `rank`,
  `portfolio`.

### 3.2 Semantics

- **Decisions** are evaluated on bar close with close-stamped features. Orders go to the
  engine under the Run's execution policy (FEAT-005 §3; default fill at next open).
- **Entries:** in v2.0, one position per instrument; the first satisfied entry fires if
  flat. Pyramiding is out of v2.0.
- **Exits** compile to engine orders:
  - stop and target become a bracket/OCO at entry;
  - trailing becomes a trailing-stop order;
  - `time` is evaluated by the runtime;
  - `when` is evaluated at close.
  The first exit to trigger wins, and `last_exit_reason` records it.
- **Model calls:** `model("vol_forecaster@production").sigma` binds an
  `InferenceOutput` field. In backtests this resolves to a **prediction series** bound at
  run setup (FEAT-006 §6); live, to the inference gateway. The abstain policy is
  `hold | flat | skip_entry` (default `skip_entry`).
- **`combine(a, b, …)`** is an equal-weight mean unless explicit weights are declared.
  Explicit weights require a validation note (BACKTEST_SUITE v2 T6).

### 3.3 Pipeline

```
text ─lexer/parser→ CST ─lower→ AST (canonical JSON, DATA-004 v2) ─typecheck/validate→ Plan
     ─compile→ { FeaturePlan (DATA-006), bytecode Programs (conditions/sizing), OrderTemplates, RiskLimits }
```

**Bytecode extensions** (`crates/strategy-runtime/src/bytecode.rs`, today
`LoadFeature, LoadBarField, Const, Add, Sub, Mul, Div, Neg, Gt, Lt, GtEq, LtEq, EqEq,
BangEq`):
- add `And, Or, Not`;
- add `Abs, Min, Max, Log, Exp, Sqrt, Sign, Clip`;
- add `LoadState(StateField)`;
- add `LoadModelField(slot)`;
- add `CrossedAbove/Below(slot_a, slot_b)`, using a per-slot previous-value register.

Rolling functions, lookback and every indicator are **not** bytecode: they compile to
DATA-006 incremental features bound to slots. Conditions only combine slot values.

### 3.4 AST (DATA-004 v2) — excerpt

```jsonc
{ "format": "tbot.strategy/2.0", "name": "vol_breakout_v3", "instrument": "$instrument", "timeframe": "1h",
  "params": [{"name": "lookback", "type": "int", "min": 10, "max": 100, "default": 40}],
  "inputs": [{"name": "rv", "expr": {"call": "rolling_std", "args": [{"call": "logret", "args": [{"field": "close"}]}, 20]}}],
  "entries": [{"side": "long", "when": {"and": [ … ]}, "cooldown": null}],
  "size": {"kind": "vol_target", "target": {"param": "tv"}, "forecast": {"model": "vol_forecaster@production", "field": "sigma"}, "clamp": [0, 1]},
  "exits": [{"kind": "stop", "dist": {"mul": [2.0, {"call": "atr", "args": [14]}]}, "unit": "abs", "from": "entry"}, …],
  "risk": {"max_position": 1.0, "max_orders_per_min": 6},
  "model_bindings": [{"ref": "vol_forecaster@production", "fields": ["sigma"], "abstain": "skip_entry"}] }
```

- **Canonicalisation:** keys sorted, defaults filled, expressions normalised as in
  DATA-006.
- `strategy_version = sha256(canonical AST after param materialisation)`, or, for Layer
  1, `sha256(code snapshot ‖ decorator metadata)`.

### 3.5 Validation (teachable errors)

`{code, line, col, rule, message, fix}`. The rules are:
- unknown function or feature (with suggestions);
- type errors;
- missing warm-up;
- an exit distance ≤ 0;
- sizing outside the clamp;
- risk limits looser than the platform risk gate, which is an error since limits are
  tighten-only;
- a model ref that doesn't resolve;
- params without ranges;
- unreachable entries;
- `combine` weights without a validation note.

## 4. Storage (`migrations/0041_strategy_v2.sql`)

```sql
CREATE TABLE strategy_versions (
  strategy_version TEXT PRIMARY KEY,           -- sha256
  slug TEXT NOT NULL, layer SMALLINT NOT NULL CHECK (layer IN (1,2)),
  format TEXT NOT NULL,                        -- tbot.strategy/2.0 | tbot.research/1.0
  ast JSONB,                                   -- layer 2
  source_text TEXT,                            -- layer 2 text (as authored)
  code_handle TEXT,                            -- layer 1 code snapshot artifact
  params_schema JSONB NOT NULL, owner_user_id UUID NOT NULL, project_id UUID,
  parent_version TEXT REFERENCES strategy_versions, created_by TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE strategy_slugs (slug TEXT PRIMARY KEY, owner_user_id UUID NOT NULL,
  current_version TEXT REFERENCES strategy_versions, deployed_version TEXT REFERENCES strategy_versions);
CREATE TABLE strategy_reconciliations (
  id TEXT PRIMARY KEY, layer1_version TEXT, layer2_version TEXT, experiment_id TEXT,
  agreement DOUBLE PRECISION, pnl_tracking_error DOUBLE PRECISION, explained_diffs JSONB,
  passed BOOLEAN, created_at TIMESTAMPTZ DEFAULT now());
```

- Versions are immutable. A slug moves `current_version`.
- **`deployed_version`** changes only through a human-approved deployment. Changing a
  deployed strategy restarts the funnel for the new version (BACKTEST_SUITE v2 §4,
  rule 5c).
- The existing v1 `strategies` rows stay. The translator (§5) writes v2
  `strategy_versions` alongside them.

## 5. v1 → v2 translation

- `tbot strategy translate-v1 <slug>` and a one-shot migration job translate every
  stored v1.0–v1.5 definition:
  - inputs → `inputs` bindings;
  - `condition`/`signal` nodes → `entry … when` (the rising-edge semantics map to
    `crossed_above` or an edge helper);
  - `place_order` + `sizing` → `size`;
  - `model_forecast`/`inference`/`decision` nodes → `model_bindings`;
  - universe/rank nodes → reserved sections (flagged if used).
- **Round-trip test:** for every stored definition, the v1 backtest (current path) and
  the v2 backtest (StrategyInstance path, same fill policy) produce identical trades. Any
  mismatch is listed and must be explained (e.g. a v1 bug) before v1 authoring is
  retired.

## 6. Moves (typed structural edits)

`tbot strategy move <base_version> --move <kind> …` produces a new version from a typed
AST diff, validated. The move kinds are the FEAT-003 §8.3 vocabulary, extended:
- `add_filter`;
- `remove_filter`;
- `add_regime_gate`;
- `swap_signal`;
- `add_exit`;
- `change_exit_unit`;
- `change_sizing`;
- `add_hysteresis`;
- `change_timeframe`;
- `lookback_ensemble`;
- `simplify`.

- The canonical AST-diff **motif** is recorded in move memory (AGENT-003 §3,
  `move_memory`).
- A novelty filter (AST similarity plus signal correlation against the project's
  existing versions) warns before submission.

## 7. Research → deploy reconciliation

- `tbot strategy reconcile <layer1_version> <layer2_version> --experiment <id>` runs
  both on the Experiment's in-sample window with the same execution policy and costs.
- **Pass** iff position agreement ≥ `recon.agreement_min` (default 0.99 of decision
  times), P&L tracking error ≤ `recon.te_max`, and every disagreement lies in a declared
  approximation class.
- A pass is required before a Layer 1 idea's Layer 2 version may enter G3 or later.

## 8. UI

The builder becomes an editor over the AST (COMP-006 §6): text and structured views of
the same AST, with a palette generated from the DATA-006 registry. Validation errors
are inline.

## 9. Test plan and acceptance

| # | Test | BS-007 IDs |
|---|---|---|
| S1 | §2.1 strategy runs as a `position_series` Experiment through G2; a `bfill` variant fails the truncation test naming the first differing timestamp | ST-01…ST-03 |
| S2 | A Layer 1 sweep samples params via the platform sampler; no argmax is exposed | ST-04 |
| S3 | A §3 example with stop, target, time exit and vol-target sizing compiles, backtests, and is replay-deterministic | ST-05, ST-06 |
| S4 | The AST round-trips text ↔ AST ↔ text (formatting normalised) | ST-07 |
| S5 | All stored v1 definitions translate; the round-trip backtests match or have listed explanations | ST-08 |
| S6 | Editing any byte of a version produces a new hash; deployed versions can't be mutated | ST-09 |
| S7 | Reconciliation passes for a faithful translation and blocks a deliberately different one | ST-10 |
| S8 | `combine` without weights is equal-weight; explicit weights without a validation note fail validation | ST-11 |

## 10. Open questions

1. When to lift OQ-040 (multi-instrument SLv2).
2. Reconciliation thresholds per timeframe.
3. Whether pyramiding and partial exits belong in v2.1.
